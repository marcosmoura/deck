# Compatibility

Read when you need to know how closely deck behaves like a good terminal, or why a program looks different inside deck.

A differential suite (`crates/deck/tests/compat.rs`, cases in `crates/deck/tests/compat/cases/*.vt`) shows the same bytes to a reference emulator (`alacritty_terminal`) and through deck, and compares the two screens cell by cell, with the cursor. With `#! history` it also compares the scrollback. Where they disagree, either deck is fixed or the difference is recorded in the known-differences table in `compat.rs`, with its reason.

## Covered

- Scrolling regions (`DECSTBM`) with `IND`, `RI`, `SU`, `SD`, `IL`, `DL`, inside and outside the region; resetting the region.
- `ICH`, `DCH`, `ECH`, `REP`.
- Tab stops: defaults, `HTS`, `TBC`, `CHT`, `CBT`.
- Autowrap on and off, the pending wrap at the last column, origin mode.
- `DECSC`/`DECRC` and `CSI s`/`CSI u`, with attributes.
- The alternate screen (`?47`, `?1047`, `?1049`) with the primary screen restored.
- DEC special graphics (`ESC ( 0`, `SO`/`SI`).
- Wide characters at the last column, over and erased by half; CJK, flags.
- `?25` visibility and the cursor shape and blinking of `DECSCUSR`.
- A redraw inside synchronized output (`?2026`).
- `CR`/`LF`, backspace at column 0, `NEL`, `IND` at the bottom.
- `ED 3` and the scrollback, through `deck capture --history`.
- Resize: reflow growing and shrinking, the cursor on a wrapped line, the alternate screen.
- Underline kinds (`4`, `4:0` to `4:5`, `21`, `24`) and the underline colour (`58` in 256-colour and RGB forms, `59`).
- The last row is not covered: the floating bar is not drawn when no mode or flash is active.

## Recorded programs

Real programs' raw output is committed in `crates/deck/tests/compat/recordings/<name>.vt` and replayed through the same differential compare on every `just test` (the `recordings!` list in `compat.rs`), so replaying needs none of the programs. Each is recorded at 100x30 from `crates/deck/tests/compat/scenarios/<name>.scenario`.

| Recording  | Program (version) | What it shows                                                         |
| ---------- | ----------------- | --------------------------------------------------------------------- |
| `nvim`     | neovim 0.12.6     | A Rust file with syntax highlighting, a vertical split, a `:` command |
| `less`     | less 668          | A 300-line file, a search with its match highlighted, scrolling up    |
| `lazygit`  | lazygit 0.66.3    | The welcome popup, then the commits view of a two-commit repository   |
| `fzf`      | fzf 0.75.0        | An inline list of 40 items, a query, the pointer moved down (live UI) |
| `git_log`  | git 2.56.1        | `git log --graph --color` with a merge, a branch and a tag            |
| `git_diff` | git 2.56.1        | `git diff --color` with changed, removed and added lines              |
| `bat`      | bat 0.26.1        | A Rust file with line numbers, a grid and highlighting                |

Not recorded: htop (it was not installed on the machine that recorded; the plan was `htop -p <pid of a sleep>`, for 3 s). `top` and `btop` are left out on purpose, they list every process.

A recording that ends in a full-screen program's alternate screen would replay as a blank screen once the program quits, so those scenarios `stop` on the interesting screen instead of quitting.

### Recording a new one

1. Write `crates/deck/tests/compat/scenarios/<name>.scenario`, one step per line: `size <cols>x<rows>` (default 100x30) and `setup <sh command>` (run in the scratch `HOME`; `$DECK_SCENARIOS` is the scenarios directory, so a fixture is `cp "$DECK_SCENARIOS/files/x" .`; git dates are fixed) before `run <argv>`, then `wait <text>`, `send <text with escapes>`, `key <name>` (deck's key names, `C-x`, `Enter`, `Up`), `sleep <ms>` and `stop` (end the recording here and terminate the program; without it the recording ends when the program exits).
2. `just record-compat <name>` plays it with the program straight on the harness's terminal and writes the recording. It refuses to replace one without `FORCE=1`.
3. Add the name to `recordings!` in `compat.rs`, run `just test-crate deck compat`, and either fix the difference or add a known-differences entry with its reason.

The program runs in `/tmp/deck-record/<name>` as its `HOME` with the environment cleared to `PATH` (without entries below the recorder's home), `TERM=xterm-256color`, `COLORTERM=truecolor`, `LANG=en_US.UTF-8`, `XDG_*_HOME`, `USER=deck`, `LOGNAME=deck` and `HOSTNAME=deck`; its `.gitconfig` is `deck <deck@example.invalid>`, so none of the user's dotfiles load. The recorder refuses a recording that contains the recorder's `$HOME`, `$USER` (4+ characters), host name (full or short), a `/Users/`, `/home/` or `/var/folders/` path, or any absolute path outside `/tmp/deck-record` and the system roots (`/usr`, `/opt`, `/bin`, `/etc`, …). `committed_recordings_are_private` checks every committed one again, without the `$USER` rule (on another machine the user's name can be a word a program prints). `GIT_CONFIG_NOSYSTEM=1` keeps a system gitconfig out of recordings. Read a new recording for tokens and other private text before committing it.

## Manual checklist

Programs that need an account or show private data are not recorded. Check them by hand after a change to the terminal, input or rendering code, in deck inside a Ghostty or kitty window and inside a plain terminal:

- Claude Code, Codex and OpenCode: start; type a multi-line prompt with Shift+Enter; scroll their output (wheel and Page Up); open a permission dialog and answer it.
- `ssh localhost`: the prompt, a full-screen program inside it, resize.
- tmux inside deck: start, split, detach and re-attach, the prefix keys.
- vim or neovim with `:set mouse=a`: click, drag-select, wheel.

Results of the manual run, 24 Nov 2026: not run. The worker that recorded the programs works without a terminal window, an account for the agents or an ssh server, so the checklist is still to be run by hand.

## Known differences

Deck's emulator is Ghostty's; the reference is `alacritty_terminal`. In each case below deck follows xterm and Ghostty.

- `?47` and `?1047` do not switch screens in the reference.
- A `DCH` count past the end of the line also blanks cells before the cursor in the reference.
- Erasing (`ECH`) or deleting (`DCH`) the right half of a wide character blanks both halves; the reference leaves the left half.
- `DECOM` set or reset homes the cursor; the reference does not home it on reset.
- A variation selector 16 makes a text symbol (`❤️`, `✔️`) two cells wide; a ZWJ sequence (`👩‍💻`) is one wide cell. The reference draws them as separate narrow or wide cells.
- Shrinking the width reflows into more rows in place; the reference pushes the top of the content into its scrollback.
- Shrinking the height of the alternate screen drops its top rows; the reference drops the bottom ones. Programs redraw on the resize.
- SGR 21 is a double underline in deck (as in xterm's ctlseqs and Ghostty); the reference reads it as bold off, the Linux console's meaning.
- An underline colour on text that is not underlined is not drawn (it has no effect); the reference keeps it on the cell. The cases reset it with `59` before text without an underline.
