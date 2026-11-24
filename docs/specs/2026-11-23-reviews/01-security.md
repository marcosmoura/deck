---
type: foundation
status: todo
---

# 01 — Security review

## Scope

A review of how deck could be abused by another local user, by a program running inside it, by a file or terminal output it reads, or through its release pipeline; supply-chain checks in CI; fixes within budget; the report. Run on Fable 5.2 at `xhigh` (overview).

## Requirements

1. **Threat model** written first, in the report: assets (the user's shells and their input, the clipboard, files the daemon can read, the release tokens), actors (another local user; a program inside a deck terminal, including untrusted output such as `cat` of a hostile file or an `ssh` session; a malicious dependency; a compromised GitHub token), and trust boundaries (the socket, the runtime and state directories, PTY output into the emulator, frames into the client, the client's writes to the outer terminal, the hooks deck installs into agents' configs, CI).
2. **Areas**, each concluded in the report even when nothing is found. This list is the minimum: features present at the reviewed commit and missing here get their own area (overview, "Review the project at execution time"):
   - socket and directories: peer uid check on every connection, runtime dir checks (mode, owner, symlinks), `state.json`, `daemon.log`, `deck.upgrade`, `deck.handoff` and the handoff temp file (permissions, races, symlink attacks, TOCTOU);
   - live upgrade: who can trigger `SIGUSR2`, the checks on the executable path, the preflight, inherited fds, what a failed or hostile handoff can do;
   - terminal output: every escape sequence a program can send that reaches outside its terminal (title, OSC 52 copy, OSC 52 read denial, colour queries, the kitty graphics file, temporary-file and shared-memory mediums and the files they can make the daemon read, query replies injected as input, hyperlinks), and resource exhaustion (huge sequences, image memory, scrollback, the 1 MiB input cap);
   - the client: anything from a frame (text, titles, cwd, agent names) that reaches the outer terminal unescaped;
   - control API and hooks: commands that run programs (`new-terminal -- …`, `send`), `agent-event` input parsing, what `deck hooks install` writes into Claude Code, Codex and OpenCode configs and whether a hostile repo can make those hooks run something;
   - environment: `DECK_*` and `DECK_TEST_*` variables (can a test hook be triggered in production?), environment passed to terminals;
   - deck's own `unsafe` blocks, each `// SAFETY:` comment checked against the code, and the risk the Zig library behind `libghostty-vt` adds (aborts, memory safety, what a hostile byte stream can reach);
   - `deck stats` (DECK-23: what it reveals over the socket) and copy mode and search (DECK-24: search input, the text requests, their OSC 52 writes);
   - dependencies and supply chain: `cargo-deny` (overview), DECK-19's Zig build fetching sources at build time, GitHub Actions pinning, token scopes in DECK-25, `install.sh` and the formula's checksums.
3. **`cargo-deny`**, `deny.toml`, `just audit`, the CI step (overview). Advisories found are fixed by updating, or accepted in `deny.toml` with a reason and a link.
4. Fixes and proposed tasks per the overview's fix budget, each fix with a test that fails without it.

## Acceptance criteria

- [ ] `docs/reviews/<date>-security.md` exists with the commit reviewed, the features added since the spec, the threat model, every area of 2 (and every added one) concluded, and the findings table.
- [ ] `just audit` passes and runs in CI.
- [ ] Every `critical` or `high` finding is fixed, or is a backlog task the lead has raised with the user (overview, Proposed tasks).
- [ ] Each fix has a regression test.

## Tests

- One per fix; the `cargo-deny` check.
