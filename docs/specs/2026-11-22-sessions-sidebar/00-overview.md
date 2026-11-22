---
branch: feat/sessions-sidebar
backlog: DECK-02
---

# Sessions, projects, panels and the sidebar

## Goal

Turn the single-terminal client into the product's shape: a session holds projects, a project holds panels, and the sidebar shows the attached session's projects with their panels nested under them. The user creates, renames, closes and switches between projects and panels from the keyboard or the mouse, and switches between sessions with a picker or the CLI. Each panel still shows one terminal (the grid is DECK-03).

## Builds on

- [DECK-01 core-daemon-terminal](../2026-11-22-core-daemon-terminal/00-overview.md): the model types, protocol, daemon, client, prefix bindings and e2e harness.
- [architecture.md](../../architecture.md): model and lifecycle rules, client look.

## Non-goals

- Several terminals in a panel and the grid (DECK-03).
- Reordering projects or panels by drag or key. New ones are appended.
- Moving a panel between projects or sessions.
- Agent status in the sidebar (DECK-06); this spec reserves its column.

## Decisions

- **Lifecycle** follows [architecture.md](../../architecture.md#model) from here on, replacing DECK-01's temporary rule: a session no longer ends when its last terminal exits. When a panel's terminal exits, the panel closes; an empty project stays; a session with no projects shows an empty state.
- **New panel**: one shell in the project's root dir; default name is the lowest unused number in that project (`1`, `2`, …).
- **New project**: the directory is asked first (prefilled with the cwd the client was started in; `~` and paths relative to that cwd are resolved on the client; it must exist), then the name, prefilled with the directory's basename. Its first panel is created with it.
- **Sidebar**: left, 28 columns by default (`sidebar_width`, 20–60), shown by default (`sidebar = true`), toggled with `prefix b`, and hidden automatically whenever the outer terminal is narrower than `sidebar_width + 1 + 20` columns, so the panel area never drops below 20 columns. Collapsing a project hides its panels in the sidebar only.
- **Sidebar focus**: `prefix Tab` moves keyboard focus into the sidebar for navigation; `Esc` or `Tab` returns it to the terminal. Mouse clicks work without it.
- **Prompts and confirmations** appear in the status bar, one line, `Esc` cancels. Closing a project or a panel with running processes always asks.
- **Help**: `prefix ?` shows every binding in an overlay, generated from the binding table.
- **Sessions**: `prefix s` opens a picker overlay; switching re-attaches the same client process to another session without leaving the outer terminal. When the attached session is killed (from the picker, `prefix s`, or `deck kill-session` in another shell), the client switches to the most recently attached other session the same way, and exits only when none is left.
- **Several clients** on one session share the session-wide state: active project, active panel and collapsed projects. Sidebar selection, prompts, confirmations and overlays are local to each client. A prompt or confirmation whose target is removed by another client is dropped with a flash (`panel "<name>" was closed`).
- **Input modes** are handled in one order, first match wins: (1) an overlay (help, picker) or a prompt or confirmation; (2) scroll mode (DECK-01); (3) sidebar focus; (4) resize repeat (DECK-03); (5) the prefix and its bindings; (6) the terminal. Inside prompts and the picker filter the prefix key is ordinary text; in sidebar focus the prefix still works (`^Space d` detaches). Pasted text goes into an open prompt or filter as text; focus events never reach overlays.

## Sub-specs

| Sub-spec                        | Type       | Crate                   | Summary                                                                 | Depends     | Status |
| ------------------------------- | ---------- | ----------------------- | ----------------------------------------------------------------------- | ----------- | ------ |
| [01](01-model-actions.md)       | foundation | deck-core, deck-daemon, deck (tests) | Actions for projects, panels and sessions; lifecycle; frames follow the active panel | DECK-01 | draft |
| [02](02-sidebar.md)             | shippable  | deck-tui, deck-core     | Sidebar, panel and project bindings, prompts, confirmations, help        | 01          | draft  |
| [03](03-session-picker.md)      | shippable  | deck-tui, deck          | Session picker, session CLI (`new`, `kill-session`, `rename-session`)    | 01          | draft  |

02 and 03 both change `deck-tui`, so they run one after the other.

## Affected docs

- `docs/features/sessions.md`: sessions, the picker, the CLI.
- `docs/features/sidebar.md` (new): projects, panels, the sidebar and its keys.
- `docs/features/keys.md`: the new bindings.
- `docs/architecture.md#model`: confirm the lifecycle text matches (it already does, including the control API's `keep` exception from DECK-05, which this spec does not implement).
- DECK-01 criteria superseded by the lifecycle change, updated by sub-spec 01's worker in the same change: the daemon test where `exit` in the last terminal ends the session and stops the daemon ([03-daemon](../2026-11-22-core-daemon-terminal/03-daemon.md), requirement on exit), and the attach criterion where `exit` prints `[session main ended]` ([04-attach-client](../2026-11-22-core-daemon-terminal/04-attach-client.md)). Their spec text is updated too, with a note pointing here.

Reviewed by spec-reviewer on 22 Nov 2026.
