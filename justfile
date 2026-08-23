fmt-check:
    cargo fmt --all -- --check

fmt:
    cargo fmt --all

clippy:
    cargo clippy --all-targets --all-features --locked

lint: fmt-check clippy

fix-lint:
    cargo fmt --all
    cargo clippy --fix --allow-dirty --allow-staged --all-targets --all-features

release version *args:
    cargo release {{version}} {{args}}

build-runner-image:
    docker build -q -f tests/csharp/runner.Dockerfile -t gh-runner tests/csharp

csharp file="": build-runner-image
    cargo run --bin csharp -- {{ if file == "" { "" } else { "--test " + file } }}

integration file="":
    cargo run --bin integration -- {{ if file == "" { "" } else { "--test " + file } }}
