// The tree of a run: what canopy reported, folded into something clickable.
//
// The model is a plain reducer over the event stream, with no VS Code in it, so it can be
// tested without spawning anything. Only the view and the spawning below know about the
// editor.

export type Status = "running" | "success" | "failure" | "skipped" | "cancelled";

/// One step, and whatever it printed.
export interface Step {
  readonly kind: "step";
  name: string;
  status: Status;
  code?: number;
  /// Everything the step printed, kept so it can be read after it has finished.
  output: string[];
  /// A composite action's steps sit under the step that used it.
  children: Step[];
}

export interface Job {
  readonly kind: "job";
  id?: string;
  label: string;
  status: Status;
  steps: Step[];
}

export interface Run {
  readonly kind: "run";
  label: string;
  status: Status;
  jobs: Job[];
}

export type Node = Run | Job | Step;

/// A run being assembled from the events as they arrive.
export class RunModel {
  readonly run: Run;
  /// Event kinds this model did not know what to do with. A newer canopy talking to an
  /// older extension lands here rather than breaking, and a test can check it stays empty.
  readonly ignored: string[] = [];
  /// The steps currently open, outermost first — what `depth` indexes into.
  private open: Step[] = [];

  constructor(label: string) {
    this.run = { kind: "run", label, status: "running", jobs: [] };
  }

  private get job(): Job | undefined {
    return this.run.jobs[this.run.jobs.length - 1];
  }

  /// Where a step at this depth belongs.
  private siblings(depth: number): Step[] {
    if (depth === 0) {
      return this.job?.steps ?? [];
    }
    return this.open[depth - 1].children;
  }

  /// Folds one event in. Anything unrecognised is ignored, so a newer canopy talking to an
  /// older extension degrades rather than breaks.
  apply(event: any): void {
    switch (event.event) {
      case "job-started":
        this.run.jobs.push({
          kind: "job",
          id: event.id,
          label: event.label,
          status: "running",
          steps: [],
        });
        this.open = [];
        break;

      case "job-passed-over":
        this.run.jobs.push({
          kind: "job",
          label: event.label,
          status: event.reason === "cancelled" ? "cancelled" : "skipped",
          steps: [],
        });
        break;

      case "job-finished":
        if (this.job) {
          this.job.status = event.conclusion as Status;
        }
        this.open = [];
        break;

      case "step-started": {
        const step: Step = {
          kind: "step",
          name: event.name,
          status: "running",
          output: [],
          children: [],
        };
        this.siblings(event.depth).push(step);
        this.open[event.depth] = step;
        this.open.length = event.depth + 1;
        break;
      }

      case "step-finished": {
        const step = this.open[event.depth];
        if (step) {
          step.status = event.conclusion as Status;
          step.code = event.code ?? undefined;
        }
        this.open.length = event.depth;
        break;
      }

      case "output":
        this.say(event.line);
        break;

      case "progress":
        this.say(event.text);
        break;

      case "message":
        this.say(`${event.level}: ${event.text}`);
        break;

      default:
        this.ignored.push(String(event.event));
    }
  }

  /// Puts a line under the innermost step that is running, or under the job when the runner
  /// is between steps — starting a container, say.
  private say(line: string): void {
    const step = this.open[this.open.length - 1];
    if (step) {
      step.output.push(line);
      return;
    }
    const job = this.job;
    if (job) {
      // Nothing to hang it on yet, so it becomes a step of its own.
      const note = job.steps.find((candidate) => candidate.name === "(setting up)");
      if (note) {
        note.output.push(line);
      } else {
        job.steps.push({
          kind: "step",
          name: "(setting up)",
          status: "success",
          output: [line],
          children: [],
        });
      }
    }
  }

  /// Settles the run once the process is over.
  finish(failed: boolean): void {
    this.run.status = failed ? "failure" : "success";
    for (const job of this.run.jobs) {
      if (job.status === "running") {
        job.status = failed ? "failure" : "success";
      }
    }
  }
}

/// Reads a stream of newline-delimited JSON, handing over whole objects.
export class Lines {
  private rest = "";

  push(chunk: string, each: (event: any) => void): void {
    this.rest += chunk;
    const lines = this.rest.split("\n");
    // The last piece may be half a line, so it waits for the next chunk.
    this.rest = lines.pop() ?? "";

    for (const line of lines) {
      if (!line.trim().startsWith("{")) {
        continue;
      }
      try {
        each(JSON.parse(line));
      } catch {
        // A line that will not parse is not worth stopping the run over.
      }
    }
  }
}
