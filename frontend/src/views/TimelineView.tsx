import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

import { addIr, profileTimeline } from "../api/client";
import type { Timeline, TraceSnapshot } from "../api/protocol";
import { compact } from "../profile";

type TimelineViewProps = {
  tracePath: string;
  snapshots: TraceSnapshot[];
  /** The trace's timeline summary, if it was measured with sample times. */
  summary: { snapshot: number; samples: number } | null;
  /** Whether the trace carries a profile at all. */
  profiled: boolean;
  /** Show line `line` of snapshot `step` in the Text view. */
  onShowInText: (step: number, line: number) => void;
};

/** The lanes' zones, each lane sorted by start (indices into the columns). */
type Lanes = number[][];

const LANE = 22;
const RULER = 26;

const KIND_HUE: Record<string, number> = { function: 212, loop: 186, block: 150, other: 220 };

export function TimelineView({ tracePath, snapshots, summary, profiled, onShowInText }: TimelineViewProps) {
  const [timeline, setTimeline] = useState<Timeline | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  /** The visible window, in ns from the first sample. */
  const [view, setView] = useState<{ from: number; to: number } | null>(null);
  const [hover, setHover] = useState<{ zone: number; x: number; y: number } | null>(null);
  const [selected, setSelected] = useState<number | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const boxRef = useRef<HTMLDivElement | null>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  const drag = useRef<{ x: number; from: number; to: number; moved: boolean } | null>(null);

  useLayoutEffect(() => {
    const box = boxRef.current;
    if (!box) return;
    const observer = new ResizeObserver(() => setSize({ width: box.clientWidth, height: box.clientHeight }));
    observer.observe(box);
    setSize({ width: box.clientWidth, height: box.clientHeight });
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    setTimeline(null);
    setSelected(null);
    if (!profiled) {
      setMessage("This trace has no profile. Measure a run with crabbit-bpgo to get one.");
      return;
    }
    if (!summary) {
      setMessage("This trace was measured without sample times; measure it again with crabbit-bpgo for a timeline.");
      return;
    }
    const snapshot = snapshots[summary.snapshot];
    if (!snapshot) return;
    setMessage("Reading the run…");
    let live = true;
    let timer: number | undefined;
    addIr(snapshot.ir)
      .then((id) => {
        const load = () =>
          profileTimeline(tracePath, id)
            .then((response) => {
              if (!live) return;
              if (response.status === "ready") {
                setTimeline(response.timeline);
                setView({ from: 0, to: response.timeline.duration });
                setMessage(null);
              } else {
                setMessage(response.message);
                if (response.status === "pending") timer = window.setTimeout(load, 1500);
              }
            })
            .catch((err: unknown) => live && setMessage(err instanceof Error ? err.message : String(err)));
        load();
      })
      .catch((err: unknown) => live && setMessage(err instanceof Error ? err.message : String(err)));
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [tracePath, snapshots, summary, profiled]);

  const lanes: Lanes = useMemo(() => {
    if (!timeline) return [];
    const out: Lanes = Array.from({ length: timeline.lanes }, () => []);
    timeline.zones.lane.forEach((lane, zone) => out[lane].push(zone));
    for (const lane of out) lane.sort((a, b) => timeline.zones.start[a] - timeline.zones.start[b]);
    return out;
  }, [timeline]);

  // Draw: the ruler, then each lane's zones in the window; zones narrower
  // than a pixel share one mark per pixel.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !timeline || !view || size.width === 0) return;
    const ratio = window.devicePixelRatio || 1;
    const height = RULER + timeline.lanes * LANE + 8;
    canvas.width = size.width * ratio;
    canvas.height = height * ratio;
    canvas.style.width = `${size.width}px`;
    canvas.style.height = `${height}px`;
    const g = canvas.getContext("2d");
    if (!g) return;
    g.setTransform(ratio, 0, 0, ratio, 0, 0);
    g.fillStyle = "#16181c";
    g.fillRect(0, 0, size.width, height);
    const span = view.to - view.from;
    const toX = (t: number) => ((t - view.from) / span) * size.width;

    // Ruler.
    const step = tickStep(span, size.width);
    g.fillStyle = "#8b949e";
    g.strokeStyle = "#2f353d";
    g.font = "11px system-ui, sans-serif";
    g.textBaseline = "middle";
    for (let t = Math.ceil(view.from / step) * step; t <= view.to; t += step) {
      const x = Math.round(toX(t)) + 0.5;
      g.beginPath();
      g.moveTo(x, RULER - 6);
      g.lineTo(x, height);
      g.stroke();
      g.fillText(formatTime(t), x + 3, RULER / 2);
    }

    const { start, end, place } = timeline.zones;
    g.font = "11px ui-monospace, SFMono-Regular, Consolas, monospace";
    lanes.forEach((zones, lane) => {
      const y = RULER + lane * LANE;
      let drawnTo = -Infinity;
      // The first zone that may be visible: binary search on starts.
      let lo = 0;
      let hi = zones.length;
      while (lo < hi) {
        const mid = (lo + hi) >> 1;
        if (end[zones[mid]] < view.from) lo = mid + 1;
        else hi = mid;
      }
      for (let k = lo; k < zones.length; k++) {
        const zone = zones[k];
        if (start[zone] > view.to) break;
        const x0 = Math.max(0, toX(start[zone]));
        const x1 = Math.min(size.width, toX(end[zone]));
        if (x1 <= drawnTo) continue;
        const x = Math.max(x0, drawnTo);
        const width = Math.max(1, x1 - x);
        const info = timeline.places[place[zone]];
        g.fillStyle = zone === selected ? "#f3f4f6" : zoneColor(info.kind, place[zone]);
        g.fillRect(x, y, width, LANE - 2);
        drawnTo = x + width;
        if (width > 40) {
          g.save();
          g.beginPath();
          g.rect(x + 3, y, width - 6, LANE - 2);
          g.clip();
          g.fillStyle = zone === selected ? "#11141a" : "#f3f4f6";
          g.fillText(info.name, x + 4, y + (LANE - 2) / 2);
          g.restore();
        }
      }
    });
  }, [timeline, view, size, lanes, selected]);

  /** The zone at `(x, y)` on the canvas, if any. */
  const zoneAt = useCallback(
    (x: number, y: number): number | null => {
      if (!timeline || !view) return null;
      const lane = Math.floor((y - RULER) / LANE);
      if (lane < 0 || lane >= lanes.length) return null;
      const t = view.from + (x / size.width) * (view.to - view.from);
      // Within a pixel of the cursor, the zone holding the time.
      const slack = (view.to - view.from) / size.width;
      const zones = lanes[lane];
      let lo = 0;
      let hi = zones.length;
      while (lo < hi) {
        const mid = (lo + hi) >> 1;
        if (timeline.zones.end[zones[mid]] < t - slack) lo = mid + 1;
        else hi = mid;
      }
      const zone = zones[lo];
      return zone !== undefined && timeline.zones.start[zone] <= t + slack ? zone : null;
    },
    [timeline, view, lanes, size.width]
  );

  const local = (event: React.MouseEvent | React.WheelEvent) => {
    const rect = canvasRef.current!.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  };

  const zoomTo = (zone: number) => {
    if (!timeline) return;
    const from = timeline.zones.start[zone];
    const to = timeline.zones.end[zone];
    const pad = (to - from) * 0.05;
    setView({ from: Math.max(0, from - pad), to: Math.min(timeline.duration, to + pad) });
  };

  const onWheel = (event: React.WheelEvent) => {
    if (!timeline || !view) return;
    const { x } = local(event);
    const span = view.to - view.from;
    const at = view.from + (x / size.width) * span;
    const factor = Math.exp(event.deltaY * 0.0015);
    const next = Math.min(timeline.duration, Math.max(1000, span * factor));
    let from = at - (x / size.width) * next;
    from = Math.max(0, Math.min(from, timeline.duration - next));
    setView({ from, to: from + next });
  };

  const onMouseDown = (event: React.MouseEvent) => {
    if (!view) return;
    drag.current = { x: event.clientX, from: view.from, to: view.to, moved: false };
  };

  const onMouseMove = (event: React.MouseEvent) => {
    const { x, y } = local(event);
    const current = drag.current;
    if (current && timeline) {
      const dx = event.clientX - current.x;
      if (Math.abs(dx) > 2) current.moved = true;
      const span = current.to - current.from;
      let from = current.from - (dx / size.width) * span;
      from = Math.max(0, Math.min(from, timeline.duration - span));
      setView({ from, to: from + span });
      setHover(null);
      return;
    }
    const zone = zoneAt(x, y);
    setHover(zone === null ? null : { zone, x: event.clientX, y: event.clientY });
  };

  const onMouseUp = (event: React.MouseEvent) => {
    const current = drag.current;
    drag.current = null;
    if (current?.moved) return;
    const { x, y } = local(event);
    setSelected(zoneAt(x, y));
  };

  const info = (zone: number) => {
    const t = timeline!;
    const place = t.places[t.zones.place[zone]];
    const duration = t.zones.end[zone] - t.zones.start[zone];
    return { place, duration, samples: t.zones.samples[zone], start: t.zones.start[zone] };
  };

  return (
    <section className="timeline-view">
      <div className="flame-toolbar">
        <div className="flame-title">
          Timeline
          {timeline ? (
            <span className="flame-total">
              {" "}
              · {formatTime(timeline.duration)} · {compact(timeline.samples)} samples · as executed (
              {snapshots[summary?.snapshot ?? 0]?.label ?? "last snapshot"})
            </span>
          ) : null}
        </div>
        {view && timeline ? (
          <span className="flame-hint">
            {formatTime(view.from)} – {formatTime(view.to)}
          </span>
        ) : null}
        <button
          type="button"
          onClick={() => timeline && setView({ from: 0, to: timeline.duration })}
          disabled={!timeline || (view?.from === 0 && view?.to === timeline.duration)}
        >
          Whole run
        </button>
      </div>
      <div
        className="timeline-canvas"
        ref={boxRef}
        onMouseLeave={() => {
          drag.current = null;
          setHover(null);
        }}
      >
        {message ? <div className="flame-message">{message}</div> : null}
        <canvas
          ref={canvasRef}
          onWheel={onWheel}
          onMouseDown={onMouseDown}
          onMouseMove={onMouseMove}
          onMouseUp={onMouseUp}
          onDoubleClick={(event) => {
            const { x, y } = local(event);
            const zone = zoneAt(x, y);
            if (zone !== null) zoomTo(zone);
          }}
        />
        {hover && timeline ? (
          <div className="flame-tooltip" style={{ left: Math.min(hover.x + 14, window.innerWidth - 440), top: hover.y + 16 }}>
            {(() => {
              const zone = info(hover.zone);
              return (
                <>
                  <div className="flame-tooltip-name">{zone.place.name}</div>
                  <div>
                    {formatTime(zone.duration)} · {compact(zone.samples)} samples ·{" "}
                    {((100 * zone.samples) / timeline.samples).toFixed(1)}% of the run
                  </div>
                  <div className="flame-tooltip-kind">
                    {zone.place.kind} · from {formatTime(zone.start)}
                    {zone.place.line ? ` · line ${zone.place.line}` : ""}
                  </div>
                </>
              );
            })()}
          </div>
        ) : null}
      </div>
      <div className="flame-details">
        {selected !== null && timeline ? (
          (() => {
            const zone = info(selected);
            return (
              <>
                <span className={`flame-kind kind-${zone.place.kind}`}>{zone.place.kind}</span>
                <span className="flame-details-name">{zone.place.name}</span>
                <span>
                  {formatTime(zone.duration)} from {formatTime(zone.start)} · {compact(zone.samples)} samples
                </span>
                <button type="button" onClick={() => zoomTo(selected)}>
                  Zoom to it
                </button>
                {zone.place.line && summary ? (
                  <button type="button" onClick={() => onShowInText(summary.snapshot, zone.place.line!)}>
                    Show in Text
                  </button>
                ) : null}
              </>
            );
          })()
        ) : (
          <span className="flame-hint">Wheel to zoom at the cursor, drag to pan, click a zone to select it, double-click to zoom to it.</span>
        )}
      </div>
    </section>
  );
}

/** A tick every 1, 2 or 5 × 10^k ns, about 100 pixels apart. */
function tickStep(span: number, width: number) {
  const target = (span / Math.max(width, 1)) * 100;
  const power = 10 ** Math.floor(Math.log10(Math.max(target, 1)));
  for (const unit of [1, 2, 5, 10]) if (unit * power >= target) return unit * power;
  return 10 * power;
}

function formatTime(ns: number) {
  if (ns >= 1e9) return `${(ns / 1e9).toFixed(ns >= 1e10 ? 1 : 3)} s`;
  if (ns >= 1e6) return `${(ns / 1e6).toFixed(ns >= 1e8 ? 0 : 2)} ms`;
  if (ns >= 1e3) return `${(ns / 1e3).toFixed(ns >= 1e5 ? 0 : 1)} µs`;
  return `${Math.round(ns)} ns`;
}

/** By kind, a shade per place, so neighbouring zones of different places differ. */
function zoneColor(kind: string, place: number) {
  const hue = KIND_HUE[kind] ?? 220;
  const shade = (place * 37) % 5;
  if (kind === "other") return "#555b66";
  return `hsl(${hue + shade * 6 - 12}, ${34 + shade * 4}%, ${30 + shade * 3}%)`;
}
