// Monaco modules outside its public API, used to hand the diff editor a
// diff computed elsewhere (views/storedDiffs.ts). They ship without types.
declare module "monaco-editor/esm/vs/editor/standalone/browser/standaloneServices.js" {
  export const StandaloneServices: {
    initialize(overrides: Record<string, unknown>): unknown;
    get<T>(id: unknown): T;
  };
}
declare module "monaco-editor/esm/vs/platform/instantiation/common/instantiation.js" {
  export const IInstantiationService: unknown;
}
declare module "monaco-editor/esm/vs/editor/browser/widget/diffEditor/diffProviderFactoryService.js" {
  export const IDiffProviderFactoryService: unknown;
  export const WorkerBasedDiffProviderFactoryService: unknown;
}
declare module "monaco-editor/esm/vs/editor/common/core/lineRange.js" {
  export class LineRange {
    constructor(startLineNumber: number, endLineNumberExclusive: number);
  }
}
declare module "monaco-editor/esm/vs/editor/common/core/range.js" {
  export class Range {
    constructor(startLine: number, startColumn: number, endLine: number, endColumn: number);
  }
}
declare module "monaco-editor/esm/vs/editor/common/diff/rangeMapping.js" {
  import type { LineRange } from "monaco-editor/esm/vs/editor/common/core/lineRange.js";
  import type { Range } from "monaco-editor/esm/vs/editor/common/core/range.js";
  export class RangeMapping {
    constructor(originalRange: Range, modifiedRange: Range);
  }
  export class DetailedLineRangeMapping {
    constructor(original: LineRange, modified: LineRange, innerChanges: RangeMapping[] | undefined);
  }
}
