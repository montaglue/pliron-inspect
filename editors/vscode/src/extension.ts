import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

/**
 * Resolve the pliron LSP server binary.
 *
 * The whole point of the `pliron.serverPath` setting is dialect bundling:
 * pliron dialects register at link time, so every project builds its own
 * tiny LSP binary linking its dialect crates, and points this extension
 * at it. When unset we fall back to the reference builtin+llvm bundle at
 * `${workspaceFolder}/target/debug/pliron-inspect-lsp-server`, and then
 * to `pliron-inspect-lsp-server` on PATH.
 */
function resolveServerPath(): string | undefined {
  const configured = vscode.workspace
    .getConfiguration("pliron")
    .get<string>("serverPath");
  const folders = vscode.workspace.workspaceFolders ?? [];
  const wsRoot = folders.length > 0 ? folders[0].uri.fsPath : undefined;

  if (configured && configured.trim().length > 0) {
    let p = configured;
    if (wsRoot) {
      p = p.replace(/\$\{workspaceFolder\}/g, wsRoot);
    }
    return p;
  }

  if (wsRoot) {
    for (const profile of ["debug", "release"]) {
      const candidate = path.join(
        wsRoot,
        "target",
        profile,
        "pliron-inspect-lsp-server"
      );
      if (fs.existsSync(candidate)) {
        return candidate;
      }
    }
  }

  // Hope it is on PATH.
  return "pliron-inspect-lsp-server";
}

export function activate(context: vscode.ExtensionContext): void {
  const start = () => {
    const command = resolveServerPath();
    if (!command) {
      void vscode.window.showWarningMessage(
        "pliron: no LSP server binary found. Set `pliron.serverPath` to your project's compiled dialect bundle."
      );
      return;
    }

    const serverOptions: ServerOptions = {
      run: { command },
      debug: { command },
    };

    const clientOptions: LanguageClientOptions = {
      documentSelector: [{ scheme: "file", language: "plir" }],
    };

    client = new LanguageClient(
      "pliron-lsp",
      "pliron IR language server",
      serverOptions,
      clientOptions
    );
    client.start().catch((err) => {
      void vscode.window.showErrorMessage(
        `pliron: failed to start LSP server '${command}': ${err}`
      );
    });
  };

  start();

  context.subscriptions.push(
    vscode.commands.registerCommand("pliron.restartServer", async () => {
      if (client) {
        await client.stop();
        client = undefined;
      }
      start();
    }),
    vscode.workspace.onDidChangeConfiguration(async (e) => {
      if (e.affectsConfiguration("pliron.serverPath")) {
        if (client) {
          await client.stop();
          client = undefined;
        }
        start();
      }
    })
  );
}

export function deactivate(): Thenable<void> | undefined {
  return client?.stop();
}
