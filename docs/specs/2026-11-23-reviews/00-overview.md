---
branch: feat/reviews
backlog: DECK-26
---

# Security, privacy and performance reviews

## Goal

Three thorough reviews of deck as it stands once DECK-19 to DECK-25 have landed, each ending with a written report in `docs/reviews/`, the small fixes made, and a backlog task for every larger finding. They are the last step before the user re-assesses whether deck is ready to be a daily driver.

Asked by the user on 23 Nov 2026, choosing "a report plus fixes for small findings, backlog tasks for the rest".

## Builds on

- Every spec before it, and in particular [DECK-19](../2026-11-23-kitty-images/00-overview.md) (images: file and shared-memory transmission mediums, a C library built with Zig), [DECK-21](../2026-11-23-live-upgrade/00-overview.md) (the handoff: exec, inherited fds, files in the runtime directory), [DECK-23](../2026-11-23-daemon-stats/00-overview.md) (`deck stats`, the stress suite) and [DECK-25](../2026-11-23-release-homebrew/00-overview.md) (release tokens, the tap).
- `docs/architecture.md` (crates, protocol, paths, `unsafe` placement) and `docs/features/`.

## Non-goals

- External audits, penetration tests against other machines, or fuzzing campaigns longer than the sub-spec's session (a fuzz target may be added if a finding calls for it).
- Fixing findings that exceed a sub-spec's budget: they become backlog tasks.
- Windows, multi-user hosts other than "another local user on the same machine", and network exposure (deck listens on no network port).

## Decisions

- **Model and effort (required by the user):** every agent that works on these sub-specs, the `worker` and the `reviewer`, runs on **Fable 5.2 (`claude-fable-5-2`) at `xhigh` effort**. This overrides the model and effort pinned in the wrappers (`.claude/agents/worker.md` and `reviewer.md` pin other models).
  - Only Claude Code runs them: the lead starts each agent with the Agent tool's `model: "fable"` and `effort: "xhigh"`. Codex cannot (its wrappers offer only its own models). OpenCode runs them only if its Claude provider offers Fable 5.2 at `xhigh`; otherwise the lead stops and tells the user.
  - The brief states the model and effort, and the agent's report starts with the model id it runs on. A report on another model is discarded and the lead stops.
  - Never inline: the lead may not be on Fable 5.2. Every code change goes to the same agent (SendMessage), MINOR review findings and merge-conflict fixes included; when that agent cannot be reached again, its replacement is also started on Fable 5.2 at `xhigh`.
- **One report per review**, `docs/reviews/<yyyy-mm-dd>-<security|privacy|performance>.md`, with: the commit reviewed, the scope and method, a threat model or data inventory, a findings table (id, severity `critical`/`high`/`medium`/`low`/`info`, area, `file:line`, what and why, status `fixed (sub-spec 0N, test <name>)` / `planned (sub-spec 04)` (performance findings waiting for 04; 04 turns each into `fixed` or a proposed task) / `DECK-NN` / `accepted: <reason>`), and the measurements (performance).
- **Fix budget, checkable:** a finding is fixed in the review's sub-spec only when the fix touches one crate, changes no protocol message or persisted field, adds no dependency, stays under about 200 changed lines, and comes with a regression test. At most 8 fixes per review sub-spec (the most severe first); everything else becomes a proposed task.
- **Proposed tasks:** the worker lists them in its report (title, severity, what, why, acceptance). The backlog is edited only on `main`, so the lead adds each as a `todo` or `needs spec` row there (`docs(spec): add DECK-NN`) before the sub-spec's commit, and the report cites those ids. New rows go under "Not ordered yet"; for each `critical` or `high` one the lead asks the user at once whether to place it first in the queue. Approving this spec authorizes those `docs(spec): add DECK-NN` commits.
- **Order:** security, then privacy, then performance measurements, then performance fixes, one at a time on the same branch, so later reviews see earlier fixes. The branch merges once all four are done.
- **New tool:** the security review adds `cargo-deny` to `.mise.toml` (aqua backend, `aqua:EmbarkStudios/cargo-deny`, pinned) with a `deny.toml` (advisories, licences, bans of duplicate majors where cheap, sources limited to crates.io), a `just audit` recipe, and a step in CI's `check` job on the Linux runner only. The advisory database changes by itself, so a new advisory can turn CI red without a code change: that is accepted, and the fix is an update or a reasoned ignore in `deny.toml`. Recorded in `docs/architecture.md#dependencies`. DECK-19's build-time fetches (Ghostty's source, Zig packages) are outside cargo-deny's view and are reviewed by hand.
- **Profilers are the user's:** `samply` or `cargo flamegraph` are not added to the repository. The lead asks the user to install one before sub-spec 03 starts; without one, the profiles are skipped and the report says so. `powermetrics` needs root and is left out.

## Sub-specs

| Sub-spec                      | Type       | Crate | Summary                                                                                   | Depends                                                       | Status |
| ----------------------------- | ---------- | ----- | ----------------------------------------------------------------------------------------- | ------------------------------------------------------------- | ------ |
| [01](01-security.md)          | foundation | all   | Security review, `cargo-deny` and `just audit`, up to 8 fixes, report                     | DECK-19, DECK-20, DECK-21, DECK-22, DECK-23, DECK-24, DECK-25 | todo   |
| [02](02-privacy.md)           | foundation | all   | Privacy review: what deck stores, logs, sends and reads; up to 8 fixes, report, user page | 01                                                            | todo   |
| [03](03-performance.md)       | foundation | all   | Performance measurements and profiles, a committed latency scenario, report               | 02                                                            | todo   |
| [04](04-performance-fixes.md) | foundation | all   | Up to 8 performance fixes from 03's findings, with before and after numbers               | 03                                                            | todo   |

## Affected docs

- `docs/reviews/` (new) with the three reports, and a `README.md` there listing them.
- `docs/index.md`: route to `docs/reviews/`.
- `docs/architecture.md`: `cargo-deny`, `just audit`, CI, and whatever a fix changes.
- `AGENTS.md`: `just audit` in the commands table.
- `docs/features/`: wherever a fix changes behaviour.

Reviewed by spec-reviewer on 23 Nov 2026.
