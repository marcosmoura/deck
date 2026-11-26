---
type: foundation
status: done
---

# 03 — Privacy review

## Scope

What deck learns about the user, where it keeps it, how long, who can read it, and what leaves the machine; fixes within budget; the report. The audit was run by reviewers on 24 Nov 2026 with the user's disposition of each finding; the worker writes the report, the user page, the test and the fixes from it. Run on Fable 5.2 at `xhigh` (overview).

## Requirements

1. **Data inventory** in the report: every piece of user data deck reads, stores or sends at the reviewed commit (the list below is the minimum; overview, "Review the project at execution time"), with where (memory, `state.json`, `daemon.log`, handoff file, recordings and captures in the repo, the clipboard, the outer terminal, GitHub), its lifetime and file permissions. At least: terminal contents and scrollback, typed input, environment variables (the session environment kept in memory and passed through the handoff), cwds, titles, program names and arguments, agent session ids and first prompts, the agent session listings deck reads from `~/.claude`, Codex and OpenCode, hook payloads (`deck agent-event` stdin), clipboard contents (OSC 52), kitty image data, `deck stats` output, crash output.
2. **Questions** each answered in the report, plus any a feature added since the spec raises:
   - Does anything leave the machine (network calls, telemetry, update checks)? Expected: no; verify every dependency's features as well as deck's code.
   - Do logs contain user data (prompts, titles, paths, commands, environment values, clipboard)? Since 23 Nov 2026 the shutdown line logs pids and terminal ids only; check every `tracing` call.
   - Does `state.json` hold anything beyond the layout and DECK-04's documented fields (no full environment, no screen contents)?
   - Are secrets in the environment (tokens) ever written to disk (the handoff file is unlinked and 0600: is it ever left behind on a failure)?
   - What reaches the repository: e2e captures in `target/`, compat recordings (DECK-22's privacy test), test fixtures.
   - Retention: rotated logs, `.bad-<secs>` state files, backups made by `deck hooks install`; are any kept forever?
3. **A `docs/features/privacy.md`** page for users: what deck stores where, for how long, and what it never does, linked from `docs/features/README.md`.
4. Fixes and proposed tasks per the overview's fix budget.

## Acceptance criteria

- [ ] `docs/reviews/<date>-privacy.md` exists with the inventory, every question answered, and the findings table.
- [ ] `docs/features/privacy.md` exists and matches the inventory.
- [ ] An end-to-end test, `crates/deck/tests/privacy.rs` (new), as the privacy audit described it: a `Home` daemon whose environment holds `DECK_PRIVACY_SECRET=<random>` and a random non-`DECK_` variable, a random typed word, a random OSC 2 title and a random `UserPromptSubmit` prompt (both longer than 32 cells), and a random OSC 52 copy. After `kill-server` (and again after a live upgrade, a failed one, and a restart), no file under the home contains any of those values, raw or base64, except the first 32 cells of the title or prompt as `panels[].name` in `state.json` (decided 24 Nov 2026: auto panel names are kept and documented).

## Tests

- `crates/deck/tests/privacy.rs`; one per fix.
