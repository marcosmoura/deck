---
type: shippable
status: todo
---

# 02 — Hugeicons images for every icon

## Scope

In Ghostty and kitty (the `Image` tier of [01](01-icon-tiers.md)), every icon deck draws is a small image made from a Hugeicons free SVG: program icons, carets, status marks and the unseen marker. The spinner stays its Nerd glyph frames until [03](03-rotating-loader.md). Other tiers are unchanged. Owning crate: `deck-tui` (assets, the image registry, drawing); `deck` (e2e test).

## Requirements

1. **Sources**: from `@hugeicons/core-free-icons` 4.4.0 (npm, MIT), stroke-rounded, 24×24 viewBox:

   | Icon        | Hugeicons name                 | File        |
   | ----------- | ------------------------------ | ----------- |
   | `claude`    | `Claude`                       | `claude`    |
   | `codex`     | `ChatGpt` (OpenAI's mark)      | `openai`    |
   | `opencode`  | none: DECK-12's `opencode.svg` | `opencode`  |
   | `terminal`  | `CommandLine`                  | `terminal`  |
   | `editor`    | `PencilEdit02`                 | `editor`    |
   | `git`       | `GitBranch`                    | `git`       |
   | `monitor`   | `DashboardSpeed02`             | `monitor`   |
   | `remote`    | `ArrowDataTransferHorizontal`  | `remote`    |
   | `build`     | `Wrench01`                     | `build`     |
   | `pager`     | `BookOpen01`                   | `pager`     |
   | `repl`      | `CodeSquare`                   | `repl`      |
   | `Expanded`  | `ArrowDown01`                  | `expanded`  |
   | `Collapsed` | `ArrowRight01`                 | `collapsed` |
   | `Spinner`   | `Loading02` (used by 03)       | `spinner`   |
   | `Waiting`   | `AlertCircle`                  | `waiting`   |
   | `Idle`      | `CheckmarkCircle02`            | `idle`      |
   | `Quiet`     | `Circle`                       | `quiet`     |
   | `Unseen`    | `Circle` with `fill` added     | `unseen`    |

   The package ships each icon as a JavaScript array of SVG elements (`dist/esm/<Name>Icon.js`); a throwaway script turns each into `<file>.svg` with `stroke="currentColor"`, `stroke-width="1.5"`, round caps and joins as given, on a 24×24 viewBox. DECK-12's Simple Icons `claude.svg` and `openai.svg` and their masks are replaced.

2. **Assets**: `crates/deck-tui/assets/logos/` becomes `crates/deck-tui/assets/icons/`, holding each `<file>.svg` and `<file>.mask` (64×64 8-bit alpha, row-major, 4096 bytes, the icon centred with a 4-pixel margin, as in DECK-12), made outside the build (for example `rsvg-convert -w 56 -h 56` and a throwaway padding script); nothing is added to the `justfile` or `.mise.toml`. Its `README.md` names the package and version, the Hugeicons MIT licence (with the licence text or a link to it), OpenCode's source as in DECK-12, the trademark note for the Claude, OpenAI and OpenCode marks, the `fill` change for `unseen`, and the exact commands. A unit test checks every mask is 4096 bytes and not all zero, and that every `Icon` but `DropTarget` has one.

3. **Images for every icon**: in the `Image` tier, every `Icon` but `Spinner` and `DropTarget` is drawn as its image's placeholder pair (DECK-12 requirement 7) in its two-cell slot, tinted with the colour its glyph would have had (01 requirement 6: `accent`, `warning`, `ok`, `muted`, the row's colour, Claude's `#d97757`), with `REVERSED` resolved as in DECK-12. Bold has no effect on an image (kitty reserves the attributes); the waiting mark's colour is enough. The `Spinner` stays its Nerd glyph frames; `DropTarget` stays its Nerd glyph for good. Hover rules (the caret's and the quiet mark's `muted` becoming `sidebar_fg`) and the dragged row's dimming (all `muted`) give the tint like any other colour, through the same style function the pre-pass uses.

4. **Registry**: DECK-12's registry goes from (logo, colour) to (`Icon`, colour) pairs, with the same rules: an id per pair from the one counter, each pair sent once per client run before the first frame that draws it, by the same pre-pass, and deleted on exit. The pre-pass covers every place of 01 requirement 5.

5. **Mask selection** is a pure function `mask(icon) -> &'static [u8; 4096]` over `include_bytes!`; `IconKind::Codex` uses `openai`.

## Screen

Ghostty, 28-column sidebar; each pair of letters stands for one two-cell image: `vv` the expanded caret, `>>` collapsed, `CC` Claude (orange), `OO` OpenAI, `TT` terminal, `EE` editor, `!!` waiting (`warning`), `ok` idle (`ok`), `••` unseen (`accent`), and `⠹` stands for the spinner's Nerd frame (one cell, then a blank):

```text
 main          + New space  │
                            │
 terminal-multiplexer    vv │
    EE editor               │
 ⠹  CC Fix login bug ••     │
 !! OO Review the API chan… │
 ok TT ~/Projects/terminal… │
 dotfiles !1           2 >> │
                            │
  + New             Resume  │
```

The columns are 01's; only the cells' content changes.

## Acceptance criteria

- [ ] Every `Icon` but `DropTarget` has a mask of 4096 bytes, not all zero; the SVGs, masks and README are committed, and running the README's commands remakes masks that match.
- [ ] `TestBackend`, `Image` tier: the caret, status, program icon and unseen slots hold placeholder pairs; the waiting and idle marks have different ids from each other and from the row icon; a working terminal's status slot holds its Nerd spinner frame; the grid border title's status slot of an idle Claude terminal holds a placeholder pair.
- [ ] Each (icon, colour) pair is transmitted once across many frames, before the first frame that draws it; exit deletes every id sent.
- [ ] With the `Nerd` and `Unicode` tiers, drawing is exactly 01's.
- [ ] End to end: with `TERM_PROGRAM=ghostty` and `COLORTERM=truecolor` set by the test, a fake `claude` panel row's program slot and its space row's caret slot hold U+10EEEE (capture `all-icon-images`); with `TMUX` added they hold Nerd glyphs.
- [ ] By hand, in Ghostty and kitty: every icon is legible at the default font size, follows its row's colour (accent on the active row), and disappears after detach.

## Tests

- `deck-tui`: masks (size, coverage, one per icon), the registry over (icon, colour) pairs, `TestBackend` drawing in the `Image` tier.
- `crates/deck/tests/titles.rs` (or the sidebar e2e file): the end-to-end criterion.

## Notes

- Hugeicons' free set is stroke only; a 1.5 stroke on 24 units is 3.5 pixels in a 56-pixel mask, about one device pixel at a normal cell height. If an icon reads too thin by hand, the worker may render the masks with a thicker stroke (`stroke-width` 2) and says so in the README.
- `architecture.md#dependencies` is unchanged: no crate; the assets' licence goes in the README and in `architecture.md#client`'s assets line.
