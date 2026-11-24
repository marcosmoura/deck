# Backlog

Read when you decide what to build next, pick up a task, or add, start or finish one.

Every task has an ID, a status and the tasks it depends on. A task is either a **simple task**, described by its note below, or **substantial**, which links its spec ([workflow.md](workflow.md#triage) says how to tell them apart). A spec's own progress lives in its sub-spec files ([specs/README.md](specs/README.md#status-rules)); the task's status summarizes it.

## Queue

Every open task, in the order the user set on 22 Nov 2026. "The next task" is picked from here ([Picking the next task](#picking-the-next-task)). The list order is the priority; adding or removing an item needs no renumbering.

6. DECK-21 Live upgrade
7. DECK-22 Terminal compatibility
8. DECK-24 Scrollback search and copy mode
9. DECK-26 Security, privacy and performance reviews
10. DECK-28 Stress suite follow-ups

### Not ordered yet

Open tasks the user has not placed in the queue:

- DECK-30 Blinking cursor shapes

## Tasks

| ID      | Title                                     | Status      | Depends on                                                    | Spec                                                                         |
| ------- | ----------------------------------------- | ----------- | ------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| DECK-01 | Core daemon and terminal                  | done        | —                                                             | [core-daemon-terminal](specs/2026-11-22-core-daemon-terminal/00-overview.md) |
| DECK-02 | Sessions, projects, panels and sidebar    | done        | DECK-01                                                       | [sessions-sidebar](specs/2026-11-22-sessions-sidebar/00-overview.md)         |
| DECK-03 | Grid layout                               | done        | DECK-02, DECK-08                                              | [grid-layout](specs/2026-11-22-grid-layout/00-overview.md)                   |
| DECK-04 | Persistence across daemon restarts        | done        | DECK-03, DECK-08                                              | [persistence](specs/2026-11-22-persistence/00-overview.md)                   |
| DECK-05 | Control API                               | done        | DECK-03, DECK-04                                              | [control-api](specs/2026-11-22-control-api/00-overview.md)                   |
| DECK-06 | Agent status, hooks and resume            | done        | DECK-05, DECK-04                                              | [agent-status](specs/2026-11-22-agent-status/00-overview.md)                 |
| DECK-08 | Config and theming                        | done        | DECK-02                                                       | [config-theming](specs/2026-11-22-config-theming/00-overview.md)             |
| DECK-09 | Sidebar titles with icons                 | done        | DECK-06                                                       | [sidebar-titles](specs/2026-11-22-sidebar-titles/00-overview.md)             |
| DECK-10 | Sidebar mouse and buttons                 | done        | DECK-06                                                       | [sidebar-mouse](specs/2026-11-22-sidebar-mouse/00-overview.md)               |
| DECK-11 | Restore agent sessions                    | done        | DECK-06, DECK-10                                              | [agent-restore](specs/2026-11-22-agent-restore/00-overview.md)               |
| DECK-12 | Agent logos as images                     | done        | DECK-09                                                       | [agent-logos](specs/2026-11-23-agent-logos/00-overview.md)                   |
| DECK-13 | OpenCode v2 hooks                         | done        | DECK-06                                                       | —                                                                            |
| DECK-14 | Release workflow and local install        | done        | —                                                             | —                                                                            |
| DECK-15 | Mouse text selection, copy on select      | done        | —                                                             | —                                                                            |
| DECK-16 | Call projects spaces                      | done        | —                                                             | —                                                                            |
| DECK-17 | Floating bar, sidebar highlight, menus    | done        | —                                                             | [floating-bar-menus](specs/2026-11-23-floating-bar-menus/00-overview.md)     |
| DECK-18 | Sidebar reorder and space gaps            | done        | —                                                             | [sidebar-reorder](specs/2026-11-23-sidebar-reorder/00-overview.md)           |
| DECK-19 | Images in panels                          | done        | —                                                             | [kitty-images](specs/2026-11-23-kitty-images/00-overview.md)                 |
| DECK-20 | Built-in icons                            | done        | —                                                             | [builtin-icons](specs/2026-11-23-builtin-icons/00-overview.md)               |
| DECK-21 | Live upgrade                              | in-progress | DECK-19                                                       | [live-upgrade](specs/2026-11-23-live-upgrade/00-overview.md)                 |
| DECK-22 | Terminal compatibility                    | in-progress | DECK-19                                                       | [terminal-compat](specs/2026-11-23-terminal-compat/00-overview.md)           |
| DECK-23 | Daemon stats and stress tests             | done        | —                                                             | [daemon-stats](specs/2026-11-23-daemon-stats/00-overview.md)                 |
| DECK-24 | Scrollback search and copy mode           | in-progress | DECK-19                                                       | [copy-search](specs/2026-11-23-copy-search/00-overview.md)                   |
| DECK-25 | Release automation and Homebrew           | done        | —                                                             | [release-homebrew](specs/2026-11-23-release-homebrew/00-overview.md)         |
| DECK-26 | Security, privacy and performance reviews | ready       | DECK-19, DECK-20, DECK-21, DECK-22, DECK-23, DECK-24, DECK-25 | [reviews](specs/2026-11-23-reviews/00-overview.md)                           |
| DECK-27 | Terminals in the sidebar                  | done        | —                                                             | [sidebar-terminals](specs/2026-11-24-sidebar-terminals/00-overview.md)       |
| DECK-28 | Stress suite follow-ups                   | todo        | DECK-23                                                       | —                                                                            |
| DECK-29 | Recreate the Homebrew tap                 | done        | DECK-25                                                       | —                                                                            |
| DECK-30 | Blinking cursor shapes                    | todo        | DECK-22                                                       | —                                                                            |

### Withdrawn

A task that will not be built moves here from the Tasks table, with the date and the reason, in a commit on `main` (`docs(backlog): withdraw DECK-NN`, made only when the user asks). It keeps its ID and is never picked.

| ID      | Title             | Withdrawn   | Reason                                                                                                        |
| ------- | ----------------- | ----------- | ------------------------------------------------------------------------------------------------------------- |
| DECK-07 | Auto-named panels | 22 Nov 2026 | Taken over by DECK-09 sub-spec 01, which names panels after every program, not only agents ([note](#deck-07)) |

## Notes

### DECK-30

**Blinking cursor shapes.** Found on 24 Nov 2026 by [DECK-22](specs/2026-11-23-terminal-compat/00-overview.md)'s compat suite: a program's blinking cursor shape (`DECSCUSR` 1, 3, 5) shows steady inside deck.

- What: the daemon's emulator (`emu.rs`) already gets `blinking` from Ghostty and drops it. Carry it in a `blink` field on the protocol's `Cursor` (a protocol bump), and have `deck-tui`'s `draw.rs::cursor_style` choose the `Blinking*` variants.
- Why: editors and shells that ask for a blinking cursor get a steady one.
- Acceptance: the compat cases `cursor_style_1`, `cursor_style_3` and `cursor_style_5` pass, and their known-difference entries are removed; a unit test of the client's cursor-style mapping; `docs/features/compatibility.md` no longer lists the difference.

### DECK-29

**Recreate the Homebrew tap.** Asked on 24 Nov 2026: `marcosmoura/homebrew-tap` dates from 2022 and holds only an unused `tsv-utils` formula; recreate it and overhaul the repository. Corrected the same day: the tap is shared by all of the owner's projects, not bound to deck.

- What: in `marcosmoura/homebrew-tap`, replace `main` with a fresh history (the same repository, so its URL, settings and secrets stay; `tsv-utils` and the old history are dropped). The tree: `README.md` (a personal tap, `brew install marcosmoura/tap/<name>` and the Homebrew 7 trust note, how a project publishes), `LICENSE` (MIT), a shared composite action `.github/actions/publish` (writes a rendered `Formula/<name>.rb` or `Casks/<name>.rb` through the contents API, creating or updating it; any error but a 404 fails), `.github/workflows/tests.yml` (on push and pull request: `brew style` and `brew audit --strict` on the tap, install and test every formula on macOS and Linux, install every cask on macOS, passing when there are none), `.github/dependabot.yml` (GitHub Actions, weekly), third-party actions pinned by SHA. No branch protection. deck's `homebrew` job renders its formula and calls the action at `@main`; any other project does the same. The force-push to `main` waits for the user's go-ahead.
- Why: the tap is where every project of the owner installs from; publishing and checking live once, in the tap.
- Acceptance: the tap's `main` has the new history and nothing of the old tree; its workflow passes `actionlint` locally and is green on GitHub; nothing in the tap assumes deck; deck's release job uses the shared action; `docs/architecture.md` (Releases, Homebrew tap) describes the shared tap.

### DECK-28

**Stress suite follow-ups.** Asked on 24 Nov 2026: the after-merge steps of [DECK-23](specs/2026-11-23-daemon-stats/00-overview.md), deferred to the end of the queue.

- What: run `DECK_SOAK_SECS=86400 just stress soak` once on the user's machine, in the background, and record its summary (rss and fds slopes, idle CPU, and whether CPU under load kept climbing, as one 600 s run hinted) in `docs/features/stats.md`; read the rss slope by hand, since the 50 MiB/h bound does not tighten with length. Once the repository has a GitHub remote, dispatch `.github/workflows/stress.yml` once, adjust any bound the hosted runners exceed (the 0.1 s flood echo bound first) and record the runners' numbers next to the macOS ones. That run is also the first check of the Linux `self_stats` code, never compiled so far. Optional: make `attach_loop` record its JSON before asserting, like the other scenarios.
- Why: DECK-23's bounds come from one macOS machine and a 600 s soak; the spec asks for a 24-hour soak and runner numbers.
- Acceptance: `docs/features/stats.md` has the 24-hour soak's summary with date and machine; once a remote exists, a green nightly run on `macos-15` and `ubuntu-24.04` with its numbers in the table, or the reason it could not run.

### DECK-16

**Call projects spaces.** Asked on 23 Nov 2026: the sidebar header reads `Spaces` with a `+ New space` chip, and everything the user sees or types says space, not project.

- What: every user-facing word: sidebar header and chip, prompts (`Space name:`, `Rename space:`), confirmations, flashes, empty states, the help overlay's `Spaces` group, CLI help and messages, `deck ls` output. Hard renames, no aliases: config keys `new_space`, `rename_space`, `close_space`; `deck new-project` becomes `deck new-space`; `--project` becomes `--space`; the `projects` key of `deck ls --json` becomes `spaces`. Rust identifiers, protocol types and persisted state keep the name project.
- Why: one name for one thing; "space" is the product's word.
- Acceptance: no user-facing string, config key, CLI command, flag or JSON key says project; tests updated to the new text; `docs/features/` and `docs/architecture.md` say space wherever they describe what the user sees or types.

### DECK-15

**Mouse text selection, copy on select.** Asked on 23 Nov 2026: select text in a terminal with the mouse, and copy it when the selection is made.

- What: in `deck-tui` only, from the client's mirror of the screen (no protocol change). A left drag in the focused terminal selects from the press cell to the pointer, clamped to that terminal's screen even when the pointer leaves it; a double-click selects a word, a triple-click the line, and a drag after them extends by words or lines. Releasing copies the selected text to the outer terminal with OSC 52 (`c`), each row trimmed of trailing blanks, rows joined with newlines, a wide character once. A click without a drag selects nothing. When the program has mouse reporting on, the mouse stays the program's unless Option/Alt is held at the press; in scroll mode the press always selects. The selection is drawn in the theme's selection colours and is cleared by the next press, and by a frame of its terminal that changes a selected row, the size or the scroll offset. Only the rows on screen can be selected (no auto-scroll into the history).
- Why: the only way to copy from a terminal today is a program's own OSC 52 or the outer terminal's bypass selection, which crosses grid borders and the sidebar.
- Acceptance: unit tests for the cell-to-text extraction (wide characters, gaps between runs, trailing blanks), word and line bounds, drag clamping, the Option/Alt override against mouse reporting, double and triple click, the OSC 52 output on release and the clearing rules; `docs/features/` and `docs/architecture.md` (client section and a Decisions row replacing "No copy mode in v1") describe it.

### DECK-14

**Release workflow and local install.** Asked on 23 Nov 2026: a release workflow, in CI and locally, that installs `deck` in the user's paths.

- What: `just install` builds and installs `deck` with `cargo install --path crates/deck --locked --root "${PREFIX:-$HOME/.local}"` (so `~/.local/bin/deck` by default) and warns when that `bin` directory is not on `PATH`. GitHub Actions (repository `marcosmoura/deck`): `.github/workflows/ci.yml` runs `just check` on macOS and Linux for pushes to `main` and pull requests, with the tools from `.mise.toml`; `.github/workflows/release.yml`, on a pushed `v*` tag that matches the workspace version, builds `deck` in release mode on native runners for `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`, packs each as `deck-<version>-<target>.tar.gz` (the binary and `README.md`) with a `.sha256` file, and publishes them in a GitHub release for the tag with `gh`. `install.sh` at the root (POSIX `sh`) picks the target from `uname`, downloads the latest release (or `DECK_VERSION`), checks the SHA-256 and installs the binary into `${DECK_INSTALL_DIR:-$HOME/.local/bin}`, warning when that is not on `PATH`.
- Why: today the only install path is a manual `cargo install` into `~/.cargo/bin`, and there is no CI and no binary release.
- Acceptance: `just install` installs a working `deck` into a temp `PREFIX`; both workflows pass `actionlint` and `install.sh` passes `shellcheck` (run locally, not added to the repo); `install.sh` installs from a locally built tarball served through its download-URL override, and refuses a bad checksum; the workflows are not run, since the repository has no remote yet (said in the summary); README (install), AGENTS.md (commands), `docs/architecture.md` (tools and a Decisions row) describe it.

### DECK-13

**OpenCode v2 hooks.** Asked on 23 Nov 2026: with OpenCode v2.3.2, the plugin `deck hooks install opencode` writes fails to load ("Plugin must export a default definition with an id and an effect or setup function").

- What: `deck hooks install opencode` writes an OpenCode v2 TUI plugin, `plugins/deck/tui.js` (default export `{ id, setup }`, auto-discovered by `packages/tui/src/plugin/discovery.ts` in `anomalyco/opencode`). It runs in each terminal's own TUI process, so `DECK_TERMINAL_ID` is that terminal's even when several TUIs share one OpenCode server (v2's default; a server plugin runs in that shared server and cannot tell terminals apart). It listens with `ctx.data.listen`, keeps only its own sessions' events (the sessions it has shown, their roots and their subagents; not its tabs, which TUIs of a directory share) and runs `deck agent-event --agent opencode --event <type>` with the event as JSON on stdin, in order and without waiting. The daemon's `OpenCodeTracker` (`deck-core::agents`) reads v2 events (`data`) and still v1's (`properties`). An old deck-written `plugins/deck.js` is reported `stale`, replaced by `install` and removed by `uninstall`; files deck did not write are never touched.
- Why: OpenCode panels get no `working`/`waiting`/`idle` status and no first-prompt name under v2, and OpenCode shows a plugin error on every start.
- Acceptance: unit tests of the tracker with v2-shaped events (status, permission, first prompt, subagent sessions) next to the v1 ones; the plugin text test updated; two TUIs sharing one OpenCode v2 server each forward only their own session's events under their own `DECK_TERMINAL_ID` (checked by running OpenCode, or the reason it could not be); `docs/features/agents.md` (the OpenCode hooks paragraph) describes v2, with the date and the sources checked.

### DECK-07

**Auto-named panels.** Asked on 22 Nov 2026: when an agent takes over a terminal, the panel's name in the sidebar follows what the agent is doing.

- What: a panel whose `name_source` is not `manual` takes its name from its first terminal (in grid order) that has an agent. Source, in order: the agent's terminal title (OSC 0/2), with leading spinner or status glyphs (any non-alphanumeric prefix) stripped, once it differs (case-insensitively) from the bare program name and the product names (`claude`, `Claude Code`, `codex`, `Codex`, `opencode`, `OpenCode`); otherwise the first prompt of the agent's session from its `UserPromptSubmit` hook event (DECK-06), first line only. Names are cut to 32 display cells (by Unicode width) with `…`. The naming logic is pure and lives in `deck-core`. The `name_source` becomes `auto`. A manual rename (`prefix ,` or `deck rename-panel`) sets `manual` and is never overwritten; `deck rename-panel --auto` (a new flag this task adds to DECK-05's command) returns the panel to `auto`. When the agent exits, the auto name stays until another agent renames it.
- Why: panels full of agents all read "claude" otherwise.
- Acceptance: unit tests in `deck-core` for the title cleanup (including product-name titles), the source order, the 32-cell cut with wide characters and the manual lock; an end-to-end test where a fake agent script sets its title with OSC 2 and its panel's sidebar row shows the new name, and where a manual rename then survives another title change; `docs/features/agents.md` (created by DECK-06) describes it.

## Statuses

A status says where the task stands. Dependencies are a separate column and never change the status.

| Status        | Meaning                                                                                                |
| ------------- | ------------------------------------------------------------------------------------------------------ |
| `todo`        | A simple task, described in its note. The next step is to do it.                                       |
| `needs spec`  | Agreed substantial work with no spec yet. The next step is to write one.                               |
| `draft spec`  | A spec is written and waits for the user's approval.                                                   |
| `ready`       | The spec is approved and no work on it has started.                                                    |
| `in-progress` | Work has started: a worktree or branch exists, or a sub-spec is committed. Another session may own it. |
| `done`        | A simple task is committed; a spec has every sub-spec `done` and its branch merged.                    |

## Dependencies

"Depends on" lists task IDs. A dependency is met when that task is `done` on `main` (merged). `<ID> sub-spec NN` is met only once that task is merged too: a sub-spec committed on an unmerged branch does not count.

## Picking the next task

The [queue](#queue) is the priority order, set by the user.

- **"The next task"**: read the backlog on `main` and walk the queue from the top. When a task is `in-progress`, ask the user one question: resume it here, or leave it (another session may own it) and take the next eligible task. Otherwise pick the first task whose status is `todo`, `ready`, `draft spec` or `needs spec` and whose dependencies are all met. Say which earlier tasks were skipped and which dependency holds each back.
- Tasks under [Not ordered yet](#not-ordered-yet) are never picked as "the next task"; ask the user to place them.
- **A named task** (`DECK-04`) is built as named. If a dependency is not met, report it and ask before going on.
- For `todo`, the lead does the task ([workflow.md](workflow.md#work)). For `needs spec`, it writes the spec from the task's note and asks for approval. For `draft spec`, it summarizes the written spec and asks for approval. For `ready` and `in-progress`, it builds or resumes it.
- Tasks can run at the same time, in separate sessions, when neither depends on the other.

## Keeping it current

Only the lead session changes a task's status, always in the main checkout and in a commit on `main` made at once, so every session sees it ([the commits](workflow.md#commits-the-flow-may-make)):

- New simple task: a `todo` row, its "Depends on" and a note (what, why, acceptance), committed as `docs(backlog): add DECK-NN`. It turns `done` in the task's own commit.
- New agreed substantial work with no spec: a `needs spec` row and a note on what the user asked for and when (`docs(backlog): add DECK-NN`).
- A new spec: a `draft spec` row (or the `needs spec` row turned into one) and the `backlog:` line in its overview, committed with the draft (`docs(spec): draft <slug>`).
- Spec approved: `ready`, in the `docs(spec): add <slug> spec` commit.
- Work starts: `in-progress`, in a `docs(spec): start <slug>` commit on `main` when the worktree is created.
- Merge: `done`, in the squash-merge commit. Discard: back to `ready` (`docs(spec): stop <slug>`).

A new ID is the highest ID ever used (Tasks and Withdrawn) plus one.

The queue changes in the same commits. A new task enters the queue where the user says; when the user names no place, it goes under [Not ordered yet](#not-ordered-yet) and the lead asks. A task leaves the queue in the commit that sets it `done` or withdraws it. Only the user reorders the queue. IDs never change and are never reused.
