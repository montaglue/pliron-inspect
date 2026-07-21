import type { DisplayCapabilities, OpenTraceResponse, RenderDocument, RenderRequest, TraceListResponse } from "./protocol";

async function requestJson<T>(url: string, init?: RequestInit): Promise<T> {
  const response = await fetch(url, init);
  const body = await response.json();
  if (!response.ok) {
    const message = typeof body?.error === "string" ? body.error : response.statusText;
    throw new Error(message);
  }
  return body as T;
}

export function getCapabilities(): Promise<DisplayCapabilities> {
  return requestJson<DisplayCapabilities>("/api/capabilities");
}

export function renderDocument(request: RenderRequest): Promise<RenderDocument> {
  return requestJson<RenderDocument>("/api/render", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(request)
  });
}

export function getTraces(): Promise<TraceListResponse> {
  return requestJson<TraceListResponse>("/api/traces");
}

export function openTrace(filepath: string): Promise<OpenTraceResponse> {
  return requestJson<OpenTraceResponse>("/api/traces/open", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ filepath })
  });
}

export function importTrace(filename: string, contents: string): Promise<OpenTraceResponse> {
  return requestJson<OpenTraceResponse>("/api/traces/import", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ filename, contents })
  });
}
