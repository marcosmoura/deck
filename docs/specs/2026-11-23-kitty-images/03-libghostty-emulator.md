---
type: foundation
status: done
---

# 03 — libghostty-vt as the daemon's emulator

## Scope

The daemon emulates every terminal with `libghostty-vt` instead of `alacritty_terminal`, on the emulator threads of sub-spec 02, with the build of sub-spec 01. Nothing changes for the user: same screen, same keys, same scrollback, same control API answers. Sub-spec 04 turns kitty graphics on in this emulator. Owning crate: `deck-daemon`; architecture.md.

## Requirements

1. **`alacritty_terminal`** leaves `deck-daemon`'s dependencies; it stays a dev-dependency of `deck` for the e2e harness (its row in architecture.md says `deck (tests)`).
2. **`Emu` on `libghostty-vt`.** `emu.rs` and `emu/capture.rs` keep their public API and behaviour: `new` (now returning `Result`, handled by sub-spec 02's `start`), `size`, `resize`, `feed -> Vec<EmuEvent>`, `scroll(ScrollTo)`, `scroll_to_bottom`, `full_frame`, `damage_frame` (may become `Option<Frame>` for requirement 3), `reset_damage`, `modes`, `find_line`, `capture`, `clamp_size`. In particular:
   - **Frames**: runs per row as today (a wide character's spacer cell sends no text; a grapheme cluster is its base character followed by the rest; trailing blank default-styled cells are not sent); styles and colours mapped to `deck_core::protocol::Style` as today (palette indices stay `Indexed`, RGB stays `Rgb`, default stays `Default`; bold, dim, italic, any underline style, inverse, strikethrough, invisible).
   - **Damage**: `damage_frame` sends the rows `libghostty-vt` reports dirty (render-state dirty rows or the rows' dirty flags), and a full frame when the whole screen is dirty or the display is scrolled back, as today; reading damage clears it.
   - **Scrollback**: `scroll` and `scroll_to_bottom` move the viewport (`scroll_viewport`); `display_offset` and `history` come from the scrollbar (`total`, `offset`, `len`). `libghostty-vt`'s `max_scrollback` is a byte budget pruned in whole pages, whatever its doc says, and it can only be set at creation; check this first. The configured `scrollback` stays a number of lines: `Emu` gives `libghostty-vt` a byte budget large enough to hold that many lines at 250 columns (the formula in a comment; wider terminals keep fewer lines, and a narrow terminal with long lines may hold more than the cap, hidden but not freed). The worker measures the budget for the default 10,000 lines and reports it; architecture.md records it, and enforces the line cap itself: `history` never exceeds it, scrolling back stops at it, and `capture` with history returns at most that many history lines.
   - **Cursor**: position, visibility (hidden when scrolled past it, as today) and shape. A terminal whose program never chose a shape, or reset it with `CSI 0 SP q`, reports `CursorShape::Default`: set the default cursor style to `BlockHollow`, which no `DECSCUSR` value selects, and map it to `Default`. This holds after entering and leaving the alternate screen and after a full reset (`ESC c`).
   - **Modes**: the same `Modes` bits from `libghostty-vt`'s mode queries (DECCKM, bracketed paste, mouse click/drag/motion, SGR mouse, focus events, alternate screen, alternate scroll).
   - **Events**: drained from the crate's event queue (`Terminal::events`) after each write: `Title` from `TitleChanged` (an empty title is `Title(None)`), `Reply` from `PtyWrite`, `ClipboardStore` from `ClipboardWrite` (`"c"` for the clipboard, `"p"` for the primary selection; other locations dropped). `libghostty-vt` ignores OSC 52 reads, so `EmuEvent::ClipboardLoad` and its warning go away.
   - **Query replies**: every reply pinned by sub-spec 02 keeps its bytes. Device attributes (DA1, DA2, DA3) come from `Terminal::set_device_attributes`, set to today's answers (the crate's default answers otherwise); size reports (`CSI 14/16/18 t`) from the crate's size-report option (8×16-pixel cells until sub-spec 04); XTVERSION from `set_xtversion`, today's answer if today's emulator gave one, else `None` so it stays unanswered. Check each against the pinned tests.
   - **Kitty keyboard protocol**: `libghostty-vt` always answers `CSI ? u` with `CSI ? <flags> u` as a pty write, and has no option to stop. `Emu` drops that answer when it turns `PtyWrite` events into replies (only that exact reply form), so programs keep seeing no support, as today. deck still encodes keys the legacy way.
   - **Colour queries** (OSC 4, 10, 11): `libghostty-vt` answers them itself, and only for colours that are set. The emulator's default palette is set to today's xterm palette (`set_default_color_palette` from `default_palette`), and its default foreground and background (`set_default_fg_color`, `set_default_bg_color`) to the colours the daemon answers with today (the attached clients' `OuterColors`, else the fixed fallback), on creation and whenever those change. `EmuEvent::Color` and the daemon's answering code go away. A program that sets OSC 10/11 itself now gets its own colour back in later answers, as in any terminal; the existing answer tests keep passing otherwise.
3. **Synchronized output** (mode 2026): `libghostty-vt` only records the mode (`Terminal::mode(Mode::SYNC_OUTPUT)`; the crate also reports `RenderHold` events, which may be used). While it is set, no damage frame is built and the damage is kept; once it is reset, or 150 ms after it was set (alacritty's `SYNC_UPDATE_TIMEOUT`), the next frame goes out with all the damage. The time comes from the daemon's injected clock (passed to `Emu`, never `Instant::now()` inside it), and the daemon's frame pump (`pump` in `server.rs`) schedules a pass at the deadline, so a frame follows the timeout even without new output.
4. **Tests**: every existing daemon and e2e test passes. Unit tests in `emu.rs` that read alacritty's grid directly (`e.term.grid()`) are rewritten to read through `libghostty-vt`'s API or through frames, keeping what they check. If an e2e test pins something that differs only by emulator (reflow on resize, a grapheme's width), the worker reports it and the lead decides; no test is loosened silently.

## Acceptance criteria

- [ ] `deck-daemon` no longer depends on `alacritty_terminal`.
- [ ] Frames for the same input are the same as before for: plain text, wide characters, combining characters, every style flag, 16, 256 and RGB colours, trailing blanks, a scrolled-back display, the alternate screen.
- [ ] Damage frames carry only changed rows; a resize and a scrolled-back display give full frames.
- [ ] Writing twice the configured scrollback gives `history` equal to the limit, also after widening the terminal from 80 to 250 columns; `capture` with history returns that many lines.
- [ ] Cursor shape reports `Default` until a program sets one with `DECSCUSR`, again after `CSI 0 SP q`, after the alternate screen and after `ESC c`.
- [ ] The replies pinned by sub-spec 02 are unchanged: DA1 and DA2 answered as before, `CSI ? u` unanswered alone and after `CSI > 1 u`.
- [ ] OSC 10/11 answers use the client's colours, else the fallback; OSC 4 answers the xterm palette; OSC 52 copy reaches the client; title changes reach the sidebar.
- [ ] With mode 2026 set, no frame goes out until it is reset; a program that never resets it gets its frame 150 ms later on the injected clock.
- [ ] `deck capture` output (text and `--ansi`, with and without history) is unchanged for the same input.
- [ ] architecture.md (emulator, dependencies, a decision row) is updated.

## Tests

- `deck-daemon` unit tests in `emu.rs` and `emu/capture.rs`: the existing cases, ported; sub-spec 02's reply tests; the cursor-shape sentinel cases; scrollback over the limit, before and after widening; damage rows after writing one line; synchronized output with and without the reset.
- The existing daemon integration tests and e2e tests, unchanged in what they assert.

## Notes

- `ghostty-vt` 0.9's handles are `!Send + !Sync` and use the default allocator; effects arrive as `terminal::Event`s drained after each `vt_write`, so the `Emu` owns its `Terminal` with no callbacks to share state with.
- `EmuThread::spawn` (sub-spec 02, `terminal.rs`) returns before the emulator is built on its thread, which was fine while `Emu::new` could not fail. With `Emu::new` returning `Result`, `spawn` must wait for it: the thread sends the constructor's result back (a `sync_channel(1)` or oneshot) and `start` fails with its error, so requirement 1 of sub-spec 02 holds.
- Kitty graphics stay off here (storage limit 0), so programs see no image support until sub-spec 04.
