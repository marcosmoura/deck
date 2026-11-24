---
type: shippable
status: done
---

# 01 — Release pull requests

## Scope

Pushes to `main` maintain a release pull request; merging it tags `v<version>`, and the existing build publishes the four tarballs. The user tries it by merging the first release pull request (`v0.1.0`). Owning area: repo (`.github/`, root config files, `.mise.toml`, `justfile`).

## Requirements

1. `release-please-config.json` and `.release-please-manifest.json` (`{".": "0.0.0"}`) at the root, with the settings of the overview's Decisions.
2. `.github/workflows/release-please.yml`: on push to `main`, runs release-please with `RELEASE_PLEASE_TOKEN`; minimal permissions; the action pinned by SHA.
3. **Fix-up job** (same workflow or its own, on pushes to release-please's branch): with the tools from `.mise.toml`, runs `cargo update --workspace` and `oxfmt CHANGELOG.md`, and if either changed a file, commits it to that branch through the GitHub contents API with `RELEASE_PLEASE_TOKEN`. It never pushes to `main`.
4. `release.yml`: a `concurrency` group keyed on the tag; the publish job uploads to an existing release (`gh release upload --clobber`) or creates it when missing; the rest unchanged.
5. `actionlint` and `shellcheck` in `.mise.toml`, `just lint-ci`, and a CI step on the Linux runner (overview).
6. `AGENTS.md`: `just lint-ci` in the commands table and one line on releases (merge the release pull request; never bump the version or tag by hand). `docs/architecture.md`: CI, Releases ("Cutting a release" rewritten), Dependencies (the two tools), a Decisions row.

## Acceptance criteria

- [ ] `just lint-ci` passes, and fails on a workflow with a deliberate error (checked once, not committed).
- [ ] The config files hold every setting of the overview (`package-name`, `include-component-in-tag: false`, `bump-minor-pre-major`, the changelog sections, the title pattern, the `extra-files` entry) and the manifest starts at `0.0.0`.
- [ ] `release.yml` keeps its tag trigger, has the concurrency group, and creates or uploads.
- [ ] The worker's report lists the user steps and what the lead checks on the first release pull request and its merge (below).

## Tests

- None in Rust. After merge, with the tokens in place, the lead checks and records in Notes: the release pull request's title is `chore(repo): release 0.1.0`; it proposes the tag `v0.1.0`; `Cargo.toml` and `Cargo.lock` are consistent and its CI is green (fix-up job included); after the user merges it, one `release.yml` run publishes the release with eight assets.

## Notes

- Workflows cannot be run before merge; keep them simple enough to review by reading.
- The first release pull request does not change the version's value (0.1.0), so the version-update path is fully exercised only by the second release; the lead checks that one too.
