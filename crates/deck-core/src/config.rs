//! `config.toml`: parsing, defaults and the default binding table.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use toml::Value;

use crate::agents::{AgentKind, check_template};
use crate::keys::KeySpec;
use crate::theme::Theme;

mod validate;

pub use validate::{Finding, Report, Severity, validate};

/// The documented defaults, printed by `deck config default` (DECK-08).
pub const DEFAULT_CONFIG_TOML: &str = include_str!("config/default.toml");

/// The help group an action is listed under, in the order the help overlay shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpGroup {
    Session,
    Panels,
    Projects,
    /// The open-ended group: actions of later specs land here unless they name another.
    Terminal,
    Sidebar,
}

impl HelpGroup {
    pub const ALL: [Self; 5] = [
        Self::Session,
        Self::Panels,
        Self::Projects,
        Self::Terminal,
        Self::Sidebar,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Session => "Session",
            Self::Panels => "Panels",
            Self::Projects => "Spaces",
            Self::Terminal => "Terminal",
            Self::Sidebar => "Sidebar",
        }
    }
}

/// One bindable action and its default keys (pressed after the prefix).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionDef {
    pub name: &'static str,
    pub keys: &'static [&'static str],
    pub group: HelpGroup,
    /// One line for the help overlay.
    pub help: &'static str,
}

const fn def(
    name: &'static str,
    keys: &'static [&'static str],
    group: HelpGroup,
    help: &'static str,
) -> ActionDef {
    ActionDef {
        name,
        keys,
        group,
        help,
    }
}

/// The single list of actions; later specs extend it.
pub const DEFAULT_BINDINGS: &[ActionDef] = &[
    def(
        "detach",
        &["d"],
        HelpGroup::Session,
        "Detach from the session",
    ),
    def(
        "sessions",
        &["s"],
        HelpGroup::Session,
        "Switch, create or kill sessions",
    ),
    def(
        "rename_session",
        &["$"],
        HelpGroup::Session,
        "Rename the session",
    ),
    def(
        "restore",
        &["R"],
        HelpGroup::Session,
        "Restore an agent session",
    ),
    def(
        "scroll",
        &["["],
        HelpGroup::Terminal,
        "Scroll back through the history",
    ),
    def(
        "new_terminal",
        &["t"],
        HelpGroup::Terminal,
        "New terminal in the panel",
    ),
    def(
        "close_terminal",
        &["x"],
        HelpGroup::Terminal,
        "Close the focused terminal",
    ),
    def(
        "focus_left",
        &["Left", "h"],
        HelpGroup::Terminal,
        "Focus the terminal on the left",
    ),
    def(
        "focus_right",
        &["Right", "l"],
        HelpGroup::Terminal,
        "Focus the terminal on the right",
    ),
    def(
        "focus_up",
        &["Up", "k"],
        HelpGroup::Terminal,
        "Focus the terminal above",
    ),
    def(
        "focus_down",
        &["Down", "j"],
        HelpGroup::Terminal,
        "Focus the terminal below",
    ),
    def(
        "resize_left",
        &["H"],
        HelpGroup::Terminal,
        "Resize the focused terminal to the left",
    ),
    def(
        "resize_right",
        &["L"],
        HelpGroup::Terminal,
        "Resize the focused terminal to the right",
    ),
    def(
        "resize_up",
        &["K"],
        HelpGroup::Terminal,
        "Resize the focused terminal upward",
    ),
    def(
        "resize_down",
        &["J"],
        HelpGroup::Terminal,
        "Resize the focused terminal downward",
    ),
    def(
        "reset_layout",
        &["="],
        HelpGroup::Terminal,
        "Reset the panel's terminal sizes",
    ),
    def("new_panel", &["c"], HelpGroup::Panels, "New panel"),
    def("next_panel", &["n"], HelpGroup::Panels, "Next panel"),
    def("prev_panel", &["p"], HelpGroup::Panels, "Previous panel"),
    def("panel_1", &["1"], HelpGroup::Panels, "Focus panel 1"),
    def("panel_2", &["2"], HelpGroup::Panels, "Focus panel 2"),
    def("panel_3", &["3"], HelpGroup::Panels, "Focus panel 3"),
    def("panel_4", &["4"], HelpGroup::Panels, "Focus panel 4"),
    def("panel_5", &["5"], HelpGroup::Panels, "Focus panel 5"),
    def("panel_6", &["6"], HelpGroup::Panels, "Focus panel 6"),
    def("panel_7", &["7"], HelpGroup::Panels, "Focus panel 7"),
    def("panel_8", &["8"], HelpGroup::Panels, "Focus panel 8"),
    def("panel_9", &["9"], HelpGroup::Panels, "Focus panel 9"),
    def(
        "rename_panel",
        &[","],
        HelpGroup::Panels,
        "Rename the panel",
    ),
    def("close_panel", &["X"], HelpGroup::Panels, "Close the panel"),
    def("new_space", &["P"], HelpGroup::Projects, "New space"),
    def(
        "new_menu",
        &["N"],
        HelpGroup::Projects,
        "New panel menu: shell or agent",
    ),
    def(
        "rename_space",
        &["."],
        HelpGroup::Projects,
        "Rename the space",
    ),
    def(
        "close_space",
        &["Q"],
        HelpGroup::Projects,
        "Close the space",
    ),
    def(
        "toggle_sidebar",
        &["b"],
        HelpGroup::Sidebar,
        "Show or hide the sidebar",
    ),
    def(
        "focus_sidebar",
        &["Tab"],
        HelpGroup::Sidebar,
        "Move focus into the sidebar",
    ),
    def(
        "next_attention",
        &["a"],
        HelpGroup::Panels,
        "Jump to the next agent that needs you",
    ),
    def("help", &["?"], HelpGroup::Sidebar, "This help"),
];

/// Every key accepted at the top level of `config.toml`.
pub const TOP_LEVEL_KEYS: &[&str] = &[
    "prefix",
    "shell",
    "shell_args",
    "scrollback_lines",
    "sidebar",
    "sidebar_width",
    "notify",
    "resume_agents",
    "keys",
    "theme",
    "agents",
];

/// The sidebar width, in columns, accepted by `sidebar_width`.
pub const SIDEBAR_WIDTH_RANGE: std::ops::RangeInclusive<u16> = 20..=60;

/// How the client tells the user, through the outer terminal, that an agent needs them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotifyMode {
    /// OSC 9, a desktop notification in terminals that know it.
    #[default]
    Osc9,
    Bell,
    None,
}

impl NotifyMode {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "osc9" => Some(Self::Osc9),
            "bell" => Some(Self::Bell),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}

/// The resolved config. This is the wire form too: plain types that postcard encodes. The
/// TOML shapes that accept a string or an array, or a string or an integer, exist only
/// while loading (`apply`), where they are converted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub prefix: KeySpec,
    pub shell: Option<String>,
    pub shell_args: Vec<String>,
    pub scrollback_lines: usize,
    /// Whether the sidebar is shown (unless the window is too narrow).
    pub sidebar: bool,
    pub sidebar_width: u16,
    pub notify: NotifyMode,
    /// Reopen each terminal's agent on its conversation after a daemon restart.
    pub resume_agents: bool,
    /// Action name to its keys; every action of [`DEFAULT_BINDINGS`] is present.
    pub keys: BTreeMap<String, Vec<KeySpec>>,
    pub theme: Theme,
    pub agents: AgentsConfig,
}

/// How deck finds one agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Program names (the basename of the command as typed, or of the script an interpreter
    /// runs, without extension) that mean this agent.
    pub process: Vec<String>,
    /// The command a new panel for this agent runs: a program and arguments split on
    /// whitespace.
    pub command: String,
    /// The command that reopens a conversation; `{session_id}` is a whole argument.
    pub resume: String,
    /// The command that reopens the most recent conversation, for when no valid session id
    /// is known.
    pub resume_latest: String,
}

/// `[agents.<kind>]`: one table per agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentsConfig {
    pub claude: AgentConfig,
    pub codex: AgentConfig,
    pub opencode: AgentConfig,
}

impl AgentsConfig {
    pub fn get(&self, kind: AgentKind) -> &AgentConfig {
        match kind {
            AgentKind::Claude => &self.claude,
            AgentKind::Codex => &self.codex,
            AgentKind::OpenCode => &self.opencode,
        }
    }

    fn get_mut(&mut self, kind: AgentKind) -> &mut AgentConfig {
        match kind {
            AgentKind::Claude => &mut self.claude,
            AgentKind::Codex => &mut self.codex,
            AgentKind::OpenCode => &mut self.opencode,
        }
    }
}

impl Default for AgentsConfig {
    fn default() -> Self {
        let one = |name: &str, resume: &str, resume_latest: &str| AgentConfig {
            process: vec![name.to_string()],
            command: name.to_string(),
            resume: resume.to_string(),
            resume_latest: resume_latest.to_string(),
        };
        Self {
            claude: one(
                "claude",
                "claude --resume {session_id}",
                "claude --continue",
            ),
            codex: one("codex", "codex resume {session_id}", "codex resume --last"),
            opencode: one(
                "opencode",
                "opencode --session {session_id}",
                "opencode --continue",
            ),
        }
    }
}

impl Default for Config {
    #[expect(
        clippy::expect_used,
        reason = "the built-in bindings and prefix are verified by the default config tests"
    )]
    fn default() -> Self {
        let keys = DEFAULT_BINDINGS
            .iter()
            .map(|a| {
                let specs = a
                    .keys
                    .iter()
                    .map(|k| KeySpec::parse(k).expect("default bindings are valid keys"))
                    .collect();
                (a.name.to_string(), specs)
            })
            .collect();
        Self {
            prefix: KeySpec::parse("M-Space").expect("valid default prefix"),
            shell: None,
            shell_args: vec!["-l".to_string()],
            scrollback_lines: 10_000,
            sidebar: true,
            sidebar_width: 28,
            notify: NotifyMode::default(),
            resume_agents: true,
            keys,
            theme: Theme::default(),
            agents: AgentsConfig::default(),
        }
    }
}

/// A parsed config and the warnings found on the way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedConfig {
    pub config: Config,
    pub warnings: Vec<String>,
}

/// Something wrong but harmless in the file; `key` is its full name, e.g. `keys.teleport`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    pub key: String,
    pub message: String,
}

impl Warning {
    pub(crate) fn unknown_key(key: &str) -> Self {
        Self {
            key: key.to_string(),
            message: format!("unknown config key `{key}`"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("config is not valid TOML: {0}")]
    Syntax(String),
    #[error("config key `{key}`: {message}")]
    Value { key: String, message: String },
}

fn bad(key: &str, message: impl Into<String>) -> ConfigError {
    ConfigError::Value {
        key: key.to_string(),
        message: message.into(),
    }
}

fn string(key: &str, value: &Value) -> Result<String, ConfigError> {
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| bad(key, "expected a string"))
}

fn key_spec(key: &str, value: &Value) -> Result<KeySpec, ConfigError> {
    KeySpec::parse(&string(key, value)?).map_err(|e| bad(key, e.to_string()))
}

/// Everything found in a config table: the config and every error and warning.
#[derive(Debug)]
pub(crate) struct Loaded {
    pub config: Config,
    pub errors: Vec<ConfigError>,
    pub warnings: Vec<Warning>,
}

impl Config {
    /// Parses config text; every key is optional. The first error is returned.
    pub fn parse(text: &str) -> Result<ParsedConfig, ConfigError> {
        let table: toml::Table = text
            .parse()
            .map_err(|e: toml::de::Error| ConfigError::Syntax(e.to_string()))?;
        let loaded = load(&table);
        if let Some(error) = loaded.errors.into_iter().next() {
            return Err(error);
        }
        Ok(ParsedConfig {
            config: loaded.config,
            warnings: loaded.warnings.into_iter().map(|w| w.message).collect(),
        })
    }
}

/// Reads a whole config table, collecting every error instead of stopping at the first.
pub(crate) fn load(table: &toml::Table) -> Loaded {
    let mut config = Config::default();
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut explicit: Vec<String> = Vec::new();
    for (key, value) in table {
        if !TOP_LEVEL_KEYS.contains(&key.as_str()) {
            warnings.push(Warning::unknown_key(key));
            continue;
        }
        if let Err(e) = apply(
            &mut config,
            key,
            value,
            &mut explicit,
            &mut errors,
            &mut warnings,
        ) {
            errors.push(e);
        }
    }
    resolve_conflicts(&mut config, &explicit, &mut errors, &mut warnings);
    Loaded {
        config,
        errors,
        warnings,
    }
}

/// Reads one top-level key. Errors of single entries in `[keys]` go to `errors` so the
/// others are still read.
fn apply(
    config: &mut Config,
    key: &str,
    value: &Value,
    explicit: &mut Vec<String>,
    errors: &mut Vec<ConfigError>,
    warnings: &mut Vec<Warning>,
) -> Result<(), ConfigError> {
    match key {
        "prefix" => config.prefix = key_spec(key, value)?,
        "shell" => config.shell = Some(string(key, value)?),
        "shell_args" => {
            let items = value
                .as_array()
                .ok_or_else(|| bad(key, "expected an array of strings"))?;
            config.shell_args = items
                .iter()
                .map(|v| string(key, v))
                .collect::<Result<_, _>>()?;
        }
        "scrollback_lines" => {
            let n = value
                .as_integer()
                .ok_or_else(|| bad(key, "expected an integer"))?;
            config.scrollback_lines = usize::try_from(n)
                .map_err(|_| bad(key, format!("must not be negative, got {n}")))?;
        }
        "sidebar" => {
            config.sidebar = value
                .as_bool()
                .ok_or_else(|| bad(key, "expected true or false"))?;
        }
        "sidebar_width" => {
            let n = value
                .as_integer()
                .ok_or_else(|| bad(key, "expected an integer"))?;
            let (min, max) = (*SIDEBAR_WIDTH_RANGE.start(), *SIDEBAR_WIDTH_RANGE.end());
            config.sidebar_width = u16::try_from(n)
                .ok()
                .filter(|w| SIDEBAR_WIDTH_RANGE.contains(w))
                .ok_or_else(|| bad(key, format!("must be between {min} and {max}, got {n}")))?;
        }
        "notify" => {
            config.notify = value
                .as_str()
                .and_then(NotifyMode::parse)
                .ok_or_else(|| bad(key, "expected \"osc9\", \"bell\" or \"none\""))?;
        }
        "resume_agents" => {
            config.resume_agents = value
                .as_bool()
                .ok_or_else(|| bad(key, "expected true or false"))?;
        }
        "keys" => {
            let keys = value
                .as_table()
                .ok_or_else(|| bad(key, "expected a table"))?;
            for (action, value) in keys {
                let path = format!("keys.{action}");
                if !DEFAULT_BINDINGS.iter().any(|a| a.name == action) {
                    warnings.push(Warning {
                        key: path.clone(),
                        message: format!("unknown action `{path}`"),
                    });
                    continue;
                }
                let specs = match value {
                    Value::Array(items) => items
                        .iter()
                        .map(|v| key_spec(&path, v))
                        .collect::<Result<Vec<_>, _>>(),
                    single => key_spec(&path, single).map(|spec| vec![spec]),
                };
                match specs {
                    Ok(specs) => {
                        config.keys.insert(action.clone(), specs);
                        explicit.push(action.clone());
                    }
                    Err(e) => errors.push(e),
                }
            }
        }
        "theme" => {
            let theme = value
                .as_table()
                .ok_or_else(|| bad(key, "expected a table"))?;
            config.theme = Theme::parse(theme, warnings).map_err(|e| bad(&e.key, e.message))?;
        }
        "agents" => {
            let agents = value
                .as_table()
                .ok_or_else(|| bad(key, "expected a table"))?;
            for (name, value) in agents {
                let path = format!("agents.{name}");
                let Some(kind) = AgentKind::parse(name) else {
                    warnings.push(Warning::unknown_key(&path));
                    continue;
                };
                let table = value
                    .as_table()
                    .ok_or_else(|| bad(&path, "expected a table"))?;
                for (field, value) in table {
                    let path = format!("{path}.{field}");
                    let agent = config.agents.get_mut(kind);
                    match field.as_str() {
                        "process" => {
                            let items = value
                                .as_array()
                                .ok_or_else(|| bad(&path, "expected an array of strings"))?;
                            agent.process = items
                                .iter()
                                .map(|v| string(&path, v))
                                .collect::<Result<_, _>>()?;
                        }
                        "command" => {
                            let command = string(&path, value)?;
                            check_command(&command).map_err(|e| bad(&path, e))?;
                            agent.command = command;
                        }
                        "resume" | "resume_latest" => {
                            let template = string(&path, value)?;
                            check_template(&template, field == "resume")
                                .map_err(|e| bad(&path, e))?;
                            if field == "resume" {
                                agent.resume = template;
                            } else {
                                agent.resume_latest = template;
                            }
                        }
                        _ => warnings.push(Warning::unknown_key(&path)),
                    }
                }
            }
        }
        _ => unreachable!("key checked against TOP_LEVEL_KEYS"),
    }
    Ok(())
}

/// Checks an agent's `command`: a template without placeholders whose program, when it has
/// a `/`, is an absolute path.
fn check_command(command: &str) -> Result<(), String> {
    check_template(command, false)?;
    match command.split_whitespace().next() {
        Some(program) if program.contains('/') && !program.starts_with('/') => {
            Err("a program with a `/` must be an absolute path".to_string())
        }
        _ => Ok(()),
    }
}

/// Makes every key belong to one action. A key the user set wins over the default keys of
/// the other actions, with a warning; a key the user set for two actions is an error.
fn resolve_conflicts(
    config: &mut Config,
    explicit: &[String],
    errors: &mut Vec<ConfigError>,
    warnings: &mut Vec<Warning>,
) {
    let mut explicit: Vec<&String> = explicit.iter().collect();
    explicit.sort();
    explicit.dedup();
    for (i, first) in explicit.iter().enumerate() {
        let Some(first_specs) = config.keys.get(*first) else {
            continue;
        };
        for second in explicit.iter().skip(i + 1) {
            let Some(second_specs) = config.keys.get(*second) else {
                continue;
            };
            for spec in first_specs {
                if second_specs.iter().any(|k| k.matches(&spec.to_event())) {
                    errors.push(bad(
                        &format!("keys.{second}"),
                        format!("key \"{spec}\" is bound to both {first} and {second}"),
                    ));
                }
            }
        }
    }
    for winner in &explicit {
        let Some(specs) = config.keys.get(*winner).cloned() else {
            continue;
        };
        for (other, list) in &mut config.keys {
            if explicit.contains(&other) {
                continue;
            }
            for spec in &specs {
                let before = list.len();
                list.retain(|k| !k.matches(&spec.to_event()));
                if list.len() != before {
                    warnings.push(Warning {
                        key: format!("keys.{winner}"),
                        message: format!("key \"{spec}\" of {winner} replaces {other}'s"),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(s: &str) -> KeySpec {
        KeySpec::parse(s).unwrap()
    }

    #[test]
    fn defaults() {
        let c = Config::default();
        assert_eq!(c.prefix, spec("M-Space"));
        assert_eq!(c.shell, None);
        assert_eq!(c.shell_args, ["-l"]);
        assert_eq!(c.scrollback_lines, 10_000);
        assert!(c.sidebar);
        assert_eq!(c.sidebar_width, 28);
        assert_eq!(c.keys["detach"], [spec("d")]);
        assert_eq!(c.keys["scroll"], [spec("[")]);
        let parsed = Config::parse("").unwrap();
        assert_eq!(parsed.config, c);
        assert!(parsed.warnings.is_empty());
    }

    /// Whether `default.toml` has `name` set, as a table, or as a `# name = ...` example.
    /// `table` is the dotted name of the table the key lives in ("" at the top level); an
    /// example line only counts inside that table's section of the file.
    fn documented(parsed: &toml::Table, table: &str, name: &str) -> bool {
        let mut section = String::new();
        parsed.contains_key(name)
            || DEFAULT_CONFIG_TOML.lines().any(|l| {
                if let Some(header) = l.strip_prefix('[') {
                    section = header.trim_end_matches(']').to_string();
                }
                section == table && l.starts_with(&format!("# {name} ="))
            })
    }

    /// Every key of `value` (a serialized default) must be documented in `parsed`.
    fn assert_documented(prefix: &str, value: &toml::Table, parsed: &toml::Table) {
        for (key, inner) in value {
            let path = format!("{prefix}{key}");
            let table = prefix.trim_end_matches('.');
            assert!(
                documented(parsed, table, key),
                "{path} missing from default.toml"
            );
            // Descend only into tables the file also has as tables (`keys`, `theme`).
            if let (Some(inner), Some(Value::Table(documented))) =
                (inner.as_table(), parsed.get(key))
            {
                assert_documented(&format!("{path}."), inner, documented);
            }
        }
    }

    #[test]
    fn default_toml_matches_defaults() {
        let parsed = Config::parse(DEFAULT_CONFIG_TOML).unwrap();
        assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
        assert_eq!(parsed.config, Config::default());

        let table: toml::Table = DEFAULT_CONFIG_TOML.parse().unwrap();
        // Every key of the defaults, serialized, and the keys that have no default value.
        let defaults = Value::try_from(Config::default()).unwrap();
        assert_documented("", defaults.as_table().unwrap(), &table);
        for key in TOP_LEVEL_KEYS {
            assert!(
                documented(&table, "", key),
                "{key} missing from default.toml"
            );
        }
        let keys = table["keys"].as_table().unwrap();
        for action in DEFAULT_BINDINGS {
            assert!(
                keys.contains_key(action.name),
                "{} missing from default.toml",
                action.name
            );
        }
        let theme = table["theme"].as_table().unwrap();
        for role in crate::theme::ROLES {
            assert!(theme.contains_key(role), "theme.{role} missing");
        }
        assert!(theme.contains_key("border_style"));
        assert!(theme.contains_key("icons"));
        assert!(theme.contains_key("icon_images"));
        assert!(theme.contains_key("icon_glyphs"));
        let glyphs = theme["glyphs"].as_table().unwrap();
        for name in [
            "expanded",
            "collapsed",
            "spinner",
            "waiting",
            "idle",
            "quiet",
            "unseen",
        ] {
            assert!(glyphs.contains_key(name), "theme.glyphs.{name} missing");
        }
        // Each commented icon line names its kind's `unicode` glyph.
        let unicode = crate::theme::IconGlyphs::preset(crate::theme::IconPreset::Unicode);
        for kind in crate::icons::IconKind::ALL {
            let want = format!("# {} = \"{}\"", kind.name(), unicode.get(kind));
            assert!(
                DEFAULT_CONFIG_TOML.lines().any(|l| l.starts_with(&want)),
                "default.toml lacks `{want}`"
            );
        }
    }

    #[test]
    fn notify_defaults_to_osc9_and_takes_three_values() {
        assert_eq!(Config::default().notify, NotifyMode::Osc9);
        for (text, mode) in [
            ("osc9", NotifyMode::Osc9),
            ("bell", NotifyMode::Bell),
            ("none", NotifyMode::None),
        ] {
            let p = Config::parse(&format!("notify = \"{text}\"")).unwrap();
            assert_eq!(p.config.notify, mode);
        }
        for text in ["notify = \"toast\"", "notify = true"] {
            let err = Config::parse(text).unwrap_err();
            assert!(err.to_string().contains("`notify`"), "{err}");
        }
    }

    #[test]
    fn agent_commands_are_checked() {
        let p = Config::parse(
            "[agents.claude]\ncommand = \"claude --model fast\"\n[agents.codex]\ncommand = \"/opt/bin/codex\"",
        )
        .unwrap();
        assert_eq!(p.config.agents.claude.command, "claude --model fast");
        assert_eq!(p.config.agents.codex.command, "/opt/bin/codex");
        assert_eq!(p.config.agents.opencode.command, "opencode");
        for text in [
            "[agents.claude]\ncommand = \"claude $HOME\"",
            "[agents.claude]\ncommand = \"claude 'x'\"",
            "[agents.claude]\ncommand = \"claude \\\\x\"",
            "[agents.claude]\ncommand = \"\"",
            "[agents.claude]\ncommand = \"  \"",
            "[agents.claude]\ncommand = \"claude {session_id}\"",
            "[agents.claude]\ncommand = \"bin/claude\"",
            "[agents.claude]\ncommand = \"./claude\"",
            "[agents.claude]\ncommand = 1",
        ] {
            match Config::parse(text) {
                Err(ConfigError::Value { key, .. }) => {
                    assert_eq!(key, "agents.claude.command", "{text}");
                }
                other => panic!("{text}: {other:?}"),
            }
        }
    }

    #[test]
    fn resume_templates_and_the_switch_default_and_can_be_set() {
        let c = Config::default();
        assert!(c.resume_agents);
        assert_eq!(c.agents.claude.command, "claude");
        assert_eq!(c.agents.codex.command, "codex");
        assert_eq!(c.agents.opencode.command, "opencode");
        assert_eq!(c.agents.claude.resume, "claude --resume {session_id}");
        assert_eq!(c.agents.codex.resume_latest, "codex resume --last");
        let p = Config::parse(
            "resume_agents = false\n[agents.claude]\nresume = \"cc -r {session_id}\"\nresume_latest = \"cc -c\"",
        )
        .unwrap();
        assert!(!p.config.resume_agents);
        assert_eq!(p.config.agents.claude.resume, "cc -r {session_id}");
        assert_eq!(p.config.agents.claude.resume_latest, "cc -c");
        assert_eq!(p.config.agents.codex, c.agents.codex);
        for (text, key) in [
            (
                "[agents.claude]\nresume = \"claude 'x'\"",
                "agents.claude.resume",
            ),
            (
                "[agents.codex]\nresume = \"codex $HOME\"",
                "agents.codex.resume",
            ),
            ("[agents.codex]\nresume = \"\"", "agents.codex.resume"),
            (
                "[agents.opencode]\nresume_latest = \"oc \\\\x\"",
                "agents.opencode.resume_latest",
            ),
            (
                "[agents.opencode]\nresume_latest = \"oc {session_id}\"",
                "agents.opencode.resume_latest",
            ),
            ("[agents.claude]\nresume = 1", "agents.claude.resume"),
            ("resume_agents = \"yes\"", "resume_agents"),
        ] {
            match Config::parse(text) {
                Err(ConfigError::Value { key: k, .. }) => assert_eq!(k, key, "{text}"),
                other => panic!("{text}: {other:?}"),
            }
        }
    }

    #[test]
    fn the_agent_process_lists_default_and_can_be_set() {
        let c = Config::default();
        assert_eq!(c.agents.claude.process, ["claude"]);
        assert_eq!(c.agents.codex.process, ["codex"]);
        assert_eq!(c.agents.opencode.process, ["opencode"]);
        let p = Config::parse(
            "[agents.claude]\nprocess = [\"claude\", \"cc\"]\n[agents.vim]\nx = 1\n[agents.codex]\nrun = 1",
        )
        .unwrap();
        assert_eq!(p.config.agents.claude.process, ["claude", "cc"]);
        assert_eq!(p.config.agents.codex.process, ["codex"]);
        assert_eq!(p.warnings.len(), 2, "{:?}", p.warnings);
        for text in [
            "agents = 1",
            "[agents]\nclaude = 1",
            "[agents.claude]\nprocess = \"x\"",
            "[agents.claude]\nprocess = [1]",
        ] {
            assert!(Config::parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn parses_values() {
        let p = Config::parse(
            r#"
            prefix = "C-a"
            shell = "/bin/zsh"
            shell_args = []
            scrollback_lines = 5
            [keys]
            detach = "q"
            scroll = ["F5", "z"]
            "#,
        )
        .unwrap();
        assert!(p.warnings.is_empty());
        assert_eq!(p.config.prefix, spec("C-a"));
        assert_eq!(p.config.shell.as_deref(), Some("/bin/zsh"));
        assert!(p.config.shell_args.is_empty());
        assert_eq!(p.config.scrollback_lines, 5);
        assert_eq!(p.config.keys["detach"], [spec("q")]);
        assert_eq!(p.config.keys["scroll"], [spec("F5"), spec("z")]);
    }

    #[test]
    fn unknown_names_warn() {
        let p = Config::parse("colour = 1\n[keys]\nteleport = \"t\"\ndetach = \"w\"").unwrap();
        assert_eq!(p.warnings.len(), 2, "{:?}", p.warnings);
        assert!(p.warnings.iter().any(|w| w.contains("colour")));
        assert!(p.warnings.iter().any(|w| w.contains("keys.teleport")));
        assert_eq!(p.config.keys["detach"], [spec("w")]);
        assert!(!p.config.keys.contains_key("teleport"));
    }

    #[test]
    fn bad_values_name_the_key() {
        for (text, key) in [
            ("scrollback_lines = -1", "scrollback_lines"),
            ("scrollback_lines = \"x\"", "scrollback_lines"),
            ("prefix = \"C-\"", "prefix"),
            ("prefix = 3", "prefix"),
            ("shell = 1", "shell"),
            ("shell_args = [1]", "shell_args"),
            ("shell_args = \"-l\"", "shell_args"),
            ("[keys]\ndetach = \"X-a\"", "keys.detach"),
            ("[keys]\ndetach = [\"d\", 2]", "keys.detach"),
            ("keys = 1", "keys"),
            ("sidebar = 1", "sidebar"),
            ("sidebar_width = 19", "sidebar_width"),
            ("sidebar_width = 61", "sidebar_width"),
            ("sidebar_width = -3", "sidebar_width"),
            ("sidebar_width = \"x\"", "sidebar_width"),
            ("theme = 1", "theme"),
            ("[theme]\naccent = \"purple\"", "theme.accent"),
        ] {
            let err = Config::parse(text).unwrap_err();
            match &err {
                ConfigError::Value { key: k, .. } => assert_eq!(k, key, "{text}"),
                other => panic!("{text}: {other:?}"),
            }
            assert!(err.to_string().contains(&format!("`{key}`")), "{err}");
        }
        assert!(matches!(
            Config::parse("prefix = "),
            Err(ConfigError::Syntax(_))
        ));
    }

    #[test]
    fn the_sidebar_keys_and_the_binding_table() {
        let p = Config::parse("sidebar = false\nsidebar_width = 60\n[keys]\nnew_panel = \"T\"")
            .unwrap();
        assert!(p.warnings.is_empty());
        assert!(!p.config.sidebar);
        assert_eq!(p.config.sidebar_width, 60);
        assert_eq!(p.config.keys["new_panel"], [spec("T")]);
        assert_eq!(
            Config::parse("sidebar_width = 20")
                .unwrap()
                .config
                .sidebar_width,
            20
        );

        let c = Config::default();
        for (action, key) in [
            ("new_panel", "c"),
            ("next_panel", "n"),
            ("prev_panel", "p"),
            ("panel_1", "1"),
            ("panel_9", "9"),
            ("rename_panel", ","),
            ("close_panel", "X"),
            ("new_space", "P"),
            ("new_menu", "N"),
            ("restore", "R"),
            ("rename_space", "."),
            ("close_space", "Q"),
            ("toggle_sidebar", "b"),
            ("focus_sidebar", "Tab"),
            ("next_attention", "a"),
            ("help", "?"),
        ] {
            assert_eq!(c.keys[action], [spec(key)], "{action}");
        }
        for def in DEFAULT_BINDINGS {
            assert!(!def.help.is_empty(), "{} has no help text", def.name);
        }
    }

    #[test]
    fn new_menu_defaults_to_n_in_the_projects_group_and_can_be_overridden() {
        let def = DEFAULT_BINDINGS
            .iter()
            .find(|d| d.name == "new_menu")
            .unwrap();
        assert_eq!(def.group, HelpGroup::Projects);
        assert_eq!(Config::default().keys["new_menu"], [spec("N")]);
        let p = Config::parse("[keys]\nnew_menu = \"M\"").unwrap();
        assert_eq!(p.config.keys["new_menu"], [spec("M")]);
        assert!(p.warnings.is_empty());
    }

    #[test]
    fn restore_defaults_to_r_in_the_session_group_and_can_be_overridden() {
        let def = DEFAULT_BINDINGS
            .iter()
            .find(|d| d.name == "restore")
            .unwrap();
        assert_eq!(def.group, HelpGroup::Session);
        assert_eq!(Config::default().keys["restore"], [spec("R")]);
        let p = Config::parse("[keys]\nrestore = \"Z\"").unwrap();
        assert_eq!(p.config.keys["restore"], [spec("Z")]);
        assert!(p.warnings.is_empty());
    }

    #[test]
    fn an_explicit_key_replaces_the_default_of_another_action() {
        let p = Config::parse("[keys]\ntoggle_sidebar = \"c\"\ndetach = \"X\"").unwrap();
        assert_eq!(p.config.keys["toggle_sidebar"], [spec("c")]);
        assert!(p.config.keys["new_panel"].is_empty());
        assert_eq!(p.config.keys["detach"], [spec("X")]);
        assert!(p.config.keys["close_panel"].is_empty());
        assert!(
            p.warnings
                .contains(&"key \"c\" of toggle_sidebar replaces new_panel's".to_string())
        );
        assert!(
            p.warnings
                .contains(&"key \"X\" of detach replaces close_panel's".to_string())
        );
        assert_eq!(p.warnings.len(), 2);
    }

    #[test]
    fn two_explicit_settings_on_one_key_are_an_error_naming_both() {
        let err = Config::parse("[keys]\nscroll = \"z\"\ndetach = \"z\"").unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("detach") && text.contains("scroll") && text.contains("\"z\""),
            "{text}"
        );
        assert!(matches!(err, ConfigError::Value { ref key, .. } if key == "keys.scroll"));
        // The defaults alone never conflict.
        assert!(Config::parse("").unwrap().warnings.is_empty());
    }

    #[test]
    fn a_user_key_taking_a_default_warns_and_wins() {
        let p = Config::parse("[keys]\ndetach = \"n\"").unwrap();
        assert_eq!(p.config.keys["detach"], [spec("n")]);
        assert!(p.config.keys["next_panel"].is_empty());
        assert_eq!(p.warnings, ["key \"n\" of detach replaces next_panel's"]);
        // Both set: the user's own choice for next_panel is not taken away.
        let err = Config::parse("[keys]\ndetach = \"n\"\nnext_panel = \"n\"").unwrap_err();
        assert!(err.to_string().contains("detach") && err.to_string().contains("next_panel"));
        // Unbinding.
        let p = Config::parse("[keys]\ndetach = []").unwrap();
        assert!(p.config.keys["detach"].is_empty() && p.warnings.is_empty());
    }

    #[test]
    fn the_raw_shapes_load_and_the_resolved_config_round_trips_through_postcard() {
        let one = Config::parse("[keys]\ndetach = \"d\"\n[theme]\naccent = 4").unwrap();
        let many =
            Config::parse("[keys]\ndetach = [\"d\", \"q\"]\n[theme]\naccent = \"blue\"").unwrap();
        assert_eq!(one.config.keys["detach"], [spec("d")]);
        assert_eq!(many.config.keys["detach"], [spec("d"), spec("q")]);
        assert_eq!(one.config.theme.accent, many.config.theme.accent);
        for config in [Config::default(), one.config, many.config] {
            let bytes = postcard::to_stdvec(&config).unwrap();
            assert_eq!(postcard::from_bytes::<Config>(&bytes).unwrap(), config);
        }
        let themed = Config::parse(
            "shell = \"/bin/zsh\"\n[theme]\naccent = \"#cba6f7\"\nborder_style = \"plain\"",
        )
        .unwrap()
        .config;
        let bytes = postcard::to_stdvec(&themed).unwrap();
        assert_eq!(postcard::from_bytes::<Config>(&bytes).unwrap(), themed);
    }
}
