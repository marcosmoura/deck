---
branch: feat/agent-logos
backlog: DECK-12
---

# Agent logos as images

## Goal

In Ghostty and kitty, the sidebar and the status bar show the real logos of Claude Code, Codex (OpenAI's mark) and OpenCode in front of an agent panel's name, drawn as small images through the kitty graphics protocol. Every other terminal, and every other program, keeps the text icon of [DECK-09](../2026-11-22-sidebar-titles/00-overview.md) (`[theme] icons` and `[theme.icon_glyphs]`), which is the stand-in.

## Builds on

- [DECK-09 sidebar-titles](../2026-11-22-sidebar-titles/00-overview.md): `IconKind`, `icon_for`, the source terminal, the icon column of sidebar rows and the status bar (`project › <status> <icon> <name>`), `[theme] icons`.
- [DECK-08 config-theming](../2026-11-22-config-theming/00-overview.md): theme roles, `default.toml` and its sync test, the rule that `Color` values are built only in the colour conversion module.
- [architecture.md](../../architecture.md#client): the client draws with ratatui on the outer terminal; config reload.

## Non-goals

- Images for programs other than the three agents (editors, git, shells keep their text icons).
- Terminals other than Ghostty and kitty, and other image protocols (iTerm2 inline images, Sixel). They get the text icon.
- Images in the session picker, the help overlay, terminal borders or DECK-10's menu.
- Images through tmux or screen when deck itself runs inside them (they get the text icon).
- Asking the terminal whether it supports graphics (a query and its reply on stdin); detection is by environment only.
- Logos in brand colours; a later `icon_images = "brand"` could add them.

## Decisions

- **Kitty graphics protocol with Unicode placeholders.** Ghostty and kitty both implement it. The client transmits each logo once as a _virtual placement_ (`U=1`, one cell: `c=1,r=1`), then draws it like text: the cell holds the placeholder character U+10EEEE with row and column diacritics, and its foreground colour carries the image id. ratatui's buffer, diffing and redraws treat it as one ordinary cell, so scrolling, hover, overlays and resizes need nothing special. Every command sets `q=2`, so the terminal sends no reply into deck's input.
- **Logos are alpha masks tinted at runtime.** Each logo ships as a 64×64 8-bit alpha mask (4096 bytes, `include_bytes!`) made once from its SVG. The client tints it with the colour the text icon would have had (the row's colour: `sidebar_fg`, `accent` for the active panel, and so on), resolved to RGB with the terminal's own answers (OSC 10 for the default foreground, OSC 4 for the 16 base colours), and sends raw RGBA (`f=32`). One image is transmitted per (logo, colour) pair, at most a handful.
- **Sources.** Claude and OpenAI from [Simple Icons](https://simpleicons.org) 15.22.0 (CC0). OpenCode from its repository's brand files (`services/site/public/brand/files/` in `anomalyco/opencode`). The SVGs are committed next to the masks. The logos are trademarks of their owners, used only to identify the program that runs.
- **Detection by environment** of the client process: on when `TERM` is `xterm-kitty` or `xterm-ghostty`, or `TERM_PROGRAM` is `ghostty`, or `KITTY_WINDOW_ID` is set; off inside tmux, Zellij or a deck panel, and without 24-bit colour. A config key forces it off.
- **Row colours, not brand colours.** A logo is tinted with its row's colour, like the text icons, so the active panel's logo is in `accent`.
- **No new crate.** Base64 (about 30 lines) and the escape sequences are written in `deck-tui`.

## Sub-specs

| Sub-spec                | Type      | Crate                             | Summary                                                                                     | Depends | Status |
| ----------------------- | --------- | --------------------------------- | ------------------------------------------------------------------------------------------- | ------- | ------ |
| [01](01-kitty-logos.md) | shippable | deck-tui, deck-core, deck (tests) | Logo masks, `icon_images`, detection, colour resolution, transmission and placeholder cells | DECK-09 | todo   |

## Affected docs

- `docs/features/sidebar.md`: logos in Ghostty and kitty.
- `docs/features/config.md` and `default.toml`: `icon_images`.
- `docs/architecture.md#client`: images through the kitty graphics protocol, the OSC 4 query, the colour exception for placeholder cells, the `crates/deck-tui/assets/logos/` directory with its licence and trademark note, and the e2e harness clearing terminal-detection variables.

Reviewed by spec-reviewer on 23 Nov 2026.

Approved by the user on 23 Nov 2026.
