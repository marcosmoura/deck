//! `Request::ListAgentSessions`: each agent's recent sessions, read with the session's
//! environment. Every request runs in its own task, so a slow agent never delays the
//! connection's other messages; the answer is queued when the task is done.
//!
//! Titles and directories are for the client that asked: nothing here logs them.

use std::cmp::Reverse;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use deck_core::agent_sessions::{
    AgentSession, is_claude_session_file, parse_claude, parse_codex, parse_opencode_list, recent,
};
use deck_core::agents::{AgentKind, check_template, render_template};
use deck_core::protocol::{Response, ServerMsg};
use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use super::{ClientId, Shared};
use crate::ListingStats;
use crate::env::{self, Env};
use crate::queue::Queue;

/// The most read from the head of a Claude transcript.
const CLAUDE_HEAD_MAX: u64 = 256 * 1024;
/// The most read from one Codex rollout.
const CODEX_READ_MAX: u64 = 4 * 1024 * 1024;
/// The most read from an OpenCode listing's output.
const OUTPUT_MAX: u64 = 4 * 1024 * 1024;
/// OpenCode listings that run at the same time.
pub(super) const OPENCODE_PARALLEL: usize = 2;
/// How long all the OpenCode listings of a request may take.
const OPENCODE_TIMEOUT: Duration = Duration::from_secs(8);

const OPENCODE_FAILED: &str = "could not run opencode session list";
const OPENCODE_TIMED_OUT: &str = "timed out";
const READ_FAILED: &str = "could not read the sessions";

/// What a listing needs of its session.
struct Input {
    env: Env,
    roots: Vec<PathBuf>,
}

impl Shared {
    /// Starts the listing of `kind` for the session of client `cid` in `tasks`; its answer is
    /// pushed to `queue` when it is done. A client without a session gets `no session` at once.
    pub fn spawn_listing(
        self: &Arc<Self>,
        cid: ClientId,
        kind: AgentKind,
        limit: u16,
        queue: &Arc<Queue>,
        tasks: &mut JoinSet<()>,
    ) {
        while tasks.try_join_next().is_some() {}
        let input = {
            let st = self.st();
            st.clients
                .get(&cid)
                .and_then(|c| c.session)
                .and_then(|sid| st.session(sid))
                .map(|sess| {
                    let mut roots: Vec<PathBuf> = Vec::new();
                    for p in &sess.model.projects {
                        if !roots.contains(&p.root) {
                            roots.push(p.root.clone());
                        }
                    }
                    Input {
                        env: sess.env.clone(),
                        roots,
                    }
                })
        };
        let Some(input) = input else {
            queue.push_msg(ServerMsg::Response(Response::Error("no session".into())));
            return;
        };
        let (shared, queue) = (Arc::clone(self), Arc::clone(queue));
        tasks.spawn(async move {
            let started = Instant::now();
            let (sessions, error) = shared
                .list(kind, usize::from(limit.clamp(1, 100)), input)
                .await;
            // Counts and timings only: titles and directories are never logged.
            tracing::info!(
                kind = kind.name(),
                count = sessions.len(),
                failed = error.is_some(),
                ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                "listed agent sessions"
            );
            queue.push_msg(ServerMsg::Response(Response::AgentSessions {
                kind,
                sessions,
                error,
            }));
        });
    }

    async fn list(
        &self,
        kind: AgentKind,
        limit: usize,
        input: Input,
    ) -> (Vec<AgentSession>, Option<String>) {
        let stats = Arc::clone(&self.listing);
        match kind {
            AgentKind::Claude => {
                let root = agent_root(&input.env, "CLAUDE_CONFIG_DIR", ".claude", "projects");
                blocking(move || root.map(|r| claude(&r, limit, &stats)).unwrap_or_default()).await
            }
            AgentKind::Codex => {
                let root = agent_root(&input.env, "CODEX_HOME", ".codex", "sessions");
                blocking(move || root.map(|r| codex(&r, limit, &stats)).unwrap_or_default()).await
            }
            AgentKind::OpenCode => {
                let command = self.config().agents.get(kind).command.clone();
                let program = check_template(&command, false)
                    .ok()
                    .and_then(|()| render_template(&command, None).into_iter().next());
                opencode(program, input, limit, &stats, &self.opencode_slots).await
            }
        }
    }
}

/// Runs a directory walk on the blocking pool.
async fn blocking(
    walk: impl FnOnce() -> Vec<AgentSession> + Send + 'static,
) -> (Vec<AgentSession>, Option<String>) {
    match tokio::task::spawn_blocking(walk).await {
        Ok(sessions) => (sessions, None),
        Err(_) => (Vec::new(), Some(READ_FAILED.to_string())),
    }
}

/// `<$var or $HOME/<home_sub>>/<leaf>`; `None` without either.
fn agent_root(env: &Env, var: &str, home_sub: &str, leaf: &str) -> Option<PathBuf> {
    let set = |key: &str| env::lookup(env, key).filter(|v| !v.is_empty());
    let base = match set(var) {
        Some(dir) => PathBuf::from(dir),
        None => Path::new(set("HOME")?).join(home_sub),
    };
    Some(base.join(leaf))
}

fn epoch_secs(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The entries of `dir` whose names `keep` accepts, as (name, path), names descending.
fn names_desc(dir: &Path, want_dir: bool, keep: impl Fn(&str) -> bool) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter(|e| {
            e.file_type()
                .is_ok_and(|t| if want_dir { t.is_dir() } else { t.is_file() })
        })
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            keep(&name).then(|| (name, e.path()))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found
}

/// Opens a session file, counted.
fn open(path: &Path, stats: &ListingStats) -> Option<fs::File> {
    let file = fs::File::open(path).ok()?;
    stats.files_opened.fetch_add(1, Ordering::Relaxed);
    Some(file)
}

/// Claude Code: `projects/*/<uuid>.jsonl`, the newest `limit × 2` by modification time, each
/// read up to 256 KiB.
fn claude(root: &Path, limit: usize, stats: &ListingStats) -> Vec<AgentSession> {
    let mut found: Vec<(SystemTime, String, PathBuf)> = Vec::new();
    for (_, dir) in names_desc(root, true, |_| true) {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if !is_claude_session_file(&name) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if let (true, Ok(modified)) = (meta.is_file(), meta.modified()) {
                found.push((modified, name, entry.path()));
            }
        }
    }
    found.sort_by(|a, b| (Reverse(a.0), &a.1).cmp(&(Reverse(b.0), &b.1)));
    found.truncate(limit.saturating_mul(2));
    let sessions = found
        .into_iter()
        .filter_map(|(modified, name, path)| {
            let mut head = Vec::new();
            let mut file = open(&path, stats)?.take(CLAUDE_HEAD_MAX);
            let n = file.read_to_end(&mut head).ok()?;
            stats.bytes_read.fetch_add(n, Ordering::Relaxed);
            let stem = name.strip_suffix(".jsonl")?;
            parse_claude(stem, &head, epoch_secs(modified))
        })
        .collect();
    recent(sessions, limit)
}

fn is_digits(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit())
}

/// Codex: `sessions/YYYY/MM/DD/rollout-*.jsonl`, walked newest name first until `limit × 2`
/// candidates exist.
fn codex(root: &Path, limit: usize, stats: &ListingStats) -> Vec<AgentSession> {
    let want = limit.saturating_mul(2);
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    'walk: for (_, year) in names_desc(root, true, is_digits) {
        for (_, month) in names_desc(&year, true, is_digits) {
            for (_, day) in names_desc(&month, true, is_digits) {
                let rollouts = names_desc(&day, false, |n| {
                    n.starts_with("rollout-") && n.ends_with(".jsonl")
                });
                for file in rollouts {
                    found.push(file);
                    if found.len() >= want {
                        break 'walk;
                    }
                }
            }
        }
    }
    let sessions = found
        .into_iter()
        .filter_map(|(name, path)| {
            let modified = fs::metadata(&path).ok()?.modified().ok()?;
            let head = codex_head(&name, &path, stats)?;
            parse_codex(&name, &head, epoch_secs(modified))
        })
        .collect();
    recent(sessions, limit)
}

/// What `parse_codex` needs of a rollout, read line by line: the first line and the first
/// `user_message` it makes a title of, whatever comes between them left out. At most 4 MiB
/// are read, and a rollout without a title in them (or whose first line is not a usable
/// `session_meta`) has none.
fn codex_head(name: &str, path: &Path, stats: &ListingStats) -> Option<Vec<u8>> {
    // A title line that always works, to check the first line once.
    const PROBE: &[u8] =
        b"{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"x\"}}\n";
    let mut reader = BufReader::new(open(path, stats)?.take(CODEX_READ_MAX));
    let mut head: Vec<u8> = Vec::new();
    let n = reader.read_until(b'\n', &mut head).ok()?;
    stats.bytes_read.fetch_add(n, Ordering::Relaxed);
    if !head.ends_with(b"\n") {
        head.push(b'\n');
    }
    let first_len = head.len();
    head.extend_from_slice(PROBE);
    parse_codex(name, &head, 0)?;
    // Each `user_message` is tried alone after the first line.
    let mut line: Vec<u8> = Vec::new();
    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line).ok()?;
        if n == 0 {
            return None;
        }
        stats.bytes_read.fetch_add(n, Ordering::Relaxed);
        if is_user_message(&line) {
            head.truncate(first_len);
            head.extend_from_slice(&line);
            if parse_codex(name, &head, 0).is_some() {
                return Some(head);
            }
        }
    }
}

/// Whether a line is an `event_msg` with a `user_message` payload.
fn is_user_message(line: &[u8]) -> bool {
    const NEEDLE: &[u8] = b"user_message";
    if !line.windows(NEEDLE.len()).any(|w| w == NEEDLE) {
        return false;
    }
    serde_json::from_slice::<Value>(line).is_ok_and(|v| {
        v.get("type").and_then(Value::as_str) == Some("event_msg")
            && v.get("payload")
                .and_then(|p| p.get("type"))
                .and_then(Value::as_str)
                == Some("user_message")
    })
}

/// OpenCode: `opencode session list` in every project root and the home directory, two at a
/// time, all inside 8 s.
async fn opencode(
    program: Option<String>,
    input: Input,
    limit: usize,
    stats: &Arc<ListingStats>,
    permits: &Arc<Semaphore>,
) -> (Vec<AgentSession>, Option<String>) {
    let Some(program) = program else {
        return (Vec::new(), Some(OPENCODE_FAILED.to_string()));
    };
    let mut dirs = input.roots;
    if let Some(home) = env::lookup(&input.env, "HOME").filter(|h| !h.is_empty()) {
        let home = PathBuf::from(home);
        if !dirs.contains(&home) {
            dirs.push(home);
        }
    }
    if dirs.is_empty() {
        return (Vec::new(), None);
    }
    // Each run stops at the deadline; its child is reaped by `Reaped`, not left to tokio.
    let deadline = tokio::time::Instant::now() + OPENCODE_TIMEOUT;
    let mut children = JoinSet::new();
    for dir in &dirs {
        children.spawn(run_opencode(
            program.clone(),
            input.env.clone(),
            dir.clone(),
            limit,
            deadline,
            Arc::clone(permits),
            Arc::clone(stats),
        ));
    }
    let mut outputs: Vec<Vec<u8>> = Vec::new();
    let mut timed_out = false;
    while let Some(done) = children.join_next().await {
        match done {
            Ok(Run::Done(Some(output))) => outputs.push(output),
            Ok(Run::TimedOut) => timed_out = true,
            Ok(Run::Done(None)) | Err(_) => {}
        }
    }
    let error = if !outputs.is_empty() {
        None
    } else if timed_out {
        Some(OPENCODE_TIMED_OUT.to_string())
    } else {
        Some(OPENCODE_FAILED.to_string())
    };
    let sessions = outputs
        .iter()
        .flat_map(|o| parse_opencode_list(o))
        .collect();
    (recent(sessions, limit), error)
}

/// Counts a running child, and the most that ran together.
struct Running<'a>(&'a ListingStats);

impl<'a> Running<'a> {
    fn start(stats: &'a ListingStats) -> Self {
        let now = stats.running.fetch_add(1, Ordering::AcqRel) + 1;
        stats.max_children.fetch_max(now, Ordering::AcqRel);
        Self(stats)
    }
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.running.fetch_sub(1, Ordering::AcqRel);
    }
}

/// How one listing ended.
enum Run {
    /// The output of the command when it exited well and printed a JSON array.
    Done(Option<Vec<u8>>),
    /// The deadline came first; a child that was started is killed and reaped by [`Reaped`].
    TimedOut,
}

/// A listing's child. Dropped before it was waited for (the deadline came, or the request's
/// task was aborted with its connection), it is killed and reaped in a task of its own: a
/// dropped `Child` is killed but may stay a zombie until another child exits.
struct Reaped(Option<Child>);

impl Drop for Reaped {
    fn drop(&mut self) {
        let Some(mut child) = self.0.take() else {
            return;
        };
        let _ = child.start_kill();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = child.wait().await;
            });
        }
    }
}

/// One listing, stopped at `deadline`.
async fn run_opencode(
    program: String,
    env: Env,
    dir: PathBuf,
    limit: usize,
    deadline: tokio::time::Instant,
    permits: Arc<Semaphore>,
    stats: Arc<ListingStats>,
) -> Run {
    let Ok(permit) = tokio::time::timeout_at(deadline, permits.acquire_owned()).await else {
        return Run::TimedOut;
    };
    let Ok(_permit) = permit else {
        return Run::Done(None);
    };
    // A slot handed over right at the deadline starts nothing.
    if tokio::time::Instant::now() >= deadline {
        return Run::TimedOut;
    }
    let spawned = Command::new(&program)
        .args(["session", "list", "--max-count"])
        .arg(limit.to_string())
        .args(["--format", "json"])
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn();
    let Ok(child) = spawned else {
        return Run::Done(None);
    };
    let mut reaped = Reaped(Some(child));
    let _running = Running::start(&stats);
    let Some(child) = reaped.0.as_mut() else {
        return Run::Done(None);
    };
    let Some(stdout) = child.stdout.take() else {
        return Run::Done(None);
    };
    let work = async {
        let mut output = Vec::new();
        let mut stdout = stdout.take(OUTPUT_MAX);
        let read = stdout.read_to_end(&mut output).await;
        drop(stdout);
        let complete = read.is_ok() && (output.len() as u64) < OUTPUT_MAX;
        if !complete {
            let _ = child.start_kill();
        }
        let status = child.wait().await.ok();
        (output, complete, status)
    };
    let Ok((output, complete, status)) = tokio::time::timeout_at(deadline, work).await else {
        return Run::TimedOut;
    };
    // Waited for: nothing is left to reap.
    reaped.0 = None;
    let success = status.is_some_and(|s| s.success());
    let is_array = serde_json::from_slice::<Value>(&output).is_ok_and(|v| v.is_array());
    Run::Done((complete && success && is_array).then_some(output))
}
