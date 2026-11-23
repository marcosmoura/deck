---
branch: feat/kitty-images
backlog: DECK-19
---

# Images in panels (libghostty-vt and kitty graphics)

## Goal

A program running in a deck panel can show images with the kitty graphics protocol (`kitten icat`, yazi previews, chafa, timg, …), and they appear in the panel when deck runs in Ghostty or kitty. To get there, the daemon's terminal emulator moves from `alacritty_terminal` to Ghostty's own emulator, `libghostty-vt`, which parses, stores and places kitty images; the daemon passes the images to the client, which draws them in the outer terminal.

Asked by the user on 23 Nov 2026: "deck is unable to display images inside the terminal", choosing the `libghostty-vt` crate from `uzaaft/libghostty-rs` over the `ghostty-vt` fork and over keeping `alacritty_terminal` with a hand-written APC filter.

## Builds on

- [DECK-01 core daemon and terminal](../2026-11-22-core-daemon-terminal/00-overview.md): the emulator, frames, damage, the attach protocol, the client queue.
- [DECK-12 agent logos](../2026-11-23-agent-logos/00-overview.md): kitty graphics detection in the client, Unicode placeholder cells, image ids carried in the foreground colour, cleanup on exit (`deck-tui/src/graphics.rs`).
- [DECK-15 mouse text selection](../../backlog.md#deck-15): selection is built from the mirror's lines.

## Non-goals

- Sixel, iTerm2 inline images (OSC 1337 `File=`), and any image protocol other than kitty's.
- Images in outer terminals other than Ghostty and kitty, inside tmux or Zellij, or in a `deck` started inside a deck panel (the client's detection from DECK-12 decides; those clients show the cells under the image).
- Kitty animation frames (`a=f`, `a=a`, `a=c`): the first frame shows.
- Images surviving a daemon restart (DECK-04 persistence does not store them) and images in `deck capture` output.
- Text under an image that sits below the text layer (`z < 0`): the image covers the cell.
- Sub-cell pixel offsets of a placement (`X`, `Y` keys): the image starts at its cell.
- Several virtual placements of one image told apart by placement id: deck's frames carry no underline colour, so a program's placeholder cells use the image's first virtual placement.
- The kitty keyboard protocol. `libghostty-vt` answers its query on its own; deck drops that answer, so programs keep seeing no support, as today.
- Hiding image support from programs when no attached client can show images. From sub-spec 04 on, the emulator answers a kitty `a=q` query with OK whatever the clients are, so programs that probe (yazi, timg, `kitten icat`) use the protocol and show a blank area in a client without images (Alacritty, WezTerm, tmux) instead of their text fallback. `images.md` says so.
- Exact image sizes for a second image-capable client whose cell size differs from the client the terminals follow: grid sizes are computed with the followed client's cell size.
- A config key for image memory or for turning images off (fixed limits; a later task can add keys).

## Decisions

- **Emulator: `libghostty-vt` 0.2 (crates.io, `uzaaft/libghostty-rs`)** with its `kitty-graphics` feature. It builds Ghostty's `libghostty-vt` from source with Zig 0.15.2 at a commit pinned by `libghostty-vt-sys` (its build script clones `ghostty-org/ghostty` into Cargo's build directory, and `zig build` fetches Ghostty's Zig packages). Zig joins `.mise.toml`, so local builds and CI get it through `mise install` and `mise-action`; the release workflow, which installs only Rust today (`install_args: rust`), installs Zig too. The API is safe Rust; deck adds no `unsafe`. A panic inside the Zig library aborts the daemon (Rust's `catch_unwind` does not cover it).
- **PNG decoding is deck's own `DecodePng`**, on the `png` crate (a direct dependency of `deck-daemon`), because the crate's `RustPngDecoder` cannot be constructed and its decode is broken in 0.2.2. The decoder is stored per thread by `libghostty-vt`, so it is installed in `Emu::new`, on whichever thread creates the emulator; installs share one lock because each also sets a process-global C option.
- **One thread per emulator.** `libghostty-vt` handles are `!Send + !Sync`. Each terminal's emulator lives on its own thread for the terminal's life; `TermHandle::with_emu(f)` keeps its shape, sends `f` to that thread and waits for the result (the closure must now be `Send + 'static`, and so must its result). This lands first, on today's alacritty `Emu`, so the concurrency change is tested on its own.
- **Toolchain first.** Whether Ghostty builds with Zig on every target is the riskiest unknown, so the dependency and Zig land first, in their own small sub-spec, and the user checks CI (on a draft pull request) and the four release targets (a build-only run of the release workflow on pull requests that touch the build) before any other work.
- **Behaviour stays the same through the swap**: the same `Frame`s, cursor, modes, scrollback, OSC 52, title, colour answers, query replies and capture output. Sub-spec 02 pins today's query replies in tests before the swap; sub-spec 03 keeps them.
- **`alacritty_terminal` stays only as the e2e harness's screen** (`deck` dev-dependency); the daemon drops it.
- **The client draws images as Unicode placeholder cells**, as DECK-12 does for logos: each image goes to the outer terminal once, with a virtual placement (`U=1`), and the cells it covers hold U+10EEEE with row and column diacritics, the foreground colour carrying the client's image id. The cells go through ratatui's buffer, so clipping at the terminal's edge, borders, dialogs, menus, the floating bar, scrolling and redraws need nothing special.
- **Placed images reach the outer terminal at their exact size.** Ghostty and kitty fit a virtual placement's whole image into its `c×r` box, keeping the aspect ratio, and ignore source rectangles. So for a placed (non-virtual) image the client builds the pixels the program asked for: the source rectangle, scaled to the placement's rendered pixel size when that differs, on a transparent canvas of exactly `cols × cell width` by `rows × cell height`. The canvas is bounded (below). A program's own virtual placements are passed through as they are, since the outer terminal treats them exactly as kitty would.
- **The daemon sends what is placed, not pixels on screen.** Each `Frame` carries the image placements that touch the terminal's viewport (in cells, possibly starting above or left of it), from `libghostty-vt`'s placement iterator. Image pixels go to a client in their own messages, once per (terminal, screen, image, generation), before the first frame that uses them, and only to clients that said they can show images. The primary and alternate screens have separate image stores and ids, so the screen is part of every image's key.
- **Image data is paced, not dropped.** A client's pending image data is bounded; an image that does not fit waits, its placements are left out of frames until it is sent, and the client is owed a new frame of that terminal once its queue drains.
- **Cell size in pixels** comes from the client (`TIOCGWINSZ` pixel size divided by cols and rows, through crossterm's `window_size`). The daemon gives it to the emulator and sets the PTY's pixel size (`ws_xpixel`, `ws_ypixel`), which programs read to size images. Without it (0 pixels), the daemon keeps today's 8×16 fallback for size reports and the client turns images off.
- **Limits:**
  - 64 MiB of kitty image storage per screen of each terminal, and `libghostty-vt`'s default APC size limit.
  - Scrollback memory: `libghostty-vt` budgets scrollback in bytes, sized to hold the configured lines at 250 columns (sub-spec 03 measures it for the default 10,000 lines; `alacritty_terminal` uses about 19 MB at 80 columns today).
  - Image messages of at most 4 MiB of pixels each (an image is split across several); at most 64 MiB of image data pending per client, except that one image is always let through when nothing is pending.
  - A placed image's canvas covers at most 297×297 cells (the diacritics table) and 64 MiB of RGBA; a larger placement is cut to that many cells, and one whose canvas would still pass 64 MiB is not drawn.
  - The file, temporary-file and shared-memory transmission mediums are on: the program runs as the same user as the daemon, and `kitten icat` uses them when it runs locally.
- **Protocol version**: one above `main`'s when the branch merges (20 today; DECK-20 also bumps it, and whichever lands second takes the next number). Bumped once in sub-spec 04; later sub-specs add to it on the same unmerged branch.

## Sub-specs

| Sub-spec                         | Type       | Crate                                     | Summary                                                                                                                                         | Depends | Status |
| -------------------------------- | ---------- | ----------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- | ------- | ------ |
| [01](01-ghostty-toolchain.md)    | foundation | repo (mise, deps, workflows), deck-daemon | `libghostty-vt` and Zig in the build, release and CI workflows, a smoke test, the build mode, first-build docs                                  | —       | todo   |
| [02](02-emulator-thread.md)      | foundation | deck-daemon                               | One emulator thread per terminal behind `with_emu`, on today's alacritty `Emu`; today's query replies pinned in tests                           | 01      | todo   |
| [03](03-libghostty-emulator.md)  | foundation | deck-daemon                               | `libghostty-vt` replaces `alacritty_terminal` behind the same `Emu` API; replies, colours, scrollback, sync output kept                         | 02      | todo   |
| [04](04-daemon-graphics.md)      | foundation | deck-core, deck-daemon, deck-tui, deck    | Cell pixel size from the client to the emulator and the PTY; kitty graphics on with deck's PNG decoder; `Emu` lists placements and image pixels | 03      | todo   |
| [05](05-image-transport.md)      | foundation | deck-core, deck-daemon                    | `Frame.images`, `Image` and `ImageGone` messages, per-client sent tracking, pacing, both screens                                                | 04      | todo   |
| [06](06-client-images.md)        | shippable  | deck-tui, deck (tests)                    | Detection, building and sending outer images, placeholder cells for placed images, cleanup, e2e tests, docs: `kitten icat` works                | 05      | todo   |
| [07](07-program-placeholders.md) | shippable  | deck-tui, deck (tests)                    | Program-made Unicode placeholders (virtual placements) drawn with the client's ids: yazi and `icat --unicode-placeholder` work                  | 06      | todo   |

## Affected docs

- `docs/architecture.md`: the emulator (`libghostty-vt`, the emulator thread, the first build), the protocol (cell size, the `Attach` image capability, `Frame.images`, `ServerMsg::Image` and `ImageGone`, the image queue class), the client (drawing program images), dependencies (`libghostty-vt`, `png`, `alacritty_terminal` now `deck (tests)` only), tools (Zig), and a decision row.
- `docs/features/`: a new `images.md` (what works, where, limits), linked from `docs/features/README.md`.
- `AGENTS.md`: the toolchain line names Zig.

Reviewed by spec-reviewer on 23 Nov 2026.

Approved by the user on 23 Nov 2026.
