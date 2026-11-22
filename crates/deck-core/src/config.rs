//! `config.toml`: parsing, defaults and the default binding table.

use std::collections::BTreeMap;

use toml::Value;

use crate::keys::KeySpec;

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
            Self::Projects => "Projects",
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
        "scroll",
        &["["],
        HelpGroup::Terminal,
        "Scroll back through the history",
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
    def("new_project", &["P"], HelpGroup::Projects, "New project"),
    def(
        "rename_project",
        &["."],
        HelpGroup::Projects,
        "Rename the project",
    ),
    def(
        "close_project",
        &["Q"],
        HelpGroup::Projects,
        "Close the project",
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
    "keys",
];

/// The sidebar width, in columns, accepted by `sidebar_width`.
pub const SIDEBAR_WIDTH_RANGE: std::ops::RangeInclusive<u16> = 20..=60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub prefix: KeySpec,
    pub shell: Option<String>,
    pub shell_args: Vec<String>,
    pub scrollback_lines: usize,
    /// Whether the sidebar is shown (unless the window is too narrow).
    pub sidebar: bool,
    pub sidebar_width: u16,
    /// Action name to its keys; every action of [`DEFAULT_BINDINGS`] is present.
    pub keys: BTreeMap<String, Vec<KeySpec>>,
}

impl Default for Config {
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
            keys,
        }
    }
}

/// A parsed config and the warnings found on the way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedConfig {
    pub config: Config,
    pub warnings: Vec<String>,
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

impl Config {
    /// Parses config text; every key is optional.
    pub fn parse(text: &str) -> Result<ParsedConfig, ConfigError> {
        let table: toml::Table = text
            .parse()
            .map_err(|e: toml::de::Error| ConfigError::Syntax(e.to_string()))?;
        let mut config = Config::default();
        let mut warnings = Vec::new();
        let mut explicit: Vec<String> = Vec::new();
        for (key, value) in &table {
            if !TOP_LEVEL_KEYS.contains(&key.as_str()) {
                warnings.push(format!("unknown config key `{key}`"));
                continue;
            }
            match key.as_str() {
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
                        .ok_or_else(|| {
                            bad(key, format!("must be between {min} and {max}, got {n}"))
                        })?;
                }
                "keys" => {
                    let keys = value
                        .as_table()
                        .ok_or_else(|| bad(key, "expected a table"))?;
                    for (action, value) in keys {
                        let path = format!("keys.{action}");
                        if !DEFAULT_BINDINGS.iter().any(|a| a.name == action) {
                            warnings.push(format!("unknown action `{path}`"));
                            continue;
                        }
                        let specs = match value {
                            Value::Array(items) => items
                                .iter()
                                .map(|v| key_spec(&path, v))
                                .collect::<Result<Vec<_>, _>>()?,
                            single => vec![key_spec(&path, single)?],
                        };
                        config.keys.insert(action.clone(), specs);
                        explicit.push(action.clone());
                    }
                }
                _ => unreachable!("key checked against TOP_LEVEL_KEYS"),
            }
        }
        resolve_conflicts(&mut config, &explicit, &mut warnings);
        Ok(ParsedConfig { config, warnings })
    }
}

/// Makes every key belong to one action. A key the user set wins over the default keys of
/// the other actions; between two explicit settings the action that sorts first by name
/// wins. Each loss is a warning.
fn resolve_conflicts(config: &mut Config, explicit: &[String], warnings: &mut Vec<String>) {
    let mut explicit: Vec<&String> = explicit.iter().collect();
    explicit.sort();
    explicit.dedup();
    for (i, winner) in explicit.iter().enumerate() {
        let specs = config.keys[*winner].clone();
        let names: Vec<String> = config.keys.keys().cloned().collect();
        for other in names {
            if other == **winner {
                continue;
            }
            // An explicit action that sorts before the winner already took its keys.
            if explicit[..i].contains(&&other) {
                continue;
            }
            let list = config.keys.get_mut(&other).expect("listed above");
            for spec in &specs {
                let before = list.len();
                list.retain(|k| !k.matches(&spec.to_event()));
                if list.len() != before {
                    warnings.push(format!("key \"{spec}\" of {winner} replaces {other}'s"));
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

    #[test]
    fn default_toml_matches_defaults() {
        let parsed = Config::parse(DEFAULT_CONFIG_TOML).unwrap();
        assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
        assert_eq!(parsed.config, Config::default());
        for key in TOP_LEVEL_KEYS {
            let set = DEFAULT_CONFIG_TOML
                .lines()
                .any(|l| l.starts_with(&format!("{key} =")) || l.starts_with(&format!("[{key}]")));
            let example = DEFAULT_CONFIG_TOML
                .lines()
                .any(|l| l.starts_with(&format!("# {key} =")));
            assert!(set || example, "{key} missing from default.toml");
        }
        let table: toml::Table = DEFAULT_CONFIG_TOML.parse().unwrap();
        let keys = table["keys"].as_table().unwrap();
        for action in DEFAULT_BINDINGS {
            assert!(
                keys.contains_key(action.name),
                "{} missing from default.toml",
                action.name
            );
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
            scroll = ["Up", "k"]
            "#,
        )
        .unwrap();
        assert!(p.warnings.is_empty());
        assert_eq!(p.config.prefix, spec("C-a"));
        assert_eq!(p.config.shell.as_deref(), Some("/bin/zsh"));
        assert!(p.config.shell_args.is_empty());
        assert_eq!(p.config.scrollback_lines, 5);
        assert_eq!(p.config.keys["detach"], [spec("q")]);
        assert_eq!(p.config.keys["scroll"], [spec("Up"), spec("k")]);
    }

    #[test]
    fn unknown_names_warn() {
        let p = Config::parse("colour = 1\n[keys]\nteleport = \"t\"\ndetach = \"x\"").unwrap();
        assert_eq!(p.warnings.len(), 2, "{:?}", p.warnings);
        assert!(p.warnings.iter().any(|w| w.contains("colour")));
        assert!(p.warnings.iter().any(|w| w.contains("keys.teleport")));
        assert_eq!(p.config.keys["detach"], [spec("x")]);
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
        let p = Config::parse("sidebar = false\nsidebar_width = 60\n[keys]\nnew_panel = \"t\"")
            .unwrap();
        assert!(p.warnings.is_empty());
        assert!(!p.config.sidebar);
        assert_eq!(p.config.sidebar_width, 60);
        assert_eq!(p.config.keys["new_panel"], [spec("t")]);
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
            ("new_project", "P"),
            ("rename_project", "."),
            ("close_project", "Q"),
            ("toggle_sidebar", "b"),
            ("focus_sidebar", "Tab"),
            ("help", "?"),
        ] {
            assert_eq!(c.keys[action], [spec(key)], "{action}");
        }
        for def in DEFAULT_BINDINGS {
            assert!(!def.help.is_empty(), "{} has no help text", def.name);
        }
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
    fn two_explicit_settings_on_one_key_go_to_the_first_action_by_name() {
        let p = Config::parse("[keys]\nscroll = \"z\"\ndetach = \"z\"").unwrap();
        assert_eq!(p.config.keys["detach"], [spec("z")]);
        assert!(p.config.keys["scroll"].is_empty());
        assert_eq!(p.warnings, ["key \"z\" of detach replaces scroll's"]);
        // The defaults alone never conflict.
        assert!(Config::parse("").unwrap().warnings.is_empty());
    }
}
