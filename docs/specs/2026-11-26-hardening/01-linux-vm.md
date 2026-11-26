---
type: foundation
status: todo
---

# 01 — The capped Linux VM

## Scope

Lays down the Linux VM every later sub-spec measures and runs long jobs in (overview, Decisions), so all of them use the same image and limits. Owning area: the repository root (`justfile`, a script under `scripts/`, docs).

## Requirements

1. `just` recipes, macOS only, that fail with a clear message when `container` is missing:
   - `just vm-up [cpus]`: create or start the VM (Ubuntu 24.04, default 4 CPUs, 8 GB, with an init process and `SYS_PTRACE` for stack traces), mount the repository read-only at `/repo` with writable tmpfs over every `target/` the tests write into, keep the toolchain and build output on a named volume, and install what `.mise.toml` pins plus `gdb` and Python as a non-root user.
   - `just vm-run <command>`: run a command in `/repo` (or a worktree under `/repo/.worktrees/<slug>`) with the toolchain, a build directory on the volume, and `mise` told not to write its lock file.
   - `just vm-down`: stop it; nothing is deleted.
2. The recipes call one script, `scripts/linux-vm.sh` (01 creates `scripts/`), added to the explicit `shellcheck` list of the `lint-ci` recipe.
3. Record `container` in `docs/architecture.md#dependencies` as an optional macOS developer tool approved by DECK-49, and say in `docs/workflow.md` that long runs go through these recipes.

## Acceptance criteria

- [ ] `just vm-up`, then `just vm-run just test` passes in the VM except tests that write into the read-only checkout, which are listed.
- [ ] `just vm-run just stress idle` runs.
- [ ] The VM uses at most the CPUs given (checked with `nproc` inside).

## Tests

The acceptance runs; `just lint-ci` covers the script.

## Notes

The user's session on 25–26 Nov 2026 set up the same VM by hand: an init process, `SYS_PTRACE`, 4 CPUs and 8 GB, tmpfs over `target/` and a named volume for the toolchain. The recipes wrap those `container` options so the setup is repeatable.
