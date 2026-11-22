---
type: shippable
status: todo
---

# 02 — Terminal input, output and waiting

## Scope

An agent types into a terminal, reads what it shows, and waits for a command to finish or for text to appear. Owning crates: `deck` (CLI), `deck-core` (requests), `deck-daemon` (handlers).

## Requirements

1. **Requests** (`PROTOCOL_VERSION` bumps): `Send { terminal, bytes, paste: bool }`, `SendKeys { terminal, keys: Vec<KeySpec> }`, `Capture { terminal, lines: Option<u32>, history: bool, ansi: bool }`, `Wait { terminal, until: WaitFor, timeout_ms: Option<u64> }` with `WaitFor { Exit, Text(String), Idle { quiet_ms: u64 } }`. `SendKeys` is encoded by the daemon with `keys::encode` ([DECK-01 02-core-types](../2026-11-22-core-daemon-terminal/02-core-types.md)) after converting each `KeySpec` to a key event, using the terminal's current `Modes` from its emulator. Requests to an exited kept terminal, except `Capture` and `Wait`, fail with `terminal <id> has exited`.
2. **Self-targeting**: `send`, `send-keys` and `wait` resolve their default terminal like the other commands, but when it would be the caller's own terminal (`DECK_TERMINAL_ID`) without an explicit `--terminal`, they fail with `refusing to act on your own terminal; pass --terminal <id>`. `wait --exit` on the caller's own terminal fails even when explicit (`cannot wait for your own terminal to exit`). `capture` reads the caller's own terminal by default.
3. **`deck send [--terminal <id>] [--enter] [--paste] <text>`**: sends the text as bytes. `--paste` wraps it in bracketed-paste markers when the terminal's modes ask for them (the reliable way to give multi-line text to agent TUIs); without `--paste` markers are never added. `--enter` appends `\r` after the text (after the closing marker with `--paste`). `-` as the text reads stdin, at most 1 MiB (`input too large` beyond).
4. **`deck send-keys [--terminal <id>] <key>...`**: each argument is a `KeySpec` (`Enter`, `C-c`, `Up`, `M-x`, or a single character); an invalid one fails with the `KeySpec` parse error naming it, before anything is sent.
5. **`deck capture [--terminal <id>] [--lines <n>] [--history] [--ansi]`**: prints the visible screen as text, each line with trailing spaces trimmed, then trailing empty lines dropped; `--lines n` then keeps the last `n` lines. `--history` prepends the scrollback; it is ignored on the alternate screen (which has none). Wide characters are printed once (their spacer cells are skipped) and combining marks stay attached. `--ansi` keeps colours and attributes as SGR sequences. Output is capped at the last 10,000 lines or 8 MiB, whichever is smaller, so a response always fits the protocol's 16 MiB frame; a capped capture prints `[truncated: <n> earlier lines]` first (plain) or sets `"truncated": true` (JSON). `--json` gives `{"terminal","cols","rows","alternate_screen","truncated","cursor":{"row","col"},"lines":[...]}`, with the terminal's current PTY size (a terminal never shown yet reports its start size) and plain-text lines. Terminals of inactive panels and detached sessions can be captured: their emulators keep running.
6. **`deck wait [--terminal <id>] (--exit | --text <s> | --idle <ms>) [--timeout <seconds>]`**, resolved by the daemon (the client never polls):
   - `--exit`: answers when the process exits, with its exit code (signal `n` as `128 + n`, from sub-spec 01's `wait_status`). The CLI prints the code (`{"exit": n}` with `--json`) and exits 0. A kept terminal that has already exited, or a removed terminal whose status the daemon still holds (60 s, at most 256 entries, oldest dropped first), answers at once. After 60 s, `no terminal <id>`.
   - `--text <s>`: checks the visible screen (main or alternate) line by line at once, then again after each change, and answers with the first line containing `s`; `s` never spans lines. The CLI prints that line.
   - `--idle <ms>`: answers once the terminal has printed nothing for `ms` milliseconds, counted from the later of the call and its last output. `--idle 0` answers at once.
   - `--timeout <seconds>` (decimals allowed, enforced by the daemon, which answers `TimedOut`): the CLI prints `timed out` and exits 124. Without `--timeout` the wait has no limit.
   - A pending wait ends with an error (exit 1) when its terminal is closed before the condition holds (`--text`, `--idle`: `terminal <id> was closed`; `--exit` still answers with the exit status the hang-up produced), when the session is killed (`session was killed`), or when the daemon stops (`deck daemon stopped`).
   - When the client disconnects (Ctrl+C on `deck wait`), the daemon drops its waiter.

## Acceptance criteria

- [ ] `deck send --terminal <other> --enter 'echo hi'` makes that terminal print `hi`; `deck capture --terminal <other>` includes `hi` and its prompt.
- [ ] `deck send 'x'` without `--terminal` from inside deck fails with the self-targeting error; `deck capture` without `--terminal` prints the caller's own screen.
- [ ] `deck send --paste --enter -` with two lines on stdin, to a program with bracketed paste on (one that echoes its input bytes), arrives as `ESC[200~…ESC[201~\r`; 2 MiB on stdin fails with `input too large`.
- [ ] `deck send-keys --terminal <id> C-c` interrupts a running `sleep 100`; `deck send-keys --terminal <id> X-q` fails naming `X-q` and sends nothing.
- [ ] `deck send-keys Up` to a program with application cursor mode on sends `ESC O A`.
- [ ] `deck send` to an exited kept terminal fails with `terminal <id> has exited`.
- [ ] `deck new-terminal --keep -- sh -c 'sleep 1; exit 4'` followed by `deck wait --terminal <id> --exit` prints `4` and exits 0 no sooner than 0.8 s and within 5 s; without `--keep` the same works; a second `wait --exit` 5 s later still prints `4`.
- [ ] `deck wait --exit` on a terminal that exits 124 prints `124` and exits 0.
- [ ] `deck wait --text ready --timeout 5` returns once `ready` appears; with `ready` already on screen it returns at once; with a text that never appears it exits 124 no sooner than 5 s and within 15 s.
- [ ] `deck wait --idle 500` returns no sooner than 450 ms and within 3 s after output stops. (Exact timing of `--idle`, `--timeout` and retention is asserted in daemon tests with an injected clock.)
- [ ] `deck close <id>` during a pending `wait --text` makes it exit 1 with `terminal <id> was closed`; `deck kill-server` during a wait makes it exit 1 with `deck daemon stopped`.
- [ ] Killing a `deck wait` client leaves no waiter in the daemon (daemon test inspects the waiter count).
- [ ] `deck capture --history --lines 3` after `seq 1 100` prints `99`, `100` and the prompt line; on a terminal running `less`, `--history` is ignored and `alternate_screen` is `true` in `--json`.
- [ ] A line with CJK characters, an emoji and a combining mark is captured exactly as printed.
- [ ] `deck capture --json` parses and its `lines` match the plain output.
- [ ] After `seq 1 20000` with `scrollback_lines = 30000`, `deck capture --history` prints the truncation line and the last 10,000 lines; `--json` has `"truncated": true`.

## Tests

- `deck-core`: request round trips; `KeySpec` to key event conversion.
- `crates/deck-daemon/tests/`: each request; wait conditions resolved by the daemon, with exact timing of `--idle` and `--timeout` against an injected clock; exit-status retention and its bound (injected clock); the capture cap; waiter cleanup on disconnect, close, session kill and shutdown; capture of wide and combining characters.
- `crates/deck/tests/control_io.rs`: one scenario per end-to-end criterion above.
