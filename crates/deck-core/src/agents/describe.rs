//! The one-line description of what an agent does, from a hook payload. Pure: the CLI builds
//! the line from the payload it reads, so only the line (never the payload) reaches the daemon.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{AgentKind, AgentStatus};

/// The longest activity line kept, in characters.
pub const ACTIVITY_MAX: usize = 400;

/// What an event does to a terminal's activity line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Activity {
    /// The line stays as it is.
    Keep,
    /// The line is removed.
    Clear,
    /// The line becomes this text (cleaned and cut again by the daemon).
    Set(String),
}

/// An activity line from any text: its first non-blank line, without control characters,
/// trimmed and cut to [`ACTIVITY_MAX`] characters; `None` when nothing is left.
pub fn clean_activity(text: &str) -> Option<String> {
    let line = text.lines().find(|l| !l.trim().is_empty())?;
    let line: String = line.chars().filter(|c| !c.is_control()).collect();
    let line: String = line.trim().chars().take(ACTIVITY_MAX).collect();
    let line = line.trim_end();
    (!line.is_empty()).then(|| line.to_string())
}

/// A `Set` of `text` cleaned, or `Clear` when nothing is left of it.
fn set(text: &str) -> Activity {
    clean_activity(text).map_or(Activity::Clear, Activity::Set)
}

/// What a client shows for an agent without an activity line.
pub fn describe_status(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Working => "working",
        AgentStatus::Waiting => "needs you",
        AgentStatus::Idle => "ready",
        AgentStatus::Active => "printing",
        AgentStatus::Quiet => "quiet",
        AgentStatus::None => "",
    }
}

fn str_at<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
}

/// The last path component of a path.
pub(super) fn file_name(path: &str) -> &str {
    let path = path.trim().trim_end_matches('/');
    path.rsplit('/').next().unwrap_or(path)
}

/// The host of a URL: no scheme, user, port, path or query.
fn host(url: &str) -> &str {
    let rest = url.trim().split_once("://").map_or(url.trim(), |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    if authority.starts_with('[') {
        return authority
            .split_once(']')
            .map_or(authority, |(h, _)| &authority[..=h.len()]);
    }
    authority.split(':').next().unwrap_or(authority)
}

pub(super) fn first_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
}

/// The first file a Codex `apply_patch` patch names.
fn patch_file(input: &Value) -> Option<&str> {
    let patch = input
        .as_str()
        .or_else(|| str_at(input, "command"))
        .or_else(|| str_at(input, "input"))
        .or_else(|| str_at(input, "patch"))?;
    patch.lines().find_map(|line| {
        let line = line.trim();
        ["Add File:", "Update File:", "Delete File:"]
            .iter()
            .find_map(|p| line.strip_prefix("*** ").and_then(|l| l.strip_prefix(p)))
            .map(str::trim)
            .filter(|f| !f.is_empty())
    })
}

/// The line for a tool call, from `tool_name` and `tool_input`.
fn tool_line(name: Option<&str>, input: &Value) -> String {
    let Some(name) = name.filter(|n| !n.trim().is_empty()) else {
        return "using a tool".to_string();
    };
    let field = |key: &str| str_at(input, key);
    match name {
        "Bash" | "shell" | "exec_command" => {
            if let Some(description) = field("description") {
                let description = description.trim();
                let mut chars = description.chars();
                let first = chars.next().map(|c| c.to_lowercase().collect::<String>());
                return first.unwrap_or_default() + chars.as_str();
            }
            let command = field("command").or_else(|| field("cmd")).map(first_line);
            match command {
                Some(c) if !c.is_empty() => format!("running {c}"),
                _ => "running a command".to_string(),
            }
        }
        "Read" => match field("file_path") {
            Some(path) => format!("reading {}", file_name(path)),
            None => "reading a file".to_string(),
        },
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => {
            match field("file_path").or_else(|| field("notebook_path")) {
                Some(path) => format!("editing {}", file_name(path)),
                None => "editing a file".to_string(),
            }
        }
        "apply_patch" => match patch_file(input) {
            Some(path) => format!("editing {}", file_name(path)),
            None => "editing files".to_string(),
        },
        "Grep" | "Glob" => match field("pattern") {
            Some(pattern) => format!("searching {}", first_line(pattern)),
            None => "searching".to_string(),
        },
        "WebFetch" => match field("url").map(host).filter(|h| !h.is_empty()) {
            Some(host) => format!("fetching {host}"),
            None => "fetching a page".to_string(),
        },
        "WebSearch" => "searching the web".to_string(),
        "Agent" | "Task" => match (field("subagent_type"), field("description")) {
            (Some(kind), Some(what)) => format!("dispatching {kind}: {}", first_line(what)),
            (Some(one), None) | (None, Some(one)) => format!("dispatching {}", first_line(one)),
            (None, None) => "dispatching agent".to_string(),
        },
        "AskUserQuestion" => {
            let header = input
                .get("questions")
                .and_then(|q| q.get(0))
                .and_then(|q| str_at(q, "header"));
            match header {
                Some(h) => format!("asking: {}", first_line(h)),
                None => "asking a question".to_string(),
            }
        }
        "TodoWrite" => "planning".to_string(),
        "Skill" => match field("skill").or_else(|| field("name")) {
            Some(skill) => format!("using skill {}", first_line(skill)),
            None => "using a skill".to_string(),
        },
        other => match other.strip_prefix("mcp__") {
            Some(rest) => match rest.split_once("__") {
                Some((server, tool)) if !tool.is_empty() => format!("using {server} {tool}"),
                _ => format!("using {}", rest.trim_end_matches('_')),
            },
            None => format!("using {other}"),
        },
    }
}

/// The line for an OpenCode tool call: `name` is OpenCode's tool id (`bash`, `read`, …) and
/// `input` the reduced input (`description`, `command`, `file`, `agent`, `header`). The words are Claude Code's.
pub(super) fn opencode_tool_line(name: &str, input: &Value) -> String {
    let claude = match name {
        "bash" | "shell" => "Bash",
        "read" => "Read",
        "edit" | "write" => "Edit",
        "grep" => "Grep",
        "glob" => "Glob",
        "webfetch" => "WebFetch",
        "task" | "subagent" => "Task",
        "todowrite" => "TodoWrite",
        "question" => "AskUserQuestion",
        // A patch names its files in its text; only the first one was kept.
        "patch" => {
            return match str_at(input, "file") {
                Some(file) => format!("editing {}", file_name(file)),
                None => "editing files".to_string(),
            };
        }
        other => other,
    };
    let mut mapped = serde_json::Map::new();
    for (from, to) in [
        ("description", "description"),
        ("command", "command"),
        ("file", "file_path"),
        ("agent", "subagent_type"),
    ] {
        if let Some(v) = input.get(from) {
            mapped.insert(to.to_string(), v.clone());
        }
    }
    if let Some(header) = input.get("header") {
        mapped.insert("questions".to_string(), json!([{ "header": header }]));
    }
    tool_line(Some(claude), &Value::Object(mapped))
}

/// Removes the Markdown marks a heading, quote, list item, emphasis or code span leaves.
fn without_marks(line: &str) -> String {
    let line = line.trim_start_matches(['#', '>', ' ', '\t']);
    let line = line.strip_prefix("- ").unwrap_or(line);
    line.chars().filter(|c| !matches!(c, '*' | '`')).collect()
}

/// The first line of a message that has something left after its marks are removed.
pub(super) fn message_line(text: &str) -> Option<String> {
    text.lines()
        .map(|l| without_marks(l.trim()))
        .find_map(|l| clean_activity(&l))
}

/// What a hook event of Claude Code or Codex says the agent does: `payload` is the JSON the
/// hook received. A subagent's tool event (`agent_id` and `agent_type` in the payload) is
/// prefixed with its type. A missing field gives a plainer line, never an error.
pub fn describe_hook(kind: AgentKind, event: &str, payload: &Value) -> Activity {
    if kind == AgentKind::OpenCode {
        return Activity::Keep;
    }
    let tool = || {
        tool_line(
            str_at(payload, "tool_name"),
            payload.get("tool_input").unwrap_or(&Value::Null),
        )
    };
    match event {
        "SessionStart" | "SessionEnd" => Activity::Clear,
        "UserPromptSubmit" => Activity::Set("thinking".to_string()),
        "PreToolUse" => {
            let line = tool();
            match (str_at(payload, "agent_id"), str_at(payload, "agent_type")) {
                (Some(_), Some(agent)) => set(&format!("{}: {line}", first_line(agent))),
                _ => set(&line),
            }
        }
        "PermissionRequest" => match str_at(payload, "tool_name") {
            Some(_) => set(&format!("needs permission: {}", tool())),
            None => Activity::Set("needs permission".to_string()),
        },
        "Notification" => match str_at(payload, "notification_type") {
            Some("permission_prompt" | "elicitation_dialog") => {
                let message = str_at(payload, "message").and_then(clean_activity);
                Activity::Set(message.unwrap_or_else(|| "needs you".to_string()))
            }
            _ => Activity::Keep,
        },
        "Stop" => {
            let tasks = payload
                .get("background_tasks")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            if tasks > 0 {
                let s = if tasks == 1 { "" } else { "s" };
                return Activity::Set(format!("waiting for {tasks} background task{s}"));
            }
            let message = str_at(payload, "last_assistant_message").and_then(message_line);
            Activity::Set(message.unwrap_or_else(|| "done".to_string()))
        }
        "Interrupt" => Activity::Set("interrupted".to_string()),
        _ => Activity::Keep,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn pre(kind: AgentKind, tool: &str, input: Value) -> Activity {
        describe_hook(
            kind,
            "PreToolUse",
            &json!({"tool_name": tool, "tool_input": input}),
        )
    }

    fn line(a: Activity) -> String {
        match a {
            Activity::Set(s) => s,
            other => panic!("not a line: {other:?}"),
        }
    }

    #[test]
    fn the_tool_line_covers_every_row() {
        use AgentKind::{Claude, Codex};
        let cases = [
            (
                Claude,
                "Bash",
                json!({"command": "ls", "description": "Run the tests"}),
                "run the tests",
            ),
            (
                Claude,
                "Bash",
                json!({"command": "\ncargo test\nmore"}),
                "running cargo test",
            ),
            (
                Claude,
                "Bash",
                json!({"command": "ls", "description": "  "}),
                "running ls",
            ),
            (Claude, "Bash", json!({}), "running a command"),
            (
                Codex,
                "shell",
                json!({"command": "git status"}),
                "running git status",
            ),
            (
                Codex,
                "exec_command",
                json!({"cmd": "make"}),
                "running make",
            ),
            (
                Claude,
                "Read",
                json!({"file_path": "/a/b/sidebar.rs"}),
                "reading sidebar.rs",
            ),
            (Claude, "Read", json!({}), "reading a file"),
            (
                Claude,
                "Edit",
                json!({"file_path": "/a/b/main.rs"}),
                "editing main.rs",
            ),
            (
                Claude,
                "MultiEdit",
                json!({"file_path": "x.rs"}),
                "editing x.rs",
            ),
            (
                Claude,
                "Write",
                json!({"file_path": "/a/n.md"}),
                "editing n.md",
            ),
            (
                Claude,
                "NotebookEdit",
                json!({"notebook_path": "/a/n.ipynb"}),
                "editing n.ipynb",
            ),
            (Claude, "Write", json!({}), "editing a file"),
            (
                Codex,
                "apply_patch",
                json!({"command": "*** Begin Patch\n*** Update File: src/lib/a.rs\n@@\n*** End Patch"}),
                "editing a.rs",
            ),
            (
                Codex,
                "apply_patch",
                json!({"command": "nothing"}),
                "editing files",
            ),
            (
                Claude,
                "Grep",
                json!({"pattern": "fn main"}),
                "searching fn main",
            ),
            (
                Claude,
                "Glob",
                json!({"pattern": "**/*.rs"}),
                "searching **/*.rs",
            ),
            (
                Claude,
                "WebFetch",
                json!({"url": "https://user:pw@example.com:8080/a/b?q=secret#x"}),
                "fetching example.com",
            ),
            (Claude, "WebFetch", json!({}), "fetching a page"),
            (
                Claude,
                "WebSearch",
                json!({"query": "secret"}),
                "searching the web",
            ),
            (
                Claude,
                "Agent",
                json!({"subagent_type": "worker", "description": "implement it"}),
                "dispatching worker: implement it",
            ),
            (
                Claude,
                "Task",
                json!({"subagent_type": "worker"}),
                "dispatching worker",
            ),
            (Claude, "Task", json!({}), "dispatching agent"),
            (
                Claude,
                "AskUserQuestion",
                json!({"questions": [{"header": "Repo flow", "question": "x"}]}),
                "asking: Repo flow",
            ),
            (Claude, "AskUserQuestion", json!({}), "asking a question"),
            (Claude, "TodoWrite", json!({"todos": []}), "planning"),
            (
                Claude,
                "Skill",
                json!({"skill": "commit"}),
                "using skill commit",
            ),
            (
                Claude,
                "mcp__github__list_prs",
                json!({}),
                "using github list_prs",
            ),
            (Claude, "mcp__a__b__c", json!({}), "using a b__c"),
            (Claude, "Frobnicate", json!({}), "using Frobnicate"),
        ];
        for (kind, tool, input, want) in cases {
            assert_eq!(line(pre(kind, tool, input)), want, "{tool}");
        }
        assert_eq!(
            line(describe_hook(Claude, "PreToolUse", &json!({}))),
            "using a tool"
        );
    }

    #[test]
    fn a_subagent_event_is_prefixed_with_its_type() {
        let payload = json!({
            "agent_id": "a1", "agent_type": "worker",
            "tool_name": "Read", "tool_input": {"file_path": "/x/sidebar.rs"},
        });
        assert_eq!(
            line(describe_hook(AgentKind::Claude, "PreToolUse", &payload)),
            "worker: reading sidebar.rs"
        );
        assert_eq!(
            line(describe_hook(AgentKind::Codex, "PreToolUse", &payload)),
            "worker: reading sidebar.rs"
        );
        // `--agent` names the main thread's type without an `agent_id`: not a subagent.
        let main = json!({"agent_type": "worker", "tool_name": "TodoWrite"});
        assert_eq!(
            line(describe_hook(AgentKind::Claude, "PreToolUse", &main)),
            "planning"
        );
        let null_id = json!({"agent_id": null, "agent_type": null, "tool_name": "TodoWrite"});
        assert_eq!(
            line(describe_hook(AgentKind::Codex, "PreToolUse", &null_id)),
            "planning"
        );
    }

    #[test]
    fn the_events_cover_every_row() {
        use AgentKind::{Claude, Codex};
        let none = json!({});
        for kind in [Claude, Codex] {
            assert_eq!(describe_hook(kind, "SessionStart", &none), Activity::Clear);
            assert_eq!(describe_hook(kind, "SessionEnd", &none), Activity::Clear);
            assert_eq!(describe_hook(kind, "PostToolUse", &none), Activity::Keep);
            assert_eq!(describe_hook(kind, "Whatever", &none), Activity::Keep);
            assert_eq!(
                line(describe_hook(kind, "UserPromptSubmit", &none)),
                "thinking"
            );
            assert_eq!(line(describe_hook(kind, "Stop", &none)), "done");
        }
        assert_eq!(
            line(describe_hook(Codex, "Interrupt", &none)),
            "interrupted"
        );
        let perm = json!({"tool_name": "Bash", "tool_input": {"command": "rm x"}});
        assert_eq!(
            line(describe_hook(Claude, "PermissionRequest", &perm)),
            "needs permission: running rm x"
        );
        assert_eq!(
            line(describe_hook(Claude, "PermissionRequest", &none)),
            "needs permission"
        );
        for t in ["permission_prompt", "elicitation_dialog"] {
            let n =
                json!({"notification_type": t, "message": "Claude needs your permission\nmore"});
            assert_eq!(
                line(describe_hook(Claude, "Notification", &n)),
                "Claude needs your permission"
            );
            let n = json!({"notification_type": t});
            assert_eq!(line(describe_hook(Claude, "Notification", &n)), "needs you");
        }
        let idle = json!({"notification_type": "idle_prompt", "message": "m"});
        assert_eq!(describe_hook(Claude, "Notification", &idle), Activity::Keep);
        assert_eq!(
            describe_hook(AgentKind::OpenCode, "Stop", &none),
            Activity::Keep
        );
    }

    #[test]
    fn stop_describes_background_work_or_the_last_message() {
        let stop = |p: Value| line(describe_hook(AgentKind::Claude, "Stop", &p));
        assert_eq!(
            stop(json!({"background_tasks": [{"id": 1}], "last_assistant_message": "x"})),
            "waiting for 1 background task"
        );
        assert_eq!(
            stop(json!({"background_tasks": [{}, {}]})),
            "waiting for 2 background tasks"
        );
        assert_eq!(
            stop(
                json!({"background_tasks": [], "last_assistant_message": "\n\n## **Done** the `thing`\nsecond"})
            ),
            "Done the thing"
        );
        assert_eq!(
            stop(json!({"last_assistant_message": "> - quoted"})),
            "quoted"
        );
        assert_eq!(stop(json!({"last_assistant_message": "- item"})), "item");
        assert_eq!(stop(json!({"last_assistant_message": "  \n "})), "done");
        assert_eq!(stop(json!({"last_assistant_message": null})), "done");
    }

    #[test]
    fn a_line_is_clean_short_and_on_one_line() {
        assert_eq!(
            clean_activity("  a\u{1b}[1mb\tc \nsecond"),
            Some("a[1mbc".into())
        );
        assert_eq!(clean_activity("\n\n  \n x "), Some("x".into()));
        assert_eq!(clean_activity(" \u{7}\n"), None);
        assert_eq!(clean_activity(""), None);
        let long = "é".repeat(600);
        assert_eq!(
            clean_activity(&long).map(|l| l.chars().count()),
            Some(ACTIVITY_MAX)
        );
        let big = describe_hook(
            AgentKind::Claude,
            "PreToolUse",
            &json!({"tool_name": "Bash", "tool_input": {"command": "x".repeat(1000)}}),
        );
        assert_eq!(line(big).chars().count(), ACTIVITY_MAX);
    }

    #[test]
    fn statuses_have_words() {
        use AgentStatus::{Active, Idle, None, Quiet, Waiting, Working};
        let words: Vec<_> = [Working, Waiting, Idle, Active, Quiet, None]
            .map(describe_status)
            .into();
        assert_eq!(
            words,
            ["working", "needs you", "ready", "printing", "quiet", ""]
        );
    }
}
