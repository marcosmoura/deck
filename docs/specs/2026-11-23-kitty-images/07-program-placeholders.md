---
type: shippable
status: todo
---

# 07 — Program-made image placeholders

## Scope

Programs that draw images with Unicode placeholders themselves (`kitten icat --unicode-placeholder`, yazi in Ghostty, image.nvim and others that use virtual placements) show their images in a deck panel inside Ghostty or kitty, through the outer image machinery of sub-spec 06. Owning crates: `deck-tui`; `deck` for the e2e tests; docs.

## Requirements

1. **Outer images for virtual placements.** One outer image per (terminal, screen, image, generation) that has a `Virtual` placement in a frame: the image's pixels as they are (`f=24` or `f=32`), with one virtual placement using the program's `cols` and `rows` (`a=p,U=1,i=<id>,c=<cols>,r=<rows>,q=2`); the outer terminal fits it the way kitty would for the program. Ids, sending and deletion after a draw follow sub-spec 06.
2. **Decoding a program placeholder.** A cell of the mirror whose text starts with U+10EEEE is a program-made placeholder:
   - its image id is the foreground colour (an `Rgb` colour is the low 24 bits, an `Indexed` colour the low 8) plus the third diacritic, if any, as the high byte;
   - its row and column are its first two diacritics; a missing one is inherited from the placeholder cell to its left in the same mirror row (column + 1, same row and id, as kitty does), and a placeholder with neither and no placeholder to its left is row 0, column 0.
3. **Drawing.** The client maps (terminal, active screen, id) to the newest generation with a `Virtual` placement in that terminal's frame and draws the cell with the outer id as foreground (RGB) and explicit row and column diacritics, never a third one. Rows or columns past the table's last entry (296) draw blank. A placeholder whose image is unknown or not ready, and every placeholder when images are off, stays the blank cell of sub-spec 06 (requirement 7).
4. **Copy.** Placeholder characters are never copied by text selection; they become spaces.
5. **Docs.** `docs/features/images.md` adds program placeholders and the limit of one virtual placement per image.

## Acceptance criteria

- [ ] A program that transmits an image, makes a virtual placement and writes U+10EEEE cells makes the client send the image with an `a=p,U=1` placement of the program's size, and the screen shows the cells with the client's outer id and explicit diacritics.
- [ ] A 32-bit id with a third diacritic, and cells without diacritics after the first, are decoded and drawn correctly.
- [ ] Without the kitty environment, program placeholders draw as blanks and no `ESC _G` from program images is written.
- [ ] Selecting over program placeholders copies spaces.
- [ ] `docs/features/images.md` is updated.

## Tests

- `deck-tui` unit tests: placeholder decoding (24-bit, 8-bit indexed, third diacritic, inherited row and column, no left neighbour), mapping to the newest generation, unknown image blank, images-off blank, copy as spaces.
- `deck` e2e test: a test program writing the virtual placement and placeholder cells with `printf`; a capture of the result.
