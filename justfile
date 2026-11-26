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

# Lint the GitHub workflows with actionlint and the shell scripts with shellcheck, and check
# that the workflows run only pinned actions and a pinned mise.
lint-ci:
    actionlint
    shellcheck install.sh packaging/install-check.sh packaging/homebrew/render.sh packaging/homebrew/check.sh .github/check-pins.sh
    sh packaging/install-check.sh
    sh packaging/homebrew/check.sh
    sh .github/check-pins.sh

# Check the dependencies with cargo-deny (`deny.toml`): advisories, licences, duplicate
# versions and sources. It downloads the advisory database.
audit:
    cargo deny --locked check

# Rewrite `mise.lock`, the URL and checksum of every tool of `.mise.toml`, for the platforms
# the lock already has. Run it after a change of `.mise.toml`; it downloads from the tools'
# releases.
lock-tools:
    mise lock

# All tests with cargo-nextest.
test:
    cargo nextest run --workspace --locked

# Tests of one crate, optionally filtered.
test-crate crate filter="":
    cargo nextest run -p {{crate}} {{filter}}

# The stress and soak suite, in a release build, one scenario at a time. `DECK_SOAK_SECS`
# sets the soak's length (default 600). The 10-minute `idle` runs only when the filter is
# exactly `idle`: the recipe passes it on as `DECK_STRESS_FILTER`.
stress filter="":
    ulimit -n 4096 && DECK_STRESS_FILTER='{{filter}}' cargo nextest run --release -p deck --test stress --run-ignored only --test-threads 1 --no-capture {{filter}}

# Record a program's output from `crates/deck/tests/compat/scenarios/<name>.scenario`; FORCE=1
# replaces an existing recording.
record-compat name:
    DECK_RECORD={{name}} cargo nextest run -p deck --test compat_record --run-ignored only --no-capture

# Build and run deck with an isolated socket and state dir.
run *args:
    cargo build -p deck
    mkdir -p "${TMPDIR:-/tmp}/deck-dev-{{ shell('printf %.8s "$1"', sha256(justfile_directory())) }}"
    env -u DECK -u DECK_SOCKET -u DECK_SESSION_ID -u DECK_PANEL_ID -u DECK_TERMINAL_ID \
        DECK_HOME="${TMPDIR:-/tmp}/deck-dev-{{ shell('printf %.8s "$1"', sha256(justfile_directory())) }}" target/debug/deck "$@"

# Build and install deck into $PREFIX/bin (default ~/.local/bin).
install:
    cargo install --path crates/deck --locked --root "${PREFIX:-$HOME/.local}"
