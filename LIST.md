Reviewed the corpus (26 cases under testdata/, plus the expect action that lets one file assert on both runners). Here's what I'd add, pessimistic cases marked ✗.

Two structural notes first
Scheduling cases aren't conformance-checked. tests/scheduling/ sits outside the testdata/**/*.yml glob both harnesses use, so those four files aren't run by either binary. That's arguably correct — gh-runner/main.rs feeds the real runner one already-planned job at a time, so matrix expansion, needs, job-level if and fail-fast never reach it (on GitHub those are the service's job, not the runner's). But it means that behaviour is only ever checked against canopy's own recording. Worth deciding whether they move into testdata/ as canopy-only cases with a marker, or stay out and get plain Rust tests.

Plan-time failures can't live in this corpus — prepare() fails the case if plan() errors. Anything about rejecting a bad workflow needs a separate corpus or unit tests.

Highest value: three places I expect canopy already diverges
Case	What it pins
✗ steps/shell-strictness.yml	GitHub runs sh -e {0}; executor.rs:69 runs sh with no flags. A script of false then echo done passes on canopy, fails on GitHub.
✗ steps/pipefail.yml	GitHub's default shell is bash -e {0}, but an explicit shell: bash is bash --noprofile --norc -eo pipefail {0}. So false | true fails only under the explicit form. canopy uses bash -e for both. Also pins that --norc means no .bashrc leakage.
✗ steps/refused-commands.yml	The mirror of unsecure-commands.yml: ::add-path:: without the opt-in. executor.rs:458 already reproduces GitHub's two-line refusal word for word — nothing holds it there today.
Workflow commands and file protocols
This is the densest compatibility surface and the corpus only scratches it.

steps/command-escaping.yml — %25, %0A, %0D decoding in command values and in properties (, and : inside file=/title=).
steps/annotations.yml — ::error file=x,line=3,col=5,title=T::msg and the ::notice:: sibling; pins which parts survive into the log.
steps/stop-commands.yml — ::stop-commands::TOK … ::TOK::, where output in between is not interpreted. Also ::echo::on/off. Worth checking canopy implements these at all.
✗ steps/bad-env-file.yml — an unterminated <<EOF heredoc in $GITHUB_ENV, and a value containing its own delimiter. GitHub fails the step with a specific message; silently accepting it is a divergence and an injection hole.
steps/output-edges.yml — a value containing =, an empty value, the same name written twice (last wins), a name written to both $GITHUB_OUTPUT and ::set-output::.
✗ steps/deprecated-set-output.yml — ::set-output:: and ::save-state:: are disabled on GitHub now; pins the refusal rather than silent acceptance.
Exit codes and failure text
✗ failures/exit-codes.yml — exit 1, exit 3, exit 255, and a step killed by a signal (137). GitHub prints Process completed with exit code N., which the harness compares as a log line — cheap and very sharp.
✗ failures/missing-action.yml — uses: ./actions/nope and uses: owner/repo@nonexistent. Pins the message and whether continue-on-error: true forgives a runner-level failure (I believe GitHub does not).
✗ failures/missing-image.yml — uses: docker://alpine:definitely-not-a-tag, and a job container: that can't be pulled: does the job fail before any step runs?
✗ failures/bad-working-directory.yml — working-directory: nope on a step.
✗ failures/timeout-leaves-state.yml — a step that writes $GITHUB_ENV then sleeps past its timeout: are the writes before the kill still applied? Complements timeouts.yml, which only checks the outcome.
Expressions
Almost untested against GitHub today, and it's canopy's own engine.

contexts/functions.yml — format(), join(), contains(), startsWith/endsWith, toJSON/fromJSON round trip, hashFiles() (stable across both runners for a fixed file).
contexts/coercion.yml — '1' == 1, true == 'true', '' == false, null comparisons, &&/|| returning operands rather than booleans, precedence.
✗ contexts/missing-references.yml — ${{ steps.nope.outputs.x }}, ${{ needs.nope.result }}, ${{ secrets.NOPE }}, a property on a string. GitHub yields empty string rather than erroring; getting this wrong in either direction breaks real workflows.
✗ contexts/injection.yml — a matrix or output value containing "; echo pwned; # interpolated into run:. Pins that substitution is literal and documents the (GitHub-shared) injection surface.
contexts/status-functions.yml — success()/failure()/always()/cancelled() at step level after a failure, including inside a composite.
Actions
✗ actions/failing-composite.yml — a failing step inside a composite: the rest of the composite is abandoned, the outer step fails, and continue-on-error inside a composite is not honoured by GitHub.
actions/nested-composite.yml — composite calling composite: github.action_path, input isolation, output propagation through two levels.
actions/input-naming.yml — who-to-greet → INPUT_WHO-TO-GREET, an input with spaces, an input whose value is multiline.
✗ actions/input-validation.yml — a required: true input omitted, and an undeclared input passed via with:. GitHub warns and carries on for both; validate.rs already produces findings, so this pins that they match.
actions/pre-post-hooks.yml — a remote node action with pre/post and post-if, ordering relative to other steps, and $GITHUB_STATE round trip. Today only docker-state.yml and post-steps.yml touch this.
✗ actions/container-file-ownership.yml — a container action writes into the workspace as root, then a later run: step tries to modify it. This bites people constantly on real GitHub and is exactly where canopy's identical-path mounting could differ.
Job containers and services
jobs/container-volumes.yml — the volumes: list on a job container, plus --user via options:.
✗ jobs/service-fails-to-start.yml — a service with a bad image or an entrypoint that exits immediately: the job's fate, and whether steps run at all.
jobs/service-env-and-options.yml — env:/options: on a service rather than the job container (untested today).
✗ jobs/no-network-for-unlisted-service.yml — extends no-services.yml: a job with service a cannot resolve b.
jobs/path-inside-container.yml — $GITHUB_PATH additions inside a container job, and whether they reach a subsequent action rather than just a run: step.
Artifacts and caches
✗ services/artifact-missing.yml — if-no-files-found: error, and downloading a name that was never uploaded.
services/cache-miss.yml — cache-hit == 'false' on a miss, restore-keys prefix fallback, and a key built with hashFiles().
services/artifact-patterns.yml — hidden files, nested directories, merge-multiple, downloading by pattern.
Output plumbing
steps/output-shapes.yml — a last line with no trailing newline, \r\n, invalid UTF-8 bytes, a line over 64 KB, ANSI escapes (kept or stripped?). All things drain/pump handle by construction and nothing pins.
✗ security/masking-edges.yml — extends secret-masking.yml: masking a multiline value, a value that also appears inside a workflow command argument, and a mask registered by an action rather than a run: step.
If you want, tell me which slice to start with and I'll write them — I'd take the three divergence candidates first, since those either find real bugs or retire the suspicion.

