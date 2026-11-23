---
type: shippable
status: done
---

# 01 — Panels named after what runs in them

## Scope

The sidebar's panel names follow what runs in each panel: the title an agent or a shell sets, the first prompt of an agent, or the program's name, until the user renames the panel. This is DECK-07's auto-naming, extended to every program. Owning crates: `deck-core` (the pure naming rules), `deck-daemon` (program tracking, applying names), `deck` (`rename-panel --auto`).

## Requirements

1. **Program per terminal**: `Terminal.program: String` (DECK-02's field, the resolved shell's basename at launch) now follows the foreground process. On DECK-06's 1 s detection tick, when `platform::foreground_process` returns an argv, the daemon sets `program` to `deck-core::agents::program_name(argv)`; when it returns `None` (the process belongs to another user, as with `sudo vim`, or exited during the read), `program` keeps its last value. `TerminalView` and `deck ls --json` carry it (`"program": "nvim"`). The protocol version bumps.
2. **`program_name(argv)`** (pure, `deck-core::agents`, also used by DECK-06's `match_argv`, which keeps its own criteria): the basename of `argv[0]` with one leading `-` removed (login shells: `-zsh` → `zsh`); when that is an interpreter (`node`, `bun`, `deno`, `sh`, `bash`, `zsh`, `dash`) and `argv[1]` exists and does not start with `-`, the basename of `argv[1]` instead; then the extension is removed (`codex.js` → `codex`). So the default shell (`/bin/zsh -l`) is `zsh`, `bash -c x` is `bash`, and `sh /tmp/fake/claude` is `claude`.
3. **Naming rule** (pure, `deck-core::naming::auto_name(input: &NameInput) -> Option<String>`, where `NameInput { agent: Option<&AgentState>, title: Option<&str>, program: &str }`; the daemon builds it from its model `Terminal`), applied to the panel's source terminal (overview):
   - **Agent** (`agent` is `Some`): its OSC 0/2 title with any leading non-alphanumeric characters stripped (spinners, status glyphs, spaces), once it is non-empty and differs, case-insensitively, from `program` and the product names (`claude`, `Claude Code`, `codex`, `Codex`, `opencode`, `OpenCode`); otherwise the first line of the agent's `first_prompt` (DECK-06); otherwise `None` (the name does not change).
   - **Any other program**: its title, only trimmed (a path such as `~/src` or vim's `[No Name] - VIM` is kept as is), when non-empty; otherwise `program`.
   - The result is trimmed, control characters are removed, and it is cut to 32 display cells (Unicode width) ending in `…`.
4. **Applying names**: the daemon recomputes the name of each affected panel whose `name_source` is `default` or `auto` after a title change, a detection tick that changed a `program` or an agent state, an agent event, a focus change inside a panel, a terminal added to or removed from a panel, and a change of grid order (DECK-03). When the result is `Some` and differs from the current name, it sets the name and `name_source = auto` and sends the new `SessionView`. Nothing is sent when the name is unchanged, so a spinner in an agent's title (stripped) does not cause traffic.
5. **Manual lock**: a manual rename (`RenamePanel`, `prefix ,`, `deck rename-panel <name>`) sets `name_source = manual`, and requirement 4 never touches that panel. `deck rename-panel --auto` (new flag on DECK-05's command; a name and `--auto` together are a usage error) sets `name_source = auto` and applies requirement 4 at once; the protocol gains `Action::AutoNamePanel { panel }` for it.
6. **Agent exit**: when the agent leaves, the source terminal is chosen again and the name follows it (usually the shell's title or name). This changes DECK-07's note, which kept the agent's name after it exited.
7. **New panels** start with DECK-02's numbered default name, which is only a fallback for the moment before the panel's first name: a panel created by `NewPanel`, `prefix c`, `deck new-panel` or `NewProject`, and the first panel of a new session, is named at once from its shell's program (or title). A panel restored from `state.json` whose `name_source` is `default` or `auto` is recomputed when its terminal is restored; a `manual` one never is.

## Screen

Sidebar of a 100×24 window, before sub-spec 02's icons. `api` was renamed by hand; `Fix login bug` is Claude Code's title with its spinner glyph stripped; `Review the API changes` is Codex's first prompt (its title is still `codex`):

```text
 main                       │
                            │
 ▾ terminal-multiplexer     │
▌⠹ Fix login bug            │
   nvim                     │
 ! Review the API changes   │
   ~/Projects/terminal-mul… │
   api                      │
```

## Acceptance criteria

- [ ] `program_name` unit tests: `["/bin/zsh", "-l"]` → `zsh`, `["-zsh"]` → `zsh`, `["sh"]` → `sh`, `["bash", "-c", "x"]` → `bash`, `["node", "/x/codex.js"]` → `codex`, `["/bin/sh", "/tmp/fake/claude"]` → `claude`; DECK-06's `match_argv` criteria still pass.
- [ ] `auto_name` unit tests: an agent title `✳ Fix login bug` gives `Fix login bug`; agent titles `claude`, `Claude Code`, `⠹ codex` fall back to the first prompt, and with no prompt give `None`; a shell with title `~/src` gives `~/src`; a shell with no title gives `zsh`; a 40-character title and a title of wide characters are cut to 32 cells with `…`; control characters are removed.
- [ ] Source terminal unit tests (with DECK-03's grid order): the first agent terminal wins over a focused non-agent terminal; with no agent, the focused terminal.
- [ ] A failed foreground read keeps the terminal's last `program` (daemon test with an injected `foreground_process` result).
- [ ] A fake agent (DECK-06's `#!/bin/sh` script named `claude` in the harness) that prints `ESC]2;✳ Fix login bug BEL` renames its panel to `Fix login bug` in the sidebar (capture `agent-title`); a fake agent that only sends an `AgentEvent` with a prompt gets the prompt's first line.
- [ ] A fake program (a `#!/bin/sh` script named `nvim` on the harness `PATH` that sets no title and waits on stdin), started in a shell without a title, renames the panel to `nvim` within 2 s, and back to the shell's name when it exits.
- [ ] After `prefix ,` renames a panel to `api`, a new title does not change it; `deck rename-panel --auto` brings back the live name.
- [ ] `deck rename-panel --auto build` fails with a usage error.
- [ ] A title that changes only in its leading spinner glyph sends no new `SessionView` (daemon test counting views).
- [ ] `deck ls --json` shows `program` per terminal and `name_source` `auto` for an auto-named panel.

## Tests

- `deck-core`: `auto_name`, source-terminal choice, program extraction from argv.
- `crates/deck-daemon/tests/`: names applied on title change, on agent events and on the tick; the manual lock; `AutoNamePanel`; no view for an unchanged name.
- `crates/deck/tests/titles.rs`: the end-to-end criteria.

## Notes

- From DECK-07's note: names are cut to 32 cells and the manual lock is never overwritten. One change: DECK-07 kept an agent's name after it exited; here the name follows the new source terminal (requirement 5), since every panel now has a live name.
