import { useMemo, useState } from "react";
import {
  Background,
  BaseEdge,
  Controls,
  Handle,
  MarkerType,
  Position,
  ReactFlow,
  type Edge,
  type EdgeProps,
  type Node,
  type NodeProps
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";

import type { Entity, GraphEdge, GraphLayout, GraphNode, GraphPoint, RenderDocument } from "../api/protocol";

type CfgViewProps = {
  document: RenderDocument | null;
  rootId?: string;
  onSelectRoot: (rootId: string) => void;
};

type CfgNodeData = {
  graphNode: GraphNode;
};

type CfgEdgeData = {
  graphEdge: GraphEdge;
  path?: string;
};

type FlowLayout = {
  nodes: Node<CfgNodeData>[];
  edges: Edge<CfgEdgeData>[];
};

const BLOCK_WIDTH = 520;
const GROUP_WIDTH = 560;
const BLOCK_MIN_HEIGHT = 76;
const BLOCK_MAX_HEIGHT = 360;
const GROUP_MIN_HEIGHT = 190;
const GRAPH_MARGIN = 48;

export function CfgView({ document, rootId, onSelectRoot }: CfgViewProps) {
  const [selected, setSelected] = useState<{ kind: "node"; id: string } | { kind: "edge"; id: string } | null>(null);

  const rootEntities = useMemo(() => functionRootEntities(document), [document]);
  const diagnostics = document?.diagnostics ?? [];
  const selectedRoot = rootId ?? rootEntities.find((entity) => entity.attrs?.selected === true)?.id ?? rootEntities[0]?.id;
  const graph = document?.graph;
  const layout = useMemo(
    () => (graph ? layoutGraph(graph.nodes, graph.edges, graph.layout) : { nodes: [], edges: [] }),
    [graph]
  );

  const selectedEntity =
    selected?.kind === "node"
      ? entityForNode(document, graph?.nodes.find((node) => node.id === selected.id))
      : undefined;
  const selectedEdge = selected?.kind === "edge" ? graph?.edges.find((edge) => edge.id === selected.id) : undefined;
  const selectedNode = selected?.kind === "node" ? graph?.nodes.find((node) => node.id === selected.id) : undefined;

  return (
    <section className="cfg-view">
      <div className="cfg-toolbar">
        <div className="cfg-title">
          <span>{document?.title ?? "CFG"}</span>
          <small>{graph?.nodes.length ?? 0} nodes</small>
        </div>
        {rootEntities.length > 1 ? (
          <label className="cfg-root-select">
            <span>Function</span>
            <select value={selectedRoot} onChange={(event) => onSelectRoot(event.currentTarget.value)}>
              {rootEntities.map((entity) => (
                <option key={entity.id} value={entity.id}>
                  {entity.label}
                </option>
              ))}
            </select>
          </label>
        ) : null}
      </div>

      {diagnostics.length > 0 && !graph ? (
        <div className="cfg-diagnostics">
          {diagnostics.map((diagnostic) => (
            <div className={`cfg-diagnostic ${diagnostic.severity}`} key={`${diagnostic.code}-${diagnostic.message}`}>
              <strong>{diagnostic.code}</strong>
              <span>{diagnostic.message}</span>
            </div>
          ))}
        </div>
      ) : (
        <div className="cfg-workspace">
          <div className="cfg-canvas">
            <ReactFlow
              nodes={layout.nodes}
              edges={layout.edges}
              nodeTypes={nodeTypes}
              edgeTypes={edgeTypes}
              fitView
              minZoom={0.08}
              maxZoom={1.8}
              nodesDraggable={false}
              onNodeClick={(_, node: Node<CfgNodeData>) => setSelected({ kind: "node", id: node.id })}
              onEdgeClick={(_, edge: Edge<CfgEdgeData>) => setSelected({ kind: "edge", id: edge.id })}
              onPaneClick={() => setSelected(null)}
            >
              <Background />
              <Controls />
            </ReactFlow>
          </div>
          <CfgInspector entity={selectedEntity} edge={selectedEdge} graphNode={selectedNode} />
        </div>
      )}
    </section>
  );
}

function CfgInspector({
  entity,
  edge,
  graphNode
}: {
  entity?: Entity;
  edge?: GraphEdge;
  graphNode?: GraphNode;
}) {
  if (!entity && !edge && !graphNode) {
    return (
      <aside className="cfg-inspector">
        <span className="cfg-inspector-empty">No selection</span>
      </aside>
    );
  }

  return (
    <aside className="cfg-inspector">
      <h2>{entity?.label ?? edge?.label ?? edge?.kind ?? "Selection"}</h2>
      {entity?.detail ? <p>{entity.detail}</p> : null}
      {graphNode?.detail ? <p>{graphNode.detail}</p> : null}
      <dl>
        {entity ? (
          <>
            <dt>Entity</dt>
            <dd>{entity.id}</dd>
            <dt>Kind</dt>
            <dd>{entity.kind}</dd>
          </>
        ) : null}
        {edge ? (
          <>
            <dt>Edge</dt>
            <dd>{edge.from} {"->"} {edge.to}</dd>
            <dt>Kind</dt>
            <dd>{edge.kind}</dd>
          </>
        ) : null}
      </dl>
    </aside>
  );
}

function CfgBlockNode({ data }: NodeProps<Node<CfgNodeData>>) {
  const attrs = data.graphNode.attrs ?? {};
  const args = Array.isArray(attrs.blockArgs) ? attrs.blockArgs.map(String) : [];
  const operationText = Array.isArray(attrs.operationText) ? attrs.operationText.map(String) : [];

  return (
    <div className="cfg-node cfg-block-node">
      <Handle type="target" position={Position.Top} />
      <div className="cfg-node-title">{data.graphNode.label}</div>
      {args.length > 0 ? (
        <div className="cfg-node-args">
          {args.map((arg, index) => (
            <span key={`${arg}-${index}`}>{arg}</span>
          ))}
        </div>
      ) : null}
      <div
        className="cfg-node-ops nowheel nodrag nopan"
        onMouseDown={(event) => event.stopPropagation()}
        onPointerDown={(event) => event.stopPropagation()}
      >
        {operationText.length === 0 ? <span className="cfg-muted">empty</span> : null}
        {operationText.map((line, index) => (
          <code key={`${line}-${index}`} title={line}>
            {line}
          </code>
        ))}
      </div>
      <Handle type="source" position={Position.Bottom} />
    </div>
  );
}

function CfgGroupNode({ data }: NodeProps<Node<CfgNodeData>>) {
  const attrs = data.graphNode.attrs ?? {};
  const operationText = typeof attrs.operationText === "string" ? attrs.operationText : data.graphNode.detail;

  return (
    <div className="cfg-node cfg-group-node">
      <div className="cfg-node-title">{data.graphNode.label}</div>
      {operationText ? <code>{operationText}</code> : null}
    </div>
  );
}

const nodeTypes = {
  cfgBlock: CfgBlockNode,
  cfgGroup: CfgGroupNode
};

function CfgRoutedEdge({
  id,
  data,
  markerEnd,
  interactionWidth,
  sourceX,
  sourceY,
  targetX,
  targetY,
  style
}: EdgeProps<Edge<CfgEdgeData>>) {
  const path = data?.path ?? fallbackCurvePath(sourceX, sourceY, targetX, targetY);
  return <BaseEdge id={id} path={path} markerEnd={markerEnd} interactionWidth={interactionWidth} style={style} />;
}

const edgeTypes = {
  cfgRouted: CfgRoutedEdge
};

function layoutGraph(graphNodes: GraphNode[], graphEdges: GraphEdge[], graphLayout?: GraphLayout): FlowLayout {
  const nodeLayouts = new Map(graphLayout?.nodes.map((node) => [node.nodeId, node]));
  const fallbackPositions = fallbackNodePositions(graphNodes, nodeLayouts, graphLayout?.height ?? 0);

  return {
    nodes: graphNodes.map((graphNode) => {
      const layout = nodeLayouts.get(graphNode.id);
      const size = layout ? { width: layout.width, height: layout.height } : nodeSize(graphNode);
      return {
        id: graphNode.id,
        type: graphNode.kind === "block" ? "cfgBlock" : "cfgGroup",
        position: layout ? { x: layout.x, y: layout.y } : fallbackPositions.get(graphNode.id) ?? { x: GRAPH_MARGIN, y: GRAPH_MARGIN },
        data: { graphNode },
        style: size,
        className: graphNode.kind === "block" ? "cfg-flow-node-block" : "cfg-flow-node-group"
      };
    }),
    edges: flowEdgesFromLayout(graphEdges, graphLayout)
  };
}

function flowEdgesFromLayout(graphEdges: GraphEdge[], graphLayout?: GraphLayout): Edge<CfgEdgeData>[] {
  const pathsByEdge = new Map(graphLayout?.edges.map((edge) => [edge.edgeId, polylinePath(edge.points)]));
  return graphEdges.map((edge) => ({
    id: edge.id,
    source: edge.from,
    target: edge.to,
    type: "cfgRouted",
    markerEnd: { type: MarkerType.ArrowClosed },
    data: {
      graphEdge: edge,
      path: pathsByEdge.get(edge.id)
    },
    interactionWidth: 18,
    zIndex: 0
  }));
}

function fallbackNodePositions(graphNodes: GraphNode[], laidOutNodes: ReadonlyMap<string, unknown>, yOffset: number) {
  const positions = new Map<string, { x: number; y: number }>();
  let fallbackIndex = 0;
  for (const node of graphNodes) {
    if (laidOutNodes.has(node.id)) {
      continue;
    }
    positions.set(node.id, {
      x: GRAPH_MARGIN + (fallbackIndex % 2) * (GROUP_WIDTH + 48),
      y: GRAPH_MARGIN + yOffset + Math.floor(fallbackIndex / 2) * (GROUP_MIN_HEIGHT + 48)
    });
    fallbackIndex += 1;
  }
  return positions;
}

function polylinePath(points: GraphPoint[]) {
  if (points.length === 0) {
    return undefined;
  }
  const [first, ...rest] = points;
  return [`M ${first.x} ${first.y}`, ...rest.map((point) => `L ${point.x} ${point.y}`)].join(" ");
}

function fallbackCurvePath(sourceX: number, sourceY: number, targetX: number, targetY: number) {
  const controlY = sourceY + Math.max(80, Math.abs(targetY - sourceY) / 2);
  return `M ${sourceX} ${sourceY} C ${sourceX} ${controlY}, ${targetX} ${controlY}, ${targetX} ${targetY}`;
}

function nodeSize(node: GraphNode) {
  if (node.kind !== "block") {
    return { width: GROUP_WIDTH, height: GROUP_MIN_HEIGHT };
  }
  const operationText = Array.isArray(node.attrs?.operationText) ? node.attrs.operationText : [];
  const args = Array.isArray(node.attrs?.blockArgs) ? node.attrs.blockArgs : [];
  return {
    width: BLOCK_WIDTH,
    height: Math.min(
      BLOCK_MAX_HEIGHT,
      Math.max(BLOCK_MIN_HEIGHT, 50 + operationText.length * 16 + (args.length > 0 ? 28 : 0))
    )
  };
}

function functionRootEntities(document: RenderDocument | null) {
  return (document?.entities ?? []).filter((entity) => entity.interfaces?.includes("FunctionLikeInterface"));
}

function entityForNode(document: RenderDocument | null, graphNode?: GraphNode) {
  if (!graphNode) return undefined;
  return document?.entities.find((entity) => entity.id === graphNode.entityId);
}
