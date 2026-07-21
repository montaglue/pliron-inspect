import type { OpenTraceResponse, TraceFileInfo } from "../api/protocol";

export type VersionRow = TraceFileInfo & {
  status: "imported" | "temp";
};

export type ProjectRow = {
  name: string;
  versions: VersionRow[];
};

type VersionsViewProps = {
  activeTrace: OpenTraceResponse | null;
  loading: boolean;
  onOpenVersion: (filepath: string) => void;
  project: ProjectRow | null;
};

export function VersionsView({ activeTrace, loading, onOpenVersion, project }: VersionsViewProps) {
  if (!project) {
    return (
      <div className="no-trace-state">
        <VersionsIcon />
        <span>Select a project to browse its versions</span>
      </div>
    );
  }

  const isActive = (version: VersionRow) =>
    version.filepath === activeTrace?.filepath || version.filepath === activeTrace?.importedFrom;

  return (
    <section className="versions-view">
      <div className="versions-view-header">
        <VersionsIcon />
        <span className="versions-view-title">{project.name}</span>
        <span className="versions-view-count">
          {project.versions.length} version{project.versions.length === 1 ? "" : "s"}
        </span>
      </div>

      <div className="versions-view-list">
        {project.versions.map((version, index) => (
          <button
            className={isActive(version) ? "version-row active" : "version-row"}
            key={version.filepath}
            type="button"
            disabled={loading}
            onClick={() => onOpenVersion(version.filepath)}
            title={version.filepath}
          >
            <VersionFileIcon />
            <span className="version-name">{version.filename}</span>
            {index === 0 ? <span className="version-latest">latest</span> : null}
            <span className="version-date">{formatModified(version.modifiedMs)}</span>
            <span className={`trace-status ${isActive(version) ? "active" : version.status}`}>
              {isActive(version) ? "active" : version.status}
            </span>
          </button>
        ))}
      </div>
    </section>
  );
}

function formatModified(modifiedMs: number | undefined): string {
  if (!modifiedMs) return "";
  return new Date(modifiedMs).toLocaleString();
}

function VersionsIcon() {
  return (
    <svg className="trace-icon" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M8 4h9v13" />
      <path d="M5 7h9v13H5z" />
      <path d="M8 12h3" />
      <path d="M8 15h5" />
    </svg>
  );
}

function VersionFileIcon() {
  return (
    <svg className="trace-file-icon" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M7 4h7l3 3v13H7z" />
      <path d="M14 4v4h4" />
      <path d="M9 13h6" />
      <path d="M9 16h4" />
    </svg>
  );
}
