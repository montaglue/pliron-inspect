//! The tree view: the op / region / block hierarchy of the whole document.

use std::collections::{BTreeMap, HashSet};

use pliron_inspect_protocol::types::{Entity, EntityKind, RenderDocument, TreeDocument, TreeNode};
use serde_json::json;

use super::{Ir, RENDER_PROTOCOL_VERSION, block_entity_id, op_entity_id, region_entity_id};

const DEFAULT_VISIBLE_NODES_PER_DEPTH: usize = 10;

struct TreeBuilder<'a> {
    ir: &'a Ir,
    entities: Vec<Entity>,
    seen_entities: HashSet<String>,
}

impl<'a> TreeBuilder<'a> {
    fn new(ir: &'a Ir) -> Self {
        Self {
            ir,
            entities: Vec::new(),
            seen_entities: HashSet::new(),
        }
    }

    fn finish(self, mut root: TreeNode) -> (Vec<Entity>, TreeDocument) {
        annotate_default_collapsed(&mut root);
        (self.entities, TreeDocument { root })
    }

    fn tree_node(
        &self,
        id: String,
        entity_id: String,
        label: String,
        detail: Option<String>,
        children: Vec<TreeNode>,
        mut attrs: BTreeMap<String, serde_json::Value>,
    ) -> TreeNode {
        attrs.insert("childCount".to_string(), json!(children.len()));

        TreeNode {
            id,
            entity_id,
            label,
            detail,
            children,
            attrs,
        }
    }

    fn operation(&mut self, op: u32) -> TreeNode {
        let info = &self.ir.model().ops[op as usize];
        let opid = self.ir.opid(op).to_string();
        let first_line = self.ir.op_first_line(op);
        let regions = info.regions.clone();

        let entity_id = op_entity_id(op);
        let mut attrs = BTreeMap::new();
        attrs.insert("opId".to_string(), json!(opid));
        attrs.insert("resultCount".to_string(), json!(info.results.len()));
        attrs.insert("operandCount".to_string(), json!(info.operands.len()));
        attrs.insert("successorCount".to_string(), json!(info.successors.len()));
        attrs.insert("regionCount".to_string(), json!(regions.len()));

        self.push_entity(Entity {
            id: entity_id.clone(),
            kind: EntityKind::Operation,
            label: opid.clone(),
            detail: Some(first_line.clone()),
            interfaces: Vec::new(),
            source_span: None,
            attrs: attrs.clone(),
        });

        let children = self.operation_children(regions);
        self.tree_node(
            format!("tree:{entity_id}"),
            entity_id,
            opid,
            Some(first_line),
            children,
            attrs,
        )
    }

    fn operation_children(&mut self, regions: Vec<u32>) -> Vec<TreeNode> {
        match regions.as_slice() {
            [] => Vec::new(),
            [region] => self.region_blocks(*region),
            _ => regions
                .into_iter()
                .enumerate()
                .map(|(region_index, region)| self.region(region, region_index))
                .collect(),
        }
    }

    fn region_blocks(&mut self, region: u32) -> Vec<TreeNode> {
        self.ir.model().regions[region as usize]
            .blocks
            .clone()
            .into_iter()
            .map(|block| self.block(block))
            .collect()
    }

    fn region(&mut self, region: u32, region_index: usize) -> TreeNode {
        let blocks = self.ir.model().regions[region as usize].blocks.clone();
        let entity_id = region_entity_id(region);
        let label = format!("region {region_index}");
        let detail = format!("{} block(s)", blocks.len());
        let mut attrs = BTreeMap::new();
        attrs.insert("regionIndex".to_string(), json!(region_index));
        attrs.insert("blockCount".to_string(), json!(blocks.len()));

        self.push_entity(Entity {
            id: entity_id.clone(),
            kind: EntityKind::Region,
            label: label.clone(),
            detail: Some(detail.clone()),
            interfaces: Vec::new(),
            source_span: None,
            attrs: attrs.clone(),
        });

        let children = blocks.into_iter().map(|block| self.block(block)).collect();
        self.tree_node(
            format!("tree:{entity_id}"),
            entity_id,
            label,
            Some(detail),
            children,
            attrs,
        )
    }

    fn block(&mut self, block: u32) -> TreeNode {
        let info = &self.ir.model().blocks[block as usize];
        let label = self.ir.block_label(block);
        let args = info.args.len();
        let ops = info.ops.clone();
        let succ_count = ops
            .last()
            .map_or(0, |op| self.ir.model().ops[*op as usize].successors.len());

        let entity_id = block_entity_id(block);
        let detail = format!(
            "{} arg(s), {} op(s), {} successor(s)",
            args,
            ops.len(),
            succ_count
        );
        let mut attrs = BTreeMap::new();
        attrs.insert("argumentCount".to_string(), json!(args));
        attrs.insert("operationCount".to_string(), json!(ops.len()));
        attrs.insert("successorCount".to_string(), json!(succ_count));

        self.push_entity(Entity {
            id: entity_id.clone(),
            kind: EntityKind::Block,
            label: label.clone(),
            detail: Some(detail.clone()),
            interfaces: Vec::new(),
            source_span: None,
            attrs: attrs.clone(),
        });

        let children = ops.into_iter().map(|op| self.operation(op)).collect();
        self.tree_node(
            format!("tree:{entity_id}"),
            entity_id,
            label,
            Some(detail),
            children,
            attrs,
        )
    }

    fn push_entity(&mut self, entity: Entity) {
        if self.seen_entities.insert(entity.id.clone()) {
            self.entities.push(entity);
        }
    }
}

fn annotate_default_collapsed(root: &mut TreeNode) {
    let mut visible_by_depth = BTreeMap::new();
    visible_by_depth.insert(0, 1);
    annotate_default_collapsed_rec(root, 0, &mut visible_by_depth);
}

fn annotate_default_collapsed_rec(
    node: &mut TreeNode,
    depth: usize,
    visible_by_depth: &mut BTreeMap<usize, usize>,
) {
    let child_count = node.children.len();
    if child_count == 0 {
        return;
    }

    let child_depth = depth + 1;
    let visible_at_child_depth = visible_by_depth.entry(child_depth).or_default();
    if *visible_at_child_depth + child_count > DEFAULT_VISIBLE_NODES_PER_DEPTH {
        node.attrs
            .insert("defaultCollapsed".to_string(), json!(true));
        return;
    }

    *visible_at_child_depth += child_count;
    for child in &mut node.children {
        annotate_default_collapsed_rec(child, child_depth, visible_by_depth);
    }
}

pub fn tree_document(ir: &Ir, snapshot_id: impl Into<String>) -> RenderDocument {
    let mut builder = TreeBuilder::new(ir);
    let root_node = builder.operation(ir.model().root);
    let title = format!("Tree: {}", root_node.label);
    let (entities, tree) = builder.finish(root_node);

    RenderDocument {
        version: RENDER_PROTOCOL_VERSION,
        view: "tree".to_string(),
        title: Some(title),
        snapshot_id: snapshot_id.into(),
        entities,
        graph: None,
        tree: Some(tree),
        diagnostics: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::diamond;
    use super::*;

    #[test]
    fn module_func_blocks_ops() {
        let doc = tree_document(&diamond(), "s");
        let root = &doc.tree.unwrap().root;
        assert_eq!(root.label, "builtin.module");
        assert_eq!(root.detail.as_deref(), Some("builtin.module @m {"));
        let func = &root.children[0].children[0];
        assert_eq!(func.label, "llvm.func");
        let blocks: Vec<_> = func.children.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(blocks, ["^entry", "^a", "^b", "^exit"]);
        assert_eq!(
            func.children[0].detail.as_deref(),
            Some("1 arg(s), 1 op(s), 2 successor(s)")
        );
    }
}
