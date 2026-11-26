---
branch: feat/reviews
backlog: DECK-26
---

# Security, privacy and performance reviews

## Goal

Three thorough reviews of deck as it stands when each review starts (at least DECK-19 to DECK-25, plus everything merged since), each ending with a written report in `docs/reviews/`, the small fixes made, and a backlog task for every larger finding. They are the last step before the user re-assesses whether deck is ready to be a daily driver.

Asked by the user on 23 Nov 2026, choosing "a report plus fixes for small findings, backlog tasks for the rest".

## Builds on

- Every spec before it, and in particular [DECK-19](../2026-11-23-kitty-images/00-overview.md) (images: file and shared-memory transmission mediums, a C library built with Zig), [DECK-21](../2026-11-23-live-upgrade/00-overview.md) (the handoff: exec, inherited fds, files in the runtime directory), [DECK-23](../2026-11-23-daemon-stats/00-overview.md) (`deck stats`, the stress suite) and [DECK-25](../2026-11-23-release-homebrew/00-overview.md) (release tokens, the tap).
- `docs/architecture.md` (crates, protocol, paths, `unsafe` placement) and `docs/features/`.

## Non-goals

- External audits, penetration tests against other machines, or fuzzing campaigns longer than the sub-spec's session (a fuzz target may be added if a finding calls for it).
- Fixing findings that exceed a sub-spec's budget: they become backlog tasks.
- Windows, multi-user hosts other than "another local user on the same machine", and network exposure (deck listens on no network port).

## Decisions

- **Model and effort (required by the user; for workers replaced on 25 Nov 2026, see Notes):** every agent that works on these sub-specs, the `worker` and the `reviewer`, runs on **Fable 5.2 (`claude-fable-5-2`) at `xhigh` effort**. This overrides the model and effort pinned in the wrappers (`.claude/agents/worker.md` and `reviewer.md` pin other models).
  - Only Claude Code runs them: the lead starts each agent with the Agent tool's `model: "fable"` and `effort: "xhigh"`. Codex cannot (its wrappers offer only its own models). OpenCode runs them only if its Claude provider offers Fable 5.2 at `xhigh`; otherwise the lead stops and tells the user.
  - The brief states the model and effort, and the agent's report starts with the model id it runs on. A report on another model is discarded and the lead stops.
  - Never inline: the lead may not be on Fable 5.2. Every code change goes to the same agent (SendMessage), MINOR review findings and merge-conflict fixes included; when that agent cannot be reached again, its replacement is also started on Fable 5.2 at `xhigh`.
- **One report per review**, `docs/reviews/<yyyy-mm-dd>-<security|privacy|performance>.md`, with: the commit reviewed, the scope and method, a threat model or data inventory, a findings table (id, severity `critical`/`high`/`medium`/`low`/`info`, area, `file:line`, what and why, status `fixed (sub-spec 0N, test <name>)` / `planned (sub-spec 0N)` (findings waiting for a later fix sub-spec: 02 for security, 06 for performance; that sub-spec turns each into `fixed` or a proposed task) / `DECK-NN` / `accepted: <reason>`), and the measurements (performance).
- **Audits run first, by reviewers (required by the user, 24 Nov 2026):** the analysis of each review (security 01, privacy 03, and for performance 04 the code analysis of hot paths, wake-up sources and allocations) is done by read-only `reviewer` agents on Fable 5.2 at `xhigh`, split by area when the scope is large, before any worker starts. Their findings are not committed (the user's rule, 25 Nov 2026: no audit files in the repository): the lead keeps them outside the repository, shows them to the user with a proposed disposition (fix here, fix in a follow-up sub-spec, backlog task, accepted), and only then a `worker` gets them: it writes the report, the tooling and the fixes, and does not redo the analysis. Performance measurements need code (scenarios), so in 04 the worker writes and runs them after the reviewers' analysis.
- **Review the project at execution time, not at spec time:** this spec was written on 23 Nov 2026 and features keep landing before and between its sub-specs. Each audit reviews `main` as it is when the audit starts (the worktree is rebased onto `main` first when `main` moved), and the worker confirms each finding still holds at the commit it starts from. Before auditing, the reviewers inventory what deck does at that commit (`docs/features/`, the `done` rows of `docs/backlog.md`, `git log` since this spec's approval commit, the crates' modules) and adds an area, question or measurement for every feature the sub-spec's lists do not cover. Those lists are the minimum, not the scope. Names, files, fields, periods and limits cited in the sub-specs are hints from 23 Nov 2026: the reviewers check each against the code and the report records what differed. The report states the commit reviewed and the features added since the spec.
- **Fix budget, checkable:** a finding is fixed in the review's sub-spec only when the fix touches one crate, changes no protocol message or persisted field, adds no dependency, stays under about 200 changed lines, and comes with a regression test. There is no fixed number of fixes: every finding that fits this budget is fixed, the most severe (or, for performance, the most impactful) first. When they would not fit one worker session, the lead proposes a split from the audit's findings and the user chooses: fix the rest in this sub-spec, in a follow-up sub-spec of this spec, or as proposed tasks. Findings over the budget become proposed tasks.
- **Proposed tasks:** the audit lists them (title, severity, what, why, acceptance). The backlog is edited only on `main`, so the lead adds each as a `todo` or `needs spec` row there (`docs(spec): add DECK-NN`) before the worker starts, and the report cites those ids. New rows go under "Not ordered yet"; for each `critical` or `high` one the lead asks the user at once whether to place it first in the queue. Approving this spec authorizes those `docs(spec): add DECK-NN` commits.
- **Order:** security, then security follow-up fixes, then privacy, then performance measurements in two rounds, then performance fixes, one at a time on the same branch, so later reviews see earlier fixes. The branch merges once all six are done.
- **New tool:** the security review adds `cargo-deny` to `.mise.toml` (aqua backend, `aqua:EmbarkStudios/cargo-deny`, pinned) with a `deny.toml` (advisories, licences, bans of duplicate majors where cheap, sources limited to crates.io), a `just audit` recipe, and a step in CI's `check` job on the Linux runner only. The advisory database changes by itself, so a new advisory can turn CI red without a code change: that is accepted, and the fix is an update or a reasoned ignore in `deny.toml`. Recorded in `docs/architecture.md#dependencies`. DECK-19's build-time fetches (Ghostty's source, Zig packages) are outside cargo-deny's view and are reviewed by hand.
- **Profilers are the user's:** `samply` or `cargo flamegraph` are not added to the repository. The lead asks the user to install one before sub-spec 04 starts; without one, the profiles are skipped and the report says so. `powermetrics` needs root and is left out.

## Sub-specs

| Sub-spec                             | Type       | Crate | Summary                                                                                                                                     | Depends                                                       | Status |
| ------------------------------------ | ---------- | ----- | ------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------- | ------ |
| [01](01-security.md)                 | foundation | all   | Security review report from the audit, `cargo-deny` and `just audit`, the high and medium fixes                                             | DECK-19, DECK-20, DECK-21, DECK-22, DECK-23, DECK-24, DECK-25 | done   |
| [02](02-security-fixes.md)           | foundation | all   | The audit's low and cheap info fixes, decided by the user on 24 Nov 2026                                                                    | 01                                                            | done   |
| [03](03-privacy.md)                  | foundation | all   | Privacy review: what deck stores, logs, sends and reads; fixes, report, user page                                                           | 02                                                            | done   |
| [04](04-performance.md)              | foundation | all   | Measuring tools, first round of measurements (latency, stalls, throughput, idle, memory, frame cost), the `kill-server` fix, report started | 03                                                            | done   |
| [05](05-performance-second-round.md) | foundation | all   | Second round of measurements (start-up, live upgrade, control connections, paste, images, client memory, benchmarks), report finished       | 04                                                            | done   |
| [06](06-performance-fixes.md)        | foundation | all   | Performance fixes from the measured findings within budget, with before and after numbers                                                   | 05                                                            | done   |

## Affected docs

- `docs/reviews/` (new) with the three reports, and a `README.md` there listing them.
- `docs/index.md`: route to `docs/reviews/`.
- `docs/architecture.md`: `cargo-deny`, `just audit`, CI, and whatever a fix changes.
- `AGENTS.md`: `just audit` in the commands table.
- `docs/features/`: wherever a fix changes behaviour.

Reviewed by spec-reviewer on 23 Nov 2026.

## Notes

- 24 Nov 2026, asked by the user: several features landed after the spec was written (DECK-27, DECK-29, DECK-30, DECK-31 at least), so every sub-spec now reviews the project as it is when it starts, not as described here, and the cap of 8 fixes per sub-spec is gone (Decisions: "Review the project at execution time", "Fix budget").
- 24 Nov 2026, asked by the user after a worker started analysing and writing at once (its work was reverted): audits run first, by read-only reviewers, and only their findings go to a worker (Decisions: "Audits run first"). The security audit (four reviewers) found more fixes than one session holds, so the user chose the split: 01 takes the high and medium fixes, a new 02 the low and cheap info ones; privacy and performance moved to 03, 04 and 05. Backlog tasks DECK-32 to DECK-37 hold the findings over budget.
- 25 Nov 2026, asked by the user: audit files are not kept in the repository. The reviewers' findings live outside it, with the lead, who hands them to each worker; only the reports in `docs/reviews/` are committed.
- 25 Nov 2026, decided by the user: the performance measurements are split in two rounds (04, new 05) because the analysis asked for more than one worker session; the fixes moved to 06. `deck kill-server` during a live upgrade is fixed in 04 because it turns CI red on `main`. The spinner draws only while visible, at 10 frames a second everywhere (06). No profiler is installed; the report says so unless one is there when 04's worker starts.
- 25 Nov 2026, asked by the user: workers no longer run on Fable 5.2. From sub-spec 04 on, each `worker` runs on the model and effort its wrapper pins (`.claude/agents/worker.md`); reviewers keep Fable 5.2 at `xhigh` as decided above.
- 25 Nov 2026, decided by the user: two of 06's fixes are accepted as certain from the code without before and after numbers, since measuring them needs new scenarios: the client handling input before daemon messages (G-7) and `Capture` running off the async threads (F-15). The parts of 06's findings left over are backlog tasks DECK-42 to DECK-47.
- 26 Nov 2026, decided by the user: every spinner runs at 8 frames a second (main's eight-spoke loader and its spinner for running commands), drawn only while visible; this replaces the 10 frames a second of the 25 Nov note (06).
