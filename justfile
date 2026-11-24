set positional-arguments

# Run the gate: format check, Clippy with warnings denied, all tests.
check:
    cargo fmt --all --check
    git ls-files -z --cached --others --exclude-standard | xargs -0 oxfmt --check
    just lint
    just test

# Format everything.
fmt:
    cargo fmt --all
    git ls-files -z --cached --others --exclude-standard | xargs -0 oxfmt

# Clippy on every crate, warnings denied.
lint:
    cargo clippy --workspace --all-targets --locked -- -D warnings

# All tests with cargo-nextest.
test:
    cargo nextest run --workspace --locked

# Tests of one crate, optionally filtered.
test-crate crate filter="":
    cargo nextest run -p {{crate}} {{filter}}

# The stress and soak suite, in a release build, one scenario at a time. `DECK_SOAK_SECS`
# sets the soak's length (default 600).
stress filter="":
    ulimit -n 4096 && cargo nextest run --release -p deck --test stress --run-ignored only --test-threads 1 --no-capture {{filter}}

# Build and run deck with an isolated socket and state dir.
run *args:
    cargo build -p deck
    mkdir -p "${TMPDIR:-/tmp}/deck-dev-{{ shell('printf %.8s "$1"', sha256(justfile_directory())) }}"
    env -u DECK -u DECK_SOCKET -u DECK_SESSION_ID -u DECK_PANEL_ID -u DECK_TERMINAL_ID \
        DECK_HOME="${TMPDIR:-/tmp}/deck-dev-{{ shell('printf %.8s "$1"', sha256(justfile_directory())) }}" target/debug/deck "$@"

# Build and install deck into $PREFIX/bin (default ~/.local/bin).
install:
    cargo install --path crates/deck --locked --root "${PREFIX:-$HOME/.local}"
