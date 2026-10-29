export type ViewId = "ir-diff" | "cfg" | "tree" | "text" | "versions";

export type DisplayCapabilities = {
  protocolVersion: number;
  views: ViewCapability[];
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
  /** The traces folder, if one is chosen. */
  dir?: string | null;
  projects: TraceProjectInfo[];
};

export type OpenTraceResponse = {
  filepath: string;
  filename: string;
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

/** One step of a trace's history: snapshot `step` against the one before. */
export type HistoryStep = {
  /** The snapshot is the previous one, byte for byte. */
  identical: boolean;
  hunks: HistoryHunk[];
  /** Who last changed each line: runs of `[step, line count]`, in order. */
  blame: [number, number][];
};

/**
 * Lines `[start, end)` (1-based) of the previous snapshot replaced by lines
 * `[start, end)` of this one. `pairs` are the lines that are the same op (or
 * block) on both sides, `[original line, modified line]`, increasing on
 * both; `inner` are their changed characters: `[original line, start column,
 * end column, modified line, start column, end column]`.
 */
export type HistoryHunk = {
  original: [number, number];
  modified: [number, number];
  pairs: [number, number][];
  inner: [number, number, number, number, number, number][];
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
  /** An IR document id from `addIr`. */
  id: string;
  view: ViewId;
  rootId?: string;
};

export type RenderDocument = {
  version: number;
  view: ViewId | string;
  title?: string;
  snapshotId: string;
  entities: Entity[];
  graph?: GraphDocument;
  tree?: TreeDocument;
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

export type Diagnostic = {
  severity: "info" | "warning" | "error";
  code: string;
  message: string;
  entityId?: string;
  attrs?: Record<string, unknown>;
};

/** The dialect engine's state, as pliron-lsp reports it. */
export type EngineState = "starting" | "syntax-only" | "building" | "ready" | "error" | "exited" | string;

export type SessionState = {
  dialects: { root: string; state: EngineState; message?: string | null; lsp?: string } | null;
  traces: string | null;
};

export type FolderMarks = {
  cargo: boolean;
  pliron: boolean;
  traces: number;
};

export type FolderListing = {
  path: string;
  parent?: string | null;
  home: string;
  marks: FolderMarks;
  folders: { name: string; path: string; marks: FolderMarks }[];
};

/** An LSP position: 0-based line, UTF-16 column. */
export type LspPosition = { line: number; character: number };
export type LspRange = { start: LspPosition; end: LspPosition };

export type HoverResult = { contents?: string | null; range?: LspRange | null; exact: boolean };

export type DefinitionResult = {
  exact: boolean;
  /** A definition in the same IR document. */
  range?: LspRange | null;
  /** A definition in the dialects' Rust source. */
  source?: { path: string; position: LspPosition; range?: LspRange | null; text?: string | null };
};

export type ReferencesResult = { ranges: LspRange[]; exact: boolean };

export type IrDiagnostic = {
  range: LspRange;
  severity: "error" | "warning" | "info" | "hint";
  source?: string | null;
  /** parse, verify, lint, ... */
  phase?: string | null;
  message: string;
};

export type DiagnosticsResult = { diagnostics: IrDiagnostic[]; exact: boolean };
