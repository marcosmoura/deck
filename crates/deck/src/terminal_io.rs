//! The commands that type into a terminal, read its screen and wait for it: `send`,
//! `send-keys`, `capture` and `wait`.
//!
//! A terminal comes from `--terminal <id>` (ids are daemon-wide), else from the environment
//! like the tree commands do. The commands that type or wait refuse the caller's own
//! terminal unless `--terminal` names it: an agent must never type into itself by accident.

use std::io::{self, Read, Write};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use clap::{ArgGroup, Args};
use clap_complete::ArgValueCandidates;
use deck_core::keys::KeySpec;
use deck_core::model::{SessionView, TerminalId};
use deck_core::protocol::{Captured, Request, Response, WaitFor, Waited};
use serde_json::{Value, json};

use crate::tree::{Api, Common, Here, check_here, check_in_session, finish, session_here};

/// The most `send -` reads from stdin.
const MAX_INPUT: usize = 1024 * 1024;
/// The exit code of a `wait` that timed out, like `timeout(1)`.
const TIMED_OUT: u8 = 124;

#[derive(Debug, Clone, Args)]
pub struct SendArgs {
    #[command(flatten)]
    pub common: Common,
    /// The terminal (default: the one this command's session or panel shows; never your own
    /// without this option).
    #[arg(long, value_name = "ID", add = ArgValueCandidates::new(crate::completion::terminals))]
    pub terminal: Option<u64>,
    /// Press Enter after the text.
    #[arg(long)]
    pub enter: bool,
    /// Send the text as a paste: wrapped in bracketed-paste markers when the terminal asked
    /// for them, so a program takes several lines as one input.
    #[arg(long)]
    pub paste: bool,
    /// The text; `-` reads it from stdin (at most 1 MiB).
    pub text: String,
}

#[derive(Debug, Clone, Args)]
pub struct SendKeysArgs {
    #[command(flatten)]
    pub common: Common,
    /// The terminal (never your own without this option).
    #[arg(long, value_name = "ID", add = ArgValueCandidates::new(crate::completion::terminals))]
    pub terminal: Option<u64>,
    /// Keys: `Enter`, `Tab`, `Esc`, `Up`, `C-c`, `M-x`, `F5`, or a single character.
    #[arg(required = true, value_name = "KEY")]
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, Args)]
pub struct CaptureArgs {
    #[command(flatten)]
    pub common: Common,
    /// The terminal (default: your own).
    #[arg(long, value_name = "ID", add = ArgValueCandidates::new(crate::completion::terminals))]
    pub terminal: Option<u64>,
    /// Print only the last N lines.
    #[arg(long, value_name = "N")]
    pub lines: Option<u32>,
    /// Include the scrollback (ignored on the alternate screen).
    #[arg(long)]
    pub history: bool,
    /// Keep colours and attributes as escape sequences.
    #[arg(long, conflicts_with = "json")]
    pub ansi: bool,
}

#[derive(Debug, Clone, Args)]
#[command(group(ArgGroup::new("until").required(true).multiple(false).args(["exit", "text", "idle"])))]
pub struct WaitArgs {
    #[command(flatten)]
    pub common: Common,
    /// The terminal (never your own without this option).
    #[arg(long, value_name = "ID", add = ArgValueCandidates::new(crate::completion::terminals))]
    pub terminal: Option<u64>,
    /// Wait for the program to exit; prints its exit code.
    #[arg(long)]
    pub exit: bool,
    /// Wait for a line of the screen to contain this text; prints that line.
    #[arg(long, value_name = "TEXT")]
    pub text: Option<String>,
    /// Wait until the terminal printed nothing for this many milliseconds.
    #[arg(long, value_name = "MS")]
    pub idle: Option<u64>,
    /// Give up after this many seconds (decimals allowed): prints `timed out`, exits 124.
    #[arg(long, value_name = "SECONDS")]
    pub timeout: Option<f64>,
}

/// The terminal a command acts on.
#[derive(Debug, Clone, Copy)]
struct Chosen {
    id: u64,
    /// `--terminal` named it.
    explicit: bool,
    /// It is the terminal this command runs in.
    own: bool,
}

impl Chosen {
    /// Fails when the terminal is the caller's own and `--terminal` did not name it.
    fn not_by_accident(self) -> anyhow::Result<()> {
        if self.own && !self.explicit {
            bail!("refusing to act on your own terminal; pass --terminal <id>");
        }
        Ok(())
    }
}

/// The focused terminal of the session's active panel.
fn focused(session: &SessionView) -> Option<u64> {
    let active = session.active_panel?;
    session
        .projects
        .iter()
        .flat_map(|p| &p.panels)
        .find(|p| p.id == active)?
        .focused_terminal
        .map(|t| t.0)
}

/// `--terminal`, else this terminal (from the environment), else the focused terminal of
/// the session's active panel.
fn resolve(api: &mut Api, common: &Common, explicit: Option<u64>) -> anyhow::Result<Chosen> {
    let own_id = Here::from_env().terminal;
    if let Some(id) = explicit {
        if common.session.is_some() {
            let tree = api.tree(None)?;
            check_in_session(&tree, common.session.as_deref(), id)?;
        }
        return Ok(Chosen {
            id,
            explicit: true,
            own: own_id == Some(id),
        });
    }
    let tree = api.tree(None)?;
    let (session, defaults) = session_here(api, &tree, common)?;
    let from_env = match defaults {
        Some(here) => {
            check_here(&tree, here)?;
            here.terminal
        }
        None => None,
    };
    let id = from_env
        .or_else(|| focused(session))
        .ok_or_else(|| anyhow!("no terminal in session \"{}\"", session.name))?;
    Ok(Chosen {
        id,
        explicit: false,
        own: own_id == Some(id),
    })
}

fn terminal_id(chosen: Chosen) -> TerminalId {
    TerminalId(chosen.id)
}

/// The text to send: the argument, or all of stdin for `-`.
fn text_bytes(text: &str) -> anyhow::Result<Vec<u8>> {
    if text != "-" {
        return Ok(text.as_bytes().to_vec());
    }
    let mut bytes = Vec::new();
    io::stdin()
        .lock()
        .take(MAX_INPUT as u64 + 1)
        .read_to_end(&mut bytes)
        .context("cannot read stdin")?;
    if bytes.len() > MAX_INPUT {
        bail!("input too large");
    }
    Ok(bytes)
}

pub fn send(args: SendArgs) -> ExitCode {
    let json = args.common.json;
    let result: anyhow::Result<Option<Value>> = (|| {
        let mut api = Api::connect()?;
        let chosen = resolve(&mut api, &args.common, args.terminal)?;
        chosen.not_by_accident()?;
        let bytes = text_bytes(&args.text)?;
        let terminal = terminal_id(chosen);
        api.done(Request::Send {
            terminal,
            bytes,
            paste: args.paste,
        })?;
        // Enter comes after the closing paste marker, so it is a request of its own.
        if args.enter {
            api.done(Request::Send {
                terminal,
                bytes: b"\r".to_vec(),
                paste: false,
            })?;
        }
        Ok(Some(json!({ "ok": true })))
    })();
    finish(json, result)
}

pub fn send_keys(args: SendKeysArgs) -> ExitCode {
    let json = args.common.json;
    let result: anyhow::Result<Option<Value>> = (|| {
        // A bad key fails before anything is sent.
        let keys = args
            .keys
            .iter()
            .map(|k| KeySpec::parse(k))
            .collect::<Result<Vec<_>, _>>()?;
        let mut api = Api::connect()?;
        let chosen = resolve(&mut api, &args.common, args.terminal)?;
        chosen.not_by_accident()?;
        api.done(Request::SendKeys {
            terminal: terminal_id(chosen),
            keys,
        })
        .map(Some)
    })();
    finish(json, result)
}

/// The document of `capture --json`.
fn capture_json(c: &Captured) -> Value {
    json!({
        "terminal": c.terminal.0,
        "cols": c.size.0,
        "rows": c.size.1,
        "alternate_screen": c.alternate_screen,
        "truncated": c.truncated > 0,
        "cursor": { "row": c.cursor.0, "col": c.cursor.1 },
        "lines": c.lines,
    })
}

/// Writes `text` to stdout; a reader that went away (`| head`) ends the output quietly.
fn write_out(text: &str) {
    let mut out = io::stdout().lock();
    let _ = out.write_all(text.as_bytes()).and_then(|()| out.flush());
}

pub fn capture(args: CaptureArgs) -> ExitCode {
    let json = args.common.json;
    let result: anyhow::Result<Option<Value>> = (|| {
        let mut api = Api::connect()?;
        let chosen = resolve(&mut api, &args.common, args.terminal)?;
        let captured = match api.ask(Request::Capture {
            terminal: terminal_id(chosen),
            lines: args.lines,
            history: args.history,
            ansi: args.ansi,
        })? {
            Response::Captured(c) => c,
            _ => bail!("unexpected answer from the daemon"),
        };
        if json {
            return Ok(Some(capture_json(&captured)));
        }
        let mut text = String::new();
        if captured.truncated > 0 {
            text.push_str(&format!(
                "[truncated: {} earlier lines]\n",
                captured.truncated
            ));
        }
        for line in &captured.lines {
            text.push_str(line);
            text.push('\n');
        }
        write_out(&text);
        Ok(None)
    })();
    finish(json, result)
}

/// The timeout in whole milliseconds, rounded up.
fn timeout_ms(seconds: f64) -> anyhow::Result<u64> {
    let limit = Duration::try_from_secs_f64(seconds)
        .map_err(|_| anyhow!("invalid --timeout {seconds}: expected a number of seconds"))?;
    Ok(u64::try_from(limit.as_nanos().div_ceil(1_000_000)).unwrap_or(u64::MAX))
}

pub fn wait(args: WaitArgs) -> ExitCode {
    let json = args.common.json;
    let result: anyhow::Result<Waited> = (|| {
        let until = match (&args.text, args.idle) {
            (Some(text), _) => WaitFor::Text(text.clone()),
            (None, Some(quiet_ms)) => WaitFor::Idle { quiet_ms },
            (None, None) => WaitFor::Exit,
        };
        let timeout_ms = args.timeout.map(timeout_ms).transpose()?;
        let mut api = Api::connect()?;
        let chosen = resolve(&mut api, &args.common, args.terminal)?;
        chosen.not_by_accident()?;
        if chosen.own && until == WaitFor::Exit {
            bail!("cannot wait for your own terminal to exit");
        }
        // The daemon enforces the limit; the answer may take as long as it takes.
        api.wait_for_answers()?;
        match api.ask(Request::Wait {
            terminal: terminal_id(chosen),
            until,
            timeout_ms,
        })? {
            Response::Waited(waited) => Ok(waited),
            _ => bail!("unexpected answer from the daemon"),
        }
    })();
    match result {
        Ok(Waited::TimedOut) => {
            if json {
                println!("{}", json!({ "timed_out": true }));
            } else {
                eprintln!("timed out");
            }
            ExitCode::from(TIMED_OUT)
        }
        Ok(waited) => {
            let value = match &waited {
                Waited::Exit(code) => json!({ "exit": code }),
                Waited::Text(line) => json!({ "line": line }),
                _ => json!({ "idle": true }),
            };
            if !json {
                match &waited {
                    Waited::Exit(code) => write_out(&format!("{code}\n")),
                    Waited::Text(line) => write_out(&format!("{line}\n")),
                    _ => {}
                }
            }
            finish(json, Ok(Some(value)))
        }
        Err(e) => finish(json, Err(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timeout_is_rounded_up_to_milliseconds() {
        assert_eq!(timeout_ms(5.0).unwrap(), 5000);
        assert_eq!(timeout_ms(0.0).unwrap(), 0);
        assert_eq!(timeout_ms(0.0001).unwrap(), 1);
        assert_eq!(timeout_ms(1.5).unwrap(), 1500);
        assert!(timeout_ms(-1.0).is_err());
        assert!(timeout_ms(f64::NAN).is_err());
    }

    #[test]
    fn only_a_terminal_named_by_the_option_may_be_your_own() {
        let own = |explicit| Chosen {
            id: 4,
            explicit,
            own: true,
        };
        assert!(own(true).not_by_accident().is_ok());
        assert_eq!(
            own(false).not_by_accident().unwrap_err().to_string(),
            "refusing to act on your own terminal; pass --terminal <id>"
        );
        let other = Chosen {
            id: 5,
            explicit: false,
            own: false,
        };
        assert!(other.not_by_accident().is_ok());
    }
}
