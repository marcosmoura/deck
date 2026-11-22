---
type: shippable
status: draft
---

# 01 — Detection, sidebar status and attention

## Scope

When the user runs `claude`, `codex` or `opencode` in a terminal, the sidebar shows the agent and whether it is printing. Once hooks report precise states (sub-spec 02), deck marks panels that finished or need the user while the user was elsewhere, notifies, and jumps to them. This sub-spec builds the whole state model, so sub-spec 02's hooks only feed it. Owning crates: `deck-daemon` (detection, state), `deck-tui` (drawing, notifications, jump), `deck-core` (types, config).

## Requirements

1. **Types** in `deck-core`: `AgentKind { Claude, Codex, OpenCode }`, `AgentStatus { None, Working, Waiting, Idle, Active, Quiet }` with an `urgency()` order as in the overview, `AgentState { kind, status, source: Hooks | Process, session_id: Option<String>, first_prompt: Option<String>, since }`. `Terminal` gains `agent: Option<AgentState>`; `SessionView` carries it per terminal plus a per-panel `unseen: bool`. Protocol version bumps.
2. **Config**: `notify = "osc9" | "bell" | "none"` (default `osc9`); `[agents.<kind>] process = [..]` with defaults `claude`, `codex`, `opencode`.
3. **Process detection**: every 1 s, on DECK-04 sub-spec 02's polling tick (off the async worker threads), the daemon reads the PTY's foreground process group leader and its argv through a new `platform::foreground_process(master) -> Option<(pid, argv)>` (`tcgetpgrp` on the master fd; argv from `/proc/<pid>/cmdline` on Linux, `sysctl KERN_PROCARGS2` on macOS). It is an agent of kind K when the basename of `argv[0]` as typed (not the resolved path), or of `argv[1]` when that basename is `node`, `bun` or `deno`, with its extension removed (`codex.js` → `codex`), is in K's `process` list. When a terminal has no hook-sourced state, detection sets `AgentState { source: Process }` with `active` (output in the last 2 s) or `quiet`, and clears it (`none`) when the foreground process is no longer an agent.
4. **Hook-sourced state** wins: once a terminal's state has `source: Hooks` (sub-spec 02), process detection only clears it when the agent process is gone from the foreground for 3 s. A hook-sourced `working` state with no hook event and no PTY output for 60 s falls back to the process-only state (protects against a missed `Stop`, for example after an interrupt that fires no hook).
5. **Request** `AgentEvent { terminal, kind, status: Option<AgentStatus>, session_id: Option<String>, prompt: Option<String> }`, handled by the daemon (sub-spec 02 sends it; tests send it directly): sets the state with `source: Hooks`. A `session_id` different from the stored one replaces it and resets `first_prompt`; the first non-empty `prompt` after that is kept (first line, at most 200 characters).
6. **Attention** (hook-sourced states only; process-only changes never set `unseen` and never notify, since every pause in output would flap): when a terminal's status goes from `working` to `waiting` or `idle`, and its panel is not the active panel of a focused client, the panel's `unseen` becomes true and the daemon sends `ServerMsg::Notify { title, body }` to one client of the session: the most recently active one (the last to send input or a viewport). `unseen` is cleared when the panel becomes active in a focused client.
7. **Client focus**: the client reports the outer terminal's focus with a new `ClientMsg::Focus(bool)`, from the focus events DECK-01 sub-spec 05 enables, and re-reports on every attach. A client that never reported counts as focused (outer terminals without focus reporting never notify for the active panel).
8. **Notifications** (client): `osc9` writes `ESC ] 9 ; <body> BEL` to the outer terminal; `bell` writes `BEL`; `none` nothing. Body: `<kind> needs you · <project> › <panel>` for `waiting`, `<kind> finished · <project> › <panel>` for `idle`. Control characters are stripped from names before they are written.
9. **Sidebar** (DECK-02 status column): the panel's most urgent glyph from the overview; spinners advance every 100 ms on the client (no redraw loop when nothing spins). Unseen panels are bold with an accent dot after the name. A collapsed project shows `!<n>` (warning) when n of its panels are waiting. Terminal titles get the glyph and kind: `⠹ claude · <title>`.
10. **Status bar**: right side shows `<n> waiting` (warning) when any terminal of the session is waiting.
11. **Jump**: binding `next_attention = "a"`: focus (panel and terminal) the next waiting terminal in sidebar order after the current one, wrapping; if none, the next unseen idle one; if none, flash `no agent needs you`.
12. **`deck ls --json`**: each terminal gains `"agent": null | {"kind","status","source","session_id"}`.

## Screen

```text
 main                       │╭─ ⠹ claude · Refactor layout engine ─────────────────────╮
                            ││                                                          │
 ▾ terminal-multiplexer     ││                                                          │
▌⠹ engine                   ││                                                          │
 ! review •                 ││                                                          │
 ● docs                     ││                                                          │
 ▸ budget !1              2 ││                                                          │
                            │╰──────────────────────────────────────────────────────────╯
 main  terminal-multiplexer › engine                       2 waiting  ^Space a next  ? help
```

- `engine`: agent working (spinner, accent). `review`: waiting (`!` warning) and unseen (bold, `•`). `docs`: idle (`●` ok). `budget` is collapsed with one waiting panel.

## Acceptance criteria

- [ ] A fake agent (a script named `claude` on `PATH` in the harness that prints, sleeps, then waits on stdin) shows the spinner while printing and `○` when quiet, without setting `unseen`; exiting it clears the glyph (capture `process-detect`).
- [ ] Scripts run as `node /x/opencode` and `node /x/codex.js` are detected as OpenCode and Codex.
- [ ] `AgentEvent { status: Working }` then `{ status: Waiting }` for a terminal in an inactive panel marks it unseen, shows `!`, shows `1 waiting`, and the client writes `ESC]9;claude needs you · <project> › <panel>BEL` to the outer terminal (checked with the harness's `raw_output()`; capture `attention`).
- [ ] With `notify = "bell"` the client writes `BEL` instead; with `none` nothing.
- [ ] Focusing the panel clears `unseen`.
- [ ] No notification is sent for a terminal in the active panel of a focused client; one is sent after that client reported `Focus(false)`.
- [ ] With two attached clients, a transition produces exactly one `Notify`, sent to the client that sent input last.
- [ ] A new `session_id` in an `AgentEvent` replaces the old one and resets `first_prompt`.
- [ ] A hook-sourced `working` terminal with no event and no output for 60 s falls back to the process-only state (daemon test with an injected clock).
- [ ] `^Space a` cycles through waiting terminals, then unseen idle ones, then flashes `no agent needs you`.
- [ ] A collapsed project shows `!1` for one waiting panel.
- [ ] `deck ls --json` shows the `agent` object.
- [ ] Panel glyph follows the urgency order with mixed terminals (unit test).

## Tests

- `deck-core`: urgency order, config parsing, argv matching (including `node` wrappers and extensions).
- `crates/deck-daemon/tests/`: detection with fake agent scripts, `AgentEvent` handling, session-id replacement, unseen and freshness rules (injected clock), `Notify` delivery to one client.
- `deck-tui`: drawing of glyphs, unseen rows, collapsed badges, status-bar count; notification encoding; focus re-report on attach.
- `crates/deck/tests/agents.rs`: one scenario per end-to-end criterion.
