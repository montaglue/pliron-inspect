import { useEffect, useMemo, useState } from "react";
import { flextree } from "d3-flextree";

import type { Entity, RenderDocument, TreeNode } from "../api/protocol";

type TreeViewProps = {
  document: RenderDocument | null;
};

type TreeLayoutNode = {
  data: TreeNode;
  x: number;
  y: number;
  depth: number;
  parent: TreeLayoutNode | null;
};

type TreeLayoutLink = {
  source: TreeLayoutNode;
  target: TreeLayoutNode;
};

type TreeLayout = {
  nodes: TreeLayoutNode[];
  links: TreeLayoutLink[];
  width: number;
  height: number;
};

const NODE_WIDTH = 260;
const NODE_MIN_HEIGHT = 58;
const NODE_DETAIL_LINE = 16;
const TREE_MARGIN = 42;
const LEVEL_GAP = 78;
const SIBLING_GAP = 22;
const ZOOM_MIN = 0.35;
const ZOOM_MAX = 2.5;
const ZOOM_STEP = 0.15;

export function TreeView({ document }: TreeViewProps) {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState<Set<string>>(() => new Set());
  const [query, setQuery] = useState("");
  const [zoom, setZoom] = useState(1);

  const tree = document?.tree;
  const diagnostics = document?.diagnostics ?? [];
  const visibleRoot = useMemo(() => (tree ? visibleTree(tree.root, collapsed) : null), [tree, collapsed]);
  const layout = useMemo(() => (visibleRoot ? layoutTree(visibleRoot) : null), [visibleRoot]);
  const normalizedQuery = query.trim().toLowerCase();
  const selectedNode = selectedId ? findTreeNode(tree?.root, selectedId) : undefined;
  const selectedEntity = entityForNode(document, selectedNode);

  useEffect(() => {
    setCollapsed(tree ? defaultCollapsedNodes(tree.root) : new Set());
    setSelectedId(null);
    setZoom(1);
  }, [tree]);

  if (diagnostics.length > 0 && !tree) {
    return (
      <section className="tree-view">
        <div className="tree-diagnostics">
          {diagnostics.map((diagnostic) => (
            <div className={`tree-diagnostic ${diagnostic.severity}`} key={`${diagnostic.code}-${diagnostic.message}`}>
              <strong>{diagnostic.code}</strong>
              <span>{diagnostic.message}</span>
            </div>
          ))}
        </div>
      </section>
    );
  }

  if (!tree || !layout) {
    return (
      <section className="tree-view">
        <div className="tree-empty">No tree document</div>
      </section>
    );
  }

  const toggleCollapse = (nodeId: string) => {
    const originalNode = findTreeNode(tree.root, nodeId);
    if (!originalNode || (originalNode.children ?? []).length === 0) {
      setSelectedId(nodeId);
      return;
    }

    setCollapsed((current) => {
      const next = new Set(current);
      if (next.has(nodeId)) {
        next.delete(nodeId);
      } else {
        next.add(nodeId);
      }
      return next;
    });
    setSelectedId(nodeId);
  };

  const zoomBy = (delta: number) => setZoom((value) => clampZoom(value + delta));

  return (
    <section className="tree-view">
      <div className="tree-toolbar">
        <div className="tree-title">
          <span>{document?.title ?? "Tree"}</span>
          <small>{layout.nodes.length} visible nodes</small>
        </div>
        <div className="tree-tools">
          <input
            aria-label="Search tree"
            placeholder="Search"
            type="search"
            value={query}
            onChange={(event) => setQuery(event.currentTarget.value)}
          />
          <button type="button" onClick={() => setCollapsed(new Set())}>
            Expand
          </button>
          <button type="button" onClick={() => setCollapsed(collapseAll(tree.root))}>
            Collapse
          </button>
          <div className="tree-zoom-controls" aria-label="Tree zoom controls">
            <button
              type="button"
              onClick={() => zoomBy(-ZOOM_STEP)}
              disabled={zoom <= ZOOM_MIN}
              title="Zoom out"
              aria-label="Zoom out"
            >
              -
            </button>
            <button type="button" onClick={() => setZoom(1)} title="Reset zoom" aria-label="Reset zoom">
              {Math.round(zoom * 100)}%
            </button>
            <button
              type="button"
              onClick={() => zoomBy(ZOOM_STEP)}
              disabled={zoom >= ZOOM_MAX}
              title="Zoom in"
              aria-label="Zoom in"
            >
              +
            </button>
          </div>
        </div>
      </div>
      <div className="tree-workspace">
        <div className="tree-canvas">
          <svg
            className="tree-svg"
            role="img"
            width={layout.width * zoom}
            height={layout.height * zoom}
            viewBox={`0 0 ${layout.width} ${layout.height}`}
            onClick={() => setSelectedId(null)}
          >
            <g>
              {layout.links.map((link) => (
                <path
                  className="tree-link"
                  d={treeLinkPath(link)}
                  key={`${link.source.data.id}-${link.target.data.id}`}
                />
              ))}
            </g>
            <g>
              {layout.nodes.map((node) => {
                const isSelected = selectedId === node.data.id;
                const isMatch = matchesQuery(node.data, normalizedQuery);
                const originalNode = findTreeNode(tree.root, node.data.id);
                const hasChildren = (originalNode?.children ?? []).length > 0;
                const isCollapsed = collapsed.has(node.data.id);
                const size = nodeSize(node.data);
                const textWidth = size.width - (hasChildren ? 48 : 24);
                return (
                  <g
                    className={[
                      "tree-node",
                      node.data.attrs?.kind ? `kind-${node.data.attrs.kind}` : "",
                      isSelected ? "selected" : "",
                      isMatch ? "matched" : ""
                    ]
                      .filter(Boolean)
                      .join(" ")}
                    key={node.data.id}
                    transform={`translate(${node.x}, ${node.y})`}
                    onClick={(event) => {
                      event.stopPropagation();
                      setSelectedId(node.data.id);
                    }}
                    onDoubleClick={(event) => {
                      event.stopPropagation();
                      toggleCollapse(node.data.id);
                    }}
                  >
                    <title>{node.data.detail ? `${node.data.label}\n${node.data.detail}` : node.data.label}</title>
                    <rect width={size.width} height={size.height} rx="6" />
                    <foreignObject x="12" y="9" width={textWidth} height={size.height - 18}>
                      <div className="tree-node-text">
                        <div className="tree-node-label">{node.data.label}</div>
                        {node.data.detail ? <div className="tree-node-detail">{node.data.detail}</div> : null}
                      </div>
                    </foreignObject>
                    {hasChildren ? (
                      <g
                        className="tree-node-toggle"
                        role="button"
                        aria-label={isCollapsed ? "Expand node" : "Collapse node"}
                        tabIndex={0}
                        transform={`translate(${size.width - 24}, 10)`}
                        onClick={(event) => {
                          event.stopPropagation();
                          toggleCollapse(node.data.id);
                        }}
                        onKeyDown={(event) => {
                          if (event.key === "Enter" || event.key === " ") {
                            event.stopPropagation();
                            toggleCollapse(node.data.id);
                          }
                        }}
                      >
                        <circle cx="8" cy="8" r="8" />
                        <text x="8" y="12" textAnchor="middle">
                          {isCollapsed ? "+" : "-"}
                        </text>
                      </g>
                    ) : null}
                  </g>
                );
              })}
            </g>
          </svg>
        </div>
        <TreeInspector entity={selectedEntity} node={selectedNode} />
      </div>
    </section>
  );
}

function TreeInspector({ entity, node }: { entity?: Entity; node?: TreeNode }) {
  if (!entity && !node) {
    return (
      <aside className="tree-inspector">
        <span className="tree-inspector-empty">No selection</span>
      </aside>
    );
  }

  return (
    <aside className="tree-inspector">
      <h2>{entity?.label ?? node?.label ?? "Selection"}</h2>
      {entity?.detail ? <p>{entity.detail}</p> : null}
      {node?.detail ? <p>{node.detail}</p> : null}
      <dl>
        {entity ? (
          <>
            <dt>Entity</dt>
            <dd>{entity.id}</dd>
            <dt>Kind</dt>
            <dd>{entity.kind}</dd>
          </>
        ) : null}
        {node ? (
          <>
            <dt>Node</dt>
            <dd>{node.id}</dd>
          </>
        ) : null}
      </dl>
    </aside>
  );
}

function layoutTree(root: TreeNode): TreeLayout {
  const layout = flextree<TreeNode>({
    children: (node) => node.children ?? [],
    nodeSize: (node) => {
      const size = nodeSize(node.data);
      return [size.width + SIBLING_GAP, size.height + LEVEL_GAP];
    },
    spacing: (nodeA, nodeB) => (nodeA.parent === nodeB.parent ? SIBLING_GAP : SIBLING_GAP * 2)
  });

  const hierarchy = layout.hierarchy(root);
  layout(hierarchy);

  const rawNodes = hierarchy.descendants() as TreeLayoutNode[];
  const minX = Math.min(...rawNodes.map((node) => node.x));
  const minY = Math.min(...rawNodes.map((node) => node.y));
  const maxX = Math.max(...rawNodes.map((node) => node.x + nodeSize(node.data).width));
  const maxY = Math.max(...rawNodes.map((node) => node.y + nodeSize(node.data).height));
  const nodes = rawNodes.map((node) => ({
    ...node,
    x: node.x - minX + TREE_MARGIN,
    y: node.y - minY + TREE_MARGIN
  }));
  const nodeById = new Map(nodes.map((node) => [node.data.id, node]));
  const links = hierarchy.links().map((link) => ({
    source: nodeById.get(link.source.data.id)!,
    target: nodeById.get(link.target.data.id)!
  }));

  return {
    nodes,
    links,
    width: maxX - minX + TREE_MARGIN * 2,
    height: maxY - minY + TREE_MARGIN * 2
  };
}

function nodeSize(node: TreeNode) {
  const detailLines = node.detail ? Math.min(2, Math.ceil(node.detail.length / 34)) : 0;
  return {
    width: NODE_WIDTH,
    height: NODE_MIN_HEIGHT + detailLines * NODE_DETAIL_LINE
  };
}

function treeLinkPath(link: TreeLayoutLink) {
  const sourceSize = nodeSize(link.source.data);
  const targetSize = nodeSize(link.target.data);
  const sourceX = link.source.x + sourceSize.width / 2;
  const sourceY = link.source.y + sourceSize.height;
  const targetX = link.target.x + targetSize.width / 2;
  const targetY = link.target.y;
  const midY = sourceY + Math.max(36, (targetY - sourceY) / 2);
  return `M ${sourceX} ${sourceY} C ${sourceX} ${midY}, ${targetX} ${midY}, ${targetX} ${targetY}`;
}

function visibleTree(node: TreeNode, collapsed: ReadonlySet<string>): TreeNode {
  return {
    ...node,
    children: collapsed.has(node.id) ? [] : (node.children ?? []).map((child) => visibleTree(child, collapsed))
  };
}

function collapseAll(root: TreeNode) {
  const collapsed = new Set<string>();
  visitTree(root, (node) => {
    if ((node.children ?? []).length > 0) {
      collapsed.add(node.id);
    }
  });
  return collapsed;
}

function defaultCollapsedNodes(root: TreeNode) {
  const collapsed = new Set<string>();
  visitTree(root, (node) => {
    if (node.attrs?.defaultCollapsed === true) {
      collapsed.add(node.id);
    }
  });
  return collapsed;
}

function findTreeNode(root: TreeNode | undefined, id: string): TreeNode | undefined {
  if (!root) return undefined;
  if (root.id === id) return root;
  for (const child of root.children ?? []) {
    const found = findTreeNode(child, id);
    if (found) return found;
  }
  return undefined;
}

function visitTree(node: TreeNode, visitor: (node: TreeNode) => void) {
  visitor(node);
  for (const child of node.children ?? []) {
    visitTree(child, visitor);
  }
}

function matchesQuery(node: TreeNode, query: string) {
  return query.length > 0 && `${node.label} ${node.detail ?? ""}`.toLowerCase().includes(query);
}

function entityForNode(document: RenderDocument | null, treeNode?: TreeNode) {
  if (!treeNode) return undefined;
  return document?.entities.find((entity) => entity.id === treeNode.entityId);
}

function clampZoom(value: number) {
  return Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, Number(value.toFixed(2))));
}
