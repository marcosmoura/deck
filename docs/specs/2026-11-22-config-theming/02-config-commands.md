---
type: shippable
status: draft
---

# 02 — Config reference and commands

## Scope

The user runs `deck config default > ~/.config/deck/config.toml` to start from a commented file with every key, `deck config check` to see what is wrong in their file, and `deck config path` to see which file deck reads. Owning crates: `deck-core` (reference file, validation with locations), `deck` (CLI).

## Requirements

1. **Reference file** `crates/deck-core/src/config/default.toml`: every key of `Config` and `Theme` with its default value, each preceded by a comment saying what it does and its allowed values, grouped in the order: general (`prefix`, `shell`, `shell_args`, `scrollback_lines`), sidebar, `[keys]` (every action with its default keys), `[theme]`, `[theme.glyphs]`. Embedded with `include_str!`.
2. **Sync test** in `deck-core`: parsing `default.toml` gives exactly `Config::default()`, and every key of `Config::default()` serialized to TOML, every `Theme` key and every action of the binding table appears in it, either set or, for optional keys and maps with no default entries (`shell`, DECK-06's `[agents.<kind>]`), as a commented example line `# <key> = …` that the test matches. A later spec that adds a key or an action fails this test until it documents the key. [architecture.md#config](../../architecture.md#config) states this rule.
3. **Locations**: `deck-core::config::validate(text) -> Report` returns errors and warnings with a line and column each: TOML syntax errors, bad values (DECK-01's error naming the key, plus its location), unknown keys (warnings, as in DECK-01), and theme errors from sub-spec 01. Uses the spans the `toml` crate provides; no new crate.
4. **`deck config default`**: prints `default.toml`.
5. **`deck config check [<file>]`**: validates the given file or the one in use. Prints `ok: <path>` and exits 0 when there are no errors; otherwise one line per finding, `<path>:<line>:<col>: error: <message>` or `… warning: …`, and exits 1 when there is at least one error (warnings alone exit 0). A missing file prints `no config file at <path>; using defaults` and exits 0.
6. **`deck config path`**: prints the path resolved from the current environment (`DECK_HOME`, `XDG_CONFIG_HOME`, the default), and ` (not found)` after it when the file does not exist. A running daemon reads the file of the environment it started with, which differs when those variables differ; `check` without a file argument uses the same resolution as `path`.
7. None of these commands start a daemon.

## Acceptance criteria

- [ ] `deck config default` output, saved as the config, gives `deck config check` → `ok` and a config equal to the defaults.
- [ ] The sync test fails when a field is added to `Config` without a line in `default.toml` (shown by the worker with a temporary change, reverted).
- [ ] A file with `prefix = "C-"`, `[theme] accent = "purple"`, `bogus = 1` and a TOML syntax error on line 9 reports each with its line and column, the first two and the syntax error as errors, `bogus` as a warning; exit code 1.
- [ ] A file with only an unknown key exits 0 with a warning.
- [ ] `deck config path` prints the `DECK_HOME` path in the harness and `$XDG_CONFIG_HOME/deck/config.toml` when that is set; ` (not found)` when missing.
- [ ] `deck config check` with no file prints the defaults message and exits 0.

## Tests

- `deck-core`: the sync test; `validate` locations for each kind of finding.
- `crates/deck/tests/config_cli.rs`: one scenario per command criterion.
