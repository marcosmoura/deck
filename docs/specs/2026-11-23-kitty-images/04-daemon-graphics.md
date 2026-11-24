---
type: foundation
status: done
---

# 04 — Cell size and kitty graphics in the daemon

## Scope

Programs in a panel learn the real cell size in pixels and can use the kitty graphics protocol: their support queries get a positive answer, and the emulator stores and places their images, PNG included. `Emu` can list what is placed in its viewport and copy an image's pixels, for sub-spec 05 to send. Nothing is drawn yet. Owning crates: `deck-core` (protocol types), `deck-daemon` (cell size, graphics, PNG), `deck-tui` (measuring and reporting the cell size), `deck` (building `Role::Attach` in the control client and the tests).

## Requirements

1. **Protocol** (`deck-core::protocol`, `PROTOCOL_VERSION` 21, one above `main`'s 20): `Role::Attach` gets `cell: Option<CellPx>` and `images: bool` (the client's capability, used by sub-spec 05; every client sends `false` until sub-spec 06), added together so the places that build `Role::Attach` (`crates/deck-tui/src/lib.rs`, `crates/deck/src/control.rs`, the tests) change once; `ClientMsg::Viewport` and `ClientMsg::ReloadViewport` get `cell: Option<CellPx>`. `CellPx { width: u16, height: u16 }` is the outer terminal's cell size in pixels, `None` when unknown. Round-trip tests.
2. **Client** (`deck-tui`): the client measures the cell size with crossterm's `window_size()` (pixel width divided by columns, pixel height by rows; `None` when the pixel size is 0) and sends it in `Attach` and in every `Viewport`/`ReloadViewport`. The client's `SIGWINCH` path forwards only a changed size today; it also sends a `Viewport` when only the cell size changed (font zoom with the same columns and rows).
3. **Cell size in the daemon**: the daemon keeps the cell size of the client the terminals follow (the one whose `Viewport` sized the PTYs, as for the size) and, when it is known:
   - passes it to every emulator of that session on resize (`Terminal::resize(cols, rows, cell_width_px, cell_height_px)`), so image grid sizes and `CSI 14/16 t` answers use it;
   - sets the PTY's pixel size to `cols × width` by `rows × height` (`PtySize::pixel_width`, `pixel_height`), in `resize_pty` and `sync_pty`, so programs that read `TIOCGWINSZ` see it;
   - re-applies both when only the cell size changed.
     Unknown, the emulator gets 8×16 and the PTY 0×0 pixels, as today. `Emu` drops the crate's own `CSI 16 t` answer today (sub-spec 03 keeps that query unanswered, as alacritty did); from this sub-spec it lets the answer through when the cell size is known and keeps dropping it while it is unknown. A terminal created later in the session starts with the known size.
4. **Kitty graphics on**: every emulator gets a 64 MiB image storage limit (`set_kitty_image_storage_limit`, both screens), the file, temporary-file and shared-memory mediums explicitly off (only direct transmission, `t=d`; see the overview's Limits for why). Added after the user tried Claude Code (overview Notes): before feeding output to the emulator, the daemon rewrites a `t=f` or `t=t` command into direct transmission by reading the file itself (on the PTY reader thread: resolved path, `O_NONBLOCK` and `O_NOCTTY`, regular files only, at most 64 MiB, honouring `S` and `O`; a `t=t` file is deleted afterwards when it is in a temp directory and its name contains `tty-graphics-protocol`). A path that fails these checks gets the command passed on unchanged, so the emulator answers it with an error. The PNG decoder is the crate's. A program's `a=q` query is answered by the emulator.
5. **PNG decoder**: the crate's `RustPngDecoder` (`png` feature, from sub-spec 01). `Emu::new` installs it (`set_png_decoder`) on the current thread before creating its terminal, holding one process-wide lock across the call, since each install also sets a process-global C option; so it works on emulator threads and in unit tests alike. Palette, grey and 16-bit PNGs must come out as RGB or RGBA; if the crate's decoder does not expand them, `Emu::image` expands them (requirement 8). A malformed PNG is rejected (the program gets the protocol's error answer), never a panic.
6. **New protocol types** (`deck-core::protocol`; names may differ from these where they would clash with `libghostty-vt`'s own types in `deck-daemon`):
   - `ImageScreen { Primary, Alternate }`.
   - `ImageFormat { Rgb, Rgba }` (3 or 4 bytes per pixel).
   - `SourceRect { x: u32, y: u32, width: u32, height: u32 }` in image pixels.
   - `PlacementKind`: `Placed { col: i32, row: i32 }`, the top-left cell relative to the viewport (`viewport_pos`), negative when the image starts above or left of it; or `Virtual`, a virtual placement (`U=1`) shown only where the program writes placeholder cells.
   - `Placement { screen: ImageScreen, image: u32, generation: u64, kind: PlacementKind, cols: u16, rows: u16, source: SourceRect, pixels: (u32, u32) }`: `image` is the program's image id, `generation` the image's generation (`Image::generation`), `cols × rows` the cells it covers (`grid_size`, saturated to `u16`), `source` the source rectangle (`source_rect`), `pixels` the rendered size in pixels (`pixel_size`).
   - `ImagePixels { generation: u64, width: u32, height: u32, format: ImageFormat, data: Vec<u8> }` stays in `deck-daemon` (it is not sent as is).
7. **Placements from `Emu`**: `Emu::placements() -> Vec<Placement>` lists every placement of the active screen that touches the viewport as shown (scrolled back or not), in drawing order (lower `z` first, then the iterator's order), copied while the graphics handles are valid. A `Virtual` placement is listed once per image (the first in iterator order) whether or not its placeholder cells are on screen.
8. **Pixels from `Emu`**: `Emu::image(screen, id) -> Option<ImagePixels>` copies an image of that screen if it is the active one, with grey formats expanded to `Rgb` or `Rgba`. An image whose pixels cannot be expressed so returns `None` and is logged once.
9. **Change detection**: `Emu` tells whether the graphics changed since it was last asked (storage generation, or a different placement list); sub-spec 05 uses it to send a frame when only images changed.

## Acceptance criteria

- [ ] `PROTOCOL_VERSION` is one above `main`'s; `cell` and `images` round-trip in `Attach`, `cell` in `Viewport` and `ReloadViewport`.
- [ ] With a client cell size of 9×18 and an 80×24 panel, `CSI 16 t` answers 18;9, `CSI 14 t` 432;720, and the PTY's `TIOCGWINSZ` pixel size is 720×432; without a cell size, today's answers and 0×0.
- [ ] A cell-size-only change reaches the emulator and the PTY.
- [ ] A kitty `a=q` query from a program gets an `OK` answer.
- [ ] A transmitted-and-placed RGBA image, a PNG, a PNG and a grey image each appear in `placements()` with the right cell position and grid size for a known cell size, and `image()` returns their pixels as RGB or RGBA.
- [ ] A transmission with `t=f`, `t=t` or `t=s` gets an error answer and stores nothing, including a `t=f` path to a FIFO, which must not block the emulator thread.
- [ ] A malformed PNG gets an error answer and the daemon keeps running.
- [ ] Scrolling the image half out of the viewport lists it with a negative `row`; scrolling the display back keeps it on the right row.
- [ ] A virtual placement is listed as `Virtual` with its `cols` and `rows`; an image placed on the alternate screen is listed with `Alternate`; a placement with `c=70000` lists `cols` as 65535, not wrapped.
- [ ] architecture.md (protocol fields, cell size, PNG decoder) is updated.

## Tests

- `deck-core`: round trips.
- `deck-tui`: the cell size from a window size, `None` for 0 pixels; a cell-size-only change produces a `Viewport`.
- `deck-daemon` unit tests on `Emu`: the image cases of the acceptance criteria; deletion (`a=d`) removes the placement; re-transmission changes the generation; the storage limit evicts an image; change detection.
- `deck-daemon` integration test: the cell size reaching the PTY and the size reports.

## Notes

- `libghostty-vt`'s `Graphics` and `Image` handles are valid only until the next mutating call; `placements()` and `image()` copy what they need first.
- Whether many `libghostty-vt` terminals on separate threads share any other global state is not known; the worker checks the crate and Ghostty's C API for process-global options besides the PNG decoder and reports any.
- Grid sizes use the followed client's cell size; another image-capable client with a different cell size gets the same `cols`, `rows` and `pixels` (a non-goal in the overview).
