---
type: shippable
status: done
---

# 02 — Program and agent icons

## Scope

Every panel row in the sidebar, and the panel in the status bar, starts with an icon for what runs in it: the agent's own icon for Claude Code, Codex and OpenCode, a terminal icon for shells and unknown programs, and a fitting icon for a fixed set of known programs. Owning crates: `deck-core` (icon table, config), `deck-tui` (drawing).

## Requirements

1. **Icon kinds** (`deck-core::icons::IconKind`) and the pure choice `icon_for(agent: Option<AgentKind>, program: Option<&str>) -> IconKind`. An agent picks its own kind; otherwise the program (compared case-insensitively) picks from this table; anything else, or no program, is `terminal`:

   | Kind       | Programs                                                                            | `unicode` glyph | `nerd` glyph (Nerd Fonts name, codepoint) |
   | ---------- | ----------------------------------------------------------------------------------- | --------------- | ----------------------------------------- |
   | `claude`   | Claude Code agent                                                                   | `✻`             | `nf-fa-asterisk`, U+F069                  |
   | `codex`    | Codex agent                                                                         | `◎`             | `nf-fa-circle_o_notch`, U+F1CE            |
   | `opencode` | OpenCode agent                                                                      | `◇`             | `nf-fa-cube`, U+F1B2                      |
   | `terminal` | `sh`, `bash`, `zsh`, `fish`, `dash`, `nu`, `ksh`, `tcsh`, and every unknown program | `❯`             | `nf-fa-terminal`, U+F120                  |
   | `editor`   | `vi`, `vim`, `nvim`, `hx`, `helix`, `nano`, `emacs`, `micro`, `kak`                 | `✎`             | `nf-fa-pencil`, U+F040                    |
   | `git`      | `git`, `lazygit`, `tig`, `gitui`                                                    | `±`             | `nf-fa-code_fork`, U+F126                 |
   | `monitor`  | `top`, `htop`, `btop`, `glances`                                                    | `◔`             | `nf-fa-tachometer`, U+F0E4                |
   | `remote`   | `ssh`, `mosh`, `et`                                                                 | `⇄`             | `nf-fa-exchange`, U+F0EC                  |
   | `build`    | `cargo`, `make`, `just`, `npm`, `pnpm`, `yarn`, `go`                                | `▲`             | `nf-fa-cog`, U+F013                       |
   | `pager`    | `less`, `more`, `man`, `bat`                                                        | `¶`             | `nf-fa-book`, U+F02D                      |
   | `repl`     | `python`, `python3`, `node`, `irb`, `ghci`, `iex`                                   | `λ`             | `nf-fa-code`, U+F121                      |

2. **Config**: `[theme] icons = "unicode" | "nerd" | "none"` (default `unicode`, next to DECK-08's `border_style`) picks the preset; `[theme.icon_glyphs]` overrides single kinds (`claude = "C"`). Every icon glyph follows DECK-08's one-cell rule; a wider glyph or an unknown kind is an error naming the key. With `icons = "none"` no icon is drawn anywhere; `[theme.icon_glyphs]` is still checked but not drawn. All of it is documented in `default.toml`, which the DECK-08 sync test checks.
3. **The `nerd` preset** is the table's last column: Font Awesome glyphs, which every Nerd Font carries at these codepoints. Nerd Fonts has no Anthropic or OpenAI logo, so the agents get generic glyphs (Claude Code's asterisk-like mark, a notch circle, a cube). Before using them, the worker checks each name and codepoint against Nerd Fonts' `glyphnames.json` and, if one is wrong, fixes the table and records it in Notes.
4. **Panel icon**: the icon of the panel's source terminal (sub-spec 01's choice), from its `agent` (DECK-06) and `program` (sub-spec 01). A panel with no terminals has no icon.
5. **Sidebar row**: `▌`, the status column, a space, the icon, a space, then the name; the name keeps DECK-02's truncation and right-aligned terminal count in the space that is left. With `icons = "none"` the icon and its space are not drawn, and the row is DECK-02's.
6. **Status bar**: `project › <status> <icon> <name>`, the status glyph of the sidebar row (DECK-06 requirement 9; left out without an agent) and no icon with `icons = "none"`. The ` · <title>` suffix from DECK-02 is drawn only when the panel's `name_source` is `manual`, using the source terminal's title, and only when that title differs from the name.
7. **Glyph literals**: the icon glyphs live only in the theme module; DECK-08's source-scan test in `deck-tui` gets both presets' glyphs added to its list of forbidden literals.

## Screen

The same sidebar as sub-spec 01, with icons (`unicode` preset); the icon column is drawn in the row's own colour (accent for the active panel):

```text
 main                       │
                            │
 ▾ terminal-multiplexer     │
▌⠹ ✻ Fix login bug          │
   ✎ nvim                   │
 ! ◎ Review the API changes │
   ❯ ~/Projects/terminal-m… │
   ± api                    │
```

- `api` was renamed by hand and runs `lazygit`: its name stays, its icon follows the program.

Status bar:

```text
 main  terminal-multiplexer › ⠹ ✻ Fix login bug                                ⌥Space ? help  d detach
```

## Acceptance criteria

- [ ] `icon_for` unit tests: each agent kind; every program in the table; `NVIM` (case); an unknown program and `None` give `terminal`; an agent running under `node` gives the agent's icon, not `repl`.
- [ ] Config tests: the three presets (every glyph of `unicode` and `nerd` is one cell), a `[theme.icon_glyphs]` override, a two-cell glyph and an unknown kind are errors naming the key; `default.toml` stays in sync.
- [ ] `deck-tui` drawing tests: the Screen above at 28 columns; `icons = "none"` draws DECK-02's rows; a manually named panel keeps its name with the live icon and shows ` · <title>` in the status bar.
- [ ] End to end: a fake `claude` agent's panel row starts with `✻` and a shell's with `❯` (capture `icons`); sub-spec 01's fake `nvim` script switches the row to `✎` within 2 s and back to `❯` after it exits.

## Tests

- `deck-core`: `icon_for`, config parsing and validation of the icon keys.
- `deck-tui`: `TestBackend` drawing of the sidebar and status bar with each preset.
- `crates/deck/tests/titles.rs`: the end-to-end criterion.

## Notes

- Every icon is one cell by `unicode-width`, but some terminals draw Nerd Font glyphs two cells wide, and terminals set to treat East Asian Ambiguous characters as wide do the same for `◎ ◇ ◔ ± ¶ λ ▲`. The space after the icon keeps the name readable when that happens. No default glyph has an emoji form (which is why `build` is `▲`, not `⚙`).
- **`[theme.icons]` is `[theme.icon_glyphs]`.** TOML cannot hold both `icons = "unicode"` and a `[theme.icons]` table under `[theme]`, so the preset keeps the name `icons` and the per-kind table is `[theme.icon_glyphs]`. The resolved theme adds `icons` and `icon_glyphs`, so `PROTOCOL_VERSION` is 13.
- **Nerd Fonts check.** All eleven names and codepoints of the `nerd` column match `glyphnames.json` of Nerd Fonts 3.5.1 (`fa-asterisk` f069, `fa-circle_o_notch` f1ce, `fa-cube` f1b2, `fa-terminal` f120, `fa-pencil` f040, `fa-code_fork` f126, `fa-tachometer` f0e4, `fa-exchange` f0ec, `fa-cog` f013, `fa-book` f02d, `fa-code` f121). No change to the table.
- **Status bar.** DECK-06's `kind ·` after the panel name is gone: the icon and the name replace it. The status glyph stays, in front of the icon. A kept terminal that exited still adds ` · exited <code>`.
- **`icons = "none"`** still validates `[theme.icon_glyphs]` (an error names the key) and does not draw it.
