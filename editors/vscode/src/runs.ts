// The Runs view: what canopy is doing, as a tree you can click into.

import * as path from "path";
import * as vscode from "vscode";
import { spawn, ChildProcess } from "child_process";

import { Lines, Node, RunModel, Status } from "./run";

/// Where a step's output is read: a read-only document rather than a shared output channel,
/// so several steps can be open at once and none of them scrolls away.
export const OUTPUT_SCHEME = "canopy-step";

/// The Runs view, one root per run, newest first.
export class Runs implements vscode.TreeDataProvider<Node> {
  private readonly changed = new vscode.EventEmitter<Node | undefined>();
  readonly onDidChangeTreeData = this.changed.event;

  private models: RunModel[] = [];
  private running?: ChildProcess;
  /// The output of every step, by the id the tree gives it.
  private readonly output = new Map<string, string[]>();

  getChildren(node?: Node): Node[] {
    if (!node) {
      return this.models.map((model) => model.run);
    }
    switch (node.kind) {
      case "run":
        return node.jobs;
      case "job":
        return node.steps;
      case "step":
        return node.children;
    }
  }

  getTreeItem(node: Node): vscode.TreeItem {
    const collapsible =
      this.getChildren(node).length > 0
        ? vscode.TreeItemCollapsibleState.Expanded
        : vscode.TreeItemCollapsibleState.None;

    const label = node.kind === "run" ? node.label : node.kind === "job" ? node.label : node.name;
    const item = new vscode.TreeItem(label, collapsible);
    item.iconPath = icon(node.status);
    item.contextValue = node.kind;

    if (node.kind === "step") {
      item.description = node.code ? `exit ${node.code}` : undefined;
      // Clicking a step opens what it printed, which is the thing a terminal cannot do.
      const id = String(this.output.size);
      this.output.set(id, node.output);
      item.command = {
        title: "Show output",
        command: "canopy.showStepOutput",
        arguments: [id, node.name],
      };
    }
    return item;
  }

  /// The lines a step printed, for the document provider to render.
  linesFor(id: string): string {
    return (this.output.get(id) ?? []).join("\n");
  }

  /// Runs a workflow, or one job of it, and builds a tree out of what it reports.
  async start(command: string, file: vscode.Uri, job?: string): Promise<void> {
    if (this.running) {
      const stop = "Stop it";
      const choice = await vscode.window.showWarningMessage(
        "A run is already going. Stop it and start this one?",
        stop
      );
      if (choice !== stop) {
        return;
      }
      this.stop();
    }

    const name = path.basename(file.fsPath);
    const model = new RunModel(job ? `${name} · ${job}` : name);
    this.models.unshift(model);
    this.changed.fire(undefined);

    const args = ["--json"];
    if (job) {
      args.push("--job", job);
    }
    args.push(file.fsPath);

    const cwd = vscode.workspace.getWorkspaceFolder(file)?.uri.fsPath;
    const child = spawn(command, args, { cwd });
    this.running = child;

    const lines = new Lines();
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk: string) => {
      lines.push(chunk, (event) => model.apply(event));
      this.changed.fire(undefined);
    });
    // canopy keeps stdout for events, so anything it says around the run arrives here.
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", () => { });

    child.on("error", (err) => {
      vscode.window.showErrorMessage(`Could not run ${command}: ${err.message}`);
    });
    child.on("close", (code) => {
      model.finish(code !== 0);
      this.running = undefined;
      this.changed.fire(undefined);
    });
  }

  /// Stops the run that is going, if one is.
  stop(): void {
    this.running?.kill();
    this.running = undefined;
  }
}

function icon(status: Status): vscode.ThemeIcon {
  switch (status) {
    case "running":
      return new vscode.ThemeIcon("sync~spin");
    case "success":
      return new vscode.ThemeIcon("pass", new vscode.ThemeColor("testing.iconPassed"));
    case "failure":
      return new vscode.ThemeIcon("error", new vscode.ThemeColor("testing.iconFailed"));
    case "cancelled":
      return new vscode.ThemeIcon("circle-slash");
    case "skipped":
      return new vscode.ThemeIcon("debug-step-over");
  }
}
