//! The commands that list and edit deck's tree from a shell: `ls --json`/`--tree`,
//! `new-project`, `new-panel`, `new-terminal`, `focus`, `close` and `rename-panel`.
//!
//! They never start a daemon. Without a target option they act "here": the session, panel
//! and terminal named by the `DECK_*` variables of the terminal they run in.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, anyhow, bail};
use clap::Args;
use deck_core::model::{
    LaunchSpec, NameSource, PanelId, PanelView, ProjectView, SessionId, SessionView, TerminalView,
};
use deck_core::protocol::{Request, Response, Target};
use serde_json::{Value, json};

use crate::control::{
    Conn, Outcome, REPLY_TIMEOUT, checked_paths, control_socket, ls, mismatch_text,
};

/// The options every command here takes.
#[derive(Debug, Clone, Args)]
pub struct Common {
    /// Print one JSON document instead of text.
    #[arg(long)]
    pub json: bool,
    /// The session to act on (default: the one this terminal is in, else the most recently
    /// attached).
    #[arg(long, value_name = "NAME")]
    pub session: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct LsArgs {
    #[command(flatten)]
    pub common: Common,
    /// Print the tree as indented text.
    #[arg(long, conflicts_with = "json")]
    pub tree: bool,
}

#[derive(Debug, Clone, Args)]
pub struct NewProjectArgs {
    #[command(flatten)]
    pub common: Common,
    /// The project's name.
    pub name: String,
    /// The project's root (default: the current directory).
    #[arg(long, value_name = "PATH")]
    pub dir: Option<PathBuf>,
    /// Make the new project the active one.
    #[arg(long)]
    pub focus: bool,
}

#[derive(Debug, Clone, Args)]
pub struct NewPanelArgs {
    #[command(flatten)]
    pub common: Common,
    /// The project, by id or name (default: this terminal's).
    #[arg(long, value_name = "ID|NAME")]
    pub project: Option<String>,
    /// The panel's name.
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,
    /// The first terminal's directory (default: the project's root).
    #[arg(long, value_name = "DIR")]
    pub cwd: Option<PathBuf>,
    /// Make the new panel the active one.
    #[arg(long)]
    pub focus: bool,
    /// The program to run instead of a shell, and its arguments.
    #[arg(last = true, value_name = "PROGRAM")]
    pub program: Vec<String>,
}

#[derive(Debug, Clone, Args)]
pub struct NewTerminalArgs {
    #[command(flatten)]
    pub common: Common,
    /// The panel (default: this terminal's).
    #[arg(long, value_name = "ID")]
    pub panel: Option<u64>,
    /// The directory (default: where the panel's focused terminal is).
    #[arg(long, value_name = "DIR")]
    pub cwd: Option<PathBuf>,
    /// An environment variable for the program; repeatable.
    #[arg(long, value_name = "KEY=VALUE", value_parser = parse_env)]
    pub env: Vec<(String, String)>,
    /// Keep the terminal after its program exits, showing the exit code.
    #[arg(long)]
    pub keep: bool,
    /// Focus the new terminal.
    #[arg(long)]
    pub focus: bool,
    /// The program to run instead of a shell, and its arguments.
    #[arg(last = true, value_name = "PROGRAM")]
    pub program: Vec<String>,
}

#[derive(Debug, Clone, Args)]
pub struct IdArgs {
    #[command(flatten)]
    pub common: Common,
    /// The id of a terminal, panel or project (as `ls --json` shows).
    pub id: u64,
}

#[derive(Debug, Clone, Args)]
pub struct RenamePanelArgs {
    #[command(flatten)]
    pub common: Common,
    /// The panel (default: this terminal's).
    #[arg(long, value_name = "ID")]
    pub panel: Option<u64>,
    /// The new name.
    pub name: String,
}

fn parse_env(text: &str) -> Result<(String, String), String> {
    match text.split_once('=') {
        Some((key, value)) if !key.is_empty() => Ok((key.to_string(), value.to_string())),
        _ => Err("expected KEY=VALUE".into()),
    }
}

/// Where a command runs, from the `DECK_*` variables.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Here {
    pub session: Option<u64>,
    pub panel: Option<u64>,
    pub terminal: Option<u64>,
}

impl Here {
    pub fn from_env() -> Self {
        let get = |name: &str| std::env::var(name).ok().and_then(|v| v.parse().ok());
        Self {
            session: get("DECK_SESSION_ID"),
            panel: get("DECK_PANEL_ID"),
            terminal: get("DECK_TERMINAL_ID"),
        }
    }
}

fn has_panel(tree: &[SessionView], id: u64) -> bool {
    tree.iter()
        .flat_map(|s| &s.projects)
        .flat_map(|p| &p.panels)
        .any(|p| p.id.0 == id)
}

fn has_terminal(tree: &[SessionView], id: u64) -> bool {
    tree.iter()
        .flat_map(|s| &s.projects)
        .flat_map(|p| &p.panels)
        .flat_map(|p| &p.terminals)
        .any(|t| t.id.0 == id)
}

/// The session a command acts on: `--session`, else this terminal's, else the most recently
/// attached (`recent`), else the only one.
pub fn pick_session<'a>(
    tree: &'a [SessionView],
    explicit: Option<&str>,
    here: Here,
    recent: impl FnOnce() -> anyhow::Result<Option<SessionId>>,
) -> anyhow::Result<&'a SessionView> {
    if let Some(name) = explicit {
        return tree
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| anyhow!("no session named \"{name}\""));
    }
    if let Some(id) = here.session {
        return tree
            .iter()
            .find(|s| s.id.0 == id)
            .ok_or_else(|| match here {
                Here {
                    terminal: Some(t), ..
                } => anyhow!("this terminal ({t}) no longer exists"),
                _ => anyhow!("this session ({id}) no longer exists"),
            });
    }
    match tree {
        [] => bail!("no session"),
        [only] => Ok(only),
        _ => {
            let id = recent()?;
            id.and_then(|id| tree.iter().find(|s| s.id == id))
                .ok_or_else(|| anyhow!("several sessions and none attached; pass --session"))
        }
    }
}

/// An id from the environment must still exist; a stale one is an error, never replaced.
pub(crate) fn check_here(tree: &[SessionView], here: Here) -> anyhow::Result<()> {
    if let Some(p) = here.panel
        && !has_panel(tree, p)
    {
        bail!("this panel ({p}) no longer exists");
    }
    if let Some(t) = here.terminal
        && !has_terminal(tree, t)
    {
        bail!("this terminal ({t}) no longer exists");
    }
    Ok(())
}

/// The panel a command acts on: `--panel`, else this terminal's panel (unless `--session`
/// turned the environment off), else the session's active panel.
pub fn pick_panel(
    tree: &[SessionView],
    session: &SessionView,
    explicit: Option<u64>,
    here: Option<Here>,
) -> anyhow::Result<PanelId> {
    if let Some(id) = explicit {
        return Ok(PanelId(id));
    }
    if let Some(here) = here {
        check_here(tree, here)?;
        if let Some(p) = here.panel {
            return Ok(PanelId(p));
        }
    }
    session
        .active_panel
        .ok_or_else(|| anyhow!("no panel in session \"{}\"", session.name))
}

/// The project `new-panel` adds to: `--project`, else this terminal's panel's, else the
/// session's active one.
pub fn pick_project(
    tree: &[SessionView],
    session: &SessionView,
    explicit: Option<&str>,
    here: Option<Here>,
) -> anyhow::Result<Target> {
    if let Some(text) = explicit {
        return Ok(Target::parse(text));
    }
    if let Some(here) = here {
        check_here(tree, here)?;
        let own = here.panel.and_then(|panel| {
            session
                .projects
                .iter()
                .find(|p| p.panels.iter().any(|x| x.id.0 == panel))
        });
        if let Some(project) = own {
            return Ok(Target::Id(project.id.0));
        }
    }
    session
        .active_project
        .map(|p| Target::Id(p.0))
        .ok_or_else(|| anyhow!("no project in session \"{}\"", session.name))
}

/// The launch spec of `--cwd`, `--env` and a program after `--`; `None` for a plain shell in
/// the default directory. An empty directory in it means "the default".
pub fn launch_spec(
    program: Vec<String>,
    cwd: Option<PathBuf>,
    env: Vec<(String, String)>,
) -> anyhow::Result<Option<LaunchSpec>> {
    if program.is_empty() && cwd.is_none() && env.is_empty() {
        return Ok(None);
    }
    let cwd = match cwd {
        Some(dir) => std::path::absolute(&dir)
            .with_context(|| format!("cannot resolve {}", dir.display()))?,
        None => PathBuf::new(),
    };
    let mut program = program.into_iter();
    Ok(Some(LaunchSpec {
        program: program.next(),
        args: program.collect(),
        cwd,
        env: env.into_iter().map(|(k, v)| (k.into(), v.into())).collect(),
    }))
}

// Output.

/// Runs a command that prints text, and turns its error into the exit code and message.
pub(crate) fn finish(json: bool, result: anyhow::Result<Option<Value>>) -> ExitCode {
    match result {
        Ok(Some(value)) if json => {
            println!("{value}");
            ExitCode::SUCCESS
        }
        Ok(_) => ExitCode::SUCCESS,
        Err(e) if json => {
            println!("{}", json!({ "error": format!("{e:#}") }));
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

fn source_name(source: NameSource) -> &'static str {
    match source {
        NameSource::Default => "default",
        NameSource::Manual => "manual",
        NameSource::Auto => "auto",
    }
}

fn terminal_json(panel: &PanelView, index: usize, t: &TerminalView) -> Value {
    json!({
        "id": t.id.0,
        "index": index + 1,
        "title": t.title,
        "program": t.program,
        "cwd": t.cwd.as_ref().map(|p| p.to_string_lossy()),
        "pid": t.pid,
        "focused": panel.focused_terminal == Some(t.id),
        "exited": t.exit_status,
        "keep": t.keep,
    })
}

fn panel_json(session: &SessionView, panel: &PanelView) -> Value {
    json!({
        "id": panel.id.0,
        "name": panel.name,
        "name_source": source_name(panel.name_source),
        "active": session.active_panel == Some(panel.id),
        "terminals": panel
            .terminals
            .iter()
            .enumerate()
            .map(|(i, t)| terminal_json(panel, i, t))
            .collect::<Vec<_>>(),
    })
}

fn project_json(session: &SessionView, project: &ProjectView) -> Value {
    json!({
        "id": project.id.0,
        "name": project.name,
        "root": project.root.to_string_lossy(),
        "collapsed": project.collapsed,
        "panels": project.panels.iter().map(|p| panel_json(session, p)).collect::<Vec<_>>(),
    })
}

/// The document of `ls --json`.
pub fn tree_json(tree: &[SessionView]) -> Value {
    json!({
        "sessions": tree
            .iter()
            .map(|s| json!({
                "id": s.id.0,
                "name": s.name,
                "attached": s.attached_clients > 0,
                "projects": s.projects.iter().map(|p| project_json(s, p)).collect::<Vec<_>>(),
            }))
            .collect::<Vec<_>>(),
    })
}

/// The text of `ls --tree`: one line per session, project, panel and terminal, indented;
/// `*` marks the active panel and the focused terminal.
pub fn tree_text(tree: &[SessionView]) -> String {
    let mut out = String::new();
    for s in tree {
        let attached = if s.attached_clients > 0 {
            " (attached)"
        } else {
            ""
        };
        out.push_str(&format!("session {} [{}]{attached}\n", s.name, s.id));
        for p in &s.projects {
            let collapsed = if p.collapsed { " (collapsed)" } else { "" };
            out.push_str(&format!(
                "  project {} [{}] {}{collapsed}\n",
                p.name,
                p.id,
                p.root.display()
            ));
            for panel in &p.panels {
                let mark = if s.active_panel == Some(panel.id) {
                    " *"
                } else {
                    ""
                };
                out.push_str(&format!("    panel {} [{}]{mark}\n", panel.name, panel.id));
                for (i, t) in panel.terminals.iter().enumerate() {
                    let mut line = format!("      {} [{}] {}", i + 1, t.id, t.program);
                    if let Some(cwd) = &t.cwd {
                        line.push_str(&format!("  {}", cwd.display()));
                    }
                    match (t.exit_status, t.pid) {
                        (Some(code), _) => line.push_str(&format!("  exited {code}")),
                        (None, Some(pid)) => line.push_str(&format!("  pid {pid}")),
                        (None, None) => {}
                    }
                    if panel.focused_terminal == Some(t.id) {
                        line.push_str(" *");
                    }
                    out.push_str(&line);
                    out.push('\n');
                }
            }
        }
    }
    out
}

// The daemon.

const NO_DAEMON: &str = "no deck daemon running";

/// A connection to a running daemon: it never starts one.
pub(crate) struct Api {
    conn: Conn,
}

impl Api {
    pub(crate) fn connect() -> anyhow::Result<Self> {
        let paths = checked_paths()?;
        let socket = control_socket(&paths);
        match Conn::open(&socket, REPLY_TIMEOUT) {
            Ok(conn) => Ok(Self { conn }),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) =>
            {
                bail!("{NO_DAEMON}")
            }
            Err(e) => bail!(
                "cannot connect to the deck daemon at {}: {e}",
                socket.display()
            ),
        }
    }

    pub(crate) fn ask(&mut self, request: Request) -> anyhow::Result<Response> {
        match self.conn.control(request) {
            Outcome::Response(Response::Error(text)) => bail!("{text}"),
            Outcome::Response(r) => Ok(r),
            Outcome::Mismatch(daemon) => bail!("{}", mismatch_text(daemon)),
            Outcome::Other(e) => Err(e),
        }
    }

    /// Waits for the daemon's answer as long as it takes: the daemon enforces any limit.
    pub(crate) fn wait_for_answers(&mut self) -> anyhow::Result<()> {
        self.conn
            .no_read_timeout()
            .context("cannot wait for the daemon")
    }

    /// Every session's tree.
    pub(crate) fn tree(&mut self, session: Option<String>) -> anyhow::Result<Vec<SessionView>> {
        match self.ask(Request::Tree { session })? {
            Response::Tree(tree) => Ok(tree),
            _ => bail!("unexpected answer from the daemon"),
        }
    }

    fn recent(&mut self) -> anyhow::Result<Option<SessionId>> {
        match self.ask(Request::ListSessions)? {
            Response::Sessions(sessions) => Ok(sessions
                .iter()
                .filter(|s| s.last_attached > 0)
                .max_by_key(|s| s.last_attached)
                .map(|s| s.id)),
            _ => bail!("unexpected answer from the daemon"),
        }
    }

    /// Sends a request that answers with the id it created.
    fn create(&mut self, request: Request) -> anyhow::Result<Value> {
        match self.ask(request)? {
            Response::Created { id } => Ok(json!({ "id": id })),
            _ => bail!("unexpected answer from the daemon"),
        }
    }

    /// Sends a request that answers `Ok`.
    pub(crate) fn done(&mut self, request: Request) -> anyhow::Result<Value> {
        match self.ask(request)? {
            Response::Ok => Ok(json!({ "ok": true })),
            _ => bail!("unexpected answer from the daemon"),
        }
    }
}

/// The session, as `pick_session` finds it, and the environment defaults that apply: none
/// when `--session` was given.
pub(crate) fn session_here<'a>(
    api: &mut Api,
    tree: &'a [SessionView],
    common: &Common,
) -> anyhow::Result<(&'a SessionView, Option<Here>)> {
    let here = Here::from_env();
    let session = pick_session(tree, common.session.as_deref(), here, || api.recent())?;
    let defaults = common.session.is_none().then_some(here);
    Ok((session, defaults))
}

/// Prints the id a `new-*` command created: `12`, or `{"id":12}`.
fn print_created(json: bool, value: &Value) {
    if !json && let Some(id) = value.get("id") {
        println!("{id}");
    }
}

/// `deck ls --json` and `deck ls --tree`; plain `deck ls` is [`ls`].
pub fn run_ls(args: LsArgs) -> anyhow::Result<ExitCode> {
    if !args.common.json && !args.tree {
        return ls(args.common.session.as_deref());
    }
    let json = args.common.json;
    let result: anyhow::Result<Option<Value>> = (|| {
        let mut api = match Api::connect() {
            Ok(api) => api,
            // No daemon: there is nothing to list, and that is not an error here.
            Err(e) if e.to_string() == NO_DAEMON => {
                if json {
                    return Ok(Some(tree_json(&[])));
                }
                println!("no sessions");
                return Ok(None);
            }
            Err(e) => return Err(e),
        };
        let tree = api.tree(args.common.session.clone())?;
        if json {
            return Ok(Some(tree_json(&tree)));
        }
        if tree.is_empty() {
            println!("no sessions");
        } else {
            print!("{}", tree_text(&tree));
        }
        Ok(None)
    })();
    Ok(finish(json, result))
}

pub fn new_project(args: NewProjectArgs) -> ExitCode {
    let json = args.common.json;
    let result: anyhow::Result<Option<Value>> = (|| {
        let dir = match &args.dir {
            Some(dir) => std::path::absolute(dir)
                .with_context(|| format!("cannot resolve {}", dir.display()))?,
            None => std::env::current_dir().context("cannot read the current directory")?,
        };
        let mut api = Api::connect()?;
        let tree = api.tree(None)?;
        let (session, _) = session_here(&mut api, &tree, &args.common)?;
        let value = api.create(Request::NewProject {
            session: session.id,
            name: args.name.clone(),
            root: dir,
            focus: args.focus,
        })?;
        print_created(json, &value);
        Ok(Some(value))
    })();
    finish(json, result)
}

pub fn new_panel(args: NewPanelArgs) -> ExitCode {
    let json = args.common.json;
    let result: anyhow::Result<Option<Value>> = (|| {
        let launch = launch_spec(args.program.clone(), args.cwd.clone(), Vec::new())?;
        let mut api = Api::connect()?;
        let tree = api.tree(None)?;
        let (session, here) = session_here(&mut api, &tree, &args.common)?;
        let project = pick_project(&tree, session, args.project.as_deref(), here)?;
        let value = api.create(Request::NewPanel {
            session: session.id,
            project,
            name: args.name.clone(),
            launch,
            focus: args.focus,
        })?;
        print_created(json, &value);
        Ok(Some(value))
    })();
    finish(json, result)
}

pub fn new_terminal(args: NewTerminalArgs) -> ExitCode {
    let json = args.common.json;
    let result: anyhow::Result<Option<Value>> = (|| {
        let launch = launch_spec(args.program.clone(), args.cwd.clone(), args.env.clone())?;
        let mut api = Api::connect()?;
        let tree = api.tree(None)?;
        let panel = match args.panel {
            Some(id) => PanelId(id),
            None => {
                let (session, here) = session_here(&mut api, &tree, &args.common)?;
                pick_panel(&tree, session, None, here)?
            }
        };
        let value = api.create(Request::NewTerminal {
            panel,
            launch,
            keep: args.keep,
            focus: args.focus,
        })?;
        print_created(json, &value);
        Ok(Some(value))
    })();
    finish(json, result)
}

/// Fails when `id` is not in the session `--session` names (no check without it).
pub(crate) fn check_in_session(
    tree: &[SessionView],
    session: Option<&str>,
    id: u64,
) -> anyhow::Result<()> {
    let Some(name) = session else {
        return Ok(());
    };
    let s = tree
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| anyhow!("no session named \"{name}\""))?;
    let found = s.id.0 == id
        || s.projects.iter().any(|p| {
            p.id.0 == id
                || p.panels
                    .iter()
                    .any(|x| x.id.0 == id || x.terminals.iter().any(|t| t.id.0 == id))
        });
    if found {
        Ok(())
    } else {
        bail!("{id} is not in session \"{name}\"")
    }
}

fn by_id(args: IdArgs, request: fn(u64) -> Request) -> ExitCode {
    let json = args.common.json;
    let result = Api::connect()
        .and_then(|mut api| {
            if args.common.session.is_some() {
                let tree = api.tree(None)?;
                check_in_session(&tree, args.common.session.as_deref(), args.id)?;
            }
            api.done(request(args.id))
        })
        .map(Some);
    finish(json, result)
}

pub fn focus(args: IdArgs) -> ExitCode {
    by_id(args, |id| Request::Focus { id })
}

pub fn close(args: IdArgs) -> ExitCode {
    by_id(args, |id| Request::Close { id })
}

pub fn rename_panel(args: RenamePanelArgs) -> ExitCode {
    let json = args.common.json;
    let result: anyhow::Result<Option<Value>> = (|| {
        let mut api = Api::connect()?;
        let tree = api.tree(None)?;
        let panel = match args.panel {
            Some(id) => PanelId(id),
            None => {
                let (session, here) = session_here(&mut api, &tree, &args.common)?;
                pick_panel(&tree, session, None, here)?
            }
        };
        api.done(Request::RenamePanel {
            panel,
            name: args.name.clone(),
        })
        .map(Some)
    })();
    finish(json, result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use deck_core::layout::Ratios;
    use deck_core::model::{ProjectId, TerminalId};

    fn terminal(id: u64) -> TerminalView {
        TerminalView {
            id: TerminalId(id),
            program: "sh".into(),
            title: None,
            cwd: Some("/work".into()),
            exit_status: None,
            pid: Some(100 + u32::try_from(id).unwrap()),
            keep: false,
        }
    }

    /// Session `name` (id `sid`): project `sid+1` with panel `sid+2` holding terminal `sid+3`.
    fn session(sid: u64, name: &str) -> SessionView {
        SessionView {
            id: SessionId(sid),
            name: name.into(),
            active_project: Some(ProjectId(sid + 1)),
            active_panel: Some(PanelId(sid + 2)),
            layout_viewport: None,
            attached_clients: 0,
            projects: vec![ProjectView {
                id: ProjectId(sid + 1),
                name: "proj".into(),
                root: "/work".into(),
                collapsed: false,
                active_panel: Some(PanelId(sid + 2)),
                panels: vec![PanelView {
                    id: PanelId(sid + 2),
                    name: "1".into(),
                    name_source: NameSource::Manual,
                    terminals: vec![terminal(sid + 3)],
                    focused_terminal: Some(TerminalId(sid + 3)),
                    ratios: Ratios::default_for(1),
                }],
            }],
        }
    }

    fn no_recent() -> anyhow::Result<Option<SessionId>> {
        Ok(None)
    }

    #[test]
    fn the_session_comes_from_the_option_then_the_environment_then_the_daemon() {
        let tree = [session(10, "main"), session(20, "other")];
        let here = Here {
            session: Some(10),
            ..Here::default()
        };
        let name = |r: anyhow::Result<&SessionView>| r.map(|s| s.name.clone()).unwrap_or_default();
        assert_eq!(
            name(pick_session(&tree, Some("other"), here, no_recent)),
            "other",
            "--session wins over the environment"
        );
        assert_eq!(name(pick_session(&tree, None, here, no_recent)), "main");
        assert_eq!(
            name(pick_session(&tree, None, Here::default(), || Ok(Some(
                SessionId(20)
            )))),
            "other",
            "outside deck: the most recently attached"
        );
        assert_eq!(
            name(pick_session(&tree[..1], None, Here::default(), no_recent)),
            "main",
            "the only session"
        );
        let err = |r: anyhow::Result<&SessionView>| r.map(|_| ()).unwrap_err().to_string();
        assert_eq!(
            err(pick_session(&[], None, Here::default(), no_recent)),
            "no session"
        );
        assert_eq!(
            err(pick_session(&tree, Some("nope"), here, no_recent)),
            "no session named \"nope\""
        );
        assert!(err(pick_session(&tree, None, Here::default(), no_recent)).contains("--session"));
        let stale = Here {
            session: Some(99),
            terminal: Some(7),
            ..Here::default()
        };
        assert_eq!(
            err(pick_session(&tree, None, stale, no_recent)),
            "this terminal (7) no longer exists"
        );
    }

    #[test]
    fn the_panel_is_this_terminals_unless_the_session_option_turned_that_off() {
        let tree = [session(10, "main"), session(20, "other")];
        let here = Here {
            session: Some(10),
            panel: Some(12),
            terminal: Some(13),
        };
        let other = tree.get(1).unwrap();
        assert_eq!(
            pick_panel(&tree, tree.first().unwrap(), None, Some(here)).unwrap(),
            PanelId(12)
        );
        // `--session other`: the environment is off, so the session's active panel.
        assert_eq!(pick_panel(&tree, other, None, None).unwrap(), PanelId(22));
        assert_eq!(
            pick_panel(&tree, other, Some(5), Some(here)).unwrap(),
            PanelId(5)
        );
        let stale = Here {
            panel: Some(99),
            ..here
        };
        assert_eq!(
            pick_panel(&tree, other, None, Some(stale))
                .unwrap_err()
                .to_string(),
            "this panel (99) no longer exists"
        );
        let gone = Here {
            terminal: Some(98),
            ..here
        };
        assert_eq!(
            pick_panel(&tree, other, None, Some(gone))
                .unwrap_err()
                .to_string(),
            "this terminal (98) no longer exists"
        );
    }

    #[test]
    fn the_project_is_named_by_id_or_name_else_this_terminals() {
        let tree = [session(10, "main")];
        let s = tree.first().unwrap();
        let here = Here {
            session: Some(10),
            panel: Some(12),
            terminal: Some(13),
        };
        assert_eq!(
            pick_project(&tree, s, Some("api"), Some(here)).unwrap(),
            Target::Name("api".into())
        );
        assert_eq!(
            pick_project(&tree, s, Some("5"), None).unwrap(),
            Target::Id(5)
        );
        assert_eq!(
            pick_project(&tree, s, None, Some(here)).unwrap(),
            Target::Id(11)
        );
        assert_eq!(pick_project(&tree, s, None, None).unwrap(), Target::Id(11));
    }

    #[test]
    fn a_launch_spec_is_made_only_when_something_asks_for_one() {
        assert_eq!(launch_spec(vec![], None, vec![]).unwrap(), None);
        let spec = launch_spec(
            vec!["sh".into(), "-c".into(), "true".into()],
            Some("/tmp".into()),
            vec![("A".into(), "1".into())],
        )
        .unwrap()
        .unwrap();
        assert_eq!(spec.program.as_deref(), Some("sh"));
        assert_eq!(spec.args, ["-c", "true"]);
        assert_eq!(spec.cwd, PathBuf::from("/tmp"));
        assert_eq!(spec.env.len(), 1);
        // `--env` alone: a shell, in the default directory.
        let spec = launch_spec(vec![], None, vec![("A".into(), "1".into())])
            .unwrap()
            .unwrap();
        assert!(spec.program.is_none() && spec.cwd.as_os_str().is_empty());
        assert_eq!(parse_env("K=a=b").unwrap(), ("K".into(), "a=b".into()));
        assert!(parse_env("nokey").is_err() && parse_env("=v").is_err());
    }

    #[test]
    fn the_tree_is_json_with_the_documented_fields() {
        let mut s = session(10, "main");
        s.attached_clients = 1;
        let t = s
            .projects
            .first_mut()
            .and_then(|p| p.panels.first_mut())
            .and_then(|p| p.terminals.first_mut())
            .unwrap();
        t.exit_status = Some(3);
        t.pid = None;
        t.keep = true;
        let doc = tree_json(&[s]);
        let sess = &doc["sessions"][0];
        assert_eq!(sess["attached"], true);
        let panel = &sess["projects"][0]["panels"][0];
        assert_eq!(panel["name_source"], "manual");
        assert_eq!(panel["active"], true);
        let term = &panel["terminals"][0];
        assert_eq!(term["index"], 1);
        assert_eq!(term["focused"], true);
        assert_eq!(term["exited"], 3);
        assert_eq!(term["keep"], true);
        assert_eq!(term["pid"], Value::Null);
        assert_eq!(term["cwd"], "/work");
        assert_eq!(term["title"], Value::Null);
        assert_eq!(tree_json(&[]), json!({"sessions": []}));
    }

    #[test]
    fn the_tree_is_indented_text() {
        let mut s = session(10, "main");
        s.attached_clients = 1;
        assert_eq!(
            tree_text(&[s]),
            "session main [10] (attached)\n  project proj [11] /work\n    panel 1 [12] *\n      \
             1 [13] sh  /work  pid 113 *\n"
        );
    }
}
