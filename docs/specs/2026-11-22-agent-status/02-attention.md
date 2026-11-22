---
type: shippable
status: todo
---

# 02 — Attention, notifications and jump

## Scope

When an agent the user is not looking at needs them or finishes, its panel is marked unseen in the sidebar, the status bar counts the waiting agents, a desktop notification goes out through the outer terminal, and `prefix a` jumps to the next agent that needs the user. Owning crates: `deck-daemon` (unseen, alerts), `deck-tui` (drawing, notifications, jump), `deck-core` (protocol, config, bindings).

## Requirements

1. **Model and protocol** (version bump): `SessionView` carries a per-panel `unseen: bool`; new `ClientMsg::Focus(bool)` and `ServerMsg::AgentAlert { kind, status, project, panel }` (named apart from DECK-08's `Notice`).
2. **Config**: `notify = "osc9" | "bell" | "none"` (default `osc9`) and the binding `next_attention = "a"`, both documented in `default.toml` (DECK-08). `[theme.glyphs]` gains `unseen = "•"`.
3. **Client focus**: the client reports the outer terminal's focus with `ClientMsg::Focus(bool)`, from the focus events DECK-01 sub-spec 06 enables, and re-reports on every attach. A client that never reported counts as focused (outer terminals without focus reporting never notify for the active panel).
4. **Unseen** (hook-sourced states only; process-only changes never set `unseen` and never alert, since every pause in output would flap): when a terminal's status goes from `working` to `waiting` or `idle`, and its panel is not the active panel of a focused client, the panel's `unseen` becomes true and the daemon sends one `AgentAlert` to the session's most recently active client (the last to send input or a viewport). `unseen` is cleared when the panel becomes active in a focused client.
5. **Notifications** (client, on `AgentAlert`): `osc9` writes `ESC ] 9 ; <body> BEL` to the outer terminal; `bell` writes `BEL`; `none` nothing. Body: `<kind> needs you · <project> › <panel>` for `waiting`, `<kind> finished · <project> › <panel>` for `idle`. Control characters are stripped from names before they are written.
6. **Sidebar**: unseen panels are bold with the `unseen` glyph (accent) after the name. A collapsed project shows `!<n>` (warning) when n of its panels are waiting.
7. **Status bar**: the right side shows `<n> waiting` (warning) when any terminal of the session is waiting.
8. **Jump** (`next_attention`): focus (panel and terminal) the next waiting terminal in sidebar order after the current one, wrapping; if none, the next unseen idle one; if none, flash `no agent needs you`.

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
 main  terminal-multiplexer › engine                       2 waiting  ⌥Space a next  ? help
```

- `review`: waiting and unseen (bold, `•`). `budget` is collapsed with one waiting panel.

## Acceptance criteria

- [ ] `AgentEvent { status: Working }` then `{ status: Waiting }` for a terminal in an inactive panel marks it unseen, shows `1 waiting`, and the client writes `ESC]9;claude needs you · <project> › <panel>BEL` to the outer terminal (checked with the harness's `raw_output()`; capture `attention`).
- [ ] With `notify = "bell"` the client writes `BEL` instead; with `none` nothing.
- [ ] Focusing the panel clears `unseen`.
- [ ] No alert is sent for a terminal in the active panel of a focused client; one is sent after that client reported `Focus(false)`.
- [ ] With two attached clients, a transition produces exactly one `AgentAlert`, sent to the client that sent input last.
- [ ] Process-only `active` → `quiet` never sets `unseen` and never alerts.
- [ ] `⌥Space a` cycles through waiting terminals, then unseen idle ones, then flashes `no agent needs you`.
- [ ] A collapsed project shows `!1` for one waiting panel.
- [ ] Names containing control characters reach the notification without them.

## Tests

- `deck-core`: protocol round trips, config and binding defaults.
- `crates/deck-daemon/tests/`: unseen rules, focus handling, one alert per transition to the right client.
- `deck-tui`: unseen rows, collapsed badges, status-bar count, notification encoding, focus re-report on attach, jump order.
- `crates/deck/tests/attention.rs`: the end-to-end criteria.

## Notes

- Limitations (documented in `docs/features/agents.md`): a session with no attached client alerts nobody (its panels are still marked unseen for the next attach), and the session picker shows no agent status.
