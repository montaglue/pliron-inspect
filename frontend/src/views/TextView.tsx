import { useEffect, useMemo, useRef, useState } from "react";
import * as monaco from "monaco-editor/esm/vs/editor/editor.api";

import { irDiagnostics } from "../api/client";
import type { IrDiagnostic, TraceSnapshot } from "../api/protocol";
import { type SnapshotProfile, compact, decisions, describe, heatLevel, percent } from "../profile";
import { IR_LANGUAGE, THEME, bindIrId, configurePlironMonaco, setSourceOpener, toRange } from "./monacoSetup";

type TextViewProps = {
  snapshots: TraceSnapshot[];
  step?: number;
  onStepChange?: (step: number) => void;
  /** The server's id of the shown snapshot, once it has it. */
  irId: string | null;
  /** The dialect engine's state; null without a dialects folder. */
  engineState: string | null;
  /** The measured profile of the shown snapshot, if the trace has one. */
  profile?: SnapshotProfile | null;
  /** Why the trace's profile does not reach this snapshot, if it does not. */
  profileNote?: string | null;
};

type Source = { model: monaco.editor.ITextModel; range: monaco.IRange };

const severities: Record<IrDiagnostic["severity"], monaco.MarkerSeverity> = {
  error: monaco.MarkerSeverity.Error,
  warning: monaco.MarkerSeverity.Warning,
  info: monaco.MarkerSeverity.Info,
  hint: monaco.MarkerSeverity.Hint
};

export function TextView({ snapshots, step, onStepChange, irId, engineState, profile, profileNote }: TextViewProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null);
  const [model, setModel] = useState<monaco.editor.ITextModel | null>(null);
  const [localStep, setLocalStep] = useState(0);
  const [problems, setProblems] = useState<{ errors: number; warnings: number } | null>(null);
  const [source, setSource] = useState<Source | null>(null);
  const currentStep = step ?? localStep;
  const setCurrentStep = (next: number) => {
    setLocalStep(next);
    onStepChange?.(next);
  };

  const currentSnapshot = snapshots[currentStep] ?? null;
  const stepLabel = useMemo(() => {
    if (!currentSnapshot) return "No snapshots";
    const label = currentSnapshot.isError ? "ERROR" : currentStep === 0 ? "initial" : currentSnapshot.label;
    return `${label} (${currentStep + 1}/${snapshots.length})`;
  }, [currentSnapshot, currentStep, snapshots.length]);

  useEffect(() => {
    if (snapshots.length === 0) return;

    configurePlironMonaco();
    setSourceOpener((sourceModel, range) => setSource({ model: sourceModel, range }));

    if (!containerRef.current || editorRef.current) return;

    editorRef.current = monaco.editor.create(containerRef.current, {
      readOnly: true,
      automaticLayout: true,
      theme: THEME,
      language: IR_LANGUAGE,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      wordWrap: "off"
    });

    return () => {
      setSourceOpener(null);
      editorRef.current?.getModel()?.dispose();
      editorRef.current?.dispose();
      editorRef.current = null;
      setModel(null);
    };
  }, [snapshots.length]);

  useEffect(() => {
    const editor = editorRef.current;
    if (!editor || !currentSnapshot) return;

    const previous = editor.getModel();
    const next = monaco.editor.createModel(currentSnapshot.ir, currentSnapshot.isError ? "plaintext" : IR_LANGUAGE);
    editor.setModel(next);
    editor.setScrollTop(0);
    editor.setScrollLeft(0);
    if (previous) {
      bindIrId(previous, null);
      previous.dispose();
    }
    setModel(next);
  }, [currentSnapshot]);

  // The engine's answers (hover, definition, references) need the id.
  useEffect(() => {
    if (model) bindIrId(model, irId);
  }, [model, irId]);

  // The measured profile: heat on each op's line, its numbers after it.
  useEffect(() => {
    const editor = editorRef.current;
    // The snapshot may have changed under us: decorate only the model the
    // editor shows (the effect runs again once `model` catches up).
    if (!editor || !model || model.isDisposed() || editor.getModel() !== model) return;
    const decorations: monaco.editor.IModelDeltaDecoration[] = [];
    if (profile) {
      for (const [line, metrics] of profile.byLine) {
        if (line < 1 || line > model.getLineCount()) continue;
        const cycles = metrics.cycles ?? 0;
        const level = heatLevel(cycles, profile.maxCycles);
        const own = decisions(metrics);
        const notable = cycles >= profile.total * 0.002 || own.length > 0;
        const parts: string[] = [];
        if (cycles > 0) parts.push(`${compact(cycles)} cycles ${percent(cycles, profile.total)}`);
        if (metrics.executions) parts.push(`${compact(metrics.executions)}×`);
        for (const [name, value] of own) parts.push(`${name.replace(/\.cycles$/, "")} ${compact(value)}`);
        const end = model.getLineMaxColumn(line);
        decorations.push({
          range: new monaco.Range(line, 1, line, end),
          options: {
            isWholeLine: true,
            className: level > 0 ? `profile-heat-${level}` : undefined,
            hoverMessage: { value: describe(metrics, profile.total) },
            after: notable && parts.length > 0 ? { content: `   ${parts.join(" · ")}`, inlineClassName: "profile-after" } : undefined
          }
        });
      }
    }
    const collection = editor.createDecorationsCollection(decorations);
    return () => {
      if (!model.isDisposed()) collection.clear();
    };
  }, [model, profile]);

  // Parse and verifier diagnostics of the snapshot, as markers.
  useEffect(() => {
    setProblems(null);
    if (!model || !irId || !engineState) return;
    let live = true;
    let timer: number | undefined;
    let tries = 0;
    const load = () =>
      irDiagnostics(irId)
        .then((result) => {
          if (!live) return;
          monaco.editor.setModelMarkers(
            model,
            "pliron",
            result.diagnostics.map((diagnostic) => ({
              ...toRange(diagnostic.range),
              severity: severities[diagnostic.severity] ?? monaco.MarkerSeverity.Error,
              message: diagnostic.message,
              source: [diagnostic.source, diagnostic.phase].filter(Boolean).join(" ") || undefined
            }))
          );
          if (result.exact) {
            setProblems({
              errors: result.diagnostics.filter((diagnostic) => diagnostic.severity === "error").length,
              warnings: result.diagnostics.filter((diagnostic) => diagnostic.severity === "warning").length
            });
          } else if (engineState === "ready" && tries++ < 20) {
            // The engine is still reading this snapshot.
            timer = window.setTimeout(load, 1500);
          }
        })
        .catch(() => undefined);
    load();
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [model, irId, engineState]);

  if (snapshots.length === 0) {
    return (
      <div className="no-trace-state">
        <TraceIcon />
        <span>Select a trace to inspect IR snapshots</span>
      </div>
    );
  }

  return (
    <section className="text-view">
      <div className="diff-toolbar">
        <div className={currentSnapshot?.isError ? "step-label error" : "step-label"}>{stepLabel}</div>
        <div className="text-status">
          <EngineStatus state={engineState} />
          {profile ? (
            <span className="profile-status" title="Measured profile of this snapshot, from the compiler's backward passes">
              profile · {compact(profile.total)} cycles
            </span>
          ) : profileNote ? (
            <span className="profile-status off" title={profileNote}>
              no profile here
            </span>
          ) : null}
          {problems ? (
            <button
              className={problems.errors ? "problems bad" : problems.warnings ? "problems warn" : "problems ok"}
              type="button"
              title="Go to the next problem (F8)"
              onClick={() => {
                editorRef.current?.focus();
                editorRef.current?.trigger("toolbar", "editor.action.marker.next", null);
              }}
            >
              {problems.errors || problems.warnings
                ? `${problems.errors} error${problems.errors === 1 ? "" : "s"}, ${problems.warnings} warning${problems.warnings === 1 ? "" : "s"}`
                : "no problems"}
            </button>
          ) : null}
          {engineState === "ready" ? (
            <span className="text-help">hover · F12 definition · ⇧F12 references · F8 problems</span>
          ) : null}
        </div>
        <div className="diff-toolbar-actions">
          <button type="button" onClick={() => setCurrentStep(Math.max(0, currentStep - 1))} disabled={currentStep <= 0}>
            Prev
          </button>
          <button
            type="button"
            onClick={() => setCurrentStep(Math.min(snapshots.length - 1, currentStep + 1))}
            disabled={currentStep >= snapshots.length - 1}
          >
            Next
          </button>
        </div>
      </div>
      <div className={source ? "text-panes with-source" : "text-panes"}>
        <div className="text-editor" ref={containerRef} />
        {source ? <SourcePane source={source} onClose={() => setSource(null)} /> : null}
      </div>
    </section>
  );
}

/** A dialect's Rust source, at a definition. */
function SourcePane({ source, onClose }: { source: Source; onClose: () => void }) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null);

  useEffect(() => {
    if (!containerRef.current) return;
    editorRef.current = monaco.editor.create(containerRef.current, {
      readOnly: true,
      automaticLayout: true,
      theme: THEME,
      minimap: { enabled: false },
      scrollBeyondLastLine: false
    });
    return () => {
      editorRef.current?.dispose();
      editorRef.current = null;
    };
  }, []);

  useEffect(() => {
    const editor = editorRef.current;
    if (!editor) return;
    editor.setModel(source.model);
    editor.setSelection(source.range);
    editor.revealRangeInCenter(source.range);
  }, [source]);

  const path = source.model.uri.path;
  return (
    <div className="source-pane">
      <div className="source-pane-head">
        <span className="source-pane-path" title={path}>
          {shortPath(path)}:{source.range.startLineNumber}
        </span>
        <button type="button" onClick={onClose} title="Close">
          ×
        </button>
      </div>
      <div className="source-pane-editor" ref={containerRef} />
    </div>
  );
}

/** A path with cargo's registry prefix shortened to the crate. */
function shortPath(path: string) {
  const registry = path.match(/\/registry\/src\/[^/]+\/(.*)$/);
  if (registry) return registry[1];
  const git = path.match(/\/git\/checkouts\/[^/]+\/[^/]+\/(.*)$/);
  return git ? git[1] : path;
}

function EngineStatus({ state }: { state: string | null }) {
  if (!state) return <span className="engine-status off">no dialects folder</span>;
  if (state === "ready") return <span className="engine-status ok">dialect engine</span>;
  if (state === "building" || state === "starting")
    return <span className="engine-status warn">dialect engine building…</span>;
  return <span className="engine-status bad">dialect engine: {state}</span>;
}

function TraceIcon() {
  return (
    <svg className="trace-icon" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M5 6h14" />
      <path d="M5 12h14" />
      <path d="M5 18h14" />
      <circle cx="8" cy="6" r="1.5" />
      <circle cx="13" cy="12" r="1.5" />
      <circle cx="10" cy="18" r="1.5" />
    </svg>
  );
}
