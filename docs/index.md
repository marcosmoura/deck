# Docs

Read first; then open only the pages your task needs.

| Page                                   | Read when                                                                             |
| -------------------------------------- | ------------------------------------------------------------------------------------- |
| [workflow.md](workflow.md)             | A request changes the product or the repo, or you pick up a backlog task              |
| [backlog.md](backlog.md)               | You decide what to build next, pick up a task, or add or finish one                   |
| [architecture.md](architecture.md)     | You write a spec or touch code: crates, model, protocol, paths, deps                  |
| [specs/README.md](specs/README.md)     | You write, build or check the status of a spec                                        |
| [specs/TEMPLATE.md](specs/TEMPLATE.md) | You write a spec                                                                      |
| [features/](features/README.md)        | You need how a shipped feature behaves (one page per feature, added as features land) |

## Where a request goes

- Daemon, PTYs, sessions surviving, persistence: architecture.md#process-model, then the core and persistence specs.
- Sidebar, panels, keys, anything on screen: architecture.md#client, then the sessions and grid specs.
- Agents (status, hooks, naming, the control API): architecture.md#agents, then the control API and agent status specs.
