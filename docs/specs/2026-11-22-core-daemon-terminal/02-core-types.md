---
type: foundation
status: done
---

# 02 — Core types

## Scope

Everything the daemon and the client share, in `deck-core`, pure and unit-tested: the model, the protocol and its framing, paths, config, key specs, the default binding table and key encoding. Sub-specs 03 (daemon), 04 (CLI and harness) and 05 (client) build on it. Owning crate: `deck-core`.

## Requirements

1. **Model** (`model.rs`): `SessionId`, `ProjectId`, `PanelId`, `TerminalId` (`u64` newtypes, serde, `Copy`, `Ord`, `Display`); `Session`, `Project`, `Panel`, `Terminal` with the fields of [architecture.md](../../architecture.md#model) (layout ratios and agent fields are added by DECK-03 and DECK-06, not now); `NameSource { Default, Manual, Auto }`; `LaunchSpec { program: Option<String>, args: Vec<String>, cwd: PathBuf, env: Vec<(String, String)> }`, where `program: None` means the configured shell. `SessionView` is a serializable snapshot of one session for clients: session id and name, every project, panel and terminal with ids, names and titles, the focus, and `layout_viewport: Option<(u16, u16)>`, the panel area the daemon sized the active panel's terminals for ([architecture.md#protocol](../../architecture.md#protocol)).
2. **Protocol** (`protocol.rs`), as in [architecture.md#protocol](../../architecture.md#protocol):
   - `PROTOCOL_VERSION: u32 = 1`.
   - `ClientMsg { Hello { version, role, cwd: PathBuf, env: Vec<(String, String)>, colors: Option<OuterColors> }, Viewport { cols, rows }, Input { terminal, bytes }, Detach, Request(Request) }`; `Role { Attach { session: Option<String>, create: bool, cols: u16, rows: u16 }, Control }`; `Request { ListSessions, KillServer }`; `OuterColors { fg: (u8, u8, u8), bg: (u8, u8, u8) }`, the outer terminal's colours as the client found them (sub-spec 05).
   - `ServerMsg { Welcome { session }, SessionView(SessionView), Frame(Frame), Clipboard { selection: String, text: String }, Bye { reason }, Response(Response) }` (`Clipboard` carries a program's OSC 52 copy to the client that writes it to the outer terminal); `ByeReason { VersionMismatch { daemon: u32 }, SessionEnded, ServerShutdown, Error(String) }`; `Response { Sessions(Vec<SessionSummary>), Ok, Error(String) }` with `SessionSummary { id, name, attached_clients, terminals }`.
   - `Hello` is the first variant of `ClientMsg` and `version` its first field; `Bye` keeps its variant index and `VersionMismatch` its shape in every future version, so a client and a daemon of different versions can always exchange them.
   - **Frames and cells**: `Frame { terminal, size: (u16, u16), cursor: Cursor, modes: Modes, display_offset: u32, history: u32, lines: Vec<(u16, Line)>, full: bool }`; `Line { runs: Vec<Run> }`; `Run { col: u16, text: String, style: Style }`, where `col` is the emulator column of the run's first cell. A wide character (emulator `WIDE_CHAR`) occupies two columns and is followed by no text for its spacer cell (`WIDE_CHAR_SPACER` and leading spacers are skipped); zero-width characters (the cell's combining extras) are appended to their base character's text. Blank default-styled cells at the end of a line are not sent. `Style { fg: Color, bg: Color, flags: u16 }` (bold, dim, italic, underline, reverse, strikethrough, hidden), `Color { Default, Indexed(u8), Rgb(u8, u8, u8) }`, `Cursor { row, col, visible, shape: Block | Bar | Underline }`, `Modes` (`u16` flags: application cursor, bracketed paste, mouse click / drag / motion, SGR mouse, focus reporting, alternate screen).
   - Framing: `encode(&msg) -> Vec<u8>`, `decode(&[u8])`, and a streaming `FrameReader` that yields whole messages from partial reads and rejects frames over 16 MiB.
3. **Paths** (`paths.rs`): resolve the config file, state dir and runtime dir from an injected environment (a small trait or a `&dyn Fn(&str) -> Option<String>`), following [architecture.md#paths](../../architecture.md#paths) including `DECK_HOME`. `socket_path()` errors when the path is 100 bytes or longer, naming the path and the limit.
4. **Config** (`config.rs`): `Config { prefix: KeySpec, shell: Option<String>, shell_args: Vec<String>, scrollback_lines: usize, keys: BTreeMap<String, Vec<KeySpec>> }`, defaults `M-Space`, `None`, `["-l"]`, `10000`, and the default binding table (requirement 6). Parsed from TOML text; a `[keys]` value may be a string or an array of strings (`detach = "d"` or `focus_left = ["Left", "h"]`), and replaces that action's default keys. An unknown top-level key or action name yields a warning in the result; a bad value is an error naming the key.
5. **`KeySpec`** (`keys.rs`): parses `C-`, `M-`, `S-` modifiers (combinable, e.g. `C-M-x`), single characters, and the names `Space`, `Enter`, `Tab`, `Esc`, `Backspace`, `Insert`, `Delete`, `Up`, `Down`, `Left`, `Right`, `Home`, `End`, `PageUp`, `PageDown`, `F1`–`F12`; `Display` gives the same form back. A `KeyEvent` type (code plus modifiers) is defined here, and `KeySpec::matches(&KeyEvent)`; the client converts crossterm events into it, so `deck-core` does not depend on crossterm. For character keys, `matches` compares the character and ignores the `Shift` flag (terminals report `X` with `Shift` set), so a binding written `X` matches a shifted `x`; `S-` is meaningful only for named keys (`S-Tab`, `S-Up`).
6. **Default binding table** (`config.rs`): the single list of actions with their default keys and a help group, which later specs extend. This sub-spec owns `detach = "d"` (group Session) and `scroll = "["` (group Terminal; used by sub-spec 06).
7. **`keys::encode(key: &KeyEvent, modes: Modes) -> Vec<u8>`**, xterm encoding: printable characters as UTF-8; `Enter` `\r`; `Tab` `\t`; `Shift+Tab` `ESC[Z`; `Backspace` `0x7f`; `Esc` `0x1b`; arrows `ESC[A`… or `ESC O A`… in application cursor mode; `Home`/`End` likewise (`ESC[H`/`ESC[F`, `ESC O H`/`ESC O F`); `Insert`, `Delete`, `PageUp`, `PageDown` as `ESC[2~`, `ESC[3~`, `ESC[5~`, `ESC[6~`; `F1`–`F4` as `ESC O P`–`S`, `F5`–`F12` as `ESC[15~`, `17~`, `18~`, `19~`, `20~`, `21~`, `23~`, `24~`; `Ctrl+a`–`Ctrl+z` as `0x01`–`0x1a`; `Option+Space` and `Ctrl+@` as `0x00`; `Ctrl+[ \ ] ^ _` as `0x1b`–`0x1f`, and `Ctrl+4`, `Ctrl+5`, `Ctrl+6`, `Ctrl+7` (how crossterm reports the bytes `0x1c`–`0x1f`) as `0x1c`–`0x1f`, so `Ctrl+\` (SIGQUIT) and `Ctrl+]` survive the round trip; `Alt+<key>` as `ESC` followed by the key's encoding; arrows, `Home`, `End`, `Insert`, `Delete`, `PageUp`, `PageDown` and `F1`–`F12` with modifiers use the xterm parameter form (`ESC[1;<m>A`, `ESC[5;<m>~`, `ESC[1;<m>P`) with `m = 1 + shift + 2·alt + 4·ctrl`. `KeySpec` converts to a `KeyEvent`, so the control API (DECK-05) can encode key names too.

8. **`base64`** (`base64.rs`): a small standard-alphabet encoder with padding, for the OSC 52 sequences the client writes (no new crate).

## Acceptance criteria

- [ ] Every `ClientMsg` and `ServerMsg` variant round-trips through `encode`/`decode`, and `FrameReader` reassembles messages split at every byte boundary and rejects a 16 MiB + 1 frame.
- [ ] A `Hello` encoded by this version decodes its `version` field even when the rest of the message is garbage (test with a truncated and a corrupted tail), so a daemon can answer `VersionMismatch` to a future client.
- [ ] `KeySpec` parses every listed form, round-trips through `Display`, and rejects `C-`, `X-a` and an empty string with an error naming the input.
- [ ] Config: defaults; `keys` as a string and as an array; an unknown key and an unknown action give warnings; `scrollback_lines = -1` and `prefix = "C-"` give errors naming the key.
- [ ] Paths follow the table in [architecture.md#paths](../../architecture.md#paths) for every combination of `DECK_HOME`, `XDG_*` and `TMPDIR`, and a 100-byte socket path is an error.
- [ ] `keys::encode` matches requirement 7 for every listed key, with and without application cursor mode and with each modifier combination listed, including `Ctrl+4`…`Ctrl+7`.
- [ ] `KeySpec::parse("X").matches` a `KeyEvent` of `X` with `Shift` set and without it; `KeySpec::parse("S-Tab")` does not match a plain `Tab`.
- [ ] `base64` matches the RFC 4648 test vectors.
- [ ] Every `Hello`, `Clipboard` and `OuterColors` shape round-trips.

## Tests

- Unit tests in `deck-core` for every criterion above (table-driven for keys and paths).

## Notes

- The cell model of requirement 2 is what keeps wide and combining characters aligned; sub-spec 03 builds frames from the emulator by it and sub-spec 05 draws by it.
- `deck-core::theme::Theme`: a plain struct with the role colours, border style and glyphs of [DECK-08 sub-spec 01](../2026-11-22-config-theming/01-theme.md) and their defaults, without parsing. `deck-tui` draws every non-terminal colour and glyph from it from the start, so DECK-08 makes it configurable without touching the drawing code.
