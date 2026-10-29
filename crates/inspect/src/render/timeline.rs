//! A timeline of a profiled run: the cycle samples one by one, in time
//! order, as zones of what was running — function, then its loops from the
//! outermost in, then the block — one lane per level, the way a frame
//! profiler nests zones. Consecutive samples in the same place are one zone,
//! running until the first sample elsewhere. The samples name the ops they
//! hit by line, in one snapshot (the code as executed); the structure comes
//! from that snapshot's model, the loops as in the flamegraph.

use std::collections::HashMap;

use pliron_lsp_protocol::op_traits;
use serde::Serialize;

use super::Ir;
use super::flame::Loops;

/// The samples of a run: each one's time after the one before (ns), and
/// the line of the op it hit (0: code outside the module).
pub struct Samples<'a> {
    pub time: &'a [u64],
    pub line: &'a [u32],
}

#[derive(Debug, Serialize)]
pub struct Timeline {
    /// From the first sample to the end of the last (ns).
    pub duration: u64,
    pub samples: usize,
    /// One past the deepest lane.
    pub lanes: u32,
    /// What zones show: a function, loop or block of the snapshot.
    pub places: Vec<Place>,
    pub zones: Zones,
}

#[derive(Debug, Serialize)]
pub struct Place {
    pub name: String,
    pub kind: &'static str,
    /// The snapshot line (1-based) its IR starts on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

/// The zones, column by column: zone `i` is `place[i]`, on `lane[i]`, from
/// `start[i]` to `end[i]` (ns from the first sample), holding `samples[i]`.
#[derive(Debug, Default, Serialize)]
pub struct Zones {
    pub lane: Vec<u32>,
    pub start: Vec<u64>,
    pub end: Vec<u64>,
    pub place: Vec<u32>,
    pub samples: Vec<u32>,
}

/// Where each op line of the snapshot is: its places, outermost first.
struct Places {
    places: Vec<Place>,
    by_line: HashMap<u32, Vec<u32>>,
}

impl Places {
    fn add(&mut self, name: String, kind: &'static str, line: Option<u32>) -> u32 {
        self.places.push(Place { name, kind, line });
        self.places.len() as u32 - 1
    }

    fn of(ir: &Ir) -> Places {
        let model = ir.model();
        let mut places = Places {
            places: Vec::new(),
            by_line: HashMap::new(),
        };
        let mut stack = vec![model.root];
        while let Some(op) = stack.pop() {
            let info = &model.ops[op as usize];
            if let (Some(symbol), Some(&body)) = (&info.symbol, info.regions.first())
                && info.traits & op_traits::SYMBOL_TABLE == 0
                && ir.region_has_blocks(body)
            {
                places.function(ir, op, symbol, body);
                continue;
            }
            for region in info.regions.iter().rev() {
                for block in model.regions[*region as usize].blocks.iter().rev() {
                    stack.extend(model.blocks[*block as usize].ops.iter().rev());
                }
            }
        }
        places
    }

    fn function(&mut self, ir: &Ir, op: u32, symbol: &str, body: u32) {
        let model = ir.model();
        let function = self.add(format!("@{symbol}"), "function", ir.op_line(op));
        let blocks = model.regions[body as usize].blocks.clone();
        let loops = Loops::of(ir, &blocks);
        let loop_places: Vec<u32> = loops
            .loops
            .iter()
            .map(|l| {
                let header = &model.blocks[l.header as usize];
                let line = header.pos.as_ref().map(|pos| pos.line);
                self.add(format!("loop {}", ir.block_label(l.header)), "loop", line)
            })
            .collect();
        for &block in &blocks {
            let info = &model.blocks[block as usize];
            let line = info
                .pos
                .as_ref()
                .map(|pos| pos.line)
                .or_else(|| info.ops.first().and_then(|&op| ir.op_line(op)));
            let block_place = self.add(ir.block_label(block), "block", line);
            // function, loops from the outermost in, block
            let mut chain = Vec::new();
            let mut at = loops.innermost.get(&block).copied();
            while let Some(l) = at {
                chain.push(loop_places[l]);
                at = loops.loops[l].parent;
            }
            chain.push(function);
            chain.reverse();
            chain.push(block_place);
            let mut ops: Vec<u32> = info.ops.clone();
            while let Some(op) = ops.pop() {
                if let Some(line) = ir.op_line(op) {
                    self.by_line.entry(line).or_insert_with(|| chain.clone());
                }
                for region in &model.ops[op as usize].regions {
                    for inner in &model.regions[*region as usize].blocks {
                        ops.extend(model.blocks[*inner as usize].ops.iter().copied());
                    }
                }
            }
        }
    }
}

/// Close the open zones deeper than `depth`, ending at `end`.
fn close_from(open: &mut Vec<(u32, u64, u32)>, depth: usize, end: u64, zones: &mut Zones) {
    while open.len() > depth {
        let (place, start, count) = open.pop().expect("deeper than depth");
        zones.lane.push(open.len() as u32);
        zones.start.push(start);
        zones.end.push(end);
        zones.place.push(place);
        zones.samples.push(count);
    }
}

/// The timeline of `samples` on the snapshot `ir`.
pub fn timeline(ir: &Ir, samples: &Samples) -> Timeline {
    let mut places = Places::of(ir);
    let outside = places.add("outside the module".to_string(), "other", None);
    let outside_chain = vec![outside];

    let mut zones = Zones::default();
    // The open zones, outermost first: (place, start, samples).
    let mut open: Vec<(u32, u64, u32)> = Vec::new();
    let mut time = 0u64;
    let mut lanes = 0u32;
    for (i, (&delta, &line)) in samples.time.iter().zip(samples.line).enumerate() {
        if i > 0 {
            time += delta;
        }
        let chain = places.by_line.get(&line).unwrap_or(&outside_chain);
        lanes = lanes.max(chain.len() as u32);
        let same = open
            .iter()
            .zip(chain)
            .take_while(|((place, _, _), wanted)| place == *wanted)
            .count();
        close_from(&mut open, same, time, &mut zones);
        for (depth, &place) in chain.iter().enumerate() {
            if depth < same {
                open[depth].2 += 1;
            } else {
                open.push((place, time, 1));
            }
        }
    }
    // The last samples last as long as a typical gap.
    let mut gaps: Vec<u64> = samples.time.iter().skip(1).copied().collect();
    gaps.sort_unstable();
    let duration = time + gaps.get(gaps.len() / 2).copied().unwrap_or(0);
    close_from(&mut open, 0, duration, &mut zones);
    Timeline {
        duration,
        samples: samples.time.len(),
        lanes,
        places: places.places,
        zones,
    }
}
