---
type: shippable
status: done
---

# 04 — Notification click brings you to the panel

## Scope

When deck's desktop notification (`<kind> needs you · <space> › <panel>`, `<kind> finished · …`) brings you back to the terminal, deck shows the panel it was about. The user tries it with `just run`: an agent finishes while another app has focus, a click on the notification (or a switch back) lands on its panel. Crate: `deck-tui` (`app.rs`).

## Requirements

- OSC 9 sends no click back, so the client uses the focus report. When it writes an alert (`notify = "osc9"` or `"bell"`) while its outer terminal is unfocused, it remembers `(panel_id, when)` from `AgentAlert.panel_id` (01); a newer alert replaces it.
- On the next focus gain (`Focus(true)` from the outer terminal), if it comes within 10 s of the alert and the panel is still in the client's last `SessionView` and is still unseen (`PanelView.unseen`) or has a `waiting` terminal, the client sends `Action::FocusProject`, `Action::FocusPanel` and `Action::FocusTerminal` for its space, the panel and the terminal the existing attention helper in `deck-tui/src/agent.rs` picks for that panel (a waiting terminal first, then an idle one), and forgets the alert. Otherwise it only forgets it.
- A focus gain later than 10 s, or after the user already went to that panel, does nothing. A client that never reported focus never jumps (it never counted as unfocused).
- `notify = "none"` writes no alert and never jumps.
- Only the client that wrote the alert jumps.

## Acceptance criteria

- [ ] Unfocused alert, focus gain at 3 s: the panel and its terminal are focused.
- [ ] Focus gain at 11 s, a panel already seen, a closed panel, `notify = "none"`, a never-reported focus: no jump.
- [ ] Two alerts: the newer one wins.
- [ ] End to end: an agent alert while the harness reports focus out, then focus in, shows the panel (screen capture).
- [ ] `docs/features/agents.md` (Attention) describes the jump and its limit: switching back to the terminal by hand within 10 s jumps too.

## Tests

- `deck-tui`: app tests with a fake clock for each criterion.
- `crates/deck/tests/`: one scenario with focus reports (`ESC [ O`, `ESC [ I`).

## Notes

- Checked on 26 Nov 2026, from each terminal's source on its default branch: none of the three reports a click on an OSC 99 notification, so the focus return stays the only way. Ghostty (`ghostty-org/ghostty`): the OSC parser has a `kitty_desktop_notification` command (`src/terminal/osc/parsers/kitty_desktop_notification.zig`, which reads the `a=focus,report` options), but the stream handlers only act on OSC 9 and 777 (`show_desktop_notification`), and the feature issue "OSC 99 style desktop notifications" (#5634) is still open. iTerm2 (`gnachman/iTerm2`): the OSC table in `sources/VT100/VT100XtermParser.m` maps 9 to `ITERM_USER_NOTIFICATION` and has no entry for 99. WezTerm (`wezterm/wezterm`): `wezterm-escape-parser/src/osc.rs` knows OSC 9 (`SystemNotification`) and 777, not 99; its changelog does not mention it. Read from source only; no terminal was run.
- A precise click needs a notification protocol that reports activation. kitty's OSC 99 does (`a=focus,report` with an id; the terminal sends back `OSC 99 ; i=<id> ; ST`). Using OSC 99 is a separate task.
- In a terminal with several windows or tabs, clicking the notification makes the terminal bring forward a window of its own choosing. deck cannot pick it.
