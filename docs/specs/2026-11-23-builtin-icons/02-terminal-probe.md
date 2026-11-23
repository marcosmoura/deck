---
type: shippable
status: done
---

# 02 — Ask the terminal: graphics, Nerd Font, else Unicode

## Scope

The client finds out what the outer terminal can show instead of guessing from a list of terminal names: it asks the terminal for its name and for kitty graphics support, and reads the font the terminal draws with. A user with a Nerd Font in iTerm2, Terminal.app, Alacritty or a Linux terminal gets Nerd glyphs; a terminal that answers the graphics query and supports Unicode placeholders gets images; everything else gets Unicode. This replaces [01](01-icon-tiers.md)'s first-cut tier rules. Owning crate: `deck-tui` (queries and reply parsing in `colors.rs`; font checks and the tier decision in a new `caps.rs`, which is the one place in `deck-tui` that spawns programs and lists font folders); `deck` (e2e harness).

## Requirements

1. **Two more questions in the start-up probe.** `colors::QUERY` (OSC 10, 11, 4, then DA1) gains, before DA1:
   - **XTVERSION** `\x1b[>0q`; the reply is `\x1bP>|<name and version>\x1b\\` (for example `kitty(0.36.4)`, `ghostty 1.2.0`, `WezTerm 20240203-…`, `iTerm2 3.5.10`, `tmux 3.5a`).
   - **Kitty graphics query** `\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\`; a terminal with the protocol replies `\x1b_Gi=31;OK\x1b\\` (or an error text instead of `OK`).

   Both replies are parsed like the colour replies (pure functions, both string terminators), never reach the input, and the probe still ends on DA1 within `ANSWER_TIMEOUT`. A late reply is swallowed in the `LATE_WINDOW` like a late colour reply, and changes nothing. For that, today's late-answer matcher (`colors::answer_step`, which knows only the OSC shapes) gains two shapes: DCS `Alt+P` `>` `|`, then at most 64 printable ASCII characters, then `Alt+\`; and APC `Alt+_` `G`, then at most 64 characters from `[A-Za-z0-9=,;:._ -]`, then `Alt+\`. Anything else ends the match and is delivered as typed. The cut-tail rule of `parse_answers` (a reply split across reads) also treats an unfinished `ESC P` or `ESC _` tail as a reply.

2. **The terminal**: `enum Terminal { Ghostty, Kitty(Option<version>), WezTerm, ITerm2, AppleTerminal, Other(String), Unknown }`, from the XTVERSION reply. With no reply, from the environment as a fallback: `TERM_PROGRAM` (`ghostty`, `WezTerm`, `iTerm.app`, `Apple_Terminal`), `TERM` (`xterm-ghostty`, `xterm-kitty`), `KITTY_WINDOW_ID`, `GHOSTTY_RESOURCES_DIR`, `WEZTERM_EXECUTABLE`. Terminal.app does not answer XTVERSION, so it is always found by `TERM_PROGRAM`. A kitty found from the environment has no version (`None`), which counts as a current kitty (0.36 or later).

3. **`Image` tier** when all of:
   - the graphics query answered `OK`, or, only when neither XTVERSION nor the graphics query was answered, DECK-12's environment rules say Ghostty or kitty (this keeps the e2e harness, which answers neither, able to opt in);
   - the terminal draws Unicode placeholders: Ghostty, or kitty 0.28 or later (Unicode placeholders arrived in 0.28.0). WezTerm and others that answer the graphics query but lack placeholders do not get images;
   - truecolor (as today), and none of `TMUX`, `ZELLIJ`, `DECK` set (as today).

4. **`Nerd` tier** (when not `Image`) when the first check that applies says the terminal has Nerd Font glyphs:
   1. **Bundled**: Ghostty, kitty 0.36 or later, and WezTerm ship the Nerd Font symbols and fall back to them for any font. (The worker checks the kitty version against its changelog.)
   2. **iTerm2**: the profile in use (`ITERM_PROFILE`, else the profile whose `Guid` is `Default Bookmark Guid`) in `defaults export com.googlecode.iterm2 -` (an XML plist on stdout): its `Normal Font`, and its `Non Ascii Font` when `Use Non-ASCII Font` is true, is a Nerd Font name.
   3. **Terminal.app**: the profile named by `Default Window Settings` in `defaults export com.apple.Terminal -`: its `Font` is archived data (base64 in the XML); the decoded bytes hold the font's PostScript name as text, and that name is a Nerd Font name.
   4. **Any other terminal, or iTerm2 and Terminal.app when their own check fails but the font fallback works** (see requirement 7): a font that has U+F120 and U+EAB4 is installed: `fc-list ':charset=f120 eab4' family` prints a line (Linux, and macOS when fontconfig is installed); on macOS without `fc-list`, a font file in `~/Library/Fonts` or `/Library/Fonts` whose name is a Nerd Font name.

   A **Nerd Font name** (pure function): contains `Nerd Font` or `NerdFont` (any case), or has `NF`, `NFM` or `NFP` as a word or right before a `-` (`JetBrainsMono NFM`, `JetBrainsMonoNFM-Regular`, `Symbols Nerd Font Mono`).

5. **`Unicode` tier** otherwise, and whenever a check cannot run (no `defaults`, no `fc-list`, unreadable output, a timeout).

6. **Running the checks.** The probe decides the terminal (within `ANSWER_TIMEOUT`, 200 ms), then the font check that applies to it starts on a thread. Programs are run directly, without a shell, with stdin and stderr closed, each killed after 1 s. The thread sends its answer on an mpsc channel that the client loop's `tokio::select!` also waits on: an answer that comes before the first frame is used by it; a later one sets the tier and marks the app dirty, so the client redraws. The tier changes at most once in a client run, from `Unicode` to `Nerd`, and never downgrades. Only the bundled check and the `Image` decision are known at once. The parsers (XTVERSION, graphics reply, the iTerm2 plist, the Terminal.app font data, `fc-list` output, Nerd Font names) are pure functions with fixtures; only the spawning is not.

7. **Spike first, by hand, on macOS**: does a terminal whose own font is not a Nerd Font still draw U+F120 when a Nerd Font is installed (macOS font fallback): Terminal.app, iTerm2 (with and without a non-ASCII font set), Alacritty? The answer decides, per terminal, whether check 4 applies to it. The worker records the results in this file's Notes before writing the code, and the lead tells the user.

8. **Multiplexers and deck panels**: deck inside tmux, Zellij, screen or a deck panel (`DECK` set) is not supported for this detection. Inside a deck panel only check 4 applies (whatever the inner emulator answers to XTVERSION). Otherwise the queries reach the multiplexer, which answers in its own name, so its answers decide (usually `Unicode`, or `Nerd` from check 4). Nothing tries to see through it.

9. **Test harness**: the e2e harness's terminal answers neither new query, so the env fallback applies as today. So that a developer's installed fonts never change test results, the harness sets `DECK_TEST_NO_FONT_CHECK=1` for the client, which skips checks 2 to 4. It is a test seam only: not in `docs/features/`, listed in `architecture.md#testing`.

## Acceptance criteria

- [ ] `QUERY` ends with XTVERSION, the graphics query, then DA1; the probe still stops at DA1; both new replies, in BEL and ST forms, split across reads (the cut-tail rule) and arriving late as key events (`answer_step`), are parsed or swallowed and never reach the input; a late `Alt+P` followed by text that breaks the shape is delivered as typed.
- [ ] XTVERSION parsing: the five example replies above give `Kitty(0.36.4)`, `Ghostty`, `WezTerm`, `ITerm2` and `Other("tmux 3.5a")`; no reply falls back to the environment; `TERM=xterm-kitty` alone gives `Kitty(None)`.
- [ ] Tier decision table (pure function over the probe answers, the environment, truecolor and the font-check result): Ghostty with `OK` → `Image`; kitty 0.36 with `OK` → `Image`, and without truecolor → `Nerd` (bundled); kitty 0.27 with `OK` → not `Image` (no placeholders) and `Nerd` only if check 4 finds a font; WezTerm with `OK` → `Nerd`; iTerm2 with a Nerd profile font → `Nerd`; iTerm2 with `Monaco` and no fallback → `Unicode`; tmux → whatever check 4 says; no answers and `TERM_PROGRAM=ghostty` → `Image` (env fallback); no answers and `KITTY_WINDOW_ID` with truecolor → `Image` (`Kitty(None)`); `DECK` set and an inner `ghostty` XTVERSION → only check 4.
- [ ] Nerd Font names: `JetBrainsMono Nerd Font`, `JetBrainsMonoNFM-Regular`, `Hack NF`, `Symbols Nerd Font Mono`, `FiraCodeNerdFontMono-Retina` are; `Monaco`, `SF Mono`, `Menlo-Regular`, `NFL Sans` are not.
- [ ] iTerm2 and Terminal.app fixtures (real `defaults export` output, trimmed and committed under the crate's test data) give the right font names; `fc-list` output with and without a line.
- [ ] A font answer sent on the channel after the first frame changes the tier and the next loop turn redraws; one that never arrives leaves `Unicode`; a second answer changes nothing.
- [ ] End to end: with `TERM_PROGRAM=WezTerm` and no answers, a panel row's icon is the Nerd `terminal` glyph (env fallback, bundled check; capture `nerd-tier`).
- [ ] By hand: Ghostty → images; kitty → images; WezTerm → Nerd; iTerm2 with a Nerd Font profile → Nerd, with Menlo → Unicode (or Nerd if the spike shows fallback); Terminal.app the same; Alacritty with a Nerd Font installed → per the spike. The spike's results are in Notes. In Terminal.app and one older terminal, the graphics query leaves no stray text on screen; in every terminal tried, the XTVERSION reply comes before the DA1 reply.

## Tests

- `deck-tui`: reply parsers, `Terminal` from XTVERSION and from the environment, the tier decision table, Nerd Font names, the plist and font-data scanners on fixtures, `fc-list` output, the late upgrade.
- `crates/deck/tests/`: existing tests pass with `DECK_TEST_NO_FONT_CHECK=1` in the harness; 01's Unicode e2e still holds; the `nerd-tier` scenario.

## Notes

- There is no terminal query for "does your font have this glyph"; reading the terminal's font setting, or the fonts fontconfig can fall back to, is the closest deck can get. iTerm2 users who load preferences from a custom folder are not read (the plist is elsewhere); they get `Unicode` unless check 4 applies.
- Spike results (by the user, 23 Nov 2026, macOS): Terminal.app with a non-Nerd profile font draws missing-glyph boxes for U+F120, U+EAB4 and U+EC82 although a Nerd Font is installed, so check 4 does not apply to it. iTerm2 is not installed there and stays without check 4; Alacritty is untested and keeps check 4. iTerm2's rule reads the `Non Ascii Font` alone when `Use Non-ASCII Font` is on, because iTerm2 draws non-ASCII glyphs with it. Decided with the user: on macOS, when `fc-list` fails or times out (a cold fontconfig cache can take over 1 s), check 4 falls back to the font-folder scan.
