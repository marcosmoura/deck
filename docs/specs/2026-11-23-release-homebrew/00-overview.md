---
branch: feat/release-homebrew
backlog: DECK-25
---

# Release automation and Homebrew

## Goal

Releasing deck is merging a pull request, and installing it is `brew install marcosmoura/tap/deck`. Every push to `main` keeps an open "release" pull request up to date with the next version and a changelog written from the Conventional Commits since the last release; merging it tags the release, builds the four targets (DECK-14's workflow) and updates the Homebrew formula in `marcosmoura/homebrew-tap`.

Asked by the user on 23 Nov 2026, choosing a release pull request (release-please) over releasing on every merge, and a formula in the existing tap over a cask.

## Builds on

- [DECK-14](../../backlog.md#deck-14): `.github/workflows/release.yml` (four native targets, tarballs and `.sha256` files, `gh release create`), `install.sh`, `just install`, `.github/workflows/ci.yml`.
- The commit rules in `AGENTS.md` (Conventional Commits, types and scopes, checked by Lefthook).

## Non-goals

- `homebrew-core`, a cask, Linux distribution packages (deb, rpm, AUR, Nix) and crates.io.
- Signing or notarising the macOS binary. Homebrew installs from a formula without notarisation, and `install.sh` downloads with `curl`, which sets no quarantine flag.
- Release notes beyond the generated changelog.
- Changing what the build produces (targets, archive layout).

## Decisions

- **release-please** (`googleapis/release-please-action`, pinned by commit SHA, as every third-party action this spec adds) with the `simple` release type (the `rust` type fails on a virtual workspace root: its Cargo.toml updater needs a `[package]`), configured by `release-please-config.json` and `.release-please-manifest.json` at the root: `package-name: deck`, `initial-version: 0.1.0`, `include-component-in-tag: false` (tags are `v<version>`, as `release.yml` requires), `bump-minor-pre-major: true` (before 1.0, `feat` bumps the minor version and breaking changes do not bump the major), changelog sections `feat`, `fix` and `perf` (others hidden), and `pull-request-title-pattern: "chore(repo): release ${version}"` so the squash-merged release commit follows `AGENTS.md`'s commit rules.
- **First release without pinning:** the manifest starts at `0.0.0` and `initial-version: 0.1.0` makes the first release `v0.1.0` (without it the `simple` type would start at 1.0.0; the workspace is already at 0.1.0, so the first release pull request leaves it unchanged in value but goes through the update path). No `release-as`, so no later commit is needed to remove it.
- **Version files:** `[workspace.package] version` in the root `Cargo.toml` is updated through an `extra-files` entry (`type: toml`, `jsonpath: $.workspace.package.version`), because the `simple` strategy knows no `Cargo.toml` (it only writes `version.txt`, skipped when missing). `Cargo.lock` and `CHANGELOG.md` are fixed on the release branch by a **fix-up job**: on every push to release-please's branch it runs `cargo update --workspace` and `oxfmt CHANGELOG.md` (release-please's Markdown fails `oxfmt --check`, which `just check` runs), and commits the result through the GitHub API with `RELEASE_PLEASE_TOKEN` (so the commit is the token owner's and CI runs again). A push with nothing to fix commits nothing, which ends the loop.
- **One build path:** release-please runs with a fine-grained personal access token, so the `v*` tag it creates fires `release.yml`'s existing `push: tags` trigger; there is no `workflow_call`. `release.yml`'s publish job uploads the assets to the release if it exists (`gh release upload --clobber`) and creates it otherwise (a tag pushed by hand). A `concurrency` group keyed on the tag prevents two runs for one release.
- **Tokens** (the user creates them; fine-grained tokens expire, so the user renews them; the README's release section says so):
  - `RELEASE_PLEASE_TOKEN`: on `marcosmoura/deck` only, Contents, Pull requests and Issues (release-please's labels) read and write. Needed because pull requests and tags made with the default `GITHUB_TOKEN` trigger no workflows.
  - `HOMEBREW_TAP_TOKEN`: on `marcosmoura/homebrew-tap` only, Contents read and write.
- **Formula:** `Formula/deck.rb` in `marcosmoura/homebrew-tap`, rendered from `packaging/homebrew/deck.rb.in` in this repository with the version and the four SHA-256 sums, installing the prebuilt binary (`on_macos`/`on_linux` × `on_arm`/`on_intel`), no redundant `version` line, a `test` block that runs `deck --version`, and caveats about `deck hooks install`. The release workflow writes it to the tap with the GitHub contents API (`HOMEBREW_TAP_TOKEN`; no git identity needed in CI), message `deck <version>`. [DECK-21](../2026-11-23-live-upgrade/00-overview.md) owns adding a `deck upgrade` caveat.
- **Checking workflows and scripts:** `actionlint` and `shellcheck` join `.mise.toml` (aqua backend, pinned) with a `just lint-ci` recipe (both on `.github/workflows/`, `install.sh` and `packaging/homebrew/render.sh`) and a step in CI's `check` job on the Linux runner, recorded in `docs/architecture.md#dependencies`. Everything else about the workflows is verified on the first real release, after merge.
- **Known gap:** release-please publishes the GitHub release before the build finishes (about 10 minutes), so for that time the latest release has no assets and `install.sh` fails with a clear message. Accepted and documented.

## Sub-specs

| Sub-spec                   | Type      | Crate | Summary                                                                                  | Depends | Status |
| -------------------------- | --------- | ----- | ---------------------------------------------------------------------------------------- | ------- | ------ |
| [01](01-release-please.md) | shippable | repo  | release-please config and workflow, fix-up job, create-or-upload publish, `just lint-ci` | —       | done   |
| [02](02-homebrew.md)       | shippable | repo  | LICENSE, formula template and renderer, tap update job, install docs                     | 01      | done   |

## Before the work (the user)

1. Create the fine-grained token `RELEASE_PLEASE_TOKEN` (repository `marcosmoura/deck`; Contents, Pull requests and Issues: read and write) and add it as an Actions secret of `marcosmoura/deck`.
2. Create the fine-grained token `HOMEBREW_TAP_TOKEN` (repository `marcosmoura/homebrew-tap`; Contents: read and write) and add it as an Actions secret of `marcosmoura/deck`.

## Affected docs

- `README.md`: install with Homebrew first, then `install.sh`, then from source.
- `docs/architecture.md`: CI and Releases (release-please, the reusable build, the tap update, the tokens), "Cutting a release" rewritten, a Decisions row.
- `AGENTS.md`: a line on releases (merge the release pull request; never bump the version by hand).
- `docs/workflow.md`: the release pull request is merged by the user, never by an agent (one line under Commits).
- `LICENSE` (new, MIT, as `Cargo.toml` declares).

Reviewed by spec-reviewer on 23 Nov 2026.
