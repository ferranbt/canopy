# gh-runner-local

A self-hosted runner that waits for jobs and runs their steps on this machine, without containers.

Register first, then listen:

```sh
cargo run -p gh-runner-registration -- --url https://github.com/OWNER/REPO --token AAAA...
cargo run -p gh-runner-local -- --credentials credentials.json --workspace _work
```

Then give the repository a workflow that asks for this runner, and run it:

```yaml
on: workflow_dispatch
jobs:
  test:
    runs-on: self-hosted
    steps:
      - uses: actions/checkout@v4
      - run: echo "hello from $(hostname)"
```

The job appears in the runner's output as it happens, and on GitHub as it would for any
other runner: live logs, per-step results, and a conclusion.
