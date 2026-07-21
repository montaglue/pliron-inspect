//! Semantic tree render documents for display tooling.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    common_traits::Named,
    context::{Context, Ptr},
    ir::{basic_block::BasicBlock, operation::Operation, region::Region},
    linked_list::ContainsLinkedList,
    printable::Printable,
};

pub const RENDER_PROTOCOL_VERSION: u32 = 1;
const DEFAULT_VISIBLE_NODES_PER_DEPTH: usize = 10;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RenderDocument {
    pub version: u32,
    pub view: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub snapshot_id: String,
    pub entities: Vec<Entity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tree: Option<TreeDocument>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Entity {
    pub id: String,
    pub kind: EntityKind,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interfaces: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_span: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: BTreeMap<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub enum EntityKind {
    Operation,
    Region,
    Block,
    Value,
    Type,
    Attribute,
    Interface,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TreeDocument {
    pub root: TreeNode,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TreeNode {
    pub id: String,
    pub entity_id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<TreeNode>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: BTreeMap<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: BTreeMap<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

struct TreeBuilder<'a> {
    ctx: &'a Context,
    entities: Vec<Entity>,
    seen_entities: HashSet<String>,
}

impl<'a> TreeBuilder<'a> {
    fn new(ctx: &'a Context) -> Self {
        Self {
            ctx,
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

    fn operation(&mut self, op: Ptr<Operation>) -> TreeNode {
        let (opid, results, operands, successors, regions, first_line) = {
            let op_ref = op.deref(self.ctx);
            (
                Operation::get_opid(op, self.ctx).disp(self.ctx).to_string(),
                op_ref.get_num_results(),
                op_ref.operands().count(),
                op_ref.successors().count(),
                op_ref.regions().collect::<Vec<_>>(),
                op.disp(self.ctx)
                    .to_string()
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
            )
        };

        let entity_id = op_entity_id(op);
        let mut attrs = BTreeMap::new();
        attrs.insert("opId".to_string(), json!(opid));
        attrs.insert("resultCount".to_string(), json!(results));
        attrs.insert("operandCount".to_string(), json!(operands));
        attrs.insert("successorCount".to_string(), json!(successors));
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

    fn operation_children(&mut self, regions: Vec<Ptr<Region>>) -> Vec<TreeNode> {
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

    fn region_blocks(&mut self, region: Ptr<Region>) -> Vec<TreeNode> {
        region
            .deref(self.ctx)
            .iter(self.ctx)
            .map(|block| self.block(block))
            .collect()
    }

    fn region(&mut self, region: Ptr<Region>, region_index: usize) -> TreeNode {
        let blocks = region.deref(self.ctx).iter(self.ctx).collect::<Vec<_>>();
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

    fn block(&mut self, block: Ptr<BasicBlock>) -> TreeNode {
        let (label, args, ops, succ_count) = {
            let block_ref = block.deref(self.ctx);
            (
                format!("^{}", block_ref.unique_name(self.ctx)),
                block_ref.arguments().count(),
                block_ref.iter(self.ctx).collect::<Vec<_>>(),
                block_ref.succs(self.ctx).len(),
            )
        };

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

pub fn build_tree_render_document(
    ctx: &Context,
    root: Ptr<Operation>,
    snapshot_id: impl Into<String>,
) -> RenderDocument {
    let mut builder = TreeBuilder::new(ctx);
    let root_node = builder.operation(root);
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
        text: None,
        diagnostics: Vec::new(),
    }
}

pub fn diagnostic_document(
    view: impl Into<String>,
    snapshot_id: impl Into<String>,
    title: impl Into<String>,
    code: impl Into<String>,
    message: impl Into<String>,
) -> RenderDocument {
    RenderDocument {
        version: RENDER_PROTOCOL_VERSION,
        view: view.into(),
        title: Some(title.into()),
        snapshot_id: snapshot_id.into(),
        entities: Vec::new(),
        graph: None,
        tree: None,
        text: None,
        diagnostics: vec![Diagnostic {
            severity: DiagnosticSeverity::Error,
            code: code.into(),
            message: message.into(),
            entity_id: None,
            attrs: BTreeMap::new(),
        }],
    }
}

fn op_entity_id(op: Ptr<Operation>) -> String {
    format!("op:{op:?}")
}

fn block_entity_id(block: Ptr<BasicBlock>) -> String {
    format!("block:{block:?}")
}

fn region_entity_id(region: Ptr<Region>) -> String {
    format!("region:{region:?}")
}
