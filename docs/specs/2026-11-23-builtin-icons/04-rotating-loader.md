---
type: shippable
status: todo
---

# 04 — A rotating loader at 60 frames per second

## Scope

In Ghostty and kitty (the `Image` tier), a working agent's status mark is Hugeicons' `Loading02` turning smoothly, one revolution per second at 60 frames per second, in the sidebar and in grid border titles. The `Nerd` and `Unicode` tiers keep their glyph frames every 100 ms. Owning crate: `deck-tui`.

## Requirements

1. **Frames**: 60 frames per revolution. Frame `k` (0..60) is the `spinner` mask of [03](03-icon-images.md) rotated clockwise by `6·k` degrees around the mask's centre (31.5, 31.5), sampled bilinearly, pixels outside the source transparent. A pure function `spinner_frame(k) -> [u8; 4096]`, computed on first use and cached for the client run (60 × 4 KiB).
2. **Images**: each (frame, colour) is an image of the registry (03 requirement 4), with its own id. The first time a colour is needed for the spinner, the pre-pass transmits all 60 frames of that colour before the frame that draws the first one, so the turn never waits on a transmission. In practice there are one to three colours: `accent`, the reversed active row's visible text colour, and `muted` while a spinning row is dragged (DECK-18 dims it), which sends its burst when the drag starts.
3. **Which frame**: from the clock, not a counter: `k = floor(elapsed · 60) mod 60`, where `elapsed` is the time since the client started (an `Instant` stored in `App`; tests pass `now`). A late tick shows the frame for its time, so the turn keeps its speed when frames are dropped. Every spinning terminal shows the same frame.
4. **Timer**: while the `Image` tier is on and a terminal spins (`agent::view_spins`, as today), the spinner deadline is the next 1/60 s boundary of `elapsed` (so ticks do not drift); otherwise `SPIN_EVERY` (100 ms) as today. When nothing spins, no timer runs (unchanged). Only the frames move: a tick that changes nothing else marks the app dirty for the spinner alone.
5. **Cost of a frame**: a spinner tick redraws through ratatui as any frame does, so only the spinner cells (two per spinning slot, new foreground colour) are written to the outer terminal. Nothing else in the draw path changes for this sub-spec unless the budget below is missed.
6. **Budget**, measured by the worker on the release build (`cargo build --release`), in Ghostty on macOS, 200×50 cells, a session with four panels of which three run a working agent (a real one, or the fake agent the e2e tests use), and one `htop` panel:
   - the draw-and-flush of a spinner tick takes under 4 ms at the 95th percentile (measured with temporary timing code that is not committed);
   - the client process uses under 15% of one core over 30 seconds (`top`);
   - the loader looks smooth by eye.
     If a target is missed, the worker reports the numbers and the slowest part of the draw, and stops; the lead brings the choice to the user (make the draw cheaper, or a lower rate).

## Screen

Same cells as 03; the status slot of a working terminal holds a placeholder pair whose id changes 60 times a second, cycling through 60 ids.

## Acceptance criteria

- [ ] `spinner_frame(0)` equals the `spinner` mask; frame 15 equals the mask rotated by 90° within ±2 per pixel; every frame keeps the mask's total alpha within 5%.
- [ ] Frame choice: `now` at 0, 16.7 ms, 1 s and 1.5 s gives frames 0, 1, 0 and 30; two spinning terminals at one `now` show the same id.
- [ ] Timer: with the `Image` tier and a spinning terminal, `wake_deadline` is at most 1/60 s ahead and on a 1/60 s boundary; in the `Nerd` and `Unicode` tiers it is today's 100 ms; with nothing spinning, `None`.
- [ ] All 60 frames of a colour are transmitted before the first frame that draws one, and never again; a second colour sends its own 60; exit deletes all ids.
- [ ] `TestBackend`: two draws 1/60 s apart differ only in the spinner cells.
- [ ] End to end: the status slot of a working fake agent holds U+10EEEE in the `Image` tier.
- [ ] The budget of requirement 6 is met, with the numbers in the worker's report.
- [ ] By hand, in Ghostty and kitty: the loader turns smoothly in the sidebar and in a border title, and stops turning (switching to the idle or waiting mark) when the agent's turn ends.

## Tests

- `deck-tui`: rotation (frame 0, 90°, alpha kept), frame choice from `now`, the timer deadlines per tier, transmission of the 60 frames, the `TestBackend` diff.
- `crates/deck/tests/`: with `TERM_PROGRAM=ghostty` and `COLORTERM=truecolor`, a fake `claude` panel whose agent is working has U+10EEEE in its status slot (capture `rotating-loader`). The harness cannot see the rotation; the frame maths is covered by the unit tests.

## Notes

- Kitty's animation protocol (`a=f`, `a=a`) would let the terminal turn the loader with no redraws, but Ghostty does not implement it; redrawing works in both.
- The bandwidth is small: a spinner tick changes the foreground colour of two cells per spinning slot (an SGR and two placeholder characters with diacritics), about 60 bytes each.
- One burst of about 1.3 MB of base64 goes out the first time a colour is needed; Ghostty and kitty take it in well under a frame on a local machine. Over SSH it may hitch once.
