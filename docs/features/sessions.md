# Sessions

A session is a set of terminals that lives in the deck daemon, with or without a client attached. A new session has one space (the directory it was created in) with one panel and one terminal; more spaces and panels come from the [sidebar](sidebar.md).

## Attach

- `deck` attaches to the most recently attached session. When there is none, it creates `main` (then `main-2`, and so on) whose space is the current directory, named after its basename (`~` for the home directory, `root` for `/`).
- `deck attach <name>` attaches to that session. When it does not exist, deck prints `deck: no session named <name>` and exits 1; with `-c` (`--create`) it creates the session instead.
- `deck new [<name>] [-c <dir>] [-d]` creates a session and attaches to it. The name defaults like `main`'s; its first space is `<dir>` (default the current directory). It never attaches to an existing session: a duplicate name or a missing directory fails with the daemon's message and exit 1. Inside deck it is refused like `deck attach`, unless `-d` is given: `-d` creates the session without attaching and prints its name.
- The first `deck` command starts the daemon when it is not running.
- Attaching is refused inside deck (`DECK` is set) and when stdin or stdout is not a terminal.
- The shell is the `shell` of the config, else `$SHELL`, else `/bin/sh`, started with `shell_args` (default `["-l"]`, a login shell). It gets the environment of the `deck` that created the session.
- The shell's size follows the window: the terminal fills the screen minus the sidebar, every row of the window, with no border; the [floating bar](keys.md#the-floating-bar) covers its last row only while it shows. The session's name is in the sidebar header (see [sidebar](sidebar.md#the-sidebar)). When the window is smaller than 20 columns or 5 rows, deck shows only `deck: window too small` and keeps the shell's last size.
- Several clients can attach to one session. The shell follows the client that typed or resized last; the others draw its layout cropped or padded to their window.

## Switching sessions

`Option+Space s` opens the session picker, a rounded overlay in the middle of the screen:

```text
            ╭─ Sessions ─────────────────────────────────────────╮
            │ > wo█                                              │
            │                                                    │
            │ ● work          3 spaces   7 terminals             │
            │   workshop      1 space    1 terminal   attached   │
            │                                                    │
            │ ⏎ switch  ^o new  ^r rename  ^x kill  esc close    │
            ╰────────────────────────────────────────────────────╯
```

- It lists every session, most recently attached first, with its space and terminal counts and `attached` when another client is attached. `●` marks the session you are in. The overlay is 60% of the width (at least 40 columns) and at most 70% of the height; the list scrolls.
- Typing filters by name (substring, case-insensitive). `Up`/`Down` or `Ctrl+p`/`Ctrl+n` move the selection, `Enter` switches, `Esc` closes. These keys are fixed, not under `[keys]`.
- `Ctrl+o` creates a session named like the filter (with an empty filter it asks `New session`; empty answers use the default name) whose space is the directory deck was started in, and switches to it. `Ctrl+r` renames the selected session and `Ctrl+x` kills it after `Kill session "<name>" and its N terminals?`, a dialog over the picker with `y yes  n no`. The prompts are centered dialogs too. A refused name shows the daemon's message in the dialog and reopens the prompt.
- `Option+Space $` opens the `Rename session` dialog for the session you are in; the sidebar header follows at once.

Switching keeps the same `deck` process and your terminal's screen: it leaves the old session and attaches to the new one without restoring the main screen. The sidebar's visibility and width carry over; the sidebar selection and open dialogs do not. When the session you are in is killed, from the picker or by `deck kill-session`, deck switches to the session attached most recently; when there is none it exits with `[session <name> ended]`.

## Detach and reattach

`Option+Space d` detaches ([keys](keys.md)). deck restores your terminal, prints `[detached from <session>]` and exits 0. The shell keeps running; `deck` attaches again and draws its screen as it is.

A session stays until it is killed; when its last terminal exits, the space stays empty.

## Commands

- `deck ls` lists the sessions by name, one per line: the name, the number of spaces and of terminals and `(attached)` while a client is attached. It prints `no sessions` when there are none. With no daemon running but saved sessions, it prints `no daemon running; <n> saved sessions will be restored by "deck"` and starts nothing.
- `deck kill-session <name>` kills a session and its terminals; `deck rename-session <old> <new>` renames one. A client attached to it sees the change at once. Errors print the daemon's message and exit 1. Neither starts the daemon: without one they print `no deck daemon running` and exit 1.
- `deck kill-server` stops the daemon, whatever protocol version it speaks. The sessions are saved first and come back with the next `deck` ([Persistence](persistence.md)). `deck kill-server --forget` also deletes the saved state, with the unreadable state files that were moved aside; with no daemon running it only deletes it and prints `state cleared`.
- `deck upgrade` hands the running daemon over to this `deck` without stopping what runs in it (see [Upgrading](#upgrading)).
- Each time the daemon stops, it writes one line to `daemon.log` saying why: `shutting down: kill-server from pid <pid> (session <s>, panel <p>, terminal <t>)`, `shutting down: last session "<name>" killed from pid <pid> (…)`, `shutting down: last session killed from an attached client`, `shutting down: the last session ended, its first terminal exited at once`, or `shutting down: SIGTERM` (`SIGINT`, `SIGHUP`). The ids are those of the deck terminal the command ran in (`DECK_SESSION_ID`, `DECK_PANEL_ID`, `DECK_TERMINAL_ID`), or `outside deck`; nothing else of the client's environment is logged. A signal does not say who sent it: `deck kill-server` signals a daemon of another protocol version with `SIGTERM`. A start line with no stop line before the next start means the daemon was killed (`SIGKILL`) or crashed.

## Messages when deck ends

| Message                                               | Exit | When                                                                 |
| ----------------------------------------------------- | ---- | -------------------------------------------------------------------- |
| `[detached from <session>]`                           | 0    | you detached                                                         |
| `[session <session> ended]`                           | 0    | the session was killed and no other is left                          |
| `[deck daemon stopped]`                               | 0    | `deck kill-server` ran                                               |
| `[deck was upgraded; run "deck" to reattach]`         | 0    | the daemon was upgraded and deck could not run the new binary itself |
| `[lost connection to the deck daemon]`                | 1    | the connection broke                                                 |
| `deck: terminal exited at once (status N); see <log>` | 1    | the shell failed to start; the log has details                       |
| `deck: <text>`                                        | 1    | any other error from the daemon                                      |

Closing the terminal deck runs in (the tab or window, a dropped ssh connection) detaches the client within a second and leaves nothing running; the session stays, and `deck ls` from another terminal still lists it.

A deck and a daemon of different versions refuse to talk: `the daemon speaks protocol N, this deck speaks M; run "deck upgrade" to hand it over to this deck; what runs in it keeps running` when the daemon is older and can be upgraded in place, else `...; run "deck kill-server" to restart it; sessions are restored when the daemon saved them`. `deck` and `deck attach` do not stop there: they upgrade an older daemon themselves.

## Upgrading

After installing a new `deck` (`brew upgrade`, `just install`, a rebuild), run `deck upgrade`. The daemon re-executes itself as the new binary, keeping its process id, so every shell, editor, dev server, REPL, ssh session and agent keeps running, with its screen and scrollback. Attached clients come back by themselves: each restores your terminal and runs `deck attach <session>` in its place, so the session redraws after about a second.

```text
$ deck upgrade
deck daemon upgraded (protocol 21 → 22, 7 terminals kept)
```

- With the same protocol version (a rebuild), it prints `deck daemon restarted in place (protocol N, <n> terminals kept)`.
- With no daemon running it prints `no deck daemon running` and exits 0. A daemon that predates live upgrade (it has no `deck.handoff` in the runtime directory) cannot be handed over: `deck upgrade` prints `the running daemon cannot be upgraded in place; run "deck kill-server" once` and exits 1; after that restart, upgrades work in place.
- The new binary is the one that ran `deck upgrade`; it is checked first. When it cannot take over, or the handoff fails, `deck upgrade` prints the reason and exits 1, and the daemon goes on as it was (`upgrade refused: <why>`, `upgrade failed: <why>`). It waits at most 30 s: `the upgrade did not finish; see <log>`.
- `deck` and `deck attach` that find an older daemon able to upgrade run `deck upgrade` themselves, print `upgrading the deck daemon (protocol A → B)` to stderr and attach. A newer daemon, or one that cannot be handed over, keeps the mismatch error above.
- While the daemon is handing over, other `deck` commands wait for it (up to 30 s) instead of starting a second daemon. A `deck wait` that is pending then ends with exit 1 and `the deck daemon is upgrading; try again`; so does any control command that was already waiting for an answer.
- Not kept: kitty images, the palette a program changed with OSC 4, 10 and 11, scroll regions, tab stops and charset state. Full-screen programs get a size change and redraw. A crash, `SIGKILL` or reboot still stops everything; see [Persistence](persistence.md).
- `daemon.log` has `upgrading: <binary> (handoff N)` when it starts, `adopted <n> terminals from pid <pid>` from the new image, and `upgrade refused: <why>` or `upgrade failed: <why>` when it did not happen.

deck also restores the terminal on errors, panics, `SIGTERM` and `SIGHUP`.

## Colours and clipboard

At attach, deck asks your terminal for its foreground and background colours, so programs that ask the same question (Codex, OpenCode, nvim) pick a matching theme. A program's copy (OSC 52, iTerm2's OSC 1337 `Copy` and kitty's OSC 5522) reaches your terminal's clipboard when it is plain text of at most 4 MiB; a larger one, or one with no `text/plain` part, is dropped. Clipboard reads are denied. Text you select with the mouse is copied the same way ([keys](keys.md#selecting-and-copying)).
