//! The `deck` binary: CLI, daemon start-up, control commands and wiring.

mod agent_event;
mod completion;
mod config_cmd;
mod control;
mod daemon_cmd;
mod guide;
mod hooks;
mod platform;
mod skills;
mod stats_cmd;
mod terminal_io;
mod tree;
mod upgrade_cmd;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueHint};
use clap_complete::ArgValueCandidates;

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
        #[arg(add = ArgValueCandidates::new(completion::sessions))]
        name: Option<String>,
        /// Create the session when it does not exist.
        #[arg(short, long, requires = "name")]
        create: bool,
    },
    /// Creates a session and attaches to it.
    New {
        /// The session's name (`main`, `main-2`, ... when omitted).
        name: Option<String>,
        /// The directory of the session's first space (default: the current one).
        #[arg(short = 'c', long = "dir", value_name = "DIR", value_hint = ValueHint::DirPath)]
        dir: Option<PathBuf>,
        /// Create the session without attaching; prints its name.
        #[arg(short, long)]
        detached: bool,
    },
    /// Kills a session and its terminals.
    KillSession {
        /// The session to kill.
        #[arg(add = ArgValueCandidates::new(completion::sessions))]
        name: String,
    },
    /// Renames a session.
    RenameSession {
        /// The session's current name.
        #[arg(add = ArgValueCandidates::new(completion::sessions))]
        old: String,
        /// Its new name.
        new: String,
    },
    /// Lists the sessions; `--json` and `--tree` show their spaces, panels and terminals.
    Ls(tree::LsArgs),
    /// Adds a space with one panel to a session; prints its id.
    #[command(name = "new-space")]
    NewProject(tree::NewProjectArgs),
    /// Adds a panel with one terminal to a space; prints the panel's id.
    NewPanel(tree::NewPanelArgs),
    /// Adds a terminal to a panel; prints its id. A program after `--` runs instead of a
    /// shell.
    NewTerminal(tree::NewTerminalArgs),
    /// Focuses a terminal (with its panel and space) or a panel.
    Focus(tree::IdArgs),
    /// Closes a terminal, a panel or a space, without asking.
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
    /// Shows what the daemon uses (memory, descriptors, threads, CPU) and holds; `--json` for
    /// scripts. Starts no daemon.
    Stats {
        /// Print one JSON document instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Stops the daemon; its sessions are restored by the next `deck`.
    KillServer {
        /// Also delete the saved state, so the next `deck` starts fresh.
        #[arg(long)]
        forget: bool,
    },
    /// Hands the running daemon over to this deck without stopping what runs in it.
    Upgrade,
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
    /// Installs, removes and checks the skill that teaches Claude Code, Codex and OpenCode to
    /// drive deck.
    Skills {
        #[command(subcommand)]
        command: skills::SkillsCommand,
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
    Daemon {
        /// A new image of the running daemon: the descriptor of the handoff file.
        /// Never 0, 1 or 2: the daemon takes the descriptor over and closes it.
        #[arg(
            long,
            value_name = "FD",
            conflicts_with = "handoff_check",
            value_parser = clap::value_parser!(i32).range(3..)
        )]
        adopt: Option<i32>,
        /// Exits 0 when this build reads handoff version N, else 1.
        #[arg(long = "handoff-check", value_name = "N")]
        handoff_check: Option<u32>,
    },
}

fn main() -> ExitCode {
    // A hook must never fail: any parse error of `agent-event` is silent and exits 0. It
    // comes before completion, so a `COMPLETE` left in an agent's environment changes
    // nothing (a shell's completion request has `--` first, never `agent-event`).
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
    // `COMPLETE=<shell> deck ...`: a shell asking for completions; answers and exits.
    completion::complete();
    let cli = Cli::parse();
    let result = match cli.command {
        Some(Command::Daemon {
            handoff_check: Some(version),
            ..
        }) => return daemon_cmd::handoff_check(version),
        Some(Command::Daemon { adopt, .. }) => daemon_cmd::run(adopt),
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
        Some(Command::Stats { json }) => Ok(stats_cmd::run(json)),
        Some(Command::KillServer { forget }) => control::kill_server(forget),
        Some(Command::Upgrade) => upgrade_cmd::run(),
        Some(Command::Config { command }) => config_cmd::run(command),
        Some(Command::Hooks { command }) => hooks::run(command),
        Some(Command::Skills { command }) => skills::run(command),
        Some(Command::AgentEvent(args)) => Ok(agent_event::run(&args)),
        Some(Command::Probe { session }) => control::probe(session),
    };
    result.unwrap_or_else(|e| {
        // The terminal may be gone (a closed tab); `eprintln!` would panic on it.
        let _ = std::io::Write::write_fmt(&mut std::io::stderr(), format_args!("deck: {e:#}\n"));
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
        assert!(Cli::try_parse_from(["deck", "daemon", "--adopt", "7"]).is_ok());
        assert!(Cli::try_parse_from(["deck", "daemon", "--adopt", "3"]).is_ok());
        // A standard descriptor is never the handoff file.
        for fd in ["0", "1", "2", "-1"] {
            let args = ["deck", "daemon", "--adopt", fd];
            assert!(Cli::try_parse_from(args).is_err(), "{fd}");
        }
        assert!(Cli::try_parse_from(["deck", "daemon", "--handoff-check", "1"]).is_ok());
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
