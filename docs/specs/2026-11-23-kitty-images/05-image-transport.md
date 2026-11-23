---
type: foundation
status: todo
---

# 05 — Sending images to clients

## Scope

The daemon tells image-capable clients what is placed in each terminal and sends them the pixels, once per image, ahead of the frames that need them, pacing big images so a slow client never pins unbounded memory and never misses an image. Clients that do not ask for images get nothing new. Nothing is drawn yet (sub-spec 06). Owning crates: `deck-core` (protocol), `deck-daemon` (sending, tracking, pacing).

## Requirements

1. **Protocol** (`deck-core::protocol`, the version sub-spec 04 set on this branch; `Role::Attach`'s `images: bool` came with sub-spec 04):
   - `Frame` gets `images: Vec<Placement>` (sub-spec 04's type): the terminal's placements, for an image-capable client, in every frame (full or not; the list is always complete) and empty for other clients.
   - `ServerMsg::Image { terminal, screen, image: u32, generation: u64, width: u32, height: u32, format: ImageFormat, offset: u32, total: u32, data: Vec<u8> }`: one piece of an image's pixels, `data` at most 4 MiB, `offset` its byte offset, `total` the whole image's byte length. Pieces of one image are sent in order.
   - `ServerMsg::ImageGone { terminal, screen, image: u32, generation: u64 }`: the client can forget that image.
   - Round-trip tests for every new message and field.
2. **Frames when only images changed**: when `Emu` reports a graphics change (sub-spec 04, requirement 9), the terminal counts as damaged for its next frame even when no cell changed (a frame with the new list and no changed rows is fine).
3. **Sent tracking**: per attached client, the daemon remembers which (terminal, screen, image, generation) it queued in full. Before a `Frame` for that client lists a placement whose image it has not queued, the daemon queues the image's pieces ahead of the frame (subject to requirement 5). If `Emu::image` returns `None` or a different generation (the image changed between building the frame and copying it), that placement is left out of that frame; the next frame lists it again.
4. **Forgetting**: the daemon checks for removal only while the image's screen is active, so switching to the alternate screen and back (open vim, quit) neither forgets nor re-sends primary-screen images. An image the client was sent that left its screen's storage (deleted, replaced by a new generation, evicted by the limit) gets `ImageGone`, queued after the frame that stops listing it (so a re-transmitted image never blinks blank), and is forgotten for that client. `ImageGone` is a never-dropped message. When a terminal closes, its images are forgotten without `ImageGone` (the client drops them with the terminal). On detach, the client's whole set is dropped. Clients attach to one session per connection (a session switch reconnects), so there is no other way for a terminal to leave a client's view.
5. **Pacing**: image pieces are a new class in the client queue (`queue.rs`), never dropped, bounded by bytes:
   - An image is queued only if the client's pending piece bytes plus the image's size stay within 64 MiB, or if no pieces are pending (so one image up to the storage limit always gets through).
   - An image that does not fit is not queued; its placements are left out of that frame, and the client is owed a new frame of that terminal when its pending pieces drain (through the queue's drain hook, as `Next::Resync` works in `conn.rs`), at which point requirement 3 runs again.
   - While the client is marked for a resync (`need_resync`), pending pieces stay queued and are sent; the full frames of the resync follow the same rule.
   - The frame-count and message-count rules for the other classes do not change, and a queued frame never goes out before the pieces queued ahead of it.
6. **Image-off clients**: a client with `images: false` never gets `Image`, `ImageGone` or a non-empty `images` list, and the daemon copies no pixels for it.

## Acceptance criteria

- [ ] Every new message and field round-trips.
- [ ] A test client with `images: true`, attached to a terminal where a program transmitted and placed a 20×10-pixel RGBA image, gets the image's pieces before the first frame listing the placement, with the cell position and grid size of sub-spec 04, and `ImageGone` after the frame that follows the program's delete (`a=d`).
- [ ] An image larger than 4 MiB arrives in pieces whose `offset`s cover `total` exactly, in order.
- [ ] Placing an image over blank cells sends a frame even though no cell changed.
- [ ] Re-transmitting an image with the same id sends the new generation's pieces and the frame listing it before the old generation's `ImageGone`.
- [ ] Entering and leaving the alternate screen sends no `ImageGone` and no second copy of a primary-screen image; an image placed on the alternate screen is sent with the alternate screen.
- [ ] A client that does not read, while a program sends images, never has more than 64 MiB of pieces pending (plus one image when none were pending).
- [ ] After an image was held back, an idle terminal (no new output) still gets its image and a frame listing it once the client reads again.
- [ ] Attaching to a panel whose visible images total more than 64 MiB (two terminals with a 48 MiB image each) delivers both images and frames listing both.
- [ ] A client with `images: false` receives no image messages and empty `images` lists.
- [ ] architecture.md#protocol describes the new fields, messages and queue class.

## Tests

- `deck-core`: round trips.
- `deck-daemon` queue unit tests: the byte budget, the always-one rule, the drain hook, order against frames.
- `deck-daemon` integration tests with a test client: the scenarios in the acceptance criteria.
