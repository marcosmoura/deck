# Completion

`deck` completes its subcommands, flags and values in bash, zsh and fish. Load it from the shell's start-up file:

| Shell | Line                           | File                         |
| ----- | ------------------------------ | ---------------------------- |
| zsh   | `source <(COMPLETE=zsh deck)`  | `~/.zshrc`                   |
| bash  | `source <(COMPLETE=bash deck)` | `~/.bashrc`                  |
| fish  | `COMPLETE=fish deck \| source` | `~/.config/fish/config.fish` |

`COMPLETE=<shell> deck` prints a small script that calls `deck` back for each word you complete. Sourcing it at start-up, rather than saving it to a file, keeps it in step with the installed `deck`. `COMPLETE=0` (or empty) turns it off.

## What completes

| Value                                                            | Candidates                                                   |
| ---------------------------------------------------------------- | ------------------------------------------------------------ |
| `attach`, `kill-session` and `rename-session` names, `--session` | the daemon's sessions, with their space count and `attached` |
| `--space`                                                        | space ids, with the space and session names                  |
| `--panel`                                                        | panel ids, with the panel, space and session names           |
| `--terminal`                                                     | terminal ids, with the program, panel and session names      |
| the id of `focus` and `close`                                    | terminal, panel and space ids, labelled with their kind      |
| `new -c`, `new-space --dir`, `--cwd`                             | directories                                                  |
| `hooks` and `skills` agents                                      | `claude`, `codex`, `opencode`                                |

Sessions and ids come from the running daemon. Completion never starts one: with no daemon running, they complete to nothing. Hidden commands (`daemon`, `agent-event`) are not offered.
