import * as monaco from "monaco-editor/esm/vs/editor/editor.api";
import editorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";
// The editor's features (hover, go to definition, references, find, ...);
// the bare API has none of them.
import "monaco-editor/esm/vs/editor/edcore.main";
import "monaco-editor/esm/vs/basic-languages/rust/rust.contribution";

import { irDefinition, irHover, irReferences } from "../api/client";
import type { LspRange } from "../api/protocol";
import { installStoredDiffs } from "./storedDiffs";

declare global {
  interface Window {
    MonacoEnvironment?: monaco.Environment;
  }
}

export const IR_LANGUAGE = "pliron-ir";
export const THEME = "pliron-dark";

let configured = false;

export function configurePlironMonaco() {
  if (configured) return;
  configured = true;
  // First: Monaco takes it only while its services have not started.
  installStoredDiffs();
  window.MonacoEnvironment = {
    getWorker() {
      return new editorWorker();
    }
  };
  registerIrLanguage();
  defineTheme();
  registerLanguageFeatures();
}

// ---- IR documents known to the server ---------------------------------------

/** The server's IR document id of each text model (by model URI). */
const irIds = new Map<string, string>();

export function bindIrId(model: monaco.editor.ITextModel, id: string | null) {
  if (id) irIds.set(model.uri.toString(), id);
  else irIds.delete(model.uri.toString());
}

function irIdOf(model: monaco.editor.ITextModel) {
  return irIds.get(model.uri.toString());
}

/** Shows a definition in the dialects' Rust source (set by the text view). */
let sourceOpener: ((model: monaco.editor.ITextModel, range: monaco.IRange) => void) | null = null;

export function setSourceOpener(opener: typeof sourceOpener) {
  sourceOpener = opener;
}

export function toRange(range: LspRange): monaco.Range {
  return new monaco.Range(
    range.start.line + 1,
    range.start.character + 1,
    range.end.line + 1,
    range.end.character + 1
  );
}

function toLsp(position: monaco.Position) {
  return { line: position.lineNumber - 1, character: position.column - 1 };
}

/** A read-only model of a Rust source file, made once. */
function sourceModel(path: string, text: string) {
  const uri = monaco.Uri.file(path);
  return monaco.editor.getModel(uri) ?? monaco.editor.createModel(text, "rust", uri);
}

function registerLanguageFeatures() {
  monaco.languages.registerHoverProvider(IR_LANGUAGE, {
    async provideHover(model, position) {
      const id = irIdOf(model);
      if (!id) return null;
      const hover = await irHover(id, toLsp(position)).catch(() => null);
      if (!hover?.contents) return null;
      return {
        contents: [{ value: hover.contents }],
        range: hover.range ? toRange(hover.range) : undefined
      };
    }
  });

  monaco.languages.registerDefinitionProvider(IR_LANGUAGE, {
    async provideDefinition(model, position) {
      const id = irIdOf(model);
      if (!id) return null;
      const definition = await irDefinition(id, toLsp(position)).catch(() => null);
      if (definition?.range) {
        return { uri: model.uri, range: toRange(definition.range) };
      }
      const source = definition?.source;
      if (source?.text) {
        const range = source.range
          ? toRange(source.range)
          : new monaco.Range(source.position.line + 1, 1, source.position.line + 1, 1);
        return { uri: sourceModel(source.path, source.text).uri, range };
      }
      return null;
    }
  });

  monaco.languages.registerReferenceProvider(IR_LANGUAGE, {
    async provideReferences(model, position) {
      const id = irIdOf(model);
      if (!id) return null;
      const references = await irReferences(id, toLsp(position)).catch(() => null);
      return (references?.ranges ?? []).map((range) => ({ uri: model.uri, range: toRange(range) }));
    }
  });

  monaco.languages.registerDocumentHighlightProvider(IR_LANGUAGE, {
    async provideDocumentHighlights(model, position) {
      const id = irIdOf(model);
      if (!id) return null;
      const references = await irReferences(id, toLsp(position)).catch(() => null);
      return (references?.ranges ?? []).map((range) => ({ range: toRange(range) }));
    }
  });

  // Go to definition into a Rust source opens it beside the IR.
  monaco.editor.registerEditorOpener({
    openCodeEditor(_source, resource, selectionOrPosition) {
      const model = monaco.editor.getModel(resource);
      if (resource.scheme !== "file" || !model || !sourceOpener) return false;
      const range =
        selectionOrPosition && "startLineNumber" in selectionOrPosition
          ? selectionOrPosition
          : selectionOrPosition
            ? monaco.Range.fromPositions(selectionOrPosition)
            : new monaco.Range(1, 1, 1, 1);
      sourceOpener(model, range);
      return true;
    }
  });
}

// ---- syntax and theme --------------------------------------------------------

function registerIrLanguage() {
  if (monaco.languages.getLanguages().some((language) => language.id === IR_LANGUAGE)) return;

  // `pliron` also names it in the hover's code blocks.
  monaco.languages.register({ id: IR_LANGUAGE, aliases: ["pliron", "plir"] });
  monaco.languages.setMonarchTokensProvider(IR_LANGUAGE, {
    defaultToken: "",
    tokenPostfix: ".pliron",
    keywords: ["slt", "sle", "sgt", "sge", "eq", "ne", "ugt", "uge", "ult", "ule"],
    tokenizer: {
      root: [
        [/\/\/.*$/, "comment"],
        [/"[^"]*"/, "string"],
        [/\^[\w.$-]+/, "tag"],
        [/@[\w.$-]+/, "entity.name.function"],
        [/builtin\.(integer|function|tuple|unit|fp\w*)\b/, "type"],
        [/[a-zA-Z_][\w]*\.[a-zA-Z_][\w]*/, "keyword"],
        [/\b(slt|sle|sgt|sge|eq|ne|ugt|uge|ult|ule)\b/, "keyword.predicate"],
        [/-?\d+/, "number"],
        [/[a-zA-Z_][\w]*/, "variable"],
        [/=/, "operator"],
        [/[{}()[\]<>]/, "delimiter.bracket"],
        [/[;,:]/, "delimiter"]
      ]
    }
  });
}

function defineTheme() {
  monaco.editor.defineTheme(THEME, {
    base: "vs-dark",
    inherit: true,
    rules: [
      { token: "keyword.pliron", foreground: "569cd6" },
      { token: "keyword.predicate.pliron", foreground: "569cd6", fontStyle: "italic" },
      { token: "type.pliron", foreground: "4ec9b0" },
      { token: "tag.pliron", foreground: "c586c0" },
      { token: "entity.name.function.pliron", foreground: "dcdcaa" },
      { token: "variable.pliron", foreground: "9cdcfe" },
      { token: "string.pliron", foreground: "ce9178" },
      { token: "number.pliron", foreground: "b5cea8" },
      { token: "comment.pliron", foreground: "6a9955" },
      { token: "operator.pliron", foreground: "d4d4d4" },
      { token: "delimiter.bracket.pliron", foreground: "ffd700" },
      { token: "delimiter.pliron", foreground: "d4d4d4" }
    ],
    colors: {}
  });
}
