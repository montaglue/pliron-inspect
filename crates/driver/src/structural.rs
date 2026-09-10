//! Serializable structural graph view of parsed crabbit IR.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    common_traits::Named,
    context::{Context, Ptr},
    ir::{basic_block::BasicBlock, operation::Operation},
    linked_list::ContainsLinkedList,
    printable::Printable,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrGraph {
    pub nodes: Vec<IrGraphNode>,
    pub edges: Vec<IrGraphEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrGraphNode {
    pub id: String,
    pub kind: IrGraphNodeKind,
    pub label: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IrGraphNodeKind {
    Operation,
    Region,
    Block,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrGraphEdge {
    pub from: String,
    pub to: String,
    pub kind: IrGraphEdgeKind,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IrGraphEdgeKind {
    Contains,
    Cfg,
}

struct PendingCfgEdge {
    from: Ptr<BasicBlock>,
    to: Ptr<BasicBlock>,
    label: String,
}

struct GraphBuilder<'a> {
    ctx: &'a Context,
    next_id: usize,
    nodes: Vec<IrGraphNode>,
    edges: Vec<IrGraphEdge>,
    block_ids: HashMap<Ptr<BasicBlock>, String>,
    pending_cfg_edges: Vec<PendingCfgEdge>,
}

impl<'a> GraphBuilder<'a> {
    fn new(ctx: &'a Context) -> Self {
        Self {
            ctx,
            next_id: 0,
            nodes: Vec::new(),
            edges: Vec::new(),
            block_ids: HashMap::new(),
            pending_cfg_edges: Vec::new(),
        }
    }

    fn finish(mut self) -> IrGraph {
        for edge in self.pending_cfg_edges {
            let Some(from) = self.block_ids.get(&edge.from) else {
                continue;
            };
            let Some(to) = self.block_ids.get(&edge.to) else {
                continue;
            };
            self.edges.push(IrGraphEdge {
                from: from.clone(),
                to: to.clone(),
                kind: IrGraphEdgeKind::Cfg,
                label: edge.label,
            });
        }

        IrGraph {
            nodes: self.nodes,
            edges: self.edges,
        }
    }

    fn node(&mut self, kind: IrGraphNodeKind, label: String, detail: String) -> String {
        let prefix = match kind {
            IrGraphNodeKind::Operation => "op",
            IrGraphNodeKind::Region => "region",
            IrGraphNodeKind::Block => "block",
        };
        let id = format!("{prefix}{}", self.next_id);
        self.next_id += 1;
        self.nodes.push(IrGraphNode {
            id: id.clone(),
            kind,
            label,
            detail,
        });
        id
    }

    fn contains_edge(&mut self, from: &str, to: &str, label: impl Into<String>) {
        self.edges.push(IrGraphEdge {
            from: from.to_string(),
            to: to.to_string(),
            kind: IrGraphEdgeKind::Contains,
            label: label.into(),
        });
    }

    fn operation(&mut self, op: Ptr<Operation>) -> String {
        let (label, detail, regions) = {
            let op_ref = op.deref(self.ctx);
            (
                Operation::get_opid(op, self.ctx).disp(self.ctx).to_string(),
                format!(
                    "{} result(s), {} operand(s), {} successor(s), {} region(s)",
                    op_ref.get_num_results(),
                    op_ref.get_num_operands(),
                    op_ref.get_num_successors(),
                    op_ref.num_regions()
                ),
                op_ref.regions().collect::<Vec<_>>(),
            )
        };

        let op_id = self.node(IrGraphNodeKind::Operation, label, detail);
        if regions.len() == 1 {
            for block_id in self.region_blocks(regions[0]) {
                self.contains_edge(&op_id, &block_id, "");
            }
        } else {
            for (index, region) in regions.into_iter().enumerate() {
                let region_id = self.region(region, index);
                self.contains_edge(&op_id, &region_id, format!("region {index}"));
            }
        }
        op_id
    }

    fn region(&mut self, region: Ptr<crate::ir::region::Region>, index: usize) -> String {
        let label = format!("region {index}");
        let block_ids = self.region_blocks(region);
        let detail = format!("{} block(s)", block_ids.len());
        let region_id = self.node(IrGraphNodeKind::Region, label, detail);

        for block_id in block_ids {
            self.contains_edge(&region_id, &block_id, "");
        }
        region_id
    }

    fn region_blocks(&mut self, region: Ptr<crate::ir::region::Region>) -> Vec<String> {
        region
            .deref(self.ctx)
            .iter(self.ctx)
            .map(|block| self.block(block))
            .collect()
    }

    fn block(&mut self, block: Ptr<BasicBlock>) -> String {
        if let Some(id) = self.block_ids.get(&block) {
            return id.clone();
        }

        let (label, mut detail_lines, ops) = {
            let block_ref = block.deref(self.ctx);
            (
                format!("^{}", block_ref.unique_name(self.ctx)),
                vec![format!(
                    "{} argument(s), {} successor(s)",
                    block_ref.get_num_arguments(),
                    block_ref.succs(self.ctx).len()
                )],
                block_ref.iter(self.ctx).collect::<Vec<_>>(),
            )
        };

        let mut nested_ops = Vec::new();
        for op in ops {
            let (successors, op_label) = {
                let op_ref = op.deref(self.ctx);
                (
                    op_ref.successors().collect::<Vec<_>>(),
                    Operation::get_opid(op, self.ctx).disp(self.ctx).to_string(),
                )
            };
            for (succ_index, succ) in successors.into_iter().enumerate() {
                let label = if successors_label_is_ambiguous(&op_label) {
                    format!("{op_label} #{succ_index}")
                } else {
                    op_label.clone()
                };
                self.pending_cfg_edges.push(PendingCfgEdge {
                    from: block,
                    to: succ,
                    label,
                });
            }
            if op_contains_blocks(self.ctx, op) {
                nested_ops.push(op);
            } else {
                detail_lines.extend(print_inline_op(self.ctx, op));
            }
        }

        let detail = detail_lines.join("\n");
        let block_id = self.node(IrGraphNodeKind::Block, label, detail);
        self.block_ids.insert(block, block_id.clone());

        for (op_index, op) in nested_ops.into_iter().enumerate() {
            let op_id = self.operation(op);
            self.contains_edge(&block_id, &op_id, format!("op {op_index}"));
        }

        block_id
    }
}

fn op_contains_blocks(ctx: &Context, op: Ptr<Operation>) -> bool {
    op.deref(ctx)
        .regions()
        .any(|region| region.deref(ctx).iter(ctx).next().is_some())
}

fn print_inline_op(ctx: &Context, op: Ptr<Operation>) -> Vec<String> {
    op.disp(ctx)
        .to_string()
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

fn successors_label_is_ambiguous(op_label: &str) -> bool {
    op_label.contains("cond_br") || op_label.contains("switch")
}

pub fn build_ir_graph(ctx: &Context, root: Ptr<Operation>) -> IrGraph {
    let mut builder = GraphBuilder::new(ctx);
    builder.operation(root);
    builder.finish()
}
