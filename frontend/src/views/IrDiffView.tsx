import { useCallback, useEffect, useMemo, useRef, useState, type MutableRefObject } from "react";
import * as monaco from "monaco-editor/esm/vs/editor/editor.api";

import type { TraceSnapshot } from "../api/protocol";
import { configureCrabbitMonaco } from "./monacoSetup";

export type IrSnapshot = TraceSnapshot;

type IrDiffViewProps = {
  snapshots: IrSnapshot[];
};

type DiffCache = Record<number, monaco.editor.ILineChange[]>;

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

export function IrDiffView({ snapshots }: IrDiffViewProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const editorRef = useRef<monaco.editor.IStandaloneDiffEditor | null>(null);
  const originalModelRef = useRef<monaco.editor.ITextModel | null>(null);
  const modifiedModelRef = useRef<monaco.editor.ITextModel | null>(null);
  const hiddenContainerRef = useRef<HTMLDivElement | null>(null);
  const hiddenEditorRef = useRef<monaco.editor.IStandaloneDiffEditor | null>(null);
  const diffCacheRef = useRef<DiffCache>({});
  const blameOverlayRef = useRef<HTMLDivElement | null>(null);
  const blameScrollRef = useRef<monaco.IDisposable | null>(null);
  const blameGenerationRef = useRef(0);

  const [currentStep, setCurrentStep] = useState(() => initialDiffStep(snapshots));
  const [hideUnchanged, setHideUnchanged] = useState(true);
  const [blameEnabled, setBlameEnabled] = useState(false);
  const [diffColorsEnabled, setDiffColorsEnabled] = useState(true);

  const currentSnapshot = snapshots[currentStep] ?? null;
  const previousSnapshot = currentStep > 0 ? snapshots[currentStep - 1] : null;

  const stepLabel = useMemo(() => {
    if (!currentSnapshot) return "No snapshots";
    const label = currentSnapshot.isError ? "ERROR" : currentStep === 0 ? "initial" : currentSnapshot.label;
    return `${label}  (${currentStep + 1}/${snapshots.length})`;
  }, [currentSnapshot, currentStep, snapshots.length]);

  const clearBlameOverlay = useCallback(() => {
    blameOverlayRef.current?.remove();
    blameOverlayRef.current = null;
    blameScrollRef.current?.dispose();
    blameScrollRef.current = null;
  }, []);

  useEffect(() => {
    configureCrabbitMonaco();

    if (!containerRef.current || editorRef.current) return;

    editorRef.current = monaco.editor.createDiffEditor(containerRef.current, {
      readOnly: true,
      renderSideBySide: true,
      automaticLayout: true,
      theme: "crabbit-dark",
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      hideUnchangedRegions: {
        enabled: true,
        contextLineCount: 3,
        minimumLineCount: 3,
        revealLineCount: 20
      }
    } as monaco.editor.IDiffEditorConstructionOptions);

    return () => {
      clearBlameOverlay();
      originalModelRef.current?.dispose();
      modifiedModelRef.current?.dispose();
      hiddenEditorRef.current?.dispose();
      hiddenContainerRef.current?.remove();
      editorRef.current?.dispose();
      originalModelRef.current = null;
      modifiedModelRef.current = null;
      hiddenEditorRef.current = null;
      hiddenContainerRef.current = null;
      editorRef.current = null;
    };
  }, [clearBlameOverlay]);

  useEffect(() => {
    setCurrentStep((step) => {
      if (step >= snapshots.length) return initialDiffStep(snapshots);
      return step;
    });
    diffCacheRef.current = {};
  }, [snapshots]);

  useEffect(() => {
    const editor = editorRef.current;
    if (!editor) return;

    clearBlameOverlay();
    updateEditorRightPadding(editor.getModifiedEditor(), 0);

    originalModelRef.current?.dispose();
    modifiedModelRef.current?.dispose();

    const beforeLanguage = previousSnapshot?.isError ? "plaintext" : "crabbit-ir";
    const afterLanguage = currentSnapshot?.isError ? "plaintext" : "crabbit-ir";

    originalModelRef.current = monaco.editor.createModel(previousSnapshot?.ir ?? "", beforeLanguage);
    modifiedModelRef.current = monaco.editor.createModel(currentSnapshot?.ir ?? "", afterLanguage);
    editor.setModel({
      original: originalModelRef.current,
      modified: modifiedModelRef.current
    });

    window.setTimeout(() => {
      editor.layout();
      void updateBlameOverlay();
    }, 0);
    // updateBlameOverlay intentionally reads current refs/state through closures.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentSnapshot, previousSnapshot, clearBlameOverlay]);

  useEffect(() => {
    const editor = editorRef.current;
    if (!editor) return;
    editor.updateOptions({
      hideUnchangedRegions: { enabled: hideUnchanged }
    } as monaco.editor.IDiffEditorOptions);
    void updateBlameOverlay();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hideUnchanged]);

  useEffect(() => {
    if (!diffColorsEnabled) {
      return;
    }
    void updateBlameOverlay();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [blameEnabled, diffColorsEnabled]);

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

  const updateBlameOverlay = useCallback(async () => {
    clearBlameOverlay();

    const editor = editorRef.current;
    if (!editor || !blameEnabled || currentStep === 0 || snapshots.length === 0) return;

    const generation = ++blameGenerationRef.current;
    const blame = await computeBlame(currentStep, snapshots, diffCacheRef, hiddenEditorRef, hiddenContainerRef);
    if (generation !== blameGenerationRef.current || blame.length === 0) return;

    const modifiedEditor = editor.getModifiedEditor();
    const lineHeight = modifiedEditor.getOption(monaco.editor.EditorOption.lineHeight);
    const passNames = [...new Set(blame)];
    const colorMap = new Map<string, string>();
    let colorIndex = 0;

    for (const name of passNames) {
      if (name === "initial") {
        colorMap.set(name, "#555555");
      } else {
        colorMap.set(name, BLAME_PALETTE[colorIndex % BLAME_PALETTE.length]);
        colorIndex++;
      }
    }

    const maxLength = Math.max(...passNames.map((name) => name.length));
    const overlayWidth = Math.ceil((maxLength + 4) * 7.225) + 8;
    const overlay = document.createElement("div");
    overlay.className = "blame-overlay";
    overlay.style.width = `${overlayWidth}px`;

    const inner = document.createElement("div");
    inner.className = "blame-overlay-inner";
    const blameRows: HTMLDivElement[] = [];

    for (let i = 0; i < blame.length; i++) {
      const row = document.createElement("div");
      row.textContent = `│  ${blame[i].padEnd(maxLength)}`;
      row.className = "blame-row";
      row.style.height = `${lineHeight}px`;
      row.style.lineHeight = `${lineHeight}px`;
      row.style.color = colorMap.get(blame[i]) ?? "#9aa4b2";
      inner.appendChild(row);
      blameRows.push(row);
    }

    overlay.appendChild(inner);
    const editorDom = modifiedEditor.getDomNode();
    const overflowGuard = editorDom?.querySelector(".overflow-guard");
    const overlayHost = overflowGuard ?? editorDom;
    if (!overlayHost) return;
    overlayHost.appendChild(overlay);
    blameOverlayRef.current = overlay;

    let lastScrollHeight = 0;
    const positionRows = () => {
      let previousTop = -1;
      for (let i = 0; i < blameRows.length; i++) {
        const top = modifiedEditor.getTopForLineNumber(i + 1);
        if (top === previousTop) {
          blameRows[i].style.display = "none";
        } else {
          blameRows[i].style.display = "";
          blameRows[i].style.top = `${top}px`;
        }
        previousTop = top;
      }
    };

    const syncScroll = () => {
      inner.style.transform = `translateY(${-modifiedEditor.getScrollTop()}px)`;
      const scrollHeight = modifiedEditor.getScrollHeight();
      if (scrollHeight !== lastScrollHeight) {
        lastScrollHeight = scrollHeight;
        positionRows();
      }
    };

    positionRows();
    lastScrollHeight = modifiedEditor.getScrollHeight();
    syncScroll();
    window.setTimeout(() => {
      positionRows();
      syncScroll();
    }, 100);

    blameScrollRef.current = modifiedEditor.onDidScrollChange(syncScroll);
    updateEditorRightPadding(modifiedEditor, overlayWidth);
  }, [blameEnabled, clearBlameOverlay, currentStep, snapshots]);

  const toggleBlame = () => {
    setBlameEnabled((enabled) => {
      const next = !enabled;
      if (!next) {
        clearBlameOverlay();
        updateEditorRightPadding(editorRef.current?.getModifiedEditor() ?? null, 0);
      }
      return next;
    });
  };

  return (
    <section className={diffColorsEnabled ? "ir-diff-view" : "ir-diff-view no-diff-colors"}>
      <div className="diff-toolbar">
        <div className={currentSnapshot?.isError ? "step-label error" : "step-label"}>{stepLabel}</div>
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
          <button className={blameEnabled ? "active" : ""} type="button" onClick={toggleBlame} disabled={currentStep <= 0}>
            {blameEnabled ? "Hide Blame" : "Blame"}
          </button>
          <button className={!diffColorsEnabled ? "active" : ""} type="button" onClick={() => setDiffColorsEnabled((enabled) => !enabled)}>
            {diffColorsEnabled ? "No Colors" : "Show Colors"}
          </button>
        </div>
      </div>
      <div className="diff-editor" ref={containerRef} />
    </section>
  );
}

function initialDiffStep(snapshots: IrSnapshot[]) {
  return snapshots.length > 1 ? 1 : 0;
}

async function computeBlame(
  currentStep: number,
  snapshots: IrSnapshot[],
    diffCacheRef: MutableRefObject<DiffCache>,
    hiddenEditorRef: MutableRefObject<monaco.editor.IStandaloneDiffEditor | null>,
    hiddenContainerRef: MutableRefObject<HTMLDivElement | null>
): Promise<string[]> {
  if (currentStep <= 0) return [];

  const cache = diffCacheRef.current;
  for (let i = 1; i <= currentStep; i++) {
    if (cache[i] === undefined) {
      cache[i] = await computeMonacoDiff(snapshots[i - 1].ir, snapshots[i].ir, hiddenEditorRef, hiddenContainerRef);
    }
  }

  let blame = snapshots[0].ir.split("\n").map(() => "initial");

  for (let step = 1; step <= currentStep; step++) {
    const lineChanges = cache[step];
    if (!lineChanges || lineChanges.length === 0) continue;

    const label = snapshots[step].label;
    const nextBlame: string[] = [];
    let previousIndex = 0;

    for (const change of lineChanges) {
      const unchangedEnd = change.originalEndLineNumber === 0 ? change.originalStartLineNumber : change.originalStartLineNumber - 1;

      while (previousIndex < unchangedEnd) {
        nextBlame.push(blame[previousIndex]);
        previousIndex++;
      }

      if (change.originalEndLineNumber > 0) {
        previousIndex = change.originalEndLineNumber;
      }

      if (change.modifiedEndLineNumber > 0) {
        const count = change.modifiedEndLineNumber - change.modifiedStartLineNumber + 1;
        for (let i = 0; i < count; i++) {
          nextBlame.push(label);
        }
      }
    }

    while (previousIndex < blame.length) {
      nextBlame.push(blame[previousIndex]);
      previousIndex++;
    }

    blame = nextBlame;
  }

  return blame;
}

function computeMonacoDiff(
  previousIr: string,
  currentIr: string,
  hiddenEditorRef: MutableRefObject<monaco.editor.IStandaloneDiffEditor | null>,
  hiddenContainerRef: MutableRefObject<HTMLDivElement | null>
): Promise<monaco.editor.ILineChange[]> {
  if (previousIr === currentIr) return Promise.resolve([]);

  const editor = getHiddenDiffEditor(hiddenEditorRef, hiddenContainerRef);
  const original = monaco.editor.createModel(previousIr, "text/plain");
  const modified = monaco.editor.createModel(currentIr, "text/plain");

  editor.setModel({ original, modified });

  return new Promise((resolve) => {
    const immediate = editor.getLineChanges();
    if (immediate !== null) {
      original.dispose();
      modified.dispose();
      resolve(immediate);
      return;
    }

    let disposable: monaco.IDisposable | null = null;
    const timeout = window.setTimeout(() => {
      disposable?.dispose();
      original.dispose();
      modified.dispose();
      resolve([]);
    }, 5000);

    disposable = editor.onDidUpdateDiff(() => {
      const changes = editor.getLineChanges();
      if (changes === null) return;
      window.clearTimeout(timeout);
      disposable?.dispose();
      original.dispose();
      modified.dispose();
      resolve(changes);
    });
  });
}

function getHiddenDiffEditor(
  hiddenEditorRef: MutableRefObject<monaco.editor.IStandaloneDiffEditor | null>,
  hiddenContainerRef: MutableRefObject<HTMLDivElement | null>
) {
  if (!hiddenEditorRef.current) {
    const container = document.createElement("div");
    container.style.cssText = "position:absolute;left:-9999px;width:1px;height:1px;overflow:hidden;";
    document.body.appendChild(container);
    hiddenContainerRef.current = container;
    hiddenEditorRef.current = monaco.editor.createDiffEditor(container, {
      automaticLayout: false
    });
  }
  return hiddenEditorRef.current;
}

function updateEditorRightPadding(editor: monaco.editor.IStandaloneCodeEditor | null, right: number) {
  editor?.updateOptions({ padding: { right } } as unknown as monaco.editor.IEditorOptions);
}
