import type {
  DefinitionResult,
  DiagnosticsResult,
  DisplayCapabilities,
  FolderListing,
  HistoryStep,
  HoverResult,
  LspPosition,
  OpenTraceResponse,
  ReferencesResult,
  RenderDocument,
  RenderRequest,
  SessionState,
  TraceListResponse
} from "./protocol";

async function requestJson<T>(url: string, init?: RequestInit): Promise<T> {
  const response = await fetch(url, init);
  const body = await response.json();
  if (!response.ok) {
    const message = typeof body?.error === "string" ? body.error : response.statusText;
    throw new Error(message);
  }
  return body as T;
}

function post<T>(url: string, body: unknown = {}): Promise<T> {
  return requestJson<T>(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body)
  });
}

export function getCapabilities(): Promise<DisplayCapabilities> {
  return requestJson<DisplayCapabilities>("/api/capabilities");
}

export function getState(): Promise<SessionState> {
  return requestJson<SessionState>("/api/state");
}

export function listFolders(path?: string | null): Promise<FolderListing> {
  return requestJson<FolderListing>(`/api/folders?path=${encodeURIComponent(path ?? "")}`);
}

export function openDialects(path: string): Promise<SessionState> {
  return post<SessionState>("/api/dialects", { path });
}

export function closeDialects(): Promise<SessionState> {
  return post<SessionState>("/api/dialects/close");
}

export function setTracesFolder(path: string): Promise<SessionState> {
  return post<SessionState>("/api/traces/folder", { path });
}

export function getTraces(): Promise<TraceListResponse> {
  return requestJson<TraceListResponse>("/api/traces");
}

export function openTrace(filepath: string): Promise<OpenTraceResponse> {
  return post<OpenTraceResponse>("/api/traces/open", { filepath });
}

export function importTrace(filename: string, contents: string): Promise<OpenTraceResponse> {
  return post<OpenTraceResponse>("/api/traces/import", { filename, contents });
}

/** Step `step` of the trace's history (diff and blame), computed once. */
export function historyStep(filepath: string, step: number): Promise<HistoryStep> {
  return post<HistoryStep>("/api/history/step", { filepath, step });
}

/** Hand an IR snapshot to the server; views and queries refer to it by id. */
export async function addIr(text: string): Promise<string> {
  return (await post<{ id: string }>("/api/ir", { text })).id;
}

export function renderDocument(request: RenderRequest): Promise<RenderDocument> {
  return post<RenderDocument>("/api/ir/render", request);
}

export function irHover(id: string, position: LspPosition): Promise<HoverResult> {
  return post<HoverResult>("/api/ir/hover", { id, position });
}

export function irDefinition(id: string, position: LspPosition): Promise<DefinitionResult> {
  return post<DefinitionResult>("/api/ir/definition", { id, position });
}

export function irReferences(id: string, position: LspPosition): Promise<ReferencesResult> {
  return post<ReferencesResult>("/api/ir/references", { id, position });
}

export function irDiagnostics(id: string): Promise<DiagnosticsResult> {
  return post<DiagnosticsResult>("/api/ir/diagnostics", { id });
}
