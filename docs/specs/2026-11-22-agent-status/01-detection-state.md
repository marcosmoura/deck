---
type: shippable
status: todo
---

# 01 — Detection, state model and status glyphs

## Scope

When the user runs `claude`, `codex` or `opencode` in a terminal, the sidebar and the terminal's title show the agent and whether it is printing. This sub-spec builds the whole per-terminal state model, so the later sub-specs only feed it (hooks, 03 and 04) or react to it (attention, 02). Owning crates: `deck-daemon` (detection, state), `deck-tui` (glyphs), `deck-core` (types, config, argv matching).

## Requirements

1. **Types** in `deck-core::agents`: `AgentKind { Claude, Codex, OpenCode }`, `AgentStatus { None, Working, Waiting, Idle, Active, Quiet }` with an `urgency()` order as in the overview, `AgentState { kind, status, source: Hooks | Process, session_id: Option<String>, first_prompt: Option<String>, since }`. `Terminal` gains `agent: Option<AgentState>`; `SessionView` carries it per terminal. Protocol version bumps.
2. **Config**: `[agents.<kind>] process = [..]` with defaults `["claude"]`, `["codex"]`, `["opencode"]`, documented in `default.toml` (DECK-08).
3. **Detection tick**: a dedicated 1 s tick over all terminals, independent of DECK-04's 5 s cwd tick, running its platform queries off the async worker threads. For each terminal it reads the PTY's foreground process group leader and its argv through a new `platform::foreground_process(master) -> Option<(pid, argv)>` (`tcgetpgrp` on the master fd; argv from `/proc/<pid>/cmdline` on Linux, `sysctl KERN_PROCARGS2` on macOS).
4. **argv matching** (pure, `deck-core::agents::match_argv(argv, &config) -> Option<AgentKind>`): take the basename of `argv[0]` as typed (not the resolved path); when that basename is an interpreter (`node`, `bun`, `deno`, `sh`, `bash`, `zsh`, `dash`), take the basename of `argv[1]` instead (this also covers wrapper scripts, which the kernel runs as `<interpreter> <script>`). Remove the extension (`codex.js` → `codex`). The result is an agent of kind K when it is in K's `process` list.
5. **Process-sourced state**: when a terminal has no hook-sourced state, detection sets `AgentState { source: Process }` with `active` (PTY output in the last 2 s) or `quiet`, and clears it (`none`) when the foreground process is no longer an agent.
6. **Hook-sourced state** wins: once a terminal's state has `source: Hooks` (sub-specs 03 and 04), detection only clears it when the agent process has been gone from the foreground for 3 s. A hook-sourced `working` state with no hook event and no PTY output for 60 s falls back to the process-only state (protects against a missed `Stop`, for example after an interrupt that fires no hook).
7. **Request** `AgentEvent { terminal, kind, status: Option<AgentStatus>, session_id: Option<String>, prompt: Option<String>, raw: Option<String> }`, handled by the daemon (sub-spec 03 sends it for Claude Code and Codex; sub-spec 04 sends `raw` for OpenCode; tests send it directly): sets the state with `source: Hooks`. A `session_id` different from the stored one replaces it and resets `first_prompt`; the first non-empty `prompt` after that is kept (first line, at most 200 characters).
8. **Answered prompts**: `Input` from a client to a terminal whose hook-sourced status is `waiting` sets it to `working` (the user answered; Claude Code and Codex fire nothing between a permission request and the next tool result).
9. **Glyphs** (DECK-02 status column, terminal titles): the panel shows its most urgent terminal's glyph from the overview; spinners advance every 100 ms on the client (no redraw loop when nothing spins). Terminal titles get the glyph and kind: `⠹ claude · <title>`, on the border title in a grid and in the status-bar title of a single terminal (no border since DECK-02). The glyphs and their roles come from `[theme.glyphs]` and the theme colours (DECK-08): this sub-spec adds `spinner` (a list of frames), `waiting`, `idle` and `quiet` to `[theme.glyphs]` and `default.toml`.
10. **`deck ls --json`**: each terminal gains `"agent": null | {"kind","status","source","session_id"}`.

## Screen

```text
 main                       │╭─ ⠹ claude · Refactor layout engine ─────────────────────╮
                            ││                                                          │
 ▾ terminal-multiplexer     ││                                                          │
▌⠹ engine                   ││                                                          │
 ! review                   ││                                                          │
 ● docs                     ││                                                          │
 ○ notes                    ││                                                          │
                            │╰──────────────────────────────────────────────────────────╯
 main  terminal-multiplexer › engine                                    ⌥Space ? help
```

- `engine`: working (spinner, accent). `review`: waiting (`!`, warning, bold). `docs`: idle (`●`, ok). `notes`: an agent detected by process only, quiet (`○`, muted).

## Acceptance criteria

- [ ] `match_argv` unit tests: `["claude"]`, `["/usr/local/bin/codex"]`, `["node", "/x/opencode"]`, `["node", "/x/codex.js"]`, `["bun", "/x/opencode.ts"]`, `["/bin/sh", "/tmp/fake/claude"]`, `["bash", "./codex"]` match; `["vim"]`, `["node", "/x/server.js"]`, `["sh"]` do not; a custom `process` list is honoured.
- [ ] A fake agent (a `#!/bin/sh` script named `claude` on `PATH` in the harness that prints, sleeps, then waits on stdin) is detected: the spinner shows while it prints and `○` when quiet; exiting it clears the glyph (capture `process-detect`). No real `node` is needed in CI.
- [ ] `AgentEvent { status: Working }`, then `{ status: Waiting }` shows `!` in the sidebar and the title (capture `waiting`); typing into that terminal turns it back to `working`.
- [ ] A new `session_id` in an `AgentEvent` replaces the old one and resets `first_prompt`.
- [ ] A hook-sourced `working` terminal with no event and no output for 60 s falls back to the process-only state (daemon test with an injected clock).
- [ ] A hook-sourced state is cleared 3 s after the agent leaves the foreground, not before.
- [ ] Panel glyph follows the urgency order with mixed terminals (unit test).
- [ ] `deck ls --json` shows the `agent` object.
- [ ] The new glyph keys appear in `default.toml` and the DECK-08 sync test passes.

## Tests

- `deck-core`: urgency order, config parsing, `match_argv`.
- `crates/deck-daemon/tests/`: detection with fake `#!/bin/sh` agent scripts, `AgentEvent` handling, session-id replacement, the answered-prompt rule, the 3 s and 60 s rules (injected clock).
- `deck-tui`: drawing of glyphs in the sidebar and titles, spinner timing.
- `crates/deck/tests/agents.rs`: the end-to-end criteria.
