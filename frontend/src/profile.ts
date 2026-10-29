// Backward-PGO profiles a trace may carry (written by the compiler's
// backward passes into the trace header, under `bpgo`): per snapshot, the
// measured metrics of its ops, keyed by where each op starts in the
// snapshot's text.
import type { TraceMeta } from "./api/protocol";

export type Metrics = Record<string, number>;

/** One op's profile: `[line, column, metrics]` (1-based). */
export type OpEntry = [number, number, Metrics];

export type Bpgo = {
  measurement?: {
    tool?: string;
    events?: string[];
    period?: number;
    command?: string[] | null;
    samples?: Record<string, number>;
    matched?: Record<string, number>;
  };
  metrics?: Record<string, string>;
  snapshots?: Record<string, OpEntry[]>;
  stops?: Record<string, { reached: number; pass?: string | null; reason: string }>;
  /** The cycle samples one by one (kept by the server): the snapshot whose
   * ops they hit, and how many there are. */
  timeline?: { snapshot: number; samples: number };
};

export function bpgoOf(meta: TraceMeta | undefined | null): Bpgo | null {
  const value = meta?.bpgo;
  return value && typeof value === "object" ? (value as Bpgo) : null;
}

/** A snapshot's profile, by line. */
export type SnapshotProfile = {
  byLine: Map<number, Metrics>;
  /** Total cycles of the snapshot's ops (decision costs included). */
  total: number;
  maxCycles: number;
};

export function snapshotProfile(bpgo: Bpgo | null, step: number): SnapshotProfile | null {
  const entries = bpgo?.snapshots?.[String(step)];
  if (!entries || entries.length === 0) return null;
  const byLine = new Map<number, Metrics>();
  let total = 0;
  let maxCycles = 0;
  for (const [line, , metrics] of entries) {
    const merged = { ...(byLine.get(line) ?? {}) };
    for (const [name, value] of Object.entries(metrics)) {
      merged[name] = name === "executions" ? Math.max(merged[name] ?? 0, value) : (merged[name] ?? 0) + value;
    }
    byLine.set(line, merged);
  }
  for (const metrics of byLine.values()) {
    const cycles = metrics.cycles ?? 0;
    maxCycles = Math.max(maxCycles, cycles);
    for (const [name, value] of Object.entries(metrics)) {
      if (name === "cycles" || name.endsWith(".cycles")) total += value;
    }
  }
  return { byLine, total, maxCycles };
}

/** Sum of the given lines' metrics: costs add up, runs take the largest. */
export function sumLines(profile: SnapshotProfile, lines: number[]): Metrics | null {
  let out: Metrics | null = null;
  for (const line of lines) {
    const metrics = profile.byLine.get(line);
    if (!metrics) continue;
    out = out ?? {};
    for (const [name, value] of Object.entries(metrics)) {
      out[name] = name === "executions" ? Math.max(out[name] ?? 0, value) : (out[name] ?? 0) + value;
    }
  }
  return out;
}

/** 1.04e10 → "10.4G". */
export function compact(value: number): string {
  const units: Array<[number, string]> = [
    [1e12, "T"],
    [1e9, "G"],
    [1e6, "M"],
    [1e3, "k"]
  ];
  for (const [scale, unit] of units) {
    if (Math.abs(value) >= scale) {
      const scaled = value / scale;
      return `${scaled >= 100 ? scaled.toFixed(0) : scaled >= 10 ? scaled.toFixed(1) : scaled.toFixed(2)}${unit}`;
    }
  }
  return value.toFixed(0);
}

export function percent(part: number, whole: number): string {
  if (whole <= 0) return "";
  const share = (100 * part) / whole;
  return share >= 10 ? `${share.toFixed(0)}%` : share >= 0.1 ? `${share.toFixed(1)}%` : "<0.1%";
}

/** Heat level 0..5 of `cycles` against the hottest op. */
export function heatLevel(cycles: number, max: number): number {
  if (cycles <= 0 || max <= 0) return 0;
  const share = Math.sqrt(cycles / max);
  return Math.max(1, Math.min(5, Math.ceil(share * 5)));
}

/** "regalloc.cycles: 1.2G, frame.cycles: 3M" — a pass's own decisions. */
export function decisions(metrics: Metrics): Array<[string, number]> {
  return Object.entries(metrics)
    .filter(([name]) => name.endsWith(".cycles"))
    .sort((a, b) => b[1] - a[1]);
}

/** Markdown describing one op's profile. */
export function describe(metrics: Metrics, total: number): string {
  const rows: string[] = [];
  if (metrics.cycles) rows.push(`**${compact(metrics.cycles)} cycles** (${percent(metrics.cycles, total)} of the run)`);
  if (metrics.executions) rows.push(`ran **${compact(metrics.executions)}** times`);
  for (const [name, value] of decisions(metrics)) {
    rows.push(`${name.replace(/\.cycles$/, "")}: ${compact(value)} cycles of its own code`);
  }
  return rows.join("  \n");
}
