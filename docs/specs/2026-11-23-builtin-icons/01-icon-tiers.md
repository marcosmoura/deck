---
type: shippable
status: done
---

# 01 — One icon set, three tiers, no icon config

## Scope

deck draws its own icons and the config can no longer change them. In Ghostty, kitty and WezTerm every icon is a Nerd Font glyph (agents in Ghostty and kitty keep DECK-12's logo images); elsewhere every icon is a Unicode glyph until [02](02-terminal-probe.md) asks the terminal and reads its font. The status column, the space caret and the unseen marker take two cells, like the program icon. The five icon config keys are gone and warn when found. Owning crates: `deck-core` (the icon table, theme and config), `deck-tui` (tier detection and drawing), `deck` (e2e harness environment).

Sub-spec 02 replaces this sub-spec's first-cut tier rules with real checks; sub-spec 03 adds images for every icon on top of the `Image` tier; until then the `Image` tier draws images only for the three agents (DECK-12) and the Nerd glyph for every other icon.

## Requirements

1. **`deck_core::icons::Icon`**: an enum with one variant per `IconKind` (by wrapping it: `Icon::Kind(IconKind)`) plus `Expanded`, `Collapsed`, `DropTarget`, `Spinner`, `Waiting`, `Idle`, `Quiet`, `Unseen`, `New`, `Resume` (the last two added after the user tried the branch: the sidebar's `+` and `↻` chips). Two pure functions give its glyphs: `unicode(icon, frame) -> &'static str` and `nerd(icon, frame) -> &'static str`, where `frame` picks the spinner frame (modulo the tier's frame count) and is ignored by the other icons. Each glyph is one cell wide (`unicode-width`), checked by a unit test over every icon and frame.

   | Icon         | Unicode                  | Nerd Font                                                    |
   | ------------ | ------------------------ | ------------------------------------------------------------ |
   | `claude`     | `✻`                      | U+EC82 nf-cod-claude                                         |
   | `codex`      | `◎`                      | U+EC81 nf-cod-openai                                         |
   | `opencode`   | `◇`                      | `▣` (no Nerd Font glyph)                                     |
   | `terminal`   | `❯`                      | U+F120 nf-fa-terminal                                        |
   | `editor`     | `✎`                      | U+F040 nf-fa-pencil                                          |
   | `git`        | `±`                      | U+F126 nf-fa-code_fork                                       |
   | `monitor`    | `◔`                      | U+F0E4 nf-fa-tachometer                                      |
   | `remote`     | `⇄`                      | U+F0EC nf-fa-exchange                                        |
   | `build`      | `▲`                      | U+F013 nf-fa-cog                                             |
   | `pager`      | `¶`                      | U+F02D nf-fa-book                                            |
   | `repl`       | `λ`                      | U+F121 nf-fa-code                                            |
   | `Expanded`   | `▾`                      | U+EAB4 nf-cod-chevron_down                                   |
   | `Collapsed`  | `▸`                      | U+EAB6 nf-cod-chevron_right                                  |
   | `DropTarget` | `➤`                      | U+F0DA nf-fa-caret_right (today's `caret_glyph`)             |
   | `Spinner`    | `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏` (10 frames) | U+EE06 to U+EE0B nf-extra-progress_spinner_1 to 6 (6 frames) |
   | `Waiting`    | `!`                      | U+F06A nf-fa-circle_exclamation                              |
   | `Idle`       | `●`                      | U+F058 nf-fa-check_circle                                    |
   | `Quiet`      | `○`                      | U+F10C nf-fa-circle_o                                        |
   | `Unseen`     | `•`                      | U+F444 nf-oct-dot_fill                                       |
   | `New`        | `+`                      | U+EA60 nf-cod-add                                            |
   | `Resume`     | `↻`                      | U+EA82 nf-cod-history                                        |

   The Unicode column is today's default; the program icons' and the drop target's Nerd column is today's `nerd` preset. The new Nerd code points were checked against Nerd Fonts' `glyphnames.json` (3.4.0); the worker checks them again.

2. **Theme and config**: `Theme` loses `icons`, `icon_images`, `glyphs`, `icon_glyphs` and `icon_colors`, with `IconPreset`, `IconImages`, `Glyphs`, `IconGlyphs` and `IconColors`. `[theme] icons`, `[theme] icon_images`, `[theme.glyphs]`, `[theme.icon_glyphs]` and `[theme.icon_colors]` each give one warning with the key's path and the message `<path> was removed: deck ships its own icons` (one per key or table, not one per key inside a table), and change nothing else; a config with them loads. `default.toml` loses them and their comments (DECK-08 sync test). The old glyph-width and icon-name errors, and the `the_removed_active_glyph_warns` behaviour, go away with them.

3. **Tier, first cut** (`deck-tui`, pure function over an environment map and `truecolor`, read once at client start like DECK-12's `detect`; [02](02-terminal-probe.md) replaces it): `enum Tier { Image, Nerd, Unicode }`, stored in `App`.
   - `Image` when DECK-12's detection says images work (its rules unchanged; the `icon_images` argument goes away).
   - Else `Nerd` when any of: `TERM` is `xterm-kitty` or `xterm-ghostty`; `TERM_PROGRAM` is `ghostty` or `WezTerm`; `KITTY_WINDOW_ID`, `GHOSTTY_RESOURCES_DIR` or `WEZTERM_EXECUTABLE` is set and non-empty.
   - Else `Unicode`.
   - Config reload no longer changes how icons are drawn (nothing to reload); a reload of `truecolor` follows DECK-12's existing behaviour.

4. **Drawing one icon**: one `deck-tui` function draws any `Icon` into a two-cell slot with a style: in the `Image` tier, an agent `IconKind` is DECK-12's placeholder pair; every other icon in the `Image` and `Nerd` tiers is its Nerd glyph in the first cell; in the `Unicode` tier, its Unicode glyph. The second cell is blank (with the slot's background). Every place below goes through it, and the transmission pre-pass (DECK-12 requirement 6) uses the same choice.

5. **Places** (every one is a two-cell slot; the rest of each layout shifts by the extra cell where there was one cell before):
   - Panel rows: blank, program icon slot, space, name; at the right, the unseen slot (after the name), the terminal count and the status slot (two cells, before the one-cell margin), one space apart. A panel without an agent has no status slot and keeps no room for it. The name is cut with `…` before whatever is on its right.
   - Space rows: the caret is a slot, then the one-cell right margin; the collapsed count stays before it (`2 ▸ `).
   - The collapsed space's waiting badge stays text, `!<n>` in `warning` bold, with the Unicode `Waiting` glyph in every tier.
   - Grid border titles of agent terminals: `<status slot> <kind> · <name>` (today `<glyph> <kind> · <name>`), still from `rect.x + 2`; the label's room is `6 + 2` cells less than the width (today `6 + width(&glyph)`).
   - The drag caret (`sidebar::caret_glyph`, DECK-18) keeps its one cell and its place; it is `DropTarget`'s glyph for the tier (`Nerd` in the `Image` tier; it gets no image).
   - The New menu and the restore dialog: unchanged (already two-cell program icons), but through the same function.
   - Every `icons = "none"` branch (`theme.icons == IconPreset::None`) is removed: icons are always drawn.
6. **Colours**: the status marks keep today's styles (spinner `accent`; waiting `warning` bold; idle `ok`; quiet `muted`; unseen `accent`). Space carets keep `muted` as today (`sidebar_fg` under hover), and so does the collapsed count; the quiet mark keeps its hover rule too. Program icons take the row's style. The Claude icon is always `#d97757` (`Color::Rgb`, built in the colour conversion module like every colour; on a reversed row it goes in the cell's background as `sidebar::icon_style` does today); Codex and OpenCode take the row's colour. In 256 colours `#d97757` goes through `downsample` as today.
7. **Spinner timing**: unchanged in this sub-spec: one frame per 100 ms in every tier; the frame index is `App.spin` modulo the tier's frame count.
8. **Protocol**: `PROTOCOL_VERSION` goes from 19 to 20, since `Theme` (sent in `Welcome` and `ServerMsg::Config`) loses five fields; `architecture.md`'s protocol line records it.
9. **Test harness**: the e2e harness also removes `GHOSTTY_RESOURCES_DIR` and `WEZTERM_EXECUTABLE` from the client's environment, so the e2e tests run in the `Unicode` tier by default whatever the developer's terminal.

## Screen

Unicode tier, 28-column sidebar (the `Nerd` tier is the same with the Nerd glyphs):

```text
 main         +  New space  │
                            │
 terminal-multiplexer    ▾  │
 ✎  editor                  │
 ✻  Fix login bug •      ⠹  │
 ◎  Review the API chan… !  │
 budget                  ▾  │
   no panels                │
 dotfiles !1           2 ▸  │
                            │
  +  New         ↻  Resume  │
```

- Panel row columns: 0 blank, 1–2 icon slot, 3 space, name from 4; the status slot is columns 25–26 (the two before the right margin), when the panel has an agent. The unseen slot follows the name after one space; the count is before the status slot.
- Space row: the caret's slot is the two columns before the right margin.
- `✻` is `#d97757`, `⠹` and `•` `accent`, `!` `warning` bold, `▾` `muted`, `✎` and `◎` the row's colour. A name with nothing after it has 23 cells (28 − 4 − 1 margin), 20 with a status slot.

Grid border title of a working Claude terminal: `╭─ ⠹  claude · Fix login bug ───╮`.

## Acceptance criteria

- [ ] Every icon and every spinner frame has a Unicode and a Nerd glyph one cell wide; the table above is what the functions return.
- [ ] Tier table: `Image` for Ghostty and kitty with truecolor outside tmux; `Nerd` for kitty without truecolor, `TERM_PROGRAM=WezTerm`, `WEZTERM_EXECUTABLE` alone; `Unicode` for `TERM=xterm-256color` alone, `TERM_PROGRAM=Apple_Terminal`, `TERM_PROGRAM=iTerm.app`, and an empty environment.
- [ ] Each of the five removed keys gives exactly one warning naming it with the message above and leaves the theme at its default; `[theme.glyphs]` with three keys gives one warning; `default.toml` has none of them and stays in sync.
- [ ] `TestBackend`, Unicode tier: the panel row, space row, unseen marker and grid border title match the screen above cell for cell; Nerd tier: the same cells hold the Nerd glyphs; Image tier: a Claude row holds DECK-12's placeholder pair and a shell row the Nerd `terminal` glyph.
- [ ] The Claude icon is `#d97757` in every tier, on a normal and on a reversed row; a Codex icon on the active row is in the row's colour.
- [ ] No `IconPreset`, `IconImages`, `Glyphs`, `IconGlyphs` or `IconColors` is left in the workspace; no code path skips icons.
- [ ] End to end (Unicode tier): a fake `claude` panel row reads `✻ ` up to its name and has a spinner frame in column 25 (any frame), and a config with `icons = "nerd"` loads with the warning in the floating bar. Existing e2e tests pass with their captures updated to the new columns, when run from Ghostty, kitty or WezTerm.
- [ ] `PROTOCOL_VERSION` is 20.
- [ ] By hand, in Ghostty, kitty, WezTerm and Terminal.app: the icons look right and line up; the Claude and OpenAI Nerd glyphs render in Ghostty, kitty and WezTerm (else see the overview's risk).

## Tests

- `deck-core`: the glyph tables (width, frame counts, wrap-around), the removed-key warnings, `default.toml` sync.
- `deck-tui`: `no_colour_or_theme_glyph_literals_in_the_drawing_code` (lib.rs) gets the new Nerd code points and `➤`, so every glyph stays in `deck-core`; `sidebar_reorder_tests.rs` compares against `DropTarget`'s glyph; tier detection, the icon-slot drawing in each tier, the screens above with `TestBackend`, colours on normal and reversed rows.
- `crates/deck/tests/`: the end-to-end criteria; `theme.rs`'s `glyphs_follow_the_theme` is deleted (the feature is gone); `support/mod.rs`: the environment change; captures of tests that show the sidebar updated.

## Notes

- The status slot moved from the start of a panel row to its right end after the user tried the branch: the loader, waiting, idle and quiet marks share it, it shows only when the panel has an agent status, and the name gets the width otherwise.
- Nerd Fonts' `extra-progress_spinner_*` glyphs come from Fira Code's progress set and are in Nerd Fonts since 3.0.
- `docs/features/config.md`: the `icons`, `icon_glyphs`, `icon_colors`, `icon_images` and `[theme.glyphs]` paragraphs become one short paragraph: deck ships its own icons (images in Ghostty and kitty, Nerd Font glyphs in terminals that bundle them, Unicode elsewhere) and the old keys warn. `docs/features/agents.md` stops pointing at `[theme.glyphs]`.
