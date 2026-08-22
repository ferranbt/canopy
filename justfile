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

# Runs every workflow under tests/testdata, or only the one named.
integration file="":
    TARGET_FILE={{file}} cargo test --test integration -- --nocapture
