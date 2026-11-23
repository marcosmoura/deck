---
branch: feat/builtin-icons
backlog: DECK-20
---

# Built-in icons

## Goal

deck ships one icon set of its own and nothing in the config changes it. Every icon deck draws (the program icons, the space carets, the agent status marks and the unseen marker) comes in three forms, and the client uses the best one the outer terminal can show:

1. an **image** made from a [Hugeicons](https://hugeicons.com) free SVG, through the kitty graphics protocol (Ghostty and kitty, confirmed by asking the terminal), with a real rotating loader for a working agent at 60 frames per second;
2. a **Nerd Font glyph**, when the terminal can draw them: it bundles the Nerd Font symbols (Ghostty, kitty, WezTerm), or its configured font is a Nerd Font (iTerm2, Terminal.app), or a Nerd Font is installed for font fallback (fontconfig on Linux);
3. a **Unicode glyph** every font has, everywhere else.

Asked by the user on 23 Nov 2026: "remove every single icon override customization, and ship with our own internal icons … svg image -> nerd fonts -> unicode", with these keys gone: `[theme] icons`, `[theme] icon_images`, `[theme.glyphs]`, `[theme.icon_glyphs]`, `[theme.icon_colors]`.

## Builds on

- [DECK-09 sidebar-titles](../2026-11-22-sidebar-titles/00-overview.md): `IconKind`, `icon_for`, the icon column of panel rows.
- [DECK-12 agent-logos](../2026-11-23-agent-logos/00-overview.md): kitty graphics detection, alpha masks tinted at runtime, Unicode placeholder cells, image ids in the foreground colour, transmission once per (image, colour) pair, cleanup on exit (`deck-tui/src/graphics.rs`). This spec extends that machinery from three logos to every icon; it does not replace it.
- [DECK-08 config-theming](../2026-11-22-config-theming/00-overview.md): theme roles, `default.toml` and its sync test, unknown keys are warnings.
- [DECK-06 agent-status](../2026-11-22-agent-status/00-overview.md): statuses, the spinner timer (`App.spin`, `spin_at`, `agent::SPIN_EVERY`), title marks in grid borders.
- [DECK-17](../2026-11-23-floating-bar-menus/00-overview.md) and [DECK-18](../2026-11-23-sidebar-reorder/00-overview.md): the sidebar as it is today (New menu, restore dialog, space rows, drag).

## Non-goals

- Any way to change an icon, its colour or the tier: no config key, no environment variable (the e2e harness's `DECK_TEST_NO_FONT_CHECK` is a test seam, not a user setting). (The theme's colour roles stay; they colour the icons as today.)
- Image protocols other than kitty's (Sixel, iTerm2), and images inside a deck panel (as DECK-12).
- Mixing terminal multiplexers: inside tmux, Zellij or screen the queries reach the multiplexer and its answers decide; nothing tries to see through it.
- Kitty animation frames (`a=f`, `a=a`): Ghostty does not implement them, so the loader is animated by redrawing.
- Brand colours for Codex and OpenCode.
- Rendering SVG at run time or at build time: masks are made once, offline, as in DECK-12. No new crate.
- Images in the help overlay and the session picker (they draw no icons today).

## Decisions

- **One icon set, in `deck-core`.** A new `deck_core::icons::Icon` names every icon deck draws: the eleven `IconKind`s, `Expanded`, `Collapsed`, `DropTarget` (DECK-18's drag caret), `Spinner`, `Waiting`, `Idle`, `Quiet` and `Unseen`. Each has a fixed Unicode glyph and a fixed Nerd Font glyph (tables in [01](01-icon-tiers.md)), and, all but `DropTarget`, an image (tables in [03](03-icon-images.md)). `Theme` loses `icons`, `icon_images`, `glyphs`, `icon_glyphs` and `icon_colors`.
- **The tier is detected, not guessed** ([02](02-terminal-probe.md)), once per client run: the start-up probe gains XTVERSION (the terminal's name and version) and the kitty graphics query. `Image` when the terminal answers the graphics query `OK`, draws Unicode placeholders (Ghostty, kitty 0.28+) and has truecolor. Else `Nerd` when the terminal bundles the Nerd Font symbols (Ghostty, kitty 0.36+, WezTerm), or its configured font is a Nerd Font (iTerm2's profile, Terminal.app's profile, read with `defaults export`), or a font with the Nerd glyphs is installed where font fallback finds it (`fc-list`, or the macOS font folders, for the terminals a by-hand spike shows fall back). Else `Unicode`. The font check for the detected terminal runs on a thread after the probe; its answer comes back on a channel of the client loop, and a late one upgrades the tier once and redraws. Environment variables are only a fallback when the terminal answers neither query. Ghostty and kitty bundle the Nerd Font symbols, so in the `Image` tier an icon without an image yet (every icon but the agents' before 03, the spinner before 04) is its Nerd glyph.
- **Every icon slot is two cells wide in every tier**: an image fills both cells, a glyph sits in the first and the second is blank (DECK-12 sub-spec 02 already does this for the program icon). This applies to the status column, the space caret and the unseen marker too (not to the drag caret, which keeps its one cell and its place), so a layout is the same in all three tiers. A square image in a 2×1 box comes out about one cell tall (DECK-12's note).
- **Colours stay roles, plus one fixed brand colour.** Status marks keep today's roles (spinner `accent`, waiting `warning` bold, idle `ok`, quiet `muted`, unseen `accent`); space carets keep `muted` (`sidebar_fg` under hover, as today); program icons take the row's colour; Claude's icon is always `#d97757` (today's default `icon_colors.claude`), in every tier. Codex and OpenCode take the row's colour.
- **Images from Hugeicons free**, the `@hugeicons/core-free-icons` npm package 4.4.0 (MIT, stroke-rounded style, 24×24 viewBox, 1.5 stroke). The package ships icons as JavaScript arrays of SVG elements; a throwaway script writes each one used as an `.svg` file, committed next to its mask. Hugeicons has no OpenCode logo, so OpenCode keeps DECK-12's `opencode.svg` and mask. The unseen marker is Hugeicons' `Circle` with a fill added (the free set is stroke only); that change is noted in the assets README.
- **The loader is Hugeicons' `Loading02`** (an open ring), rotated by the client at run time: one revolution per second, 60 frames, each frame the mask rotated by 6° around its centre (bilinear), tinted and sent once per colour. The frame shown is picked from the clock, so a late tick skips a frame instead of slowing the turn. The spinner timer runs at 60 Hz only while the `Image` tier is on and a spinner is on screen; the text tiers keep today's 100 ms frames.
- **Removed keys warn, they do not fail.** `[theme] icons`, `[theme] icon_images` and the tables `[theme.glyphs]`, `[theme.icon_glyphs]` and `[theme.icon_colors]` each give one warning (`theme.icons was removed: deck ships its own icons`) and are otherwise ignored, so an existing config keeps loading. DECK-08's per-key glyph checks and the "removed active glyph" warning go with them.
- **Protocol version 20.** `Theme` travels in `Welcome` and `ServerMsg::Config`, so removing its five fields changes the wire format: sub-spec 01 bumps `PROTOCOL_VERSION` from 19 to 20. DECK-19 also plans a bump to 20; whichever lands second takes the next number.
- **No new crate, no new tool.** Masks are made with `rsvg-convert` (or any rasterizer) and a throwaway script, outside the build, as in DECK-12.

## Sub-specs

| Sub-spec                    | Type      | Crate                             | Summary                                                                                                                                                                                        | Depends | Status |
| --------------------------- | --------- | --------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------- | ------ |
| [01](01-icon-tiers.md)      | shippable | deck-core, deck-tui, deck (tests) | `Icon` with fixed Nerd and Unicode glyphs, tier detection, two-cell slots everywhere, the five config keys removed with warnings, docs                                                         | —       | todo   |
| [02](02-terminal-probe.md)  | shippable | deck-tui, deck (tests)            | XTVERSION and kitty graphics queries in the start-up probe; Nerd Font checks (bundled, iTerm2 and Terminal.app profile fonts, fontconfig); the real tier decision; a macOS font-fallback spike | 01      | todo   |
| [03](03-icon-images.md)     | shippable | deck-tui, deck (tests)            | Hugeicons SVGs and masks for every icon; images for every icon in the `Image` tier (the spinner still text)                                                                                    | 02      | todo   |
| [04](04-rotating-loader.md) | shippable | deck-tui                          | The `Loading02` image rotated at 60 frames per second: frames built and sent lazily per colour, a 60 Hz timer while a spinner shows, a frame budget                                            | 03      | todo   |

## Affected docs

- `docs/features/config.md` and `default.toml`: the five keys and their sections removed; one paragraph on the built-in icons and the removal warning.
- `docs/features/sidebar.md`, `docs/features/agents.md`: the icon tiers, two-cell status and caret slots, the loader; mockups updated.
- `docs/architecture.md`: the protocol line (version 20 and why), the start-up probe (XTVERSION, graphics query) and the font checks (`defaults`, `fc-list`) under the client, the `DECK_TEST_NO_FONT_CHECK` test seam under testing, and `caps.rs` as the one `deck-tui` module that spawns programs, the config section's list of `[theme]` keys, and the `unicode-width` row's purpose in the dependency table.
- `docs/architecture.md#client`: the tier detection, the icon table in `deck-core`, images for every icon, the 60 Hz spinner; the "Agent logos" paragraph generalised; `crates/deck-tui/assets/` with the Hugeicons licence. "Nerd Font glyphs are never required" stays true (Unicode is the last tier).

## Risks

- **Nerd Font versions.** The Codicons Claude and OpenAI glyphs (U+EC82, U+EC81) are recent additions to Nerd Fonts (today's `nerd` preset already uses them). A terminal that bundles an older symbols font draws a missing-glyph box for those two in the `Nerd` tier. Checked by hand in sub-spec 01 in Ghostty, kitty and WezTerm, and in 02 wherever it detects `Nerd`; where they are missing, the agent rows use their Unicode glyph in the `Nerd` tier (a one-line table change).
- **Font detection is indirect.** No terminal answers "does your font have this glyph"; deck reads the terminal's font setting or what font fallback can find. A terminal deck cannot read (iTerm2 with a custom preferences folder, an unknown macOS terminal without fallback) gets `Unicode` even with a Nerd Font. Sub-spec 02's spike settles how macOS fallback behaves before the code is written.
- **60 frames per second.** Each spinner frame is a full `terminal.draw`; ratatui writes only the changed cells (two per spinning icon), but the draw itself walks every panel. Sub-spec 04 measures it and stops for a decision if the budget is missed.
- **DECK-19 (images in panels)** is drafted at the same time and also changes `graphics.rs`, the client's image ids and the protocol version. The user put DECK-20 first in the queue (23 Nov 2026). Whichever lands second merges the other's changes, keeps ids unique within a client run from one counter, and takes the next protocol version.

Reviewed by spec-reviewer on 23 Nov 2026.
