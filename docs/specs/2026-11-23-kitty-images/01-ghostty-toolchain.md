---
type: foundation
status: todo
---

# 01 — libghostty-vt and Zig in the build

## Scope

`libghostty-vt` builds and links in `deck-daemon` on macOS and Linux, through the pinned toolchain, with a smoke test proving it runs. The emulator is still alacritty; sub-spec 03 ports it. It comes first because whether Ghostty builds with Zig on every target is the riskiest unknown of the spec, and landing it first lets the user check CI and the four release targets before any other work. Owning crates: the repository root (`Cargo.toml`, `.mise.toml`, `AGENTS.md`, architecture.md) and `deck-daemon`.

## Requirements

1. **Dependency.** `libghostty-vt = "0.2"` in `[workspace.dependencies]`, used by `deck-daemon` with `default-features = false` and the `kitty-graphics` feature (not `png`; sub-spec 04 brings its own decoder). Recorded in [architecture.md#dependencies](../../architecture.md#dependencies). `alacritty_terminal` stays in `deck-daemon` until sub-spec 03.
2. **Zig 0.15.2** in `.mise.toml` (the `minimum_zig_version` of the Ghostty commit pinned by `libghostty-vt-sys` 0.2.2; if a newer 0.2.x is used, its pinned commit's version). The tools paragraph of architecture.md and the toolchain line of `AGENTS.md` name it.
3. **Build mode.** `libghostty-vt-sys` builds Ghostty in Zig's `Debug` mode for Cargo's dev profile, which may turn on slow integrity checks; check that first. `LIBGHOSTTY_VT_SYS_OPTIMIZE` overrides the mode for every profile, and changing it rebuilds Ghostty. The rule: `[env]` in `.cargo/config.toml` sets `LIBGHOSTTY_VT_SYS_OPTIMIZE = "ReleaseSafe"` (without `force`), so plain `cargo` and every `just` recipe build the same way; `just install` and `release.yml` export `LIBGHOSTTY_VT_SYS_OPTIMIZE=ReleaseFast` themselves, which `[env]` does not override. If the check shows `Debug` is fast enough, drop the `[env]` line and say so. architecture.md records the choice and its reason.
4. **First build** (architecture.md): network access is needed to clone Ghostty into Cargo's build directory (again per profile, per worktree and after `cargo clean`) and for `zig build` to fetch Ghostty's Zig packages; offline builds set `GHOSTTY_SOURCE_DIR` (a Ghostty checkout at the pinned commit) and `GHOSTTY_ZIG_SYSTEM_DIR` (pre-fetched Zig packages). If Zig's global cache needs a writable place (sandboxes), the note says how to set it.
5. **Smoke test** in `deck-daemon`: create a `libghostty-vt` `Terminal`, feed it `"hi\x1b[31mX"`, and read back the text and the colour of `X` through its API.
6. **CI and release.**
   - `ci.yml` gets Zig through `mise-action` and `.mise.toml`.
   - `release.yml` installs only Rust today (`install_args: rust`); it becomes `install_args: rust zig`.
   - `release.yml` gets a `pull_request` trigger limited to changes of `Cargo.toml`, `Cargo.lock`, `.mise.toml`, `.cargo/config.toml` and `release.yml` itself: on it, the four-target build matrix (macos-15, macos-15-intel, ubuntu-22.04, ubuntu-22.04-arm) runs, and the tag check, packaging and `publish` jobs run only for `refs/tags/v*`. So every change to the build is checked on all four targets before a release.
   - The worker cannot push. With the user's go-ahead, the lead pushes the branch and opens a draft pull request, and the user checks that CI and the release build pass on it before sub-spec 02 starts.

## Acceptance criteria

- [ ] `mise install` then `just check` passes from a clean clone (network on), including the smoke test.
- [ ] `cargo tree -p deck-daemon` shows `libghostty-vt`.
- [ ] The dev and test build mode is set and documented; `just test` takes no longer than before by more than the one-time Ghostty build.
- [ ] architecture.md (dependency, Zig, first build, build mode) and `AGENTS.md` are updated.
- [ ] `release.yml` installs Zig, builds on pull requests that touch the build, and publishes only on `v*` tags.
- [ ] CI and the four-target release build pass on the draft pull request (checked by the user).

## Tests

- The smoke test of requirement 5; every existing test, unchanged.
