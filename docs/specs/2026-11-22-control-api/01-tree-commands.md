---
type: shippable
status: done
---

# 01 — Tree commands

## Scope

From any shell, the user or an agent lists deck's tree and creates, focuses, renames and closes projects, panels and terminals, including terminals that run a given command and stay after it exits. Owning crates: `deck` (CLI), `deck-core` (requests, target types), `deck-daemon` (handlers), `deck-tui` (the exited-terminal title and `⌥Space x` on it).

## Requirements

1. **Requests** (`PROTOCOL_VERSION` bumps), each a thin wrapper as the overview's decisions say. `Target` is `Id(u64)` or `Name(String)`; the daemon resolves it within the request's session.

   | Request                                                                          | Resolves to                | Calls (DECK-02 / DECK-03)                                        |
   | -------------------------------------------------------------------------------- | -------------------------- | ---------------------------------------------------------------- |
   | `Tree { session: Option<String> }`                                               | one session, or all        | returns `Vec<SessionView>`                                       |
   | `NewProject { session, name, root, focus }`                                      | session                    | `NewProject` action                                              |
   | `NewPanel { session, project: Target, name, launch: Option<LaunchSpec>, focus }` | project                    | `NewPanel` action, with the launch spec                          |
   | `NewTerminal { panel: PanelId, launch: Option<LaunchSpec>, keep, focus }`        | panel                      | `NewTerminal` action (room check included), with the launch spec |
   | `Focus { id }`                                                                   | terminal or panel          | `FocusTerminal` / `FocusPanel`                                   |
   | `Close { id }`                                                                   | terminal, panel or project | `CloseTerminal` / `ClosePanel` / `CloseProject`                  |
   | `RenamePanel { panel, name }`                                                    | panel                      | `RenamePanel` action                                             |

   A `LaunchSpec` with `program: None` is a shell; its `cwd` defaults as in requirement 3. Responses: `Created { id }`, `Tree(Vec<SessionView>)`, `Ok`, or `Error(String)`. `Request` and `Action` are separate enums that share some variant names; both call the same daemon functions.
   1a. **Changes to the DECK-02/03 action functions** (in `deck-daemon`, not to their protocol variants): the functions behind `NewProject`, `NewPanel` and `NewTerminal` gain `launch: Option<LaunchSpec>`, `focus: bool` and (for terminals) `keep: bool`. The attach client's actions call them with `launch: None` (a shell in the default cwd), `focus: true`, `keep: false`, so their behavior and the DECK-02/03 tests stay as they are. `focus: false` leaves the session's active project, panel and focused terminal unchanged.
   1b. **`SessionView` gains** (protocol version bump): per terminal `pid: Option<u32>`, `exited: Option<i32>` and `keep: bool` (the terminal `cwd` is already there from DECK-04); per session `attached_clients: u32` (the same count as `SessionSummary`). `ls --json`'s `attached` is `attached_clients > 0`.

2. **Target resolution** in `deck`, following the overview's decisions, with these errors: `no deck daemon running`, `no session` (outside deck, none exists), `no session named "<s>"`, `no terminal <id>`, `no panel <id>`, `<id> is not a terminal or panel` (for `focus`), `<id> is not a terminal, panel or project` (for `close`), `no project named "<name>" in session "<s>"`, and for stale environment ids `this terminal (<id>) no longer exists` / `this panel (<id>) no longer exists`.
3. **Commands** (all accept `--json` and `--session <name>`):
   - `deck ls`: without flags, DECK-02's session list (with its project column), unchanged. `deck ls --json`: `{"sessions":[{"id","name","attached","projects":[{"id","name","root","collapsed","panels":[{"id","name","name_source","active","terminals":[{"id","index","title","program","cwd","pid","focused","exited":null|code}]}]}]}]}`, built from the `SessionView`s. `deck ls --tree` prints the same tree as indented text.
   - `deck new-project <name> [--dir <path>] [--focus]`: `--dir` defaults to the current dir.
   - `deck new-panel [--project <id|name>] [--name <n>] [--cwd <dir>] [--focus] [-- <program> <args>...]`: `--cwd` defaults to the project root.
   - `deck new-terminal [--panel <id>] [--cwd <dir>] [--env K=V]... [--keep] [--focus] [-- <program> <args>...]`: without `--panel`, the current panel; without `--cwd`, the cwd of that panel's focused terminal, else the project root.
   - These three print the new id (`12`, or `{"id": 12}` with `--json`).
   - `deck focus <id>`: focuses a terminal (and its panel and project) or a panel. `deck close <id>`: closes a terminal, a panel or a project, without confirmation. Both print nothing (`{"ok": true}` with `--json`).
   - `deck rename-panel [--panel <id>] <name>`: sets the name with `name_source = Manual`.
4. **`--keep`**: the terminal stays after exit with `exited: Some(code)`; a death by signal `n` is reported as `128 + n`. The code comes from `deck-daemon::platform::wait_status(pid)`, a `waitpid` on the child pid that the daemon reaps itself (portable-pty's `ExitStatus` reports a signal death as code 1 with a platform-specific text), on macOS and Linux alike. Its title becomes `<title> · exited <code>` and its border and title are drawn muted. `Input` to it is dropped by the daemon. `⌥Space x` on an exited terminal closes it without the DECK-03 confirmation; on a running terminal the confirmation stays. A panel whose terminals have all exited (and are kept) stays until they are closed.
   4a. **Persistence of `keep`**: `TerminalState` (DECK-04) gains `keep: bool` with `#[serde(default)]`. On restore, a kept terminal that had exited is dropped (its panel follows the lifecycle rules, without a notice); a kept terminal that was still running is relaunched like any explicit program, with `keep` set.
5. **Commands with a program** get the same environment as shells (the session's client environment, `TERM`, `COLORTERM`), plus `--env` values, then the `DECK_*` variables, set last so `--env` cannot override them. They run directly (no shell). `--` separates deck's options from the program's.
6. **No daemon start**: every command in this sub-spec fails with `no deck daemon running` (exit 1) when none is running and never starts one, except `deck ls`: plain, it keeps DECK-01's `no sessions` and exit 0; with `--json` it prints `{"sessions": []}` and exits 0. Usage errors from the argument parser exit 2.
7. **Socket**: `DECK_SOCKET`, when set, is used instead of the socket path from `DECK_HOME` or the XDG paths.

## Screen

An exited kept terminal (terminal 2), next to a running one; the exited border and title are muted, the running focused one uses the accent:

```text
╭─ 1 · sh ────────────────────────────────╮╭─ 2 · cargo test · exited 101 ─────────╮
│$ █                                      ││test result: FAILED. 41 passed; 1 failed│
│                                         ││                                        │
╰─────────────────────────────────────────╯╰────────────────────────────────────────╯
```

## Acceptance criteria

- [ ] Inside deck, `deck new-terminal -- sh -c 'echo ready; sleep 30'` prints an id, the panel gains a terminal showing `ready`, and focus stays on the calling terminal.
- [ ] `deck ls --json` parses as JSON and lists that terminal with its program, pid and cwd; `deck ls --tree` shows the same tree as text (capture `ls-tree`); plain `deck ls` prints DECK-02's list.
- [ ] `deck new-terminal --keep -- sh -c 'exit 3'` leaves a terminal titled `… · exited 3`, drawn muted (capture `exited`), listed with `"exited": 3`; `⌥Space x` closes it at once without a confirmation, while `⌥Space x` on a running terminal still asks.
- [ ] `deck new-terminal --keep -- sh -c 'kill -TERM $$'` is listed with `"exited": 143` on macOS and Linux (unit test of the mapping plus this end-to-end check).
- [ ] `deck new-terminal --env DECK_TERMINAL_ID=1 -- env` shows the terminal's real id, not `1`.
- [ ] `deck new-panel --name tests --focus` creates and focuses a panel named `tests`; `deck new-project api --dir /tmp` creates a project and prints its id.
- [ ] `deck focus <id>` from outside deck changes what an attached client shows.
- [ ] `deck close <panel-id>` closes the panel and its terminals; `deck close <project-id>` closes the project.
- [ ] `deck rename-panel build` renames the caller's panel, with `name_source` `manual` in `ls --json`.
- [ ] Each error of requirement 2 is printed for its case and exits 1; with `--json` it prints `{"error": ...}`. A command run in a terminal after its panel was closed elsewhere (stale `DECK_PANEL_ID`) gets the stale-id error.
- [ ] `--session other` from inside a terminal of `main` targets `other`'s active panel, not the caller's panel.
- [ ] Outside deck with no daemon, `deck new-terminal` prints `no deck daemon running` and exits 1 without starting one; with a daemon but no session it prints `no session`.
- [ ] The DECK-03 room check applies to `new-terminal`, with its error text.
- [ ] A kept terminal that exited and a kept terminal still running (`sleep 300`): after `deck kill-server` and `deck`, the exited one is gone and the running one is back with `"keep": true` in `ls --json`; `state.json` has `keep` on both before the restart.

## Tests

- `deck-core`: request round trips; target parsing; `TerminalState.keep` default for old files.
- `crates/deck-daemon/tests/`: each request through the shared action functions, including `keep`, exit codes from `wait_status` (normal exit and `SIGTERM` → 143, on the OS the test runs on), `env` precedence, `focus: false`, and `keep` on save and restore.
- `deck-tui`: `TestBackend` drawing of an exited terminal's title and border.
- `crates/deck/tests/control.rs`: one scenario per acceptance criterion, calling `deck` from inside a harness terminal and from outside.
