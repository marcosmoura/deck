//! `Request::ListAgentSessions` and `PanelLaunch::Resume`. Every test has its own temp `HOME`,
//! `CLAUDE_CONFIG_DIR`, `CODEX_HOME` and `XDG_DATA_HOME` in the session's environment, so the
//! daemon never looks at the real ones; agents are fake `#!/bin/sh` scripts; no real agent runs.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use deck_core::action::{Action, ActionError, PanelLaunch};
use deck_core::agent_sessions::AgentSession;
use deck_core::agents::AgentKind;
use deck_core::protocol::{ClientMsg, Request, Response, ServerMsg};
use deck_daemon::{ListingStats, Options};
use support::{Attach, Client, TestDaemon};

type Listing = (Vec<AgentSession>, Option<String>);

/// A temp world: a home, the agents' directories, a `bin` for fake programs and project roots.
struct Sb {
    base: tempfile::TempDir,
}

impl Sb {
    fn new() -> Self {
        let base = tempfile::Builder::new()
            .prefix("ag")
            .tempdir_in("/tmp")
            .unwrap();
        let sb = Self { base };
        for d in ["home", "claude", "codex", "bin", "xdg"] {
            fs::create_dir_all(sb.p(d)).unwrap();
        }
        sb
    }

    /// A path in the world, canonical so `pwd -P` agrees with it.
    fn p(&self, rel: &str) -> PathBuf {
        self.base.path().canonicalize().unwrap().join(rel)
    }

    fn home(&self) -> PathBuf {
        self.p("home")
    }

    fn bin(&self) -> PathBuf {
        self.p("bin")
    }

    fn root(&self, name: &str) -> PathBuf {
        let root = self.p(&format!("roots/{name}"));
        fs::create_dir_all(&root).unwrap();
        root
    }

    /// The session's environment: this world's directories and `path` as the `PATH`.
    fn env(&self, path: &str) -> Vec<(OsString, OsString)> {
        let set = |k: &str, v: &Path| (OsString::from(k), v.as_os_str().to_os_string());
        vec![
            ("PS1".into(), "$ ".into()),
            ("PATH".into(), path.into()),
            set("HOME", &self.home()),
            set("CLAUDE_CONFIG_DIR", &self.p("claude")),
            set("CODEX_HOME", &self.p("codex")),
            set("XDG_DATA_HOME", &self.p("xdg")),
        ]
    }

    /// A `PATH` of the sandbox's `bin` alone: no real agent can be found, and the fakes call
    /// anything else by absolute path.
    fn path_with_system(&self) -> String {
        self.bin().display().to_string()
    }

    fn attach(&self, root: &Path, path: &str) -> Attach {
        Attach {
            cwd: root.to_path_buf(),
            env: self.env(path),
            ..Attach::default()
        }
    }
}

fn uuid(n: u32) -> String {
    format!("{n:08x}-1111-4222-8333-444455556666")
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn touch(path: &Path, secs: u64) {
    let file = fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
        .unwrap();
}

fn claude_line(id: &str, cwd: &str, prompt: &str) -> String {
    format!(
        "{{\"type\":\"user\",\"sessionId\":\"{id}\",\"cwd\":\"{cwd}\",\"isSidechain\":false,\"message\":{{\"role\":\"user\",\"content\":\"{prompt}\"}}}}\n"
    )
}

/// A Claude transcript at `projects/<dir>/<id>.jsonl`, modified at `secs`.
fn claude_file(sb: &Sb, dir: &str, n: u32, cwd: &str, prompt: &str, secs: u64) -> PathBuf {
    let path = sb.p(&format!("claude/projects/{dir}/{}.jsonl", uuid(n)));
    write(&path, &claude_line(&uuid(n), cwd, prompt));
    touch(&path, secs);
    path
}

fn codex_meta(id: &str, cwd: &str) -> String {
    format!(
        "{{\"timestamp\":\"2026-11-02T09:00:00Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"{cwd}\"}}}}\n"
    )
}

fn codex_user(prompt: &str) -> String {
    format!(
        "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":\"{prompt}\"}}}}\n"
    )
}

/// A Codex rollout in `sessions/<day>/`, modified at `secs`.
fn codex_file(sb: &Sb, day: &str, n: u32, cwd: &str, prompt: &str, secs: u64) -> PathBuf {
    let path = sb.p(&format!(
        "codex/sessions/{day}/rollout-2026-11-22T10-00-00-{}.jsonl",
        uuid(n)
    ));
    write(
        &path,
        &format!("{}{}", codex_meta(&uuid(n), cwd), codex_user(prompt)),
    );
    touch(&path, secs);
    path
}

fn script(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A fake `opencode` that appends `<HOME>|<XDG_DATA_HOME>|<cwd>` to `<bin>/opencode.rec` and
/// then runs `body`.
fn fake_opencode(sb: &Sb, body: &str) {
    let rec = sb.bin().join("opencode.rec");
    script(
        &sb.bin().join("opencode"),
        &format!(
            "#!/bin/sh\necho \"$$\" >> '{pids}'\necho \"$HOME|$XDG_DATA_HOME|$(pwd -P)|$*\" >> '{rec}'\n{body}\n",
            rec = rec.display(),
            pids = sb.bin().join("opencode.pids").display()
        ),
    );
}

/// The pids of the fake `opencode` runs so far (a run that `exec`s keeps its pid).
fn opencode_pids(sb: &Sb) -> Vec<i32> {
    fs::read_to_string(sb.bin().join("opencode.pids"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect()
}

/// Waits until none of `pids` exists any more.
async fn all_gone(pids: &[i32]) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while pids.iter().any(|p| support::sys::alive(*p)) {
        assert!(Instant::now() < deadline, "children still alive: {pids:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn opencode_runs(sb: &Sb) -> Vec<String> {
    fs::read_to_string(sb.bin().join("opencode.rec"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// A fake agent that records its directory, `DECK_PANEL_ID` and arguments (one per line).
fn fake_agent(sb: &Sb, name: &str) {
    let rec = sb.bin().join(format!("{name}.rec"));
    script(
        &sb.bin().join(name),
        &format!(
            "#!/bin/sh\nout=\"$(pwd -P)\n$DECK_PANEL_ID\"\nfor a; do out=\"$out\n$a\"; done\necho \"$out\nEND\" > '{}'\nread line\n",
            rec.display()
        ),
    );
}

async fn recorded(sb: &Sb, name: &str) -> Vec<String> {
    let rec = sb.bin().join(format!("{name}.rec"));
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        // The agent writes `END` last: without it the file is only partly written.
        if let Ok(text) = fs::read_to_string(&rec)
            && let Some(done) = text.trim_end().strip_suffix("\nEND")
        {
            return done.lines().map(str::to_string).collect();
        }
        assert!(Instant::now() < deadline, "{name} did not run");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn daemon_with_stats() -> (TestDaemon, Arc<ListingStats>) {
    let stats = Arc::new(ListingStats::default());
    let daemon = TestDaemon::start_with(
        support::config(),
        Options {
            listing: Arc::clone(&stats),
            ..Options::default()
        },
    )
    .await;
    (daemon, stats)
}

async fn attached(sb: &Sb, root: &Path, path: &str) -> (TestDaemon, Arc<ListingStats>, Client) {
    let (daemon, stats) = daemon_with_stats().await;
    let c = Client::attach(&daemon.paths.socket, sb.attach(root, path)).await;
    (daemon, stats, c)
}

/// The answer to the request for `kind` that was sent after `log[start]`.
async fn answer(c: &mut Client, kind: AgentKind, start: usize, within: Duration) -> Listing {
    let deadline = Instant::now() + within;
    loop {
        let found = c.log[start..].iter().find_map(|m| match m {
            ServerMsg::Response(Response::AgentSessions {
                kind: k,
                sessions,
                error,
            }) if *k == kind => Some((sessions.clone(), error.clone())),
            _ => None,
        });
        if let Some(found) = found {
            return found;
        }
        let left = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or_else(|| panic!("no {kind:?} answer in time"));
        if c.recv(left).await.is_none() {
            assert!(!c.eof, "the connection ended");
        }
    }
}

/// The first `count` answers for `kind` that came after `log[start]`.
async fn answers(c: &mut Client, kind: AgentKind, start: usize, count: usize) -> Vec<Listing> {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let found: Vec<Listing> = c.log[start..]
            .iter()
            .filter_map(|m| match m {
                ServerMsg::Response(Response::AgentSessions {
                    kind: k,
                    sessions,
                    error,
                }) if *k == kind => Some((sessions.clone(), error.clone())),
                _ => None,
            })
            .collect();
        if found.len() >= count {
            return found;
        }
        let left = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or_else(|| panic!("{} of {count} {kind:?} answers in time", found.len()));
        if c.recv(left).await.is_none() {
            assert!(!c.eof, "the connection ended");
        }
    }
}

async fn send_list(c: &mut Client, kind: AgentKind, limit: u16) -> usize {
    let start = c.log.len();
    c.send(&ClientMsg::Request(Request::ListAgentSessions {
        kind,
        limit,
    }))
    .await;
    start
}

async fn list(c: &mut Client, kind: AgentKind, limit: u16) -> Listing {
    let start = send_list(c, kind, limit).await;
    answer(c, kind, start, Duration::from_secs(12)).await
}

fn ids(sessions: &[AgentSession]) -> Vec<String> {
    sessions.iter().map(|s| s.id.clone()).collect()
}

fn no_errors_in_log(c: &Client) {
    assert!(
        !c.log
            .iter()
            .any(|m| matches!(m, ServerMsg::Response(Response::Error(_)))),
        "a listing never answers with Response::Error"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn claude_sessions_are_listed_newest_first_and_a_touched_one_moves_up() {
    let sb = Sb::new();
    claude_file(&sb, "-w-api", 1, "/w/api", "Oldest prompt", 1_000_000);
    let middle = claude_file(&sb, "-w-api", 2, "/w/api", "Middle prompt", 2_000_000);
    claude_file(&sb, "-w-web", 3, "/w/web", "Newest prompt", 3_000_000);
    let root = sb.root("p");
    let (daemon, _stats, mut c) = attached(&sb, &root, "/bin").await;

    let (sessions, error) = list(&mut c, AgentKind::Claude, 50).await;
    assert_eq!(error, None);
    assert_eq!(ids(&sessions), [uuid(3), uuid(2), uuid(1)]);
    assert_eq!(sessions[0].title, "Newest prompt");
    assert_eq!(sessions[0].cwd, PathBuf::from("/w/web"));
    assert_eq!(sessions[0].kind, AgentKind::Claude);
    assert_eq!(sessions[0].updated, 3_000_000);
    assert_eq!(sessions[2].title, "Oldest prompt");
    assert_eq!(sessions[2].cwd, PathBuf::from("/w/api"));

    touch(&middle, 4_000_000);
    let (sessions, _) = list(&mut c, AgentKind::Claude, 50).await;
    assert_eq!(ids(&sessions), [uuid(2), uuid(3), uuid(1)]);
    assert_eq!(sessions[0].updated, 4_000_000);

    // `limit` is clamped to 1..=100 and cuts the list.
    let (sessions, _) = list(&mut c, AgentKind::Claude, 1).await;
    assert_eq!(ids(&sessions), [uuid(2)]);
    let (sessions, _) = list(&mut c, AgentKind::Claude, 0).await;
    assert_eq!(sessions.len(), 1, "0 counts as 1");
    no_errors_in_log(&c);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn claude_candidates_leave_out_agent_files_and_nested_directories() {
    let sb = Sb::new();
    claude_file(&sb, "-w-api", 1, "/w/api", "Real", 1_000_000);
    // The newest of all, but a subagent's transcript, and one in a subagent directory.
    let agent = sb.p("claude/projects/-w-api/agent-abcd1234.jsonl");
    write(&agent, &claude_line(&uuid(8), "/w/api", "Subagent"));
    touch(&agent, 9_000_000);
    let nested = sb.p(&format!(
        "claude/projects/-w-api/{}/subagents/{}.jsonl",
        uuid(1),
        uuid(9)
    ));
    write(&nested, &claude_line(&uuid(9), "/w/api", "Nested"));
    touch(&nested, 9_000_000);
    let (daemon, _stats, mut c) = attached(&sb, &sb.root("p"), "/bin").await;

    let (sessions, error) = list(&mut c, AgentKind::Claude, 50).await;
    assert_eq!(error, None);
    assert_eq!(ids(&sessions), [uuid(1)]);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn codex_sessions_are_newest_first_and_old_day_directories_are_not_read() {
    let sb = Sb::new();
    // Names and modification times agree: the 22nd is newest.
    codex_file(&sb, "2026/11/22", 1, "/w/a", "Day 22", 3_000_000);
    codex_file(&sb, "2026/11/21", 2, "/w/b", "Day 21 b", 2_000_100);
    codex_file(&sb, "2026/11/21", 3, "/w/c", "Day 21 c", 2_000_200);
    for n in 4..9 {
        codex_file(
            &sb,
            "2026/11/20",
            n,
            "/w/d",
            "Day 20",
            1_000_000 + u64::from(n),
        );
    }
    let (daemon, stats, mut c) = attached(&sb, &sb.root("p"), "/bin").await;

    // limit 1: two candidates, taken from the two newest days only.
    let (sessions, error) = list(&mut c, AgentKind::Codex, 1).await;
    assert_eq!(error, None);
    assert_eq!(ids(&sessions), [uuid(1)]);
    assert_eq!(
        stats.files_opened.load(Ordering::Relaxed),
        2,
        "the 20th was never opened"
    );

    // limit 50: everything, newest first.
    let (sessions, _) = list(&mut c, AgentKind::Codex, 50).await;
    assert_eq!(sessions.len(), 8);
    assert_eq!(sessions[0].title, "Day 22");
    assert_eq!(sessions[0].cwd, PathBuf::from("/w/a"));
    assert_eq!(sessions[0].kind, AgentKind::Codex);
    assert_eq!(ids(&sessions[..3]), [uuid(1), uuid(3), uuid(2)]);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_directories_come_from_the_sessions_environment() {
    let a = Sb::new();
    let b = Sb::new();
    codex_file(&a, "2026/11/22", 1, "/w/a", "In A", 1_000_000);
    codex_file(&b, "2026/11/22", 2, "/w/b", "In B", 1_000_000);
    // `b` has no `CODEX_HOME`: its `HOME` is searched instead.
    fs::rename(b.p("codex/sessions"), {
        fs::create_dir_all(b.home().join(".codex")).unwrap();
        b.home().join(".codex/sessions")
    })
    .unwrap();
    let (daemon, _stats) = daemon_with_stats().await;
    let mut ca = Client::attach(&daemon.paths.socket, a.attach(&a.root("p"), "/bin")).await;
    let mut env_b = b.env("/bin");
    env_b.retain(|(k, _)| k != "CODEX_HOME");
    let mut cb = Client::attach(
        &daemon.paths.socket,
        Attach {
            session: Some("other".into()),
            cwd: b.root("p"),
            env: env_b,
            ..Attach::default()
        },
    )
    .await;

    let (in_a, _) = list(&mut ca, AgentKind::Codex, 50).await;
    let (in_b, _) = list(&mut cb, AgentKind::Codex, 50).await;
    assert_eq!(ids(&in_a), [uuid(1)]);
    assert_eq!(ids(&in_b), [uuid(2)]);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_without_home_or_agent_directories_lists_nothing() {
    let sb = Sb::new();
    claude_file(&sb, "-w-api", 1, "/w/api", "Not for you", 1_000_000);
    let (daemon, _stats) = daemon_with_stats().await;
    let mut c = Client::attach(
        &daemon.paths.socket,
        Attach {
            cwd: sb.root("p"),
            env: vec![("PS1".into(), "$ ".into())],
            ..Attach::default()
        },
    )
    .await;
    for kind in [AgentKind::Claude, AgentKind::Codex] {
        assert_eq!(list(&mut c, kind, 50).await, (Vec::new(), None), "{kind:?}");
    }
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_roots_give_empty_lists_without_an_error() {
    let sb = Sb::new();
    fs::remove_dir_all(sb.p("claude")).unwrap();
    fs::remove_dir_all(sb.p("codex")).unwrap();
    let (daemon, stats, mut c) = attached(&sb, &sb.root("p"), "/bin").await;
    for kind in [AgentKind::Claude, AgentKind::Codex] {
        assert_eq!(list(&mut c, kind, 50).await, (Vec::new(), None), "{kind:?}");
    }
    assert_eq!(stats.files_opened.load(Ordering::Relaxed), 0);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_claude_transcript_is_read_only_up_to_256_kib() {
    let sb = Sb::new();
    let path = claude_file(&sb, "-w-api", 1, "/w/api", "Big one", 1_000_000);
    let mut text = fs::read_to_string(&path).unwrap();
    // A single 10 MiB line after the first one.
    text.push_str(&"x".repeat(10 * 1024 * 1024));
    fs::write(&path, text).unwrap();
    let (daemon, stats, mut c) = attached(&sb, &sb.root("p"), "/bin").await;

    let (sessions, _) = list(&mut c, AgentKind::Claude, 50).await;
    assert_eq!(ids(&sessions), [uuid(1)], "the head has the title");
    assert_eq!(stats.bytes_read.load(Ordering::Relaxed), 256 * 1024);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_codex_rollout_is_read_line_by_line_up_to_its_first_prompt_or_4_mib() {
    let sb = Sb::new();
    let dir = "codex/sessions/2026/11/22";
    let name = |n: u32| {
        sb.p(&format!(
            "{dir}/rollout-2026-11-22T10-00-00-{}.jsonl",
            uuid(n)
        ))
    };

    // 1. Large instructions in the first line and response items before the prompt, more than
    //    a fixed 256 KiB head holds: still listed.
    let mut late = codex_meta(&uuid(1), "/w/late")
        .trim_end()
        .trim_end_matches("}}")
        .to_string();
    writeln!(late, ",\"instructions\":\"{}\"}}}}", "i".repeat(600 * 1024)).unwrap();
    for _ in 0..300 {
        writeln!(
            late,
            "{{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"text\":\"{}\"}}}}",
            "r".repeat(1024)
        )
        .unwrap();
    }
    late.push_str(&codex_user("Late prompt"));
    late.push_str(&"z".repeat(10 * 1024 * 1024));
    write(&name(1), &late);
    touch(&name(1), 3_000_000);

    // 2. The prompt is in the second line: the 10 MiB after it are not read.
    let mut early = format!(
        "{}{}",
        codex_meta(&uuid(2), "/w/early"),
        codex_user("Early")
    );
    early.push_str(&"z".repeat(10 * 1024 * 1024));
    write(&name(2), &early);
    touch(&name(2), 2_000_000);

    // 3. No prompt at all: 10 MiB of lines, read up to 4 MiB, not listed.
    let mut none = codex_meta(&uuid(3), "/w/none");
    for _ in 0..(10 * 1024) {
        writeln!(
            none,
            "{{\"type\":\"response_item\",\"x\":\"{}\"}}",
            "n".repeat(1000)
        )
        .unwrap();
    }
    write(&name(3), &none);
    touch(&name(3), 1_000_000);

    let (daemon, stats, mut c) = attached(&sb, &sb.root("p"), "/bin").await;

    let (sessions, error) = list(&mut c, AgentKind::Codex, 1).await;
    assert_eq!(error, None);
    // limit 1 reads two candidates (names descending: 3 then 2); read only the third file.
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, uuid(2));
    let read_two_and_three = stats.bytes_read.load(Ordering::Relaxed);
    assert!(
        (4 * 1024 * 1024..4 * 1024 * 1024 + 4096).contains(&read_two_and_three),
        "the third file stopped at 4 MiB and the second right after its prompt: {read_two_and_three}"
    );

    stats.bytes_read.store(0, Ordering::Relaxed);
    let (sessions, _) = list(&mut c, AgentKind::Codex, 50).await;
    assert_eq!(ids(&sessions), [uuid(1), uuid(2)]);
    assert_eq!(sessions[0].title, "Late prompt");
    assert_eq!(sessions[0].cwd, PathBuf::from("/w/late"));
    let total = stats.bytes_read.load(Ordering::Relaxed);
    assert!(
        total < 3 * 4 * 1024 * 1024 + 1024 * 1024,
        "no file is read past 4 MiB: {total}"
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn opencode_runs_with_the_sessions_environment_and_path() {
    let sb = Sb::new();
    // Only in the session's `bin`; the daemon's own PATH does not have it.
    fake_opencode(
        &sb,
        "printf '[{\"id\":\"ses_one\",\"title\":\"From the fake\",\"updated\":1766000000,\"directory\":\"/w/o\"}]'",
    );
    let root = sb.root("p");
    let (daemon, _stats, mut c) = attached(&sb, &root, &sb.path_with_system()).await;

    let (sessions, error) = list(&mut c, AgentKind::OpenCode, 7).await;
    assert_eq!(error, None);
    assert_eq!(ids(&sessions), ["ses_one"]);
    assert_eq!(sessions[0].kind, AgentKind::OpenCode);
    assert_eq!(sessions[0].title, "From the fake");

    let runs = opencode_runs(&sb);
    let want = |dir: &Path| {
        format!(
            "{}|{}|{}|session list --max-count 7 --format json",
            sb.home().display(),
            sb.p("xdg").display(),
            dir.display()
        )
    };
    assert_eq!(runs, [want(&sb.home())], "one run, in the home directory");
    daemon.stop().await.unwrap();
}

/// Security review, Q-4: OpenCode runs the plugins and MCP servers of its working directory,
/// so the listing never runs in a space root.
#[tokio::test(flavor = "multi_thread")]
async fn opencode_runs_only_in_the_home_directory_never_in_a_space_root() {
    let sb = Sb::new();
    let (r1, r2) = (sb.root("one"), sb.root("two"));
    let home = sb.home();
    let item = |id: &str, updated: u64| {
        format!(
            "{{\"id\":\"{id}\",\"title\":\"T {id}\",\"updated\":{updated},\"directory\":\"/w\"}}"
        )
    };
    // A different array in each directory: only the home's may be shown.
    fake_opencode(
        &sb,
        &format!(
            "case \"$(pwd -P)\" in\n'{r1}') printf '[{a}]';;\n'{r2}') printf '[{b}]';;\n'{home}') printf '[{c},{d}]';;\n*) printf 'nothing here';;\nesac",
            r1 = r1.display(),
            r2 = r2.display(),
            home = home.display(),
            a = item("ses_a", 1_766_000_009),
            b = item("ses_b", 1_766_000_008),
            c = item("ses_c", 1_766_000_001),
            d = item("ses_d", 1_766_000_002),
        ),
    );
    // The session was created by a shell in a space root: its `PWD` and `OLDPWD` say so. A
    // `/bin/sh` fake cannot show what it gets of them (a shell resets `PWD` to its real
    // directory when it starts); a unit test of `listing.rs` reads the command for that.
    let mut env = sb.env(&sb.path_with_system());
    env.push(("PWD".into(), r1.clone().into()));
    env.push(("OLDPWD".into(), r2.clone().into()));
    let (daemon, _stats) = daemon_with_stats().await;
    let mut c = Client::attach(
        &daemon.paths.socket,
        Attach {
            cwd: r1.clone(),
            env,
            ..Attach::default()
        },
    )
    .await;
    c.action(Action::NewProject {
        name: "two".into(),
        root: r2.clone(),
    })
    .await
    .unwrap();

    let (sessions, error) = list(&mut c, AgentKind::OpenCode, 50).await;
    assert_eq!(error, None);
    assert_eq!(
        ids(&sessions),
        ["ses_d", "ses_c"],
        "the home's, newest first"
    );
    let dirs: Vec<String> = opencode_runs(&sb)
        .iter()
        .filter_map(|l| l.split('|').nth(2).map(str::to_string))
        .collect();
    assert_eq!(
        dirs,
        [home.display().to_string()],
        "one run, in the home directory, and none in a space root"
    );
    daemon.stop().await.unwrap();
}

/// A session whose `HOME` is missing, empty or relative has no directory to list in: a
/// relative one would name a directory under the daemon's own.
#[tokio::test(flavor = "multi_thread")]
async fn opencode_does_not_run_without_an_absolute_home() {
    let sb = Sb::new();
    fake_opencode(
        &sb,
        "printf '[{\"id\":\"ses_one\",\"title\":\"T\",\"updated\":1766000000,\"directory\":\"/w\"}]'",
    );
    let (daemon, _stats) = daemon_with_stats().await;
    let path = sb.path_with_system();
    for (session, home) in [
        ("missing", None),
        ("empty", Some("")),
        ("relative", Some(".")),
    ] {
        let mut env = sb.env(&path);
        env.retain(|(k, _)| k != "HOME");
        if let Some(home) = home {
            env.push(("HOME".into(), home.into()));
        }
        let mut c = Client::attach(
            &daemon.paths.socket,
            Attach {
                session: Some(session.into()),
                cwd: sb.root(session),
                env,
                ..Attach::default()
            },
        )
        .await;
        assert_eq!(
            list(&mut c, AgentKind::OpenCode, 50).await,
            (Vec::new(), None),
            "{session}: an empty answer"
        );
    }
    assert_eq!(opencode_runs(&sb), Vec::<String>::new(), "no run at all");
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn at_most_two_opencode_children_run_at_once() {
    let sb = Sb::new();
    fake_opencode(&sb, "/bin/sleep 0.3\nprintf '[]'");
    let (daemon, stats, mut c) = attached(&sb, &sb.root("r0"), &sb.path_with_system()).await;

    // One child per request: several requests at once still run two at a time.
    let start = send_list(&mut c, AgentKind::OpenCode, 50).await;
    for _ in 1..7 {
        send_list(&mut c, AgentKind::OpenCode, 50).await;
    }
    let answers = answers(&mut c, AgentKind::OpenCode, start, 7).await;
    assert!(
        answers.iter().all(|(s, e)| s.is_empty() && e.is_none()),
        "empty arrays are fine: {answers:?}"
    );
    assert_eq!(opencode_runs(&sb).len(), 7, "one run per request");
    assert_eq!(stats.max_children.load(Ordering::Relaxed), 2);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_slow_opencode_never_blocks_the_connection_and_times_out_after_8_seconds() {
    let sb = Sb::new();
    claude_file(&sb, "-w-api", 1, "/w/api", "Claude one", 1_000_000);
    codex_file(&sb, "2026/11/22", 2, "/w/a", "Codex one", 1_000_000);
    fake_opencode(&sb, "exec /bin/sleep 30");
    let (daemon, _stats, mut c) = attached(&sb, &sb.root("p"), &sb.path_with_system()).await;

    let begun = Instant::now();
    let pending = send_list(&mut c, AgentKind::OpenCode, 50).await;

    // An input and an action on the same connection are answered at once.
    c.type_line("echo MARK$((20+22))").await;
    c.wait_text("MARK42").await;
    c.action(Action::RenameSession {
        name: "renamed".into(),
    })
    .await
    .unwrap();
    assert!(begun.elapsed() < Duration::from_secs(4));

    // The other two agents answer before it.
    let (claude, _) = list(&mut c, AgentKind::Claude, 50).await;
    let (codex, _) = list(&mut c, AgentKind::Codex, 50).await;
    assert_eq!((claude.len(), codex.len()), (1, 1));
    assert!(begun.elapsed() < Duration::from_secs(7));
    assert!(
        !c.log[pending..].iter().any(|m| matches!(
            m,
            ServerMsg::Response(Response::AgentSessions {
                kind: AgentKind::OpenCode,
                ..
            })
        )),
        "OpenCode is still pending"
    );

    let (sessions, error) = answer(
        &mut c,
        AgentKind::OpenCode,
        pending,
        Duration::from_secs(12),
    )
    .await;
    let took = begun.elapsed();
    assert!(sessions.is_empty());
    assert_eq!(error.as_deref(), Some("timed out"));
    let pids = opencode_pids(&sb);
    assert!(!pids.is_empty());
    all_gone(&pids).await;
    assert!(
        (Duration::from_secs(7)..Duration::from_secs(11)).contains(&took),
        "about 8 s: {took:?}"
    );
    no_errors_in_log(&c);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn garbage_or_a_missing_opencode_gives_an_empty_list_and_the_error_text() {
    let sb = Sb::new();
    let (daemon, _stats, mut c) = attached(&sb, &sb.root("p"), &sb.path_with_system()).await;

    // No `opencode` anywhere on the session's PATH.
    let (sessions, error) = list(&mut c, AgentKind::OpenCode, 50).await;
    assert!(sessions.is_empty());
    assert_eq!(
        error.as_deref(),
        Some("could not run opencode session list")
    );

    for body in [
        "printf 'not json at all'",
        "printf '{\"a\":1}'",
        "printf '[]'; exit 3",
    ] {
        fake_opencode(&sb, body);
        let (sessions, error) = list(&mut c, AgentKind::OpenCode, 50).await;
        assert!(sessions.is_empty(), "{body}");
        assert_eq!(
            error.as_deref(),
            Some("could not run opencode session list"),
            "{body}"
        );
    }
    no_errors_in_log(&c);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_list_request_from_a_control_client_returns_no_session() {
    let daemon = TestDaemon::start().await;
    let mut c = Client::control(&daemon.paths.socket).await;
    let response = c
        .request(Request::ListAgentSessions {
            kind: AgentKind::Claude,
            limit: 50,
        })
        .await;
    assert_eq!(response, Response::Error("no session".into()));
    daemon.stop().await.unwrap();
}

fn panel_count(c: &Client) -> usize {
    c.view
        .as_ref()
        .unwrap()
        .projects
        .iter()
        .map(|p| p.panels.len())
        .sum()
}

async fn resume(c: &mut Client, kind: AgentKind, id: &str, cwd: &Path) -> Result<(), ActionError> {
    let project = c.view.as_ref().unwrap().projects[0].id;
    c.action(Action::NewPanelWith {
        project,
        launch: PanelLaunch::Resume {
            kind,
            session_id: id.into(),
            cwd: cwd.to_path_buf(),
        },
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn resume_runs_the_template_with_the_id_in_the_sessions_directory() {
    let sb = Sb::new();
    fake_agent(&sb, "claude");
    fake_agent(&sb, "codex");
    let root = sb.root("p");
    let dir = sb.root("elsewhere");
    let (daemon, _stats, mut c) = attached(&sb, &root, &sb.bin().display().to_string()).await;

    resume(&mut c, AgentKind::Claude, &uuid(5), &dir)
        .await
        .unwrap();
    c.wait(|c| panel_count(c) == 2).await;
    let view = c.view.clone().unwrap();
    let panel = &view.projects[0].panels[1];
    assert_eq!(view.active_panel, Some(panel.id));
    assert_eq!(panel.terminals[0].program, "claude");
    let lines = recorded(&sb, "claude").await;
    assert_eq!(lines[0], dir.display().to_string());
    assert_eq!(lines[1], panel.id.to_string());
    assert_eq!(lines[2..], ["--resume".to_string(), uuid(5)]);
    assert!(
        c.notices().is_empty(),
        "no warning when the directory exists"
    );

    resume(&mut c, AgentKind::Codex, "ses_abc.1", &dir)
        .await
        .unwrap();
    let lines = recorded(&sb, "codex").await;
    assert_eq!(lines[2..], ["resume".to_string(), "ses_abc.1".to_string()]);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn resume_falls_back_to_the_project_root_with_a_warning() {
    let sb = Sb::new();
    fake_agent(&sb, "claude");
    let root = sb.root("p");
    let gone = sb.p("roots/gone");
    let (daemon, _stats, mut c) = attached(&sb, &root, &sb.bin().display().to_string()).await;

    resume(&mut c, AgentKind::Claude, &uuid(5), &gone)
        .await
        .unwrap();
    let lines = recorded(&sb, "claude").await;
    assert_eq!(lines[0], root.display().to_string());
    assert_eq!(lines[2..], ["--resume".to_string(), uuid(5)]);
    let text = format!(
        "{} no longer exists; started in {}",
        gone.display(),
        root.display()
    );
    c.wait(|c| c.has_notice(&text)).await;
    assert!(
        c.notices()
            .iter()
            .any(|(level, t)| *level == deck_core::protocol::NoticeLevel::Warning && *t == text)
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invalid_session_id_or_a_missing_program_creates_no_panel() {
    let sb = Sb::new();
    let root = sb.root("p");
    // Nothing on the PATH: not even a `claude`.
    let (daemon, _stats, mut c) = attached(&sb, &root, &sb.bin().display().to_string()).await;
    let before = panel_count(&c);

    for id in ["-p", "x; ls", "", "a b", "--resume"] {
        assert_eq!(
            resume(&mut c, AgentKind::Claude, id, &root).await,
            Err(ActionError::InvalidSessionId),
            "{id:?}"
        );
    }
    assert_eq!(panel_count(&c), before);

    match resume(&mut c, AgentKind::Claude, &uuid(5), &root).await {
        Err(ActionError::ProgramNotFound { program, kind, key }) => {
            assert_eq!(program, "claude");
            assert_eq!(kind, AgentKind::Claude);
            assert_eq!(key, "resume");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(panel_count(&c), before);
    daemon.stop().await.unwrap();
}

/// Everything the daemon logs while the tests of this binary run.
fn captured() -> &'static Arc<Mutex<String>> {
    static LOG: OnceLock<Arc<Mutex<String>>> = OnceLock::new();
    LOG.get_or_init(|| {
        let log = Arc::new(Mutex::new(String::new()));
        let _ = tracing::subscriber::set_global_default(Capture(Arc::clone(&log)));
        log
    })
}

struct Capture(Arc<Mutex<String>>);

struct Fields<'a>(&'a mut String);

impl tracing::field::Visit for Fields<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let _ = write!(self.0, "{}={value:?} ", field.name());
    }
}

impl tracing::Subscriber for Capture {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        let mut line = String::new();
        event.record(&mut Fields(&mut line));
        if let Ok(mut log) = self.0.lock() {
            log.push_str(&line);
            log.push('\n');
        }
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

#[tokio::test(flavor = "multi_thread")]
async fn the_daemon_log_has_no_titles_or_directories() {
    let log = captured();
    let sb = Sb::new();
    claude_file(
        &sb,
        "-q-secret",
        1,
        "/q-secret-dir-7731",
        "Claude title 7731",
        1_000_000,
    );
    codex_file(
        &sb,
        "2026/11/22",
        2,
        "/q-secret-dir-7731",
        "Codex title 7731",
        1_000_000,
    );
    fake_opencode(
        &sb,
        "printf '[{\"id\":\"ses_s\",\"title\":\"OpenCode title 7731\",\"updated\":1766000000,\"directory\":\"/q-secret-dir-7731\"}]'",
    );
    fake_agent(&sb, "claude");
    let path = sb.bin().display().to_string();
    let (daemon, _stats, mut c) = attached(&sb, &sb.root("p"), &path).await;

    let mut shown = 0;
    for kind in [AgentKind::Claude, AgentKind::Codex, AgentKind::OpenCode] {
        let (sessions, error) = list(&mut c, kind, 50).await;
        assert_eq!(error, None);
        shown += sessions.iter().filter(|s| s.title.contains("7731")).count();
    }
    assert_eq!(shown, 3, "the client does get the titles");

    // A resume whose directory is gone: the notice is sent, not logged.
    let gone = sb.p("roots/gone-q-secret-dir-7731");
    resume(&mut c, AgentKind::Claude, &uuid(1), &gone)
        .await
        .unwrap();
    c.wait(|c| c.has_notice("no longer exists")).await;

    let text = log.lock().unwrap().clone();
    assert!(
        text.contains("listed agent sessions"),
        "the log is captured: {text}"
    );
    assert!(
        !text.contains("7731"),
        "no title or directory in the log:\n{text}"
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_the_connection_during_a_listing_kills_its_children() {
    let sb = Sb::new();
    fake_opencode(&sb, "exec /bin/sleep 30");
    let (daemon, _stats, mut c) = attached(&sb, &sb.root("p"), &sb.path_with_system()).await;
    send_list(&mut c, AgentKind::OpenCode, 50).await;
    let deadline = Instant::now() + Duration::from_secs(5);
    while opencode_pids(&sb).is_empty() {
        assert!(Instant::now() < deadline, "the child did not start");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let pids = opencode_pids(&sb);
    assert!(pids.iter().all(|p| support::sys::alive(*p)));
    drop(c);
    all_gone(&pids).await;
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_two_children_limit_holds_across_concurrent_requests() {
    let sb = Sb::new();
    fake_opencode(&sb, "/bin/sleep 0.3\nprintf '[]'");
    let (daemon, stats) = daemon_with_stats().await;
    let path = sb.path_with_system();
    let mut a = Client::attach(&daemon.paths.socket, sb.attach(&sb.root("a"), &path)).await;
    let mut b = Client::attach(
        &daemon.paths.socket,
        Attach {
            session: Some("other".into()),
            ..sb.attach(&sb.root("b"), &path)
        },
    )
    .await;
    // Two requests from each client: four children in all, one per request.
    let (sa, sb_start) = (
        send_list(&mut a, AgentKind::OpenCode, 50).await,
        send_list(&mut b, AgentKind::OpenCode, 50).await,
    );
    send_list(&mut a, AgentKind::OpenCode, 50).await;
    send_list(&mut b, AgentKind::OpenCode, 50).await;
    answers(&mut a, AgentKind::OpenCode, sa, 2).await;
    answers(&mut b, AgentKind::OpenCode, sb_start, 2).await;
    assert_eq!(opencode_runs(&sb).len(), 4);
    assert_eq!(stats.max_children.load(Ordering::Relaxed), 2);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_missing_directory_notice_goes_only_to_the_client_that_asked() {
    let sb = Sb::new();
    fake_agent(&sb, "claude");
    let root = sb.root("p");
    let path = sb.bin().display().to_string();
    let (daemon, _stats) = daemon_with_stats().await;
    let mut a = Client::attach(&daemon.paths.socket, sb.attach(&root, &path)).await;
    let mut other = Client::attach(
        &daemon.paths.socket,
        Attach {
            session: Some("other".into()),
            ..sb.attach(&sb.root("q"), &path)
        },
    )
    .await;

    let gone = sb.p("roots/gone");
    resume(&mut a, AgentKind::Claude, &uuid(5), &gone)
        .await
        .unwrap();
    a.wait(|c| c.has_notice("no longer exists")).await;

    // Nobody else hears of it, now or on a later attach.
    while other.recv(Duration::from_millis(500)).await.is_some() {}
    assert!(other.notices().is_empty(), "{:?}", other.notices());
    let mut late = Client::attach(
        &daemon.paths.socket,
        Attach {
            session: Some("late".into()),
            ..sb.attach(&sb.root("r"), &path)
        },
    )
    .await;
    while late.recv(Duration::from_millis(500)).await.is_some() {}
    assert!(late.notices().is_empty(), "{:?}", late.notices());
    daemon.stop().await.unwrap();
}
