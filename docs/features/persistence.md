# Persistence

Sessions survive the daemon stopping: after `deck kill-server`, a crash, a logout or a reboot, the next `deck` brings back every session with its projects, panels, terminal grid, names, ratios and focus.

## What comes back

- Sessions, projects, panels and terminals with their ids, names, order, grid ratios, the focused terminal of each panel, the active project and panel of each session, collapsed projects, and which session was attached last (the session picker's order).
- Every terminal starts again with a fresh process: a shell, or the explicit program and arguments it was started with. Shells start in the directory they were last in (see [Where a shell is](#where-a-shell-is)). If that directory is gone, in the project's directory; if that is gone too, in your home directory.
- A terminal that cannot start (its program no longer exists) is dropped and the next client shows a notice naming it; its panel goes if it was the only terminal. A restored terminal that fails at once is removed the same way, and its session stays.

A directory that is not valid UTF-8 cannot be saved exactly: that terminal starts in its project directory instead.

## Where a shell is

deck follows each terminal's working directory, so a restored shell comes back where you left it, and a new terminal opens where the focused terminal of its panel is (a new panel still opens in the project's directory; if the focused terminal's directory is gone, a new terminal falls back to the focused terminal's starting directory, then the project's directory).

- A shell that reports its directory with OSC 7 (`ESC ] 7 ; file://<host>/<path> BEL`) is followed at once. The host may be empty, `localhost` or the machine's host name; the path is percent-decoded. A terminal that has reported once is never looked up again.
- For the others, deck asks the operating system every 5 s for the directory of the terminal's own process (the shell, or the program started explicitly). A job running in the foreground is ignored: its directory is not where the shell is. A `cd` is therefore saved up to 5 s plus the 500 ms save window later.
- Many shells do not send OSC 7 unless configured to. In zsh or bash, `printf '\033]7;file://localhost%s\007' "$PWD"` from the prompt command (`precmd` or `PROMPT_COMMAND`) makes the change instant.

## What does not

- Scrollback and what was on the screen. A restored terminal starts at 80×24 until a client attaches.
- Whatever ran inside a shell (an editor, a dev server): only the shell itself, or an explicit program, is started again. A coding agent is the exception: it is reopened on its conversation, see [agents](agents.md#resume).
- Environment variables of the `deck` that created a session. Restored terminals get the environment of the `deck` that started the new daemon, plus any variables their launch added; `DECK_*` variables are regenerated. The state file never holds your environment.

## The file

`state.json` is in the state directory (`~/.local/state/deck/`, or `$XDG_STATE_HOME/deck/`), with mode `0600` in a `0700` directory. The daemon rewrites it 500 ms after a change, atomically, and once more when it stops; a daemon killed with `SIGKILL` loses at most the last 500 ms of changes.

- An unreadable file is moved to `state.json.bad-<unix seconds>`, deck starts with no sessions and shows `previous state could not be read; see <log>`.
- A file written by a newer deck (a higher `version`) is left alone and `deck` prints `state.json was written by a newer deck (version <n>); upgrade deck or run "deck kill-server --forget"` and exits 1.

## Commands

- `deck kill-server` saves and stops; `deck` restores.
- `deck kill-server --forget` stops the daemon and deletes the saved state, so the next `deck` starts with a new `main`. With no daemon running it deletes the state and prints `state cleared`.
- `deck ls` with no daemon running and saved sessions prints `no daemon running; <n> saved sessions will be restored by "deck"` (and starts nothing).

## Limitations

- After `kill -9` of the daemon, child processes that ignore `SIGHUP` (`nohup`, their own `setsid`) keep running, and the restored daemon starts explicit programs again next to them.
- A shell without OSC 7 is only looked at every 5 s: a `cd` followed within seconds by `kill -9` of the daemon is lost.
- Terminals made with `new-terminal --keep` are saved with `keep`. One that had exited is not restored; one that was running is started again and stays after it exits (see [Control API](control-api.md#kept-terminals)).
