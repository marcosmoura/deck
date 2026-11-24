---
type: shippable
status: done
---

# 02 — Homebrew formula

## Scope

`brew install marcosmoura/tap/deck` installs the released binary on macOS and Linux, and every release updates the formula. Owning area: repo (`LICENSE`, `packaging/homebrew/`, `.github/workflows/release.yml`, `README.md`).

## Requirements

1. `LICENSE` at the root with the MIT text (copyright the repository owner, as in the git history), matching `Cargo.toml`'s `license = "MIT"`.
2. `packaging/homebrew/deck.rb.in`: a formula template in the style of the tap's `Formula/tsv-utils.rb`, with `desc`, `homepage`, `license "MIT"`, `on_macos`/`on_linux` blocks each with `on_arm`/`on_intel` `url` and `sha256` for the four release tarballs (no separate `version` line; Homebrew reads it from the URL), `def install` with `bin.install "deck"`, a `test do` that asserts `deck --version` prints the version, and `caveats` telling the user to run `deck hooks install` for agent status. Placeholders are `@VERSION@` and `@SHA256_<TARGET>@`.
3. `packaging/homebrew/render.sh` (POSIX `sh`): `render.sh <version> <dir with .sha256 files>` prints the formula; it fails naming the missing target when a sum is missing.
4. A `homebrew` job in `release.yml`, after the publish job: downloads the release's `.sha256` files, renders the formula, and writes `Formula/deck.rb` in `marcosmoura/homebrew-tap` through the GitHub contents API with `HOMEBREW_TAP_TOKEN`, message `deck <version>`. A failure there does not undo the release.
5. `README.md` lists Homebrew first under Install, then `install.sh`, then from source.

## Acceptance criteria

- [ ] A Rust-free check in `just lint-ci` (or a test script it runs) renders the template with fake sums and fails if any placeholder is left; `render.sh` with a missing sum fails naming the target.
- [ ] `just lint-ci` passes (shellcheck on `render.sh`, actionlint on `release.yml`).
- [ ] `LICENSE` exists and `README.md` has the Homebrew install first.

## Tests

- The render check above. After the first release, the user runs `brew install marcosmoura/tap/deck` and `brew test deck`; the lead records the result in Notes.
