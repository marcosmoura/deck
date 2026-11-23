---
type: foundation
status: todo
---

# 02 — Privacy review

## Scope

What deck learns about the user, where it keeps it, how long, who can read it, and what leaves the machine; fixes within budget; the report. Run on Fable 5.2 at `xhigh` (overview).

## Requirements

1. **Data inventory** in the report: every piece of user data deck reads, stores or sends, with where (memory, `state.json`, `daemon.log`, handoff file, recordings and captures in the repo, the clipboard, the outer terminal, GitHub), its lifetime and file permissions. At least: terminal contents and scrollback, typed input, environment variables (the session environment kept in memory and passed through the handoff), cwds, titles, program names and arguments, agent session ids and first prompts, the agent session listings deck reads from `~/.claude`, Codex and OpenCode, hook payloads (`deck agent-event` stdin), clipboard contents (OSC 52), kitty image data, `deck stats` output, crash output.
2. **Questions** each answered in the report:
   - Does anything leave the machine (network calls, telemetry, update checks)? Expected: no; verify every dependency's features as well as deck's code.
   - Do logs contain user data (prompts, titles, paths, commands, environment values, clipboard)? Since 23 Nov 2026 the shutdown line logs pids and terminal ids only; check every `tracing` call.
   - Does `state.json` hold anything beyond the layout and DECK-04's documented fields (no full environment, no screen contents)?
   - Are secrets in the environment (tokens) ever written to disk (the handoff file is unlinked and 0600: is it ever left behind on a failure)?
   - What reaches the repository: e2e captures in `target/`, compat recordings (DECK-22's privacy test), test fixtures.
   - Retention: rotated logs, `.bad-<secs>` state files, backups made by `deck hooks install`; are any kept forever?
3. **A `docs/features/privacy.md`** page for users: what deck stores where, for how long, and what it never does, linked from `docs/features/README.md`.
4. Fixes (at most 8) and proposed tasks per the overview.

## Acceptance criteria

- [ ] `docs/reviews/<date>-privacy.md` exists with the inventory, every question answered, and the findings table.
- [ ] `docs/features/privacy.md` exists and matches the inventory.
- [ ] An end-to-end test, `crates/deck/tests/privacy.rs` (new), runs a `Home` daemon whose environment holds `DECK_PRIVACY_SECRET=<random>`, types a random word, sets a random title with OSC 2, sends a `UserPromptSubmit` hook event with a random prompt through `deck agent-event`, and copies a random string with OSC 52 (`printf` in the terminal); after `kill-server`, `daemon.log` and `state.json` contain none of those values.

## Tests

- `crates/deck/tests/privacy.rs`; one per fix.
