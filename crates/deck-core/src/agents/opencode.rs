//! The state machine of one terminal's OpenCode: fed the raw events the deck plugin forwards,
//! it drops subagent sessions and tells what the status, session and first prompt are.
//! Pure: no I/O, one value per terminal, bounded memory.
//!
//! Events are the `{ "type", "properties" }` objects of OpenCode's plugin `event` hook.

use std::collections::VecDeque;

use serde_json::Value;

use super::{AgentStatus, clip_prompt};

/// The events the plugin subscribes to and the tracker understands.
pub const OPENCODE_EVENTS: [&str; 8] = [
    "session.created",
    "session.updated",
    "session.status",
    "session.idle",
    "permission.asked",
    "permission.replied",
    "message.updated",
    "message.part.updated",
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
}

#[derive(Debug, Default)]
pub struct OpenCodeTracker {
    children: VecDeque<String>,
    user_messages: VecDeque<String>,
}

fn remember(list: &mut VecDeque<String>, cap: usize, id: &str) {
    if list.iter().any(|known| known == id) {
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

impl OpenCodeTracker {
    /// Whether `id` is a subagent session.
    fn is_child(&self, id: &str) -> bool {
        self.children.iter().any(|c| c == id)
    }

    /// Feeds one event (parsed by the caller, outside any lock); returns what to apply, or
    /// `None` when it changes nothing.
    pub fn feed(&mut self, doc: &Value) -> Option<OpenCodeUpdate> {
        let kind = text(doc, &["type"])?;
        let props = doc.get("properties")?;
        if matches!(kind, "session.created" | "session.updated") {
            if let (Some(id), Some(_)) = (
                text(props, &["info", "id"]),
                text(props, &["info", "parentID"]),
            ) {
                remember(&mut self.children, MAX_CHILDREN, id);
            }
            return None;
        }
        let session = text(props, &["sessionID"])
            .or_else(|| text(props, &["info", "sessionID"]))
            .or_else(|| text(props, &["part", "sessionID"]));
        let child = session.is_some_and(|s| self.is_child(s));
        // A subagent's events are dropped, except that its permission prompts still need the
        // user. They carry no session id: the root's resumable one is kept.
        if child && !matches!(kind, "permission.asked" | "permission.replied") {
            return None;
        }
        let status = |status| {
            Some(OpenCodeUpdate {
                status: Some(status),
                session_id: session.filter(|_| !child).map(str::to_string),
                prompt: None,
            })
        };
        match kind {
            "session.status" => match text(props, &["status", "type"])? {
                "busy" => status(AgentStatus::Working),
                "idle" => status(AgentStatus::Idle),
                _ => None,
            },
            "session.idle" => status(AgentStatus::Idle),
            "permission.asked" => status(AgentStatus::Waiting),
            "permission.replied" => status(AgentStatus::Working),
            "message.updated" => {
                if text(props, &["info", "role"]) == Some("user")
                    && let Some(id) = text(props, &["info", "id"])
                {
                    remember(&mut self.user_messages, MAX_USER_MESSAGES, id);
                }
                None
            }
            "message.part.updated" => {
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

    fn update(status: Option<AgentStatus>, session: Option<&str>) -> Option<OpenCodeUpdate> {
        Some(OpenCodeUpdate {
            status,
            session_id: session.map(str::to_string),
            prompt: None,
        })
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
            update(Some(AgentStatus::Waiting), Some("s1"))
        );
        assert_eq!(
            t.feed_raw(replied),
            update(Some(AgentStatus::Working), Some("s1"))
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
            update(Some(AgentStatus::Working), None)
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
                prompt: Some("fix the bug".into())
            })
        );
        // Only once per message.
        assert_eq!(t.feed_raw(&part("m1", "text", "fix the bug now")), None);
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
}
