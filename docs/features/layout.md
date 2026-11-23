# Layout

A panel shows one or more terminals in an automatic grid. You add, close and focus terminals with the [keys](keys.md#bindings); deck decides where each one goes and how big its shell is.

## The grid

Terminals keep the order they were created in, and a new one is appended last.

- One terminal fills the panel area and has no border; its title is in the status bar.
- Two or more terminals each get a border and a title, `<number> · <title>` (the number is the terminal's place in the panel; the title is the program's, else the program name).
- An even count is a grid with as few more columns than rows as possible (2: side by side, 4: 2×2, 6: 3×2).
- An odd count above 1 gives terminal 1 the left half at full height, and the others share the right half as a grid (3: two stacked, 5: 2×2).

```text
2                3                4                5
┌──────┬──────┐  ┌──────┬──────┐  ┌──────┬──────┐  ┌──────┬───┬───┐
│      │      │  │      │  2   │  │  1   │  2   │  │      │ 2 │ 3 │
│  1   │  2   │  │  1   ├──────┤  ├──────┼──────┤  │  1   ├───┼───┤
│      │      │  │      │  3   │  │  3   │  4   │  │      │ 4 │ 5 │
└──────┴──────┘  └──────┴──────┘  └──────┴──────┘  └──────┴───┴───┘
```

The focused terminal's border is in the theme's `border_focused` colour and its title in `accent`; the others use `border` and `title`. The border shape is `border_style` ([config](config.md)). Only the focused terminal shows a cursor, and keys go to it.

## Adding and closing

- `new_terminal` (`t` after the prefix) starts a shell in the space's root at the end of the panel and focuses it. It is refused with `not enough room for another terminal` when the default sizes for one more terminal would leave any terminal under 10×3 cells inside its border in the current window.
- `close_terminal` (`x`) asks `Close terminal <number>? (y/n)`, then closes the focused terminal. A terminal whose shell exits goes the same way.
- The others reflow to the new count. When the focused terminal goes, focus moves to the terminal that took its place, else the previous one.
- When the last terminal goes, the panel closes.

The panel's sizes are kept as proportions. Adding or closing a terminal keeps custom proportions where it can (the default sizes always follow the rule above; a 60/40 split of two terminals is still 60% for the big one with a third). When a window is too small for the stored sizes, deck draws and sizes a squeezed copy, and the stored sizes come back when the window grows.

## Focus

- `focus_left`, `focus_right`, `focus_up`, `focus_down` (`Left`/`h`, `Right`/`l`, `Up`/`k`, `Down`/`j` after the prefix) move focus to the terminal beside the focused one in that direction: the nearest one whose rows (or columns) overlap. A tie goes to the topmost (left or right) or the leftmost (up or down). There is no wrap-around and no memory of the previous focus: from terminal 1 of an odd layout, `Right` always goes to terminal 2.
- Clicking in an unfocused terminal focuses it. That click is not sent to the program; later clicks are.
- The wheel scrolls the terminal under the pointer, focused or not, and does not change focus.

## Resizing

- `resize_left`, `resize_right`, `resize_up`, `resize_down` (`H`, `L`, `K`, `J` after the prefix, so `Shift+h` resizes where `h` focuses) move one border of the focused terminal by 5% of the space it divides (at least one cell). The border is the terminal's own edge on the side of the key when it has one, so the terminal grows; otherwise it is the opposite edge, and the terminal shrinks. In the 3-terminal layout, `L` on terminal 1 moves the main border right and grows it, `L` on terminal 2 moves the same border right and shrinks terminal 2, and `J` on terminal 3 moves the row border down and shrinks it.
- A border stops where a terminal would get under 10×3 cells inside its border. Nothing is sent when a key has no room; with a single terminal the keys do nothing.
- **Repeat**: for one second after a resize key, the four resize keys and `=` act without the prefix, and each one extends the second. The status bar shows a `RESIZE` badge (the `warning` role, like `PREFIX`) and the hints `H J K L resize  = reset`. Any other key, a paste or a focus event ends the repeat and is handled normally, so a key held at the limit never reaches the terminal.
- `reset_layout` (`=`) restores the default sizes for the panel's terminal count.
- **Dragging**: press the left button on a border between two terminals, move, and release. The border follows the pointer and is drawn in the `accent` colour while you drag; the same limits apply. The press, the motion and the release are never sent to a program, even one that asked for the mouse. A panel with one terminal has no border to drag.

The sizes are stored per panel. A resize key or a drag asks the daemon to store the new proportions; if the panel changed meanwhile (a terminal was added or exited) the request is refused and the next view puts things right, with no message.

## Sizes and several clients

The daemon sizes each shell from its place in the grid, for the window of the client that was active last. Every client draws that layout, cropped to its own window when it is smaller, so all of them see the same focus and grid.
