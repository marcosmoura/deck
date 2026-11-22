//! Coding agents (Claude Code, Codex, OpenCode) in terminals: the per-terminal state, the
//! urgency order and the matching of a process's argv to an agent.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::AgentsConfig;

mod opencode;

pub use opencode::{OPENCODE_EVENTS, OpenCodeTracker, OpenCodeUpdate};

/// The agents deck knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentKind {
    Claude,
    Codex,
    OpenCode,
}

impl AgentKind {
    pub const ALL: [Self; 3] = [Self::Claude, Self::Codex, Self::OpenCode];

    /// The name used in `config.toml` (`[agents.<name>]`), in `ls --json` and in titles.
    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.name() == name)
    }
}

/// What an agent is doing in a terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentStatus {
    None,
    Working,
    /// Needs the user: a permission prompt or a question.
    Waiting,
    /// Finished its turn, ready for the next prompt.
    Idle,
    /// Detected by process only: printing.
    Active,
    /// Detected by process only: not printing.
    Quiet,
}

impl AgentStatus {
    /// How much the user's attention is wanted: higher is more urgent. The order is
    /// waiting, working, active, idle, quiet, none.
    pub fn urgency(self) -> u8 {
        match self {
            Self::Waiting => 5,
            Self::Working => 4,
            Self::Active => 3,
            Self::Idle => 2,
            Self::Quiet => 1,
            Self::None => 0,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Working => "working",
            Self::Waiting => "waiting",
            Self::Idle => "idle",
            Self::Active => "active",
            Self::Quiet => "quiet",
        }
    }

    /// The most urgent of some statuses (`None` for no statuses at all).
    pub fn most_urgent(statuses: impl IntoIterator<Item = Self>) -> Self {
        statuses
            .into_iter()
            .max_by_key(|s| s.urgency())
            .unwrap_or(Self::None)
    }
}

/// Where a terminal's agent state came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentSource {
    /// The agent's own hooks: precise.
    Hooks,
    /// The PTY's foreground process: only printing or not.
    Process,
}

impl AgentSource {
    pub fn name(self) -> &'static str {
        match self {
            Self::Hooks => "hooks",
            Self::Process => "process",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentState {
    pub kind: AgentKind,
    pub status: AgentStatus,
    pub source: AgentSource,
    pub session_id: Option<String>,
    /// The first line (at most [`PROMPT_MAX`] characters) of the session's first prompt.
    pub first_prompt: Option<String>,
    /// When the status last changed, on the daemon's clock.
    pub since: Duration,
}

/// The longest first prompt kept, in characters.
pub const PROMPT_MAX: usize = 200;

/// The first line of a prompt, at most [`PROMPT_MAX`] characters; `None` when it is blank.
pub fn clip_prompt(prompt: &str) -> Option<String> {
    let line = prompt.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(line.chars().take(PROMPT_MAX).collect())
}

/// What a hook event of Claude Code or Codex tells deck.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookUpdate {
    pub status: AgentStatus,
    pub session_id: Option<String>,
    pub prompt: Option<String>,
}

/// The hook events deck installs a hook for, per agent. OpenCode has no hooks of this kind.
pub fn hook_events(kind: AgentKind) -> &'static [&'static str] {
    match kind {
        AgentKind::Claude => &[
            "SessionStart",
            "UserPromptSubmit",
            "PostToolUse",
            "PermissionRequest",
            "Notification",
            "Stop",
            "SessionEnd",
        ],
        AgentKind::Codex => &[
            "SessionStart",
            "UserPromptSubmit",
            "PostToolUse",
            "PermissionRequest",
            "Stop",
            "Interrupt",
            "SessionEnd",
        ],
        AgentKind::OpenCode => &[],
    }
}

/// Maps a Claude Code or Codex hook event to what it tells deck; `None` for events deck
/// ignores. The `session_id` of the payload is always passed on; the `prompt` only for
/// `UserPromptSubmit`.
pub fn map_hook_event(
    kind: AgentKind,
    event: &str,
    notification_type: Option<&str>,
    session_id: Option<&str>,
    prompt: Option<&str>,
) -> Option<HookUpdate> {
    let status = match (kind, event) {
        (AgentKind::OpenCode, _) => return None,
        (_, "SessionStart") => AgentStatus::Idle,
        (_, "UserPromptSubmit" | "PostToolUse") => AgentStatus::Working,
        (_, "PermissionRequest") => AgentStatus::Waiting,
        (_, "SessionEnd") => AgentStatus::None,
        (AgentKind::Claude, "Stop") => AgentStatus::Idle,
        (AgentKind::Claude, "Notification") => match notification_type {
            Some("permission_prompt" | "elicitation_dialog") => AgentStatus::Waiting,
            Some("idle_prompt") => AgentStatus::Idle,
            _ => return None,
        },
        (AgentKind::Codex, "Stop" | "Interrupt") => AgentStatus::Idle,
        _ => return None,
    };
    Some(HookUpdate {
        status,
        session_id: session_id.map(str::to_string),
        prompt: if event == "UserPromptSubmit" {
            prompt.map(str::to_string)
        } else {
            None
        },
    })
}

/// Programs that run a script named in their first argument.
const INTERPRETERS: [&str; 7] = ["node", "bun", "deno", "sh", "bash", "zsh", "dash"];

fn basename(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
}

const SHELLS: [&str; 4] = ["sh", "bash", "zsh", "dash"];

/// What follows the resume command in the script of a resumed shell terminal.
pub const RESUME_EXEC_TAIL: &str = "; exec \"$0\" \"$@\"";

/// The name of the program a process runs, from its argv: the basename of `argv[0]` as typed
/// (not the resolved path) without a login dash; when that is an interpreter and `argv[1]`
/// exists and is not an option, the basename of `argv[1]` instead (the script it runs). A
/// shell that runs deck's resume wrapper (`sh -c '<command>; exec "$0" "$@"' <shell> ...`)
/// has no job control, so the agent stays in its process group: the program is then the
/// first word of the command.
pub fn program_name(argv: &[String]) -> Option<String> {
    let first = basename(argv.first()?);
    let first = first.strip_prefix('-').unwrap_or(first);
    if SHELLS.contains(&first)
        && argv.get(1).is_some_and(|a| a == "-c")
        && let Some(command) = argv.get(2).and_then(|c| c.strip_suffix(RESUME_EXEC_TAIL))
        && let Some(word) = command.split_whitespace().next()
    {
        return Some(basename(word).to_string());
    }
    if INTERPRETERS.contains(&first)
        && let Some(script) = argv.get(1).filter(|a| !a.starts_with('-'))
    {
        return Some(basename(script).to_string());
    }
    Some(first.to_string())
}

/// Which agent a process is: its [`program_name`] without extension (`codex.js` is `codex`)
/// is in an agent's `process` list.
pub fn match_argv(argv: &[String], config: &AgentsConfig) -> Option<AgentKind> {
    let name = program_name(argv)?;
    let stem = Path::new(&name)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(&name);
    AgentKind::ALL
        .into_iter()
        .find(|k| config.get(*k).process.iter().any(|p| p == stem))
}

/// What the state file keeps about a terminal's agent so a restart can reopen it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentResume {
    pub kind: AgentKind,
    pub session_id: Option<String>,
}

/// The only place a session id may appear in a resume template: as a whole argument.
pub const SESSION_ID_PLACEHOLDER: &str = "{session_id}";

/// The longest session id used.
const SESSION_ID_MAX: usize = 128;

/// Whether `id` may be put on a command line: `^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$`. The
/// first character is never `-`, so an id can never become an option.
pub fn valid_session_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && id.len() <= SESSION_ID_MAX
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
}

/// Characters with a meaning to `sh` (quotes, escapes, expansions, operators, globs, comments).
/// They are refused in templates so the command can be joined with spaces and run by `sh -c`.
fn is_shell_syntax(c: char) -> bool {
    matches!(
        c,
        '\'' | '"'
            | '\\'
            | '$'
            | '`'
            | ';'
            | '&'
            | '|'
            | '<'
            | '>'
            | '('
            | ')'
            | '*'
            | '?'
            | '['
            | ']'
            | '{'
            | '}'
            | '#'
            | '~'
            | '!'
    ) || c.is_control()
}

/// Checks a resume template: program and arguments split on whitespace, no shell syntax,
/// and `{session_id}` only as a whole argument after the program (`allow_id`: the
/// `resume_latest` template has no id to fill in).
pub fn check_template(template: &str, allow_id: bool) -> Result<(), String> {
    if template.split_whitespace().next().is_none() {
        return Err("expected a command".to_string());
    }
    for (i, word) in template.split_whitespace().enumerate() {
        if word == SESSION_ID_PLACEHOLDER {
            if i == 0 {
                return Err("the program cannot be `{session_id}`".to_string());
            }
            if !allow_id {
                return Err("`{session_id}` is not available here".to_string());
            }
        } else if let Some(c) = word.chars().find(|c| is_shell_syntax(*c)) {
            let shown = if c.is_control() {
                "a control character".to_string()
            } else {
                format!("`{c}`")
            };
            return Err(format!(
                "{shown} is not allowed (`{{session_id}}` only as a whole argument)"
            ));
        }
    }
    Ok(())
}

/// The program and arguments of a template, with `session_id` in place of `{session_id}`.
/// Call [`check_template`] first.
pub fn render_template(template: &str, session_id: Option<&str>) -> Vec<String> {
    template
        .split_whitespace()
        .map(|w| match (w == SESSION_ID_PLACEHOLDER, session_id) {
            (true, Some(id)) => id.to_string(),
            _ => w.to_string(),
        })
        .collect()
}

/// The command (program first) that reopens an agent's conversation: the `resume` template
/// with the session id when it is valid, else `resume_latest`. `None` when the template in
/// use is not valid (a config built in code can skip the load-time check).
pub fn resume_command(agents: &AgentsConfig, resume: &AgentResume) -> Option<Vec<String>> {
    let config = agents.get(resume.kind);
    let id = resume
        .session_id
        .as_deref()
        .filter(|id| valid_session_id(id));
    let (template, allow_id) = match id {
        Some(_) => (&config.resume, true),
        None => (&config.resume_latest, false),
    };
    check_template(template, allow_id).ok()?;
    Some(render_template(template, id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(ToString::to_string).collect()
    }

    fn matches(items: &[&str]) -> Option<AgentKind> {
        match_argv(&argv(items), &AgentsConfig::default())
    }

    #[test]
    fn agents_are_recognised_by_their_argv() {
        for (items, kind) in [
            (&["claude"][..], AgentKind::Claude),
            (&["/usr/local/bin/codex"], AgentKind::Codex),
            (&["node", "/x/opencode"], AgentKind::OpenCode),
            (&["node", "/x/codex.js"], AgentKind::Codex),
            (&["bun", "/x/opencode.ts"], AgentKind::OpenCode),
            (&["/bin/sh", "/tmp/fake/claude"], AgentKind::Claude),
            (&["bash", "./codex"], AgentKind::Codex),
            (&["-claude"], AgentKind::Claude),
        ] {
            assert_eq!(matches(items), Some(kind), "{items:?}");
        }
        // An option after the interpreter hides the script: not matched.
        assert_eq!(matches(&["node", "--inspect", "/x/claude"]), None);
    }

    #[test]
    fn other_programs_are_not_agents() {
        assert_eq!(matches(&["vim"]), None);
        assert_eq!(matches(&["node", "/x/server.js"]), None);
        assert_eq!(matches(&["sh"]), None);
        assert_eq!(matches(&["-zsh"]), None);
        assert_eq!(matches(&[]), None);
        assert_eq!(matches(&["/bin/sh", "-c", "claude"]), None);
    }

    #[test]
    fn a_custom_process_list_is_honoured() {
        let mut config = AgentsConfig::default();
        config.claude.process = vec!["cc".into()];
        assert_eq!(
            match_argv(&argv(&["/bin/cc"]), &config),
            Some(AgentKind::Claude)
        );
        assert_eq!(match_argv(&argv(&["claude"]), &config), None);
        config.codex.process = vec![];
        assert_eq!(match_argv(&argv(&["codex"]), &config), None);
    }

    #[test]
    fn the_program_name_unwraps_interpreters_only_for_scripts() {
        assert_eq!(program_name(&argv(&["-zsh"])).as_deref(), Some("zsh"));
        assert_eq!(program_name(&argv(&["node"])).as_deref(), Some("node"));
        assert_eq!(
            program_name(&argv(&["node", "-e", "x"])).as_deref(),
            Some("node")
        );
        assert_eq!(
            program_name(&argv(&["/usr/bin/env", "claude"])).as_deref(),
            Some("env")
        );
        assert_eq!(
            program_name(&argv(&["bash", "/a/b/run.sh"])).as_deref(),
            Some("run.sh")
        );
        assert_eq!(program_name(&[]), None);
    }

    #[test]
    fn a_resume_wrapper_is_the_agent_it_runs() {
        let wrapper = argv(&[
            "/bin/sh",
            "-c",
            "claude --resume abc; exec \"$0\" \"$@\"",
            "zsh",
            "-i",
        ]);
        assert_eq!(program_name(&wrapper).as_deref(), Some("claude"));
        assert_eq!(
            match_argv(&wrapper, &AgentsConfig::default()),
            Some(AgentKind::Claude)
        );
        let abs = argv(&[
            "bash",
            "-c",
            "/opt/bin/codex.js resume --last; exec \"$0\" \"$@\"",
            "zsh",
        ]);
        assert_eq!(
            match_argv(&abs, &AgentsConfig::default()),
            Some(AgentKind::Codex)
        );
        // Only deck's wrapper is looked through: other `-c` scripts stay the shell.
        assert_eq!(
            program_name(&argv(&["bash", "-c", "x"])).as_deref(),
            Some("bash")
        );
        assert_eq!(matches(&["/bin/sh", "-c", "claude"]), None);
    }

    #[test]
    fn urgency_orders_the_statuses() {
        use AgentStatus::{Active, Idle, None, Quiet, Waiting, Working};
        let order = [Waiting, Working, Active, Idle, Quiet, None];
        for pair in order.windows(2) {
            assert!(pair[0].urgency() > pair[1].urgency(), "{pair:?}");
        }
        assert_eq!(AgentStatus::most_urgent([Quiet, Idle, Active]), Active);
        assert_eq!(AgentStatus::most_urgent([Idle, Working, Waiting]), Waiting);
        assert_eq!(AgentStatus::most_urgent([]), None);
    }

    #[test]
    fn prompts_keep_their_first_line_within_the_limit() {
        assert_eq!(
            clip_prompt("  \n\nfix it\nplease").as_deref(),
            Some("fix it")
        );
        assert_eq!(clip_prompt("   \n "), None);
        let long = "é".repeat(300);
        assert_eq!(
            clip_prompt(&long).map(|p| p.chars().count()),
            Some(PROMPT_MAX)
        );
    }

    /// Sample payloads written from the hook docs of Claude Code and Codex (22 Nov 2026).
    fn map(
        kind: AgentKind,
        event: &str,
        notification: Option<&str>,
    ) -> Option<(AgentStatus, bool, bool)> {
        map_hook_event(kind, event, notification, Some("s1"), Some("fix it"))
            .map(|u| (u.status, u.session_id.is_some(), u.prompt.is_some()))
    }

    #[test]
    fn the_mapping_covers_every_row_of_the_table() {
        use AgentKind::{Claude, Codex};
        use AgentStatus::{Idle, None as Gone, Waiting, Working};
        let rows = [
            (Claude, "SessionStart", Option::None, (Idle, true, false)),
            (
                Claude,
                "UserPromptSubmit",
                Option::None,
                (Working, true, true),
            ),
            (Claude, "PostToolUse", Option::None, (Working, true, false)),
            (
                Claude,
                "PermissionRequest",
                Option::None,
                (Waiting, true, false),
            ),
            (
                Claude,
                "Notification",
                Some("permission_prompt"),
                (Waiting, true, false),
            ),
            (
                Claude,
                "Notification",
                Some("elicitation_dialog"),
                (Waiting, true, false),
            ),
            (
                Claude,
                "Notification",
                Some("idle_prompt"),
                (Idle, true, false),
            ),
            (Claude, "Stop", Option::None, (Idle, true, false)),
            (Claude, "SessionEnd", Option::None, (Gone, true, false)),
            (Codex, "SessionStart", Option::None, (Idle, true, false)),
            (
                Codex,
                "UserPromptSubmit",
                Option::None,
                (Working, true, true),
            ),
            (Codex, "PostToolUse", Option::None, (Working, true, false)),
            (
                Codex,
                "PermissionRequest",
                Option::None,
                (Waiting, true, false),
            ),
            (Codex, "Stop", Option::None, (Idle, true, false)),
            (Codex, "Interrupt", Option::None, (Idle, true, false)),
            (Codex, "SessionEnd", Option::None, (Gone, true, false)),
        ];
        for (kind, event, notification, expected) in rows {
            assert_eq!(map(kind, event, notification), Some(expected), "{event}");
        }
    }

    #[test]
    fn other_hook_events_are_ignored() {
        use AgentKind::{Claude, Codex, OpenCode};
        for (kind, event, notification) in [
            (Claude, "PreToolUse", None),
            (Claude, "Notification", Some("auth_success")),
            (Claude, "Notification", Some("elicitation_complete")),
            (Claude, "Notification", None),
            (Claude, "Interrupt", None),
            (Codex, "Notification", Some("idle_prompt")),
            (Codex, "PreToolUse", None),
            (OpenCode, "Stop", None),
        ] {
            assert_eq!(map(kind, event, notification), None, "{kind:?} {event}");
        }
    }

    #[test]
    fn a_missing_payload_field_is_not_recorded() {
        let update = map_hook_event(AgentKind::Claude, "UserPromptSubmit", None, None, None);
        assert_eq!(
            update,
            Some(HookUpdate {
                status: AgentStatus::Working,
                session_id: None,
                prompt: None
            })
        );
    }

    #[test]
    fn the_installed_events_all_map_to_something() {
        for kind in [AgentKind::Claude, AgentKind::Codex] {
            for event in hook_events(kind) {
                let notification = Some("permission_prompt");
                assert!(map(kind, event, notification).is_some(), "{event}");
            }
        }
    }

    #[test]
    fn names_round_trip() {
        for kind in AgentKind::ALL {
            assert_eq!(AgentKind::parse(kind.name()), Some(kind));
        }
        assert_eq!(AgentKind::parse("vim"), None);
    }

    fn resume(kind: AgentKind, id: Option<&str>) -> AgentResume {
        AgentResume {
            kind,
            session_id: id.map(str::to_string),
        }
    }

    #[test]
    fn session_ids_are_checked() {
        for ok in [
            "abc",
            "a",
            "ses_4f2a1c",
            "0a1b-2c3d",
            "a.b:c_d-e",
            &"a".repeat(128),
        ] {
            assert!(valid_session_id(ok), "{ok}");
        }
        for bad in [
            "",
            "-p",
            "--resume",
            ".x",
            "abc; rm -rf ~",
            "a b",
            "a$b",
            "a'b",
            "a\nb",
            "é",
            &"a".repeat(129),
        ] {
            assert!(!valid_session_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn templates_are_checked_and_rendered() {
        assert!(check_template("claude --resume {session_id}", true).is_ok());
        assert!(check_template("  codex   resume\t--last ", false).is_ok());
        assert_eq!(
            render_template("claude  --resume {session_id}", Some("abc")),
            ["claude", "--resume", "abc"]
        );
        for bad in [
            "",
            "   ",
            "claude 'x'",
            "claude \"x\"",
            "claude \\x",
            "claude $HOME",
            "claude `id`",
            "claude; id",
            "claude && id",
            "claude | id",
            "claude > f",
            "claude *",
            "claude --resume={session_id}",
            "claude {session_id}x",
            "{session_id} x",
        ] {
            assert!(check_template(bad, true).is_err(), "{bad:?}");
        }
        assert!(check_template("claude --resume {session_id}", false).is_err());
    }

    #[test]
    fn the_resume_command_uses_a_valid_id_else_the_latest() {
        let agents = AgentsConfig::default();
        let cmd = |kind, id| resume_command(&agents, &resume(kind, id));
        assert_eq!(
            cmd(AgentKind::Claude, Some("abc")),
            Some(vec!["claude".into(), "--resume".into(), "abc".into()])
        );
        assert_eq!(
            cmd(AgentKind::Claude, None),
            Some(vec!["claude".into(), "--continue".into()])
        );
        assert_eq!(
            cmd(AgentKind::Codex, Some("-p")),
            Some(vec!["codex".into(), "resume".into(), "--last".into()])
        );
        assert_eq!(
            cmd(AgentKind::OpenCode, Some("ses_1")),
            Some(vec!["opencode".into(), "--session".into(), "ses_1".into()])
        );
        assert_eq!(
            cmd(AgentKind::Claude, Some("abc; rm -rf ~")),
            Some(vec!["claude".into(), "--continue".into()])
        );
        let mut custom = AgentsConfig::default();
        custom.claude.resume = "claude; id".into();
        assert_eq!(
            resume_command(&custom, &resume(AgentKind::Claude, Some("abc"))),
            None
        );
    }
}
