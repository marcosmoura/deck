# AGENTS.md

**deck** is a terminal multiplexer written in Rust, with a rich terminal UI and
first-class support for coding agents (Claude Code, Codex, OpenCode). A
background daemon keeps **sessions** alive; a session holds **projects** (shown
in the sidebar), a project holds **panels**, and a panel shows one or more
**terminals** in an automatic grid. The product is specified in
[docs/specs/](docs/specs/README.md) and built task by task from
[docs/backlog.md](docs/backlog.md).

Start at [docs/index.md](docs/index.md), then read only the docs your task needs.

## Essentials

- One Cargo workspace at the root, one binary (`deck`). The crates, their
  boundaries, the protocol and the approved dependencies are in
  [docs/architecture.md](docs/architecture.md).
- Toolchain: mise pins Rust (with rustfmt and Clippy), `just`, cargo-nextest,
  Lefthook and oxfmt (`.mise.toml`). oxfmt formats what rustfmt does not:
  Markdown, TOML, JSON and YAML. `just` is the command surface.
- Platforms: macOS and Linux. Unix PTYs and Unix sockets only; no Windows code.

## Commands

The tooling comes with [DECK-01 sub-spec 01](docs/specs/2026-11-22-core-daemon-terminal/01-workspace-tooling.md):
these commands exist in the DECK-01 worktree once that sub-spec is committed
there, and on `main` once DECK-01 is merged. Until then, use the plain Cargo
equivalents in [docs/workflow.md](docs/workflow.md#checks). Run them from the repository root.

| Command                       | What it does                                                     |
| ----------------------------- | ---------------------------------------------------------------- |
| `mise install`                | Install the pinned toolchain                                     |
| `lefthook install`            | Install the Git hooks (once per clone)                           |
| `just check`                  | The gate: format check, Clippy with warnings denied, all tests   |
| `just fmt`                    | Format everything: Rust with rustfmt, the rest with oxfmt        |
| `just lint`                   | Clippy on every crate, warnings denied                           |
| `just test`                   | All tests with cargo-nextest                                     |
| `just test-crate <crate> [f]` | Tests of one crate, optionally filtered, e.g. `deck-core layout` |
| `just run [args]`             | Build and run `deck` with an isolated socket and state dir       |
| `just install`                | Install `deck` into `$PREFIX/bin` (default `~/.local/bin`)       |

`just check` is the gate before a commit. Plain `cargo` commands (`cargo build -p
<crate>`, `cargo nextest run -p <crate> <filter>`, `cargo clippy -p <crate>`) are
fine while iterating.

## Crate boundaries

- `deck-core` is pure: no I/O, no threads, no async. Model, layout, protocol
  types, key encoding and config parsing live there and are unit-tested.
- `deck-daemon` owns PTYs, terminal emulation, persistence and agent status.
  `deck-tui` owns rendering and input. They never depend on each other; both
  depend on `deck-core`. `deck` (the binary) wires them together and owns the CLI.
- Dependency versions are declared once in the root `[workspace.dependencies]`;
  crates opt in with `workspace = true` and choose their own features.

## Commits

[Conventional Commits](https://www.conventionalcommits.org/), checked by the
Lefthook `commit-msg` hook once DECK-01 is merged.

```text
<type>(<scope>): <subject>
```

- Types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`,
  `ci`, `chore`, `revert`.
- Scopes (optional, one per commit): `core`, `daemon`, `tui`, `cli` for crate
  changes; `spec` for specs and the backlog; `agents` for the agent workflow;
  `repo` for root tooling; `deps` for dependency updates; `docs` for other docs.
  Omit the scope when a change spans several crates.
- Subject: imperative, lower case, no trailing period, at most 100 characters
  for the whole header. One logical change per commit.

## Workflow

The session the user talks to is the **lead** and runs the repo flow itself;
there is no orchestrator agent. Read [docs/workflow.md](docs/workflow.md) before
you start it. In short:

- A question or a fix of a line or two: just do it. A request for a backlog
  task ("the next task", `DECK-NN`) is already a yes to the repo flow. For
  anything else, think it through, size it (a **simple task** fits one focused
  session of under about 500k tokens; anything bigger needs a **spec**) and ask
  the user whether to run it through the repo flow.
- Spec: write it (split into shippable or foundation sub-specs when big), run
  `spec-reviewer`, commit the draft, show the user a summary, change it until
  they approve, commit it, then ask whether to start the work.
- Work: inline when it fits one session, otherwise one `worker` per sub-spec,
  one sub-spec at a time, in the spec's worktree. Parallel work happens across
  backlog tasks in separate sessions.
- While a subagent runs, end the turn and wait to be woken; never poll.
- After work that went through the repo flow, always run `reviewer`. The lead
  fixes inline work; delegated work goes back to the same worker, which keeps
  its context.
- This workflow authorizes the lead to start `spec-reviewer`, `worker` and
  `reviewer` subagents without asking again (Codex and OpenCode included).
- "Pick the next task" means: walk the queue in [docs/backlog.md](docs/backlog.md#picking-the-next-task).

## Hard rules

- Ask approval of a spec before implementing it. Commit an approved spec on
  `main` right after the approval, before any other step.
- Never merge a branch while a sub-spec of its spec is not `done`.
- When merging, squash the commits, following
  [docs/workflow.md](docs/workflow.md#finish).
- Update the docs that describe a behavior in the same change as the behavior
  (`docs/features/`, `docs/architecture.md`).
- Do not add a crate, tool, lint rule or test runner the approved task did not
  ask for. New ones are added to `[workspace.dependencies]` or `.mise.toml` and
  recorded in [docs/architecture.md](docs/architecture.md#dependencies).
- A command written in a doc, spec or prompt must exist in the `justfile`, in
  `deck` itself, or be a plain `cargo`/`git` command. Never invent one. A spec
  may define a new `deck` command or `just` recipe; it exists once that spec
  is merged into `main`.
- No `unsafe` outside the small platform modules named in
  [docs/architecture.md](docs/architecture.md); each `unsafe` block has a
  `// SAFETY:` comment.
- The user's yes to the repo flow, the approval of a spec, or a request for a
  backlog task authorizes the commits listed in
  [docs/workflow.md](docs/workflow.md#commits-the-flow-may-make) for that task,
  and no others.
- The backlog is read and edited only on `main`, in the main checkout.
- Never set git identity. Never push, merge or delete a branch without the
  user's go-ahead in the current session. Never bypass hooks.
