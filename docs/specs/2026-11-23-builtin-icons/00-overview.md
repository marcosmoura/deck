---
branch: feat/builtin-icons
backlog: DECK-20
---

# Built-in icons

## Goal

deck ships one icon set of its own and nothing in the config changes it. Every icon deck draws (the program icons, the space carets, the agent status marks and the unseen marker) comes in three forms, and the client uses the best one the outer terminal can show:

1. an **image** made from a [Hugeicons](https://hugeicons.com) free SVG, through the kitty graphics protocol (Ghostty and kitty), with a real rotating loader for a working agent at 60 frames per second;
2. a **Nerd Font glyph**, in terminals known to ship the Nerd Font symbols;
3. a **Unicode glyph** every font has, everywhere else.

Asked by the user on 23 Nov 2026: "remove every single icon override customization, and ship with our own internal icons … svg image -> nerd fonts -> unicode", with these keys gone: `[theme] icons`, `[theme] icon_images`, `[theme.glyphs]`, `[theme.icon_glyphs]`, `[theme.icon_colors]`.

## Builds on

- [DECK-09 sidebar-titles](../2026-11-22-sidebar-titles/00-overview.md): `IconKind`, `icon_for`, the icon column of panel rows.
- [DECK-12 agent-logos](../2026-11-23-agent-logos/00-overview.md): kitty graphics detection, alpha masks tinted at runtime, Unicode placeholder cells, image ids in the foreground colour, transmission once per (image, colour) pair, cleanup on exit (`deck-tui/src/graphics.rs`). This spec extends that machinery from three logos to every icon; it does not replace it.
- [DECK-08 config-theming](../2026-11-22-config-theming/00-overview.md): theme roles, `default.toml` and its sync test, unknown keys are warnings.
- [DECK-06 agent-status](../2026-11-22-agent-status/00-overview.md): statuses, the spinner timer (`App.spin`, `spin_at`, `agent::SPIN_EVERY`), title marks in grid borders.
- [DECK-17](../2026-11-23-floating-bar-menus/00-overview.md) and [DECK-18](../2026-11-23-sidebar-reorder/00-overview.md): the sidebar as it is today (New menu, restore dialog, space rows, drag).

## Non-goals

- Any way to change an icon, its colour or the tier: no config key, no environment variable. (The theme's colour roles stay; they colour the icons as today.)
- Image protocols other than kitty's (Sixel, iTerm2), and images inside tmux, Zellij or a deck panel: DECK-12's detection is kept as is, minus its config key.
- Asking the terminal which font it has; the Nerd Font tier is chosen from the environment only.
- Kitty animation frames (`a=f`, `a=a`): Ghostty does not implement them, so the loader is animated by redrawing.
- Brand colours for Codex and OpenCode.
- Rendering SVG at run time or at build time: masks are made once, offline, as in DECK-12. No new crate.
- Images in the help overlay and the session picker (they draw no icons today).

## Decisions

- **One icon set, in `deck-core`.** A new `deck_core::icons::Icon` names every icon deck draws: the eleven `IconKind`s, `Expanded`, `Collapsed`, `DropTarget` (DECK-18's drag caret), `Spinner`, `Waiting`, `Idle`, `Quiet` and `Unseen`. Each has a fixed Unicode glyph and a fixed Nerd Font glyph (tables in [01](01-icon-tiers.md)), and, all but `DropTarget`, an image (tables in [02](02-icon-images.md)). `Theme` loses `icons`, `icon_images`, `glyphs`, `icon_glyphs` and `icon_colors`.
- **The tier is chosen once per client run, from its environment and truecolor**, as a pure function in `deck-tui`: `Image` when DECK-12's detection says images work; else `Nerd` when the outer terminal is one that bundles the Nerd Font symbols (Ghostty, kitty, WezTerm; detected by `TERM`, `TERM_PROGRAM`, `KITTY_WINDOW_ID`, `GHOSTTY_RESOURCES_DIR` or `WEZTERM_EXECUTABLE`, also inside tmux, Zellij or a deck panel, because the outer terminal still draws the glyphs); else `Unicode`. Ghostty and kitty bundle the Nerd Font symbols, so in the `Image` tier an icon without an image yet (every icon but the agents' after 01, the spinner after 02) is its Nerd glyph.
- **Every icon slot is two cells wide in every tier**: an image fills both cells, a glyph sits in the first and the second is blank (DECK-12 sub-spec 02 already does this for the program icon). This applies to the status column, the space caret and the unseen marker too (not to the drag caret, which keeps its one cell and its place), so a layout is the same in all three tiers. A square image in a 2×1 box comes out about one cell tall (DECK-12's note).
- **Colours stay roles, plus one fixed brand colour.** Status marks keep today's roles (spinner `accent`, waiting `warning` bold, idle `ok`, quiet `muted`, unseen `accent`); space carets keep `muted` (`sidebar_fg` under hover, as today); program icons take the row's colour; Claude's icon is always `#d97757` (today's default `icon_colors.claude`), in every tier. Codex and OpenCode take the row's colour.
- **Images from Hugeicons free**, the `@hugeicons/core-free-icons` npm package 4.4.0 (MIT, stroke-rounded style, 24×24 viewBox, 1.5 stroke). The package ships icons as JavaScript arrays of SVG elements; a throwaway script writes each one used as an `.svg` file, committed next to its mask. Hugeicons has no OpenCode logo, so OpenCode keeps DECK-12's `opencode.svg` and mask. The unseen marker is Hugeicons' `Circle` with a fill added (the free set is stroke only); that change is noted in the assets README.
- **The loader is Hugeicons' `Loading02`** (an open ring), rotated by the client at run time: one revolution per second, 60 frames, each frame the mask rotated by 6° around its centre (bilinear), tinted and sent once per colour. The frame shown is picked from the clock, so a late tick skips a frame instead of slowing the turn. The spinner timer runs at 60 Hz only while the `Image` tier is on and a spinner is on screen; the text tiers keep today's 100 ms frames.
- **Removed keys warn, they do not fail.** `[theme] icons`, `[theme] icon_images` and the tables `[theme.glyphs]`, `[theme.icon_glyphs]` and `[theme.icon_colors]` each give one warning (`theme.icons was removed: deck ships its own icons`) and are otherwise ignored, so an existing config keeps loading. DECK-08's per-key glyph checks and the "removed active glyph" warning go with them.
- **Protocol version 20.** `Theme` travels in `Welcome` and `ServerMsg::Config`, so removing its five fields changes the wire format: sub-spec 01 bumps `PROTOCOL_VERSION` from 19 to 20. DECK-19 also plans a bump to 20; whichever lands second takes the next number.
- **No new crate, no new tool.** Masks are made with `rsvg-convert` (or any rasterizer) and a throwaway script, outside the build, as in DECK-12.

## Sub-specs

| Sub-spec                    | Type      | Crate                             | Summary                                                                                                                                             | Depends | Status |
| --------------------------- | --------- | --------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- | ------- | ------ |
| [01](01-icon-tiers.md)      | shippable | deck-core, deck-tui, deck (tests) | `Icon` with fixed Nerd and Unicode glyphs, tier detection, two-cell slots everywhere, the five config keys removed with warnings, docs              | —       | todo   |
| [02](02-icon-images.md)     | shippable | deck-tui, deck (tests)            | Hugeicons SVGs and masks for every icon; images for every icon in the `Image` tier (the spinner still text)                                         | 01      | todo   |
| [03](03-rotating-loader.md) | shippable | deck-tui                          | The `Loading02` image rotated at 60 frames per second: frames built and sent lazily per colour, a 60 Hz timer while a spinner shows, a frame budget | 02      | todo   |

## Affected docs

- `docs/features/config.md` and `default.toml`: the five keys and their sections removed; one paragraph on the built-in icons and the removal warning.
- `docs/features/sidebar.md`, `docs/features/agents.md`: the icon tiers, two-cell status and caret slots, the loader; mockups updated.
- `docs/architecture.md`: the protocol line (version 20 and why), the config section's list of `[theme]` keys, and the `unicode-width` row's purpose in the dependency table.
- `docs/architecture.md#client`: the tier detection, the icon table in `deck-core`, images for every icon, the 60 Hz spinner; the "Agent logos" paragraph generalised; `crates/deck-tui/assets/` with the Hugeicons licence. "Nerd Font glyphs are never required" stays true (Unicode is the last tier).

## Risks

- **Nerd Font versions.** The Codicons Claude and OpenAI glyphs (U+EC82, U+EC81) are recent additions to Nerd Fonts (today's `nerd` preset already uses them). A terminal that bundles an older symbols font draws a missing-glyph box for those two in the `Nerd` tier. Checked by hand in sub-spec 01 in Ghostty (inside tmux), kitty (256 colours) and WezTerm; where they are missing, the agent rows use their Unicode glyph in the `Nerd` tier (a one-line table change).
- **A stale environment.** Inside tmux or a deck panel the `Nerd` tier trusts variables inherited from whatever started the tmux server or the daemon. A tmux session started in Ghostty and attached later from Terminal.app draws missing-glyph boxes. Accepted: there is no reliable way to ask the terminal which font it has.
- **60 frames per second.** Each spinner frame is a full `terminal.draw`; ratatui writes only the changed cells (two per spinning icon), but the draw itself walks every panel. Sub-spec 03 measures it and stops for a decision if the budget is missed.
- **DECK-19 (images in panels)** is drafted at the same time and also changes `graphics.rs`, the client's image ids and the protocol version. The user put DECK-20 first in the queue (23 Nov 2026). Whichever lands second merges the other's changes, keeps ids unique within a client run from one counter, and takes the next protocol version.

Reviewed by spec-reviewer on 23 Nov 2026.
