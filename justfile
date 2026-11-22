set positional-arguments

# Run the gate: format check, Clippy with warnings denied, all tests.
check:
    cargo fmt --all --check
    oxfmt --check
    just lint
    just test

# Format everything.
fmt:
    cargo fmt --all
    oxfmt

# Clippy on every crate, warnings denied.
lint:
    cargo clippy --workspace --all-targets --locked -- -D warnings

# All tests with cargo-nextest.
test:
    cargo nextest run --workspace --locked

# Tests of one crate, optionally filtered.
test-crate crate filter="":
    cargo nextest run -p {{crate}} {{filter}}

# Build and run deck with an isolated socket and state dir.
run *args:
    cargo build -p deck
    mkdir -p "${TMPDIR:-/tmp}/deck-dev-{{ shell('printf %.8s "$1"', sha256(justfile_directory())) }}"
    DECK_HOME="${TMPDIR:-/tmp}/deck-dev-{{ shell('printf %.8s "$1"', sha256(justfile_directory())) }}" target/debug/deck "$@"
