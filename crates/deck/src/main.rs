//! The `deck` binary: CLI, daemon start-up, control commands and wiring.

mod agent_event;
mod config_cmd;
mod control;
mod daemon_cmd;
mod guide;
mod hooks;
mod platform;
mod terminal_io;
mod tree;

use std::path::PathBuf;
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
        /// The session to attach to; it must exist unless `-c` is given.
        name: Option<String>,
        /// Create the session when it does not exist.
        #[arg(short, long, requires = "name")]
        create: bool,
    },
    /// Creates a session and attaches to it.
    New {
        /// The session's name (`main`, `main-2`, ... when omitted).
        name: Option<String>,
        /// The directory of the session's first project (default: the current one).
        #[arg(short = 'c', long = "dir", value_name = "DIR")]
        dir: Option<PathBuf>,
        /// Create the session without attaching; prints its name.
        #[arg(short, long)]
        detached: bool,
    },
    /// Kills a session and its terminals.
    KillSession {
        /// The session to kill.
        name: String,
    },
    /// Renames a session.
    RenameSession {
        /// The session's current name.
        old: String,
        /// Its new name.
        new: String,
    },
    /// Lists the sessions; `--json` and `--tree` show their projects, panels and terminals.
    Ls(tree::LsArgs),
    /// Adds a project with one panel to a session; prints its id.
    NewProject(tree::NewProjectArgs),
    /// Adds a panel with one terminal to a project; prints the panel's id.
    NewPanel(tree::NewPanelArgs),
    /// Adds a terminal to a panel; prints its id. A program after `--` runs instead of a
    /// shell.
    NewTerminal(tree::NewTerminalArgs),
    /// Focuses a terminal (with its panel and project) or a panel.
    Focus(tree::IdArgs),
    /// Closes a terminal, a panel or a project, without asking.
    Close(tree::IdArgs),
    /// Renames a panel.
    RenamePanel(tree::RenamePanelArgs),
    /// Types text into a terminal.
    Send(terminal_io::SendArgs),
    /// Types keys (`Enter`, `C-c`, `Up`, `M-x`, a character) into a terminal.
    SendKeys(terminal_io::SendKeysArgs),
    /// Prints what a terminal shows.
    Capture(terminal_io::CaptureArgs),
    /// Waits for a terminal's program to exit, for text on its screen or for it to go quiet.
    Wait(terminal_io::WaitArgs),
    /// Prints how an agent inside deck uses these commands (the text of the agent skill).
    Guide,
    /// Stops the daemon; its sessions are restored by the next `deck`.
    KillServer {
        /// Also delete the saved state, so the next `deck` starts fresh.
        #[arg(long)]
        forget: bool,
    },
    /// Prints the default config, checks a config file or shows where it lives.
    Config {
        #[command(subcommand)]
        command: config_cmd::ConfigCommand,
    },
    /// Installs, removes and checks the hooks that tell deck what Claude Code and Codex do.
    Hooks {
        #[command(subcommand)]
        command: hooks::HooksCommand,
    },
    /// Reports an agent's hook event from inside deck; used by the hooks, never prints.
    #[command(name = "agent-event", hide = true)]
    AgentEvent(agent_event::AgentEventArgs),
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
    // A hook must never fail: any parse error of `agent-event` is silent and exits 0.
    if std::env::args_os()
        .nth(1)
        .is_some_and(|a| a == "agent-event")
    {
        return match <agent_event::AgentEventArgs as Parser>::try_parse_from(
            std::env::args_os().skip(1),
        ) {
            Ok(args) => agent_event::run(&args),
            Err(_) => ExitCode::SUCCESS,
        };
    }
    let cli = Cli::parse();
    let result = match cli.command {
        Some(Command::Daemon) => daemon_cmd::run(),
        Some(Command::Attach { name, create }) => control::attach(name, create),
        None => control::attach(None, false),
        Some(Command::New {
            name,
            dir,
            detached,
        }) => control::new_session(name, dir, detached),
        Some(Command::KillSession { name }) => control::kill_session(name),
        Some(Command::RenameSession { old, new }) => control::rename_session(old, new),
        Some(Command::Ls(args)) => tree::run_ls(args),
        Some(Command::NewProject(args)) => Ok(tree::new_project(args)),
        Some(Command::NewPanel(args)) => Ok(tree::new_panel(args)),
        Some(Command::NewTerminal(args)) => Ok(tree::new_terminal(args)),
        Some(Command::Focus(args)) => Ok(tree::focus(args)),
        Some(Command::Close(args)) => Ok(tree::close(args)),
        Some(Command::RenamePanel(args)) => Ok(tree::rename_panel(args)),
        Some(Command::Send(args)) => Ok(terminal_io::send(args)),
        Some(Command::SendKeys(args)) => Ok(terminal_io::send_keys(args)),
        Some(Command::Capture(args)) => Ok(terminal_io::capture(args)),
        Some(Command::Wait(args)) => Ok(terminal_io::wait(args)),
        Some(Command::Guide) => Ok(guide::run()),
        Some(Command::KillServer { forget }) => control::kill_server(forget),
        Some(Command::Config { command }) => config_cmd::run(command),
        Some(Command::Hooks { command }) => hooks::run(command),
        Some(Command::AgentEvent(args)) => Ok(agent_event::run(&args)),
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

    #[test]
    fn the_session_commands_parse() {
        let ok = |args: &[&str]| {
            let mut argv = vec!["deck"];
            argv.extend_from_slice(args);
            Cli::try_parse_from(argv).is_ok()
        };
        assert!(ok(&["new"]) && ok(&["new", "x", "-c", "/tmp", "-d"]));
        assert!(ok(&["attach", "-c", "x"]) && ok(&["attach", "--create", "x"]));
        assert!(!ok(&["attach", "-c"]));
        assert!(ok(&["kill-session", "x"]) && !ok(&["kill-session"]));
        assert!(ok(&["rename-session", "a", "b"]) && !ok(&["rename-session", "a"]));
        assert!(ok(&["kill-server"]) && ok(&["kill-server", "--forget"]));
    }
}
