//! `config.toml`: parsing, defaults and the default binding table.

use std::collections::BTreeMap;

use toml::Value;

use crate::keys::KeySpec;

/// The documented defaults, printed by `deck config default` (DECK-08).
pub const DEFAULT_CONFIG_TOML: &str = include_str!("config/default.toml");

/// The help group an action is listed under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpGroup {
    Session,
    Terminal,
}

/// One bindable action and its default keys (pressed after the prefix).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionDef {
    pub name: &'static str,
    pub keys: &'static [&'static str],
    pub group: HelpGroup,
}

/// The single list of actions; later specs extend it.
pub const DEFAULT_BINDINGS: &[ActionDef] = &[
    ActionDef {
        name: "detach",
        keys: &["d"],
        group: HelpGroup::Session,
    },
    ActionDef {
        name: "scroll",
        keys: &["["],
        group: HelpGroup::Terminal,
    },
];

/// Every key accepted at the top level of `config.toml`.
pub const TOP_LEVEL_KEYS: &[&str] = &["prefix", "shell", "shell_args", "scrollback_lines", "keys"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub prefix: KeySpec,
    pub shell: Option<String>,
    pub shell_args: Vec<String>,
    pub scrollback_lines: usize,
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
                    }
                }
                _ => unreachable!("key checked against TOP_LEVEL_KEYS"),
            }
        }
        Ok(ParsedConfig { config, warnings })
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
}
