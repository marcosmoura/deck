---
type: foundation
status: todo
---

# 02 — One emulator thread per terminal

## Scope

Each terminal's emulator moves onto a thread of its own, reached only through `TermHandle::with_emu`. The emulator is still `alacritty_terminal`; nothing changes for the user. This isolates the concurrency change before sub-spec 03 swaps in `libghostty-vt`, whose handles cannot cross threads. It also pins today's replies to terminal queries in tests, so sub-spec 03 can prove it keeps them. Owning crate: `deck-daemon`.

## Requirements

1. **The thread.** `TermHandle` holds no `Emu` in a `Mutex`. `terminal::start` spawns one thread per terminal, named `emu-<terminal id>`, which creates and owns the `Emu` and runs the closures sent to it, in order, until the terminal is released. If the thread cannot be spawned or the `Emu` cannot be created (sub-spec 03's constructor returns a `Result`), `start` fails the way a PTY that cannot open fails today.
2. **`with_emu(f)`** keeps its name and meaning: it runs `f` on the emulator and returns `Some(result)`, or `None` once the terminal was released. `f: FnOnce(&mut Emu) -> R + Send + 'static`, `R: Send + 'static`. It blocks the caller until the result is back. Call sites that pass borrowed data clone it (`feed(bytes)`, `find_line(text)`, and the rest).
   - A panic inside `f` is caught on the emulator thread (`catch_unwind`), logged with the terminal id, and that call returns `None`; the thread keeps serving.
   - A call from the emulator thread itself would deadlock; a `debug_assert!` checks for it.
3. **Resize.** `resize_pty` still resizes the PTY and the emulator as one step, so `sync_pty` never sees one without the other: the PTY resize runs inside the closure on the emulator thread, with the PTY master reached through an owned handle (an `Arc`) the closure can hold. `sync_pty` reads the emulator's size through `with_emu` too.
4. **Release.** `release` ends the thread after the closures already queued; later `with_emu` calls return `None` at once, without blocking. The thread is never joined from code that can run on the emulator thread itself (e.g. a `Drop` of the last handle reference).
5. **Replies pinned.** New `emu.rs` tests record the exact bytes today's emulator answers to (or that it gives no answer):
   - DA1 (`CSI c`) and DA2 (`CSI > c`);
   - DSR (`CSI 5 n`, `CSI 6 n`);
   - `CSI 14 t`, `CSI 16 t`, `CSI 18 t`;
   - DECRQM for a set and an unset private mode;
   - XTVERSION (`CSI > q`);
   - the kitty keyboard query `CSI ? u`, alone and after a push (`CSI > 1 u`); today there is no answer;
   - OSC 4 for a palette index the program never set.
     Sub-spec 03 must keep these bytes.

## Acceptance criteria

- [ ] `TermHandle` has no `Mutex<Option<Emu>>`; each live terminal has one `emu-<id>` thread, which ends after `release`.
- [ ] `with_emu` after `release` returns `None` without blocking; a panicking closure returns `None` and the next call works.
- [ ] Closures sent from several threads at once run one at a time, each call getting its own result.
- [ ] The reply tests of requirement 5 exist and pass.
- [ ] Every existing daemon and e2e test passes unchanged in what it asserts.
- [ ] architecture.md describes the emulator thread.

## Tests

- `deck-daemon` unit tests for the emulator thread: results in order; release ends the thread and later calls return `None`; a panic is contained; concurrent callers.
- `deck-daemon` `emu.rs`: the reply tests of requirement 5.
- The existing tests, run as they are.

## Notes

- The 14 `with_emu` call sites are in `server.rs`, `server/terminal_io.rs` and `server/waiting.rs`, plus `resize_pty` and `sync_pty` in `terminal.rs`.
