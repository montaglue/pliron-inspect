import { useEffect, useMemo, useRef, useState } from "react";
import * as monaco from "monaco-editor/esm/vs/editor/editor.api";

import type { TraceSnapshot } from "../api/protocol";
import { configureStairMonaco } from "./monacoSetup";

type TextViewProps = {
  snapshots: TraceSnapshot[];
  step?: number;
  onStepChange?: (step: number) => void;
};

export function TextView({ snapshots, step, onStepChange }: TextViewProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null);
  const modelRef = useRef<monaco.editor.ITextModel | null>(null);
  const [localStep, setLocalStep] = useState(0);
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

    configureStairMonaco();

    if (!containerRef.current || editorRef.current) return;

    editorRef.current = monaco.editor.create(containerRef.current, {
      readOnly: true,
      automaticLayout: true,
      theme: "stair-dark",
      language: "stair-ir",
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      wordWrap: "off"
    });

    return () => {
      modelRef.current?.dispose();
      editorRef.current?.dispose();
      modelRef.current = null;
      editorRef.current = null;
    };
  }, [snapshots.length]);

  useEffect(() => {
    const editor = editorRef.current;
    if (!editor || !currentSnapshot) return;

    modelRef.current?.dispose();
    modelRef.current = monaco.editor.createModel(currentSnapshot.ir, currentSnapshot.isError ? "plaintext" : "stair-ir");
    editor.setModel(modelRef.current);
    editor.setScrollTop(0);
    editor.setScrollLeft(0);
  }, [currentSnapshot]);

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
      <div className="text-editor" ref={containerRef} />
    </section>
  );
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
