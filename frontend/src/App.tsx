import { useEffect, useRef, useState, type ReactNode } from "react";

import { getCapabilities, getTraces, importTrace, openTrace, renderDocument } from "./api/client";
import type { DisplayCapabilities, OpenTraceResponse, RenderDocument, TraceListResponse, TraceSnapshot, ViewId } from "./api/protocol";
import { CfgView } from "./views/CfgView";
import { IrDiffView } from "./views/IrDiffView";
import { TextView } from "./views/TextView";
import { TreeView } from "./views/TreeView";
import { VersionsView, type ProjectRow, type VersionRow } from "./views/VersionsView";

const sampleSnapshots: TraceSnapshot[] = [
  {
    label: "initial",
    ir: `builtin.module {
  llvm.func @main() {
  ^entry:
    %0 = llvm.mlir.constant(1) : builtin.integeri32
    llvm.return
  }
}`
  },
  {
    label: "convert-arith-to-llvm",
    ir: `builtin.module {
  llvm.func @main() {
  ^entry:
    %0 = llvm.mlir.constant(1) : builtin.integeri32
    %1 = llvm.mlir.constant(41) : builtin.integeri32
    %2 = llvm.add %0, %1 : builtin.integeri32
    llvm.return
  }
}`
  },
  {
    label: "lower-cf",
    ir: `builtin.module {
  llvm.func @main() {
  ^entry:
    %0 = llvm.mlir.constant(1) : builtin.integeri32
    %1 = llvm.mlir.constant(41) : builtin.integeri32
    %2 = llvm.add %0, %1 : builtin.integeri32
    llvm.br ^exit
  ^exit:
    llvm.return
  }
}`
  }
];

const samplePipeline = [
  "verify",
  "convert-arith-to-llvm",
  "convert-cf-to-llvm",
  "lower-llvm-block-args-to-phi"
];


export function App() {
  const [capabilities, setCapabilities] = useState<DisplayCapabilities | null>(null);
  const [activeView, setActiveView] = useState<ViewId>("text");
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [document, setDocument] = useState<RenderDocument | null>(null);
  const [cfgRootId, setCfgRootId] = useState<string | undefined>(undefined);
  const [traceList, setTraceList] = useState<TraceListResponse | null>(null);
  const [selectedProject, setSelectedProject] = useState<string | null>(null);
  const [activeTrace, setActiveTrace] = useState<OpenTraceResponse | null>(null);
  const [snapshots, setSnapshots] = useState<TraceSnapshot[]>([]);
  const [pipeline, setPipeline] = useState<string[]>(samplePipeline);
  const [tracesLoading, setTracesLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

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

  useEffect(() => {
    if (activeView === "text" || activeView === "ir-diff" || activeView === "versions") {
      setDocument(null);
      return;
    }

    renderDocument({
      view: activeView,
      snapshotId: activeTrace?.filepath ?? "none",
      rootId: activeView === "cfg" ? cfgRootId : undefined,
      text: snapshots[snapshots.length - 1]?.ir ?? "",
      options: { language: "stair-ir" }
    })
      .then((doc) => {
        setDocument(doc);
        setError(null);
      })
      .catch((err: unknown) => setError(err instanceof Error ? err.message : String(err)));
  }, [activeTrace?.filepath, activeView, cfgRootId, snapshots]);

  const activateTrace = (trace: OpenTraceResponse, project?: string) => {
    setActiveTrace(trace);
    setSnapshots(trace.snapshots);
    setCfgRootId(undefined);
    if (trace.pipeline.length > 0) {
      setPipeline(trace.pipeline);
    }
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

  return (
    <main className={sidebarCollapsed ? "app-shell sidebar-collapsed" : "app-shell"}>
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-copy">
            <div className="brand-title">pliron-inspect</div>
            <div className="brand-subtitle">Render-document prototype</div>
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

        <nav className="view-list" aria-label="Views">
          {(capabilities?.views ?? fallbackViews).map((view) => (
            <button
              className={view.id === activeView ? "view-button active" : "view-button"}
              key={view.id}
              type="button"
              onClick={() => setActiveView(view.id)}
              title={view.label}
            >
              <ViewIcon view={view.id} />
              <span className="view-label">{view.label}</span>
              <small>{view.id}</small>
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
        <PipelinePanel passes={pipeline} />
      </aside>

      <section className="workspace">
        <div className="workspace-body">
          {error ? <div className="error-banner">{error}</div> : null}

          {activeView === "text" ? (
            <TextView snapshots={snapshots} />
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
          ) : activeView === "cfg" ? (
            <CfgView document={document} rootId={cfgRootId} onSelectRoot={setCfgRootId} />
          ) : activeView === "tree" ? (
            <TreeView document={document} />
          ) : (
            <section className="content-grid">
              <article className="panel">
                <h2>Document</h2>
                <pre>{JSON.stringify(document, null, 2)}</pre>
              </article>

              <article className="panel">
                <h2>Renderer Target</h2>
                <RendererPreview document={document} />
              </article>
            </section>
          )}
        </div>
      </section>
    </main>
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
          accept=".stx"
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
            title="Import .stx trace"
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
        {projects.length === 0 ? <div className="trace-empty">No trace projects found</div> : null}
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

function PipelinePanel({ passes }: { passes: string[] }) {
  return (
    <section className="pipeline-panel" aria-label="Pass pipeline">
      <div className="pipeline-header">
        <div className="pipeline-title-row">
          <PipelineIcon />
          <span>Pipeline</span>
        </div>
        <span className="pipeline-lock" title="Locked">
          <LockIcon />
        </span>
      </div>

      <div className="pipeline-list" aria-disabled="true">
        {passes.length === 0 ? <div className="pipeline-empty">No pipeline recorded</div> : null}
        {passes.map((passName, index) => (
          <div className="pipeline-pass" key={`${passName}-${index}`}>
            <GripIcon />
            <span className="pipeline-pass-name">{passName}</span>
            <button className="pipeline-delete" type="button" disabled title="Delete pass">
              <XIcon />
            </button>
          </div>
        ))}
      </div>

      <button className="pipeline-add" type="button" disabled>
        <PlusIcon />
        <span>Add pass</span>
      </button>
    </section>
  );
}

function projectRowsFromList(traceList: TraceListResponse): ProjectRow[] {
  const byName = new Map<string, VersionRow[]>();
  const add = (projects: TraceListResponse["libraryProjects"], status: VersionRow["status"]) => {
    for (const project of projects) {
      const versions = byName.get(project.name) ?? [];
      versions.push(...project.versions.map((version) => ({ ...version, status })));
      byName.set(project.name, versions);
    }
  };
  add(traceList.libraryProjects, "imported");
  add(traceList.tempProjects, "temp");

  return [...byName.entries()]
    .map(([name, versions]) => {
      // A temp trace and its imported library copy share a filename within a
      // project — show it once, preferring the imported (mutable) copy.
      const byFilename = new Map<string, VersionRow>();
      for (const version of versions) {
        const existing = byFilename.get(version.filename);
        if (!existing) {
          byFilename.set(version.filename, version);
          continue;
        }
        // Importing copies the file, so the library copy's mtime is the
        // import time, not the compile time — keep the earliest.
        const times = [existing.modifiedMs, version.modifiedMs].filter(
          (time): time is number => typeof time === "number" && time > 0
        );
        const preferred = existing.status === "imported" ? existing : version;
        byFilename.set(version.filename, {
          ...preferred,
          modifiedMs: times.length > 0 ? Math.min(...times) : preferred.modifiedMs
        });
      }
      return {
        name,
        versions: [...byFilename.values()].sort(
          (left, right) =>
            (right.modifiedMs ?? 0) - (left.modifiedMs ?? 0) || right.filename.localeCompare(left.filename)
        )
      };
    })
    .sort((left, right) => left.name.localeCompare(right.name));
}

function projectForTrace(traceList: TraceListResponse, trace: OpenTraceResponse): string | null {
  const match = projectRowsFromList(traceList).find((project) =>
    project.versions.some(
      (version) => version.filepath === trace.filepath || version.filepath === trace.importedFrom
    )
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
    case "egraph":
      return <EGraphIcon />;
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

function EGraphIcon() {
  return (
    <SvgIcon>
      <circle cx="8" cy="8" r="3" />
      <circle cx="16" cy="8" r="3" />
      <circle cx="12" cy="16" r="3" />
      <path d="M10.6 8h2.8" />
      <path d="M9.5 10.5 11 13.2" />
      <path d="M14.5 10.5 13 13.2" />
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

function GripIcon() {
  return (
    <svg className="pipeline-grip" viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="9" cy="7" r="1" />
      <circle cx="15" cy="7" r="1" />
      <circle cx="9" cy="12" r="1" />
      <circle cx="15" cy="12" r="1" />
      <circle cx="9" cy="17" r="1" />
      <circle cx="15" cy="17" r="1" />
    </svg>
  );
}

function LockIcon() {
  return (
    <svg className="pipeline-lock-icon" viewBox="0 0 24 24" aria-hidden="true">
      <rect x="6" y="10" width="12" height="10" rx="2" />
      <path d="M9 10V7a3 3 0 0 1 6 0v3" />
    </svg>
  );
}

function XIcon() {
  return (
    <svg className="pipeline-action-icon" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M6 6l12 12" />
      <path d="M18 6 6 18" />
    </svg>
  );
}

function PlusIcon() {
  return (
    <svg className="pipeline-action-icon" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M12 5v14" />
      <path d="M5 12h14" />
    </svg>
  );
}

function RendererPreview({ document }: { document: RenderDocument | null }) {
  if (!document) {
    return <div className="empty">Waiting for render document</div>;
  }

  if (document.diagnostics?.length) {
    return (
      <div className="diagnostics">
        {document.diagnostics.map((diagnostic) => (
          <div className={`diagnostic ${diagnostic.severity}`} key={diagnostic.code}>
            <strong>{diagnostic.code}</strong>
            <span>{diagnostic.message}</span>
          </div>
        ))}
      </div>
    );
  }

  if (document.text) {
    return <pre className="text-preview">{document.text.text}</pre>;
  }

  return <div className="empty">No renderer payload yet</div>;
}

const fallbackViews: Array<{ id: ViewId; label: string; description: string }> = [
  { id: "text", label: "Text", description: "Plain Monaco IR snapshots" },
  { id: "ir-diff", label: "IR Diff", description: "Monaco snapshot diff" },
  { id: "cfg", label: "CFG", description: "React Flow + Graphviz geometry" },
  { id: "tree", label: "Tree", description: "D3 flextree operation hierarchy" },
  { id: "egraph", label: "EGraph", description: "Cytoscape equivalence graph" },
  { id: "versions", label: "Versions", description: "Trace versions of the selected project" }
];
