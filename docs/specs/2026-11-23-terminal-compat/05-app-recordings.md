---
type: shippable
status: done
---

# 05 — Recordings of real programs

## Scope

Real programs' output, recorded once and committed, replayed through the differential harness on every `just test`, so a regression in what neovim or htop look like inside deck fails CI. Includes the recipe to record more and the compatibility doc's program list. Owning crate: `deck` (tests), repo (`justfile`).

## Requirements

1. **Scenario files** in `crates/deck/tests/compat/scenarios/<name>.scenario`, one step per line: `run <argv…>`, `size <cols>x<rows>`, `wait <text>`, `send <text with escapes>`, `key <name>` (deck's key names, e.g. `C-x`, `Enter`, `Up`), `sleep <ms>`, `setup <sh command>` (run before `run`, in the scratch `HOME`).
2. **Recording**: `just record-compat <name>` runs an ignored test (`DECK_RECORD=<name> cargo nextest run -p deck --test compat_record --run-ignored only`) that plays the scenario with `Harness::spawn` (no deck) at the given size, and writes the program's raw output to `crates/deck/tests/compat/recordings/<name>.vt` (binary, not the escape syntax of [01](01-differential-harness.md)). It refuses to overwrite without `FORCE=1`.
   - **Environment**: cleared, then only `PATH`, `TERM=xterm-256color`, `COLORTERM=truecolor`, `LANG=en_US.UTF-8`, `HOME` and `XDG_*_HOME` pointing into a scratch directory, `USER=deck`, `LOGNAME=deck`, `HOSTNAME=deck`. No `GIT_*`, `EDITOR`, `LESS`, `FZF_*`, `BAT_*` or `NVIM_APPNAME`, so the user's dotfiles are never loaded.
   - **Identity**: the scratch `HOME` gets a `.gitconfig` with `name = deck` and `email = deck@example.invalid`; scenarios' `setup` steps make repositories there.
3. **Replay**: each recording is a compat case (the recorded size) and runs in `just test`.
4. **Recordings committed**, made on macOS by the worker: neovim (open a sample Rust file with syntax highlighting, a vertical split, a `:` command, quit), less (a long file, search, scroll), htop limited to one known process (`htop -p <pid of a sleep started by setup>`, 3 s, then quit), lazygit (a scratch repo with two commits, the log view, quit), fzf (a list, a query, select), `git log --graph --color` and `git diff --color` in a scratch repo, `bat` on a sample file. No `top`/`btop` (they list every process). A program not installed on the worker's machine is skipped and listed.
5. **Privacy**: the recorder refuses to write a recording that contains the recorder's real `$HOME`, `$USER` (when at least 4 characters long), host name (full and short, as OSC 7 carries it) or any `/Users/` or `/home/` path outside the scratch directory. A committed test checks every recording against a fixed denylist (`/Users/`, `/home/`, and the scratch-dir prefix is the only allowed path root) plus the current machine's `$HOME` and host name. The worker also reads each recording for tokens before committing.
6. **`docs/features/compatibility.md`** lists each recorded program and version, the manual checklist for programs that cannot be recorded (Claude Code, Codex and OpenCode: start, type a multi-line prompt with Shift+Enter, scroll their output, a permission dialog; ssh to localhost; tmux inside deck; vim with mouse on), and how to record a new one.

## Acceptance criteria

- [ ] `just record-compat <name>` records a scenario, and refuses to overwrite without `FORCE=1`.
- [ ] Every committed recording passes the differential compare, or is a known difference with a reason.
- [ ] The privacy test passes, and fails on a recording that contains the test's `$HOME` or a `/Users/` path; the recorder refuses such a recording.
- [ ] The compatibility doc has the program list with versions and the manual checklist.
- [ ] `just` has the recipe and `AGENTS.md`'s table names it.

## Tests

- `crates/deck/tests/compat_record.rs` (ignored, the recorder), the replays through `compat.rs`, the privacy test.

## Notes

- Recordings depend on program versions; a later re-record that changes them is fine as long as the differential compare passes.
- The worker runs the manual checklist once and records the results in the compatibility doc, with the date.

## Deviations

Recorded by the lead on 24 Nov 2026.

- A `stop` scenario step ends a recording while the program is still on screen: the replay compares only the final screen, and a full-screen program that quits leaves it blank. nvim, less, lazygit and fzf stop before quitting, so their quit path (leaving the alternate screen, resetting modes) is not covered by a recording.
- htop is not installed on the machine that made the recordings, so it has none; bat was installed and is recorded.
- The manual checklist was not run: an agent has no terminal window, agent accounts or ssh server. The compatibility doc says so; the user runs it.
