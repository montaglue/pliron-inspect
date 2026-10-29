import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as monaco from "monaco-editor/esm/vs/editor/editor.api";

import { historyStep } from "../api/client";
import type { HistoryHunk, HistoryStep, TraceSnapshot } from "../api/protocol";
import { IR_LANGUAGE, THEME, configurePlironMonaco } from "./monacoSetup";
import { forgetDiff, storeDiff } from "./storedDiffs";

export type IrSnapshot = TraceSnapshot;

type IrDiffViewProps = {
  snapshots: IrSnapshot[];
  /** The trace file: the server keeps its history (diff and blame of every step). */
  tracePath: string;
};

/** The step on screen and its history (none for the initial snapshot). */
type Shown = { step: number; history: HistoryStep | null };

const BLAME_PALETTE = [
  "#4fc1ff",
  "#c586c0",
  "#dcdcaa",
  "#ce9178",
  "#b5cea8",
  "#d16969",
  "#9cdcfe",
  "#608b4e",
  "#d7ba7d",
  "#569cd6"
];

const BLAME_STYLE_ID = "ir-diff-blame-styles";

export function IrDiffView({ snapshots, tracePath }: IrDiffViewProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const editorRef = useRef<monaco.editor.IStandaloneDiffEditor | null>(null);
  const modelsRef = useRef<{ original: monaco.editor.ITextModel; modified: monaco.editor.ITextModel } | null>(null);
  const historyRef = useRef(new Map<number, Promise<HistoryStep>>());
  const shownRef = useRef<Shown | null>(null);

  const [currentStep, setCurrentStep] = useState(() => initialDiffStep(snapshots));
  const [shown, setShown] = useState<Shown | null>(null);
  const [hideUnchanged, setHideUnchanged] = useState(true);
  const [blameEnabled, setBlameEnabled] = useState(false);
  const [diffColorsEnabled, setDiffColorsEnabled] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const currentSnapshot = snapshots[currentStep] ?? null;

  const stepLabel = useMemo(() => {
    if (!currentSnapshot) return "No snapshots";
    const label = currentSnapshot.isError ? "ERROR" : currentStep === 0 ? "initial" : currentSnapshot.label;
    return `${label}  (${currentStep + 1}/${snapshots.length})`;
  }, [currentSnapshot, currentStep, snapshots.length]);

  /** The history of `step`, asked for once per trace. */
  const history = useCallback(
    (step: number) => {
      let pending = historyRef.current.get(step);
      if (!pending) {
        pending = historyStep(tracePath, step);
        historyRef.current.set(step, pending);
        pending.catch(() => historyRef.current.delete(step));
      }
      return pending;
    },
    [tracePath]
  );

  useEffect(() => {
    historyRef.current = new Map();
    setCurrentStep((step) => (step >= snapshots.length ? initialDiffStep(snapshots) : step));
  }, [snapshots, tracePath]);

  useEffect(() => {
    configurePlironMonaco();
    if (!containerRef.current || editorRef.current) return;

    const editor = monaco.editor.createDiffEditor(containerRef.current, {
      readOnly: true,
      renderSideBySide: true,
      automaticLayout: true,
      theme: THEME,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      hideUnchangedRegions: {
        enabled: true,
        contextLineCount: 3,
        minimumLineCount: 3,
        revealLineCount: 20
      }
    } as monaco.editor.IDiffEditorConstructionOptions);
    editorRef.current = editor;

    // The cursor's row, and the row it corresponds to on the other side.
    const originalEditor = editor.getOriginalEditor();
    const modifiedEditor = editor.getModifiedEditor();
    const originalRows = originalEditor.createDecorationsCollection();
    const modifiedRows = modifiedEditor.createDecorationsCollection();
    const highlight = (side: "original" | "modified", line: number) => {
      const hunks = shownRef.current?.history?.hunks;
      const [here, there] = side === "original" ? [originalRows, modifiedRows] : [modifiedRows, originalRows];
      here.set([rowDecoration(line, line + 1, "diff-cursor-row")]);
      const other = hunks ? correspondingLines(hunks, line, side) : null;
      there.set(other ? [rowDecoration(other[0], other[1], "diff-corresponding-row")] : []);
    };
    const listeners = [
      originalEditor.onDidChangeCursorPosition((event) => highlight("original", event.position.lineNumber)),
      modifiedEditor.onDidChangeCursorPosition((event) => highlight("modified", event.position.lineNumber))
    ];

    return () => {
      listeners.forEach((listener) => listener.dispose());
      const models = modelsRef.current;
      editor.dispose();
      if (models) {
        forgetDiff(models.modified);
        models.original.dispose();
        models.modified.dispose();
      }
      modelsRef.current = null;
      editorRef.current = null;
    };
  }, []);

  // Show a step: its diff comes from the history, Monaco computes none.
  useEffect(() => {
    const editor = editorRef.current;
    if (!editor) return;
    let live = true;
    const step = currentStep;
    const current = snapshots[step] ?? null;
    const previous = step > 0 ? snapshots[step - 1] : null;

    const show = (stepHistory: HistoryStep | null) => {
      if (!live) return;
      const old = modelsRef.current;
      const original = monaco.editor.createModel(previous?.ir ?? "", previous?.isError ? "plaintext" : IR_LANGUAGE);
      const modified = monaco.editor.createModel(current?.ir ?? "", current?.isError ? "plaintext" : IR_LANGUAGE);
      if (stepHistory) storeDiff(original, modified, stepHistory);
      // Swap first, dispose after: the editor must never hold a disposed model.
      editor.setModel({ original, modified });
      modelsRef.current = { original, modified };
      if (old) {
        forgetDiff(old.modified);
        old.original.dispose();
        old.modified.dispose();
      }
      const next = { step, history: stepHistory };
      shownRef.current = next;
      setShown(next);
    };

    if (step === 0) {
      show(null);
    } else {
      history(step).then(
        (stepHistory) => {
          setError(null);
          show(stepHistory);
        },
        (err: unknown) => {
          if (!live) return;
          setError(err instanceof Error ? err.message : String(err));
          show(null);
        }
      );
    }
    // The neighbours, so the next step's diff is already here.
    for (const neighbour of [step + 1, step - 1]) {
      if (neighbour >= 1 && neighbour < snapshots.length) history(neighbour).catch(() => undefined);
    }
    return () => {
      live = false;
    };
  }, [currentStep, snapshots, history]);

  useEffect(() => {
    editorRef.current?.updateOptions({
      hideUnchangedRegions: { enabled: hideUnchanged }
    } as monaco.editor.IDiffEditorOptions);
  }, [hideUnchanged]);

  // Blame: who last changed each line of the shown snapshot, in its gutter.
  useEffect(() => {
    const editor = editorRef.current;
    if (!editor || !shown?.history || !blameEnabled) return;
    const labels = snapshots.map((snapshot, index) => (index === 0 ? "initial" : snapshot.label));
    installBlameStyles(labels);
    const modifiedEditor = editor.getModifiedEditor();
    const decorations: monaco.editor.IModelDeltaDecoration[] = [];
    let line = 1;
    for (const [step, count] of shown.history.blame) {
      decorations.push({
        range: new monaco.Range(line, 1, line + count - 1, 1),
        options: { isWholeLine: true, linesDecorationsClassName: `blame-mark blame-step-${step}` }
      });
      line += count;
    }
    // Long pass names end in an ellipsis rather than squeeze the code.
    const width = Math.min(18, Math.max(...labels.map((label) => label.length))) + 1;
    modifiedEditor.updateOptions({ lineDecorationsWidth: `${width}ch` } as unknown as monaco.editor.IEditorOptions);
    const collection = modifiedEditor.createDecorationsCollection(decorations);
    return () => {
      collection.clear();
      modifiedEditor.updateOptions({ lineDecorationsWidth: 10 });
    };
  }, [shown, blameEnabled, snapshots]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      const target = event.target as HTMLElement | null;
      if (target?.matches("input, textarea, [contenteditable='true']")) return;
      if (event.key === "ArrowLeft") {
        setCurrentStep((step) => Math.max(0, step - 1));
      } else if (event.key === "ArrowRight") {
        setCurrentStep((step) => Math.min(snapshots.length - 1, step + 1));
      }
    }

    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [snapshots.length]);

  const identical = shown?.step === currentStep && shown.history?.identical === true;
  const note = identical ? "no change" : null;

  return (
    <section className={diffColorsEnabled ? "ir-diff-view" : "ir-diff-view no-diff-colors"}>
      <div className="diff-toolbar">
        <div className={currentSnapshot?.isError ? "step-label error" : "step-label"}>
          {stepLabel}
          {note ? <span className="step-note"> · {note}</span> : null}
          {error ? <span className="step-note error"> · {error}</span> : null}
        </div>
        <div className="diff-toolbar-actions">
          <button type="button" onClick={() => setCurrentStep((step) => Math.max(0, step - 1))} disabled={currentStep <= 0}>
            Prev
          </button>
          <button
            type="button"
            onClick={() => setCurrentStep((step) => Math.min(snapshots.length - 1, step + 1))}
            disabled={currentStep >= snapshots.length - 1}
          >
            Next
          </button>
          <button type="button" onClick={() => setHideUnchanged((enabled) => !enabled)}>
            {hideUnchanged ? "Show All" : "Hide Unchanged"}
          </button>
          <button
            className={blameEnabled ? "active" : ""}
            type="button"
            onClick={() => setBlameEnabled((enabled) => !enabled)}
            disabled={currentStep <= 0}
          >
            {blameEnabled ? "Hide Blame" : "Blame"}
          </button>
          <button className={!diffColorsEnabled ? "active" : ""} type="button" onClick={() => setDiffColorsEnabled((enabled) => !enabled)}>
            {diffColorsEnabled ? "No Colors" : "Show Colors"}
          </button>
        </div>
      </div>
      <div className="diff-body">
        <div className="diff-editor" ref={containerRef} />
        {/* Monaco never hides an unchanged region that is the whole file. */}
        {identical && hideUnchanged ? (
          <div className="diff-identical">
            <span>
              {currentSnapshot?.label ?? "This pass"} changed nothing: the snapshot is the previous one.
            </span>
            <button type="button" onClick={() => setHideUnchanged(false)}>
              Show All
            </button>
          </div>
        ) : null}
      </div>
    </section>
  );
}

function initialDiffStep(snapshots: IrSnapshot[]) {
  return snapshots.length > 1 ? 1 : 0;
}

function rowDecoration(start: number, end: number, className: string): monaco.editor.IModelDeltaDecoration {
  return {
    range: new monaco.Range(start, 1, Math.max(start, end - 1), 1),
    options: { isWholeLine: true, className }
  };
}

/**
 * The line on the other side that `line` of `side` corresponds to, as
 * `[start, end)`: the same line outside the changes; inside a change, the
 * line it is the same op (or block) as, and none if it has no counterpart
 * (it was added or removed).
 */
export function correspondingLines(
  hunks: HistoryHunk[],
  line: number,
  side: "original" | "modified"
): [number, number] | null {
  const [here, there] = side === "original" ? [0, 1] : [1, 0];
  let delta = 0;
  for (const hunk of hunks) {
    const [start, end] = side === "original" ? hunk.original : hunk.modified;
    const otherEnd = side === "original" ? hunk.modified[1] : hunk.original[1];
    if (line < start) break;
    if (line < end) {
      const pair = hunk.pairs.find((candidate) => candidate[here] === line);
      return pair ? [pair[there], pair[there] + 1] : null;
    }
    delta = otherEnd - end;
  }
  return [line + delta, line + delta + 1];
}

/** One gutter label per step: `.blame-step-<i>` shows step i's pass name. */
function installBlameStyles(labels: string[]) {
  const css = labels
    .map(
      (label, index) =>
        `.blame-step-${index}::before { content: ${JSON.stringify(label)}; color: ${
          index === 0 ? "#6b7280" : BLAME_PALETTE[(index - 1) % BLAME_PALETTE.length]
        }; }`
    )
    .join("\n");
  let style = document.getElementById(BLAME_STYLE_ID) as HTMLStyleElement | null;
  if (!style) {
    style = document.createElement("style");
    style.id = BLAME_STYLE_ID;
    document.head.appendChild(style);
  }
  if (style.textContent !== css) style.textContent = css;
}
