# Sessions

A session is a set of terminals that lives in the deck daemon, with or without a client attached. Today a session has one project (the directory it was created in) and one terminal.

## Attach

- `deck` attaches to the most recently attached session. When there is none, it creates `main` (then `main-2`, and so on) whose project is the current directory, named after its basename (`~` for the home directory, `root` for `/`).
- `deck attach <name>` attaches to that session. When it does not exist, deck prints `deck: no session named <name>` and exits 1.
- The first `deck` command starts the daemon when it is not running.
- Attaching is refused inside deck (`DECK` is set) and when stdin or stdout is not a terminal.
- The shell is the `shell` of the config, else `$SHELL`, else `/bin/sh`, started with `shell_args` (default `["-l"]`, a login shell). It gets the environment of the `deck` that created the session.
- The shell's size follows the window: the frame fills the screen minus the bottom status-bar row. When the window is smaller than 20 columns or 5 rows, deck shows only `deck: window too small` and keeps the shell's last size.
- Several clients can attach to one session. The shell follows the client that typed or resized last; the others draw its layout cropped or padded to their window.

## Detach and reattach

`Option+Space d` detaches ([keys](keys.md)). deck restores your terminal, prints `[detached from <session>]` and exits 0. The shell keeps running; `deck` attaches again and draws its screen as it is.

A session ends when its last terminal exits.

## Commands

- `deck ls` lists the sessions, one per line: the name, the number of terminals and `(attached)` while a client is attached. It prints `no sessions` when there are none.
- `deck kill-server` stops the daemon and every session, whatever protocol version the daemon speaks.

## Messages when deck ends

| Message                                            | Exit | When                                         |
| -------------------------------------------------- | ---- | -------------------------------------------- |
| `[detached from <session>]`                        | 0    | you detached                                 |
| `[session <session> ended]`                        | 0    | the last terminal exited                     |
| `[deck daemon stopped]`                            | 0    | `deck kill-server` ran                       |
| `[lost connection to the deck daemon]`             | 1    | the connection broke                         |
| `deck: terminal exited at once (status N); see <log>` | 1 | the shell failed to start; the log has details |
| `deck: <text>`                                     | 1    | any other error from the daemon              |

A deck and a daemon of different versions refuse to talk: `the daemon speaks protocol N, this deck speaks M; run "deck kill-server" to restart it`.

deck also restores the terminal on errors, panics, `SIGTERM` and `SIGHUP`.

## Colours and clipboard

At attach, deck asks your terminal for its foreground and background colours, so programs that ask the same question (Codex, OpenCode, nvim) pick a matching theme. A program's OSC 52 copy reaches your terminal's clipboard; OSC 52 reads are denied.
