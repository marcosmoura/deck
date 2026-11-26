# Completion

`deck` completes its subcommands, flags and values in bash, zsh and fish. Load it from the shell's start-up file:

| Shell | Line                           | File                         |
| ----- | ------------------------------ | ---------------------------- |
| zsh   | `source <(COMPLETE=zsh deck)`  | `~/.zshrc`                   |
| bash  | `eval "$(COMPLETE=bash deck)"` | `~/.bashrc`                  |
| fish  | `COMPLETE=fish deck \| source` | `~/.config/fish/config.fish` |

`COMPLETE=<shell> deck` prints a small script that calls `deck` back for each word you complete. Sourcing it at start-up, rather than saving it to a file, keeps it in step with the installed `deck`. `COMPLETE=0` (or empty) turns it off. The bash line uses `eval` because `source <(...)` defines nothing in the bash 3.2 that macOS ships as `/bin/bash`.

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

In bash, a name that would need quoting is not offered: a directory or a session whose name has a space, a control character or a character the shell acts on (`$`, a backquote, quotes, `*`, `?`, brackets, `;`, `&`, `|` and the like). bash inserts what `deck` answers as it is, so such a name could run a command when you press Enter; type it yourself, quoted. zsh and fish quote what they insert and offer every name.

Sessions and ids come from the running daemon. Completion never starts one: with no daemon running, they complete to nothing. Hidden commands (`daemon`, `agent-event`) are not offered.
