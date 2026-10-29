import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

import { profileFlamegraph } from "../api/client";
import type { FlameGraph, FlameNode } from "../api/protocol";
import { compact, percent } from "../profile";

type FlameGraphViewProps = {
  tracePath: string;
  step: number;
  /** The server's id of the snapshot's IR, once it has it. */
  irId: string | null;
  /** Whether the trace carries a profile at all. */
  profiled: boolean;
  /** The shown snapshot, for the header. */
  snapshot: string;
  onShowInText: (line: number) => void;
};

/** A box as laid out: its row and its horizontal span (0..1). */
type Box = { node: FlameNode; path: string[]; depth: number; x: number; width: number };

const ROW = 22;
/** Boxes narrower than this many pixels are not drawn. */
const MIN_WIDTH = 0.5;

/** What a pass's own code is, by the pass a decision slice is named after. */
const DECISION_LABELS: Record<string, string> = {
  regalloc: "spill code (register allocation)",
  frame: "prologue/epilogue (frame lowering)",
  abi: "ABI glue (instruction selection)",
  placement: "branches (block placement)",
  midend: "mid-end code without a source",
  unattributed: "unattributed"
};

export function FlameGraphView({ tracePath, step, irId, profiled, snapshot, onShowInText }: FlameGraphViewProps) {
  const [graph, setGraph] = useState<FlameGraph | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [partial, setPartial] = useState<string | null>(null);
  /** The zoomed box, by the names on its path from the root. */
  const [focusPath, setFocusPath] = useState<string[]>([]);
  const [selected, setSelected] = useState<Box | null>(null);
  const [hovered, setHovered] = useState<{ box: Box; x: number; y: number } | null>(null);
  const [query, setQuery] = useState("");
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [width, setWidth] = useState(0);

  useLayoutEffect(() => {
    const element = containerRef.current;
    if (!element) return;
    const observer = new ResizeObserver(() => setWidth(element.clientWidth));
    observer.observe(element);
    setWidth(element.clientWidth);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (!profiled) {
      setGraph(null);
      setMessage("This trace has no profile. Measure a run with crabbit-bpgo to get one.");
      return;
    }
    if (!irId) {
      setMessage("Reading the snapshot…");
      return;
    }
    let live = true;
    let timer: number | undefined;
    const load = () =>
      profileFlamegraph(tracePath, step, irId)
        .then((response) => {
          if (!live) return;
          if (response.status === "ready") {
            setGraph(response.graph);
            setPartial(response.partial ?? null);
            setMessage(null);
            setSelected(null);
          } else {
            setGraph(null);
            setMessage(response.message);
            if (response.status === "pending") timer = window.setTimeout(load, 1500);
          }
        })
        .catch((err: unknown) => live && setMessage(err instanceof Error ? err.message : String(err)));
    load();
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [tracePath, step, irId, profiled]);

  // The focus as far as its path exists in this snapshot's graph (the same
  // function or loop stays zoomed across passes).
  const focus = useMemo(() => {
    if (!graph) return null;
    let node = graph.root;
    const path: string[] = [];
    const ancestors: FlameNode[] = [];
    for (const name of focusPath) {
      const next = node.children?.find((child) => child.name === name);
      if (!next) break;
      ancestors.push(node);
      path.push(name);
      node = next;
    }
    return { node, path, ancestors };
  }, [graph, focusPath]);

  const boxes = useMemo(() => {
    if (!focus || width === 0) return [];
    const out: Box[] = [];
    // The chain above the focus: full width, one row each.
    focus.ancestors.forEach((node, depth) => out.push({ node, path: focus.path.slice(0, depth), depth, x: 0, width: 1 }));
    const top = focus.ancestors.length;
    const lay = (node: FlameNode, path: string[], depth: number, x: number, span: number) => {
      if (span * width < MIN_WIDTH) return;
      out.push({ node, path, depth, x, width: span });
      let at = x;
      for (const child of node.children ?? []) {
        const childSpan = node.cycles > 0 ? (span * child.cycles) / node.cycles : 0;
        lay(child, [...path, child.name], depth + 1, at, childSpan);
        at += childSpan;
      }
    };
    lay(focus.node, focus.path, top, 0, 1);
    return out;
  }, [focus, width]);

  const needle = query.trim().toLowerCase();
  const matches = (node: FlameNode) =>
    needle !== "" && (node.name.toLowerCase().includes(needle) || (node.text ?? "").toLowerCase().includes(needle));
  const matchedCycles = useMemo(() => {
    if (!focus || needle === "") return 0;
    // Matches nested in matches count once.
    const sum = (node: FlameNode): number =>
      matches(node) ? node.cycles : (node.children ?? []).reduce((total, child) => total + sum(child), 0);
    return sum(focus.node);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focus, needle]);

  const total = graph?.total ?? 0;
  const focusCycles = focus?.node.cycles ?? 0;
  const rows = boxes.reduce((max, box) => Math.max(max, box.depth + 1), 0);

  const zoomTo = (box: Box) => {
    setSelected(box);
    if (box.node.children?.length) setFocusPath(box.path);
  };

  return (
    <section className="flame-view">
      <div className="flame-toolbar">
        <div className="flame-title">
          Flamegraph · {snapshot}
          {graph ? <span className="flame-total"> · {compact(total)} cycles</span> : null}
        </div>
        <input
          className="flame-search"
          placeholder="Search ops, blocks, loops…"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
        {needle !== "" && focus ? (
          <span className="flame-matched">
            {compact(matchedCycles)} · {percent(matchedCycles, focusCycles)} of the zoomed box
          </span>
        ) : null}
        <button type="button" onClick={() => setFocusPath([])} disabled={focusPath.length === 0}>
          Reset zoom
        </button>
      </div>
      {partial ? <div className="flame-note">{partial}</div> : null}
      <div className="flame-canvas" ref={containerRef} onMouseLeave={() => setHovered(null)}>
        {message ? <div className="flame-message">{message}</div> : null}
        <div className="flame-boxes" style={{ height: rows * ROW }}>
          {boxes.map((box) => {
            const pixels = box.width * width;
            const dim = needle !== "" && !matches(box.node);
            const isSelected = selected?.node === box.node;
            return (
              <div
                key={`${box.depth}:${box.path.join("/")}:${box.x}`}
                className={`flame-box kind-${box.node.kind}${dim ? " dim" : ""}${isSelected ? " selected" : ""}`}
                style={{
                  left: `${box.x * 100}%`,
                  width: `${box.width * 100}%`,
                  top: box.depth * ROW,
                  background: color(box.node, focusCycles)
                }}
                onClick={() => zoomTo(box)}
                onDoubleClick={() => box.node.line && onShowInText(box.node.line)}
                onMouseMove={(event) => setHovered({ box, x: event.clientX, y: event.clientY })}
              >
                {pixels > 36 ? <span>{label(box.node)}</span> : null}
              </div>
            );
          })}
        </div>
        {hovered ? <Tooltip box={hovered.box} x={hovered.x} y={hovered.y} total={total} focus={focusCycles} /> : null}
      </div>
      <div className="flame-details">
        {selected ? (
          <>
            <span className={`flame-kind kind-${selected.node.kind}`}>{selected.node.kind}</span>
            <span className="flame-details-name">{selected.node.text ?? label(selected.node)}</span>
            <span>
              {compact(selected.node.cycles)} cycles · {percent(selected.node.cycles, total)} of the run
            </span>
            {selected.node.executions ? <span>ran {compact(selected.node.executions)}×</span> : null}
            {selected.node.line ? (
              <button type="button" onClick={() => onShowInText(selected.node.line!)}>
                Show in Text
              </button>
            ) : null}
          </>
        ) : (
          <span className="flame-hint">Click a box to zoom in; double-click, or select and use Show in Text, to see its IR.</span>
        )}
      </div>
    </section>
  );
}

function label(node: FlameNode) {
  return node.kind === "decision" ? DECISION_LABELS[node.name] ?? `${node.name} (own code)` : node.name;
}

/** By kind; ops warm by their share of the zoomed box, like a flamegraph. */
function color(node: FlameNode, focus: number) {
  switch (node.kind) {
    case "module":
      return "#3b4252";
    case "function":
      return "hsl(212, 42%, 38%)";
    case "loop":
      return "hsl(186, 38%, 32%)";
    case "block":
      return "hsl(150, 26%, 30%)";
    case "decision":
      return "hsl(282, 40%, 46%)";
    case "other":
      return "#555b66";
    case "op": {
      const share = focus > 0 ? Math.min(1, node.cycles / focus) : 0;
      const heat = Math.sqrt(share);
      return `hsl(${38 - 34 * heat}, ${62 + 20 * heat}%, ${40 + 6 * heat}%)`;
    }
  }
}

function Tooltip({ box, x, y, total, focus }: { box: Box; x: number; y: number; total: number; focus: number }) {
  const node = box.node;
  const decisions = (node.children ?? []).filter((child) => child.kind === "decision");
  const left = Math.min(x + 14, window.innerWidth - 440);
  return (
    <div className="flame-tooltip" style={{ left, top: y + 16 }}>
      <div className="flame-tooltip-name">{node.text ?? label(node)}</div>
      <div>
        {compact(node.cycles)} cycles · {percent(node.cycles, total)} of the run
        {focus !== total ? ` · ${percent(node.cycles, focus)} of the zoomed box` : ""}
      </div>
      {node.executions ? <div>ran {compact(node.executions)} times</div> : null}
      {decisions.map((decision) => (
        <div key={decision.name}>
          {label(decision)}: {compact(decision.cycles)} cycles
        </div>
      ))}
      <div className="flame-tooltip-kind">
        {node.kind}
        {node.line ? ` · line ${node.line}` : ""}
      </div>
    </div>
  );
}
