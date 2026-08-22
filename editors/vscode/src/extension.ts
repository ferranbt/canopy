import * as vscode from "vscode";
import * as fs from "fs";
import * as path from "path";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";
import { OUTPUT_SCHEME, Runs } from "./runs";

let client: LanguageClient | undefined;
let output: vscode.OutputChannel;

export async function activate(context: vscode.ExtensionContext) {
  output = vscode.window.createOutputChannel("Canopy");
  context.subscriptions.push(output);

  const server = resolveServer();
  if (server) {
    start(server, context);
  } else {
    await reportMissing();
  }

  registerRunning(context, server);
}

function registerRunning(
  context: vscode.ExtensionContext,
  server: string | undefined
) {
  const runs = new Runs();
  context.subscriptions.push(
    vscode.window.createTreeView("canopy.runs", { treeDataProvider: runs })
  );

  // A step's output is a read-only document, so several can be open at once.
  context.subscriptions.push(
    vscode.workspace.registerTextDocumentContentProvider(OUTPUT_SCHEME, {
      provideTextDocumentContent: (uri) => runs.linesFor(uri.path.split("/")[1] ?? ""),
    })
  );

  context.subscriptions.push(
    // What the language server's code lenses invoke: the file, and a job if one was named.
    vscode.commands.registerCommand(
      "canopy.run",
      async (target?: string, job?: string) => {
        const file = target
          ? vscode.Uri.parse(target)
          : vscode.window.activeTextEditor?.document.uri;
        if (!file) {
          vscode.window.showErrorMessage("No workflow to run.");
          return;
        }
        if (!server) {
          await reportMissing();
          return;
        }
        await runs.start(server, file, job);
      }
    ),
    vscode.commands.registerCommand("canopy.stop", () => runs.stop()),
    vscode.commands.registerCommand(
      "canopy.showStepOutput",
      async (id: string, name: string) => {
        const uri = vscode.Uri.parse(`${OUTPUT_SCHEME}:/${id}/${name}`);
        const doc = await vscode.workspace.openTextDocument(uri);
        await vscode.window.showTextDocument(doc, { preview: true });
      }
    )
  );
}

export function deactivate(): Thenable<void> | undefined {
  return client?.stop();
}

function resolveServer(): string | undefined {
  if (process.env.SERVER_PATH) {
    return process.env.SERVER_PATH;
  }
  const configured =
    vscode.workspace.getConfiguration("canopy").get<string>("server.path") ||
    "canopy";
  return which(configured);
}

function which(cmd: string): string | undefined {
  if (cmd.includes(path.sep)) {
    return fs.existsSync(cmd) ? cmd : undefined;
  }
  for (const dir of (process.env.PATH || "").split(path.delimiter)) {
    const candidate = path.join(dir, cmd);
    if (fs.existsSync(candidate)) {
      return candidate;
    }
  }
  return undefined;
}

function start(command: string, context: vscode.ExtensionContext) {
  output.appendLine(`Starting language server: ${command} lsp`);
  const serverOptions: ServerOptions = {
    command,
    args: ["lsp"],
    transport: TransportKind.stdio,
  };
  const clientOptions: LanguageClientOptions = {
    // Only workflow files: everything the server knows about is the workflow schema,
    // so it has nothing to say about YAML in general.
    documentSelector: [
      {
        scheme: "file",
        language: "yaml",
        pattern: "**/.github/workflows/*.{yml,yaml}",
      },
    ],
    // Server `window/logMessage`s (e.g. "canopy-lsp ready") land here too.
    outputChannel: output,
  };
  client = new LanguageClient(
    "canopy",
    "GitHub Actions",
    serverOptions,
    clientOptions
  );
  client
    .start()
    .catch((err) =>
      output.appendLine(`Language server failed to start: ${err}`)
    );
  context.subscriptions.push({ dispose: () => void client?.stop() });
}

async function reportMissing() {
  const choice = await vscode.window.showInformationMessage(
    "The canopy language server was not found. Set the path to it?",
    "Open settings"
  );
  if (choice === "Open settings") {
    vscode.commands.executeCommand(
      "workbench.action.openSettings",
      "canopy.server.path"
    );
  }
}
