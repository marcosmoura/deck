---
type: shippable
status: done
---

# 03 — Live reload

## Scope

The user edits `config.toml` while deck runs, saves, and within about a second the colours, border style, key bindings, prefix and sidebar change in every attached client; a mistake shows an error with its line and changes nothing. Owning crates: `deck-daemon` (reading, watching), `deck-tui` (applying), `deck-core` (protocol).

## Requirements

1. **Protocol** (version bump): `Welcome` gains `config: Config` (the resolved wire form of the overview's decisions); new `ServerMsg::Config(Config)` and `ServerMsg::Notice { level: Info | Warning | Error, text }`. A notice is sent to every attached client; when no client is attached, the daemon keeps the last 20 notices (dropping the oldest, with every notice also in `daemon.log`) and sends them to the next client that attaches.
2. **The daemon reads the config** at start and is the only reader from now on: clients take the config from `Welcome` and stop reading the file (DECK-01's client-side read and sub-spec 01's interim parse are removed). Because the client no longer knows `sidebar` and `sidebar_width` before `Welcome`, `Role::Attach { cols, rows }` now carries the outer window size, and the daemon derives the first panel area with `deck-core::layout::panel_area(outer, &config, sidebar_visible)`, which this sub-spec adds in a new `deck-core::layout` module (moving DECK-02's client-side computation there) and which the client uses to draw; a new session's first terminal still starts at the right size. An invalid file at start: the daemon uses the defaults and queues an error notice.
3. **Watching**: every second the daemon compares the file's modification time, size and inode (and ctime where the platform provides it, so a same-size edit within one mtime tick is still seen) with the last read (a missing file counts as "defaults"). On a change it reads and validates the file (sub-spec 02's `validate`). When validation fails and the modification time or size changed again by the next tick (an editor still writing), the daemon reads it again instead of reporting the error. Editors that save by rename are handled because the path is checked, not an open handle.
4. **Valid change**: the daemon replaces its config, sends `ServerMsg::Config` to every client, and sends an info notice `config reloaded` (shown for 2 s). Clients apply it at once, only the values that changed (so a sidebar the user hid with `prefix b` stays hidden unless `sidebar` itself changed): theme, `border_style`, glyphs, bindings and prefix, `sidebar`, `sidebar_width` (re-sending their `Viewport`, which resizes PTYs when the panel area changed). An armed prefix or an open prompt is kept. Settings that shape new terminals (`shell`, `shell_args`, `scrollback_lines`) apply to terminals started afterwards; existing terminals are not touched.
5. **Invalid change**: the daemon keeps the last good config and sends an error notice with the first error and its location, `config.toml:<line>:<col>: <message> (kept the previous config)`, shown for 6 s. Warnings alone do not block the reload; they are sent as one warning notice.
6. **Client display** of notices: a flash in the status bar coloured by level and shown for a time per level: info `muted` 2 s, warning `warning` 4 s, error `error` 6 s; queued one after the other when several arrive. This is the rule for every notice, DECK-04's included.

## Acceptance criteria

- [ ] With deck attached, writing `[theme] accent = "red"` to the config makes the active panel row and the session name red (within 5 s; the test sets `DECK_TEST_CONFIG_POLL_MS=200`), and `config reloaded` flashes (capture `reloaded`).
- [ ] Changing `[keys] detach = "q"` makes `⌥Space q` detach and `⌥Space d` flash `no binding for d`, without reattaching.
- [ ] Changing `sidebar_width` from 28 to 40 moves the panel area and the shell's `stty size` shrinks by 12 columns.
- [ ] Writing a file with `accent = "purple"` shows `config.toml:2:10: …` (the actual location) in the error colour, and the previous accent stays.
- [ ] Saving through a temp file and rename (as editors do) is picked up.
- [ ] Deleting the file returns everything to the defaults.
- [ ] Changing `shell_args` does not restart running shells; a new terminal uses the new value.
- [ ] Two attached clients both apply a change.
- [ ] A reload with one key set by the user for two actions keeps the previous config and shows the error notice; a reload where a user's key replaces another action's default applies and shows a warning notice naming both actions.
- [ ] A same-size edit written within the same second as the previous one is picked up (inode or ctime differs).
- [ ] An invalid file at daemon start: deck starts with the defaults and the first client shows the error notice.
- [ ] With `sidebar_width = 40` in the config, a new session's first shell starts at the outer width minus 41 and the outer height minus the status bar (a single terminal has no border since DECK-02), with no resize afterwards (the harness sees one size from the first `stty size`).
- [ ] The client no longer opens `config.toml` (daemon test with a client in a different `XDG_CONFIG_HOME` still gets the daemon's config).

## Tests

- `deck-core`: protocol round trips for `Welcome`, `Config` and `Notice`.
- `crates/deck-daemon/tests/`: change detection (write, rename, delete) with a short poll interval set through a hidden env var (`DECK_TEST_CONFIG_POLL_MS`), notice queueing without clients, invalid-file handling.
- `deck-tui`: applying a new config (theme, bindings, sidebar width) and notice flashes.
- `crates/deck/tests/config_reload.rs`: the end-to-end criteria.
