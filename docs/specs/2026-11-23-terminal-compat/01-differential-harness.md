---
type: foundation
status: done
---

# 01 — Differential harness

## Scope

A test helper that shows the same bytes on the reference emulator and through deck and compares the two screens, plus a first small corpus that proves it works. Owning crate: `deck` (tests only). [02](02-escape-corpus.md) and [05](05-app-recordings.md) fill it with cases.

## Requirements

1. **Cases** are files in `crates/deck/tests/compat/cases/<name>.vt`: raw bytes, written by hand with a `printf`-style escape syntax and converted at load (`\e`, `\x1b`, `\r`, `\n`, `\t`, `\\`, `\xNN`), so they stay readable in review. A case may start with header lines `#! size 80x24` and `#! resize 100x30` (a resize after the bytes, compared again); other `#` lines at the top are comments.
2. **`compat::compare(case) -> Result<(), Diff>`** in `crates/deck/tests/support/compat.rs`:
   - reference: `Harness::spawn(size, ["/bin/sh", "-c", "stty raw -echo; cat \"$0\"; exec sleep 100000", file])`;
   - deck: a `Harness` running `deck` with `shell = "/bin/sh"` and `shell_args = ["-c", "stty raw -echo; cat \"$0\"; exec sleep 100000", file]`, sidebar off, one terminal;
   - raw mode with echo off on both sides keeps query answers off the screen and bare line feeds bare (overview);
   - it waits until both screens have been stable for 300 ms (and at most 5 s), then compares every cell: the full grapheme (base character and combining marks), fg, bg and the attribute flags, and the cursor position and visibility.
3. **Normalisation**: named colours 0–15 compare equal to indexed 0–15; the default fg/bg compare equal to themselves only.
4. **Known differences**: a table in `crates/deck/tests/compat.rs`, each entry being a case name (or `*`), what is ignored (an attribute, a region, the cursor) and a reason. A case listed as fully known-different must still differ (so a fix is noticed and the entry removed).
5. **`Diff`** prints a readable report: the case, the size, the first 20 differing cells as `(row, col) ref='a' red/default bold  deck='a' default/default`, and both screens' text side by side, and writes both screens to `target/e2e-captures/compat/<case>/`.
6. `CellView` in the harness gains the full grapheme (with combining marks) and the underline kind and colour that `alacritty_terminal` exposes, for [03](03-underline-styles.md).
7. **First corpus** (about 10 cases): plain text, 16/256/truecolour SGR, bold/dim/italic/underline/inverse/strikethrough/hidden, cursor movement (`CUP`, `CUU`/`CUD`/`CUF`/`CUB`), erase (`ED`, `EL`), a line longer than the screen, a wide character, a combining mark, and one resize case.

## Acceptance criteria

- [ ] Every first-corpus case passes, or is a known difference with a reason.
- [ ] A case built to differ (a test-only hook that alters one cell of deck's screen) fails with a report naming that cell.
- [ ] A known-different entry whose case starts to pass fails the run with `known difference no longer differs: <case>`.
- [ ] The compat tests run in `just test`; the whole compat suite (this corpus, [02](02-escape-corpus.md)'s and [05](05-app-recordings.md)'s) stays under 60 s on the worker's machine, checked again by 02 and 05 (stated in each report).

## Tests

- `crates/deck/tests/compat.rs` (new) runs every case in `compat/cases/`, one test per case (a generated list or a macro), so nextest reports them by name.

## Notes

- `screen_text()` drops combining marks today; the compare must not.
