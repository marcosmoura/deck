# Sessions

A session is a set of terminals that lives in the deck daemon, with or without a client attached. A new session has one space (the directory it was created in) with one panel and one terminal; more spaces and panels come from the [sidebar](sidebar.md).

## Attach

- `deck` attaches to the most recently attached session. When there is none, it creates `main` (then `main-2`, and so on) whose space is the current directory, named after its basename (`~` for the home directory, `root` for `/`).
- `deck attach <name>` attaches to that session. When it does not exist, deck prints `deck: no session named <name>` and exits 1; with `-c` (`--create`) it creates the session instead.
- `deck new [<name>] [-c <dir>] [-d]` creates a session and attaches to it. The name defaults like `main`'s; its first space is `<dir>` (default the current directory). It never attaches to an existing session: a duplicate name or a missing directory fails with the daemon's message and exit 1. Inside deck it is refused like `deck attach`, unless `-d` is given: `-d` creates the session without attaching and prints its name.
- The first `deck` command starts the daemon when it is not running.
- Attaching is refused inside deck (`DECK` is set) and when stdin or stdout is not a terminal.
- The shell is the `shell` of the config, else `$SHELL`, else `/bin/sh`, started with `shell_args` (default `["-l"]`, a login shell). It gets the environment of the `deck` that created the session.
- The shell's size follows the window: the terminal fills the screen minus the sidebar and the bottom status-bar row, with no border. The status bar shows `space › <icon> <panel>` after the session name, muted, and cut with `…` when it does not fit beside the hints; a panel you named by hand also shows ` · <title>` (the terminal's OSC title) when it differs from the name; both are left out when the panel area is empty (see [sidebar](sidebar.md#the-sidebar)). When the window is smaller than 20 columns or 5 rows, deck shows only `deck: window too small` and keeps the shell's last size.
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
- `Ctrl+o` creates a session named like the filter (with an empty filter it asks `New session:`; empty answers use the default name) whose space is the directory deck was started in, and switches to it. `Ctrl+r` renames the selected session and `Ctrl+x` kills it after `Kill session "<name>" and its N terminals? (y/n)`. A refused name shows the daemon's message and reopens the prompt.
- `Option+Space $` asks `Rename session:` for the session you are in; the status bar follows at once.

Switching keeps the same `deck` process and your terminal's screen: it leaves the old session and attaches to the new one without restoring the main screen. The sidebar's visibility and width carry over; the sidebar selection and open dialogs do not. When the session you are in is killed, from the picker or by `deck kill-session`, deck switches to the session attached most recently; when there is none it exits with `[session <name> ended]`.

## Detach and reattach

`Option+Space d` detaches ([keys](keys.md)). deck restores your terminal, prints `[detached from <session>]` and exits 0. The shell keeps running; `deck` attaches again and draws its screen as it is.

A session stays until it is killed; when its last terminal exits, the space stays empty.

## Commands

- `deck ls` lists the sessions by name, one per line: the name, the number of spaces and of terminals and `(attached)` while a client is attached. It prints `no sessions` when there are none. With no daemon running but saved sessions, it prints `no daemon running; <n> saved sessions will be restored by "deck"` and starts nothing.
- `deck kill-session <name>` kills a session and its terminals; `deck rename-session <old> <new>` renames one. A client attached to it sees the change at once. Errors print the daemon's message and exit 1. Neither starts the daemon: without one they print `no deck daemon running` and exit 1.
- `deck kill-server` stops the daemon, whatever protocol version it speaks. The sessions are saved first and come back with the next `deck` ([Persistence](persistence.md)). `deck kill-server --forget` also deletes the saved state; with no daemon running it only deletes it and prints `state cleared`.

## Messages when deck ends

| Message                                               | Exit | When                                           |
| ----------------------------------------------------- | ---- | ---------------------------------------------- |
| `[detached from <session>]`                           | 0    | you detached                                   |
| `[session <session> ended]`                           | 0    | the session was killed and no other is left    |
| `[deck daemon stopped]`                               | 0    | `deck kill-server` ran                         |
| `[lost connection to the deck daemon]`                | 1    | the connection broke                           |
| `deck: terminal exited at once (status N); see <log>` | 1    | the shell failed to start; the log has details |
| `deck: <text>`                                        | 1    | any other error from the daemon                |

A deck and a daemon of different versions refuse to talk: `the daemon speaks protocol N, this deck speaks M; run "deck kill-server" to restart it; sessions are restored when the daemon saved them`.

deck also restores the terminal on errors, panics, `SIGTERM` and `SIGHUP`.

## Colours and clipboard

At attach, deck asks your terminal for its foreground and background colours, so programs that ask the same question (Codex, OpenCode, nvim) pick a matching theme. A program's OSC 52 copy reaches your terminal's clipboard; OSC 52 reads are denied. Text you select with the mouse is copied the same way ([keys](keys.md#selecting-and-copying)).
