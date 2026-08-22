import * as assert from "assert";
import * as fs from "fs";
import * as path from "path";

import { Lines, RunModel, Step } from "../../run";

/// Folds a list of events into a run, the way the stream would.
function fold(events: object[]): RunModel {
  const model = new RunModel("ci.yml");
  for (const event of events) {
    model.apply(event);
  }
  return model;
}

suite("the run tree", () => {
  test("puts steps under their job", () => {
    const model = fold([
      { event: "job-started", id: "build", label: "build" },
      { event: "step-started", index: 0, name: "Checkout", depth: 0 },
      { event: "output", stream: "stdout", line: "cloning" },
      {
        event: "step-finished",
        index: 0,
        name: "Checkout",
        depth: 0,
        conclusion: "success",
        code: 0,
      },
      { event: "job-finished", id: "build", label: "build", conclusion: "success" },
    ]);

    assert.strictEqual(model.run.jobs.length, 1);
    const job = model.run.jobs[0];
    assert.strictEqual(job.status, "success");
    assert.deepStrictEqual(
      job.steps.map((step) => step.name),
      ["Checkout"]
    );
    // A step keeps what it printed, which is what clicking it shows.
    assert.deepStrictEqual(job.steps[0].output, ["cloning"]);
  });

  test("nests a composite action's steps inside the step that used it", () => {
    const model = fold([
      { event: "job-started", id: "build", label: "build" },
      { event: "step-started", index: 0, name: "./greet", depth: 0 },
      { event: "step-started", index: 0, name: "Print it", depth: 1 },
      { event: "output", stream: "stdout", line: "hello" },
      {
        event: "step-finished",
        index: 0,
        name: "Print it",
        depth: 1,
        conclusion: "success",
        code: 0,
      },
      {
        event: "step-finished",
        index: 0,
        name: "./greet",
        depth: 0,
        conclusion: "success",
        code: 0,
      },
      { event: "job-finished", id: "build", label: "build", conclusion: "success" },
    ]);

    const outer = model.run.jobs[0].steps;
    assert.strictEqual(outer.length, 1, "the inner step is not a sibling");
    assert.deepStrictEqual(
      outer[0].children.map((step) => step.name),
      ["Print it"]
    );
    // The line belongs to the step that was open when it arrived.
    assert.deepStrictEqual(outer[0].children[0].output, ["hello"]);
    assert.deepStrictEqual(outer[0].output, []);
  });

  test("keeps a failure and its exit code", () => {
    const model = fold([
      { event: "job-started", id: "build", label: "build" },
      { event: "step-started", index: 0, name: "Run tests", depth: 0 },
      {
        event: "step-finished",
        index: 0,
        name: "Run tests",
        depth: 0,
        conclusion: "failure",
        code: 1,
      },
      { event: "job-finished", id: "build", label: "build", conclusion: "failure" },
    ]);

    const step = model.run.jobs[0].steps[0];
    assert.strictEqual(step.status, "failure");
    assert.strictEqual(step.code, 1);
    assert.strictEqual(model.run.jobs[0].status, "failure");
  });

  test("shows a job that never ran, and why", () => {
    const model = fold([
      { event: "job-passed-over", label: "deploy", reason: "skipped" },
      { event: "job-passed-over", label: "test (2)", reason: "cancelled" },
    ]);

    assert.deepStrictEqual(
      model.run.jobs.map((job) => [job.label, job.status]),
      [
        ["deploy", "skipped"],
        ["test (2)", "cancelled"],
      ]
    );
  });

  test("hangs what the runner says between steps on the job", () => {
    const model = fold([
      { event: "job-started", id: "build", label: "build" },
      { event: "progress", text: "starting catthehacker/ubuntu:act-latest" },
    ]);

    const steps = model.run.jobs[0].steps;
    assert.strictEqual(steps.length, 1);
    assert.match(steps[0].output[0], /starting catthehacker/);
  });

  test("settles anything still running when the process ends", () => {
    const model = fold([
      { event: "job-started", id: "build", label: "build" },
      { event: "step-started", index: 0, name: "Hangs", depth: 0 },
    ]);
    model.finish(true);

    assert.strictEqual(model.run.status, "failure");
    assert.strictEqual(model.run.jobs[0].status, "failure");
  });

  test("ignores an event it does not know", () => {
    const model = fold([
      { event: "job-started", id: "build", label: "build" },
      { event: "something-new-from-a-later-canopy", detail: 1 },
    ]);

    assert.strictEqual(model.run.jobs.length, 1);
  });
});

suite("reading the stream", () => {
  test("waits for a line to be whole before parsing it", () => {
    const lines = new Lines();
    const seen: any[] = [];

    // Split mid-object, the way a pipe would deliver it.
    lines.push('{"event":"job-star', (event) => seen.push(event));
    // Compared by length: asserting against `[]` would narrow `seen` to never[] here.
    assert.strictEqual(seen.length, 0, "half a line is not an event yet");

    lines.push('ted","id":"build","label":"build"}\n', (event) => seen.push(event));
    assert.deepStrictEqual(seen, [
      { event: "job-started", id: "build", label: "build" },
    ]);
  });

  test("passes over anything that is not an event", () => {
    const lines = new Lines();
    const seen: any[] = [];

    lines.push('not json\n{"event":"progress","text":"ok"}\n', (event) =>
      seen.push(event)
    );

    assert.deepStrictEqual(seen, [{ event: "progress", text: "ok" }]);
  });
});

suite("against what canopy actually emits", () => {
  // Captured from `canopy --json` rather than written by hand, so the model is tested
  // against the real wire format. Regenerate with:
  //   canopy --json tests/testdata/composite-and-node-actions.yml -C tests/testdata \
  //     > editors/vscode/testFixture/events.jsonl
  function recorded(): RunModel {
    const file = path.resolve(__dirname, "../../../testFixture/events.jsonl");
    const model = new RunModel("composite-and-node-actions.yml");
    const lines = new Lines();
    lines.push(fs.readFileSync(file, "utf8"), (event) => model.apply(event));
    return model;
  }

  test("understands every event in a real run", () => {
    const model = recorded();

    // The one that matters: if canopy starts emitting something new, this fails rather
    // than the extension quietly dropping it.
    assert.deepStrictEqual(model.ignored, []);
  });

  test("builds the tree a real run describes", () => {
    const model = recorded();

    assert.ok(model.run.jobs.length >= 2, "expected the jobs of the workflow");
    assert.ok(
      model.run.jobs.every((job) => job.status === "success"),
      "the fixture is of a run that passed"
    );

    // A composite action's steps came out nested, which is the whole point of `depth`.
    const nested = model.run.jobs
      .flatMap((job) => job.steps)
      .filter((step) => step.children.length > 0);
    assert.ok(nested.length > 0, "expected a step with steps inside it");

    // Every line landed on some step rather than being dropped.
    const lines = (steps: Step[]): number =>
      steps.reduce(
        (total, step) => total + step.output.length + lines(step.children),
        0
      );
    assert.ok(lines(model.run.jobs.flatMap((job) => job.steps)) > 0);
  });
});
