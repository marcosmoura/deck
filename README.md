# deck

A terminal multiplexer written in Rust, built for coding agents. A background daemon keeps your sessions running; `deck` opens a rounded frame around a shell, and you can detach and come back to it later.

Status: early. Today `deck` attaches to a session with one shell. Projects, panels, the sidebar and agent support come with later work (see [docs/backlog.md](docs/backlog.md)).

## Install from source

macOS or Linux, with the Rust toolchain pinned in `.mise.toml` (`mise install`):

```sh
cargo install --path crates/deck
```

To try it without installing, with an isolated socket and state directory: `just run`.

## Use

```sh
deck                # attach to the most recent session, or start one called "main"
deck attach work    # attach to the session "work" (it must exist)
deck ls             # list the sessions
deck kill-server    # stop the daemon and every session
```

Press `Option+Space d` (`⌥Space d`) to detach: the shell keeps running in the daemon, and `deck` brings you back to it exactly as it was. On macOS, Option must act as Alt in your terminal for `Option+Space` (deck also accepts the non-breaking space some terminals send instead), and a launcher such as Raycast or Alfred may claim `Option+Space`: change either one, or set `prefix` in the config ([keys](docs/features/keys.md)).

More: [sessions](docs/features/sessions.md), [keys](docs/features/keys.md).
