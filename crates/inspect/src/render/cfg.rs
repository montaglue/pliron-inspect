//! The CFG view: the blocks of one function-like op, with nested regions
//! drawn inside the block of the op that holds them.

use std::collections::{BTreeMap, HashMap, HashSet};

use flowblocks::{Cfg, ControlEdgeId, EdgeKind as FlowEdgeKind};
use pliron_inspect_protocol::types::{
    Entity, EntityKind, GraphDocument, GraphEdge, GraphEdgeLayout, GraphLayout, GraphNode,
    GraphNodeLayout, GraphPoint, RenderDocument,
};
use pliron_lsp_protocol::op_traits;
use serde_json::json;

use super::{
    Ir, RENDER_PROTOCOL_VERSION, block_entity_id, diagnostic_document, op_entity_id,
    region_entity_id,
};

/// An op whose CFG can be shown: a symbol (`@f`) with a body, other than a
/// symbol table (a module).
struct FunctionRoot {
    entity_id: String,
    op: u32,
    body: u32,
    label: String,
}

struct PendingCfgEdge {
    from: u32,
    to: u32,
    label: Option<String>,
    terminator_opid: String,
    successor_index: usize,
}

struct CfgBuilder<'a> {
    ir: &'a Ir,
    entities: Vec<Entity>,
    seen_entities: HashSet<String>,
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    block_nodes: HashMap<u32, String>,
    block_order: HashMap<u32, usize>,
    pending_edges: Vec<PendingCfgEdge>,
}

const BLOCK_WIDTH: f32 = 520.0;
const BLOCK_MIN_HEIGHT: f32 = 76.0;
const BLOCK_MAX_HEIGHT: f32 = 360.0;
const GROUP_WIDTH: f32 = 560.0;
const GROUP_MIN_HEIGHT: f32 = 190.0;
const GRAPH_MARGIN: f32 = 48.0;

impl<'a> CfgBuilder<'a> {
    fn new(ir: &'a Ir, entities: Vec<Entity>) -> Self {
        let seen_entities = entities.iter().map(|entity| entity.id.clone()).collect();
        Self {
            ir,
            entities,
            seen_entities,
            nodes: Vec::new(),
            edges: Vec::new(),
            block_nodes: HashMap::new(),
            block_order: HashMap::new(),
            pending_edges: Vec::new(),
        }
    }

    fn finish(mut self, roots: Vec<String>) -> (Vec<Entity>, GraphDocument) {
        for pending in self.pending_edges {
            let Some(from) = self.block_nodes.get(&pending.from) else {
                continue;
            };
            let Some(to) = self.block_nodes.get(&pending.to) else {
                continue;
            };

            let mut attrs = BTreeMap::new();
            attrs.insert("successorIndex".to_string(), json!(pending.successor_index));
            attrs.insert("terminatorOpId".to_string(), json!(pending.terminator_opid));
            if let (Some(from_order), Some(to_order)) = (
                self.block_order.get(&pending.from),
                self.block_order.get(&pending.to),
            ) {
                attrs.insert("isBackedge".to_string(), json!(to_order <= from_order));
            }

            let edge_id = format!("edge:{}:{}:{}", from, to, pending.successor_index);
            self.edges.push(GraphEdge {
                id: edge_id,
                from: from.clone(),
                to: to.clone(),
                kind: "cfg".to_string(),
                label: pending.label,
                attrs,
            });
        }

        let layout = build_flowblocks_layout(&self.nodes, &self.edges);
        (
            self.entities,
            GraphDocument {
                nodes: self.nodes,
                edges: self.edges,
                roots,
                layout,
            },
        )
    }

    fn build_region(
        &mut self,
        region: u32,
        parent_id: Option<String>,
        depth: usize,
    ) -> Vec<String> {
        self.ir.model().regions[region as usize]
            .blocks
            .clone()
            .into_iter()
            .map(|block| self.block(block, parent_id.clone(), depth))
            .collect()
    }

    fn block(&mut self, block: u32, parent_id: Option<String>, depth: usize) -> String {
        if let Some(id) = self.block_nodes.get(&block) {
            return id.clone();
        }

        let info = &self.ir.model().blocks[block as usize];
        let label = self.ir.block_label(block);
        let args = info
            .args
            .iter()
            .map(|arg| self.ir.value_name(*arg))
            .collect::<Vec<_>>();
        let ops = info.ops.clone();
        let succ_count = ops
            .last()
            .map_or(0, |op| self.ir.model().ops[*op as usize].successors.len());

        let mut operation_text = Vec::new();
        let mut nested_ops = Vec::new();
        for op in ops {
            let opid = self.ir.opid(op).to_string();
            let successors = self.ir.model().ops[op as usize].successors.clone();
            for (successor_index, succ) in successors.iter().copied().enumerate() {
                self.pending_edges.push(PendingCfgEdge {
                    from: block,
                    to: succ,
                    label: edge_label(&opid, successors.len(), successor_index),
                    terminator_opid: opid.clone(),
                    successor_index,
                });
            }

            if self.ir.op_has_blocks(op) {
                operation_text.push(format!("{opid} <nested region>"));
                nested_ops.push(op);
            } else {
                operation_text.extend(self.ir.op_lines(op));
            }
        }

        let entity_id = block_entity_id(block);
        let node_id = format!("node:{entity_id}");
        self.block_nodes.insert(block, node_id.clone());
        self.block_order.insert(block, self.block_order.len());
        self.push_entity(Entity {
            id: entity_id.clone(),
            kind: EntityKind::Block,
            label: label.clone(),
            detail: Some(format!("{succ_count} successor(s)")),
            interfaces: Vec::new(),
            source_span: None,
            attrs: {
                let mut attrs = BTreeMap::new();
                attrs.insert("blockArgs".to_string(), json!(args));
                attrs.insert("depth".to_string(), json!(depth));
                attrs
            },
        });

        let mut attrs = BTreeMap::new();
        attrs.insert("blockArgs".to_string(), json!(args));
        attrs.insert("operationText".to_string(), json!(operation_text));
        attrs.insert("successorCount".to_string(), json!(succ_count));
        attrs.insert("depth".to_string(), json!(depth));
        attrs.insert("isNested".to_string(), json!(parent_id.is_some()));

        self.nodes.push(GraphNode {
            id: node_id.clone(),
            entity_id,
            kind: "block".to_string(),
            label,
            detail: Some(format!("{succ_count} successor(s)")),
            parent_id: parent_id.clone(),
            attrs,
        });

        for op in nested_ops {
            self.nested_op(op, node_id.clone(), depth + 1);
        }

        node_id
    }

    fn nested_op(&mut self, op: u32, parent_id: String, depth: usize) -> String {
        let opid = self.ir.opid(op).to_string();
        let first_line = self.ir.op_first_line(op);

        let entity_id = op_entity_id(op);
        self.push_entity(Entity {
            id: entity_id.clone(),
            kind: EntityKind::Operation,
            label: opid.clone(),
            detail: Some(first_line.clone()),
            interfaces: Vec::new(),
            source_span: None,
            attrs: {
                let mut attrs = BTreeMap::new();
                attrs.insert("opId".to_string(), json!(opid));
                attrs.insert("depth".to_string(), json!(depth));
                attrs
            },
        });

        let node_id = format!("node:nested:{entity_id}");
        let non_empty_regions = self.ir.model().ops[op as usize]
            .regions
            .iter()
            .copied()
            .filter(|region| self.ir.region_has_blocks(*region))
            .collect::<Vec<_>>();

        let mut attrs = BTreeMap::new();
        attrs.insert("opId".to_string(), json!(opid));
        attrs.insert("operationText".to_string(), json!(first_line));
        attrs.insert(
            "nestedRegionCount".to_string(),
            json!(non_empty_regions.len()),
        );
        attrs.insert("depth".to_string(), json!(depth));
        attrs.insert("isNested".to_string(), json!(true));

        self.nodes.push(GraphNode {
            id: node_id.clone(),
            entity_id,
            kind: "nested-op".to_string(),
            label: "nested region".to_string(),
            detail: Some(format!("{} region(s)", non_empty_regions.len())),
            parent_id: Some(parent_id),
            attrs,
        });

        if non_empty_regions.len() == 1 {
            self.build_region(non_empty_regions[0], Some(node_id.clone()), depth + 1);
        } else {
            for (region_index, region) in non_empty_regions.into_iter().enumerate() {
                let region_node_id =
                    self.region_group(region, region_index, node_id.clone(), depth + 1);
                self.build_region(region, Some(region_node_id), depth + 2);
            }
        }

        node_id
    }

    fn region_group(
        &mut self,
        region: u32,
        region_index: usize,
        parent_id: String,
        depth: usize,
    ) -> String {
        let entity_id = region_entity_id(region);
        self.push_entity(Entity {
            id: entity_id.clone(),
            kind: EntityKind::Region,
            label: format!("region {region_index}"),
            detail: None,
            interfaces: Vec::new(),
            source_span: None,
            attrs: BTreeMap::new(),
        });

        let node_id = format!("node:{entity_id}");
        let mut attrs = BTreeMap::new();
        attrs.insert("regionIndex".to_string(), json!(region_index));
        attrs.insert("depth".to_string(), json!(depth));
        attrs.insert("isNested".to_string(), json!(true));

        self.nodes.push(GraphNode {
            id: node_id.clone(),
            entity_id,
            kind: "region".to_string(),
            label: format!("region {region_index}"),
            detail: None,
            parent_id: Some(parent_id),
            attrs,
        });
        node_id
    }

    fn push_entity(&mut self, entity: Entity) {
        if self.seen_entities.insert(entity.id.clone()) {
            self.entities.push(entity);
        }
    }
}

fn build_flowblocks_layout(nodes: &[GraphNode], edges: &[GraphEdge]) -> Option<GraphLayout> {
    let block_nodes = nodes
        .iter()
        .filter(|node| node.kind == "block")
        .collect::<Vec<_>>();
    if block_nodes.is_empty() {
        return None;
    }

    let mut cfg = Cfg::new();
    let mut block_ids = HashMap::with_capacity(block_nodes.len());
    let mut node_ids = HashMap::with_capacity(block_nodes.len());
    for node in block_nodes {
        let (width, height) = graph_node_size(node);
        let block_id = cfg.add_node(width, height).ok()?;
        block_ids.insert(node.id.clone(), block_id);
        node_ids.insert(block_id, node.id.clone());
    }

    let outgoing_counts = cfg_outgoing_counts(edges);
    let mut edge_ids = HashMap::<ControlEdgeId, String>::new();
    for edge in edges {
        let (Some(from), Some(to)) = (block_ids.get(&edge.from), block_ids.get(&edge.to)) else {
            continue;
        };
        let edge_id = cfg
            .add_edge(
                *from,
                *to,
                flow_edge_kind(edge, outgoing_counts.get(&edge.from).copied().unwrap_or(0)),
            )
            .ok()?;
        edge_ids.insert(edge_id, edge.id.clone());
    }

    let layout = cfg.layout().ok()?;
    let edge_polylines = layout
        .edges
        .iter()
        .map(|edge| (edge.id, edge.polyline()))
        .collect::<Vec<_>>();
    let bounds = layout_bounds(&layout.blocks, &edge_polylines);
    let offset_x = GRAPH_MARGIN - bounds.min_x;
    let offset_y = GRAPH_MARGIN - bounds.min_y;

    let mut layout_nodes = Vec::with_capacity(layout.blocks.len());
    for block in &layout.blocks {
        let node_id = node_ids.get(&block.id)?.clone();
        layout_nodes.push(GraphNodeLayout {
            node_id,
            x: block.top_left.x + offset_x,
            y: block.top_left.y + offset_y,
            width: block.size.width,
            height: block.size.height,
            rank: block.rank,
            column: block.column,
        });
    }

    let mut layout_edges = Vec::with_capacity(layout.edges.len());
    for (flow_edge_id, points) in edge_polylines {
        let edge_id = edge_ids.get(&flow_edge_id)?.clone();
        layout_edges.push(GraphEdgeLayout {
            edge_id,
            points: points
                .into_iter()
                .map(|point| GraphPoint {
                    x: point.x + offset_x,
                    y: point.y + offset_y,
                })
                .collect(),
        });
    }

    Some(GraphLayout {
        engine: "flowblocks".to_string(),
        width: bounds.width() + GRAPH_MARGIN * 2.0,
        height: bounds.height() + GRAPH_MARGIN * 2.0,
        nodes: layout_nodes,
        edges: layout_edges,
    })
}

#[derive(Clone, Copy)]
struct LayoutBounds {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
}

impl LayoutBounds {
    fn empty() -> Self {
        Self {
            min_x: f32::INFINITY,
            min_y: f32::INFINITY,
            max_x: f32::NEG_INFINITY,
            max_y: f32::NEG_INFINITY,
        }
    }

    fn include(&mut self, x: f32, y: f32) {
        self.min_x = self.min_x.min(x);
        self.min_y = self.min_y.min(y);
        self.max_x = self.max_x.max(x);
        self.max_y = self.max_y.max(y);
    }

    fn normalized(self) -> Self {
        if self.min_x.is_finite()
            && self.min_y.is_finite()
            && self.max_x.is_finite()
            && self.max_y.is_finite()
        {
            self
        } else {
            Self {
                min_x: 0.0,
                min_y: 0.0,
                max_x: 0.0,
                max_y: 0.0,
            }
        }
    }

    fn width(self) -> f32 {
        self.max_x - self.min_x
    }

    fn height(self) -> f32 {
        self.max_y - self.min_y
    }
}

fn layout_bounds(
    blocks: &[flowblocks::LayoutBlock],
    edges: &[(ControlEdgeId, Vec<flowblocks::Point>)],
) -> LayoutBounds {
    let mut bounds = LayoutBounds::empty();
    for block in blocks {
        bounds.include(block.top_left.x, block.top_left.y);
        bounds.include(
            block.top_left.x + block.size.width,
            block.top_left.y + block.size.height,
        );
    }
    for (_, points) in edges {
        for point in points {
            bounds.include(point.x, point.y);
        }
    }
    bounds.normalized()
}

fn cfg_outgoing_counts(edges: &[GraphEdge]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for edge in edges {
        *counts.entry(edge.from.clone()).or_insert(0) += 1;
    }
    counts
}

fn flow_edge_kind(edge: &GraphEdge, outgoing_count: usize) -> FlowEdgeKind {
    match edge.label.as_deref() {
        Some("true") => return FlowEdgeKind::True,
        Some("false") => return FlowEdgeKind::False,
        _ => {}
    }

    if outgoing_count == 2 {
        match edge
            .attrs
            .get("successorIndex")
            .and_then(serde_json::Value::as_u64)
        {
            Some(0) => FlowEdgeKind::True,
            Some(1) => FlowEdgeKind::False,
            _ => FlowEdgeKind::Default,
        }
    } else {
        FlowEdgeKind::Default
    }
}

fn graph_node_size(node: &GraphNode) -> (f32, f32) {
    if node.kind != "block" {
        return (GROUP_WIDTH, GROUP_MIN_HEIGHT);
    }

    let operation_count = node
        .attrs
        .get("operationText")
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len);
    let arg_count = node
        .attrs
        .get("blockArgs")
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len);
    let height = (50.0 + operation_count as f32 * 16.0 + if arg_count > 0 { 28.0 } else { 0.0 })
        .clamp(BLOCK_MIN_HEIGHT, BLOCK_MAX_HEIGHT);

    (BLOCK_WIDTH, height)
}

/// The CFG of the function `requested_root` (an entity id from an earlier
/// document, or a label like `@f`), else of the first function.
pub fn cfg_document(
    ir: &Ir,
    snapshot_id: impl Into<String>,
    requested_root: Option<&str>,
) -> RenderDocument {
    let snapshot_id = snapshot_id.into();
    let function_roots = discover_function_roots(ir);
    if function_roots.is_empty() {
        return diagnostic_document(
            "cfg",
            snapshot_id,
            "CFG",
            "no-function-roots",
            "No symbol op with a body (such as a function) was found.",
        );
    }

    let selected_index = requested_root
        .and_then(|root_id| {
            function_roots
                .iter()
                .position(|root| root.entity_id == root_id || root.label == root_id)
        })
        .unwrap_or(0);
    let selected = &function_roots[selected_index];

    let mut root_entities = Vec::new();
    for (index, root) in function_roots.iter().enumerate() {
        let mut attrs = BTreeMap::new();
        attrs.insert("cfgRoot".to_string(), json!(true));
        attrs.insert("rootId".to_string(), json!(root.entity_id));
        attrs.insert("selected".to_string(), json!(index == selected_index));
        attrs.insert("opId".to_string(), json!(ir.opid(root.op)));
        root_entities.push(Entity {
            id: root.entity_id.clone(),
            kind: EntityKind::Operation,
            label: root.label.clone(),
            detail: Some(ir.opid(root.op).to_string()),
            interfaces: Vec::new(),
            source_span: None,
            attrs,
        });
    }

    let mut builder = CfgBuilder::new(ir, root_entities);
    let root_node_ids = builder.build_region(selected.body, None, 0);
    let (entities, graph) = builder.finish(root_node_ids);

    RenderDocument {
        version: RENDER_PROTOCOL_VERSION,
        view: "cfg".to_string(),
        title: Some(format!("CFG: {}", selected.label)),
        snapshot_id,
        entities,
        graph: Some(graph),
        tree: None,
        diagnostics: Vec::new(),
    }
}

/// Function roots in document order. Their ids are their symbols, so the
/// selected function stays selected across the snapshots of a trace.
fn discover_function_roots(ir: &Ir) -> Vec<FunctionRoot> {
    let mut roots = Vec::new();
    let mut seen = HashMap::<String, usize>::new();
    let mut stack = vec![ir.model().root];
    while let Some(op) = stack.pop() {
        let info = &ir.model().ops[op as usize];
        if let (Some(symbol), Some(&body)) = (&info.symbol, info.regions.first())
            && info.traits & op_traits::SYMBOL_TABLE == 0
            && ir.region_has_blocks(body)
        {
            let label = format!("@{symbol}");
            let count = seen.entry(label.clone()).or_default();
            *count += 1;
            let entity_id = match *count {
                1 => format!("fn:{label}"),
                n => format!("fn:{label}#{n}"),
            };
            roots.push(FunctionRoot {
                entity_id,
                op,
                body,
                label,
            });
        }
        // Children in reverse, so they pop in document order.
        for region in info.regions.iter().rev() {
            for block in ir.model().regions[*region as usize].blocks.iter().rev() {
                stack.extend(ir.model().blocks[*block as usize].ops.iter().rev());
            }
        }
    }
    roots
}

fn edge_label(opid: &str, successor_count: usize, successor_index: usize) -> Option<String> {
    if successor_count > 1 || successors_label_is_ambiguous(opid) {
        Some(format!("#{successor_index}"))
    } else {
        None
    }
}

fn successors_label_is_ambiguous(opid: &str) -> bool {
    opid.contains("cond") || opid.contains("switch")
}

#[cfg(test)]
mod tests {
    use super::super::tests::diamond;
    use super::*;

    #[test]
    fn diamond_cfg_of_the_function_not_the_module() {
        let doc = cfg_document(&diamond(), "s", None);
        assert!(doc.diagnostics.is_empty(), "{:?}", doc.diagnostics);
        assert_eq!(doc.title.as_deref(), Some("CFG: @f"));
        let roots: Vec<_> = doc
            .entities
            .iter()
            .filter(|e| e.attrs.contains_key("cfgRoot"))
            .map(|e| e.id.as_str())
            .collect();
        assert_eq!(roots, ["fn:@f"]);

        let graph = doc.graph.unwrap();
        let labels: Vec<_> = graph.nodes.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(labels, ["^entry", "^a", "^b", "^exit"]);
        assert_eq!(graph.nodes[0].attrs["blockArgs"], json!(["x"]));
        assert_eq!(
            graph.nodes[0].attrs["operationText"],
            json!(["llvm.cond_br if x ^a() else ^b()"])
        );
        let edges: Vec<_> = graph
            .edges
            .iter()
            .map(|e| (e.from.as_str(), e.to.as_str(), e.label.as_deref()))
            .collect();
        assert_eq!(
            edges,
            [
                ("node:block:1", "node:block:2", Some("#0")),
                ("node:block:1", "node:block:3", Some("#1")),
                ("node:block:2", "node:block:4", None),
                ("node:block:3", "node:block:4", None),
            ]
        );
        assert!(graph.layout.is_some());
    }

    #[test]
    fn a_function_is_selected_by_its_symbol() {
        let doc = cfg_document(&diamond(), "s", Some("fn:@f"));
        assert_eq!(doc.title.as_deref(), Some("CFG: @f"));
        let doc = cfg_document(&diamond(), "s", Some("@f"));
        assert_eq!(doc.title.as_deref(), Some("CFG: @f"));
    }
}
