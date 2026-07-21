import * as monaco from "monaco-editor/esm/vs/editor/editor.api";
import editorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";

declare global {
  interface Window {
    MonacoEnvironment?: monaco.Environment;
  }
}

let monacoConfigured = false;
let stairLanguageRegistered = false;

export function configureStairMonaco() {
  configureWorker();
  registerStairLanguage();
  defineStairTheme();
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

function registerStairLanguage() {
  if (stairLanguageRegistered || monaco.languages.getLanguages().some((language) => language.id === "stair-ir")) {
    stairLanguageRegistered = true;
    return;
  }

  monaco.languages.register({ id: "stair-ir" });
  monaco.languages.setMonarchTokensProvider("stair-ir", {
    defaultToken: "",
    tokenPostfix: ".stair",
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
  stairLanguageRegistered = true;
}

function defineStairTheme() {
  monaco.editor.defineTheme("stair-dark", {
    base: "vs-dark",
    inherit: true,
    rules: [
      { token: "keyword.stair", foreground: "569cd6" },
      { token: "keyword.control.stair", foreground: "c586c0" },
      { token: "keyword.predicate.stair", foreground: "569cd6", fontStyle: "italic" },
      { token: "type.stair", foreground: "4ec9b0" },
      { token: "tag.stair", foreground: "c586c0" },
      { token: "entity.name.function.stair", foreground: "dcdcaa" },
      { token: "variable.stair", foreground: "9cdcfe" },
      { token: "string.stair", foreground: "ce9178" },
      { token: "number.stair", foreground: "b5cea8" },
      { token: "comment.stair", foreground: "6a9955" },
      { token: "operator.stair", foreground: "d4d4d4" },
      { token: "delimiter.bracket.stair", foreground: "ffd700" },
      { token: "delimiter.stair", foreground: "d4d4d4" }
    ],
    colors: {}
  });
}
