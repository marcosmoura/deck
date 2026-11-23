//! Pure parsers for the sessions the agents keep on disk, and the helpers the restore dialog
//! needs: ordering, filter, project choice and age text. The daemon reads the files and passes
//! bytes in; nothing here does I/O or panics on arbitrary input.

use std::cmp::Reverse;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use unicode_width::UnicodeWidthStr;

use crate::agents::{AgentKind, valid_session_id};
use crate::model::ProjectId;

/// The longest title, in display cells.
pub const TITLE_MAX_CELLS: usize = 80;

/// One resumable agent session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSession {
    pub kind: AgentKind,
    pub id: String,
    pub title: String,
    #[serde(with = "crate::wire::path")]
    pub cwd: PathBuf,
    /// Epoch seconds.
    pub updated: u64,
}

/// Turn `raw` into a one-line title: control characters become spaces, whitespace runs
/// collapse, and the result is cut to [`TITLE_MAX_CELLS`] cells with `…`.
pub fn clean_title(raw: &str) -> String {
    let mut out = String::new();
    for word in raw
        .split(|c: char| c.is_control() || c.is_whitespace())
        .filter(|w| !w.is_empty())
    {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    cut_cells(&out, TITLE_MAX_CELLS)
}

fn cut_cells(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    // Measure the growing prefix as a string so sequences such as `❤️` count as the renderer
    // counts them; the loop ends within about `max` characters.
    let mut out = String::new();
    for c in s.chars() {
        out.push(c);
        if out.width() > max.saturating_sub(1) {
            out.pop();
            break;
        }
    }
    out.push('…');
    out
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36
        && s.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// Whether a file name in a Claude Code project directory is a session transcript:
/// `<uuid>.jsonl`.
pub fn is_claude_session_file(name: &str) -> bool {
    name.strip_suffix(".jsonl").is_some_and(is_uuid)
}

/// The JSON object lines of `head`; anything else (blank, cut, not an object) is skipped.
fn json_lines(head: &[u8]) -> impl Iterator<Item = Value> + '_ {
    head.split(|b| *b == b'\n')
        .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
        .filter(Value::is_object)
}

fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// The cleaned first line of `text`, empty when it has none.
fn first_line_title(text: &str) -> String {
    clean_title(text.trim().lines().next().unwrap_or(""))
}

/// The text of a Claude user line: a string, or the first `text` block of a list that is not
/// empty or a `<` wrapper.
fn claude_user_text(line: &Value) -> Option<&str> {
    let content = line.get("message")?.get("content")?;
    match content {
        Value::String(s) => Some(s),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| str_field(b, "type") == Some("text"))
            .filter_map(|b| str_field(b, "text"))
            .find(|t| !t.trim().is_empty() && !t.trim().starts_with('<')),
        _ => None,
    }
}

/// Parse the head of a Claude Code transcript. `file_stem` is the session id.
pub fn parse_claude(file_stem: &str, head: &[u8], mtime: u64) -> Option<AgentSession> {
    if !is_uuid(file_stem) || !valid_session_id(file_stem) {
        return None;
    }
    let mut cwd: Option<&str> = None;
    let mut summary: Option<String> = None;
    let mut prompt: Option<String> = None;
    let lines: Vec<Value> = json_lines(head).collect();
    for line in &lines {
        if cwd.is_none() {
            cwd = str_field(line, "cwd").filter(|c| !c.is_empty());
        }
        if flag(line, "isSidechain") || flag(line, "isMeta") {
            continue;
        }
        match str_field(line, "type") {
            Some("summary") if summary.is_none() => {
                if let Some(s) = str_field(line, "summary") {
                    let title = first_line_title(s);
                    if !title.is_empty() {
                        summary = Some(title);
                    }
                }
            }
            Some("user") if prompt.is_none() => {
                if let Some(text) = claude_user_text(line) {
                    let text = text.trim();
                    if !text.is_empty() && !text.starts_with('<') {
                        let title = first_line_title(text);
                        if !title.is_empty() {
                            prompt = Some(title);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Some(AgentSession {
        kind: AgentKind::Claude,
        id: file_stem.to_string(),
        title: summary.or(prompt)?,
        cwd: PathBuf::from(cwd?),
        updated: mtime,
    })
}

/// The trailing UUID of `rollout-<timestamp>-<id>.jsonl`.
fn codex_id_from_name(file_name: &str) -> Option<&str> {
    let stem = file_name.strip_suffix(".jsonl")?;
    let id = stem.get(stem.len().checked_sub(36)?..)?;
    is_uuid(id).then_some(id)
}

/// Parse the head of a Codex rollout file.
pub fn parse_codex(file_name: &str, head: &[u8], mtime: u64) -> Option<AgentSession> {
    let mut lines = json_lines_strict(head);
    let meta = lines.next()?;
    if str_field(&meta, "type") != Some("session_meta") {
        return None;
    }
    let payload = meta.get("payload")?;
    let id = str_field(payload, "id")
        .filter(|id| !id.is_empty())
        .or_else(|| codex_id_from_name(file_name))?;
    if !valid_session_id(id) {
        return None;
    }
    let cwd = str_field(payload, "cwd").filter(|c| !c.is_empty())?;
    let title = lines
        .filter(|l| str_field(l, "type") == Some("event_msg"))
        .filter_map(|l| {
            let p = l.get("payload")?;
            (str_field(p, "type") == Some("user_message"))
                .then(|| str_field(p, "message").map(first_line_title))
                .flatten()
        })
        .find(|t| !t.is_empty())?;
    Some(AgentSession {
        kind: AgentKind::Codex,
        id: id.to_string(),
        title,
        cwd: PathBuf::from(cwd),
        updated: mtime,
    })
}

/// Like [`json_lines`], but the first line must itself be an object, so a rollout whose first
/// line is garbage yields nothing instead of starting at a later line.
fn json_lines_strict(head: &[u8]) -> impl Iterator<Item = Value> + '_ {
    let mut raw = head.split(|b| *b == b'\n');
    let first = raw
        .next()
        .and_then(|l| serde_json::from_slice::<Value>(l).ok())
        .filter(Value::is_object);
    let ok = first.is_some();
    first.into_iter().chain(
        raw.filter(move |_| ok)
            .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
            .filter(Value::is_object),
    )
}

/// Epoch seconds from a number in seconds or milliseconds (above 10¹¹ is milliseconds).
fn epoch_seconds(v: &Value) -> Option<u64> {
    let n = v
        .as_u64()
        .or_else(|| v.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64))?;
    Some(if n > 100_000_000_000 { n / 1000 } else { n })
}

/// Parse the array printed by `opencode session list --format json`.
pub fn parse_opencode_list(json: &[u8]) -> Vec<AgentSession> {
    let Ok(Value::Array(items)) = serde_json::from_slice::<Value>(json) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let id = str_field(item, "id").filter(|id| valid_session_id(id))?;
            let directory = str_field(item, "directory").filter(|d| !d.is_empty())?;
            let title = clean_title(str_field(item, "title")?);
            if title.is_empty() {
                return None;
            }
            Some(AgentSession {
                kind: AgentKind::OpenCode,
                id: id.to_string(),
                title,
                cwd: PathBuf::from(directory),
                updated: item
                    .get("updated")
                    .and_then(epoch_seconds)
                    .or_else(|| item.get("created").and_then(epoch_seconds))?,
            })
        })
        .collect()
}

/// Newest first (ties by id), duplicate ids removed keeping the newest, at most `limit`.
pub fn recent(mut sessions: Vec<AgentSession>, limit: usize) -> Vec<AgentSession> {
    sessions.sort_by(|a, b| (Reverse(a.updated), &a.id).cmp(&(Reverse(b.updated), &b.id)));
    let mut seen = HashSet::new();
    sessions.retain(|s| seen.insert(s.id.clone()));
    sessions.truncate(limit);
    sessions
}

/// Whether every word of `query` is a case-insensitive substring of the title or directory.
pub fn matches(session: &AgentSession, query: &str) -> bool {
    let title = session.title.to_lowercase();
    let dir = session.cwd.to_string_lossy().to_lowercase();
    query
        .split_whitespace()
        .map(str::to_lowercase)
        .all(|w| title.contains(&w) || dir.contains(&w))
}

/// The project a session reopens in: the one whose root is `cwd` or an ancestor of it (the
/// longest root wins, a tie goes to `active`), else `active`; `None` without projects.
pub fn project_for(
    cwd: &Path,
    projects: &[(ProjectId, PathBuf)],
    active: Option<ProjectId>,
) -> Option<ProjectId> {
    if projects.is_empty() {
        return None;
    }
    let mut best: Option<(usize, ProjectId)> = None;
    for (id, root) in projects {
        if !cwd.starts_with(root) {
            continue;
        }
        let depth = root.components().count();
        best = match best {
            Some((d, _)) if d > depth => best,
            Some((d, b)) if d == depth && (Some(b) == active || Some(*id) != active) => best,
            _ => Some((depth, *id)),
        };
    }
    best.map(|(_, id)| id).or(active)
}

/// How long ago `updated` was: `now`, `5m`, `3h`, `2d`, `6w`, `8mo`, `2y`.
pub fn age(now: u64, updated: u64) -> String {
    let secs = now.saturating_sub(updated);
    let days = secs / 86_400;
    match secs {
        0..60 => "now".to_string(),
        60..3600 => format!("{}m", secs / 60),
        3600..86_400 => format!("{}h", secs / 3600),
        _ if days < 7 => format!("{days}d"),
        _ if days < 60 => format!("{}w", days / 7),
        _ if days < 365 => format!("{}mo", days / 30),
        _ => format!("{}y", days / 365),
    }
}

/// `cwd` with the home prefix shown as `~`.
pub fn short_dir(cwd: &Path, home: &Path) -> String {
    if !home.as_os_str().is_empty()
        && let Ok(rest) = cwd.strip_prefix(home)
    {
        return if rest.as_os_str().is_empty() {
            "~".to_string()
        } else {
            format!("~/{}", rest.display())
        };
    }
    cwd.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! fixture {
        ($name:literal) => {
            include_bytes!(concat!("../tests/fixtures/agent-sessions/", $name)).as_slice()
        };
    }

    const U: &str = "0a1b2c3d-1111-4222-8333-444455556666";
    const V: &str = "0b2c3d4e-2222-4333-8444-555566667777";

    fn claude(head: &[u8]) -> Option<AgentSession> {
        parse_claude(U, head, 100)
    }

    fn title_of(head: &[u8]) -> Option<String> {
        claude(head).map(|s| s.title)
    }

    fn session(id: &str, title: &str, cwd: &str, updated: u64) -> AgentSession {
        AgentSession {
            kind: AgentKind::Claude,
            id: id.into(),
            title: title.into(),
            cwd: cwd.into(),
            updated,
        }
    }

    #[test]
    fn serde_round_trip_keeps_a_non_utf8_path() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        let mut s = session("a", "t", "/x", 5);
        s.cwd = PathBuf::from(OsString::from_vec(b"/home/u/\xff\xfe".to_vec()));
        let bytes = postcard::to_stdvec(&s).unwrap();
        assert_eq!(postcard::from_bytes::<AgentSession>(&bytes).unwrap(), s);
    }

    #[test]
    fn claude_file_names() {
        assert!(is_claude_session_file(&format!("{U}.jsonl")));
        assert!(!is_claude_session_file("agent-ab12cd34.jsonl"));
        assert!(!is_claude_session_file(&format!("{U}.json")));
        assert!(!is_claude_session_file(""));
    }

    #[test]
    fn claude_agent_file_is_none() {
        assert_eq!(
            parse_claude("agent-ab12cd34", fixture!("claude-user-string.jsonl"), 1),
            None
        );
    }

    #[test]
    fn claude_stem_wins_over_session_id() {
        let s = claude(fixture!("claude-id-mismatch.jsonl")).unwrap();
        assert_eq!(s.id, U);
        assert_eq!(s.title, "Stem wins");
        assert_eq!(s.cwd, PathBuf::from("/home/u/work/api"));
        assert_eq!(s.updated, 100);
        assert_eq!(s.kind, AgentKind::Claude);
    }

    #[test]
    fn claude_meta_line_is_skipped() {
        assert_eq!(
            title_of(fixture!("claude-meta-first.jsonl")).as_deref(),
            Some("Real question here")
        );
    }

    #[test]
    fn claude_summary_wins() {
        assert_eq!(
            title_of(fixture!("claude-summary.jsonl")).as_deref(),
            Some("Fix the login redirect")
        );
    }

    #[test]
    fn claude_prompt_title_first_line_only() {
        assert_eq!(
            title_of(fixture!("claude-user-string.jsonl")).as_deref(),
            Some("Add a retry to the client")
        );
    }

    #[test]
    fn claude_wrappers_are_skipped() {
        assert_eq!(
            title_of(fixture!("claude-wrappers.jsonl")).as_deref(),
            Some("Now the real prompt")
        );
    }

    #[test]
    fn claude_sidechain_is_skipped() {
        assert_eq!(
            title_of(fixture!("claude-sidechain-first.jsonl")).as_deref(),
            Some("Main thread prompt")
        );
    }

    #[test]
    fn claude_block_list_content() {
        assert_eq!(
            title_of(fixture!("claude-user-blocks.jsonl")).as_deref(),
            Some("Explain the cache layer")
        );
    }

    #[test]
    fn claude_without_title_or_cwd_is_none() {
        assert_eq!(claude(fixture!("claude-empty.jsonl")), None);
        assert_eq!(claude(fixture!("claude-assistant-only.jsonl")), None);
        let no_cwd = br#"{"type":"user","message":{"content":"hi"}}"#;
        assert_eq!(claude(no_cwd), None);
        let empty_cwd = br#"{"type":"user","cwd":"","message":{"content":"hi"}}"#;
        assert_eq!(claude(empty_cwd), None);
    }

    #[test]
    fn claude_cut_last_line_is_ignored() {
        let s = claude(fixture!("claude-truncated.jsonl")).unwrap();
        assert_eq!(s.title, "Complete first line");
        assert_eq!(s.cwd, PathBuf::from("/home/u/work/api"));
        // Only the cut line: nothing usable.
        let cut = br#"{"type":"user","cwd":"/home/u/w","message":{"content":"cu"#;
        assert_eq!(claude(cut), None);
    }

    #[test]
    fn claude_survives_garbage() {
        for head in [&b"\xff\xfe\x00"[..], b"{", b"[]", b"null\n\n123", b"\n\n"] {
            assert_eq!(claude(head), None);
        }
        for head in huge_lines() {
            assert_eq!(claude(&head), None);
            assert_eq!(parse_codex("rollout.jsonl", &head, 1), None);
            assert!(parse_opencode_list(&head).is_empty());
        }
    }

    /// A deeply nested line and a very long line.
    fn huge_lines() -> [Vec<u8>; 2] {
        [
            "[".repeat(100_000).into_bytes(),
            format!("{{\"type\":\"user\",\"cwd\":\"{}", "x".repeat(300 * 1024)).into_bytes(),
        ]
    }

    #[test]
    fn claude_ide_wrapper_blocks_are_skipped() {
        assert_eq!(
            title_of(fixture!("claude-ide-blocks.jsonl")).as_deref(),
            Some("Real prompt after the wrapper")
        );
    }

    #[test]
    fn emoji_presentation_sequences_count_as_the_renderer_counts() {
        let t = clean_title(&"❤️".repeat(60));
        assert!(t.width() <= 80, "{}", t.width());
        assert!(t.ends_with('…'));
    }

    #[test]
    fn opencode_updated_falls_back_to_created() {
        let body = br#"[
          {"id":"a","title":"t","directory":"/w","created":1766000000},
          {"id":"b","title":"t","directory":"/w"}
        ]"#;
        let list = parse_opencode_list(body);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].updated, 1_766_000_000);
    }

    #[test]
    fn codex_normal_rollout() {
        let name = format!("rollout-2026-11-02T09-00-00-{V}.jsonl");
        let s = parse_codex(&name, fixture!("codex-normal.jsonl"), 7).unwrap();
        assert_eq!(s.kind, AgentKind::Codex);
        assert_eq!(s.id, V);
        assert_eq!(s.title, "Port the form to hooks");
        assert_eq!(s.cwd, PathBuf::from("/home/u/work/web"));
        assert_eq!(s.updated, 7);
    }

    #[test]
    fn codex_without_user_message_is_none() {
        let name = format!("rollout-x-{V}.jsonl");
        assert_eq!(parse_codex(&name, fixture!("codex-no-user.jsonl"), 1), None);
    }

    #[test]
    fn codex_id_from_file_name() {
        let name = format!("rollout-2026-11-02T09-00-00-{V}.jsonl");
        let s = parse_codex(&name, fixture!("codex-id-in-name.jsonl"), 1).unwrap();
        assert_eq!(s.id, V);
        assert_eq!(s.title, "Id only in the name");
        assert_eq!(
            parse_codex(
                "rollout-nothing.jsonl",
                fixture!("codex-id-in-name.jsonl"),
                1
            ),
            None
        );
    }

    #[test]
    fn codex_meta_must_be_first() {
        let name = format!("rollout-x-{V}.jsonl");
        assert_eq!(
            parse_codex(&name, fixture!("codex-meta-not-first.jsonl"), 1),
            None
        );
        assert_eq!(
            parse_codex(&name, b"garbage\n{\"type\":\"session_meta\"}", 1),
            None
        );
        assert_eq!(parse_codex(&name, b"", 1), None);
    }

    #[test]
    fn codex_bad_id_is_dropped() {
        let head = br#"{"type":"session_meta","payload":{"id":"-p","cwd":"/w"}}
{"type":"event_msg","payload":{"type":"user_message","message":"hi"}}"#;
        assert_eq!(parse_codex("rollout.jsonl", head, 1), None);
    }

    #[test]
    fn opencode_list_fixture() {
        let list = parse_opencode_list(fixture!("opencode-list.json"));
        let ids: Vec<_> = list.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["ses_0123abc", "ses_0456def"]);
        assert_eq!(list[0].title, "Refactor the router");
        assert_eq!(list[0].cwd, PathBuf::from("/home/u/work/api"));
        assert_eq!(list[0].updated, 1_766_000_000);
        assert_eq!(list[0].kind, AgentKind::OpenCode);
        // Milliseconds are converted.
        assert_eq!(list[1].updated, 1_766_000_000);
    }

    #[test]
    fn opencode_non_array_is_empty() {
        for body in [&b"{}"[..], b"", b"null", b"not json", b"\xff", b"\"x\""] {
            assert!(parse_opencode_list(body).is_empty());
        }
    }

    #[test]
    fn titles_are_one_line_within_80_cells() {
        let long = "x".repeat(200);
        let t = clean_title(&long);
        assert_eq!(t.width(), 80);
        assert!(t.ends_with('…'));

        let wide = "界".repeat(100);
        let t = clean_title(&wide);
        assert!(t.width() <= 80, "{}", t.width());
        assert!(t.ends_with('…'));

        let messy = "a\n\tb\r\n  c\u{7}d";
        assert_eq!(clean_title(messy), "a b c d");

        assert_eq!(clean_title(&"y".repeat(80)), "y".repeat(80));
        assert_eq!(clean_title("   "), "");
    }

    #[test]
    fn ids_follow_the_shared_rule() {
        let item =
            |id: &str| format!(r#"[{{"id":"{id}","title":"t","directory":"/w","updated":1}}]"#);
        for bad in ["-p", "a b;rm"] {
            assert!(
                parse_opencode_list(item(bad).as_bytes()).is_empty(),
                "{bad}"
            );
        }
        for ok in [U, "ses_0123abc"] {
            assert_eq!(parse_opencode_list(item(ok).as_bytes()).len(), 1, "{ok}");
        }
        assert_eq!(parse_claude("-p", b"", 1), None);
    }

    #[test]
    fn recent_dedups_sorts_and_limits() {
        let list = vec![
            session("a", "old a", "/w", 10),
            session("b", "b", "/w", 30),
            session("a", "new a", "/w", 20),
            session("c", "c", "/w", 30),
        ];
        let out = recent(list.clone(), 10);
        let got: Vec<_> = out
            .iter()
            .map(|s| (s.id.as_str(), s.title.as_str()))
            .collect();
        assert_eq!(got, [("b", "b"), ("c", "c"), ("a", "new a")]);
        assert_eq!(recent(list, 2).len(), 2);
        assert!(recent(Vec::new(), 5).is_empty());
    }

    #[test]
    fn matches_words_in_title_or_dir() {
        let s = session("a", "Fix the login bug", "/home/u/work/budget", 1);
        assert!(matches(&s, "fix login"));
        assert!(matches(&s, "FIX LOGIN"));
        assert!(matches(&s, "login budget"));
        assert!(matches(&s, ""));
        assert!(matches(&s, "   "));
        assert!(!matches(&s, "fix logout"));
    }

    #[test]
    fn project_choice() {
        let p = |n, path: &str| (ProjectId(n), PathBuf::from(path));
        let projects = vec![p(1, "/home/u/work"), p(2, "/home/u/work/api")];
        let active = Some(ProjectId(1));
        let pick = |cwd: &str, projects: &[(ProjectId, PathBuf)], a| {
            project_for(Path::new(cwd), projects, a)
        };
        assert_eq!(
            pick("/home/u/work/api/src", &projects, active),
            Some(ProjectId(2))
        );
        assert_eq!(
            pick("/home/u/work/api", &projects, active),
            Some(ProjectId(2))
        );
        assert_eq!(
            pick("/home/u/work/web", &projects, None),
            Some(ProjectId(1))
        );
        // Not a string prefix.
        assert_eq!(
            pick("/home/u/work2", &projects, Some(ProjectId(2))),
            Some(ProjectId(2))
        );
        assert_eq!(pick("/elsewhere", &projects, None), None);
        assert_eq!(pick("/x", &[], active), None);
        // A tie goes to the active project.
        let twins = vec![p(3, "/w"), p(4, "/w")];
        assert_eq!(pick("/w/a", &twins, Some(ProjectId(4))), Some(ProjectId(4)));
        assert_eq!(pick("/w/a", &twins, Some(ProjectId(3))), Some(ProjectId(3)));
    }

    #[test]
    fn age_table() {
        let now = 1_000_000_000;
        let cases = [
            (0, "now"),
            (59, "now"),
            (60, "1m"),
            (3599, "59m"),
            (3 * 3600, "3h"),
            (47 * 3600, "1d"),
            (6 * 86_400, "6d"),
            (13 * 86_400, "1w"),
            (42 * 86_400, "6w"),
            (240 * 86_400, "8mo"),
            (400 * 86_400, "1y"),
            (800 * 86_400, "2y"),
        ];
        for (secs, text) in cases {
            assert_eq!(age(now, now - secs), text, "{secs}s");
        }
        assert_eq!(age(now, now + 500), "now");
    }

    #[test]
    fn short_dir_uses_tilde() {
        let home = Path::new("/home/u");
        assert_eq!(short_dir(Path::new("/home/u"), home), "~");
        assert_eq!(short_dir(Path::new("/home/u/work/api"), home), "~/work/api");
        assert_eq!(short_dir(Path::new("/home/user/x"), home), "/home/user/x");
        assert_eq!(short_dir(Path::new("/srv/x"), home), "/srv/x");
        assert_eq!(short_dir(Path::new("/srv/x"), Path::new("")), "/srv/x");
    }
}
