import { useEffect, useState } from "react";

import { getTraces } from "../api/client";
import type { SessionState, TraceListResponse } from "../api/protocol";
import { FolderIcon, FolderPicker, type FolderPickerOptions } from "./FolderPicker";

type StartViewProps = {
  session: SessionState;
  /** The last folders chosen in this browser, where pickers start. */
  remembered: { dialects: string | null; traces: string | null };
  onDialects: (path: string) => Promise<void>;
  onCloseDialects: () => Promise<void>;
  onTraces: (path: string) => Promise<void>;
  onContinue: () => void;
};

/** The first page: choose the dialects folder and the traces folder. */
export function StartView({ session, remembered, onDialects, onCloseDialects, onTraces, onContinue }: StartViewProps) {
  const [picker, setPicker] = useState<FolderPickerOptions | null>(null);
  const [traces, setTraces] = useState<TraceListResponse | null>(null);
  const [dialectsError, setDialectsError] = useState<string | null>(null);
  const [tracesError, setTracesError] = useState<string | null>(null);

  useEffect(() => {
    if (!session.traces) {
      setTraces(null);
      return;
    }
    getTraces()
      .then(setTraces)
      .catch((err: unknown) => setTracesError(err instanceof Error ? err.message : String(err)));
  }, [session.traces]);

  const dialects = session.dialects;
  const engine = dialects ? engineLine(dialects.state, dialects.message) : null;
  const traceCount = traces?.projects.reduce((sum, project) => sum + project.versions.length, 0) ?? 0;

  const chooseDialects = () =>
    setPicker({
      title: "Choose the dialects folder",
      hint: "A cargo workspace whose crates define pliron dialects — marked “pliron project”.",
      start: parentOf(dialects?.root ?? remembered.dialects),
      good: (marks) => marks.pliron,
      onPick: (path) => {
        setDialectsError(null);
        onDialects(path).catch((err: unknown) => setDialectsError(err instanceof Error ? err.message : String(err)));
      }
    });
  const chooseTraces = () =>
    setPicker({
      title: "Choose the traces folder",
      hint: "The folder your compiler writes its traces (.crt) to — marked with their count.",
      start: dialects?.root ?? parentOf(session.traces ?? remembered.traces),
      good: (marks) => marks.traces > 0,
      onPick: (path) => {
        setTracesError(null);
        onTraces(path).catch((err: unknown) => setTracesError(err instanceof Error ? err.message : String(err)));
      }
    });

  return (
    <main className="start">
      <header className="start-header">
        <h1>pliron-inspect</h1>
        <p>See what your pliron compiler did to the IR, pass by pass.</p>
      </header>

      <div className="start-cards">
        <section className={dialects?.state === "ready" ? "start-card chosen" : "start-card"}>
          <div className="start-card-head">
            <FolderIcon />
            <div>
              <h2>Dialects</h2>
              <p>The cargo workspace whose crates define your pliron dialects.</p>
            </div>
          </div>
          <div className={dialects ? "start-path" : "start-path empty"}>{dialects?.root ?? "No folder chosen"}</div>
          <div className="start-actions">
            <button className="primary" type="button" onClick={chooseDialects}>
              Choose folder…
            </button>
            {dialects ? (
              <button type="button" onClick={() => void onCloseDialects()}>
                Close
              </button>
            ) : null}
          </div>
          <div className={`start-status ${dialectsError ? "bad" : engine?.tone ?? "muted"}`}>
            {dialectsError ??
              engine?.text ??
              "Its dialect engine reads the IR: CFG and tree views, hover, definitions and diagnostics."}
          </div>
        </section>

        <section className={session.traces ? "start-card chosen" : "start-card"}>
          <div className="start-card-head">
            <TracesIcon />
            <div>
              <h2>Traces</h2>
              <p>The folder your compiler writes its traces to, one subfolder per project.</p>
            </div>
          </div>
          <div className={session.traces ? "start-path" : "start-path empty"}>{session.traces ?? "No folder chosen"}</div>
          <div className="start-actions">
            <button className="primary" type="button" onClick={chooseTraces}>
              Choose folder…
            </button>
          </div>
          <div className={`start-status ${tracesError ? "bad" : session.traces && traces && traceCount === 0 ? "warn" : "muted"}`}>
            {tracesError ??
              (!session.traces
                ? "The IR after each pass of a compilation: a trace (.crt) per compilation."
                : !traces
                  ? "Reading…"
                  : traceCount === 0
                    ? "No traces (.crt) in this folder yet."
                    : `${traces.projects.length} project${traces.projects.length === 1 ? "" : "s"}, ${traceCount} trace${traceCount === 1 ? "" : "s"}`)}
          </div>
        </section>
      </div>

      <div className="start-continue">
        <button className="primary large" type="button" disabled={!session.traces} onClick={onContinue}>
          Inspect traces →
        </button>
        <span className="start-continue-note">
          {!session.traces
            ? "Choose a traces folder to continue."
            : !dialects
              ? "Without a dialects folder only the text views work."
              : ""}
        </span>
      </div>

      {picker ? <FolderPicker options={picker} onClose={() => setPicker(null)} /> : null}
    </main>
  );
}

/** The dialect engine's state, in words. */
export function engineLine(state: string, message?: string | null): { text: string; tone: "ok" | "warn" | "bad" } {
  const detail = message ? ` — ${message}` : "";
  switch (state) {
    case "ready":
      return { text: `Dialect engine ready${detail}`, tone: "ok" };
    case "starting":
      return { text: "Starting pliron-lsp…", tone: "warn" };
    case "building":
      return { text: `Building the dialect engine${detail}…`, tone: "warn" };
    case "syntax-only":
      return { text: `No dialect engine for this folder${detail}`, tone: "warn" };
    default:
      return { text: message || `Dialect engine: ${state}`, tone: "bad" };
  }
}

function parentOf(path: string | null | undefined) {
  return path ? path.replace(/\/[^/]*$/, "") : null;
}

function TracesIcon() {
  return (
    <svg className="traces-icon" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M3 17l5-6 4 4 4-7 5 9" />
      <path d="M3 21h18" />
    </svg>
  );
}
