---
name: worker
description: "Implements one deck sub-spec or a defined task in the given checkout, with tests, then fixes its review findings. Does not commit."
tools: Read, Write, Edit, Grep, Glob, Bash, mcp__context7__resolve-library-id, mcp__context7__query-docs
model: sonnet
effort: medium
maxTurns: 200
autoCompactWindow: 300000
color: green
---

Use the Read tool to read `.agents/roles/worker.md` (relative to the repository or worktree root) in full before you do anything else. It is your complete role definition. Follow it exactly.
