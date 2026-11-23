# Agents

Read when you need how deck shows what a coding agent (Claude Code, Codex, OpenCode) is doing.

## States

A terminal that runs an agent has an agent state: its kind, a status and a source.

| Status    | Glyph | Meaning                                               |
| --------- | ----- | ----------------------------------------------------- |
| `waiting` | `!`   | The agent needs you (warning colour, bold).           |
| `working` | `⠋`…  | The agent works: a spinner, one frame every 100 ms.   |
| `active`  | `⠋`…  | Found by its process, and it printed in the last 2 s. |
| `idle`    | `●`   | The agent is done and waits for a prompt.             |
| `quiet`   | `○`   | Found by its process, and it printed nothing for 2 s. |
| none      |       | No agent: nothing is drawn.                           |

Urgency, most to least: `waiting`, `working`, `active`, `idle`, `quiet`, none. The sidebar shows, in the status column of a panel row, the glyph of the panel's most urgent terminal. The glyphs and colours come from `[theme.glyphs]` (`spinner` is a list of frames, `waiting`, `idle`, `quiet`) and the theme colours; see the reference in `deck config default`.

A panel with an agent is named after it in the sidebar: the agent's title without its spinner, else the first line of its first prompt (see [sidebar](sidebar.md#the-sidebar)). A grid border title for an agent terminal reads `<glyph> <kind> · <name>` in place of the number. The status bar shows `project › <status> <icon> <name>` for the active panel (`project › ⠹ ✻ Fix login bug`), the same status glyph and icon as the sidebar row, so the state stays visible when the sidebar is hidden; there is no status glyph without an agent. The client redraws the spinner only while one spins.

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
- **OpenCode**: the plugin `deck.js`, in `$OPENCODE_CONFIG_DIR/plugins/` when that variable is set, else `$XDG_CONFIG_HOME/opencode/plugins/` (default `~/.config/opencode/plugins/`). It subscribes to `session.created`, `session.updated`, `session.status`, `session.idle`, `permission.asked`, `permission.replied`, `message.updated` and `message.part.updated`, and for each runs `deck agent-event --agent opencode --event <type>` with the event (`{ "type", "properties" }`) as JSON on stdin, through the plugin's `$` shell helper and without waiting for it; the calls are chained on one promise queue so the events arrive in order. The plugin holds no logic: the daemon keeps one `OpenCodeTracker` (`deck-core::agents`) per terminal, which drops events of subagent sessions (a session whose `info.parentID` is set, learned from `session.created` and `session.updated`; at most 256 are remembered) except their `permission.asked` and `permission.replied`, which still set `waiting` and `working` without replacing the root's session id, maps `session.status` `busy`/`idle` (and the deprecated `session.idle`) to `working`/`idle`, `permission.asked` to `waiting` and `permission.replied` to `working`, and takes the first prompt from the first `text` part of a `user` message (`message.updated`, then `message.part.updated`; at most 64 user message ids are remembered). `retry` statuses and other events are ignored. `status` is `stale` when the deck binary of the plugin is gone or an event is missing; a `deck.js` that deck did not write is never replaced or removed. Checked on 23 Nov 2026 against the OpenCode docs (Context7, `/anomalyco/opencode`: plugins, events, `session-status-event` schema, `OPENCODE_CONFIG_DIR`); payload field names of `message.updated` and `message.part.updated` follow the SDK types and were not run against a live OpenCode.
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

Hook-sourced states only (a process-only `active` to `quiet` pause happens all the time and never counts). When a terminal goes from `working` to `waiting` or `idle` and nobody is looking at it, its panel is **unseen**, and the daemon sends one `ServerMsg::AgentAlert { kind, status, project, panel }`.

- **Looking** means the panel is the session's active panel and a client of the session has its outer terminal focused. A client reports focus with `ClientMsg::Focus(bool)` from the outer terminal's focus events, and again on every attach once it saw one; a client that never reported counts as focused, so a terminal without focus reporting never notifies for the active panel.
- **Unseen** panels are bold in the sidebar with the `unseen` glyph (`•`, accent) after the name, until the panel is the active panel of a focused client. A collapsed project shows `!<n>` (warning) after its name when `n` of its panels wait.
- **The alert** goes to the one client of the session that sent input or a viewport last, and the client writes it to the outer terminal as `notify` says: `"osc9"` (default) `ESC ] 9 ; <body> BEL`, `"bell"` a lone `BEL`, `"none"` nothing. The body is `<kind> needs you · <project> › <panel>` for `waiting` and `<kind> finished · <project> › <panel>` for `idle`; control characters are stripped from the names. OSC 9 shows a desktop notification in iTerm2, Ghostty and WezTerm and is ignored by terminals that do not know it (kitty uses OSC 99 and is not targeted). Inside tmux or over ssh it reaches the desktop only if they pass it through.
- **The status bar** shows `<n> waiting` (warning) on its right while any terminal of the session waits, and the `a next` hint.
- **Jump**: `prefix a` (`next_attention`) focuses the next waiting terminal in sidebar order after the current one, wrapping; with none waiting, the next idle terminal in an unseen panel; with none, it flashes `no agent needs you`. Panels of collapsed projects are included.

Limitations: a session with no attached client alerts nobody (its panels are still marked unseen for the next attach), and the session picker shows no agent status.

```toml
notify = "bell"
```

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
