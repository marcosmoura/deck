# Images

Read when you need how pictures from programs show in a panel.

A program in a panel can show an image with the kitty graphics protocol, and deck draws it in the panel when it runs in **Ghostty** or **kitty**. The picture is clipped to its terminal, moves with the text, is hidden under dialogs and menus, and goes when the program deletes it.

## What works

- Programs: `kitten icat picture.png`, chafa and timg in kitty mode, and any program that writes the protocol (`a=T` or `a=t` with `a=p`, with a position). The first frame of an animation shows.
- Transmission: direct (`t=d`, the image inside the program's output), and files (`t=f`) and temporary files (`t=t`): deck reads the file itself (regular files only, at most 64 MiB, honouring `S` and `O`) and gives the emulator its data, so programs that send a path, such as Claude Code, work. A `t=t` file is deleted afterwards when it is in a temp directory and its name contains `tty-graphics-protocol`. A path that is not a readable regular file (a FIFO, a device, a missing file) gets an error answer. Shared memory (`t=s`) is refused, so `kitten icat` falls back to another medium. PNG, RGB and RGBA data are accepted.
- Outer terminals: Ghostty and kitty, in a client that draws in truecolor and knows its cell size in pixels. deck decides as it does for its icon images: the `Image` tier, found at start by asking the terminal (XTVERSION and a kitty graphics query), never behind tmux, Zellij or screen, or in a `deck` started inside a deck panel. Nothing in `[theme]` turns it off.
- Programs that place their own Unicode placeholders (yazi, `icat --unicode-placeholder`, image.nvim): the program makes a virtual placement (`a=p,U=1` or `a=T,U=1`) and writes U+10EEEE cells whose foreground colour is the image id, with row, column and (for a 32-bit id) high-byte diacritics. deck sends the image's own pixels to the outer terminal with a virtual placement of the program's size, and draws those cells with its own id, so they show the picture. Missing diacritics are inherited from the cell to the left, as in kitty. Cells of an unknown image, past the 297th row or column, or when images are off, show as blanks.

## Claude Code and other programs that check the terminal

Claude Code shows its image thumbnails (pasted images) only in a terminal that names itself `ghostty` or `kitty`, which it learns from XTVERSION (`CSI > q`) or `TERM`/`TERM_PROGRAM`. A deck panel says `TERM=xterm-256color` and `TERM_PROGRAM=deck`, which Claude Code does not know, so deck answers XTVERSION itself: at attach the client asks its outer terminal for its name (together with the colour queries, with the same timeout) and, when the client can show images, the panels of its session answer `CSI > q` with that name (`ghostty 1.2.0`, for example). It follows the client the terminals follow; with a client that shows no images, or an outer terminal that gives no name, XTVERSION stays unanswered.

## What shows elsewhere

In any other outer terminal (Alacritty, WezTerm, tmux, ...) the image's cells show what the program left under them, usually blanks. Programs that probe for the protocol (`kitten icat`, yazi, timg) are told it works, so they draw an image and not their text fallback: in a client without images they show a blank area.

## Limits

- 64 MiB of image data per screen of each terminal; no animation; no sub-cell offsets (an image starts at its cell).
- Images cover text, and are not kept across a daemon restart.
- An image shows at most 297 columns by 297 rows; a placement past that is cut, and one whose pixels would pass 64 MiB (`cols x rows` cells at the client's cell size) is not drawn.
- Many large placements are drawn a few at a time: the client scales at most 64 MiB of pixels per draw, the rest on the next draws, and keeps at most 128 MiB of scaled images in the outer terminal. A placement past that stays undrawn until others leave the screen.
- A second client with a different cell size sees sizes computed for the first one.
- Very large placements (hundreds of cells at large cell sizes) cost a lot of CPU and bytes when first drawn, and again after a cell size change.
- The outer terminal's own image quota may evict images silently.
- Only the terminals of the active panel show images.
- A program's cells use the image's first virtual placement: deck does not read the placement id (which a program puts in the underline colour of its placeholder cells), so several placements of one image told apart by placement id are not told apart.

## Selection and copy

Selecting text over an image copies the text under it; the image cells are not highlighted. A program's placeholder characters are never copied: they become spaces.

## How to check

In Ghostty or kitty, run `deck`, then `kitten icat picture.png` in a panel. Print more lines to see the image scroll with the text; open the help (`prefix ?`) over it to see it hidden.
