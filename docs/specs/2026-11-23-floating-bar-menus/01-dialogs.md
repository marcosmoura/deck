---
type: shippable
status: done
---

# 01 — Dialogs

## Scope

Prompts and confirmations leave the status bar and become centered dialogs. The status bar stays as it is otherwise; sub-spec 02 removes it. Owning crate: `deck-tui` (`modal.rs`, `draw.rs`'s `draw_prompt` and the confirmation branch of `draw_status`); end-to-end tests in `deck`.

## Requirements

1. **The box.** `Modal::Prompt` and `Modal::Confirm` are drawn as a centered rounded box (`border_type(theme.border_style)`, background `overlay_bg`, border `border`, title in `accent` bold), over everything else, the session picker included. Width: a prompt's from its title and hint row, a confirmation's from its question and hint row, plus 4, at least 40 columns (the editor text and a flash never widen it; they are cut), at most the window width minus 2. It is 5 rows with its borders. A title or question longer than the inner width is cut with `…`.
2. **Prompt.** Title: the prompt's label without its colon (`Rename space`, `Directory`, `Space name`, `New session`, `Rename session`, ...). Row 1: the one-line editor (the text with the cursor cell, cut at the start to keep the cursor in sight, as `draw_prompt` does today). Row 2 blank. Row 3: `⏎ save  esc cancel` in `muted`, or the flash in its colour while one is live (a refused value: the reason in red for 4 s).
3. **Confirmation.** No title. Row 1: the question in `warning`, without the ` (y/n)` suffix (the question strings in `modal.rs` and `picker.rs` drop it). Row 2 blank. Row 3: `y yes  n no` in `muted`, or the flash.
4. **Unchanged.** Keys, paste, retries, what the prompt reopens with, and the closing rules of `modal.rs` (target closed meanwhile) do not change. While a dialog is open the terminal cursor is not shown, and the status bar shows what it shows with no modal open, minus the flash (the dialog shows it).
5. **Smallest window.** The 5-row dialog fits the smallest window (`MIN_SIZE`, 20×5) and is never drawn when the window is too small.

## Screen

Rename dialog, centered in an 80×24 window:

```text
                    ╭ Rename panel ────────────────────────╮
                    │ editor█                              │
                    │                                      │
                    │ ⏎ save  esc cancel                   │
                    ╰──────────────────────────────────────╯
```

Confirmation:

```text
                    ╭──────────────────────────────────────╮
                    │ Close panel "zsh" and its terminal?  │
                    │                                      │
                    │ y yes  n no                          │
                    ╰──────────────────────────────────────╯
```

A refused name: row 3 shows the daemon's reason in `error` for 4 s, and the editor keeps the text.

## Acceptance criteria

- [ ] Every prompt (rename panel, space, session; new space's directory and name; new session) and every confirmation (close terminal, panel, space; kill session) is a centered dialog with the keys of today.
- [ ] A refused value shows its reason in the dialog for 4 s and the dialog keeps the text.
- [ ] Long titles and questions are cut with `…`; the dialog fits a 20×5 window.
- [ ] The status bar no longer draws a prompt or a question.
- [ ] `docs/features/sidebar.md` (Prompts), `sessions.md` and `layout.md` (the close question) describe the dialogs.

## Tests

- `deck-tui` unit tests: the prompt and confirmation dialogs (size, centering, title, the editor's cut, the retry flash on row 3, the cut title and question, the 20×5 window); the cursor hidden while one is open; a dialog drawn over the picker.
- End-to-end (`crates/deck/tests/`): the tests that type into a prompt or answer a question find the label and the question in the dialog instead of the status bar; one scenario renames a panel through the dialog and one refuses a duplicate space name.

## Notes

A live flash widens the dialog up to the window width minus 2 (decided in review: refusals were cut).
