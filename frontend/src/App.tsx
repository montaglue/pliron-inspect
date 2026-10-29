import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import {
  addIr,
  closeDialects,
  getCapabilities,
  getState,
  getTraces,
  importTrace,
  openDialects,
  openTrace,
  renderDocument,
  setTracesFolder
} from "./api/client";
import type {
  DisplayCapabilities,
  OpenTraceResponse,
  RenderDocument,
  SessionState,
  TraceListResponse,
  TraceSnapshot,
  ViewId
} from "./api/protocol";
import { type Bpgo, bpgoOf, snapshotProfile } from "./profile";
import { FolderIcon } from "./start/FolderPicker";
import { StartView, engineLine } from "./start/StartView";
import { CfgView } from "./views/CfgView";
import { IrDiffView } from "./views/IrDiffView";
import { TextView } from "./views/TextView";
import { TreeView } from "./views/TreeView";
import { VersionsView, type ProjectRow } from "./views/VersionsView";

const SAVED_DIALECTS = "pliron-inspect.dialects";
const SAVED_TRACES = "pliron-inspect.traces";

function saved(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function save(key: string, value: string | null) {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    // Storage may be unavailable (private windows); the choice just is not remembered.
  }
}

/** The dialect engine is changing state: poll the server more often. */
function engineBusy(session: SessionState | null) {
  const state = session?.dialects?.state;
  return state === "starting" || state === "building";
}

export function App() {
  const [session, setSession] = useState<SessionState | null>(null);
  const [screen, setScreen] = useState<"start" | "viewer">("start");
  const [bootError, setBootError] = useState<string | null>(null);

  // Reopen the folders chosen last time in this browser (the server may
  // have restarted since).
  useEffect(() => {
    (async () => {
      let state = await getState();
      const dialects = saved(SAVED_DIALECTS);
      if (!state.dialects && dialects) {
        state = await openDialects(dialects).catch(() => {
          save(SAVED_DIALECTS, null);
          return state;
        });
      }
      const traces = saved(SAVED_TRACES);
      if (!state.traces && traces) {
        state = await setTracesFolder(traces).catch(() => {
          save(SAVED_TRACES, null);
          return state;
        });
      }
      setSession(state);
      setScreen(state.traces ? "viewer" : "start");
    })().catch((err: unknown) => setBootError(err instanceof Error ? err.message : String(err)));
  }, []);

  const refresh = useCallback(() => {
    getState()
      .then(setSession)
      .catch(() => undefined);
  }, []);

  useEffect(() => {
    if (!session) return;
    const timer = window.setTimeout(refresh, engineBusy(session) ? 1500 : 10000);
    return () => window.clearTimeout(timer);
  }, [session, refresh]);

  if (bootError) {
    return <div className="boot-state bad">Cannot reach the pliron-inspect server: {bootError}</div>;
  }
  if (!session) {
    return <div className="boot-state">Connecting…</div>;
  }

  if (screen === "start") {
    return (
      <StartView
        session={session}
        remembered={{ dialects: saved(SAVED_DIALECTS), traces: saved(SAVED_TRACES) }}
        onDialects={async (path) => {
          setSession(await openDialects(path));
          save(SAVED_DIALECTS, path);
        }}
        onCloseDialects={async () => {
          setSession(await closeDialects());
          save(SAVED_DIALECTS, null);
        }}
        onTraces={async (path) => {
          setSession(await setTracesFolder(path));
          save(SAVED_TRACES, path);
        }}
        onContinue={() => setScreen("viewer")}
      />
    );
  }

  return <Viewer key={session.traces ?? ""} session={session} onChangeFolders={() => setScreen("start")} />;
}

function Viewer({ session, onChangeFolders }: { session: SessionState; onChangeFolders: () => void }) {
  const [capabilities, setCapabilities] = useState<DisplayCapabilities | null>(null);
  const [activeView, setActiveView] = useState<ViewId>("text");
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [document, setDocument] = useState<RenderDocument | null>(null);
  const [cfgRootId, setCfgRootId] = useState<string | undefined>(undefined);
  const [traceList, setTraceList] = useState<TraceListResponse | null>(null);
  const [selectedProject, setSelectedProject] = useState<string | null>(null);
  const [activeTrace, setActiveTrace] = useState<OpenTraceResponse | null>(null);
  const [snapshots, setSnapshots] = useState<TraceSnapshot[]>([]);
  const [snapshotStep, setSnapshotStep] = useState(0);
  const [irId, setIrId] = useState<string | null>(null);
  const [tracesLoading, setTracesLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const engineState = session.dialects?.state ?? null;

  useEffect(() => {
    getCapabilities()
      .then(setCapabilities)
      .catch((err: unknown) => setError(err instanceof Error ? err.message : String(err)));
  }, []);

  const refreshTraces = () => {
    setTracesLoading(true);
    getTraces()
      .then((traces) => {
        setTraceList(traces);
        setError(null);
      })
      .catch((err: unknown) => setError(err instanceof Error ? err.message : String(err)))
      .finally(() => setTracesLoading(false));
  };

  useEffect(() => {
    refreshTraces();
  }, []);

  // The server keeps the snapshot shown; the views and the text view's
  // language features refer to it by id.
  const snapshotText = snapshots[snapshotStep]?.ir ?? null;
  useEffect(() => {
    setIrId(null);
    if (snapshotText === null) return;
    let live = true;
    addIr(snapshotText)
      .then((id) => live && setIrId(id))
      .catch((err: unknown) => live && setError(err instanceof Error ? err.message : String(err)));
    return () => {
      live = false;
    };
  }, [snapshotText]);

  useEffect(() => {
    if (activeView !== "cfg" && activeView !== "tree") {
      setDocument(null);
      return;
    }
    if (!irId) return;
    let live = true;
    let timer: number | undefined;
    const render = () =>
      renderDocument({ id: irId, view: activeView, rootId: activeView === "cfg" ? cfgRootId : undefined })
        .then((doc) => {
          if (!live) return;
          setDocument(doc);
          setError(null);
          // While the dialect engine builds or reads the IR, ask again.
          if (doc.diagnostics?.some((diagnostic) => diagnostic.code === "engine-pending")) {
            timer = window.setTimeout(render, 1500);
          }
        })
        .catch((err: unknown) => live && setError(err instanceof Error ? err.message : String(err)));
    render();
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [irId, activeView, cfgRootId, engineState]);

  const activateTrace = (trace: OpenTraceResponse, project?: string) => {
    setActiveTrace(trace);
    setSnapshots(trace.snapshots);
    setSnapshotStep(trace.snapshots.length > 0 ? trace.snapshots.length - 1 : 0);
    setError(null);
    return getTraces().then((list) => {
      setTraceList(list);
      setSelectedProject(project ?? projectForTrace(list, trace));
    });
  };

  const openTraceFile = (filepath: string, project?: string) => {
    setTracesLoading(true);
    openTrace(filepath)
      .then((trace) => activateTrace(trace, project))
      .catch((err: unknown) => setError(err instanceof Error ? err.message : String(err)))
      .finally(() => setTracesLoading(false));
  };

  const openProject = (project: ProjectRow) => {
    const latest = project.versions[0];
    if (latest) {
      openTraceFile(latest.filepath, project.name);
    }
  };

  const importTraceFile = (file: File) => {
    setTracesLoading(true);
    file
      .text()
      .then((contents) => importTrace(file.name, contents))
      .then((trace) => activateTrace(trace))
      .catch((err: unknown) => setError(err instanceof Error ? err.message : String(err)))
      .finally(() => setTracesLoading(false));
  };

  const projectRows = traceList ? projectRowsFromList(traceList) : [];
  const selectedProjectRow = projectRows.find((project) => project.name === selectedProject) ?? null;
  const pipeline = activeTrace?.pipeline ?? [];
  const bpgo = useMemo(() => bpgoOf(activeTrace?.meta), [activeTrace]);
  const profile = useMemo(() => snapshotProfile(bpgo, snapshotStep), [bpgo, snapshotStep]);
  const profileNote = useMemo(() => (bpgo && !profile ? whereProfileStops(bpgo, snapshots) : null), [bpgo, profile, snapshots]);

  return (
    <main className={sidebarCollapsed ? "app-shell sidebar-collapsed" : "app-shell"}>
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-copy">
            <div className="brand-title">pliron-inspect</div>
            <div className="brand-subtitle">pliron compiler traces</div>
          </div>
          <button
            className="sidebar-toggle"
            type="button"
            onClick={() => setSidebarCollapsed((collapsed) => !collapsed)}
            title={sidebarCollapsed ? "Expand sidebar" : "Collapse sidebar"}
            aria-label={sidebarCollapsed ? "Expand sidebar" : "Collapse sidebar"}
          >
            <SidebarToggleIcon collapsed={sidebarCollapsed} />
          </button>
        </div>

        <FoldersPanel session={session} onChange={onChangeFolders} />

        <nav className="view-list" aria-label="Views">
          {(capabilities?.views ?? fallbackViews).map((view) => (
            <button
              className={view.id === activeView ? "view-button active" : "view-button"}
              key={view.id}
              type="button"
              onClick={() => setActiveView(view.id)}
              title={view.description}
            >
              <ViewIcon view={view.id} />
              <span className="view-label">{view.label}</span>
            </button>
          ))}
        </nav>

        <TracePanel
          activeTrace={activeTrace}
          loading={tracesLoading}
          onImportTrace={importTraceFile}
          onOpenProject={openProject}
          onRefresh={refreshTraces}
          projects={projectRows}
          selectedProject={selectedProject}
        />
        <PipelinePanel
          passes={pipeline}
          snapshots={snapshots}
          step={snapshotStep}
          onSelect={(step) => setSnapshotStep(step)}
          profiled={bpgo}
        />
      </aside>

      <section className="workspace">
        <div className="workspace-body">
          {error ? <div className="error-banner">{error}</div> : null}

          {activeView === "text" ? (
            <TextView
              snapshots={snapshots}
              step={snapshotStep}
              onStepChange={setSnapshotStep}
              irId={irId}
              engineState={engineState}
              profile={profile}
              profileNote={profileNote}
            />
          ) : activeView === "ir-diff" ? (
            snapshots.length > 0 ? (
              <IrDiffView snapshots={snapshots} />
            ) : (
              <div className="no-trace-state">
                <TraceIcon />
                <span>Select a trace to inspect IR snapshots</span>
              </div>
            )
          ) : activeView === "versions" ? (
            <VersionsView
              activeTrace={activeTrace}
              loading={tracesLoading}
              onOpenVersion={(filepath) => openTraceFile(filepath, selectedProjectRow?.name)}
              project={selectedProjectRow}
            />
          ) : snapshots.length === 0 ? (
            <div className="no-trace-state">
              <TraceIcon />
              <span>Select a trace to inspect IR snapshots</span>
            </div>
          ) : activeView === "cfg" ? (
            <CfgView
              document={document}
              rootId={cfgRootId}
              onSelectRoot={setCfgRootId}
              snapshot={snapshotLabel(snapshots, snapshotStep)}
              profile={profile}
            />
          ) : (
            <TreeView document={document} snapshot={snapshotLabel(snapshots, snapshotStep)} />
          )}
        </div>
      </section>
    </main>
  );
}

/** Where the trace's profile stops, in words (for a snapshot it misses). */
function whereProfileStops(bpgo: Bpgo, snapshots: TraceSnapshot[]): string {
  const stops = Object.values(bpgo.stops ?? {});
  const stop = stops.filter((s) => s.pass).sort((a, b) => b.reached - a.reached)[0];
  if (!stop) return "This snapshot has no profile.";
  const label = snapshots[stop.reached]?.label ?? `snapshot ${stop.reached}`;
  return `The profile reaches back to ${label}: ${stop.reason}.`;
}

function snapshotLabel(snapshots: TraceSnapshot[], step: number) {
  const snapshot = snapshots[step];
  if (!snapshot) return undefined;
  return `${step === 0 ? "initial" : snapshot.label} (${step + 1}/${snapshots.length})`;
}

/** The chosen folders, and the way back to the start page. */
function FoldersPanel({ session, onChange }: { session: SessionState; onChange: () => void }) {
  const dialects = session.dialects;
  const engine = dialects ? engineLine(dialects.state, dialects.message) : null;
  const name = (path: string | null | undefined) => (path ? path.split("/").filter(Boolean).pop() : null);
  return (
    <button className="folders-panel" type="button" onClick={onChange} title="Change the folders">
      <span className="folders-row" title={dialects?.root}>
        <FolderIcon />
        <span className="folders-label">Dialects</span>
        <span className="folders-name">{name(dialects?.root) ?? "none"}</span>
        <span className={`engine-dot ${engine?.tone ?? "off"}`} title={engine?.text ?? "No dialects folder"} />
      </span>
      <span className="folders-row" title={session.traces ?? undefined}>
        <TraceIcon />
        <span className="folders-label">Traces</span>
        <span className="folders-name">{name(session.traces) ?? "none"}</span>
      </span>
      <span className="folders-change">Change folders…</span>
    </button>
  );
}

function TracePanel({
  activeTrace,
  loading,
  onOpenProject,
  onImportTrace,
  onRefresh,
  projects,
  selectedProject
}: {
  activeTrace: OpenTraceResponse | null;
  loading: boolean;
  onImportTrace: (file: File) => void;
  onOpenProject: (project: ProjectRow) => void;
  onRefresh: () => void;
  projects: ProjectRow[];
  selectedProject: string | null;
}) {
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  return (
    <section className="trace-panel" aria-label="Trace projects">
      <div className="trace-header">
        <div className="trace-title-row">
          <TraceIcon />
          <span>Projects</span>
        </div>
        <input
          ref={fileInputRef}
          className="trace-file-input"
          type="file"
          accept=".crt"
          onChange={(event) => {
            const file = event.currentTarget.files?.[0];
            event.currentTarget.value = "";
            if (file) onImportTrace(file);
          }}
        />
        <div className="trace-actions">
          <button
            className="trace-action trace-import-action"
            type="button"
            onClick={() => fileInputRef.current?.click()}
            disabled={loading}
            title="Import a .crt trace into the traces folder"
          >
            <FolderPlusIcon />
          </button>
          <button className="trace-action" type="button" onClick={onRefresh} disabled={loading} title="Refresh traces">
            <RefreshIcon />
          </button>
        </div>
      </div>

      <button className="trace-active" type="button" disabled title={activeTrace?.filepath}>
        <span className="trace-active-label">Active</span>
        <span className="trace-active-name">
          {activeTrace ? `${selectedProject ?? "?"} · ${activeTrace.filename}` : "No trace selected"}
        </span>
      </button>

      <div className="trace-list-header">
        <span>{loading ? "Loading" : "Available"}</span>
        <span>{projects.length}</span>
      </div>

      <div className="trace-list">
        {projects.length === 0 ? <div className="trace-empty">No traces in this folder</div> : null}
        {projects.map((project) => (
          <button
            className={project.name === selectedProject ? "trace-row selected" : "trace-row"}
            key={project.name}
            type="button"
            onClick={() => onOpenProject(project)}
            title={`${project.name} — ${project.versions.length} version${project.versions.length === 1 ? "" : "s"}`}
          >
            <TraceFileIcon />
            <span className="trace-name">{project.name}</span>
            <span className="trace-status">{project.versions.length}</span>
          </button>
        ))}
      </div>
    </section>
  );
}

/** The trace's passes; a pass shows the IR after it. */
function PipelinePanel({
  passes,
  snapshots,
  step,
  onSelect,
  profiled
}: {
  passes: string[];
  snapshots: TraceSnapshot[];
  step: number;
  onSelect: (step: number) => void;
  /** The trace's profile: passes whose output has one are marked. */
  profiled: Bpgo | null;
}) {
  // Snapshot 0 is the input; snapshot i + 1 is the IR after pass i.
  const aligned = snapshots.length === passes.length + 1;
  const withProfile = new Set(Object.keys(profiled?.snapshots ?? {}).map(Number));
  return (
    <section className="pipeline-panel" aria-label="Pass pipeline">
      <div className="pipeline-header">
        <div className="pipeline-title-row">
          <PipelineIcon />
          <span>Pipeline</span>
        </div>
        <span className="pipeline-count">{passes.length || ""}</span>
      </div>

      <div className="pipeline-list">
        {passes.length === 0 ? <div className="pipeline-empty">No pipeline recorded</div> : null}
        {passes.map((passName, index) => (
          <button
            className={[
              "pipeline-pass",
              aligned && step === index + 1 ? "current" : "",
              withProfile.has(index + 1) ? "profiled" : ""
            ].join(" ")}
            key={`${passName}-${index}`}
            type="button"
            disabled={!aligned}
            onClick={() => onSelect(index + 1)}
            title={
              (aligned ? `Show the IR after ${passName}` : passName) +
              (withProfile.has(index + 1) ? " (has a measured profile)" : "")
            }
          >
            <span className="pipeline-pass-index">{index + 1}</span>
            <span className="pipeline-pass-name">{passName}</span>
          </button>
        ))}
      </div>
    </section>
  );
}

function projectRowsFromList(traceList: TraceListResponse): ProjectRow[] {
  return traceList.projects
    .map((project) => ({
      name: project.name,
      versions: [...project.versions].sort(
        (left, right) =>
          (right.modifiedMs ?? 0) - (left.modifiedMs ?? 0) || right.filename.localeCompare(left.filename)
      )
    }))
    .sort((left, right) => left.name.localeCompare(right.name));
}

function projectForTrace(traceList: TraceListResponse, trace: OpenTraceResponse): string | null {
  const match = projectRowsFromList(traceList).find((project) =>
    project.versions.some((version) => version.filepath === trace.filepath)
  );
  return match?.name ?? (trace.meta.name || null);
}

function ViewIcon({ view }: { view: ViewId }) {
  switch (view) {
    case "ir-diff":
      return <DiffIcon />;
    case "cfg":
      return <CfgIcon />;
    case "tree":
      return <TreeIcon />;
    case "text":
      return <TextIcon />;
    case "versions":
      return <VersionsIcon />;
  }
}

function SvgIcon({ children }: { children: ReactNode }) {
  return (
    <svg className="view-icon" viewBox="0 0 24 24" aria-hidden="true">
      {children}
    </svg>
  );
}

function DiffIcon() {
  return (
    <SvgIcon>
      <path d="M6 4v16" />
      <path d="M18 4v16" />
      <path d="M9 8h5" />
      <path d="M12 5l3 3-3 3" />
      <path d="M15 16h-5" />
      <path d="M12 13l-3 3 3 3" />
    </SvgIcon>
  );
}

function CfgIcon() {
  return (
    <SvgIcon>
      <circle cx="7" cy="6" r="2.5" />
      <circle cx="17" cy="12" r="2.5" />
      <circle cx="7" cy="18" r="2.5" />
      <path d="M9.2 7.3 14.8 10.7" />
      <path d="M14.8 13.3 9.2 16.7" />
    </SvgIcon>
  );
}

function TreeIcon() {
  return (
    <SvgIcon>
      <path d="M12 5v5" />
      <path d="M7 14v5" />
      <path d="M17 14v5" />
      <path d="M12 10H7v4" />
      <path d="M12 10h5v4" />
      <circle cx="12" cy="4" r="2" />
      <circle cx="7" cy="20" r="2" />
      <circle cx="17" cy="20" r="2" />
    </SvgIcon>
  );
}

function TextIcon() {
  return (
    <SvgIcon>
      <path d="M7 4h7l3 3v13H7z" />
      <path d="M14 4v4h4" />
      <path d="M9 12h6" />
      <path d="M9 16h6" />
    </SvgIcon>
  );
}

function SidebarToggleIcon({ collapsed }: { collapsed: boolean }) {
  return (
    <svg className="sidebar-toggle-icon" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M4 5h16v14H4z" />
      <path d="M9 5v14" />
      {collapsed ? <path d="m13 9 3 3-3 3" /> : <path d="m16 9-3 3 3 3" />}
    </svg>
  );
}

function PipelineIcon() {
  return (
    <svg className="pipeline-icon" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M5 5h14" />
      <path d="M5 12h14" />
      <path d="M5 19h14" />
      <path d="M8 3v4" />
      <path d="M16 10v4" />
      <path d="M11 17v4" />
    </svg>
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

function VersionsIcon() {
  return (
    <SvgIcon>
      <path d="M8 4h9v13" />
      <path d="M5 7h9v13H5z" />
      <path d="M8 12h3" />
      <path d="M8 15h5" />
    </SvgIcon>
  );
}

function TraceFileIcon() {
  return (
    <svg className="trace-file-icon" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M7 4h7l3 3v13H7z" />
      <path d="M14 4v4h4" />
      <path d="M9 13h6" />
      <path d="M9 16h4" />
    </svg>
  );
}

function FolderPlusIcon() {
  return (
    <svg className="trace-action-icon" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M4 7h7l2 2h7v10H4z" />
      <path d="M14 14h4" />
      <path d="M16 12v4" />
    </svg>
  );
}

function RefreshIcon() {
  return (
    <svg className="trace-action-icon" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M20 6v5h-5" />
      <path d="M4 18v-5h5" />
      <path d="M18 9a6 6 0 0 0-10-3l-4 4" />
      <path d="M6 15a6 6 0 0 0 10 3l4-4" />
    </svg>
  );
}

const fallbackViews: Array<{ id: ViewId; label: string; description: string }> = [
  { id: "text", label: "Text", description: "IR snapshots" },
  { id: "ir-diff", label: "IR Diff", description: "Snapshot diff" },
  { id: "cfg", label: "CFG", description: "Control-flow graph of a function" },
  { id: "tree", label: "Tree", description: "Operation hierarchy" },
  { id: "versions", label: "Versions", description: "Trace versions of the selected project" }
];
