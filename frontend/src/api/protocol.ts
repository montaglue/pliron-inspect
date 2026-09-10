export type ViewId = "ir-diff" | "cfg" | "tree" | "egraph" | "text" | "versions";

export type DisplayCapabilities = {
  protocolVersion: number;
  views: ViewCapability[];
  cliCommands: string[];
  frontend: FrontendStack;
};

export type TraceFileInfo = {
  filename: string;
  filepath: string;
  modifiedMs?: number;
};

export type TraceProjectInfo = {
  name: string;
  versions: TraceFileInfo[];
};

export type TraceListResponse = {
  libraryProjects: TraceProjectInfo[];
  tempProjects: TraceProjectInfo[];
  libraryDirs: string[];
  tempDirs: string[];
};

export type OpenTraceResponse = {
  filepath: string;
  filename: string;
  importedFrom?: string;
  meta: TraceMeta;
  snapshots: TraceSnapshot[];
  pipeline: string[];
};

export type TraceMeta = {
  name: string;
  kind: string;
  entry?: string;
  source?: string;
  pipeline?: string[];
  target?: string;
  note?: string;
  [key: string]: unknown;
};

export type TraceSnapshot = {
  label: string;
  ir: string;
  isError?: boolean;
};

export type ViewCapability = {
  id: ViewId;
  label: string;
  description: string;
};

export type FrontendStack = {
  framework: string;
  bundler: string;
  language: string;
  graph: string;
  tree: string;
  text: string;
  egraph: string;
};

export type RenderRequest = {
  view: ViewId;
  snapshotId?: string;
  rootId?: string;
  options?: Record<string, unknown>;
  text?: string;
};

export type RenderDocument = {
  version: number;
  view: ViewId | string;
  title?: string;
  snapshotId: string;
  entities: Entity[];
  graph?: GraphDocument;
  tree?: TreeDocument;
  text?: TextDocument;
  diagnostics?: Diagnostic[];
};

export type Entity = {
  id: string;
  kind: "operation" | "region" | "block" | "value" | "type" | "attribute" | "interface";
  label: string;
  detail?: string;
  interfaces?: string[];
  sourceSpan?: SourceSpan;
  attrs?: Record<string, unknown>;
};

export type SourceSpan = {
  startLine: number;
  startColumn: number;
  endLine: number;
  endColumn: number;
};

export type GraphDocument = {
  nodes: GraphNode[];
  edges: GraphEdge[];
  roots?: string[];
  layout?: GraphLayout;
};

export type GraphNode = {
  id: string;
  entityId: string;
  kind: string;
  label: string;
  detail?: string;
  parentId?: string;
  attrs?: Record<string, unknown>;
};

export type GraphEdge = {
  id: string;
  from: string;
  to: string;
  kind: string;
  label?: string;
  attrs?: Record<string, unknown>;
};

export type GraphLayout = {
  engine: string;
  width: number;
  height: number;
  nodes: GraphNodeLayout[];
  edges: GraphEdgeLayout[];
};

export type GraphNodeLayout = {
  nodeId: string;
  x: number;
  y: number;
  width: number;
  height: number;
  rank: number;
  column: number;
};

export type GraphEdgeLayout = {
  edgeId: string;
  points: GraphPoint[];
};

export type GraphPoint = {
  x: number;
  y: number;
};

export type TreeDocument = {
  root: TreeNode;
};

export type TreeNode = {
  id: string;
  entityId: string;
  label: string;
  detail?: string;
  children?: TreeNode[];
  attrs?: Record<string, unknown>;
};

export type TextDocument = {
  language: "crabbit-ir" | "plaintext" | string;
  text: string;
  spans?: TextSpan[];
};

export type TextSpan = {
  entityId: string;
  start: number;
  end: number;
};

export type Diagnostic = {
  severity: "info" | "warning" | "error";
  code: string;
  message: string;
  entityId?: string;
  attrs?: Record<string, unknown>;
};
