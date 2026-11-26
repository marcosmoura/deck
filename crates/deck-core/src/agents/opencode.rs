//! The state machine of one terminal's OpenCode: fed the raw events the deck plugin forwards,
//! it drops subagent sessions and tells what the status, session and first prompt are.
//! Pure: no I/O, one value per terminal, bounded memory.
//!
//! Events are the `{ "type", "data" }` objects of OpenCode v2's event stream; the
//! `{ "type", "properties" }` objects of v1's plugin `event` hook are read too.

use std::collections::VecDeque;

use serde_json::{Value, json};

use super::describe::{file_name, first_line, message_line, opencode_tool_line};
use super::{Activity, AgentStatus, clean_activity, clip_prompt};

/// The events the plugin subscribes to and the tracker understands.
///
/// The plugin subscribes to the v2 ones; the tracker also reads the v1 `session.updated`,
/// `message.updated` and `message.part.updated`, for a v1 plugin that is still installed.
pub const OPENCODE_EVENTS: [&str; 16] = [
    "session.created",
    "session.status",
    "session.idle",
    "session.execution.started",
    "session.execution.succeeded",
    "session.execution.interrupted",
    "session.execution.failed",
    "form.created",
    "form.replied",
    "form.cancelled",
    "permission.asked",
    "permission.replied",
    "session.inbox.enqueued",
    "session.tool.input.started",
    "session.tool.called",
    "session.text.ended",
];

/// At most this many subagent session ids are remembered.
pub const MAX_CHILDREN: usize = 256;
/// At most this many user message ids are remembered.
pub const MAX_USER_MESSAGES: usize = 64;

/// What an event tells deck; every field is optional.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCodeUpdate {
    pub status: Option<AgentStatus>,
    pub session_id: Option<String>,
    pub prompt: Option<String>,
    /// What the event does to the activity line.
    pub activity: Activity,
}

#[derive(Debug, Default)]
pub struct OpenCodeTracker {
    children: VecDeque<String>,
    user_messages: VecDeque<String>,
    /// Tool call id and its line, for a permission that names the call.
    calls: VecDeque<(String, String)>,
    /// The line of the call the last permission prompt named, when it was a known one.
    asked_line: Option<String>,
    /// Whether a tool ran since the turn began.
    turn_tool: bool,
    /// The first line of the turn's last assistant text.
    last_text: Option<String>,
}

/// At most this many tool calls are remembered.
pub const MAX_CALLS: usize = 64;
/// The longest command, description or text a reduced event keeps, in characters.
const REDUCED_MAX: usize = 120;

/// A longer id is not remembered: OpenCode's are a few dozen bytes, and the lists would
/// otherwise hold whatever a client of the daemon sends.
pub const MAX_ID_LEN: usize = 128;

fn remember(list: &mut VecDeque<String>, cap: usize, id: &str) {
    if id.len() > MAX_ID_LEN || list.iter().any(|known| known == id) {
        return;
    }
    if list.len() >= cap {
        list.pop_front();
    }
    list.push_back(id.to_string());
}

fn text<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut at = value;
    for key in path {
        at = at.get(key)?;
    }
    at.as_str().filter(|s| !s.is_empty())
}

/// The one line of `text` a reduced event keeps, cut to `max` characters.
fn one_line(text: &str, max: usize) -> Option<String> {
    let line = first_line(text);
    let line: String = line.chars().filter(|c| !c.is_control()).take(max).collect();
    (!line.trim().is_empty()).then_some(line)
}

/// The first file a patch (`*** Add File: <path>`, `Update`, `Delete`) names.
fn patch_file(patch: &str) -> Option<&str> {
    patch.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("*** ")?;
        ["Add File:", "Update File:", "Delete File:"]
            .iter()
            .find_map(|p| rest.strip_prefix(p))
            .map(str::trim)
            .filter(|f| !f.is_empty())
    })
}

/// A tool's input cut down to what its line is made of: a description, the first
/// line of a command, a file name, an agent type and a question's header. Nothing else of it (a file's content, an edit, a
/// pattern, a URL) is kept.
fn reduce_input(input: &Value) -> Value {
    let field = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| input.get(k).and_then(Value::as_str))
    };
    let mut out = serde_json::Map::new();
    if let Some(d) = field(&["description"]).and_then(|d| one_line(d, REDUCED_MAX)) {
        out.insert("description".into(), json!(d));
    }
    if let Some(c) = field(&["command"]).and_then(|c| one_line(c, REDUCED_MAX)) {
        out.insert("command".into(), json!(c));
    }
    // A patch names its files in its text: only the first one is kept.
    let file = field(&["file", "filePath", "file_path", "path"])
        .or_else(|| field(&["patchText"]).and_then(patch_file));
    if let Some(f) = file.map(file_name).and_then(|f| one_line(f, REDUCED_MAX)) {
        out.insert("file".into(), json!(f));
    }
    if let Some(a) = field(&["agent", "subagent_type"]).and_then(|a| one_line(a, REDUCED_MAX)) {
        out.insert("agent".into(), json!(a));
    }
    let header = input
        .pointer("/questions/0/header")
        .and_then(Value::as_str)
        .or_else(|| field(&["header"]));
    if let Some(h) = header.and_then(|h| one_line(h, REDUCED_MAX)) {
        out.insert("header".into(), json!(h));
    }
    Value::Object(out)
}

/// The event as the daemon needs it, before it leaves `deck agent-event`: the type, the
/// session ids and the status fields stay as they are; a tool event keeps only the tool's
/// name and its reduced input ([`reduce_input`]), a text event only the first line of its text
/// ([`ACTIVITY_MAX`](super::ACTIVITY_MAX) characters; 200 for a v1 part, which can be the prompt). Other events pass unchanged.
/// Handles v2's `{ "type", "data" }` and v1's `{ "type", "properties" }`.
pub fn reduce_opencode_event(doc: &Value) -> Value {
    let Some(kind) = text(doc, &["type"]) else {
        return doc.clone();
    };
    let key = if doc.get("data").is_some() {
        "data"
    } else {
        "properties"
    };
    let Some(props) = doc.get(key) else {
        return doc.clone();
    };
    let id = |value: &Value, name: &str| value.get(name).filter(|v| v.is_string()).cloned();
    let mut out = serde_json::Map::new();
    let put = |value: &Value, names: &[&str], into: &mut serde_json::Map<String, Value>| {
        for name in names {
            if let Some(v) = id(value, name) {
                into.insert((*name).to_string(), v);
            }
        }
    };
    match kind {
        "session.tool.called" => {
            put(props, &["sessionID", "id", "name"], &mut out);
            out.insert(
                "input".into(),
                reduce_input(props.get("input").unwrap_or(&Value::Null)),
            );
        }
        "deck.text" | "session.text.ended" => {
            put(props, &["sessionID"], &mut out);
            if let Some(t) = text(props, &["text"]).and_then(clean_activity) {
                out.insert("text".into(), json!(t));
            }
        }
        "permission.asked" => {
            put(
                props,
                &["sessionID", "id", "action", "permission"],
                &mut out,
            );
            if let Some(source) = props.get("source") {
                let mut kept = serde_json::Map::new();
                put(source, &["type", "messageID", "id"], &mut kept);
                out.insert("source".into(), Value::Object(kept));
            }
            if let Some(tool) = props.get("tool") {
                let mut kept = serde_json::Map::new();
                put(tool, &["messageID", "callID"], &mut kept);
                out.insert("tool".into(), Value::Object(kept));
            }
            let resource = props
                .pointer("/resources/0")
                .and_then(Value::as_str)
                .and_then(clean_activity);
            if let Some(r) = resource {
                out.insert("resources".into(), json!([r]));
            }
        }
        "form.created" => {
            let form = props.get("form").unwrap_or(&Value::Null);
            let mut kept = serde_json::Map::new();
            put(form, &["id", "sessionID"], &mut kept);
            let title = text(form, &["title"])
                .or_else(|| form.pointer("/fields/0/title").and_then(Value::as_str))
                .and_then(clean_activity);
            if let Some(t) = title {
                kept.insert("title".into(), json!(t));
            }
            out.insert("form".into(), Value::Object(kept));
        }
        "session.inbox.enqueued" => {
            put(props, &["sessionID", "inboxID"], &mut out);
            let item = props.get("item").unwrap_or(&Value::Null);
            let mut kept = serde_json::Map::new();
            put(item, &["type"], &mut kept);
            if text(item, &["type"]) == Some("user")
                && let Some(t) = text(item, &["payload", "text"]).and_then(clip_prompt)
            {
                kept.insert("payload".into(), json!({ "text": t }));
            }
            out.insert("item".into(), Value::Object(kept));
        }
        "message.part.updated" => {
            let part = props.get("part").unwrap_or(&Value::Null);
            let mut kept = serde_json::Map::new();
            put(part, &["id", "sessionID", "messageID", "type"], &mut kept);
            match text(part, &["type"]) {
                Some("tool") => {
                    put(part, &["callID", "tool"], &mut kept);
                    let state = part.get("state").unwrap_or(&Value::Null);
                    let mut s = serde_json::Map::new();
                    put(state, &["status"], &mut s);
                    s.insert(
                        "input".into(),
                        reduce_input(state.get("input").unwrap_or(&Value::Null)),
                    );
                    kept.insert("state".into(), Value::Object(s));
                }
                Some("text") => {
                    if let Some(t) = text(part, &["text"]).and_then(clip_prompt) {
                        kept.insert("text".into(), json!(t));
                    }
                }
                _ => return doc.clone(),
            }
            out.insert("part".into(), Value::Object(kept));
        }
        _ => return doc.clone(),
    }
    json!({ "type": kind, key: Value::Object(out) })
}

/// A `Set` of `line` cleaned, or `Keep` when nothing is left of it.
fn line_activity(line: &str) -> Activity {
    clean_activity(line).map_or(Activity::Keep, Activity::Set)
}

impl OpenCodeTracker {
    fn remember_call(&mut self, id: &str, line: &str) {
        if id.len() > MAX_ID_LEN {
            return;
        }
        if self.calls.len() >= MAX_CALLS {
            self.calls.pop_front();
        }
        self.calls.push_back((id.to_string(), line.to_string()));
    }

    /// A tool call began: the line shows it, and the turn has run a tool.
    fn tool_started(&mut self, call: Option<&str>, line: &str) -> Option<OpenCodeUpdate> {
        let activity = line_activity(line);
        if let (Some(call), Activity::Set(line)) = (call, &activity) {
            self.remember_call(call, line);
        }
        self.turn_tool = true;
        Some(OpenCodeUpdate {
            status: None,
            session_id: None,
            prompt: None,
            activity,
        })
    }

    /// A turn started: `thinking`, unless a tool of the turn already gave the line.
    fn turn_start(&mut self) -> Activity {
        if self.turn_tool {
            Activity::Keep
        } else {
            self.last_text = None;
            Activity::Set("thinking".to_string())
        }
    }

    /// The turn ended: the line is its last text, else `done`.
    fn turn_end(&mut self) -> Activity {
        self.turn_tool = false;
        Activity::Set(self.last_text.clone().unwrap_or_else(|| "done".to_string()))
    }

    fn call_line(&self, id: &str) -> Option<&str> {
        self.calls
            .iter()
            .find(|(call, _)| call == id)
            .map(|(_, line)| line.as_str())
    }

    /// Whether `id` is a subagent session.
    fn is_child(&self, id: &str) -> bool {
        self.children.iter().any(|c| c == id)
    }

    /// Feeds one event (parsed by the caller, outside any lock); returns what to apply, or
    /// `None` when it changes nothing.
    pub fn feed(&mut self, doc: &Value) -> Option<OpenCodeUpdate> {
        let kind = text(doc, &["type"])?;
        let props = doc.get("data").or_else(|| doc.get("properties"))?;
        if matches!(kind, "session.created" | "session.updated") {
            // v2 puts the session on the payload, v1 under `info`.
            let (id, parent) = if props.get("info").is_some() {
                (
                    text(props, &["info", "id"]),
                    text(props, &["info", "parentID"]),
                )
            } else {
                (text(props, &["sessionID"]), text(props, &["parentID"]))
            };
            if let (Some(id), Some(_)) = (id, parent) {
                remember(&mut self.children, MAX_CHILDREN, id);
            }
            return None;
        }
        let session = text(props, &["sessionID"])
            .or_else(|| text(props, &["form", "sessionID"]))
            .or_else(|| text(props, &["info", "sessionID"]))
            .or_else(|| text(props, &["part", "sessionID"]));
        let child = session.is_some_and(|s| self.is_child(s));
        // A subagent's events are dropped, except that its permission prompts still need the
        // user. They carry no session id: the root's resumable one is kept.
        if child
            && !matches!(
                kind,
                "permission.asked"
                    | "permission.replied"
                    | "form.created"
                    | "form.replied"
                    | "form.cancelled"
            )
        {
            return None;
        }
        let status = |status, activity| {
            Some(OpenCodeUpdate {
                status: Some(status),
                session_id: session.filter(|_| !child).map(str::to_string),
                prompt: None,
                activity,
            })
        };
        match kind {
            // v2.4 marks a turn with `session.execution.*`; earlier v2 with `session.status`.
            "session.status" => match text(props, &["status", "type"])? {
                "busy" => status(AgentStatus::Working, self.turn_start()),
                "idle" => status(AgentStatus::Idle, self.turn_end()),
                _ => None,
            },
            "session.execution.started" => status(AgentStatus::Working, self.turn_start()),
            "session.idle" | "session.execution.succeeded" => {
                status(AgentStatus::Idle, self.turn_end())
            }
            "session.execution.interrupted" | "session.execution.failed" => {
                self.turn_end();
                let word = kind.trim_start_matches("session.execution.");
                status(AgentStatus::Idle, Activity::Set(word.to_string()))
            }
            // A question for the user: a form, by its title or its first field's.
            "form.created" => {
                let title = text(props, &["form", "title"])
                    .or_else(|| {
                        props
                            .pointer("/form/fields/0/title")
                            .and_then(Value::as_str)
                    })
                    .and_then(clean_activity);
                let activity = match title {
                    Some(t) => line_activity(&format!("asking: {t}")),
                    None => Activity::Set("asking a question".to_string()),
                };
                status(AgentStatus::Waiting, activity)
            }
            "form.replied" | "form.cancelled" => status(AgentStatus::Working, Activity::Clear),
            "permission.asked" => {
                // v2 names the tool in `action` and the call in `source.id`; v1 in `permission`
                // and `tool.callID`.
                let call =
                    text(props, &["source", "id"]).or_else(|| text(props, &["tool", "callID"]));
                let tool = call.and_then(|c| self.call_line(c)).map(str::to_string);
                self.asked_line = tool.clone();
                let tool = tool.or_else(|| {
                    let name = text(props, &["action"]).or_else(|| text(props, &["permission"]))?;
                    Some(opencode_tool_line(name, &Value::Null))
                });
                let activity = match tool {
                    Some(line) => line_activity(&format!("needs permission: {line}")),
                    None => Activity::Set("needs permission".to_string()),
                };
                status(AgentStatus::Waiting, activity)
            }
            "permission.replied" => {
                // A granted call runs: its line is back. A rejected or unknown one clears it.
                let granted = matches!(text(props, &["reply"]), Some("once" | "always"));
                let line = self.asked_line.take().filter(|_| granted);
                status(
                    AgentStatus::Working,
                    line.map_or(Activity::Clear, Activity::Set),
                )
            }
            "session.tool.called" => {
                let line = opencode_tool_line(
                    text(props, &["name"]).unwrap_or(""),
                    &reduce_input(props.get("input").unwrap_or(&Value::Null)),
                );
                self.tool_started(text(props, &["id"]), &line)
            }
            "deck.text" | "session.text.ended" => {
                self.last_text = text(props, &["text"]).and_then(message_line);
                None
            }
            "message.updated" => {
                if text(props, &["info", "role"]) == Some("user")
                    && let Some(id) = text(props, &["info", "id"])
                {
                    remember(&mut self.user_messages, MAX_USER_MESSAGES, id);
                }
                None
            }
            // v2: the prompt arrives whole, in the message a user queued.
            "session.inbox.enqueued" => {
                if text(props, &["item", "type"]) != Some("user") {
                    return None;
                }
                let prompt = clip_prompt(text(props, &["item", "payload", "text"])?)?;
                Some(OpenCodeUpdate {
                    status: None,
                    session_id: session.map(str::to_string),
                    prompt: Some(prompt),
                    activity: Activity::Keep,
                })
            }
            "message.part.updated" => {
                if text(props, &["part", "type"]) == Some("tool") {
                    if text(props, &["part", "state", "status"]) != Some("running") {
                        return None;
                    }
                    let call = text(props, &["part", "callID"]);
                    if call.is_some_and(|c| self.call_line(c).is_some()) {
                        return None;
                    }
                    let line = opencode_tool_line(
                        text(props, &["part", "tool"]).unwrap_or(""),
                        &reduce_input(props.pointer("/part/state/input").unwrap_or(&Value::Null)),
                    );
                    return self.tool_started(call, &line);
                }
                if text(props, &["part", "type"]) != Some("text") {
                    return None;
                }
                let message = text(props, &["part", "messageID"])?;
                let at = self.user_messages.iter().position(|m| m == message)?;
                let prompt = clip_prompt(text(props, &["part", "text"])?)?;
                self.user_messages.remove(at);
                Some(OpenCodeUpdate {
                    status: None,
                    session_id: session.map(str::to_string),
                    prompt: Some(prompt),
                    activity: Activity::Keep,
                })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    trait FeedRaw {
        fn feed_raw(&mut self, raw: &str) -> Option<OpenCodeUpdate>;
    }

    impl FeedRaw for OpenCodeTracker {
        fn feed_raw(&mut self, raw: &str) -> Option<OpenCodeUpdate> {
            self.feed(&serde_json::from_str(raw).ok()?)
        }
    }

    /// Samples shaped like the events in the OpenCode docs and its SDK types (`session.status`
    /// carries `sessionID` and `status`, `session.updated` an `info` with `parentID`,
    /// `message.updated` an `info` with `role`, `message.part.updated` a `part`).
    fn status(session: &str, kind: &str) -> String {
        format!(
            r#"{{"type":"session.status","properties":{{"sessionID":"{session}","status":{{"type":"{kind}"}}}}}}"#
        )
    }

    fn child(id: &str) -> String {
        format!(
            r#"{{"type":"session.created","properties":{{"info":{{"id":"{id}","parentID":"root","title":"sub"}}}}}}"#
        )
    }

    fn update_with(
        status: Option<AgentStatus>,
        session: Option<&str>,
        activity: Activity,
    ) -> Option<OpenCodeUpdate> {
        Some(OpenCodeUpdate {
            status,
            session_id: session.map(str::to_string),
            prompt: None,
            activity,
        })
    }

    /// The update of a status event with no text or tool before it: `busy` is `thinking`, a
    /// reply clears the line.
    fn update(status: Option<AgentStatus>, session: Option<&str>) -> Option<OpenCodeUpdate> {
        let activity = match status {
            Some(AgentStatus::Idle) => Activity::Set("done".into()),
            Some(AgentStatus::Waiting) => Activity::Set("needs permission".into()),
            Some(AgentStatus::Working) => Activity::Set("thinking".into()),
            _ => Activity::Keep,
        };
        update_with(status, session, activity)
    }

    #[test]
    fn an_oversized_id_is_not_remembered() {
        let mut t = OpenCodeTracker::default();
        t.feed_raw(&child(&"c".repeat(MAX_ID_LEN + 1)));
        let long = "m".repeat(MAX_ID_LEN + 1);
        t.feed_raw(&format!(
            r#"{{"type":"message.updated","properties":{{"info":{{"id":"{long}","role":"user","sessionID":"s1"}}}}}}"#
        ));
        assert!(t.children.is_empty() && t.user_messages.is_empty());
        t.feed_raw(&child(&"c".repeat(MAX_ID_LEN)));
        assert_eq!(t.children.len(), 1);
    }

    #[test]
    fn busy_and_idle_come_from_session_status() {
        let mut t = OpenCodeTracker::default();
        assert_eq!(
            t.feed_raw(&status("s1", "busy")),
            update(Some(AgentStatus::Working), Some("s1"))
        );
        assert_eq!(
            t.feed_raw(&status("s1", "idle")),
            update(Some(AgentStatus::Idle), Some("s1"))
        );
        assert_eq!(t.feed_raw(&status("s1", "retry")), None);
    }

    #[test]
    fn the_deprecated_session_idle_still_means_idle() {
        let mut t = OpenCodeTracker::default();
        let raw = r#"{"type":"session.idle","properties":{"sessionID":"s1"}}"#;
        assert_eq!(t.feed_raw(raw), update(Some(AgentStatus::Idle), Some("s1")));
    }

    #[test]
    fn permissions_wait_and_resume() {
        let mut t = OpenCodeTracker::default();
        let asked = r#"{"type":"permission.asked","properties":{"id":"p1","sessionID":"s1","permission":"bash"}}"#;
        let replied = r#"{"type":"permission.replied","properties":{"sessionID":"s1","requestID":"p1","reply":"once"}}"#;
        assert_eq!(
            t.feed_raw(asked),
            update_with(
                Some(AgentStatus::Waiting),
                Some("s1"),
                Activity::Set("needs permission: running a command".into())
            )
        );
        assert_eq!(
            t.feed_raw(replied),
            update_with(Some(AgentStatus::Working), Some("s1"), Activity::Clear)
        );
    }

    #[test]
    fn subagent_sessions_are_dropped() {
        let mut t = OpenCodeTracker::default();
        assert_eq!(t.feed_raw(&child("sub1")), None);
        let updated =
            r#"{"type":"session.updated","properties":{"info":{"id":"sub2","parentID":"root"}}}"#;
        assert_eq!(t.feed_raw(updated), None);
        for id in ["sub1", "sub2"] {
            assert_eq!(t.feed_raw(&status(id, "idle")), None, "{id}");
        }
        // A subagent's permission prompts still need the user, and keep the root's session id.
        let asked = r#"{"type":"permission.asked","properties":{"sessionID":"sub1"}}"#;
        assert_eq!(t.feed_raw(asked), update(Some(AgentStatus::Waiting), None));
        let replied =
            r#"{"type":"permission.replied","properties":{"sessionID":"sub1","reply":"once"}}"#;
        assert_eq!(
            t.feed_raw(replied),
            update_with(Some(AgentStatus::Working), None, Activity::Clear)
        );
        // The root session is unaffected.
        assert_eq!(
            t.feed_raw(&status("root", "idle")),
            update(Some(AgentStatus::Idle), Some("root"))
        );
        // A root session's own created event records nothing.
        let root = r#"{"type":"session.created","properties":{"info":{"id":"root2"}}}"#;
        assert_eq!(t.feed_raw(root), None);
        assert!(t.feed_raw(&status("root2", "busy")).is_some());
    }

    #[test]
    fn the_first_user_prompt_is_found_and_assistant_messages_are_ignored() {
        let mut t = OpenCodeTracker::default();
        let msg = |id: &str, role: &str| {
            format!(
                r#"{{"type":"message.updated","properties":{{"info":{{"id":"{id}","sessionID":"s1","role":"{role}"}}}}}}"#
            )
        };
        let part = |message: &str, kind: &str, text: &str| {
            format!(
                r#"{{"type":"message.part.updated","properties":{{"part":{{"id":"prt","sessionID":"s1","messageID":"{message}","type":"{kind}","text":"{text}"}}}}}}"#
            )
        };
        // A part before its message is known, and one of an assistant message, give nothing.
        assert_eq!(t.feed_raw(&part("m1", "text", "early")), None);
        assert_eq!(t.feed_raw(&msg("m1", "user")), None);
        assert_eq!(t.feed_raw(&msg("m2", "assistant")), None);
        assert_eq!(t.feed_raw(&part("m2", "text", "I will")), None);
        assert_eq!(t.feed_raw(&part("m1", "reasoning", "hm")), None);
        assert_eq!(
            t.feed_raw(&part("m1", "text", "fix the bug\\nplease")),
            Some(OpenCodeUpdate {
                status: None,
                session_id: Some("s1".into()),
                prompt: Some("fix the bug".into()),
                activity: Activity::Keep,
            })
        );
        // Only once per message.
        assert_eq!(t.feed_raw(&part("m1", "text", "fix the bug now")), None);
    }

    /// The v2 shapes (checked against OpenCode's `packages/schema`): the payload is `data`,
    /// `session.created` carries `sessionID` and `parentID`, a prompt is a `session.inbox.enqueued`.
    #[test]
    fn v2_events_give_status_permissions_prompt_and_subagents() {
        let mut t = OpenCodeTracker::default();
        let v2 =
            |kind: &str, data: &str| format!(r#"{{"id":"evt","type":"{kind}","data":{data}}}"#);
        assert_eq!(
            t.feed_raw(&v2(
                "session.status",
                r#"{"sessionID":"s1","status":{"type":"busy"}}"#
            )),
            update(Some(AgentStatus::Working), Some("s1"))
        );
        assert_eq!(
            t.feed_raw(&v2(
                "session.status",
                r#"{"sessionID":"s1","status":{"type":"idle"}}"#
            )),
            update(Some(AgentStatus::Idle), Some("s1"))
        );
        assert_eq!(
            t.feed_raw(&v2("session.idle", r#"{"sessionID":"s1"}"#)),
            update(Some(AgentStatus::Idle), Some("s1"))
        );
        assert_eq!(
            t.feed_raw(&v2(
                "permission.asked",
                r#"{"id":"per_1","sessionID":"s1","action":"bash","resources":["ls"]}"#
            )),
            update_with(
                Some(AgentStatus::Waiting),
                Some("s1"),
                Activity::Set("needs permission: running a command".into())
            )
        );
        assert_eq!(
            t.feed_raw(&v2(
                "permission.replied",
                r#"{"sessionID":"s1","requestID":"per_1","reply":"once"}"#
            )),
            update_with(Some(AgentStatus::Working), Some("s1"), Activity::Clear)
        );
        let inbox = |session: &str, kind: &str| {
            v2(
                "session.inbox.enqueued",
                &format!(
                    r#"{{"sessionID":"{session}","inboxID":"msg_1","item":{{"type":"{kind}","payload":{{"text":"fix the bug\nplease"}},"delivery":"steer"}}}}"#
                ),
            )
        };
        assert_eq!(
            t.feed_raw(&inbox("s1", "user")),
            Some(OpenCodeUpdate {
                status: None,
                session_id: Some("s1".into()),
                prompt: Some("fix the bug".into()),
                activity: Activity::Keep,
            })
        );
        assert_eq!(t.feed_raw(&inbox("s1", "synthetic")), None);
        // A subagent session: recorded by its created event, then dropped, except permissions.
        assert_eq!(
            t.feed_raw(&v2(
                "session.created",
                r#"{"sessionID":"sub1","parentID":"s1","slug":"x","version":"2"}"#
            )),
            None
        );
        assert_eq!(
            t.feed_raw(&v2(
                "session.created",
                r#"{"sessionID":"root2","slug":"x"}"#
            )),
            None
        );
        assert_eq!(
            t.feed_raw(&v2(
                "session.status",
                r#"{"sessionID":"sub1","status":{"type":"idle"}}"#
            )),
            None
        );
        assert_eq!(t.feed_raw(&inbox("sub1", "user")), None);
        assert_eq!(
            t.feed_raw(&v2("permission.asked", r#"{"sessionID":"sub1"}"#)),
            update(Some(AgentStatus::Waiting), None)
        );
        assert!(
            t.feed_raw(&v2(
                "session.status",
                r#"{"sessionID":"root2","status":{"type":"busy"}}"#
            ))
            .is_some()
        );
        assert_eq!(
            t.feed_raw(&v2(
                "session.status",
                r#"{"sessionID":"s1","status":{"type":"retry"}}"#
            )),
            None
        );
    }

    #[test]
    fn unknown_and_malformed_events_are_ignored() {
        let mut t = OpenCodeTracker::default();
        for raw in [
            "",
            "not json",
            "[]",
            r#"{"type":"file.edited","properties":{"file":"a"}}"#,
            r#"{"type":"session.status"}"#,
            r#"{"type":"session.status","properties":{"sessionID":"s"}}"#,
            r#"{"properties":{}}"#,
            r#"{"type":"message.updated","properties":{}}"#,
            r#"{"type":"message.part.updated","properties":{"part":{"type":"text"}}}"#,
        ] {
            assert_eq!(t.feed_raw(raw), None, "{raw}");
        }
        // A status without a session id still applies.
        let raw = r#"{"type":"session.idle","properties":{}}"#;
        assert_eq!(t.feed_raw(raw), update(Some(AgentStatus::Idle), None));
    }

    #[test]
    fn memory_is_bounded_oldest_first() {
        let mut t = OpenCodeTracker::default();
        for n in 0..MAX_CHILDREN + 10 {
            t.feed_raw(&child(&format!("c{n}")));
        }
        assert_eq!(t.children.len(), MAX_CHILDREN);
        // The oldest were forgotten, the newest are kept.
        assert!(t.feed_raw(&status("c0", "idle")).is_some());
        assert_eq!(
            t.feed_raw(&status(&format!("c{}", MAX_CHILDREN + 9), "idle")),
            None
        );

        for n in 0..MAX_USER_MESSAGES + 5 {
            let raw = format!(
                r#"{{"type":"message.updated","properties":{{"info":{{"id":"m{n}","role":"user"}}}}}}"#
            );
            t.feed_raw(&raw);
        }
        assert_eq!(t.user_messages.len(), MAX_USER_MESSAGES);
        assert!(!t.user_messages.iter().any(|m| m == "m0"));
        assert!(
            t.user_messages
                .iter()
                .any(|m| m == &format!("m{}", MAX_USER_MESSAGES + 4))
        );
    }

    fn set(line: &str) -> Activity {
        Activity::Set(line.to_string())
    }

    fn v2(kind: &str, data: &str) -> String {
        format!(r#"{{"id":"evt","type":"{kind}","data":{data}}}"#)
    }

    fn called(session: &str, id: &str, name: &str, input: &str) -> String {
        v2(
            "session.tool.called",
            &format!(
                r#"{{"sessionID":"{session}","assistantMessageID":"m","id":"{id}","name":"{name}","input":{input},"executed":false}}"#
            ),
        )
    }

    fn line_of(t: &mut OpenCodeTracker, raw: &str) -> Option<Activity> {
        t.feed_raw(raw).map(|u| u.activity)
    }

    #[test]
    fn each_tool_has_its_claude_code_line() {
        let mut t = OpenCodeTracker::default();
        let rows = [
            (
                "bash",
                r#"{"command":"cargo test\n--all"}"#,
                "running cargo test",
            ),
            (
                "bash",
                r#"{"command":"ls","description":"List files"}"#,
                "list files",
            ),
            ("bash", r#"{}"#, "running a command"),
            (
                "read",
                r#"{"filePath":"/a/b/sidebar.rs"}"#,
                "reading sidebar.rs",
            ),
            (
                "edit",
                r#"{"filePath":"/a/b/sidebar.rs","oldString":"x"}"#,
                "editing sidebar.rs",
            ),
            (
                "write",
                r#"{"filePath":"/a/notes.md","content":"x"}"#,
                "editing notes.md",
            ),
            ("grep", r#"{"pattern":"secret"}"#, "searching"),
            ("glob", r#"{"pattern":"*.rs"}"#, "searching"),
            (
                "webfetch",
                r#"{"url":"https://example.com/x"}"#,
                "fetching a page",
            ),
            (
                "task",
                r#"{"description":"Review the diff","prompt":"long"}"#,
                "dispatching Review the diff",
            ),
            ("todowrite", r#"{"todos":[]}"#, "planning"),
            ("lsp", r#"{}"#, "using lsp"),
            (
                "shell",
                r#"{"command":"cargo build","workdir":"/x","background":false}"#,
                "running cargo build",
            ),
            (
                "subagent",
                r#"{"agent":"explore","description":"Find the bug","prompt":"long"}"#,
                "dispatching explore: Find the bug",
            ),
            (
                "question",
                r#"{"questions":[{"header":"Repo flow","question":"Q?"}]}"#,
                "asking: Repo flow",
            ),
            ("question", r#"{}"#, "asking a question"),
            (
                "read",
                r#"{"path":"/a/b/sidebar.rs"}"#,
                "reading sidebar.rs",
            ),
            (
                "edit",
                r#"{"path":"/a/b/sidebar.rs"}"#,
                "editing sidebar.rs",
            ),
            (
                "patch",
                r#"{"patchText":"*** Begin Patch\n*** Update File: src/lib.rs\n@@\n+x\n*** End Patch"}"#,
                "editing lib.rs",
            ),
            ("patch", r#"{"patchText":"nothing"}"#, "editing files"),
        ];
        for (n, (name, input, line)) in rows.into_iter().enumerate() {
            let raw = called("s1", &format!("c{n}"), name, input);
            assert_eq!(line_of(&mut t, &raw), Some(set(line)), "{name} {input}");
        }
    }

    #[test]
    fn v1_running_tool_parts_give_the_same_lines_once() {
        let mut t = OpenCodeTracker::default();
        let part = |status: &str| {
            format!(
                r#"{{"type":"message.part.updated","properties":{{"part":{{"id":"p","sessionID":"s1","messageID":"m","type":"tool","callID":"c1","tool":"read","state":{{"status":"{status}","input":{{"filePath":"/x/a.rs"}}}}}}}}}}"#
            )
        };
        assert_eq!(line_of(&mut t, &part("pending")), None);
        assert_eq!(line_of(&mut t, &part("running")), Some(set("reading a.rs")));
        assert_eq!(line_of(&mut t, &part("running")), None);
        assert_eq!(line_of(&mut t, &part("completed")), None);
    }

    #[test]
    fn a_turn_thinks_then_ends_with_its_last_text_or_done() {
        let mut t = OpenCodeTracker::default();
        let busy = v2(
            "session.status",
            r#"{"sessionID":"s1","status":{"type":"busy"}}"#,
        );
        let idle = v2(
            "session.status",
            r#"{"sessionID":"s1","status":{"type":"idle"}}"#,
        );
        assert_eq!(line_of(&mut t, &busy), Some(set("thinking")));
        // After a tool a repeated busy leaves the line alone.
        t.feed_raw(&called("s1", "c1", "read", r#"{"filePath":"a.rs"}"#));
        assert_eq!(line_of(&mut t, &busy), Some(Activity::Keep));
        let text = v2(
            "deck.text",
            r#"{"sessionID":"s1","text":"All **done**\nsecond"}"#,
        );
        assert_eq!(t.feed_raw(&text), None);
        assert_eq!(line_of(&mut t, &idle), Some(set("All done")));
        // The next turn starts clean: no stale text.
        assert_eq!(line_of(&mut t, &busy), Some(set("thinking")));
        assert_eq!(line_of(&mut t, &idle), Some(set("done")));
        // The deprecated idle event ends a turn the same way.
        t.feed_raw(&text);
        let old = v2("session.idle", r#"{"sessionID":"s1"}"#);
        assert_eq!(line_of(&mut t, &old), Some(set("All done")));
    }

    #[test]
    fn a_permission_names_its_tool() {
        let mut t = OpenCodeTracker::default();
        t.feed_raw(&called("s1", "c1", "bash", r#"{"command":"rm -rf x"}"#));
        let asked = |extra: &str| {
            v2(
                "permission.asked",
                &format!(
                    r#"{{"id":"per_1","sessionID":"s1","action":"bash","resources":["x"]{extra}}}"#
                ),
            )
        };
        assert_eq!(
            line_of(
                &mut t,
                &asked(r#","source":{"type":"tool","messageID":"m","id":"c1"}"#)
            ),
            Some(set("needs permission: running rm -rf x"))
        );
        assert_eq!(
            line_of(&mut t, &asked("")),
            Some(set("needs permission: running a command"))
        );
        // A granted call runs again: its line comes back; a rejection clears it.
        t.feed_raw(&asked(
            r#","source":{"type":"tool","messageID":"m","id":"c1"}"#,
        ));
        let reply = |reply: &str| {
            v2(
                "permission.replied",
                &format!(r#"{{"sessionID":"s1","requestID":"per_1","reply":"{reply}"}}"#),
            )
        };
        assert_eq!(
            line_of(&mut t, &reply("once")),
            Some(set("running rm -rf x"))
        );
        t.feed_raw(&asked(
            r#","source":{"type":"tool","messageID":"m","id":"c1"}"#,
        ));
        assert_eq!(line_of(&mut t, &reply("reject")), Some(Activity::Clear));
        let other = v2("permission.asked", r#"{"sessionID":"s1"}"#);
        assert_eq!(line_of(&mut t, &other), Some(set("needs permission")));
        let replied = v2(
            "permission.replied",
            r#"{"sessionID":"s1","requestID":"per_1","reply":"once"}"#,
        );
        assert_eq!(line_of(&mut t, &replied), Some(Activity::Clear));
    }

    #[test]
    fn subagent_tool_and_text_events_are_ignored() {
        let mut t = OpenCodeTracker::default();
        t.feed_raw(&v2(
            "session.created",
            r#"{"sessionID":"sub","parentID":"s1"}"#,
        ));
        assert_eq!(
            t.feed_raw(&called("sub", "c1", "bash", r#"{"command":"ls"}"#)),
            None
        );
        assert_eq!(
            t.feed_raw(&v2("deck.text", r#"{"sessionID":"sub","text":"hi"}"#)),
            None
        );
        assert!(t.last_text.is_none() && !t.turn_tool && t.calls.is_empty());
    }

    #[test]
    fn remembered_calls_are_bounded() {
        let mut t = OpenCodeTracker::default();
        for n in 0..MAX_CALLS + 5 {
            t.feed_raw(&called(
                "s1",
                &format!("c{n}"),
                "bash",
                r#"{"command":"ls"}"#,
            ));
        }
        assert_eq!(t.calls.len(), MAX_CALLS);
    }

    #[test]
    fn reduction_keeps_the_line_and_drops_the_rest() {
        let marker = "MARKER-SECRET";
        let called = serde_json::json!({"type":"session.tool.called","data":{
            "sessionID":"s1","id":"c1","name":"write","executed":true,
            "input":{"filePath":format!("/x/{marker}.txt"),"content":format!("body {marker}"),
                     "command":format!("echo hi\n{marker}"),"description":"Write it"},
            "native": {"out": marker}}});
        let reduced = reduce_opencode_event(&called);
        let dump = reduced.to_string();
        assert!(
            dump.contains("echo hi") && dump.contains("Write it") && dump.contains("sessionID")
        );
        assert!(!dump.contains("body") && !dump.contains("native") && !dump.contains("executed"));
        assert_eq!(
            dump.matches(marker).count(),
            1,
            "only the file name: {dump}"
        );

        let v1 = serde_json::json!({"type":"message.part.updated","properties":{"part":{
            "type":"tool","sessionID":"s1","callID":"c","tool":"bash",
            "state":{"status":"running","input":{"command":format!("ls\n{marker}")},
                     "output": marker, "metadata": {"o": marker}}}}});
        assert!(!reduce_opencode_event(&v1).to_string().contains(marker));

        let text = serde_json::json!({"type":"deck.text","data":{"sessionID":"s1",
            "text":format!("{}\n{marker}", "a".repeat(600))}});
        let reduced = reduce_opencode_event(&text);
        assert!(!reduced.to_string().contains(marker));
        assert_eq!(
            reduced["data"]["text"].as_str().map(str::len),
            Some(crate::agents::ACTIVITY_MAX)
        );
        let ended = serde_json::json!({"type":"session.text.ended","data":{"sessionID":"s1",
            "assistantMessageID":"m","text":format!("first\n{marker}")}});
        assert_eq!(
            reduce_opencode_event(&ended),
            serde_json::json!({"type":"session.text.ended","data":{"sessionID":"s1","text":"first"}})
        );
        let part = serde_json::json!({"type":"message.part.updated","properties":{"part":{
            "type":"text","sessionID":"s1","messageID":"m","text":format!("fix it\n{marker}")}}});
        let reduced = reduce_opencode_event(&part);
        assert!(!reduced.to_string().contains(marker));
        assert_eq!(reduced["properties"]["part"]["text"], "fix it");

        // The tracker reads what the reduction leaves.
        let mut t = OpenCodeTracker::default();
        assert_eq!(
            t.feed(&reduce_opencode_event(&called)).map(|u| u.activity),
            Some(set("editing MARKER-SECRET.txt"))
        );
    }

    #[test]
    fn permissions_prompts_and_patches_are_reduced_too() {
        let marker = "MARKER-SECRET";
        let asked = serde_json::json!({"type":"permission.asked","data":{
            "id":"per_1","sessionID":"s1","action":"shell",
            "resources":[format!("rm x\n{marker}"), marker],
            "save":[marker],"metadata":{"m":marker},"message":marker,
            "source":{"type":"tool","messageID":"m","id":"c1"}}});
        let reduced = reduce_opencode_event(&asked);
        assert!(!reduced.to_string().contains(marker), "{reduced}");
        assert_eq!(reduced["data"]["resources"][0], "rm x");
        assert_eq!(reduced["data"]["source"]["id"], "c1");

        let inbox = serde_json::json!({"type":"session.inbox.enqueued","data":{
            "sessionID":"s1","inboxID":"i","item":{"type":"user","delivery":marker,
            "payload":{"text":format!("fix it\n{marker}"),"files":[marker]}}}});
        let reduced = reduce_opencode_event(&inbox);
        assert!(!reduced.to_string().contains(marker), "{reduced}");
        let mut t = OpenCodeTracker::default();
        assert_eq!(
            t.feed(&reduced).and_then(|u| u.prompt).as_deref(),
            Some("fix it")
        );
        let other = serde_json::json!({"type":"session.inbox.enqueued","data":{
            "sessionID":"s1","item":{"type":"synthetic","payload":{"text":marker}}}});
        assert!(!reduce_opencode_event(&other).to_string().contains(marker));

        let patch = serde_json::json!({"type":"session.tool.called","data":{
            "sessionID":"s1","id":"c","name":"patch",
            "input":{"patchText":format!("*** Begin Patch\n*** Add File: /x/new.rs\n+{marker}\n*** End Patch")}}});
        let reduced = reduce_opencode_event(&patch);
        assert!(!reduced.to_string().contains(marker), "{reduced}");
        assert_eq!(reduced["data"]["input"]["file"], "new.rs");
        let sub = serde_json::json!({"type":"session.tool.called","data":{
            "sessionID":"s1","id":"c","name":"subagent",
            "input":{"agent":"explore","description":"d","prompt":marker}}});
        assert!(!reduce_opencode_event(&sub).to_string().contains(marker));
        let q = serde_json::json!({"type":"session.tool.called","data":{
            "sessionID":"s1","id":"c","name":"question",
            "input":{"questions":[{"header":"H","question":marker,"options":[marker]}]}}});
        let reduced = reduce_opencode_event(&q);
        assert!(!reduced.to_string().contains(marker), "{reduced}");
        assert_eq!(reduced["data"]["input"]["header"], "H");
    }

    #[test]
    fn other_events_pass_unchanged() {
        for raw in [
            r#"{"type":"session.status","data":{"sessionID":"s","status":{"type":"busy"}}}"#,
            r#"{"type":"message.part.updated","properties":{"part":{"type":"reasoning","text":"a\nb"}}}"#,
            r#"{"x":1}"#,
        ] {
            let doc: Value = serde_json::from_str(raw).unwrap();
            assert_eq!(reduce_opencode_event(&doc), doc, "{raw}");
        }
    }

    /// OpenCode v2.4 marks a turn with `session.execution.*` and asks with forms; the shapes
    /// the herdr integration (v13) reads from a v2.4.2 TUI: `data.sessionID`, and a form's
    /// session in `data.form.sessionID`.
    #[test]
    fn v2_4_turns_and_forms_set_the_status_and_the_line() {
        let mut t = OpenCodeTracker::default();
        let ev = |kind: &str, data: Value| json!({"type": kind, "data": data});
        let s1 = || json!({"sessionID": "ses_1"});
        assert_eq!(
            t.feed(&ev("session.execution.started", s1())),
            update(Some(AgentStatus::Working), Some("ses_1"))
        );
        t.feed(&ev(
            "deck.text",
            json!({"sessionID": "ses_1", "text": "Pong. I am here"}),
        ));
        assert_eq!(
            t.feed(&ev("session.execution.succeeded", s1())),
            update_with(
                Some(AgentStatus::Idle),
                Some("ses_1"),
                Activity::Set("Pong. I am here".into())
            )
        );
        assert_eq!(
            t.feed(&ev("session.execution.interrupted", s1())),
            update_with(
                Some(AgentStatus::Idle),
                Some("ses_1"),
                Activity::Set("interrupted".into())
            )
        );
        assert_eq!(
            t.feed(&ev("session.execution.failed", s1())),
            update_with(
                Some(AgentStatus::Idle),
                Some("ses_1"),
                Activity::Set("failed".into())
            )
        );
        let form = json!({"form": {"id": "frm_1", "sessionID": "ses_1", "title": "Pick a model"}});
        assert_eq!(
            t.feed(&ev("form.created", form)),
            update_with(
                Some(AgentStatus::Waiting),
                Some("ses_1"),
                Activity::Set("asking: Pick a model".into())
            )
        );
        let untitled = json!({"form": {"id": "frm_2", "sessionID": "ses_1"}});
        assert_eq!(
            t.feed(&ev("form.created", untitled)),
            update_with(
                Some(AgentStatus::Waiting),
                Some("ses_1"),
                Activity::Set("asking a question".into())
            )
        );
        assert_eq!(
            t.feed(&ev(
                "form.replied",
                json!({"sessionID": "ses_1", "id": "frm_2"})
            )),
            update_with(Some(AgentStatus::Working), Some("ses_1"), Activity::Clear)
        );
    }

    #[test]
    fn a_form_is_reduced_to_its_ids_and_title() {
        let doc = json!({"type": "form.created", "data": {"form": {
            "id": "frm_1", "sessionID": "ses_1", "title": "Pick SECRET-MARKER",
            "fields": [{"key": "k", "title": "Which one", "description": "SECRET-MARKER-2"}]
        }}});
        let reduced = reduce_opencode_event(&doc).to_string();
        assert!(
            reduced.contains("frm_1") && reduced.contains("ses_1"),
            "{reduced}"
        );
        assert!(!reduced.contains("SECRET-MARKER-2"), "{reduced}");
        assert!(!reduced.contains("fields"), "{reduced}");
    }
}
