import type { Diagnostic } from "../api/protocol";

/** What a drawn view misses (shown above it). */
export function ViewWarning({ diagnostics }: { diagnostics: Diagnostic[] }) {
  return (
    <div className="view-warning">
      {diagnostics.map((diagnostic) => (
        <p key={`${diagnostic.code}-${diagnostic.message}`}>{diagnostic.message}</p>
      ))}
    </div>
  );
}

/** Why a view has nothing to draw (yet): the dialect engine is building,
 * it cannot read the IR, no dialects folder is chosen. */
export function ViewNotice({ diagnostics }: { diagnostics: Diagnostic[] }) {
  const pending = diagnostics.some((diagnostic) => diagnostic.code === "engine-pending");
  return (
    <div className={pending ? "view-notice pending" : "view-notice"}>
      {pending ? <span className="spinner" aria-hidden="true" /> : null}
      <div>
        {diagnostics.map((diagnostic) => (
          <p key={`${diagnostic.code}-${diagnostic.message}`}>{diagnostic.message}</p>
        ))}
      </div>
    </div>
  );
}
