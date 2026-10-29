// Monaco's diff editor gets its diffs from a service. This replaces that
// service so a diff editor whose two models have a stored diff (from the
// trace history the server computes once) shows that diff instead of
// computing one: on whole snapshots Monaco takes seconds per diff and gives
// up after five. Models without a stored diff are diffed by Monaco as usual.

import type * as monaco from "monaco-editor/esm/vs/editor/editor.api";
import {
  IDiffProviderFactoryService,
  WorkerBasedDiffProviderFactoryService
} from "monaco-editor/esm/vs/editor/browser/widget/diffEditor/diffProviderFactoryService.js";
import { LineRange } from "monaco-editor/esm/vs/editor/common/core/lineRange.js";
import { Range } from "monaco-editor/esm/vs/editor/common/core/range.js";
import { DetailedLineRangeMapping, RangeMapping } from "monaco-editor/esm/vs/editor/common/diff/rangeMapping.js";
import { StandaloneServices } from "monaco-editor/esm/vs/editor/standalone/browser/standaloneServices.js";
import { IInstantiationService } from "monaco-editor/esm/vs/platform/instantiation/common/instantiation.js";

import type { HistoryStep } from "../api/protocol";

type DocumentDiff = {
  identical: boolean;
  quitEarly: boolean;
  changes: DetailedLineRangeMapping[];
  moves: unknown[];
};

type DiffProvider = {
  onDidChange: unknown;
  computeDiff(original: monaco.editor.ITextModel, modified: monaco.editor.ITextModel, options: unknown, token: unknown): Promise<DocumentDiff>;
  dispose?(): void;
};

type DiffProviderFactory = { createDiffProvider(options: unknown): DiffProvider };

/** Stored diffs by the URI of their modified model, with their original model's URI. */
const stored = new Map<string, { original: string; diff: DocumentDiff }>();

/** Show `step` as the diff of `original` against `modified`. */
export function storeDiff(original: monaco.editor.ITextModel, modified: monaco.editor.ITextModel, step: HistoryStep) {
  stored.set(modified.uri.toString(), { original: original.uri.toString(), diff: toDocumentDiff(step) });
}

export function forgetDiff(modified: monaco.editor.ITextModel) {
  stored.delete(modified.uri.toString());
}

function toDocumentDiff(step: HistoryStep): DocumentDiff {
  return {
    identical: step.identical,
    quitEarly: false,
    moves: [],
    changes: step.hunks.map(
      (hunk) =>
        new DetailedLineRangeMapping(
          new LineRange(hunk.original[0], hunk.original[1]),
          new LineRange(hunk.modified[0], hunk.modified[1]),
          hunk.inner.map(
            ([ol, os, oe, ml, ms, me]) => new RangeMapping(new Range(ol, os, ol, oe), new Range(ml, ms, ml, me))
          )
        )
    )
  };
}

class StoredDiffProvider implements DiffProvider {
  readonly onDidChange: unknown;

  constructor(private readonly fallback: DiffProvider) {
    this.onDidChange = fallback.onDidChange;
  }

  computeDiff(original: monaco.editor.ITextModel, modified: monaco.editor.ITextModel, options: unknown, token: unknown) {
    const entry = stored.get(modified.uri.toString());
    if (entry && entry.original === original.uri.toString()) return Promise.resolve(entry.diff);
    return this.fallback.computeDiff(original, modified, options, token);
  }

  dispose() {
    this.fallback.dispose?.();
  }
}

class StoredDiffProviderFactory implements DiffProviderFactory {
  declare readonly _serviceBrand: undefined;
  private monacoFactory: DiffProviderFactory | null = null;

  createDiffProvider(options: unknown): DiffProvider {
    this.monacoFactory ??= StandaloneServices.get<{
      createInstance(ctor: unknown): DiffProviderFactory;
    }>(IInstantiationService).createInstance(WorkerBasedDiffProviderFactoryService);
    return new StoredDiffProvider(this.monacoFactory.createDiffProvider(options));
  }
}

let installed: boolean | null = null;

/**
 * Put the stored diffs in place of Monaco's diff service. Monaco takes
 * service replacements only when its services start, so this has to run
 * before the first editor or model is made. Returns whether it is in place.
 */
export function installStoredDiffs(): boolean {
  if (installed !== null) return installed;
  const factory = new StoredDiffProviderFactory();
  StandaloneServices.initialize({ diffProviderFactoryService: factory });
  installed = StandaloneServices.get(IDiffProviderFactoryService) === factory;
  if (!installed) {
    console.error("pliron-inspect: Monaco's services started before the stored diffs could be installed");
  }
  return installed;
}
