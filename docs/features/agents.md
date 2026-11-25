# Agents

Read when you need how deck shows what a coding agent (Claude Code, Codex, OpenCode) is doing.

## States

A terminal that runs an agent has an agent state: its kind, a status and a source.

| Status    | Glyph | Meaning                                                                               |
| --------- | ----- | ------------------------------------------------------------------------------------- |
| `waiting` | `!`   | The agent needs you (warning colour, bold).                                           |
| `working` | `⠋`…  | The agent works: a spinner (glyph tiers: one frame every 100 ms; images: 8 fps).      |
| `active`  | `⠋`…  | Found by its process, and it printed in the last 2 s (accent colour, like `working`). |
| `idle`    | `●`   | The agent is done and waits for a prompt.                                             |
| `quiet`   | `○`   | Found by its process, and it printed nothing for 2 s.                                 |
| none      |       | No agent: nothing is drawn.                                                           |

Urgency, most to least: `waiting`, `working`, `active`, `idle`, `quiet`, none. The sidebar shows, in the status slot at the right end of a panel row, the icon of the panel's most urgent terminal, in a two-cell slot (the glyph, then a blank cell). The icons are fixed (see [config](config.md#icons)); the spinner has 10 frames in Unicode and 6 in Nerd Font glyphs, one frame every 100 ms in either, and the colours come from the theme: the spinner `accent`, `waiting` `warning` and bold, `idle` `ok`, `quiet` `muted`.

A panel with an agent is named after it in the sidebar: the agent's title without its spinner, else the first line of its first prompt (see [sidebar](sidebar.md#the-sidebar)). A grid border title for an agent terminal reads `<status slot> <kind> · <name>` in place of the number (`╭─ ⠹  claude · Fix login bug ──╮`, the slot two cells wide). There is no status icon without an agent. In Ghostty and kitty the spinner is Hugeicons' `Loader`, eight spokes that fade behind the brightest one, turning once a second, one spoke at a time (the same frame on every spinner); in other terminals it keeps its glyph frames. The client redraws the spinner only while one spins.

## Detection by process

Every second the daemon asks the operating system for the foreground process of each terminal's PTY (`sysctl` on macOS, `/proc` on Linux) and compares its program name with `[agents.<kind>] process` (a list of names; defaults: `claude`, `codex`, `opencode`). A login dash and a script interpreter (`node claude.js`) are looked through, and an extension is ignored. The same tick sets each terminal's `program` to that name (`ls --json` shows it) and leaves it as it was when the process cannot be read. A match gives the state `source: process`: `active` while the terminal printed in the last 2 s, else `quiet`. When the agent leaves the foreground the state is cleared.

```toml
[agents.claude]
process = ["claude", "my-claude-wrapper"]
```

An unknown kind or field under `[agents]` is a warning.

## Hook events

`Request::AgentEvent` sets the state with `source: hooks`, which wins over the process: `status`, `session_id` and the first line of a `prompt` (at most 200 characters, the first one of a session). A new `session_id` replaces the stored one and resets the first prompt. Rules:

- A hook state is cleared 3 s after the agent left the foreground.
- A hook `working` with no event and no output for 60 s falls back to the process state.
- Typing into a terminal whose hook status is `waiting` sets it to `working` (the user answered). Focus reports do not count.

`deck ls --json` shows `"agent": null` or `{ "kind", "status", "source", "session_id" }` per terminal. The hooks that send these events are installed with `deck hooks install` (next section).

## Hooks

`deck hooks install claude codex opencode` makes the agents report what they do, so deck knows when they work, wait for you or finish. It edits only the agents' own config files, only when you run it.

| Command                           | What it does                                                                 |
| --------------------------------- | ---------------------------------------------------------------------------- |
| `deck hooks install <agent>... `  | Adds deck's hooks; `--print` shows what would be written and writes nothing. |
| `deck hooks uninstall <agent>...` | Removes only deck's hooks.                                                   |
| `deck hooks status`               | One line per agent: `installed`, `not installed` or `stale`.                 |

- **Claude Code**: `$CLAUDE_CONFIG_DIR/settings.json` (default `~/.claude/settings.json`), under `hooks`: one entry per event (`SessionStart`, `UserPromptSubmit`, `PostToolUse`, `PermissionRequest`, `Notification`, `Stop`, `SessionEnd`), no matcher, `"async": true`, so the agent never waits for deck.
- **Codex**: `$CODEX_HOME/hooks.json` (default `~/.codex/hooks.json`) with the `{ "description", "hooks" }` wrapper: `SessionStart`, `UserPromptSubmit`, `PostToolUse`, `PermissionRequest`, `Stop`, `Interrupt`, `SessionEnd`, each with `"matcher": ""` and `"timeout": 2`. Codex only runs hooks you reviewed: open `/hooks` in Codex and trust them. Codex keys a trust entry by a hash deck cannot compute, so `status` cannot tell whether you did and says `installed (trust them in /hooks if Codex asks)`. `status` also tells when deck entries are found in `config.toml`; Codex loads both files and warns about duplicates.
- **OpenCode** (v2): a TUI plugin, `plugins/deck/tui.js`, in `$OPENCODE_CONFIG_DIR/plugins/` when that variable is set, else `$XDG_CONFIG_HOME/opencode/plugins/` (default `~/.config/opencode/plugins/`). It is a TUI plugin, not a server plugin, on purpose: by default OpenCode v2's TUI connects to one shared background server that is started detached with the environment of whichever terminal started it first, so a server plugin would report every session with that terminal's `DECK_TERMINAL_ID`. Each TUI loads the subdirectories of its `plugins/` directory and runs a TUI plugin (`tui.js`, a default export `{ id: "deck", setup(ctx) }`) in its own process, where `DECK_TERMINAL_ID` is the right one; the server finds no server entry in the directory and loads nothing. `setup` calls `ctx.data.listen`, which gets the events of every session on the shared server, so the plugin keeps only the events of its own sessions: the session of the route it shows and its root (`ctx.ui.router.current()`, `ctx.data.session.root`), and the subagent sessions a `session.created` with a `parentID` of one of them starts. It forwards `session.created`, `session.status`, `session.idle`, `permission.asked`, `permission.replied` and `session.inbox.enqueued` by running `deck agent-event --agent opencode --event <type>` with the event (`{ "type", "data", … }`) as JSON on stdin, through `node:child_process` `spawn`, without waiting for it; the calls are chained on one promise queue so the events arrive in order; the cleanup it returns unsubscribes. The plugin interprets nothing else: the daemon keeps one `OpenCodeTracker` (`deck-core::agents`) per terminal, which drops events of subagent sessions (a session whose `session.created` has a `parentID`; at most 256 are remembered) except their `permission.asked` and `permission.replied`, which still set `waiting` and `working` without replacing the root's session id, maps `session.status` `busy`/`idle` (and the deprecated `session.idle`) to `working`/`idle`, `permission.asked` to `waiting` and `permission.replied` to `working`, and takes the first prompt from `session.inbox.enqueued` when its `item.type` is `user` (`item.payload.text`). `retry` statuses and other events are ignored. The tracker reads v1's shape too (`{ "type", "properties" }`, with `info.parentID`, `message.updated` and `message.part.updated`), but deck no longer installs a v1 plugin: OpenCode v2 rejects one with "Plugin must export a default definition with an id and an effect or setup function". `install` removes a `plugins/deck.js` that deck wrote (it keeps a backup), `uninstall` removes both files, and `status` reports such an old `deck.js` as `stale`, as well as a plugin whose deck binary is gone or that lacks an event. A `tui.js` or `deck.js` that deck did not write is never replaced or removed. Checked on 23 Nov 2026 against the OpenCode source (`anomalyco/opencode`, branch `dev`, from `raw.githubusercontent.com`): `packages/core/src/plugin/module.ts` and `packages/plugin/src/host.ts` (entrypoints `server` and `tui`), `packages/tui/src/plugin/discovery.ts` (each subdirectory of `plugins/`), `packages/plugin/src/tui/{plugin,context}.ts` (the TUI `Context`: `data.listen`, `ui.router`, `ui.tabs`), `packages/cli/src/services/server-connection.ts` (the shared server), and `packages/schema/src/{session-status-event,permission,session-event,session-inbox}.ts` for the payloads; and against the installed OpenCode v2.3.2 with two TUIs on one server (`opencode serve` and `opencode --server`, a temp config dir and XDG dirs, a fake `deck` that logs `DECK_TERMINAL_ID` and stdin): each TUI's plugin saw the events of both sessions and forwarded only its own session's `session.created` and `session.inbox.enqueued`, each with its own terminal id, and `opencode debug agents --standalone` loaded the directory without a warning (checked with an earlier version of the plugin that also claimed its open tabs; the filter only narrowed since). Not seen live: `session.status`, `permission.asked` and `permission.replied` (they need a model), so their fields come from the source schemas, and the TUI plugin's own-session filter was only exercised with the route and one session per TUI (subagents come from the source). Open tabs are not claimed: OpenCode shares the tab list between the TUIs of a directory. A session this terminal has shown stays its own, so opening another terminal's session here (from the session list, say) sends that session's events to both terminals.
- **Safe writes**: a symlinked file is written in place (the symlink stays), after copying the current file to `$XDG_STATE_HOME/deck/backups/<agent>-<file>-<unix-seconds>` (`$DECK_HOME/state/backups/` when `DECK_HOME` is set; a name that exists gets `-1`, `-2`... and no backup is overwritten); the new content goes to a temp file next to it and is renamed over it. The JSON is parsed and written back with 2-space indentation and its key order kept, so the first install may reformat the file. The temp file gets the target's permissions from the start. The plugin file is written the same way (backup, temp file, rename) and has no JSON to edit. A file that is not valid JSON, or a symlink to a file that does not exist, is left untouched and reported.
- **Idempotent**: deck's entries are found by their `deck agent-event` command and replaced, never duplicated.
- **The command** is `deck agent-event --agent <agent>` when the running `deck` is the one on `PATH`, else the absolute path of the binary, and `install` warns that moving or rebuilding it makes the hooks `stale`. A bare `deck` is `stale` too when no `deck` is on `PATH`.

`deck agent-event --agent <claude|codex|opencode> [--event <name>]` is what the hooks run (hidden from `deck --help`). It reads the payload from stdin (at most 1 MiB) and sends it for `DECK_TERMINAL_ID`: for Claude Code and Codex it takes the event from `--event` or `hook_event_name` and maps it; for `opencode` it forwards the raw payload unmapped, and the daemon's `OpenCodeTracker` interprets it. It prints nothing, always exits 0 (a wrong argument too), does nothing outside deck or without a daemon, never starts one and gives up after 500 ms in total, enforced by a watchdog; it reads stdin to the end first so the agent never gets a broken pipe.

| Agent  | Event                                                           | Status    |
| ------ | --------------------------------------------------------------- | --------- |
| Both   | `SessionStart`                                                  | `idle`    |
| Both   | `UserPromptSubmit` (records the prompt)                         | `working` |
| Both   | `PostToolUse`                                                   | `working` |
| Both   | `PermissionRequest`                                             | `waiting` |
| Both   | `SessionEnd`                                                    | `none`    |
| Claude | `Stop`; `Notification` with `idle_prompt`                       | `idle`    |
| Claude | `Notification` with `permission_prompt` or `elicitation_dialog` | `waiting` |
| Codex  | `Stop`, `Interrupt`                                             | `idle`    |

The `session_id` of every payload is recorded. Other events and `notification_type` values are ignored. Formats checked on 22 Nov 2026 against the Claude Code hooks reference (the docs snapshot of Context7, whose Claude Code releases run up to v2.1.278; the docs carry no per-field version) and the Codex source on its `main` branch (no release tag: `codex-rs/hooks`, `codex-rs/protocol/src/hooks.rs`) (`codex-rs/hooks`, `codex-rs/protocol/src/hooks.rs`): the event names, `hook_event_name`, `session_id`, `prompt` and `notification_type` fields, Claude Code's `"async": true` on command hooks, and Codex's `hooks.json` handler fields (`type`, `command`, `timeout`, `async`, ...).

## Attention

Hook-sourced states only (a process-only `active` to `quiet` pause happens all the time and never counts). When a terminal goes from `working` to `waiting` or `idle` and nobody is looking at it, its panel is **unseen**, and the daemon sends one `ServerMsg::AgentAlert { kind, status, project, panel }`. A [running command](sidebar.md#running-commands) that ends while nobody is looking marks its panel unseen too, without an alert.

- **Looking** means the panel is the session's active panel and a client of the session has its outer terminal focused. A client reports focus with `ClientMsg::Focus(bool)` from the outer terminal's focus events, and again on every attach once it saw one; a client that never reported counts as focused, so a terminal without focus reporting never notifies for the active panel.
- **Unseen** panels are bold in the sidebar with the unseen marker (`•`, accent, a two-cell slot) after the name, until the panel is the active panel of a focused client. A collapsed space shows `!<n>` (warning) after its name when `n` of its panels wait.
- **The alert** goes to the one client of the session that sent input or a viewport last, and the client writes it to the outer terminal as `notify` says: `"osc9"` (default) `ESC ] 9 ; <body> BEL`, `"bell"` a lone `BEL`, `"none"` nothing. The body is `<kind> needs you · <space> › <panel>` for `waiting` and `<kind> finished · <space> › <panel>` for `idle`; control characters are stripped from the names. OSC 9 shows a desktop notification in iTerm2, Ghostty and WezTerm and is ignored by terminals that do not know it (kitty uses OSC 99 and is not targeted). Inside tmux or over ssh it reaches the desktop only if they pass it through.
- **Jump**: `prefix a` (`next_attention`) focuses the next waiting terminal in sidebar order after the current one, wrapping; with none waiting, the next idle terminal in an unseen panel; with none, it flashes `no agent needs you`. Panels of collapsed spaces are included.

Limitations: a session with no attached client alerts nobody (its panels are still marked unseen for the next attach), and the session picker shows no agent status.

```toml
notify = "bell"
```

## Starting an agent in a new panel

The action `NewPanelWith` (sent by the [`New` menu](sidebar.md#the-new-button-and-menu) and by DECK-11's restore) starts a new panel in a space that runs an agent's `[agents.<kind>] command` (defaults `claude`, `opencode`, `codex`), in the space's directory. The command is run directly, with no shell, as the program of an explicit-program terminal (like `deck new-terminal -- <program>`), so the panel closes when the agent quits. The panel is added, active and focused like `prefix c`, and is named at once after its program (`claude`).

```toml
[agents.claude]
command = "claude --model fast"
```

`command` is a program and arguments split on whitespace, with the same limits as a resume template (no quote, backslash, `$` or other shell syntax) and no `{session_id}`. An empty command, one that breaks those limits, or a program with a `/` that is not an absolute path is a config error naming the key (`agents.claude.command`).

Before anything is created the daemon looks the program up: a program with a `/` must be an executable file, any other is searched in the `PATH` of the session's environment, which is the one of the client that created the session (it is not refreshed when another client attaches, and the daemon's own `PATH` is never used). When it is missing, no panel is created, focus stays and the client flashes `"opencode" not found in PATH (set [agents.opencode] command)`. If the program is found but cannot be started (the space's directory is gone, say), the error is the operating system's text and nothing is created either. A script with a bad interpreter line starts, fails and closes its panel like any program that exits at once.

`deck new-panel -- <program>` passes its own program and does not use `command`.

The terminal is saved and restored as an explicit-program terminal. When the program's name is in the kind's `process` list (the default `command` always is), the agent is detected and [resumed](#resume) after a daemon restart. For a `command` whose program is not in `process`, the agent is not detected, and a restart relaunches the command fresh.

## Listing and restoring sessions

The daemon lists each agent's recent sessions for the restore dialog (`Request::ListAgentSessions { kind, limit }`, the 50 newest by default, `limit` clamped to 1 to 100) and can start a panel that resumes one (`NewPanelWith` with `PanelLaunch::Resume { kind, session_id, cwd }`). Both use the environment of the client that created the session (`HOME`, `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `XDG_DATA_HOME`, `PATH`), never the daemon's, and a listing needs an attached session (a control client gets `no session`).

| Agent       | Where sessions are read from                                                                                    | Bounds                                                                                                                    |
| ----------- | --------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| Claude Code | `$CLAUDE_CONFIG_DIR/projects/*/<uuid>.jsonl`, default `~/.claude/projects` (one level; `agent-*.jsonl` skipped) | The newest `limit × 2` files by modification time, 256 KiB read from the head of each                                     |
| Codex       | `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-*.jsonl`, default `~/.codex/sessions`                                  | Day directories newest first until `limit × 2` rollouts; read line by line up to the first prompt, at most 4 MiB per file |
| OpenCode    | `opencode session list --max-count <limit> --format json`, run in every space root and in `~`                   | At most 2 at a time, 8 s in all, 4 MiB of output each                                                                     |

- Each request is answered from its own task, so a slow agent delays neither the others nor the connection's input and actions. The answer, `Response::AgentSessions { kind, sessions, error }`, arrives when it is ready, out of order with other answers; clients route it by `kind`. A failure (a command that is missing, prints no JSON array or times out) is the `error` text (`could not run opencode session list`, `timed out`) with an empty list, never a `Response::Error`; a missing agent directory is an empty list without an error.
- Codex candidates are chosen by file name (creation date) and sorted by modification time, so an old session resumed recently can be missed when more than `limit × 2` newer rollouts exist. OpenCode sessions of a directory that is neither a space root nor `~` are not listed. The program of the OpenCode listing is the first word of `[agents.opencode] command`.
- `PanelLaunch::Resume` runs the agent's `resume` template with the session id (invalid ids are refused with `not a valid session id`; the program is looked up in the session's `PATH`, and a missing one names `resume`: `set [agents.claude] resume`). The terminal starts in the session's directory, or in the space root with the warning `<dir> no longer exists; started in <space root>` when it is gone. The panel is saved and resumed after a daemon restart like any agent panel.
- Titles and directories go only to the client that asked: the daemon log has counts and timings only, and `state.json` has none of them.

## The restore dialog

The `Resume` button at the right of the sidebar's bottom bar, `Option+Space R` (binding `restore`, in the Session group of the help) and `R` in sidebar focus open a centered overlay, 90% of the window wide (at least 60 columns) and 80% high (at least 10 rows), with a filter line, three columns (`Claude Code`, `OpenCode`, `Codex`, each with its count) and a hint line. With no space, they flash `no space to restore into` instead.

- **Opening** sends three `ListAgentSessions { kind, limit: 50 }` requests at once. Each column shows a muted `loading…` until its own answer comes, then its sessions, its error in the warning colour, or a muted `no sessions`. One slow agent never holds the others up, and closing the dialog drops answers that arrive later. An answer that arrives late from a dialog just closed can briefly fill a reopened one, until the fresh answer replaces it.
- **Rows**: two lines per session, the title and, in muted text, `<directory> · <age> ago` (the directory cut from the left, `~` for the home directory). The selection starts on the first row of the first column that has any; each column scrolls on its own to keep it in sight. Each column heading starts with its agent's icon, two cells and a space (`✻  Claude Code (50)`), an image where images are on. Below 60 window columns one column shows at a time, `‹ ✻  Claude Code (12) ›`, and `Tab` switches.
- **Filter**: typing filters all three columns at once; every word must be in the title or the directory (case-insensitive). The selection moves to the nearest row. `Esc` clears the filter, a second `Esc` closes. [Keys](keys.md#restore-dialog).
- **Mouse**: hovering a row highlights it, a click selects it and a click on the selected row resumes it, the wheel scrolls the column under the pointer, a click on a header moves the selection to that column, a click outside closes the dialog.
- **Resuming** opens a new panel in the space whose root holds the session's directory (the longest root; the active space on a tie), else in the active space, running the agent's `resume` command in the session's directory (see above for a directory that is gone). The dialog closes at once and the new panel is active and focused. A refusal (`"claude" not found in PATH ...`, `not a valid session id`) is a 4 s error flash and creates nothing.

## Resume

After `deck kill-server` or a reboot, each terminal that had an agent in the foreground when the state was saved comes back with that agent reopened on its conversation (`resume_agents = true`, the default). The saved state keeps, per terminal, the agent's kind and its `session_id` (from hooks; `null` when only the process was seen). The state is saved 500 ms after the agent or its session id changes, and once more, before the terminals are hung up, when the daemon stops. While the daemon is shutting down it ignores agent events and detection changes, so an agent's `SessionEnd` hook or the hang-up's exits cannot clear what resume needs.

- **A shell terminal** is relaunched as `/bin/sh -c '<resume command>; exec "$0" "$@"' <shell> <shell_args...>`: `sh` runs the agent, then replaces itself with your shell and its `shell_args`. Quitting the agent leaves you in your shell, in the same directory. Nothing is typed into a shell or written to its history, and the terminal's launch spec stays the plain shell, so a later restore without an agent starts a plain shell.
- **A terminal that was started with the agent as its program** (`deck new-terminal -- claude`) is relaunched with the resume command as its program and arguments, in its original directory with its original `--env`. No shell is involved. The program is the one of the configured template (found on the daemon's `PATH`), so the terminal's original arguments (`--model x`, say) are dropped. A terminal whose program is not an agent is relaunched as saved.
- The `resume` template is used when a valid session id is known, else `resume_latest`.

```toml
resume_agents = true # false: restore a plain shell

[agents.claude]
resume = "claude --resume {session_id}"
resume_latest = "claude --continue"
```

| Agent    | `resume`                          | `resume_latest`       |
| -------- | --------------------------------- | --------------------- |
| Claude   | `claude --resume {session_id}`    | `claude --continue`   |
| Codex    | `codex resume {session_id}`       | `codex resume --last` |
| OpenCode | `opencode --session {session_id}` | `opencode --continue` |

A template is a program and arguments, split on whitespace. `{session_id}` may appear only as a whole argument of `resume`, never in the program, and `resume_latest` has none. A quote, a backslash, `$`, a backtick or any other shell syntax (`; & | < > ( ) * ? [ ] { } # ~ !`) is a config error that names the key (`deck config check` shows it), because the command is joined with spaces and run by `sh -c`. A session id is used only when it matches `^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$` (it can never start with `-`); any other id is dropped, logged, and `resume_latest` runs. The program of a template is found on the daemon's `PATH`. The resumed agent is detected again like any other: the `sh` wrapper of a resumed shell terminal is recognised as the agent it runs, and the restored terminal has its agent state from the start, so a second restart resumes it again.

Limitations:

- **Several agents of one kind in one directory** each resume their own session when their session ids are known (hooks installed). Without ids every one of them runs `resume_latest`, which opens the same most recent conversation each time.
- **Staleness.** After a crash or `kill -9` the restore uses the last save: an agent that quit in the last 500 ms may be relaunched, and a `session_id` that changed in that window may point at the previous conversation. A hook-reported agent also stays in the state for up to 3 s after it leaves the foreground (see the hook rules above), so quitting the agent and stopping the server within 3 s relaunches it.
