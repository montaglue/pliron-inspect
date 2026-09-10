import * as monaco from "monaco-editor/esm/vs/editor/editor.api";
import editorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";

declare global {
  interface Window {
    MonacoEnvironment?: monaco.Environment;
  }
}

let monacoConfigured = false;
let crabbitLanguageRegistered = false;

export function configureCrabbitMonaco() {
  configureWorker();
  registerCrabbitLanguage();
  defineCrabbitTheme();
}

function configureWorker() {
  if (monacoConfigured) return;
  window.MonacoEnvironment = {
    getWorker() {
      return new editorWorker();
    }
  };
  monacoConfigured = true;
}

function registerCrabbitLanguage() {
  if (crabbitLanguageRegistered || monaco.languages.getLanguages().some((language) => language.id === "crabbit-ir")) {
    crabbitLanguageRegistered = true;
    return;
  }

  monaco.languages.register({ id: "crabbit-ir" });
  monaco.languages.setMonarchTokensProvider("crabbit-ir", {
    defaultToken: "",
    tokenPostfix: ".crabbit",
    keywords: ["slt", "sle", "sgt", "sge", "eq", "ne", "ugt", "uge", "ult", "ule"],
    tokenizer: {
      root: [
        [/\/\/.*$/, "comment"],
        [/"[^"]*"/, "string"],
        [/\^[\w.$-]+/, "tag"],
        [/@[\w.$-]+/, "entity.name.function"],
        [
          /[a-zA-Z_][\w]*\.[a-zA-Z_][\w]*/,
          {
            cases: {
              "builtin.integeri1": "type",
              "builtin.integeri8": "type",
              "builtin.integeri16": "type",
              "builtin.integeri32": "type",
              "builtin.integeri64": "type",
              "builtin.function": "type",
              "builtin.tuple": "type",
              "@default": "keyword"
            }
          }
        ],
        [/\b(slt|sle|sgt|sge|eq|ne|ugt|uge|ult|ule)\b/, "keyword.predicate"],
        [/-?\d+/, "number"],
        [/[a-zA-Z_][\w]*/, "variable"],
        [/=/, "operator"],
        [/[{}()[\]<>]/, "delimiter.bracket"],
        [/[;,:]/, "delimiter"]
      ]
    }
  });
  crabbitLanguageRegistered = true;
}

function defineCrabbitTheme() {
  monaco.editor.defineTheme("crabbit-dark", {
    base: "vs-dark",
    inherit: true,
    rules: [
      { token: "keyword.crabbit", foreground: "569cd6" },
      { token: "keyword.control.crabbit", foreground: "c586c0" },
      { token: "keyword.predicate.crabbit", foreground: "569cd6", fontStyle: "italic" },
      { token: "type.crabbit", foreground: "4ec9b0" },
      { token: "tag.crabbit", foreground: "c586c0" },
      { token: "entity.name.function.crabbit", foreground: "dcdcaa" },
      { token: "variable.crabbit", foreground: "9cdcfe" },
      { token: "string.crabbit", foreground: "ce9178" },
      { token: "number.crabbit", foreground: "b5cea8" },
      { token: "comment.crabbit", foreground: "6a9955" },
      { token: "operator.crabbit", foreground: "d4d4d4" },
      { token: "delimiter.bracket.crabbit", foreground: "ffd700" },
      { token: "delimiter.crabbit", foreground: "d4d4d4" }
    ],
    colors: {}
  });
}
