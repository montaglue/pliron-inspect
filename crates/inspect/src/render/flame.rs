//! A flamegraph of where a run's measured cycles go in one snapshot:
//! module → function → loop → block → op, each box as wide as the cycles
//! under it. The cycles come from the trace's profile (backward PGO), kept
//! per line an op starts on; the structure comes from the dialect engine's
//! model, and the loops from each function's control flow (natural loops
//! of its back edges, nested by containment).

use std::collections::{BTreeMap, HashMap, HashSet};

use pliron_lsp_protocol::op_traits;
use serde::Serialize;

use super::Ir;

/// The profile of the ops starting on each line (1-based): metric → value.
pub type LineProfile = HashMap<u32, BTreeMap<String, f64>>;

/// Cycles an op's own code took.
const CYCLES: &str = "cycles";
/// How often an op ran.
const EXECUTIONS: &str = "executions";
/// Cycles of code a pass created as its own decision (`regalloc.cycles`).
const DECISION_SUFFIX: &str = ".cycles";

#[derive(Debug, Serialize)]
pub struct FlameGraph {
    pub root: FlameNode,
    /// Cycles of the whole profile, the root's width.
    pub total: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Module,
    Function,
    Loop,
    Block,
    Op,
    /// Code a pass created as its own decision (spill code, prologue, ...);
    /// named after the pass.
    Decision,
    /// Profiled lines on which the model has no op.
    Other,
}

#[derive(Debug, Serialize)]
pub struct FlameNode {
    pub name: String,
    pub kind: Kind,
    /// Cycles under the box: its own and its children's.
    pub cycles: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executions: Option<f64>,
    /// The snapshot line (1-based) the box's IR starts on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// The box's IR, when its name shortens it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<FlameNode>,
}

impl FlameNode {
    fn new(name: String, kind: Kind, line: Option<u32>, children: Vec<FlameNode>) -> FlameNode {
        let children: Vec<FlameNode> = children.into_iter().filter(|c| c.cycles > 0.0).collect();
        FlameNode {
            name,
            kind,
            cycles: children.iter().map(|c| c.cycles).sum(),
            executions: None,
            line,
            text: None,
            children,
        }
    }
}

/// The flamegraph of `ir` under `profile`.
pub fn flamegraph(ir: &Ir, profile: &LineProfile) -> FlameGraph {
    let model = ir.model();
    let mut taken = HashSet::new();
    let mut functions = Vec::new();
    // Functions: symbol ops with a body of blocks, in document order.
    let mut stack = vec![model.root];
    while let Some(op) = stack.pop() {
        let info = &model.ops[op as usize];
        if let (Some(symbol), Some(&body)) = (&info.symbol, info.regions.first())
            && info.traits & op_traits::SYMBOL_TABLE == 0
            && ir.region_has_blocks(body)
        {
            functions.push(function_node(ir, op, symbol, body, profile, &mut taken));
            continue;
        }
        for region in info.regions.iter().rev() {
            for block in model.regions[*region as usize].blocks.iter().rev() {
                stack.extend(model.blocks[*block as usize].ops.iter().rev());
            }
        }
    }
    let untaken: f64 = profile
        .iter()
        .filter(|(line, _)| !taken.contains(*line))
        .map(|(_, metrics)| cost(metrics))
        .sum();
    if untaken > 0.0 {
        functions.push(FlameNode {
            name: "lines without an op of the model".to_string(),
            kind: Kind::Other,
            cycles: untaken,
            executions: None,
            line: None,
            text: None,
            children: Vec::new(),
        });
    }
    let root_info = &model.ops[model.root as usize];
    let name = root_info
        .symbol
        .as_ref()
        .map_or_else(|| root_info.opid.clone(), |symbol| format!("@{symbol}"));
    let root = FlameNode::new(name, Kind::Module, ir.op_line(model.root), functions);
    FlameGraph {
        total: root.cycles,
        root,
    }
}

/// An op's cycles: its own code's and its decisions'.
fn cost(metrics: &BTreeMap<String, f64>) -> f64 {
    metrics
        .iter()
        .filter(|(name, _)| name.as_str() == CYCLES || name.ends_with(DECISION_SUFFIX))
        .map(|(_, value)| value)
        .sum()
}

/// The profile of the op starting on `line`, unless an op took it already.
fn take<'p>(
    profile: &'p LineProfile,
    taken: &mut HashSet<u32>,
    line: Option<u32>,
) -> Option<&'p BTreeMap<String, f64>> {
    let line = line?;
    let metrics = profile.get(&line)?;
    taken.insert(line).then_some(metrics)
}

/// One box per decision metric of `metrics` (`regalloc.cycles` → `regalloc`).
fn decisions(metrics: &BTreeMap<String, f64>, line: Option<u32>) -> Vec<FlameNode> {
    metrics
        .iter()
        .filter_map(|(name, &value)| {
            let pass = name.strip_suffix(DECISION_SUFFIX)?;
            (value > 0.0).then(|| FlameNode {
                name: pass.to_string(),
                kind: Kind::Decision,
                cycles: value,
                executions: None,
                line,
                text: None,
                children: Vec::new(),
            })
        })
        .collect()
}

fn function_node(
    ir: &Ir,
    op: u32,
    symbol: &str,
    body: u32,
    profile: &LineProfile,
    taken: &mut HashSet<u32>,
) -> FlameNode {
    let model = ir.model();
    let line = ir.op_line(op);
    // The function's own decisions (ABI glue, frame, ...) sit on its op.
    let own = take(profile, taken, line)
        .map(|metrics| decisions(metrics, line))
        .unwrap_or_default();
    let blocks = model.regions[body as usize].blocks.clone();
    let mut block_nodes: HashMap<u32, FlameNode> = blocks
        .iter()
        .map(|&block| (block, block_node(ir, block, profile, taken)))
        .collect();
    let loops = Loops::of(ir, &blocks);
    let mut children = loops.level(ir, &blocks, None, &mut block_nodes);
    children.extend(own);
    FlameNode::new(format!("@{symbol}"), Kind::Function, line, children)
}

fn block_node(ir: &Ir, block: u32, profile: &LineProfile, taken: &mut HashSet<u32>) -> FlameNode {
    let info = &ir.model().blocks[block as usize];
    let ops: Vec<FlameNode> = info
        .ops
        .iter()
        .map(|&op| op_node(ir, op, profile, taken))
        .collect();
    let executions = ops
        .iter()
        .filter_map(|op| op.executions)
        .fold(None, |max: Option<f64>, e| {
            Some(max.map_or(e, |m| m.max(e)))
        });
    let line = info
        .pos
        .as_ref()
        .map(|pos| pos.line)
        .or_else(|| ops.first().and_then(|op| op.line));
    let mut node = FlameNode::new(ir.block_label(block), Kind::Block, line, ops);
    node.executions = executions;
    node
}

fn op_node(ir: &Ir, op: u32, profile: &LineProfile, taken: &mut HashSet<u32>) -> FlameNode {
    let model = ir.model();
    let line = ir.op_line(op);
    let metrics = take(profile, taken, line);
    let mut children = metrics.map(|m| decisions(m, line)).unwrap_or_default();
    // Ops nested in the op's regions.
    for region in &model.ops[op as usize].regions {
        for block in &model.regions[*region as usize].blocks {
            for inner in &model.blocks[*block as usize].ops {
                children.push(op_node(ir, *inner, profile, taken));
            }
        }
    }
    let own = metrics.and_then(|m| m.get(CYCLES)).copied().unwrap_or(0.0);
    let text = without_marker(&ir.op_first_line(op));
    let name = op_name(
        &text,
        &model.ops[op as usize].opid,
        !model.ops[op as usize].results.is_empty(),
    );
    let children: Vec<FlameNode> = children.into_iter().filter(|c| c.cycles > 0.0).collect();
    FlameNode {
        cycles: own + children.iter().map(|c| c.cycles).sum::<f64>(),
        executions: metrics.and_then(|m| m.get(EXECUTIONS)).copied(),
        text: (name != text).then_some(text),
        name,
        kind: Kind::Op,
        line,
        children,
    }
}

/// An op's box name: `v1 = llvm.mul` for an op with results, else its
/// first line, cut short.
fn op_name(text: &str, opid: &str, has_results: bool) -> String {
    if has_results && let Some((results, _)) = text.split_once(" = ") {
        return format!("{results} = {opid}");
    }
    const LONGEST: usize = 64;
    match text.char_indices().nth(LONGEST) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

/// `text` without a trailing location marker (` !12`, ` !12;`) or `;`.
fn without_marker(text: &str) -> String {
    let body = text.trim_end_matches(';').trim_end();
    match body.rsplit_once(" !") {
        Some((head, digits))
            if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) =>
        {
            head.trim_end().to_string()
        }
        _ => body.to_string(),
    }
}

/// The natural loops of a function's blocks.
struct Loops {
    loops: Vec<Loop>,
    /// The innermost loop of each block in one.
    innermost: HashMap<u32, usize>,
}

struct Loop {
    header: u32,
    blocks: HashSet<u32>,
    parent: Option<usize>,
}

impl Loops {
    fn of(ir: &Ir, blocks: &[u32]) -> Loops {
        let model = ir.model();
        let index: HashMap<u32, usize> = blocks.iter().enumerate().map(|(i, &b)| (b, i)).collect();
        let successors: Vec<Vec<usize>> = blocks
            .iter()
            .map(|&block| {
                let mut out: Vec<usize> = model.blocks[block as usize]
                    .ops
                    .iter()
                    .flat_map(|&op| model.ops[op as usize].successors.iter())
                    .filter_map(|succ| index.get(succ).copied())
                    .collect();
                out.dedup();
                out
            })
            .collect();
        let mut predecessors = vec![Vec::new(); blocks.len()];
        for (from, succs) in successors.iter().enumerate() {
            for &to in succs {
                predecessors[to].push(from);
            }
        }
        let idom = dominators(&successors, &predecessors);
        let dominates = |a: usize, mut b: usize| loop {
            if a == b {
                return true;
            }
            match idom[b] {
                Some(parent) if parent != b => b = parent,
                _ => return false,
            }
        };

        // Natural loops, one per header: the header and every block that
        // reaches a back edge's source without passing the header.
        let mut by_header: BTreeMap<usize, HashSet<usize>> = BTreeMap::new();
        for (from, succs) in successors.iter().enumerate() {
            for &header in succs {
                if idom[from].is_none() || !dominates(header, from) {
                    continue;
                }
                let body = by_header
                    .entry(header)
                    .or_insert_with(|| HashSet::from([header]));
                let mut work = vec![from];
                while let Some(block) = work.pop() {
                    if body.insert(block) {
                        work.extend(predecessors[block].iter().copied());
                    }
                }
            }
        }
        let mut loops: Vec<Loop> = by_header
            .into_iter()
            .map(|(header, body)| Loop {
                header: blocks[header],
                blocks: body.into_iter().map(|i| blocks[i]).collect(),
                parent: None,
            })
            .collect();
        // Innermost first: a loop's parent is the smallest other loop
        // holding its header.
        let mut order: Vec<usize> = (0..loops.len()).collect();
        order.sort_by_key(|&l| loops[l].blocks.len());
        for (k, &l) in order.iter().enumerate() {
            loops[l].parent = order[k + 1..]
                .iter()
                .copied()
                .find(|&outer| loops[outer].blocks.contains(&loops[l].header));
        }
        let mut innermost = HashMap::new();
        for &l in &order {
            for &block in &loops[l].blocks {
                innermost.entry(block).or_insert(l);
            }
        }
        Loops { loops, innermost }
    }

    /// The boxes of one level (`None`: the function), in program order: a
    /// loop where its header is, a block of the level where it is.
    fn level(
        &self,
        ir: &Ir,
        blocks: &[u32],
        level: Option<usize>,
        block_nodes: &mut HashMap<u32, FlameNode>,
    ) -> Vec<FlameNode> {
        let mut out = Vec::new();
        for &block in blocks {
            if let Some(l) = self
                .loops
                .iter()
                .position(|candidate| candidate.header == block && candidate.parent == level)
            {
                // A loop runs as often as its header.
                let header = block_nodes.get(&block);
                let (runs, line) = (
                    header.and_then(|b| b.executions),
                    header.and_then(|b| b.line),
                );
                let children = self.level(ir, blocks, Some(l), block_nodes);
                let mut node = FlameNode::new(
                    format!("loop {}", ir.block_label(block)),
                    Kind::Loop,
                    line,
                    children,
                );
                node.executions = runs;
                out.push(node);
            } else if self.innermost.get(&block).copied() == level
                && let Some(node) = block_nodes.remove(&block)
            {
                out.push(node);
            }
        }
        out
    }
}

/// Immediate dominators from block 0 (Cooper, Harvey and Kennedy), by
/// block index; `None` for a block block 0 does not reach.
fn dominators(successors: &[Vec<usize>], predecessors: &[Vec<usize>]) -> Vec<Option<usize>> {
    let n = successors.len();
    if n == 0 {
        return Vec::new();
    }
    // Reverse postorder from block 0.
    let mut postorder = Vec::with_capacity(n);
    let mut seen = vec![false; n];
    let mut stack = vec![(0usize, 0usize)];
    seen[0] = true;
    while let Some((block, next)) = stack.pop() {
        if let Some(&succ) = successors[block].get(next) {
            stack.push((block, next + 1));
            if !seen[succ] {
                seen[succ] = true;
                stack.push((succ, 0));
            }
        } else {
            postorder.push(block);
        }
    }
    let mut rank = vec![usize::MAX; n];
    for (i, &block) in postorder.iter().enumerate() {
        rank[block] = i;
    }
    let mut idom: Vec<Option<usize>> = vec![None; n];
    idom[0] = Some(0);
    let intersect = |idom: &[Option<usize>], mut a: usize, mut b: usize| {
        while a != b {
            while rank[a] < rank[b] {
                a = idom[a].expect("processed");
            }
            while rank[b] < rank[a] {
                b = idom[b].expect("processed");
            }
        }
        a
    };
    let mut changed = true;
    while changed {
        changed = false;
        for &block in postorder.iter().rev().skip(1) {
            let mut new = None;
            for &pred in &predecessors[block] {
                if idom[pred].is_none() {
                    continue;
                }
                new = Some(match new {
                    None => pred,
                    Some(current) => intersect(&idom, pred, current),
                });
            }
            if new.is_some() && idom[block] != new {
                idom[block] = new;
                changed = true;
            }
        }
    }
    idom
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dominators_of_a_loop_in_a_diamond() {
        // 0 → 1 → {2, 3} → 4 → 1 (back edge), 4 → 5
        let successors = vec![vec![1], vec![2, 3], vec![4], vec![4], vec![1, 5], vec![]];
        let mut predecessors = vec![Vec::new(); successors.len()];
        for (from, succs) in successors.iter().enumerate() {
            for &to in succs {
                predecessors[to].push(from);
            }
        }
        let idom = dominators(&successors, &predecessors);
        assert_eq!(
            idom,
            vec![Some(0), Some(0), Some(1), Some(1), Some(1), Some(4)]
        );
    }

    #[test]
    fn markers_and_long_lines_in_names() {
        assert_eq!(
            without_marker("v1 = llvm.add v2, v3 !12;"),
            "v1 = llvm.add v2, v3"
        );
        assert_eq!(
            op_name("v1 = llvm.add v2, v3", "llvm.add", true),
            "v1 = llvm.add"
        );
        assert_eq!(op_name("aarch64.ret", "aarch64.ret", false), "aarch64.ret");
    }
}
