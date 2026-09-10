//! Semantic CFG render documents for display tooling.

use std::collections::{BTreeMap, HashMap, HashSet};

use flowblocks::{Cfg, ControlEdgeId, EdgeKind as FlowEdgeKind};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    FunctionLikeInterface,
    common_traits::Named,
    context::{Context, Ptr},
    ir::{basic_block::BasicBlock, op::op_cast, operation::Operation, region::Region},
    linked_list::ContainsLinkedList,
    printable::Printable,
};

pub const RENDER_PROTOCOL_VERSION: u32 = 1;

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
    pub graph: Option<GraphDocument>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tree: Option<serde_json::Value>,
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
pub struct GraphDocument {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<GraphLayout>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    pub id: String,
    pub entity_id: String,
    pub kind: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: BTreeMap<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: BTreeMap<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GraphLayout {
    pub engine: String,
    pub width: f32,
    pub height: f32,
    pub nodes: Vec<GraphNodeLayout>,
    pub edges: Vec<GraphEdgeLayout>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GraphNodeLayout {
    pub node_id: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub rank: usize,
    pub column: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdgeLayout {
    pub edge_id: String,
    pub points: Vec<GraphPoint>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct GraphPoint {
    pub x: f32,
    pub y: f32,
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

struct FunctionRoot {
    entity_id: String,
    op: Ptr<Operation>,
    body: Ptr<Region>,
    label: String,
    detail: String,
}

struct PendingCfgEdge {
    from: Ptr<BasicBlock>,
    to: Ptr<BasicBlock>,
    label: Option<String>,
    terminator_opid: String,
    successor_index: usize,
}

struct CfgBuilder<'a> {
    ctx: &'a Context,
    entities: Vec<Entity>,
    seen_entities: HashSet<String>,
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    block_nodes: HashMap<Ptr<BasicBlock>, String>,
    block_order: HashMap<Ptr<BasicBlock>, usize>,
    pending_edges: Vec<PendingCfgEdge>,
}

const BLOCK_WIDTH: f32 = 520.0;
const BLOCK_MIN_HEIGHT: f32 = 76.0;
const BLOCK_MAX_HEIGHT: f32 = 360.0;
const GROUP_WIDTH: f32 = 560.0;
const GROUP_MIN_HEIGHT: f32 = 190.0;
const GRAPH_MARGIN: f32 = 48.0;

impl<'a> CfgBuilder<'a> {
    fn new(ctx: &'a Context, entities: Vec<Entity>) -> Self {
        let seen_entities = entities.iter().map(|entity| entity.id.clone()).collect();
        Self {
            ctx,
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
        region: Ptr<Region>,
        parent_id: Option<String>,
        depth: usize,
    ) -> Vec<String> {
        region
            .deref(self.ctx)
            .iter(self.ctx)
            .map(|block| self.block(block, parent_id.clone(), depth))
            .collect()
    }

    fn block(&mut self, block: Ptr<BasicBlock>, parent_id: Option<String>, depth: usize) -> String {
        if let Some(id) = self.block_nodes.get(&block) {
            return id.clone();
        }

        let (label, args, ops, succ_count) = {
            let block_ref = block.deref(self.ctx);
            (
                format!("^{}", block_ref.unique_name(self.ctx)),
                block_ref
                    .arguments()
                    .map(|arg| arg.disp(self.ctx).to_string())
                    .collect::<Vec<_>>(),
                block_ref.iter(self.ctx).collect::<Vec<_>>(),
                block_ref.succs(self.ctx).len(),
            )
        };

        let mut operation_text = Vec::new();
        let mut nested_ops = Vec::new();
        for op in ops {
            let (opid, successors, has_regions) = {
                let op_ref = op.deref(self.ctx);
                (
                    Operation::get_opid(op, self.ctx).disp(self.ctx).to_string(),
                    op_ref.successors().collect::<Vec<_>>(),
                    op_has_non_empty_regions(self.ctx, op),
                )
            };

            for (successor_index, succ) in successors.iter().copied().enumerate() {
                self.pending_edges.push(PendingCfgEdge {
                    from: block,
                    to: succ,
                    label: edge_label(&opid, successors.len(), successor_index),
                    terminator_opid: opid.clone(),
                    successor_index,
                });
            }

            if has_regions {
                operation_text.push(format!("{opid} <nested region>"));
                nested_ops.push(op);
            } else {
                operation_text.extend(print_inline_op(self.ctx, op));
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

    fn nested_op(&mut self, op: Ptr<Operation>, parent_id: String, depth: usize) -> String {
        let (opid, regions, first_line) = {
            let op_ref = op.deref(self.ctx);
            (
                Operation::get_opid(op, self.ctx).disp(self.ctx).to_string(),
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
        let non_empty_regions = regions
            .iter()
            .copied()
            .filter(|region| region_has_blocks(self.ctx, *region))
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
        region: Ptr<Region>,
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

pub fn build_cfg_render_document(
    ctx: &Context,
    root: Ptr<Operation>,
    snapshot_id: impl Into<String>,
    requested_root_id: Option<&str>,
) -> RenderDocument {
    let snapshot_id = snapshot_id.into();
    let function_roots = discover_function_roots(ctx, root);
    if function_roots.is_empty() {
        return diagnostic_document(
            "cfg",
            snapshot_id,
            "CFG render",
            "no-function-roots",
            "No operations implementing FunctionLikeInterface with a body were found.",
        );
    }

    let selected_index = requested_root_id
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
        attrs.insert("rootId".to_string(), json!(root.entity_id));
        attrs.insert("selected".to_string(), json!(index == selected_index));
        attrs.insert("opId".to_string(), json!(root_opid(ctx, root.op)));
        root_entities.push(Entity {
            id: root.entity_id.clone(),
            kind: EntityKind::Operation,
            label: root.label.clone(),
            detail: Some(root.detail.clone()),
            interfaces: vec!["FunctionLikeInterface".to_string()],
            source_span: None,
            attrs,
        });
    }

    let mut builder = CfgBuilder::new(ctx, root_entities);
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

fn discover_function_roots(ctx: &Context, root: Ptr<Operation>) -> Vec<FunctionRoot> {
    let mut roots = Vec::new();
    collect_function_roots(ctx, root, &mut roots);
    roots
}

fn collect_function_roots(ctx: &Context, op: Ptr<Operation>, roots: &mut Vec<FunctionRoot>) {
    {
        let op_obj = Operation::get_op_dyn(op, ctx);
        // Preferred: an explicit FunctionLikeInterface impl. Fallback: any
        // symbol-carrying op with a non-empty region — dialects (machine
        // IR, imported MIR) whose crates cannot implement the foreign
        // trait (orphan rule) still get CFG/Tree roots this way.
        let root = if let Some(function_like) = op_cast::<dyn FunctionLikeInterface>(&*op_obj) {
            function_like.body_region(ctx).map(|body| (body, function_like.display_name(ctx)))
        } else if let Some(symbol) =
            op_cast::<dyn pliron::builtin::op_interfaces::SymbolOpInterface>(&*op_obj)
        {
            op.deref(ctx)
                .regions()
                .next()
                .map(|body| (body, format!("@{}", symbol.get_symbol_name(ctx))))
        } else {
            None
        };
        if let Some((body, label)) = root
            && region_has_blocks(ctx, body)
        {
            roots.push(FunctionRoot {
                entity_id: op_entity_id(op),
                op,
                body,
                label,
                detail: Operation::get_opid(op, ctx).disp(ctx).to_string(),
            });
        }
    }

    let regions = op.deref(ctx).regions().collect::<Vec<_>>();
    for region in regions {
        let blocks = region.deref(ctx).iter(ctx).collect::<Vec<_>>();
        for block in blocks {
            let ops = block.deref(ctx).iter(ctx).collect::<Vec<_>>();
            for nested_op in ops {
                collect_function_roots(ctx, nested_op, roots);
            }
        }
    }
}

fn op_has_non_empty_regions(ctx: &Context, op: Ptr<Operation>) -> bool {
    op.deref(ctx)
        .regions()
        .any(|region| region_has_blocks(ctx, region))
}

fn region_has_blocks(ctx: &Context, region: Ptr<Region>) -> bool {
    region.deref(ctx).get_head().is_some()
}

fn root_opid(ctx: &Context, op: Ptr<Operation>) -> String {
    Operation::get_opid(op, ctx).disp(ctx).to_string()
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

fn print_inline_op(ctx: &Context, op: Ptr<Operation>) -> Vec<String> {
    op.disp(ctx)
        .to_string()
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
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
