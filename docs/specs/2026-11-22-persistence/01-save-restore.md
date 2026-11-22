---
type: shippable
status: done
---

# 01 — Save and restore

## Scope

The user stops the daemon (or it dies) and the next `deck` shows the same sessions, projects, panels, grid and focus, with fresh shells. Owning crates: `deck-daemon` (store, restore, shutdown), `deck-core` (state types, `Notice`), `deck` (`--forget`, showing notices), `deck-tui` (the notice flash).

## Requirements

1. **State types** in `deck-core::state`: `StateFile { version: u32, next_id: u64, last_attached: Vec<SessionId>, sessions: Vec<SessionState> }` and nested `ProjectState`, `PanelState`, `TerminalState` holding the fields the overview's decisions list, plus `TerminalState.cwd: Option<PathBuf>` (the launch cwd until sub-spec 02 tracks it). A terminal's saved `env` holds only the additions of its launch spec (DECK-05's `--env`; empty for shells), never the session's client environment and never the `DECK_*` variables, which are regenerated on launch. Restored terminals get the daemon's start environment plus those additions (overview decision). Later specs add fields as `Option`s with `#[serde(default)]` (DECK-06 adds agent resume info), which old files still parse without a version bump. `version` is 1. Conversions between the daemon's model and the state types are pure and tested. The file is mode `0600` because `--env` additions may hold secrets.
2. **Saving**: any change to a saved field marks the state dirty: actions, terminals added or removed, focus changes, `last_attached` changes on attach, collapse changes (a title change alone does not count). The first dirty mark schedules a save 500 ms later (a fixed window, not trailing); marks inside the window join that save. The save, off the async worker threads (`spawn_blocking`):
   - creates the state dir with mode `0700` when missing;
   - writes `state.json.tmp` with mode `0600`, fsyncs it, renames it over `state.json`, and fsyncs the directory.

   A failed save is logged and retried on the next change; it never stops the daemon.

3. **Shutdown**: on `kill-server`, `SIGTERM`, `SIGINT` and `SIGHUP`, the daemon first sets a `shutting_down` flag. From then on, terminal exits do not change the model, and the pending save timer is cancelled. Then it saves synchronously, then hangs up the terminals (DECK-01). Terminals that end because of the shutdown therefore stay in the saved state.
4. **`deck kill-server --forget`**: the CLI stops the daemon as usual (DECK-01 waits until the lock and socket are released), then deletes `state.json` and any `state.json.tmp`. With no daemon running it deletes them anyway and prints `state cleared`. The protocol is unchanged.
5. **Session end**: `KillSession` removes the session from the state like any other change. When the last session is killed, the daemon saves an empty state before exiting.
6. **Start-up order**: take the lock (DECK-01), delete a leftover `state.json.tmp`, read and validate `state.json`, rebuild the model, bind the socket, then launch the terminals. Clients can connect while terminals are still starting, so the client's 3 s connection wait never covers the launches.
7. **Validation** of a parsed file, before rebuilding: `next_id` becomes `max(next_id, highest saved id + 1)`; a duplicate id drops the later item; a duplicate session or project name gets ` (2)`, ` (3)`, … appended; focus or `last_attached` entries pointing at missing items are dropped and focus falls back by the DECK-02 rules; ratios not valid for the restored count become the defaults. Restored panels bypass the DECK-03 room check.
8. **Launch**: each terminal, shell (`program: None`) or explicit program, starts in the first directory that exists of: its saved cwd, its project root, the home dir. The daemon checks each candidate with `std::fs::metadata(..).is_dir()` itself before spawning, because portable-pty 0.9 silently falls back to the home dir when the cwd is missing. Shells use the configured shell; programs use the same launch spec otherwise. A terminal whose launch still fails (a missing program, for example) is dropped, logged, and listed in a notice; its panel follows the lifecycle rules (a panel left empty is removed; projects and sessions stay).
9. **Notices**: restore problems are sent as `ServerMsg::Notice` ([DECK-08 sub-spec 03](../2026-11-22-config-theming/03-live-reload.md)), which already queues notices for the next attaching client and shows them as flashes.
10. **Bad file**: a `state.json` that fails to parse is renamed to `state.json.bad-<unix-seconds>`; the daemon logs the reason, starts with no sessions, and queues the notice `previous state could not be read; see <log path>`. The client then creates `main` as usual and shows it.
11. **Newer file**: a `state.json` with a `version` greater than 1 is left untouched and the daemon refuses to start, logging `state.json was written by a newer deck (version <n>); upgrade deck or run "deck kill-server --forget"`. The client prints the same line (from the log's last line, or the generic start-up failure of DECK-01 pointing at the log) and exits 1.
12. **Restored sizes**: restored terminals start at 80×24 until a client sends a viewport.
13. **Version message**: the `VersionMismatch` hint from DECK-01 now says `run "deck kill-server" to restart it; sessions are restored when the daemon saved them`. This stays true across versions because `kill-server` stops a daemon of another protocol version with `SIGTERM` (the pid in `deck.lock`, DECK-01), which runs the shutdown save of requirement 3.
14. **`deck ls` without a daemon**: when `state.json` exists and holds at least one session, `deck ls` prints `no daemon running; <n> saved sessions will be restored by "deck"` and exits 0 without starting a daemon; with no file or an empty one it keeps `no sessions`. `deck ls --json` (DECK-05) is unaffected.

## Acceptance criteria

- [ ] Two sessions, one with two projects, panels with 1, 3 and 5 terminals, custom names, custom ratios, a collapsed project and focus on terminal 3 of the 5-terminal panel: after `deck kill-server` and `deck`, everything is back, with ids unchanged (captures `before`, `after`).
- [ ] A terminal running a program that exits at once on `SIGHUP` (`sh -c 'trap "exit 0" HUP; while :; do sleep 1; done'`, as an explicit program in a `state.json` the test writes before starting the daemon, since nothing can create explicit-program terminals before DECK-05) is still there after `deck kill-server` and `deck`, and the same holds when the daemon gets `SIGTERM` or `SIGHUP` instead.
- [ ] After `kill -9` on the daemon, a change made more than 1 s before the kill is restored.
- [ ] `deck kill-server --forget` then `deck` starts fresh with `main`; `deck kill-server --forget` with no daemon running deletes `state.json` and prints `state cleared`.
- [ ] Killing a session, then restarting, does not bring it back.
- [ ] A corrupt `state.json` is renamed to `state.json.bad-*`, the client shows the notice, and `main` is created.
- [ ] A `state.json` with `version: 2` is left unchanged, the daemon does not start, and `deck` prints the newer-version line and exits 1.
- [ ] A terminal whose saved cwd was deleted starts in its project root; with the project root deleted too, in the home dir.
- [ ] A terminal whose explicit program no longer exists (seeded in `state.json` by the test) is dropped, its empty panel is removed, and the attaching client shows a notice naming it.
- [ ] A file with a duplicate terminal id, a duplicate project name and focus on a missing panel restores without error, following requirement 7.
- [ ] The restored terminal's environment has fresh `DECK_TERMINAL_ID` and `DECK_SOCKET` values and comes from the daemon's start environment: a variable set only in the client that created the session before the restart (`FOO=1 deck`, then `deck kill-server`, then `deck` without it) is absent after the restore; `state.json` contains no `DECK_` variables and no client environment.
- [ ] A terminal whose saved cwd is missing gets the project root even though portable-pty would have used the home dir (the daemon checked it).
- [ ] With the daemon stopped and two sessions saved, `deck ls` prints `no daemon running; 2 saved sessions will be restored by "deck"` and no daemon starts.
- [ ] After a daemon of a different `PROTOCOL_VERSION` (a test build flag) saved state, `deck kill-server` from the current build stops it through `SIGTERM` and the next `deck` restores its sessions.
- [ ] `state.json` and the state dir have modes `0600` and `0700`, and `state.json` is never seen half-written (a reader looping during 100 saves always parses it); a leftover `state.json.tmp` is deleted at start.
- [ ] Model → state → model round trips equal the original for a generated model with every field set.

## Tests

- `deck-core`: state conversions, validation (requirement 7) and serde round trips; `Notice` round trip.
- `crates/deck-daemon/tests/`: explicit-program terminals are created by seeding `state.json`; save debounce, atomic write and modes, the shutdown flag (no model change after it), restore with fallbacks, start-up order (a client connects before slow launches finish), bad and newer files, notices queued for the next client.
- `crates/deck/tests/persistence.rs`: the end-to-end criteria above, restarting the daemon inside the test.
