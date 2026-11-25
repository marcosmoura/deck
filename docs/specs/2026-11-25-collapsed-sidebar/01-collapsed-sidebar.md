---
type: shippable
status: done
---

# 01 — Collapsed sidebar

## Scope

The user can click a button before the session name to collapse the sidebar to 8 columns, see every terminal's status and number there, click an entry to focus that terminal, and click the button at the top to expand it again. A `collapse_sidebar` binding and a `sidebar_collapsed` config key do the same from the keyboard and at start. Owning crates: `deck-core` (config key, binding, layout constant, icons, protocol version), `deck-tui` (state, drawing, input) and `deck-daemon` (the attach viewport). End-to-end tests are in `crates/deck/tests/`. Every decision is in the [overview](00-overview.md#decisions).

## Requirements

- **deck-core.**
  - `config.rs`: `Config.sidebar_collapsed: bool` (default `false`), parsed from the top-level `sidebar_collapsed` key and listed in `TOP_LEVEL_KEYS`. A `collapse_sidebar` entry in `DEFAULT_BINDINGS` with no keys, in `HelpGroup::Sidebar`, described `Collapse or expand the sidebar`. Both in `config/default.toml` with comments.
  - `layout.rs`: `COLLAPSED_SIDEBAR_WIDTH: u16 = 8`. The client passes the width in force (full or collapsed) to `sidebar_cols` and `panel_area`; their rule is unchanged.
  - `icons.rs`: `Icon::CollapseSidebar` and `Icon::ExpandSidebar` with the glyphs of the overview, added to `Icon::ALL`.
  - `protocol.rs`: `PROTOCOL_VERSION` up by one, with the history line in `docs/architecture.md`.
- **deck-daemon.** Where the attach viewport is derived from `config.sidebar_width` and `config.sidebar` (`server.rs`), use `COLLAPSED_SIDEBAR_WIDTH` when `config.sidebar_collapsed` is set. The client's `check_viewport` keeps mirroring exactly what the daemon derived (from the config, not from the new width helper).
- **deck-tui state.**
  - `App.sidebar_collapsed: bool`, from `config.sidebar_collapsed` at start and on a reload that changes it (which sets `sized` in `apply_config`, so it sends `ReloadViewport`). One helper gives the width in force (`COLLAPSED_SIDEBAR_WIDTH` or `sidebar_width`), and `sidebar_cols`, `viewport`, `hover_at`, drawing and every other place that now reads `sidebar_width` for the drawn width use it (not `check_viewport`, see above). `sidebar_width` itself stays the full width, and a separator drag never changes it while collapsed.
  - `Action::CollapseSidebar` (`"collapse_sidebar"` in `input.rs`): flips the state, shows a hidden sidebar, flashes `sidebar hidden (window too narrow)` when it still cannot show, refuses to expand in a 29 to 40 column window (flash `window too narrow to expand`), maps the selection as the overview says, runs `ensure_visible`, `refresh_hover` and `check_row_gesture` (as `apply_config` does), and sends `viewport_msg` unless the window is too small.
  - A click on either header chip does the same as the action.
- **deck-tui rows.** A `collapsed_rows(view)` next to `rows_with` returns `Row::Terminal` for every terminal of every space (folds ignored, single-terminal panels included) and a new `Row::Rule` between two spaces that have entries. `Row::Rule::sel()` is `None`. While collapsed, `App::sidebar_rows()` and `preview_rows()` return these rows, so every caller follows (`selectable`, `ensure_visible`, `clamp_offset`, `check_row_gesture`, `row_drag_to`, the wheel, `hover_at`, drawing); a row drag never starts. `default_selection` returns the focused terminal of the active panel while collapsed, and `normalize_selection` falls back as the overview says (never to a panel or space row, which is not drawn).
- **deck-tui drawing** (`sidebar::draw`), collapsed:
  - The header with only the `ExpandSidebar` chip; the body with entries and rules; the bottom bar with only the `+` chip.
  - An entry's mark: the status mark from `terminal_status` when it has one, else `terminal_icon_kind`, through `icon_slot` (images where on). The number from column 4, from the entry's index among `Row::Terminal` rows plus one.
  - Band, accent, bold, muted, hover and dimming as the overview says.
- **deck-tui drawing**, expanded: the `CollapseSidebar` chip at columns 0 to 3 of the header, the session name from column 4, cut before the `New space` chip.
- **Hover.** `Hover` gains `CollapseSidebar` and `ExpandSidebar` (the header chips). Collapsed, the bottom bar hovers only `New`.
- **Mouse.** The chip-release dispatch in `app.rs` ends in a catch-all that opens the New space prompt: give `CollapseSidebar` and `ExpandSidebar` their own arms. A click on an entry runs the existing terminal row path (`click_row` with `Sel::Terminal`). A right press opens the terminal menu for it. A press on the separator does nothing while collapsed. Chip clicks need a press and a release on the same chip and are ignored while a prompt, a confirmation or an overlay is open.
- **Sidebar focus.** Keys on an entry follow the terminal row rules; `Left`/`h` and `Right`/`l` do nothing while collapsed. The start selection is the focused terminal of the active panel. A collapse or expand during sidebar focus maps the selection as the overview says.
- **Icons.** The two SVGs and masks in `crates/deck-tui/assets/icons/`, made with the README's commands, and their `include_bytes!` in `graphics::mask`. Update the README's sources table and loop.
- **Docs.** As the overview's Affected docs list.

## Screen

Expanded, 28 columns. The collapse chip is at columns 0 to 3 and the session name moves to column 4:

```text
 «  main      +  New space  │
                            │
 terminal-multiplexer    ▾  │
 ✻  claude               ⠹  │
 ❯  api + tests          !  │
  ├ ❯  zsh                  │
  └ ✻  Write tests       !  │
 dotfiles              1 ▸  │
                            │
  +  New         ↻  Resume  │
```

The same session collapsed: 8 columns and the separator. `dotfiles` is folded but its terminal still shows. The focused terminal of the active panel (`Write tests`, number 3) has the band:

```text
   »    │   ← ExpandSidebar chip, muted; hover background under the pointer
        │
 ⠹  1   │   ← claude: spinner in its status colour
 ❯  2   │   ← zsh: no agent, program icon
 !  3   │   ← band (focused terminal of the active panel)
 ────── │   ← muted rule between two spaces
 ✎  4   │   ← dotfiles' only terminal: editor icon
        │
   +    │   ← New chip; opens the New menu
```

A kept terminal that exited (number 5) has its mark and number muted; three-digit numbers fit before the margin:

```text
 ❯  5   │   ← muted
 ●  123 │
```

Sidebar focus, collapsed, selection on entry 2. The band moves to it, and the focused terminal of the active panel is accent, bold:

```text
 ⠹  1   │
 ❯  2   │   ← band (selected)
 !  3   │   ← accent, bold
```

A 20-column expanded sidebar: the `New space` chip shows only its icon, and the name gets columns 4 to 13:

```text
 «  terminal-…  +   │
```

## Acceptance criteria

- [ ] The expanded header shows the collapse chip before the session name; the name starts at column 4 and is cut before the `New space` chip at every width from 20 to 60.
- [ ] Clicking the collapse chip shrinks the sidebar to 8 columns plus the separator, and the panel area widens by the difference (the terminals are resized). Clicking the expand chip returns to the previous width.
- [ ] Collapsed, every terminal of every space has one entry in sidebar order, folded spaces included, numbered from 1, with a rule between spaces and nothing for an empty space.
- [ ] An entry shows its agent's status mark, or its program icon when it has no agent status, in images where images are on; an exited kept terminal is muted; an unseen panel's entries are bold.
- [ ] The focused terminal of the active panel has the band outside sidebar focus; in sidebar focus the band follows the selection.
- [ ] A click on an entry makes its panel and space active and focuses that terminal; a right click opens the terminal menu; entries hover; a press never drags; a press on the separator does nothing while collapsed.
- [ ] Collapsed, the bottom bar has only the `+` chip, which opens the New menu.
- [ ] In sidebar focus while collapsed, `Up`/`Down` move over entries, and `Enter`, `x`, `r`, `c`, `n` act as on a terminal row. Collapsing or expanding in sidebar focus keeps the selection on the same terminal (or its panel's row).
- [ ] `collapse_sidebar` bound in `[keys]` toggles the state and shows a hidden sidebar; unbound by default, listed as `unbound` in the help overlay.
- [ ] `sidebar_collapsed = true` starts collapsed; a reload that changes it applies to attached clients; a reload that does not keep the user's state.
- [ ] With the window under 29 columns the collapsed sidebar hides, and shows again when the window grows. In a 29 to 40 column window the expand chip leaves it collapsed and flashes `window too narrow to expand`.
- [ ] A client attached with `sidebar_collapsed = true` sends no corrective viewport after attach (the daemon derived the collapsed width).
- [ ] `PROTOCOL_VERSION` is up by one; the docs in the overview's list are updated.
- [ ] `just check` passes.

## Tests

- `deck-daemon` unit test: the attach viewport with `sidebar_collapsed` set.
- `deck-core` unit tests: `sidebar_collapsed` parsing and default; `collapse_sidebar` present in the defaults with no keys; `sidebar_cols` and `panel_area` with the collapsed width at 28, 29 and 80 columns; the two icons' glyphs.
- `deck-tui` unit tests (`sidebar_tests.rs` or a new `sidebar_collapsed_tests.rs`):
  - `collapsed_rows` with folded spaces, an empty space, single and grid panels: order, rules, numbering.
  - Drawing: the expanded header with the chip at widths 20, 26 and 28; the collapsed header, entries (status mark, program icon, muted exited, bold unseen, three-digit number), band and accent in and out of sidebar focus, the bottom bar.
  - The action and chip clicks: viewport sent with the right width, hidden sidebar shown, flash when too narrow, expand refused at 35 columns, chips ignored under an overlay, neither chip opening the New space prompt.
  - Hover and click on an entry (both actions sent), right press opening the terminal menu, separator press ignored while collapsed, wheel scrolling and `ensure_visible`.
  - Sidebar focus: moving over entries, keys on an entry, the selection mapped across a collapse (from a terminal, panel, space and empty space row) and an expand (to a terminal row, a single-terminal panel row, a folded space row); `normalize_selection` while collapsed after the selected terminal closes; a click on a single-terminal panel's entry surviving the next snapshot.
  - Config: start collapsed, reload changing `sidebar_collapsed`, reload not changing it.
- End-to-end, in `crates/deck/tests/sidebar_mouse.rs` (or a new `sidebar_collapsed.rs`):
  - Click the collapse chip and check the screen shows the collapsed entries and a wider panel area; click the expand chip and check the full sidebar returns.
  - With two panels, collapse, click the first panel's entry and check it becomes active.
  - Start with `sidebar_collapsed = true` and a bound `collapse_sidebar` key; check it starts collapsed and the key expands it.

## Notes

Changed after trying it (25 Nov 2026): the collapsed strip's expand and `+` chips moved from columns 0 to 3 (icon at 1 and 2) to columns 1 to 4 (icon at 2 and 3), so both buttons are centred in the strip.

Changed again the same day: a 4-cell chip cannot be centred in 7 columns (one column on one side, two on the other), so the strip is now 8 columns (`COLLAPSED_SIDEBAR_WIDTH`) and both chips take columns 2 to 5 (icon at 3 and 4), two columns on each side. Entries keep their columns; the rule runs to column 6; the strip needs a 29-column window.
