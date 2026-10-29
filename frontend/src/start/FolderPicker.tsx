import { useEffect, useState } from "react";

import { listFolders } from "../api/client";
import type { FolderListing, FolderMarks } from "../api/protocol";

export type FolderPickerOptions = {
  title: string;
  hint: string;
  /** Folder to start browsing in (default: the home directory). */
  start?: string | null;
  /** Whether a folder looks like what is being picked (highlighted). */
  good: (marks: FolderMarks) => boolean;
  onPick: (path: string) => void;
};

/**
 * A browser over the server's folders (under the home directory): the page
 * may run on another machine, so the browser's own file dialog cannot pick
 * them.
 */
export function FolderPicker({ options, onClose }: { options: FolderPickerOptions; onClose: () => void }) {
  const [listing, setListing] = useState<FolderListing | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [showHidden, setShowHidden] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const browse = (path?: string | null) => {
    listFolders(path)
      .then((next) => {
        setListing(next);
        setSelected(null);
        setError(null);
      })
      .catch((err: unknown) => {
        // A remembered folder may be gone: fall back to the home directory.
        if (path) browse(null);
        else setError(err instanceof Error ? err.message : String(err));
      });
  };

  useEffect(() => browse(options.start), [options]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const crumbs: Array<[string, string]> = [];
  if (listing) {
    crumbs.push(["~", listing.home]);
    const rel = listing.path.slice(listing.home.length).split("/").filter(Boolean);
    rel.forEach((name, index) => crumbs.push([name, `${listing.home}/${rel.slice(0, index + 1).join("/")}`]));
  }
  const shown = (listing?.folders ?? []).filter(
    (folder) => showHidden || !folder.name.startsWith(".") || folder.marks.cargo || folder.marks.traces > 0
  );
  const target = listing
    ? listing.folders.find((folder) => folder.path === selected) ?? { path: listing.path, marks: listing.marks }
    : null;

  const use = () => {
    if (!target) return;
    onClose();
    options.onPick(target.path);
  };

  return (
    <div className="picker-modal" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <div className="picker-dialog" role="dialog" aria-modal="true" aria-label={options.title}>
        <div className="picker-dialog-head">
          <div>
            <div className="picker-dialog-title">{options.title}</div>
            <div className="picker-dialog-hint">{options.hint}</div>
          </div>
          <button className="picker-dialog-close" type="button" title="Close" onClick={onClose}>
            ×
          </button>
        </div>

        <div className="picker-crumbs">
          {crumbs.map(([name, path], index) => (
            <span key={path}>
              {index > 0 ? <span className="picker-crumb-sep">/</span> : null}
              <button type="button" onClick={() => browse(path)}>
                {name}
              </button>
            </span>
          ))}
        </div>

        <div className="picker-tools">
          <button type="button" disabled={!listing?.parent} onClick={() => browse(listing?.parent)}>
            ↑ Up
          </button>
          <label>
            <input type="checkbox" checked={showHidden} onChange={(event) => setShowHidden(event.currentTarget.checked)} />{" "}
            show hidden folders
          </label>
          <span className="picker-tools-hint">click to select · double-click to open</span>
        </div>

        <div className="picker-list">
          {error ? <div className="picker-empty bad">{error}</div> : null}
          {listing && shown.length === 0 ? <div className="picker-empty">No subfolders here.</div> : null}
          {shown.map((folder) => (
            <div
              className={[
                "picker-row",
                options.good(folder.marks) ? "good" : "",
                selected === folder.path ? "selected" : ""
              ].join(" ")}
              key={folder.path}
              onClick={() => setSelected(folder.path)}
              onDoubleClick={() => browse(folder.path)}
            >
              <FolderIcon />
              <span className="picker-row-name">{folder.name}</span>
              <FolderChips marks={folder.marks} />
              <button
                className="picker-row-enter"
                type="button"
                title="Open"
                onClick={(event) => {
                  event.stopPropagation();
                  browse(folder.path);
                }}
              >
                ›
              </button>
            </div>
          ))}
        </div>

        <div className="picker-foot">
          <div className="picker-foot-target">
            {target ? (
              <>
                <code>{target.path.replace(listing?.home ?? "", "~")}</code> <FolderChips marks={target.marks} />
              </>
            ) : null}
          </div>
          <button type="button" onClick={onClose}>
            Cancel
          </button>
          <button className="primary" type="button" disabled={!target} onClick={use}>
            {selected ? "Use selected folder" : "Use this folder"}
          </button>
        </div>
      </div>
    </div>
  );
}

export function FolderChips({ marks }: { marks: FolderMarks }) {
  return (
    <>
      {marks.pliron ? <span className="chip pliron">pliron project</span> : marks.cargo ? <span className="chip">cargo</span> : null}
      {marks.traces > 0 ? (
        <span className="chip traces">
          {marks.traces} trace{marks.traces === 1 ? "" : "s"}
        </span>
      ) : null}
    </>
  );
}

export function FolderIcon() {
  return (
    <svg className="folder-icon" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M3 6.5A2.5 2.5 0 0 1 5.5 4h3.6c.7 0 1.3.3 1.8.8l1.1 1.2h6.5A2.5 2.5 0 0 1 21 8.5v9a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 17.5v-11z" />
    </svg>
  );
}
