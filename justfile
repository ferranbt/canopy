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

bench filter="":
    cargo bench -p canopy-bench {{ if filter == "" { "" } else { "-- " + filter } }}

release version *args:
    cargo release {{version}} {{args}}

build-runner-image:
    docker build -q -f tests/gh-runner/runner.Dockerfile -t gh-runner tests/gh-runner

gh-runner file="": build-runner-image
    cargo run --bin integration -- --runner official-gh {{ if file == "" { "" } else { "--test " + file } }}

integration file="" runner="" validate="":
    cargo run --bin integration -- {{ if file == "" { "" } else { "--test " + file } }} {{ if runner == "" { "" } else { "--runner " + runner } }} {{ if validate == "" { "" } else { "--validate" } }}

count-tests:
    #!/usr/bin/env sh
    set -e
    count=$(find tests/testdata -name '*.yml' | wc -l | tr -d ' ')
    printf '{"schemaVersion":1,"label":"conformance","message":"%s workflows","color":"2ea44f"}\n' "$count" > tests/num_tests.json
    echo "$count conformance tests"
