import * as assert from "assert";
import * as path from "path";
import * as vscode from "vscode";

function fixture(name: string): vscode.Uri {
  return vscode.Uri.file(
    path.resolve(__dirname, "../../../testFixture/.github/workflows", name)
  );
}

// Resolve once the server has published diagnostics for `uri` — i.e. it has
// analyzed the document. Event-driven, so tests wait on the real signal rather
// than a fixed delay.
function analyzed(uri: vscode.Uri): Promise<void> {
  return new Promise((resolve) => {
    const subscription = vscode.languages.onDidChangeDiagnostics((event) => {
      if (event.uris.some((u) => u.toString() === uri.toString())) {
        subscription.dispose();
        resolve();
      }
    });
  });
}

async function open(uri: vscode.Uri): Promise<vscode.TextDocument> {
  // An already-open document was analyzed on its first open and won't re-publish
  // diagnostics, so only wait for the analysis when opening it afresh.
  const alreadyOpen = vscode.workspace.textDocuments.some(
    (d) => d.uri.toString() === uri.toString()
  );
  const ready = alreadyOpen ? Promise.resolve() : analyzed(uri);
  const doc = await vscode.workspace.openTextDocument(uri);
  await vscode.window.showTextDocument(doc);
  await ready;
  return doc;
}

// The canopy diagnostics of a document, by the rule that found them.
function findings(uri: vscode.Uri): Map<string, vscode.Diagnostic> {
  const ours = vscode.languages
    .getDiagnostics(uri)
    .filter((d) => d.source === "canopy");
  return new Map(ours.map((d) => [String(d.code), d]));
}

suite("canopy language server", () => {
  suiteSetup(async () => {
    await vscode.extensions.getExtension("ferranborreguero.canopy-gh")?.activate();
  });

  test("reports a step that does nothing", async () => {
    const uri = fixture("problems.yml");
    await open(uri);

    const shape = findings(uri).get("step-shape");
    assert.ok(shape, "expected a step-shape diagnostic");
    assert.strictEqual(shape!.severity, vscode.DiagnosticSeverity.Error);
    // The `- name: nothing to do` step, 0-based.
    assert.strictEqual(shape!.range.start.line, 7);
  });

  test("reports two steps sharing an id", async () => {
    const uri = fixture("problems.yml");
    await open(uri);

    const duplicate = findings(uri).get("duplicate-step-ids");
    assert.ok(duplicate, "expected a duplicate-step-ids diagnostic");
    // The second `- id: twice`, not the first.
    assert.strictEqual(duplicate!.range.start.line, 10);
  });

  test("reports a job needing one that is not there", async () => {
    const uri = fixture("problems.yml");
    await open(uri);

    const needs = findings(uri).get("needs-exist");
    assert.ok(needs, "expected a needs-exist diagnostic");
    assert.strictEqual(needs!.range.start.line, 16); // `needs: gone`
    assert.ok(needs!.message.includes("gone"), needs!.message);
  });

  test("reports an expression that does not parse", async () => {
    const uri = fixture("problems.yml");
    await open(uri);

    const syntax = findings(uri).get("expression-syntax");
    assert.ok(syntax, "expected an expression-syntax diagnostic");
    assert.strictEqual(syntax!.range.start.line, 12); // the `if:` line
  });

  test("passes over what a canopy:ignore comment silences", async () => {
    const uri = fixture("silenced.yml");
    await open(uri);

    const rules = findings(uri);
    // The job output is a warning and the comment above it silences that one; the
    // duplicate step id has no comment, so it is still reported.
    assert.ok(!rules.has("job-outputs"), "job-outputs should be silenced");
    const duplicate = rules.get("duplicate-step-ids");
    assert.ok(duplicate, "the finding without a directive should still be reported");
    assert.strictEqual(duplicate!.range.start.line, 14);
    // A warning, not an error: this workflow runs.
    assert.strictEqual(
      duplicate!.severity,
      vscode.DiagnosticSeverity.Warning
    );
  });

  test("a directive cannot silence something that stops the workflow", async () => {
    const uri = fixture("problems.yml");
    await open(uri);

    // `needs-exist` is validation rather than lint: the workflow cannot run, which is not
    // a matter of opinion, so no comment applies to it.
    const needs = findings(uri).get("needs-exist");
    assert.ok(needs, "expected needs-exist to be reported");
    assert.strictEqual(needs!.severity, vscode.DiagnosticSeverity.Error);
  });

  test("says nothing about a workflow that is sound", async () => {
    const uri = fixture("sound.yml");
    await open(uri);

    assert.deepStrictEqual([...findings(uri).keys()], []);
  });

  // These reach GitHub. The server resolves references with `git ls-remote` and the
  // releases API, so a machine without network will fail them rather than skip them.
  suite("references", () => {
    const UNPINNED = 7; // `- uses: actions/checkout@v4`
    const PINNED = 8; // the same action, written as the commit it is at

    function line(number: number): vscode.Range {
      return new vscode.Range(
        new vscode.Position(number, 0),
        new vscode.Position(number, 80)
      );
    }

    async function hints(uri: vscode.Uri): Promise<vscode.InlayHint[]> {
      return vscode.commands.executeCommand<vscode.InlayHint[]>(
        "vscode.executeInlayHintProvider",
        uri,
        line(0).union(line(20))
      );
    }

    test("dates a reference that is not pinned", async () => {
      const uri = fixture("pinning.yml");
      await open(uri);

      const found = (await hints(uri)).find(
        (hint) => hint.position.line === UNPINNED
      );
      assert.ok(found, "expected a hint on the unpinned reference");
      assert.match(String(found!.label), /^published \d{4}-\d{2}-\d{2}$/);
    });

    test("names the version a pinned commit belongs to", async () => {
      const uri = fixture("pinning.yml");
      await open(uri);

      const found = (await hints(uri)).find(
        (hint) => hint.position.line === PINNED
      );
      // The tag standing at that commit, which is what makes a pinned file readable.
      assert.ok(found, "expected a hint on the pinned reference");
      assert.match(String(found!.label), /^v\d/);
    });

    test("offers to pin a reference, and not one already pinned", async () => {
      const uri = fixture("pinning.yml");
      const doc = await open(uri);

      const offered = await vscode.commands.executeCommand<vscode.CodeAction[]>(
        "vscode.executeCodeActionProvider",
        uri,
        line(UNPINNED)
      );
      const pin = offered.find((action) => action.title.startsWith("Pin "));
      assert.ok(pin, "expected a pin action");

      const edits = pin!.edit!.get(uri);
      assert.strictEqual(edits.length, 1);
      // It replaces the reference and nothing around it.
      assert.strictEqual(doc.getText(edits[0].range), "v4");
      assert.match(edits[0].newText, /^[0-9a-f]{40}$/);

      const none = await vscode.commands.executeCommand<vscode.CodeAction[]>(
        "vscode.executeCodeActionProvider",
        uri,
        line(PINNED)
      );
      assert.ok(
        !none.some((action) => action.title.startsWith("Pin ")),
        "a pinned reference has nothing to offer"
      );
    });
  });

  test("outlines the jobs, with their steps under them", async () => {
    const uri = fixture("sound.yml");
    await open(uri);

    const outline = await vscode.commands.executeCommand<vscode.DocumentSymbol[]>(
      "vscode.executeDocumentSymbolProvider",
      uri
    );

    // Written order, not alphabetical: an outline that reorders the file is wrong.
    assert.deepStrictEqual(
      outline.map((job) => job.name),
      ["build", "ship"]
    );
    // Selecting a job puts the cursor on its key.
    assert.strictEqual(outline[0].selectionRange.start.line, 4);
    // Its steps hang off it, named the way the runner announces them.
    assert.deepStrictEqual(
      outline[0].children.map((step) => step.name),
      ['echo "version=1" >> "$GITHUB_OUTPUT"']
    );
  });

  test("offers to run the workflow and each job", async () => {
    const uri = fixture("sound.yml");
    await open(uri);

    const lenses = await vscode.commands.executeCommand<vscode.CodeLens[]>(
      "vscode.executeCodeLensProvider",
      uri
    );

    const titles = lenses.map((lens) => lens.command!.title);
    assert.ok(titles.includes("▶ Run workflow"), titles.join(", "));
    assert.strictEqual(
      titles.filter((title) => title === "▶ Run job").length,
      2
    );

    // A job lens says which job, so the editor knows what to run.
    const build = lenses.find(
      (lens) => lens.command!.arguments?.[1] === "build"
    );
    assert.ok(build, "expected a lens naming the build job");
    assert.strictEqual(build!.range.start.line, 4);
    assert.strictEqual(build!.command!.command, "canopy.run");
  });

  test("re-checks a document as it is edited", async () => {
    const uri = fixture("sound.yml");
    const doc = await open(uri);

    // Breaking a `needs` makes the finding appear...
    const broken = analyzed(uri);
    const edit = new vscode.WorkspaceEdit();
    const needs = new vscode.Range(
      new vscode.Position(13, 0),
      new vscode.Position(13, doc.lineAt(13).text.length)
    );
    assert.strictEqual(doc.getText(needs), "    needs: build");
    edit.replace(uri, needs, "    needs: gone");
    await vscode.workspace.applyEdit(edit);
    await broken;
    assert.ok(findings(uri).has("needs-exist"), "expected the edit to be checked");

    // ...and undoing it makes the finding go away again.
    const fixed = analyzed(uri);
    await vscode.commands.executeCommand("undo");
    await fixed;
    assert.deepStrictEqual([...findings(uri).keys()], []);
  });
});
