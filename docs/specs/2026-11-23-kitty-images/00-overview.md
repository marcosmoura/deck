---
branch: feat/kitty-images
backlog: DECK-19
---

# Images in panels (libghostty-vt and kitty graphics)

## Goal

A program running in a deck panel can show images with the kitty graphics protocol (`kitten icat`, yazi previews, chafa, timg, …), and they appear in the panel when deck runs in Ghostty or kitty. To get there, the daemon's terminal emulator moves from `alacritty_terminal` to Ghostty's own emulator, `libghostty-vt`, which parses, stores and places kitty images; the daemon passes the images to the client, which draws them in the outer terminal.

Asked by the user on 23 Nov 2026: "deck is unable to display images inside the terminal". The user first chose the `libghostty-vt` crate from `uzaaft/libghostty-rs`; when its pinned Ghostty turned out to need Zig 0.15.2, which cannot link on macOS 27, the user switched to the `ghostty-vt` crate (see Notes).

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

- **Emulator: Ghostty's `libghostty-vt`, through the `ghostty-vt` crate, pinned to `=0.9.0`** (crates.io, from `prabirshrestha/gpui-cn`; a fork of `libghostty-rs` with the same API shape) with its `kitty-graphics` and `png` features. Its `ghostty-vt-sys` links Ghostty's official, versioned `libghostty-vt` 1.0.1 release, every file checked against a sha256 in its `GHOSTTY.lock`: on macOS the release's prebuilt static library (no Zig), on Linux a Zig 0.17 build of the release's source with its dependency tarball (Zig fetches nothing). Zig 0.17.1 joins `.mise.toml`, so local builds and CI get it through `mise install` and `mise-action`; the release workflow, which installs only Rust today (`install_args: rust`), installs Zig too. The API is safe Rust; deck adds no `unsafe`. A panic inside the Zig library aborts the daemon (Rust's `catch_unwind` does not cover it). The exact pin keeps the third-party layer fixed; moving back to upstream `libghostty-rs` once it links a `libghostty-vt` 1.x release is a small change, since the API shape is the same.
- **PNG decoding is the crate's `RustPngDecoder`** (its `png` feature). The decoder is stored per thread by `libghostty-vt`, so it is installed in `Emu::new`, on whichever thread creates the emulator; installs share one lock because each also sets a process-global C option.
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
  - Direct transmission (`t=d`, the image data inside the program's output) goes to the emulator as it is. The file and temporary-file mediums (`t=f`, `t=t`) are off in the emulator, because Ghostty opens their path without `O_NONBLOCK` and a FIFO would block its thread; deck reads such files itself on the PTY reader thread (resolved path, not under `/proc`, `/sys` or `/dev` other than `/dev/shm`, `O_NONBLOCK` and `O_NOCTTY`, regular files only, at most 64 MiB, 128 MiB per second per terminal) and gives the emulator direct transmission in bounded pieces. Shared memory (`t=s`) is refused, so `kitten icat` falls back to direct transmission.
- **Protocol version**: 21: one above DECK-20's 20, which landed on `main` first. Bumped once in sub-spec 04; later sub-specs add to it on the same unmerged branch.

## Sub-specs

| Sub-spec                         | Type       | Crate                                     | Summary                                                                                                                                              | Depends | Status |
| -------------------------------- | ---------- | ----------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- | ------- | ------ |
| [01](01-ghostty-toolchain.md)    | foundation | repo (mise, deps, workflows), deck-daemon | `ghostty-vt` and Zig in the build, release and CI workflows, a smoke test, first-build docs                                                          | —       | done   |
| [02](02-emulator-thread.md)      | foundation | deck-daemon                               | One emulator thread per terminal behind `with_emu`, on today's alacritty `Emu`; today's query replies pinned in tests                                | 01      | done   |
| [03](03-libghostty-emulator.md)  | foundation | deck-daemon                               | `libghostty-vt` replaces `alacritty_terminal` behind the same `Emu` API; replies, colours, scrollback, sync output kept                              | 02      | done   |
| [04](04-daemon-graphics.md)      | foundation | deck-core, deck-daemon, deck-tui, deck    | Cell pixel size from the client to the emulator and the PTY; kitty graphics on with the crate's PNG decoder; `Emu` lists placements and image pixels | 03      | done   |
| [05](05-image-transport.md)      | foundation | deck-core, deck-daemon                    | `Frame.images`, `Image` and `ImageGone` messages, per-client sent tracking, pacing, both screens                                                     | 04      | done   |
| [06](06-client-images.md)        | shippable  | deck-tui, deck (tests)                    | Detection, building and sending outer images, placeholder cells for placed images, cleanup, e2e tests, docs: `kitten icat` works                     | 05      | done   |
| [07](07-program-placeholders.md) | shippable  | deck-tui, deck (tests)                    | Program-made Unicode placeholders (virtual placements) drawn with the client's ids: yazi and `icat --unicode-placeholder` work                       | 06      | done   |

## Affected docs

- `docs/architecture.md`: the emulator (`libghostty-vt`, the emulator thread, the first build), the protocol (cell size, the `Attach` image capability, `Frame.images`, `ServerMsg::Image` and `ImageGone`, the image queue class), the client (drawing program images), dependencies (`ghostty-vt` and `ghostty-vt-sys`, the `png` crate through it, `alacritty_terminal` now `deck (tests)` only), tools (Zig), and a decision row.
- `docs/features/`: a new `images.md` (what works, where, limits), linked from `docs/features/README.md`.
- `AGENTS.md`: the toolchain line names Zig.

Reviewed by spec-reviewer on 23 Nov 2026.

Approved by the user on 23 Nov 2026.

## Notes

- 23 Nov 2026, during sub-spec 01: `libghostty-vt` 0.2.2 pins a Ghostty commit that requires exactly Zig 0.15.x, and Zig 0.15.2 cannot link anything on macOS 27.3 (the user's machine); Zig 0.16 and 0.17 can. The user compared the `ghostty-vt` fork with writing deck's own bindings and chose the fork, pinned exactly. Verified before the switch: a probe crate on `ghostty-vt` 0.9.0 builds and runs on that machine, and its `RustPngDecoder` can be constructed. Spec changes: the crate and its features, Zig 0.17.1, no build-mode setting (the fork has none; macOS links the prebuilt release), the crate's PNG decoder instead of deck's own, and query replies through the crate's terminal options and events instead of callbacks.
- 23 Nov 2026, after sub-spec 01: instead of the draft pull request, the lead ran the Linux path in an Ubuntu 22.04 arm64 VM (Apple `container`): `libghostty-vt` 1.0.1 built from source with Zig 0.17.1, and format, Clippy and all 1144 tests passed as a non-root user. The user chose to start sub-spec 02 before the draft pull request; the x86_64 Linux and Intel macOS release builds stay unchecked until it is opened (sub-spec 01's last criterion).
- 24 Nov 2026, review of sub-spec 04: a kitty `t=f` (or `t=t`, or on Linux `t=s`) path to a FIFO blocks Ghostty's `open()`, and the daemon waits on emulator threads under its state lock, so one program could freeze every session. The file, temporary-file and shared-memory mediums are turned off; only direct transmission is accepted (Limits and sub-spec 04 changed).
- 24 Nov 2026, after the user tried Claude Code in deck: its images stayed off because it allows kitty images only for a terminal named ghostty or kitty, and deck panels give no such name (deck drops XTVERSION). The user chose that deck answers XTVERSION with the outer terminal's name when the followed client can show images (sub-spec 07, requirement 5), as herdr appears to do.
- 24 Nov 2026, Claude Code capture: named `ghostty`, Claude Code sends its images as a file path (`a=T,U=1,q=1,f=100,t=f`) without probing, so refusing `t=f` loses them. The daemon now reads `t=f` and `t=t` files itself before the emulator sees the command (opened with `O_NONBLOCK`, regular files only, at most 64 MiB, `t=t` files deleted afterwards when they are temp files named like kitty's), and feeds them to the emulator as direct data. The emulator's own file mediums stay off, so a path is never opened inside Ghostty; `t=s` stays refused (sub-spec 04, requirement 4).
