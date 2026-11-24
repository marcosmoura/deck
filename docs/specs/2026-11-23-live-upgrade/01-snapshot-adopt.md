---
type: foundation
status: done
---

# 01 — Snapshot, adopted PTYs and pausable readers

## Scope

The three building blocks the handoff in [02](02-handoff.md) needs, each testable on its own: a VT replay of a terminal's state, PTY and child types built from an inherited fd and pid, and PTY readers that can stop reading without losing bytes. Owning crate: `deck-daemon`. Nothing changes for the user.

## Requirements

1. **`Emu::snapshot(&self) -> Vec<u8>`**: bytes that, fed into a fresh `Emu` of the same size and scrollback limit, reproduce:
   - the primary screen's history and screen, every cell's text and `Style` (as `Frame`s carry it), wide characters and combining marks included;
   - when the alternate screen is active, its content, after `CSI ? 1049 h` (so the primary screen stays behind it);
   - the cursor position, visibility and shape (DECSCUSR);
   - the modes in `protocol::Modes`: application cursor keys, bracketed paste, mouse click/drag/motion, SGR mouse, focus reporting, alternate scroll (`?1007`);
   - the title (OSC 2);
   - where the emulator exposes them: the scroll region (`DECSTBM`), autowrap, origin and insert modes, the saved cursor (`DECSC`), and the pending-wrap state at the last column;
   - with the alternate screen active, the primary screen's cursor (and saved cursor) set before `CSI ? 1049 h`, so leaving the alternate screen puts it back.
     A snapshot is taken outside synchronized output: if a program is inside `?2026`, the snapshot waits for its end or the emulator's timeout.
     Line wrapping must survive: a soft-wrapped line in the source is soft-wrapped in the copy, so a later resize reflows it the same way.
2. The snapshot never contains a sequence that makes the emulator reply (no DA, DSR, DECRQM, XTVERSION or colour queries), so feeding it produces no `EmuEvent::Reply`.
3. **Adopted PTY** in `deck-daemon/src/platform/`: `AdoptedMaster::from_fd(OwnedFd)` implementing `portable_pty::MasterPty` (`resize` with `TIOCSWINSZ`, `get_size` with `TIOCGWINSZ`, `try_clone_reader` and `take_writer` as dups, `process_group_leader` with `tcgetpgrp`, `as_raw_fd`, `tty_name` returning `None`). `AdoptedChild::new(pid)` implementing `portable_pty::Child` and `ChildKiller` (`try_wait`/`wait` through `platform::wait_status`, `kill` sends `SIGHUP` and then `SIGKILL` after the same grace period as portable-pty's, `process_id`). Every `unsafe` block has a `// SAFETY:` comment.
4. **Pausable readers**: the `pty-reader` thread waits with a `platform::poll_readable(fd, timeout)` helper (`poll(2)`, a timeout of at most 50 ms; `unsafe` stays in `platform/`) before each read and checks a per-terminal pause flag. `TermHandle::pause_reader()` sets it and returns once the reader has acknowledged, and the reader acknowledges only after the last chunk it read has been fed to the emulator (or the terminal has hit EOF); bytes not read stay in the kernel. `resume_reader()` clears it.
   The reader holds back an incomplete escape sequence or UTF-8 character at the end of a chunk (at most 8 KiB; it feeds the held bytes when the PTY stays quiet for one poll), so that the emulator is at the start of a sequence when a pause takes effect. `pause_reader()` returns `Result<Vec<u8>, PauseTimeout>`: the held bytes (the kitty scanner's partial command first, then the carry), which [02](02-handoff.md) stores in the handoff file and feeds to the copy right after the snapshot, before the output still in the kernel. The reader keeps them, so a rollback needs only `resume_reader()`. The limit is 1 s.
5. **Pausable waiters**: likewise, `pause_waiter()` stops the `pty-wait` thread from calling `waitpid` and returns any exit status it had already reaped; `resume_waiter()` restarts it. `pause_waiter()` returns `Result<Option<i32>, PauseTimeout>`, the same error as `pause_reader()`, with the same 1 s limit.
6. Pausing costs nothing while not paused: the poll timeout only bounds how fast a pause is seen; output latency stays as today.
7. `AdoptedMaster`'s writer does not send portable-pty's `\n` + VEOF on drop (that is portable-pty's own behaviour for the writers it makes).

## Acceptance criteria

- [ ] Round trip: for each of a corpus of inputs (plain shell output with 256-colour and truecolour SGR, every `Style` flag, a soft-wrapped long line, CJK and emoji, combining marks, `less` on the alternate screen, a cursor moved and hidden, bracketed paste and SGR mouse on, a bar cursor, a title), `full_frame` of the copy equals `full_frame` of the source, and the same holds after scrolling both to the top of the history.
- [ ] A soft-wrapped 200-column line in an 80-column source, replayed and then both resized to 120 columns, gives equal frames.
- [ ] Feeding a snapshot produces no `EmuEvent::Reply`.
- [ ] An `AdoptedMaster` made from a dup of a live portable-pty master reads the child's output, writes input it receives, resizes the PTY (the child sees the new size with `stty size`) and reports the foreground process group.
- [ ] An `AdoptedChild` for a live child reports its exit code and a signal death as `128 + signal`, and `kill` stops a child that ignores `SIGHUP`.
- [ ] A paused reader leaves output in the kernel: a child writing 100 KiB while paused loses no byte once resumed (compared through `capture`).
- [ ] `pause_reader` returns within 200 ms for an idle terminal and for one whose child exited.
- [ ] A child that exits while its waiter is paused is not reaped until `resume_waiter`, and a status reaped just before the pause is returned by `pause_waiter`.

## Tests

- `deck-daemon` unit tests next to `emu.rs` for the snapshot corpus and the replies check.
- `crates/deck-daemon/tests/`: adopted master and child against real children, the paused reader.

## Notes

- If DECK-19's `libghostty-vt` offers its own VT formatter for the screen, the worker may build `snapshot` on it, as long as requirement 1 and the acceptance criteria hold; say so in the report.
- Deviations recorded after implementation:
  - A snapshot taken while a program holds synchronized output (`?2026`) shows the screen as it is and never writes the mode. It does not wait for the hold to end: with the readers paused only the emulator's timeout could end it, so the rest of the frame arrives after the handoff.
  - The saved cursor (`DECSC`) is not replayed: the emulator does not expose it. The primary screen's cursor behind the alternate screen is replayed.
  - The adopted-PTY and pause tests are unit tests next to the code (`platform/adopted/tests.rs`, `terminal.rs`), not in `crates/deck-daemon/tests/`, because `platform` and `terminal` are private modules of the crate.
  - `deck-daemon` gained `anyhow` (already a workspace dependency) to name `portable_pty::Error` in the `MasterPty` impl.
  - A sequence a program leaves unfinished for longer than one reader poll (50 ms) is fed to the emulator anyway, so a pause in that window leaves the emulator mid-sequence; accepted as rare.
