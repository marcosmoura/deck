//! The `deck` binary: CLI, daemon start-up, control commands and wiring.

mod control;
mod daemon_cmd;
mod platform;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "deck",
    version,
    about = "A terminal multiplexer for coding agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Attaches to a session (the most recent one, or a new `main`, without a name).
    Attach {
        /// The session to attach to; it must exist.
        name: Option<String>,
    },
    /// Lists the sessions.
    Ls,
    /// Stops the daemon.
    KillServer,
    /// Starts the daemon if needed, attaches and detaches again (for tests).
    #[command(name = "_probe", hide = true)]
    Probe {
        #[arg(long)]
        session: Option<String>,
    },
    /// Runs the daemon in the foreground.
    #[command(hide = true)]
    Daemon,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Some(Command::Daemon) => daemon_cmd::run(),
        Some(Command::Attach { name }) => control::attach(name),
        None => control::attach(None),
        Some(Command::Ls) => control::ls(),
        Some(Command::KillServer) => control::kill_server(),
        Some(Command::Probe { session }) => control::probe(session),
    };
    result.unwrap_or_else(|e| {
        eprintln!("deck: {e:#}");
        ExitCode::FAILURE
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn the_daemon_and_probe_subcommands_are_hidden_from_help() {
        let help = Cli::command().render_help().to_string();
        assert!(!help.contains("\n  daemon"), "{help}");
        assert!(!help.contains("_probe"), "{help}");
        assert!(Cli::try_parse_from(["deck", "daemon"]).is_ok());
        assert!(Cli::try_parse_from(["deck", "_probe", "--session", "x"]).is_ok());
    }
}
