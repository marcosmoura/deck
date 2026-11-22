---
type: shippable
status: done
---

# 01 — Theme

## Scope

The user sets colours, the border style and the sidebar glyphs in `[theme]`, and every part of deck's own UI follows them: borders, titles, sidebar, status bar, badges, prompts, overlays. Terminal contents keep the colours their programs print. Owning crates: `deck-core` (parsing, `Theme`), `deck-tui` (drawing).

## Requirements

1. **Colour syntax** (`deck-core::theme::ThemeColor`, the parsed form, which may name a role; the resolved `Theme` holds `deck-core::protocol::Color` from DECK-01, so there is one colour type for drawing), parsed from a string or an integer: `"default"` (the terminal's own foreground or background), the 16 ANSI names `black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `white` and `bright-black` … `bright-white`, an integer `0`–`255` (the 256-colour palette), `"#rrggbb"` (case-insensitive), or the name of another role (`border_focused = "accent"`). References are resolved after parsing; a reference to an unknown role or a cycle is an error naming the keys involved.
2. **Roles** and defaults:

   | Role             | Default         | Used for                                                       |
   | ---------------- | --------------- | -------------------------------------------------------------- |
   | `accent`         | `blue`          | focused border and title, active panel row, session name       |
   | `muted`          | `bright-black`  | hints, project names and the terminal title in the status bar, empty states, exited terminals |
   | `ok`             | `green`         | success states (DECK-06 idle)                                  |
   | `warning`        | `yellow`        | `PREFIX`, `SCROLL`, `RESIZE` badges, scroll indicator, waiting |
   | `error`          | `red`           | error flashes                                                  |
   | `border`         | `bright-black`  | unfocused terminal borders and the sidebar separator           |
   | `border_focused` | `accent`        | the focused terminal's border                                  |
   | `title`          | `default`       | unfocused terminal titles                                      |
   | `sidebar_fg`     | `default`       | sidebar text                                                   |
   | `sidebar_bg`     | `default`       | sidebar background                                             |
   | `status_fg`      | `default`       | status bar text                                                |
   | `status_bg`      | `default`       | status bar background                                          |
   | `selection_fg`   | `default`       | the sidebar selection and picker selection text (drawn reversed while both selection roles are `default`) |
   | `selection_bg`   | `default`       | the selection background                                       |
   | `overlay_bg`     | `default`       | help and picker overlays                                       |

3. **Styles**: `border_style = "rounded" | "plain" | "thick" | "double"` (default `rounded`) for terminal borders and overlays. `[theme.glyphs]`: `expanded = "▾"`, `collapsed = "▸"`, `active = "▌"` (DECK-06 adds its status and unseen glyphs here). A glyph must be exactly one cell wide (measured with `unicode-width`, approved for `deck-core` by this spec), because the sidebar's gutter and marker columns are one cell; anything else is an error naming the key.
4. **`Theme`** in `deck-core` holds the resolved roles, styles and glyphs; `Theme::default()` equals the tables above. `deck-tui` draws every non-terminal element from the `Theme` it was given; no colour or theme glyph literal remains in `deck-tui` outside the defaults. The module that converts `protocol::Color` to ratatui colours is allowed to construct colours. Allowed glyph literals: `…` (truncation), `›` (status-bar path), the box-drawing characters of the chosen `border_style`, and the separator `│`.
5. **Colour depth**: when the outer terminal does not report truecolor (`COLORTERM` is neither `truecolor` nor `24bit`), `#rrggbb` roles are drawn as the nearest 256-palette colour, and so are RGB colours in terminal contents (programs inside deck always see `COLORTERM=truecolor`, so the client downsamples them the same way). Indexed and default colours in terminal contents are passed through unchanged.
6. **Until sub-spec 03**, the client parses `[theme]` from the config it already reads at attach (DECK-01); sub-spec 03 moves reading to the daemon.

## Screen

The DECK-02 sidebar mockup with this config:

```toml
[theme]
accent = "#cba6f7"
border_style = "plain"
sidebar_bg = 236
```

```text
 main                       │$ nvim
                            │
 ▾ terminal-multiplexer     │
▌  editor                   │
   claude                   │
 main  terminal-multiplexer › editor · nvim              ⌥Space ? help  d detach
```

With the help overlay open (`⌥Space ?`), its corners are plain: `┌ Help ─…─┐`.

- The sidebar's background is palette colour 236; the active row and the session name are `#cba6f7` (or its nearest palette colour without truecolor); overlay corners are plain. A single-terminal panel has no border since DECK-02; terminal borders return with DECK-03 for panels with several terminals and use the same roles and `border_style`.

## Acceptance criteria

- [ ] Every colour form parses: `"default"`, all 16 names, `0`, `255`, `"#CBA6F7"`, and a role reference; `256`, `"#ggg"`, `"purple"`, an unknown role and a cycle (`accent = "muted"`, `muted = "accent"`) each give an error naming the key.
- [ ] `Theme::default()` matches the role table, the styles and the glyphs.
- [ ] With the Screen config, the harness sees the active panel row's cells in `#cba6f7`, the sidebar cells with background 236 and the help overlay's plain corners (capture `themed`), checked through `cell(row, col)`.
- [ ] With `COLORTERM` unset in the harness, the same active row is drawn with a 256-palette colour, and `printf '\033[38;2;203;166;247mx'` inside deck reaches the outer terminal as a 256-palette colour too. The harness sets `COLORTERM=truecolor` explicitly in the other theme tests, so neither depends on the CI environment.
- [ ] `border_style = "double"` draws `╔═╗` borders on the help overlay and the session picker (terminal borders follow it once DECK-03 draws them).
- [ ] `[theme.glyphs] collapsed = ">"` changes the collapsed marker; a two-cell glyph (`"日"`), an empty string and `"ab"` are errors.
- [ ] A unit test in `deck-tui` scans its own source files and finds no `Color::` constructor outside the theme module and the colour-conversion module, and no theme glyph (`▾`, `▸`, `▌`) outside the theme module.

## Tests

- `deck-core`: colour parsing table, reference resolution and cycles, glyph width, defaults.
- `deck-tui`: `TestBackend` drawings with the default theme and with a custom one (each role visible at least once), truecolor fallback.
- `crates/deck/tests/theme.rs`: the end-to-end criteria.
