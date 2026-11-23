---
type: shippable
status: todo
---

# 01 — Agent logos through the kitty graphics protocol

## Scope

In Ghostty or kitty, a Claude Code, Codex or OpenCode panel shows the agent's logo instead of its text icon, in its sidebar row and in the status bar. Elsewhere nothing changes. Owning crates: `deck-tui` (assets, detection, colour resolution, escape sequences, drawing), `deck-core` (the config key), `deck` (test harness environment).

## Requirements

1. **Config**: `[theme] icon_images = "auto" | "off"` (default `auto`), documented in `default.toml` (DECK-08 sync test). Any other value is an error naming `theme.icon_images`. With `icons = "none"` no icon and no image is drawn. The key is reloadable: going to `off` (or to `icons = "none"`) draws text icons from the next frame; images already sent stay in the terminal until exit.
2. **Detection** (pure function in `deck-tui` over an environment map, read once from the client's environment at start): images are on when `icon_images = "auto"` and `TERM` is `xterm-kitty` or `xterm-ghostty`, or `TERM_PROGRAM` is `ghostty`, or `KITTY_WINDOW_ID` is set and non-empty. They are off, whatever the rest says, when any of `TMUX`, `ZELLIJ` or `DECK` is set and non-empty (deck sets `DECK` in every panel, so a `deck` started inside a panel uses text icons), or `TERM` starts with `screen` or `tmux`. Images also need 24-bit colour: when the client draws in 256 colours (`truecolor` off, DECK-08), images are off.
3. **Assets**: `crates/deck-tui/assets/logos/` holds `claude.svg`, `openai.svg`, `opencode.svg` (the sources in the overview) and `claude.mask`, `openai.mask`, `opencode.mask`: 64×64 8-bit alpha, row-major, 4096 bytes each, the logo centred with a 4-pixel margin. The masks are a one-off artifact made outside the build with whatever tool is at hand (for example `rsvg-convert` to a 56×56 PNG, then a throwaway script that pads it to 64×64 and keeps the alpha channel); nothing is added to the repository's tooling, `justfile` or `.mise.toml`. A `README.md` in that directory names each source with its version or commit, its licence, the trademark note, and the exact commands used, so a reviewer can make the masks again and compare. A unit test checks each mask is 4096 bytes and not all zero. `IconKind::Codex` uses `openai`.
4. **Tint colour**: an image is a (logo, RGB) pair whose pixels are that colour with the mask as alpha. The RGB is the colour the text icon would have had (the row's style: `sidebar_fg`, `accent` for the active panel, and so on), resolved to RGB:
   - `Rgb(r, g, b)` as is;
   - `Default`: the outer terminal's foreground from the OSC 10 reply the client already asks for (`colors.rs`); without a reply, `#c0c0c0`;
   - `Indexed(16..=255)`: the xterm 6×6×6 cube and grey ramp (the inverse of `nearest_256`);
   - `Indexed(0..=15)`: the outer terminal's own palette, asked with `OSC 4;n;?` for n = 0..15 in the same start-up query as OSC 10 and 11 (before the DA1 request, so the existing probe still ends on DA1), and parsed like the OSC 10/11 replies; an index with no reply uses the standard xterm value.
5. **Ids**: each pair gets a 24-bit id, never 0, unique within the client run (a counter from `0xDEC001`). The placeholder's foreground is `Color::Rgb` of the id's three bytes, built in the colour conversion module (`convert.rs`), the one exception to DECK-08's rule, with a comment. The 256-colour `downsample` never sees it, since images are off without truecolor (requirement 2).
6. **Transmission**: before each `terminal.draw`, the client computes the pairs the frame needs with the same function the drawing uses to pick an icon and its style (a pre-pass over the app state, no change to `draw(&App)`), and for each pair not yet sent writes to `terminal.backend_mut()`, then flushes: `ESC _G a=T,U=1,f=32,s=64,v=64,c=1,r=1,i=<id>,q=2,m=1;<base64> ESC \` with the RGBA data in base64 chunks of at most 4096 bytes (`m=1` on every chunk but the last, which has `m=0`; later chunks carry only `m` and `q`). Each pair is sent once per client run.
7. **Drawing**: where DECK-09 draws an agent's icon (sidebar row, status bar), the cell's symbol is U+10EEEE followed by U+0305 twice (row 0, column 0), with the id colour as foreground; the row's background and modifiers are kept. The placeholder and diacritic constants live in the new graphics module next to the escape-sequence code; they are not theme glyphs, so DECK-08's glyph scan is unchanged. Other kinds, and every icon when images are off, are DECK-09's text glyphs. Widths do not change (one cell), so truncation and counts stay DECK-09's.
8. **Cleanup**: when the client exits or detaches, it writes `ESC _G a=d,d=I,i=<id>,q=2 ESC \` for every id it sent, before leaving the alternate screen. A crash leaves a few kilobytes in the terminal's image store, freed when the window closes.
9. **No input**: every command has `q=2`; the client never waits for or parses a graphics reply.
10. **Test harness**: the e2e harness (`crates/deck/tests/support/mod.rs`) removes `TERM_PROGRAM`, `KITTY_WINDOW_ID`, `TMUX`, `ZELLIJ` and `DECK` from the client's environment by default, so a developer's own terminal never changes test results; a test opts in with `.env(...)`.

## Screen

Ghostty, 28-column sidebar; `C`, `O` and `◆` below stand for the one-cell Claude, OpenAI and OpenCode logos, tinted like the text around them (the active row's logo in `accent`):

```text
 main                       │
                            │
 ▾ terminal-multiplexer     │
▌⠹ C Fix login bug          │
   ✎ nvim                   │
 ! O Review the API changes │
   ◆ Plan the release       │
   ❯ ~/Projects/terminal-m… │
```

Status bar: ` main  terminal-multiplexer › ⠹ C Fix login bug   ⌥Space ? help  d detach`.

The same screen in another terminal, inside tmux, or with 256 colours is DECK-09's, with `✻`, `◎` and `◇` (or the `nerd` glyphs).

## Acceptance criteria

- [ ] Detection table: Ghostty (`TERM_PROGRAM=ghostty`, `TERM=xterm-ghostty`), kitty (`TERM=xterm-kitty`, `KITTY_WINDOW_ID=1`); off with `TMUX`, `ZELLIJ` or `DECK` set, with `TERM=screen-256color`, with `TERM=xterm-256color` alone, with `icon_images = "off"`, and without truecolor.
- [ ] Colour resolution: `Rgb`, `Default` with and without an OSC 10 reply, `Indexed(4)` with an OSC 4 reply and without one, `Indexed(110)` from the cube, `Indexed(244)` from the grey ramp; the OSC 4 replies are parsed in both BEL and ST forms and never reach the input.
- [ ] Base64 matches the RFC 4648 test vectors; the transmit sequence of a 64×64 image has the exact header above, chunks of at most 4096 base64 bytes, `m=1` on all but the last, and decodes back to the tinted RGBA.
- [ ] `TestBackend` drawing with images on: the Claude row's icon cell is U+10EEEE with two U+0305 and the id colour as foreground; a shell row still shows `❯`; the status bar's icon cell is the placeholder; the active row's logo has a different id from an inactive row's (accent vs `sidebar_fg`).
- [ ] With images off (or `icons = "none"`), drawing is byte-for-byte DECK-09's.
- [ ] Each pair is transmitted once across many frames and before the first frame that draws it; a colour change after a config reload sends the new pair; reloading to `icon_images = "off"` draws text icons; exit writes one delete per sent id (writer captured in a unit test).
- [ ] `icon_images = "always"` is a config error naming `theme.icon_images`; `default.toml` stays in sync.
- [ ] End to end: with `TERM_PROGRAM=ghostty` and `COLORTERM=truecolor` set by the test, a fake `claude` panel's sidebar row holds U+10EEEE in the icon cell (capture `logos`); with `TMUX=/tmp/x,1,0` added it holds `✻`; existing e2e tests pass when run from Ghostty, kitty or tmux.
- [ ] By hand, in Ghostty and in kitty: the three logos show in the sidebar and the status bar, follow the active row's colour, and disappear after `d` detach.

## Tests

- `deck-tui`: detection, colour resolution and OSC 4 parsing, base64, the transmit and delete sequences, the image registry (once per pair, ids, cleanup), `TestBackend` drawing.
- `deck-core`: parsing of `icon_images`, `default.toml` sync.
- `crates/deck/tests/titles.rs`: the end-to-end criterion; `crates/deck/tests/support/mod.rs`: the environment change.

## Notes

- Kitty documents Unicode placeholders in its graphics protocol page; Ghostty implements them (virtual placements with `U=1`, direct RGBA with `f=32`, chunking, `a=d,d=I`). The worker checks the current kitty protocol text for the placeholder rules before writing the code.
- The third diacritic (the id's high byte) is left out on purpose: ids stay under 2^24. The row and column diacritics are written explicitly so a placeholder never takes its row and column from a placeholder to its left.
- A cell is taller than it is wide, so the square logo is scaled to the cell's width. It is small but recognisable at common font sizes; a later change could use two cells.
