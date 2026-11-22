---
branch: feat/control-api
backlog: DECK-05
---

# Control API

## Goal

Agents and scripts drive deck from the shell: list the tree, open panels and terminals (for example a test runner next to the agent), send text and keys, read a terminal's screen, and wait for a command to finish. Every command defaults to "here" using the `DECK_*` variables, prints plain text for people and JSON with `--json` for programs, and ships with a skill that teaches agents how to use it.

## Builds on

- [DECK-01 core-daemon-terminal](../2026-11-22-core-daemon-terminal/00-overview.md): the protocol, `LaunchSpec`, `KeySpec` and `keys::encode` ([02-core-types](../2026-11-22-core-daemon-terminal/02-core-types.md)), the daemon's `Modes` per terminal.
- [DECK-02 sessions-sidebar](../2026-11-22-sessions-sidebar/00-overview.md) and [DECK-03 grid-layout](../2026-11-22-grid-layout/00-overview.md): the actions behind every command (validation, lifecycle, room check, focus rules), `SessionView`, `deck ls`.
- [DECK-04 persistence](../2026-11-22-persistence/00-overview.md) (a backlog dependency): each terminal's tracked cwd in `SessionView`, the state file that sub-spec 01 extends with `keep`, and the daemon's platform module (`waitpid`).
- [architecture.md#agents](../../architecture.md#agents): the `DECK_*` variables.

## Non-goals

- A network API, an HTTP server or an MCP server. The socket and the CLI are the API.
- Subscriptions to a stream of events (beyond `deck wait`).
- Driving another user's daemon.
- Creating sessions or starting the daemon from a control command (`deck new` from DECK-02 does that).
- Installing the skill automatically: the README says how to link it.

## Decisions

- **Thin wrappers**: control requests resolve their targets to ids in the daemon and call the same functions as the DECK-02 and DECK-03 actions. No validation or lifecycle rule is written twice. Launches reuse `LaunchSpec`; the tree is a list of `SessionView`s.
- **Targets**: ids are daemon-wide and unique across kinds (sessions, projects, panels, terminals share one counter), so `--terminal <id>` and `--panel <id>` need no session, and `deck focus <id>` / `deck close <id>` take a bare id. `--project` takes an id or a name within the target session; `--session` takes a name.
- **Defaults**: with no target option, the current place comes from `DECK_SESSION_ID`, `DECK_PANEL_ID` and `DECK_TERMINAL_ID`. An explicit `--session` wins over them and turns the environment defaults off (the panel and terminal defaults then come from that session's active panel and focused terminal). Outside deck, the default session is the most recently attached one, else the only session, else an error. A stale id in the environment (its terminal or panel is gone) is an error, never silently replaced.
- **Never yourself by accident**: `send`, `send-keys` and every `wait` mode refuse to act on the caller's own terminal unless `--terminal` names it explicitly (`wait --exit` refuses it always). `capture` may read the caller's own terminal.
- **Focus**: terminals and panels created through the API do not take focus unless `--focus` is given, so an agent never steals the user's focus.
- **Exited terminals**: a terminal created with `--keep` stays after its process exits, shows `exited <code>` in its title, and is closed by the user (`^Space x`, without confirmation for an exited terminal) or `deck close`. `deck wait --exit` works with or without `--keep`: the daemon keeps a removed terminal's exit status for 60 s for waiters. Exit codes come from the daemon's own `waitpid` on the child pid (in `deck-daemon::platform`), because portable-pty 0.9 reports a death by signal as code 1 with only a text name; a death by signal `n` is `128 + n`. A kept terminal is saved with `keep` (DECK-04's state file); after a restart, kept terminals that had exited are not restored, and running ones are relaunched with `keep` set.
- **Output and exit codes**: plain text by default, one JSON document with `--json`. Exit code 0 on success, 1 on any error (message on stderr, or `{"error": "..."}` on stdout with `--json`), 124 on a `wait` timeout. `wait --exit` reports the process's exit code as output (`3`, or `{"exit": 3}`) and itself exits 0, so a program exiting 1 or 124 is never confused with an error or a timeout.
- **No daemon start**: control commands never start a daemon and never create sessions. Errors come in this order: `no deck daemon running`, then `no session` (or the missing target).
- **Socket**: `DECK_SOCKET`, when set, wins over the paths of [architecture.md#paths](../../architecture.md#paths).
- **Agent skill**: `skills/deck/SKILL.md` in this repo (Agent Skills format), readable by Claude Code, Codex and OpenCode once the user links it into their skills directory. `deck guide` prints the same text.

## Security

The trust boundary is the user's uid: the daemon accepts only same-uid peers ([architecture.md#process-model](../../architecture.md#process-model)), and any same-uid process, including a sandboxed or prompt-injected agent, can run commands and type into any terminal through `new-terminal` and `send`. This is accepted: it is the same power the user's shell has. Consequences written into this spec:

- `LaunchSpec.env` given through the API cannot override `DECK_*` variables (they are set last).
- Text returned by `capture` and `wait --text` is untrusted data; the skill tells agents never to follow instructions found in it.
- Agent sandboxes (Claude Code, Codex) may block the Unix socket; the skill and the README say how that shows up (`no deck daemon running` or a permission error) and that the sandbox has to allow the socket path.

## Sub-specs

| Sub-spec                  | Type      | Crate                                  | Summary                                                                                     | Depends | Status |
| ------------------------- | --------- | -------------------------------------- | ------------------------------------------------------------------------------------------- | ------- | ------ |
| [01](01-tree-commands.md) | shippable | deck, deck-core, deck-daemon, deck-tui | `ls --json`, `new-project`, `new-panel`, `new-terminal`, `focus`, `close`, `rename-panel`; `--keep` | DECK-04 | todo   |
| [02](02-terminal-io.md)   | shippable | deck, deck-core, deck-daemon           | `send`, `send-keys`, `capture`, `wait`                                                      | 01      | todo   |
| [03](03-agent-skill.md)   | shippable | deck, root (`skills/`, `README.md`)    | `skills/deck/SKILL.md`, `deck guide`, README                                                | 02      | todo   |
## Affected docs

- `docs/features/control-api.md` (new): every command, its options, output, JSON shapes and exit codes, and the security notes.
- `skills/deck/SKILL.md` (new) and `README.md`: the skill and how to install it.
- `docs/architecture.md#protocol`: the control requests; `#process-model`: the control commands are `ls`, `new-project`, `new-panel`, `new-terminal`, `focus`, `close`, `rename-panel`, `send`, `send-keys`, `capture`, `wait`.

Reviewed by spec-reviewer on 22 Nov 2026.
