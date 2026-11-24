---
type: shippable
status: done
---

# 06 — Drawing placed images in the client

## Scope

Run `kitten icat picture.png` (or chafa, timg with the kitty protocol) in a deck panel inside Ghostty or kitty, and the picture shows in the panel at its size, clipped to the terminal, moving with the text, hidden under dialogs and menus, gone when the program deletes it. Elsewhere the image's cells stay as the program left them. Program-made placeholders come in sub-spec 07. Owning crates: `deck-tui`; `deck` for the e2e tests and harness; docs.

## Requirements

1. **Detection.** The client sends `Attach { images: true }` when DECK-12's environment rule says the outer terminal speaks the kitty protocol (`TERM` `xterm-kitty` or `xterm-ghostty`, `TERM_PROGRAM` `ghostty`, or `KITTY_WINDOW_ID` set; never with `TMUX`, `ZELLIJ` or `DECK` set, or `TERM` starting with `screen` or `tmux`), the client draws in truecolor, and the cell size is known. `[theme] icon_images` governs only the logos, not program images. `graphics::detect` is split so both callers share the environment rule.
2. **Receiving pixels.** The client assembles each image's pieces, keyed by (terminal, screen, image, generation). A piece with `offset` 0 restarts the assembly; a piece whose `offset` is not where the assembly ended discards it. Pieces of an older generation are discarded. The assembled pixels are kept until `ImageGone`, the terminal leaving the session view, or detach.
3. **Outer ids.** One allocator for every image id the client sends, logos included: never 0, at most 24 bits, and an id freed by a delete is reused.
4. **Outer images for placed images.** One outer image per (terminal, screen, image, generation, `source`, `pixels`, drawn cols, drawn rows, cell size), built by a pure function of `deck-tui`:
   - the canvas is `cols × cell width` by `rows × cell height` pixels, transparent, with `cols` and `rows` cut to at most 297 each (the diacritics table);
   - the `source` rectangle of the image is scaled to `pixels` when the sizes differ (bilinear), computing only the part that lands on the canvas, and drawn at the canvas's top left;
   - a canvas over 64 MiB of RGBA is not built and the placement is not drawn (logged once);
   - it is sent with `a=t` (`f=32`, `s`, `v`, `i`, chunked with `m=1`, `q=2`) followed by one virtual placement (`a=p,U=1,i=<id>,c=<cols>,r=<rows>,q=2`); the outer terminal fits it to the box with no scaling.
5. **Deleting outer images.** After each draw, an outer image no placement of that draw used is deleted (`a=d,d=I,i=<id>,q=2`) and its id freed. Switching panels deletes the outer images of the panel left; switching back builds them again from the kept pixels.
6. **Drawing.** After a terminal's text is drawn into the buffer, before the text selection highlight (which skips image cells, so they keep the background they had) and before overlays, each `Placement` with `kind: Placed { col, row }` whose outer image is ready fills the cells of its rectangle (`col..col+cols`, `row..row+rows`, cut as in requirement 4) that lie inside the terminal's area:
   - each cell's symbol is U+10EEEE with the diacritics of its row and column inside the image, foreground the outer id as RGB, no modifiers, the background of the cell underneath; row and column diacritics are always written, so each cell stands alone;
   - the diacritics table is copied from kitty's `rowcolumn-diacritics.txt` (or Ghostty's `src/terminal/kitty/graphics_unicode.zig`, in the clone the build makes), not typed by hand; DECK-12's two constants move into it, and a test checks its 297 entries and first and last values;
   - a wide character cut by the rectangle's edge has its other half drawn as a blank cell;
   - placements are drawn in list order, so a later one covers an earlier one; a placement whose outer image is not ready draws nothing until it is.
7. **Program placeholders, for now.** `Virtual` placements are ignored until sub-spec 07 (their pixels are received and kept). A mirror cell whose text starts with U+10EEEE (a program-made placeholder) is drawn as a blank cell with the cell's background, images on or off, so a program's image id never reaches the outer terminal, where it could match one of the client's own ids.
8. **Images off.** Apart from requirement 7, a client with images off draws exactly as today.
9. **Selection and copy.** Text selection (DECK-15) is built from the mirror's lines, so selecting over a placed image copies the text under it, and selected image cells show the image without the highlight.
10. **Cleanup.** On exit and detach, the client deletes every outer image it sent for programs (as it does for logos), before leaving the alternate screen. A cell size change makes new outer images (their key includes the cell size); the old ones go by requirement 5.
11. **Docs.** `docs/features/images.md`: which programs and protocols work, in which outer terminals, what shows elsewhere, the limits (64 MiB per terminal screen, no animation, no sub-cell offsets, images cover text, not kept across a daemon restart, images cut at 297 rows or columns and not drawn past 64 MiB, a second client with another cell size sees sizes computed for the first, the outer terminal's own image quota may evict images silently, programs that probe for the protocol show a blank area instead of their text fallback in a client without images), and how to check (`kitten icat`). Linked from `docs/features/README.md`. architecture.md#client describes program images next to the agent logos.

## Screen

A two-terminal panel in Ghostty; the left terminal ran `kitten icat cat.png` (the image is 30×10 cells). `▒` stands for the image's pixels, drawn by the outer terminal in placeholder cells:

```text
╭─ zsh ──────────────────────────────────╮╭─ zsh ──────────────────╮
│$ kitten icat cat.png                   ││$                       │
│▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒          ││                        │
│▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒          ││                        │
│▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒          ││                        │
│  … 10 rows of image …                  ││                        │
│$ █                                     ││                        │
╰────────────────────────────────────────╯╰────────────────────────╯
```

The same terminal after 5 more lines of output: the image's top 4 rows are above the viewport, the rest shows from the first row.

```text
╭─ zsh ──────────────────────────────────╮
│▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒          │   image rows 4–9
│▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒          │
│$ ls                                    │
```

A dialog or menu over the image hides the covered cells; the image shows again when it closes. In a client without images the image's area shows the cells underneath (usually blank).

## Acceptance criteria

- [ ] In the e2e harness with `TERM=xterm-kitty`, `COLORTERM=truecolor` and a PTY pixel size, a program in a panel that transmits and places a small RGBA image makes the client write an `a=t` command with the canvas size and an `a=p,U=1` placement with the grid size, and the screen shows U+10EEEE in the placement's cells with the expected diacritics and foreground id.
- [ ] The placement is clipped at the terminal's border in a two-terminal panel; the border cells are not overwritten.
- [ ] Output that scrolls the terminal moves the placeholder cells with the text; scrolling the image out of view removes them and deletes the outer image.
- [ ] `a=d` from the program removes the placeholder cells and makes the client write `a=d,d=I` for its outer id.
- [ ] A placement with `c=60000,r=60000` of a 1×1 image draws at most 297×297 cells or nothing, and the client's memory stays bounded.
- [ ] Without the kitty environment (images off), the same program output leaves no `ESC _G` from program images in the client's output.
- [ ] A program-written U+10EEEE cell draws as a blank cell, with images on and off.
- [ ] A dialog opened over the image covers its cells; closing it shows them again.
- [ ] Detaching writes a delete for every program image sent.
- [ ] `docs/features/images.md` exists and is linked; architecture.md#client is updated.

## Tests

- `deck-tui` unit tests: detection (environment, truecolor, no cell size); assembling pieces, restarting at offset 0, discarding on a gap and on an older generation; the shared id allocator and id reuse; building a canvas (crop to source, scale, pad, cut at 297 cells, refuse past 64 MiB, a huge `c`/`r`); outer image creation and deletion after a draw and on a panel switch; drawing into a `TestBackend` buffer (cells, diacritics, colours, clipping at the area's edges, negative `row`/`col`, list order, a cut wide character, the selection highlight under an image); the diacritics table.
- `deck` e2e tests (`crates/deck/tests/`): the scenarios in the acceptance criteria, with a test program that writes the kitty escape sequences directly (`printf` of a fixed 2×2 RGBA image in base64), so no image tool is needed. The harness gains a way to set the PTY's pixel size and the terminal-detection variables for one test (it clears them by default since DECK-12), and whatever access to a cell's full symbol and colours the assertions need. Captures: the image in a single terminal, clipped in a two-terminal panel, scrolled half out.

## Notes

- Images are drawn only for the terminals of the active panel, since frames come only for those.
- No code in `deck-tui` sets an underline colour, and this design needs none: each placed image has its own outer image with one virtual placement.
