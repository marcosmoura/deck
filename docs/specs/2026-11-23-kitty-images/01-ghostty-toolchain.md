---
type: foundation
status: done
---

# 01 — ghostty-vt and Zig in the build

## Scope

`ghostty-vt` (Ghostty's `libghostty-vt` 1.0.1) builds and links in `deck-daemon` on macOS and Linux, through the pinned toolchain, with a smoke test proving it runs. The emulator is still alacritty; sub-spec 03 ports it. It comes first because whether the library builds on every target is the riskiest unknown of the spec, and landing it first lets the user check CI and the four release targets before any other work. Owning crates: the repository root (`Cargo.toml`, `.mise.toml`, `AGENTS.md`, architecture.md, the workflows) and `deck-daemon`.

## Requirements

1. **Dependency.** `ghostty-vt = "=0.9.0"` in `[workspace.dependencies]`, used by `deck-daemon` with `default-features = false` and the `kitty-graphics` and `png` features. Recorded in [architecture.md#dependencies](../../architecture.md#dependencies) with `ghostty-vt-sys` and `png` (which come with it). `alacritty_terminal` stays in `deck-daemon` until sub-spec 03.
2. **Zig 0.17.1** in `.mise.toml` (the release's `minimum_zig_version` is 0.17.0). macOS builds do not use it (they link the prebuilt release); Linux builds do. The tools paragraph of architecture.md and the toolchain line of `AGENTS.md` name it.
3. **No build-mode setting.** `ghostty-vt-sys` has none: macOS links the release's prebuilt library, and Linux builds the source `ReleaseFast` for a baseline CPU (hard-coded by the crate). Nothing goes into `.cargo/config.toml`, the `justfile` or the workflows for it.
4. **First build** (architecture.md): network access is needed to download the pinned release files from `release.files.ghostty.org` (on macOS the XCFramework; on Linux the source and dependency tarballs), verified against the sha256s in `ghostty-vt-sys`'s `GHOSTTY.lock`, once per Cargo build directory. Offline builds set `GHOSTTY_RELEASE_DIR` (a directory holding those release files); `GHOSTTY_VT_LIB_DIR` links an already built library.
5. **Smoke test** in `deck-daemon`: create a `ghostty-vt` `Terminal`, feed it `"hi\x1b[31mX"`, and read back the text and the colour of `X` through its API.
6. **CI and release.**
   - `ci.yml` gets Zig through `mise-action` and `.mise.toml`.
   - `release.yml` installs only Rust today (`install_args: rust`); it becomes `install_args: rust zig`.
   - `release.yml` gets a `pull_request` trigger limited to changes of any `Cargo.toml` (`**/Cargo.toml`), `Cargo.lock`, `.mise.toml` and `release.yml` itself: on it, the four-target build matrix (macos-15, macos-15-intel, ubuntu-22.04, ubuntu-22.04-arm) runs, and the tag check, packaging and `publish` jobs run only for `refs/tags/v*`. So every change to the build is checked on all four targets before a release.
   - The worker cannot push. With the user's go-ahead, the lead pushes the branch and opens a draft pull request, and the user checks that CI and the release build pass on it before sub-spec 02 starts.

## Acceptance criteria

- [ ] `mise install` then `just check` passes from a clean clone (network on), including the smoke test.
- [ ] `cargo tree -p deck-daemon` shows `ghostty-vt` 0.9.0 and `ghostty-vt-sys`.
- [ ] No build-mode variable is set anywhere.
- [ ] architecture.md (dependencies, Zig, first build) and `AGENTS.md` are updated.
- [ ] `release.yml` installs Zig, builds on pull requests that touch the build, and publishes only on `v*` tags.
- [ ] CI and the four-target release build pass on the draft pull request (checked by the user).

## Tests

- The smoke test of requirement 5; every existing test, unchanged.
