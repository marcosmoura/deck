---
type: foundation
status: todo
---

# 01 — Workspace and tooling

## Scope

The repository's build and check tooling, and empty skeletons of the four crates, so every later sub-spec starts from a green `just check`. Owns the root files and the crate skeletons.

## Requirements

1. **Workspace.** Root `Cargo.toml` with `[workspace]` (`resolver = "3"`, members `crates/*`), `[workspace.package]` (edition 2024, `rust-version` matching the pinned toolchain, license MIT), and `[workspace.dependencies]` with every crate of [architecture.md](../../architecture.md#dependencies) at the listed version, without features (`default-features = false` only where a crate needs defaults off). `Cargo.lock` is committed.
2. **Lints.** `[workspace.lints]`: `rust.unsafe_code = "deny"` (the platform modules opt in with `#![allow(unsafe_code)]` at module level and nothing else does), `rust.missing_debug_implementations = "warn"`, `clippy.all = "warn"` (Clippy's default set; `just lint` turns warnings into errors). Every crate has `[lints] workspace = true`.
3. **Crates.** `crates/deck-core` (lib), `crates/deck-daemon` (lib), `crates/deck-tui` (lib), `crates/deck` (bin `deck`, `src/main.rs`). Each compiles, has a crate doc comment saying what it owns (from [architecture.md](../../architecture.md#crates)) and one trivial test so nextest has something to run. `deck --version` prints `deck <version>`.
4. **Toolchain.** `.mise.toml` pins `rust` (current stable, with `rustfmt` and `clippy` components), `just`, `cargo-nextest` and `lefthook` to exact versions. `rustfmt.toml` sets `max_width = 100` and `edition = "2024"`. `.config/nextest.toml` sets the default profile to print only failures and the summary (`status-level = "fail"`, `final-status-level = "fail"`, `failure-output = "final"`).
5. **justfile** with exactly the recipes in [AGENTS.md](../../../AGENTS.md#commands):
   - `check`: `cargo fmt --all --check`, then `just lint`, then `just test`.
   - `fmt`: `cargo fmt --all`.
   - `lint`: `cargo clippy --workspace --all-targets --locked -- -D warnings`.
   - `test`: `cargo nextest run --workspace --locked`.
   - `test-crate crate filter=""`: `cargo nextest run -p {{crate}} {{filter}}`.
   - `run *args`: builds, then runs `target/debug/deck {{args}}` with `DECK_HOME` set to `${TMPDIR:-/tmp}/deck-dev-<the first 8 hex characters of just's built-in sha256() of the checkout's absolute path>` (no external hash tool, which differs between macOS and Linux) (created if missing), so the socket path stays under the 100-byte limit even inside `.worktrees/<slug>` and each checkout gets its own daemon.
6. **Lefthook** (`lefthook.yml`): `pre-commit` runs `cargo fmt --all --check` when staged files include `*.rs`; `commit-msg` checks the first line against the Conventional Commits rule of [AGENTS.md](../../../AGENTS.md#commits) with a shell `grep -E` (types and scopes listed, header at most 100 characters) and prints the rule on failure.
7. **Ignore** `target/` and `.worktrees/` (already in `.gitignore`).

## Acceptance criteria

- [ ] `mise install` then `lefthook install` succeed on a clean clone.
- [ ] `just check` passes, and runs format check, Clippy with `-D warnings`, and nextest over four crates.
- [ ] `just run --version` prints `deck <version>`, from the main checkout and from a worktree under `.worktrees/`.
- [ ] The `commit-msg` check, run on a message file (`lefthook run commit-msg <file>` or the script directly), rejects `bad message` and accepts `chore(repo): add workspace`. No commit is made to test it.
- [ ] `lefthook run pre-commit` with an unformatted `.rs` file staged fails (the worker unstages it afterwards; no commit is made).
- [ ] Adding `unsafe {}` to `deck-core` fails `just lint`.

## Tests

- The trivial test in each crate.
- The hook and lint criteria are checked by hand by the worker, without committing, and listed in its report.

## Notes

- Use `cargo search` or crates.io for the exact current patch versions; keep the majors and minors of [architecture.md](../../architecture.md#dependencies).
