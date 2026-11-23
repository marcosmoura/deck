---
type: shippable
status: todo
---

# 03 — `deck upgrade` and reattaching clients

## Scope

The user-facing part: `deck upgrade`, automatic upgrade when `deck` meets an older daemon, and attached clients that come back by themselves. Owning crates: `deck` (command, automatic path), `deck-core` (`ByeReason::Upgrading`, protocol bump), `deck-tui` (reattach).

## Requirements

1. **`deck upgrade`** works on the runtime directory of the daemon it would talk to: inside deck, the parent directory of `DECK_SOCKET`; outside, the one from the environment, like other commands. It reads the pid from `deck.lock` and the marker `deck.handoff` ([02](02-handoff.md)). Then:
   - no daemon running: prints `no deck daemon running` and exits 0;
   - no marker, or a marker whose `pid` differs from `deck.lock`'s (left by a daemon that died): prints `the running daemon cannot be upgraded in place; run "deck kill-server" once` and exits 1, sending no signal;
   - a marker saying `upgrading`: waits for that handoff to end first;
   - otherwise removes any old `deck.upgrade.result`, writes its own `current_exe()` (canonicalised) to `<runtime>/deck.upgrade` (0600, temp name and rename), sends `SIGUSR2`, and waits up to 30 s for either a marker with a higher `generation` (success) or `deck.upgrade.result` (refusal or failure). On success it prints `deck daemon upgraded (protocol A → B, <n> terminals kept)` (`<n>` from the marker's `terminals`), or `deck daemon restarted in place (protocol N, <n> terminals kept)` when the versions are equal (useful after a rebuild), and exits 0. On a result it prints the reason and exits 1; on timeout it prints `the upgrade did not finish; see <log>` and exits 1.
2. **`ByeReason::Upgrading { exe: PathBuf }`**, appended after the existing variants, so `Bye`'s variant index and the older reasons keep their shape. The quiesce step of [02](02-handoff.md) sends it instead of `ServerShutdown`. `PROTOCOL_VERSION` goes up by one.
3. **Reattach**: a TUI client that receives `Upgrading { exe }` restores the outer terminal as on exit and then executes `<exe> attach <session name>` (replacing its own process, same tty and environment), so the user is back in the same session. If the exec fails, it prints `[deck was upgraded; run "deck" to reattach]` and exits 0. Control clients (`deck ls`, `send`, `wait` …) print `the deck daemon is upgrading; try again` and exit 1.
4. **Automatic upgrade**: `deck` and `deck attach` that get `VersionMismatch { daemon }` with `daemon < PROTOCOL_VERSION`, and find `deck.handoff`, run the upgrade of 1, print `upgrading the deck daemon (protocol A → B)` to stderr, then attach as usual. Without `deck.handoff`, or when the daemon is newer, today's mismatch message stays. Other commands keep today's mismatch error, now ending with `run "deck upgrade"` when `deck.handoff` exists, else `run "deck kill-server"`.
5. `deck upgrade` appears in `deck --help`; `mismatch_text` names it as in 4.
6. **Clients wait out a handoff**: while the marker says `upgrading`, `ensure_daemon` and a reattaching client wait (polling the marker and the socket) up to 30 s instead of giving up after `START_TIMEOUT` and starting more daemons.

## Screen

The attached client, during an upgrade (about a second), shows nothing of its own: it leaves the alternate screen, the new client enters it again and draws the session. In the shell where `deck upgrade` ran:

```text
$ deck upgrade
deck daemon upgraded (protocol 21 → 22, 7 terminals kept)
```

## Acceptance criteria

- [ ] `deck upgrade` with the same build: an attached client (harness PTY) comes back to the same session with the same screen, and a `sleep` started before still runs with the same pid.
- [ ] `deck upgrade` against a daemon started with `DECK_TEST_PROTOCOL_VERSION` set to an older version (and `deck.handoff` present) prints the protocol change; `deck ls` works afterwards.
- [ ] `deck upgrade` without `deck.handoff` (removed by the test), or with a marker naming another pid, prints the `kill-server once` line, exits 1 and sends no signal (the daemon keeps running).
- [ ] `deck upgrade` pointing at a binary whose `--handoff-check` fails prints the refusal reason and exits 1.
- [ ] A reattaching client and a `deck ls` started during a handoff with 30 terminals of 10,000 lines each both succeed, and no second daemon process is started.
- [ ] `deck upgrade` with no daemon prints `no deck daemon running` and exits 0.
- [ ] `deck` against an older daemon with `deck.handoff` upgrades it and attaches; without it, prints today's mismatch error.
- [ ] A `deck wait --exit` pending during the upgrade ends with exit 1 and the `upgrading` message.
- [ ] `deck ls` against an older daemon ends its error with `run "deck upgrade"`.
- [ ] A protocol test pins `Bye`'s variant index and decodes a `ByeReason::Upgrading` after the four older reasons.

## Tests

- `crates/deck/tests/upgrade.rs`: the criteria above.
- `deck-core`: protocol tests for `ByeReason`.
- `deck-tui`: the client's handling of `Upgrading` (unit test with the exec behind a closure).

## Notes

- After this sub-spec merges, the lead adds to the summary: the daemon running on the user's machine predates this spec, so the first switch to it needs `deck kill-server` once.
