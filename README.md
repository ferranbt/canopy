# Canopy

A modular implementation of the GitHub Actions protocol, in Rust.

It comes in two halves. The **SDK** is a set of crates covering the protocol itself: the
workflow format, the expression language, the contexts, the planner, the runner, and the
wire protocol a self-hosted runner speaks to GitHub. The  `canopy` **binary** is one composition of those crates, which runs a
workflow on your machine.

## canopy

```sh
canopy run .github/workflows/ci.yml          # run it here
canopy run ci.yml --job build -n             # what would run, and in what order
canopy lint ci.yml                           # what is wrong with it
canopy lsp                                   # language server, over stdio
```

## The SDK

| crate | what it is |
| --- | --- |
| [`gh-actions-spec`](crates/gh-actions-spec) | The workflow and `action.yml` file formats, as types |
| [`gh-actions-expr`](crates/gh-actions-expr) | The `${{ }}` language: lexer, parser, evaluator, and what an expression reads |
| [`gh-actions-context`](crates/gh-actions-context) | The contexts a run exposes |
| [`gh-actions-plan`](crates/gh-actions-plan) | Converts workflows to valid jobs |
| [`gh-actions-lint`](crates/gh-actions-lint) | Linting checks for the workflows |
| [`gh-actions-runner`](crates/gh-actions-runner) | Runs a planned job |
| [`gh-actions-services`](crates/gh-actions-services) | The artifact and cache services a job talks to |
| [`gh-actions-listener`](crates/gh-actions-listener) | The protocol a self-hosted runner speaks: register, poll, acquire, report |

## Examples

[`examples`](examples) holds other ways the same crates go together:

| example | what it is |
| --- | --- |
| [`gh-runner-local`](examples/gh-runner-local) | Takes jobs from GitHub and runs them on this machine |
| [`gh-runner-fargate-ondemand`](examples/gh-runner-fargate-ondemand) | Takes jobs from GitHub and gives each one an AWS Fargate task |
