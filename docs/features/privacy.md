# Privacy

What deck knows about you, where it keeps it, for how long, and what it never does. The evidence is in the [privacy review](../reviews/2026-11-24-privacy.md).

## In short

- deck makes no network connection. It has no telemetry, no crash reporting, no update check and no account.
- What deck knows stays in the daemon's memory and in a few files under your home directory, readable by you only.
- Programs you run inside deck (shells, agents, `ssh`) behave as they do in any terminal: what they send over the network is theirs.

Two directories hold everything deck writes for itself:

- The **state directory**: `~/.local/state/deck/` (`$XDG_STATE_HOME/deck/` when that is set). It is inside your home, so backup and sync tools copy it.
- The **runtime directory**: `$XDG_RUNTIME_DIR/deck/`, else `$TMPDIR/deck-<uid>/`, else `/tmp/deck-<uid>/`.

Both are `0700`. "You only" below means your user: any program that runs as you can read these files and talk to the daemon ([Control API](control-api.md#security)).

## What deck never does

- Connect to a network. The binary has no HTTP, TLS or DNS code; its only sockets are Unix sockets in the runtime directory.
- Read the clipboard. A program's request to read it is refused.
- Write what is on a screen, the scrollback or your keystrokes to a file with a name. The one file that holds them, the handoff of a [live upgrade](sessions.md#upgrading), is deleted before its first byte is written (see below).
- Read an agent's conversations, unless you open the restore dialog.
- Change an agent's configuration, unless you run `deck hooks install` or `deck skills install`.
- Log what a terminal shows, what you type, an environment value, a copy, a title, a prompt or an agent's session id.

## What deck keeps in memory

Gone when the daemon stops (`deck kill-server`, a reboot). A row says so when a part of it is also written to a file; the rest is never in a named file.

| What                                                                                                                                                                                                                        | Until                                                                        |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| Each terminal's screen and scrollback (10,000 lines by default), its images (64 MiB per screen)                                                                                                                             | The terminal closes                                                          |
| Input on its way to a program (at most 1 MiB per terminal)                                                                                                                                                                  | It is written or dropped                                                     |
| Each session's environment: every variable of the shell that created the session, tokens too                                                                                                                                | The session ends; six variables (`SSH_AUTH_SOCK`, ...) are renewed on attach |
| The daemon's own environment: that of the `deck` that started it                                                                                                                                                            | The daemon stops; sessions restored after a restart get this environment     |
| The environment of every `deck` command, sent whole when it connects                                                                                                                                                        | The command ends                                                             |
| Each terminal's title, working directory and foreground program. Also saved in `state.json`: the directory, the program a terminal was started with and its arguments, and the first 32 cells of a title that names a panel | The terminal closes                                                          |
| Per agent: its status, its kind and session id, and the first line of its first prompt (200 characters). Also saved in `state.json`: the kind, the session id, and the first 32 cells of a prompt that names a panel        | The agent leaves the terminal, or reports another session                    |
| Exit statuses of closed terminals                                                                                                                                                                                           | 60 s                                                                         |
| Notices nobody has seen yet (the last 20). `daemon.log` has a line for each                                                                                                                                                 | A client attaches                                                            |
| A copy on its way to your terminal (4 MiB each, 16 MiB waiting per client)                                                                                                                                                  | It is sent; dropped when no client is attached                               |
| Your terminal's name, version, colours and cell size                                                                                                                                                                        | The session ends                                                             |
| The agent sessions listed for the restore dialog (id, title, directory, time)                                                                                                                                               | The dialog closes                                                            |

## What deck writes to disk

| File                                                     | What it holds                                                                                                                                                                                                                                                                                                           | Mode                         | How long                                                                                       | To delete                       |
| -------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------- | ---------------------------------------------------------------------------------------------- | ------------------------------- |
| `state.json` (and `state.json.tmp` during a save)        | The layout: session, space and panel names, space directories, each terminal's directory, its program and arguments, its `--env` values, `keep` (with the exit status of a kept terminal that ended), and for an agent its kind and session id. A panel name can be the first words of a title or a prompt (see below). | `0600`                       | Rewritten 500 ms after a change and when the daemon stops                                      | `deck kill-server --forget`     |
| `state.json.bad-<seconds>`                               | A state file deck could not read, moved aside                                                                                                                                                                                                                                                                           | As it was                    | Until you delete it                                                                            | `deck kill-server --forget`     |
| `daemon.log`, `daemon.log.1`                             | Starts, stops, upgrades, errors and a line of counts every 10 minutes. It can name your user (in the socket's path), the path of a program that could not start, a session or space name, the path of the binary of an upgrade, and the text of an error in `config.toml`.                                              | `0600`                       | `daemon.log` moves to `.1` when a daemon starts and finds it over 10 MiB; deck deletes neither | By hand                         |
| `backups/<agent>-<file>-<seconds>`                       | A whole copy of an agent's config file from before `deck hooks install` or `uninstall` changed it: for Claude Code your `settings.json`, with an `env` block and its keys if you have one                                                                                                                               | `0600` in a `0700` directory | The newest 3 of each file                                                                      | By hand                         |
| `state.lock`, and `deck.lock` in the runtime directory   | Locks; `deck.lock` holds the daemon's pid                                                                                                                                                                                                                                                                               | `0600`                       | They stay                                                                                      | By hand, with no daemon running |
| Runtime directory: `deck.sock`                           | The socket                                                                                                                                                                                                                                                                                                              | In the `0700` directory      | Removed when the daemon stops                                                                  |                                 |
| Runtime directory: `deck.handoff`                        | The daemon's version, pid, the number of upgrades and of terminals                                                                                                                                                                                                                                                      | `0600`                       | Removed when the daemon stops                                                                  |                                 |
| Runtime directory: `deck.upgrade`, `deck.upgrade.result` | The path of the binary to upgrade to; why the last upgrade was refused or failed                                                                                                                                                                                                                                        | `0600`                       | The request is deleted when read; the result stays until the next upgrade                      | By hand                         |

All of them are in the state directory unless the table says otherwise.

- **The handoff file of a live upgrade** holds everything: every session's environment, every screen and its scrollback. It is created `0600` as `deck.handoff.<pid>.tmp` in the runtime directory and deleted at once, before anything is written to it; the daemon keeps only the open descriptor, for the milliseconds to seconds the upgrade takes, and a failed upgrade closes it. Nothing with a name ever holds its content.
- **Files deck writes for agents, when you ask**: its hooks in Claude Code's `settings.json` and Codex's `hooks.json`, its OpenCode plugin and the `deck` skill ([agents](agents.md#hooks), [Control API](control-api.md#agent-skill)). They hold deck's own path (a path under your home for an install in `~/.local/bin`, which matters if you publish your dotfiles) and nothing about you.
- **A temp copy beside an agent's config, for a moment**: `deck hooks install` and `uninstall` write the new content of the file to `.<name>.deck-<pid>` in the same directory, with the file's own permissions, and rename it over the file. For Claude Code that is a whole copy of your `settings.json`, keys included. It stays only if deck is killed in the middle of the write: delete it by hand. `deck skills install` writes the skill the same way; that temp file holds the skill's text only.
- **A crash**: a daemon that panics writes the message and the place in the code to `daemon.log`. One that is killed by the system (out of memory, a fault in the terminal library) is the operating system's to record: on Linux a core dump can hold every session's environment and scrollback (DECK-38 will stop that); a macOS crash report holds the stacks, not the memory.

## What deck reads about you

- **The environment and directory of every `deck` command**, sent to the daemon when it connects. The daemon keeps it for a new session. Otherwise it reads only `DECK_SESSION_ID`, `DECK_PANEL_ID` and `DECK_TERMINAL_ID`, and, when a client attaches, the six variables it renews in the session (`SSH_AUTH_SOCK`, `SSH_CONNECTION`, `SSH_TTY`, `DISPLAY`, `WAYLAND_DISPLAY`, `XAUTHORITY`).
- **`config.toml`**, by the daemon only.
- **Each terminal's foreground program** every second (its arguments are read to find the name, the name is kept) and **its directory** every 5 s.
- **The machine's host name**, to check a directory a shell reports.
- **Your terminal's answers**: its name and version, its colours, its cell size, whether it shows images and speaks the kitty keyboard protocol.
- **Fonts, on macOS**: to decide whether its icons will show, the client runs `defaults export` on the settings of iTerm2 or Terminal.app (the whole settings domain, to find one font name), runs `fc-list` and looks into the font folders. Nothing of it is kept, sent or logged.
- **Hook events**, once `deck hooks install` ran: an agent runs `deck agent-event` with a description of the event on stdin (at most 1 MiB is read). For Claude Code and Codex, deck takes the event's name, the notification type, the session id and the first line of the prompt (200 characters) and drops the rest (the transcript's path, tool inputs and outputs) before anything is sent to the daemon. OpenCode's plugin passes six kinds of event whole to the daemon, which keeps the same three things: the status, the session id and the first line of the first prompt.
- **Agent sessions, only when the restore dialog opens**: the newest 100 transcripts under `~/.claude/projects` (256 KiB of each), up to 100 rollouts under `~/.codex/sessions` (up to their first message, 4 MiB at most), and the output of `opencode session list` run in your home directory. For each session deck shows an id, a title (the first line of its summary or first prompt), a directory and a time. They are never stored or logged.

## What deck passes on

- **To your terminal**: copies, as OSC 52 (what a program copies, what you select with the mouse, what copy mode yanks); notifications (`<agent> needs you · <space> › <panel>`); images; questions about its features. deck sets no window title.
- **To programs inside deck**: the session's environment, the `DECK_*` variables, `TERM_PROGRAM=deck` and deck's version, and, when they ask, your terminal's name, version and colours. A program on a remote host reached with `ssh` can ask too, as it could without deck.
- **To any program that runs as you**, through the [Control API](control-api.md): every session's names, titles, directories, pids, agent kinds, statuses and session ids, and the screen and scrollback of every terminal (`deck capture`). `deck stats` adds the daemon's memory, descriptors and CPU time; shell completion shows session, space and panel names. A coding agent that uses these commands sends what it reads to its model provider: the `deck` skill tells agents to read only terminals they started or you pointed them to, and nothing enforces that.
- **Programs deck starts by itself**: your shell, an agent after a restart (`resume_agents`, on by default), `opencode session list` for the restore dialog (which can start OpenCode's background service, with the session's environment), and on macOS `defaults` and `fc-list`. What those programs do on the network is theirs.

## Things that may surprise you

- **Selecting text with the mouse copies it** to your clipboard when you release the button.
- **A panel's name can show your words.** A panel you did not rename is named after its terminal's title or after the first line of the first prompt you gave its agent, cut to 32 cells. That name is in desktop notifications (and their history, a lock screen, a shared screen), in `deck ls`, in shell completion menus and in `state.json` until it changes. `notify = "bell"` or `"none"` ([config](config.md)) keeps it out of notifications; a panel you rename keeps your name.
- **The restore dialog shows the first line of recent conversations of every project**, not only the one you are in.
- **A session keeps the environment of the shell that created it**, tokens included, for its whole life, and gives it to every terminal. After a restart, every restored session gets the environment of the `deck` that started the daemon: a token you rotated stays in a running daemon until `deck kill-server`.
- **Agents reopen their conversations after a restart** (`resume_agents = true`).
- **`--env` values and program arguments given through the Control API are saved** in `state.json`, in plain text, until their terminal closes, and like the text of `deck send <text>` they are command-line arguments that other users of the machine can see with `ps`. For a secret, use `deck send --terminal <id> -`, which reads the text from stdin.
- **`deck hooks install --print` prints the whole settings file**, keys included.

## Deleting what deck stores

- `deck kill-server --forget` stops the daemon (its memory goes with it) and deletes the saved state: `state.json` and every `state.json.bad-*`.
- The log (`daemon.log`, `daemon.log.1`) and the backups of agent configs (`backups/`) are deleted by hand.
- `deck hooks uninstall <agent>...` and `deck skills uninstall <agent>...` remove what deck added to the agents.
- To erase everything: `deck kill-server`, then delete the state directory. The runtime directory holds nothing about you once the daemon stopped.

## Installing and building

Installing is where the network is used: `install.sh` and Homebrew download a release from GitHub, and a build from source downloads crates from crates.io and, once, the terminal library `libghostty-vt` from `release.files.ghostty.org`. The built binary connects to nothing.

## For contributors

- The end-to-end tests write screen captures to `target/e2e-captures/` and the stress suite its numbers to `target/stress/`. Test daemons get your environment (minus `DECK*` and `ENV`), so a capture can show one of its values; `target/` is ignored by git.
- Recordings for the [compat suite](compatibility.md#recording-a-new-one) are committed, so the recorder runs each program in a scratch home with a cleared environment and refuses output that shows your home, user or host name or a path outside the scratch directory. It does not catch everything (an e-mail address, encoded text): read a new recording before you commit it.
- `crates/deck/tests/privacy.rs` gives a daemon random secrets in every way data comes in and checks that no file under its home holds them after an upgrade, a failed upgrade, a stop and a restart.
