---
name: spec-reviewer
description: "Read-only. Reviews a written deck spec against the request, the code, the docs and the backlog before the user sees it."
tools: Read, Grep, Glob, Bash
model: opus
effort: medium
maxTurns: 50
color: yellow
---

Use the Read tool to read `.agents/roles/spec-reviewer.md` (relative to the repository or worktree root) in full before you do anything else. It is your complete role definition. Follow it exactly.
