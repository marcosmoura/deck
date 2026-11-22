//! `deck config default | check | path`: read-only, never starts a daemon.

use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::Subcommand;
use deck_core::config::{DEFAULT_CONFIG_TOML, Severity, validate};
use deck_core::paths;

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Prints the commented default config, with every key.
    Default,
    /// Checks a config file (the one in use by default) and lists what is wrong.
    Check {
        /// The file to check.
        file: Option<PathBuf>,
    },
    /// Prints the path of the config file in use.
    Path,
}

pub fn run(command: ConfigCommand) -> anyhow::Result<ExitCode> {
    match command {
        ConfigCommand::Default => {
            print!("{DEFAULT_CONFIG_TOML}");
            Ok(ExitCode::SUCCESS)
        }
        ConfigCommand::Path => {
            let path = config_path()?;
            let note = if path.exists() { "" } else { " (not found)" };
            println!("{}{note}", path.display());
            Ok(ExitCode::SUCCESS)
        }
        ConfigCommand::Check { file } => check(file),
    }
}

fn config_path() -> anyhow::Result<PathBuf> {
    Ok(paths::config_file(&|name| std::env::var(name).ok())?)
}

fn check(file: Option<PathBuf>) -> anyhow::Result<ExitCode> {
    let path = match file {
        Some(path) => path,
        None => config_path()?,
    };
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            println!("no config file at {}; using defaults", path.display());
            return Ok(ExitCode::SUCCESS);
        }
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
    };
    let report = validate(&text);
    for finding in &report.findings {
        let level = match finding.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        println!(
            "{}:{}:{}: {level}: {}",
            path.display(),
            finding.line,
            finding.column,
            finding.message
        );
    }
    if report.has_errors() {
        return Ok(ExitCode::FAILURE);
    }
    println!("ok: {}", path.display());
    Ok(ExitCode::SUCCESS)
}
