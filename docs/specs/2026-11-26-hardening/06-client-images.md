---
type: foundation
status: todo
---

# 06 — Client image memory (DECK-44, DECK-45)

## Scope

Two client fixes in `crates/deck-tui/src/placed.rs`, client only (overview, Decisions). Owning crate: `deck-tui`.

1. **Memory released (DECK-44).** The client keeps every received image's pixels with no cap; with 8 images of 16 MiB it reached 148.7 MiB on macOS and did not fall after those terminals closed, although the code drops their pixels: the cause is open.
2. **Images kept across a switch (DECK-45).** The client deletes the outer terminal's copies of images a draw did not use, so leaving a panel and coming back rebuilds, rescales and re-sends every image on it (3.1 MB for one image).

## Requirements

1. First confirm DECK-44's retained footprint shows in the VM; if it does not, take its before/after on macOS in a window the user names.
2. Cap what the client keeps: drop the pixels of images no live terminal shows; a terminal that comes back gets them re-sent.
3. Keep recently used outer images for a while within the client's image budget.

## Acceptance criteria

- [ ] The cause of the retained footprint is named; the `client_memory` scenario shows it falling after the image terminals close and staying under a stated cap.
- [ ] The `images` scenario shows no bytes re-sent on a switch away and back within the budget; images beyond the budget are still released.
- [ ] `docs/features/images.md` (the client's budgets) is updated.

## Tests

The `client_memory` and `images` scenarios; unit tests of the budget.
