---
type: foundation
status: done
---

# 02 — Security follow-up fixes

## Scope

The low and cheap info findings of the security audit (handed to the worker by the lead), which did not fit [01](01-security.md)'s session. The user chose this split and the dispositions on 24 Nov 2026. Run on Fable 5.2 at `xhigh` (overview). No new analysis: the worker confirms each finding still holds at the commit it starts from, fixes it, and turns its `planned (sub-spec 02)` status in the security report into `fixed (sub-spec 02, test <name>)`.

## Requirements

1. Fix, with the test or check the audit names (most severe first):
   - **A-2** state directory, backups and `daemon.log` permissions; correct `docs/features/persistence.md`.
   - **A-3, B-3, D-2** control characters: reject them in `Osc7Scanner::parse`, replace them in `tree_text`, filter them from completion help.
   - **A-4** the skill and `docs/features/control-api.md`: one sentence warning that allowing the deck socket lets an agent run programs outside its sandbox and type into every terminal; the untrusted-data rule extended to names, titles and directories from `deck ls`.
   - **A-7** hooks and the OpenCode plugin call `deck` by its absolute `PATH` entry (not canonicalised); docs updated.
   - **A-1 residual** (found by 01's worker, not in the audit): when the socket is refused or silent, `deck kill-server` sends `SIGTERM` to the pid in `deck.lock` (`crates/deck/src/control.rs`, about line 609) after only the client's single `lstat` of the runtime directory; the explicit `deck upgrade` does the same with `SIGUSR2` (`crates/deck/src/upgrade_cmd.rs`, `perform`, whose only connection gates nothing). In both, apply the strict directory check (owner, mode, type) before reading the lock, so a planted directory cannot name another process.
   - **A-8** control connections that create no session send only `DECK_*` in `Hello`.
   - **A-9** `--adopt` accepts descriptors from 3 up.
   - **A-10** agent session ids failing `valid_session_id` are dropped at ingestion.
   - **B-4** titles keep no control characters.
   - **B-5** clipboard writes: a size cap, `text/plain` only, clipboard bytes counted in the queue; docs name OSC 1337 and OSC 5522.
   - **B-9** a single OSC 7 no longer switches off the OS cwd lookup for good, and cwd changes are throttled like titles.
   - **B-10** the client strips ESC from pastes.
   - **B-11** `own_fd` made `unsafe fn` or validated; the `listen()` fallback fixed; pid 0 and negatives refused before `kill`; `docs/architecture.md`'s `unsafe` list completed.
   - **C-3** `install.sh`: curl `--proto '=https' --tlsv1.2`, wrapped in `main`, `mktemp` staging; `packaging/install-check.sh` run by `just lint-ci`.
   - **C-4** the tap's publish action pinned by SHA; the DECK-29 decision's doc updated.
   - **C-5** `mise.lock` for the four platforms and `MISE_LOCKED=1` in the workflows (if it cannot fit, the worker reports it and the lead adds a task).
   - **C-6** `LICENSE` in the release tarballs; `publish = false` on the workspace crates (and `deny.toml`'s wildcard setting adjusted to match).
   - **C-7** the lock-drift rule in `docs/workflow.md` and a check in `.agents/roles/reviewer.md`.
   - **C-9** `SECURITY.md` (private reporting route) and `.github/dependabot.yml` for `github-actions` and `cargo`.
   - **C-10** `persist-credentials: false` on checkouts, `cache: false` on mise-action in `release.yml`, packaging exercised on PRs without uploading if cheap.
   - **D-4** completion handled after the `agent-event` guard.
   - Not security: the completion docs show `eval "$(COMPLETE=bash deck)"`, which works on macOS's bash 3.2.
2. The fix budget of the overview applies to each fix.

## Acceptance criteria

- [ ] Every finding of requirement 1 is `fixed` in the security report, or reported by the worker as no longer holding or not fitting (the lead then adds a task).
- [ ] Each fix has its regression test or check; `just lint-ci` and `just audit` pass.

## Tests

- One per fix, as named in the audit's findings.

## Notes

- 24 Nov 2026: A-8 moved to [04](04-performance.md) by the lead. Since A-1 the environment reaches only a peer-checked daemon of the same user, so the security concern is closed; sending only `DECK_*` changed the timing of `deck kill-server` during a handoff, which 04 measures before any change.
