---
type: foundation
status: todo
---

# 07 — Terminal history sized by width (DECK-42)

## Scope

Each terminal's history is a byte budget sized for 250 columns (`crates/deck-daemon/src/emu.rs`, 26,648,576 bytes for 10,000 lines), so a long-lived terminal ends near 26.6 MiB whatever its width; 30 terminals hold about 800 MiB, most of it unreachable. Owning crate: `deck-daemon`.

## Requirements

1. Size the budget from the real width, through ghostty-vt 0.9.0's `Terminal::set_scrollback_max_bytes` (and `set_scrollback_max_lines`), which change it after creation.
2. Widening keeps at least 10,000 lines up to 250 columns; narrowing never drops the budget below the configured lines at the new width.
3. A live upgrade keeps each terminal's budget for its width.

## Acceptance criteria

- [ ] The `memory` scenario shows a terminal at 80 columns holding about a third of today's memory once its budget fills.
- [ ] Widening, narrowing and a live upgrade keep at least 10,000 lines (unit or e2e tests).
- [ ] `docs/architecture.md`'s Emulator paragraph states the new budget.

## Tests

The `memory` scenario; unit tests of the budget on resize.
