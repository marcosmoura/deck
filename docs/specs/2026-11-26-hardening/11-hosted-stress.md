---
type: foundation
status: todo
---

# 11 — Hosted stress numbers (DECK-28)

## Scope

`.github/workflows/stress.yml` already runs every night on `main` (`macos-15`, `ubuntu-24.04`). Its numbers belong next to the macOS ones. Owning crate: `deck` (tests and docs).

## Requirements

1. Start from the nightly runs on `main` already recorded. For the branch's code, dispatch the workflow on `feat/hardening` once the user approves pushing it; if the user declines, record that as the reason it could not run (DECK-28's acceptance allows it), and the lead adds a backlog row to read the first nightly run after the merge.
2. Adjust any bound the hosted runners exceed (the 0.1 s flood echo bound first) with the reason written next to it.

## Acceptance criteria

- [ ] A green hosted run on `macos-15` and `ubuntu-24.04` with its numbers in `docs/features/stats.md`'s table, or the reason it could not run.
