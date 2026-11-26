//! The saved form of the model (`state.json`): what the daemon writes and reads back to
//! restore sessions after a restart. Pure: the daemon does the I/O.
//!
//! Later specs add fields as `Option`s with `#[serde(default)]`, which old files still
//! parse without a version bump.

use std::collections::HashSet;
use std::ffi::OsString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::agents::AgentResume;
use crate::layout::Ratios;
use crate::model::{
    LaunchSpec, NameSource, Panel, PanelId, Project, ProjectId, Session, SessionId, Terminal,
    TerminalId,
};

/// The version of the file format this build reads and writes.
pub const STATE_VERSION: u32 = 1;

/// Paths are saved as text (so `state.json` stays readable); a path that is not UTF-8
/// loses its odd bytes, and the terminal then falls back to its project root.
mod text_path {
    use std::path::{Path, PathBuf};

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(p: &Path, s: S) -> Result<S::Ok, S::Error> {
        p.to_string_lossy().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<PathBuf, D::Error> {
        Ok(PathBuf::from(String::deserialize(d)?))
    }
}

mod opt_text_path {
    use std::path::PathBuf;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(p: &Option<PathBuf>, s: S) -> Result<S::Ok, S::Error> {
        p.as_ref().map(|p| p.to_string_lossy()).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<PathBuf>, D::Error> {
        Ok(Option::<String>::deserialize(d)?.map(PathBuf::from))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateFile {
    pub version: u32,
    /// The next id to hand out.
    pub next_id: u64,
    /// Sessions that were attached, least recently first.
    pub last_attached: Vec<SessionId>,
    pub sessions: Vec<SessionState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionState {
    pub id: SessionId,
    pub name: String,
    pub active_project: Option<ProjectId>,
    pub projects: Vec<ProjectState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectState {
    pub id: ProjectId,
    pub name: String,
    #[serde(with = "text_path")]
    pub root: PathBuf,
    pub collapsed: bool,
    pub active_panel: Option<PanelId>,
    pub panels: Vec<PanelState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelState {
    pub id: PanelId,
    pub name: String,
    pub name_source: NameSource,
    pub focused_terminal: Option<TerminalId>,
    pub ratios: Ratios,
    pub terminals: Vec<TerminalState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalState {
    pub id: TerminalId,
    /// The explicit program, or `None` for the configured shell.
    pub program: Option<String>,
    pub args: Vec<String>,
    /// Only the additions of the launch spec (`--env`): never the client's environment and
    /// never `DECK_*`.
    pub env: Vec<(String, String)>,
    /// The launch directory until the live one is tracked.
    #[serde(with = "opt_text_path")]
    pub cwd: Option<PathBuf>,
    /// The terminal stays after its process exits.
    #[serde(default)]
    pub keep: bool,
    /// The exit code of a kept terminal whose process ended; such a terminal is not
    /// restored.
    #[serde(default)]
    pub exited: Option<i32>,
    /// The agent that was in the foreground, to resume after a restart.
    #[serde(default)]
    pub agent: Option<AgentResume>,
}

fn is_deck_var(key: &[u8]) -> bool {
    key.starts_with(b"DECK_") || key == b"DECK"
}

impl TerminalState {
    pub fn from_model(t: &Terminal) -> Self {
        Self {
            id: t.id,
            program: t.launch.program.clone(),
            args: t.launch.args.clone(),
            env: t
                .launch
                .env
                .iter()
                .filter(|(k, _)| !is_deck_var(k.as_bytes()))
                .map(|(k, v)| {
                    (
                        k.to_string_lossy().into_owned(),
                        v.to_string_lossy().into_owned(),
                    )
                })
                .collect(),
            cwd: t.cwd.clone().or_else(|| Some(t.launch.cwd.clone())),
            keep: t.keep,
            exited: t.exit_status.filter(|_| t.keep),
            agent: t.agent.as_ref().map(|a| AgentResume {
                kind: a.kind,
                session_id: a.session_id.clone(),
            }),
        }
    }

    /// The terminal as restored. `program` is its display name; its launch directory is the
    /// saved one, else `fallback_cwd`.
    pub fn to_model(&self, program: String, fallback_cwd: &Path) -> Terminal {
        Terminal {
            id: self.id,
            launch: LaunchSpec {
                program: self.program.clone(),
                args: self.args.clone(),
                cwd: self
                    .cwd
                    .clone()
                    .unwrap_or_else(|| fallback_cwd.to_path_buf()),
                env: self
                    .env
                    .iter()
                    .map(|(k, v)| (OsString::from(k), OsString::from(v)))
                    .collect(),
            },
            program,
            title: None,
            cwd: None,
            exit_status: None,
            keep: self.keep,
            agent: None,
        }
    }

    /// A kept terminal whose process had ended: it is not restored.
    pub fn is_spent(&self) -> bool {
        self.keep && self.exited.is_some()
    }
}

impl PanelState {
    pub fn from_model(p: &Panel) -> Self {
        Self {
            id: p.id,
            name: p.name.clone(),
            name_source: p.name_source,
            focused_terminal: p.focused_terminal,
            ratios: p.ratios.clone(),
            terminals: p.terminals.iter().map(TerminalState::from_model).collect(),
        }
    }

    pub fn to_model(&self, root: &Path, program: &dyn Fn(&TerminalState) -> String) -> Panel {
        Panel {
            id: self.id,
            name: self.name.clone(),
            name_source: self.name_source,
            terminals: self
                .terminals
                .iter()
                .map(|t| t.to_model(program(t), root))
                .collect(),
            focused_terminal: self.focused_terminal,
            ratios: self.ratios.clone(),
        }
    }
}

impl ProjectState {
    pub fn from_model(p: &Project) -> Self {
        Self {
            id: p.id,
            name: p.name.clone(),
            root: p.root.clone(),
            collapsed: p.collapsed,
            active_panel: p.active_panel,
            panels: p.panels.iter().map(PanelState::from_model).collect(),
        }
    }

    pub fn to_model(&self, program: &dyn Fn(&TerminalState) -> String) -> Project {
        Project {
            id: self.id,
            name: self.name.clone(),
            root: self.root.clone(),
            panels: self
                .panels
                .iter()
                .map(|p| p.to_model(&self.root, program))
                .collect(),
            active_panel: self.active_panel,
            collapsed: self.collapsed,
        }
    }
}

impl SessionState {
    pub fn from_model(s: &Session) -> Self {
        Self {
            id: s.id,
            name: s.name.clone(),
            active_project: s.active_project,
            projects: s.projects.iter().map(ProjectState::from_model).collect(),
        }
    }

    /// The session as restored; `program` names the program a terminal runs.
    pub fn to_model(&self, program: &dyn Fn(&TerminalState) -> String) -> Session {
        Session {
            id: self.id,
            name: self.name.clone(),
            projects: self.projects.iter().map(|p| p.to_model(program)).collect(),
            active_project: self.active_project,
        }
    }
}

/// `base`, or `base (2)`, `base (3)`, ... when `taken` holds it.
fn unique_name(base: &str, taken: &HashSet<String>) -> String {
    if !taken.contains(base) {
        return base.to_string();
    }
    let mut n = 2u64;
    loop {
        let name = format!("{base} ({n})");
        if !taken.contains(&name) {
            return name;
        }
        n += 1;
    }
}

impl StateFile {
    /// The file for these sessions. `next_id` is the next id to hand out; `last_attached`
    /// lists session ids, least recently attached first.
    pub fn new<'a>(
        next_id: u64,
        last_attached: Vec<SessionId>,
        sessions: impl IntoIterator<Item = &'a Session>,
    ) -> Self {
        Self {
            version: STATE_VERSION,
            next_id,
            last_attached,
            sessions: sessions.into_iter().map(SessionState::from_model).collect(),
        }
    }

    /// Makes a parsed file safe to rebuild and returns what it had to change, one line each:
    /// `next_id` moves past every saved id, a duplicate id drops the later item, a duplicate
    /// session or project name gets ` (2)`, ` (3)`, ..., references to missing items are
    /// dropped and focus falls back to the first item, ratios invalid for the restored
    /// count become the defaults, and a panel without terminals goes. A kept terminal that
    /// had exited goes without a note.
    pub fn validate(&mut self) -> Vec<String> {
        let mut notes = Vec::new();
        let mut max_id = 0u64;
        let mut sessions = HashSet::new();
        let mut projects = HashSet::new();
        let mut panels = HashSet::new();
        let mut terminals = HashSet::new();
        let mut session_names = HashSet::new();

        let mut keep_sessions = Vec::new();
        for mut session in std::mem::take(&mut self.sessions) {
            max_id = max_id.max(session.id.0);
            if !sessions.insert(session.id.0) {
                notes.push(format!("duplicate session id {}: dropped", session.id));
                continue;
            }
            let name = unique_name(&session.name, &session_names);
            if name != session.name {
                notes.push(format!(
                    "duplicate session name {}: now {name}",
                    session.name
                ));
                session.name = name;
            }
            session_names.insert(session.name.clone());

            let mut project_names = HashSet::new();
            let mut keep_projects = Vec::new();
            for mut project in std::mem::take(&mut session.projects) {
                max_id = max_id.max(project.id.0);
                if !projects.insert(project.id.0) {
                    notes.push(format!("duplicate space id {}: dropped", project.id));
                    continue;
                }
                let name = unique_name(&project.name, &project_names);
                if name != project.name {
                    notes.push(format!("duplicate space name {}: now {name}", project.name));
                    project.name = name;
                }
                project_names.insert(project.name.clone());

                let mut keep_panels = Vec::new();
                for mut panel in std::mem::take(&mut project.panels) {
                    max_id = max_id.max(panel.id.0);
                    if !panels.insert(panel.id.0) {
                        notes.push(format!("duplicate panel id {}: dropped", panel.id));
                        continue;
                    }
                    panel.terminals.retain(|t| {
                        max_id = max_id.max(t.id.0);
                        let fresh = terminals.insert(t.id.0);
                        if !fresh {
                            notes.push(format!("duplicate terminal id {}: dropped", t.id));
                        }
                        fresh
                    });
                    // A kept terminal that had exited is not restored; no note for it.
                    let had = panel.terminals.len();
                    panel.terminals.retain(|t| !t.is_spent());
                    if panel.terminals.is_empty() {
                        if had > 0 {
                            continue;
                        }
                        notes.push(format!("panel {} has no terminals: dropped", panel.id));
                        continue;
                    }
                    if !panel
                        .focused_terminal
                        .is_some_and(|f| panel.terminals.iter().any(|t| t.id == f))
                    {
                        panel.focused_terminal = panel.terminals.first().map(|t| t.id);
                    }
                    if !panel.ratios.is_valid_for(panel.terminals.len()) {
                        panel.ratios = Ratios::default_for(panel.terminals.len());
                    }
                    keep_panels.push(panel);
                }
                project.panels = keep_panels;
                if !project
                    .active_panel
                    .is_some_and(|a| project.panels.iter().any(|p| p.id == a))
                {
                    project.active_panel = project.panels.first().map(|p| p.id);
                }
                keep_projects.push(project);
            }
            session.projects = keep_projects;
            if !session
                .active_project
                .is_some_and(|a| session.projects.iter().any(|p| p.id == a))
            {
                session.active_project = session.projects.first().map(|p| p.id);
            }
            keep_sessions.push(session);
        }
        self.sessions = keep_sessions;

        self.next_id = self.next_id.max(max_id.saturating_add(1));
        let mut seen = HashSet::new();
        let known: HashSet<u64> = self.sessions.iter().map(|s| s.id.0).collect();
        self.last_attached
            .retain(|s| known.contains(&s.0) && seen.insert(s.0));
        notes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terminal(id: u64, program: Option<&str>) -> Terminal {
        Terminal {
            id: TerminalId(id),
            launch: LaunchSpec {
                program: program.map(str::to_string),
                args: vec!["-c".into(), "x".into()],
                cwd: "/work/a".into(),
                env: vec![
                    ("TOKEN".into(), "s3".into()),
                    ("DECK_TERMINAL_ID".into(), "9".into()),
                ],
            },
            program: "sh".into(),
            title: None,
            cwd: None,
            exit_status: None,
            keep: false,
            agent: None,
        }
    }

    fn panel(id: u64, terminals: Vec<Terminal>) -> Panel {
        let n = terminals.len();
        Panel {
            id: PanelId(id),
            name: format!("p{id}"),
            name_source: NameSource::Manual,
            focused_terminal: terminals.last().map(|t| t.id),
            ratios: Ratios::default_for(n),
            terminals,
        }
    }

    fn project(id: u64, name: &str, panels: Vec<Panel>) -> Project {
        Project {
            id: ProjectId(id),
            name: name.into(),
            root: "/work".into(),
            active_panel: panels.last().map(|p| p.id),
            panels,
            collapsed: id.is_multiple_of(2),
        }
    }

    fn session(id: u64, name: &str, projects: Vec<Project>) -> Session {
        Session {
            id: SessionId(id),
            name: name.into(),
            active_project: projects.last().map(|p| p.id),
            projects,
        }
    }

    fn sample() -> Vec<Session> {
        let mut custom = panel(
            12,
            vec![
                terminal(13, None),
                terminal(14, Some("/bin/sleep")),
                terminal(15, None),
            ],
        );
        custom.ratios.main = Some(700);
        custom.focused_terminal = Some(TerminalId(14));
        vec![
            session(
                1,
                "main",
                vec![
                    project(10, "one", vec![panel(11, vec![terminal(16, None)]), custom]),
                    project(20, "two", vec![panel(21, vec![terminal(22, None)])]),
                ],
            ),
            session(2, "other", vec![]),
        ]
    }

    /// The display name a restored terminal gets in these tests.
    fn name_of(t: &TerminalState) -> String {
        t.program.clone().unwrap_or_else(|| "sh".into())
    }

    #[test]
    fn a_model_survives_state_and_back() {
        let mut sessions = sample();
        // The live cwd is saved in place of the launch one.
        sessions[0].projects[0].panels[0].terminals[0].cwd = Some("/live".into());
        let file = StateFile::new(30, vec![SessionId(2), SessionId(1)], &sessions);
        assert_eq!(file.version, 1);

        let back: Vec<Session> = file.sessions.iter().map(|s| s.to_model(&name_of)).collect();
        // Titles and the live cwd are not saved; the live cwd becomes the launch one.
        let mut expected = sessions.clone();
        for t in expected
            .iter_mut()
            .flat_map(|s| &mut s.projects)
            .flat_map(|p| &mut p.panels)
            .flat_map(|p| &mut p.terminals)
        {
            t.launch.env.retain(|(k, _)| k != "DECK_TERMINAL_ID");
            if let Some(cwd) = t.cwd.take() {
                t.launch.cwd = cwd;
            }
            t.program = name_of(&TerminalState::from_model(t));
        }
        assert_eq!(back, expected);

        // And through the wire format.
        let bytes = postcard::to_stdvec(&file).unwrap();
        assert_eq!(postcard::from_bytes::<StateFile>(&bytes).unwrap(), file);
    }

    #[test]
    fn the_saved_env_holds_no_deck_variables() {
        let t = TerminalState::from_model(&terminal(1, None));
        assert_eq!(t.env, [("TOKEN".to_string(), "s3".to_string())]);
    }

    #[test]
    fn a_valid_file_is_left_alone() {
        let sessions = sample();
        let mut file = StateFile::new(30, vec![SessionId(1)], &sessions);
        let before = file.clone();
        assert!(file.validate().is_empty());
        assert_eq!(file, before);
    }

    #[test]
    fn next_id_moves_past_every_saved_id() {
        let mut file = StateFile::new(3, vec![], &sample());
        file.validate();
        assert_eq!(file.next_id, 23);
    }

    #[test]
    fn duplicates_drop_the_later_item_and_names_get_a_number() {
        let mut sessions = sample();
        sessions[0].projects[1].name = "one".into();
        sessions.push(session(3, "main", vec![]));
        let mut file = StateFile::new(30, vec![], &sessions);
        file.sessions[0].projects[1].panels[0].terminals[0].id = TerminalId(13);
        file.validate();

        assert_eq!(file.sessions[2].name, "main (2)");
        assert_eq!(file.sessions[0].projects[1].name, "one (2)");
        // The panel that held the duplicate terminal had nothing else: it is gone.
        assert!(file.sessions[0].projects[1].panels.is_empty());
        assert_eq!(file.sessions[0].projects[1].active_panel, None);
    }

    #[test]
    fn a_duplicate_session_or_project_id_drops_it_with_its_contents() {
        let mut sessions = sample();
        sessions.push(session(1, "again", vec![]));
        let mut file = StateFile::new(30, vec![], &sessions);
        file.sessions[0].projects[1].id = ProjectId(10);
        file.validate();
        assert_eq!(file.sessions.len(), 2);
        assert_eq!(file.sessions[0].projects.len(), 1);
    }

    #[test]
    fn focus_on_missing_items_falls_back_to_the_first() {
        let mut file = StateFile::new(
            30,
            vec![SessionId(7), SessionId(1), SessionId(1)],
            &sample(),
        );
        file.sessions[0].active_project = Some(ProjectId(99));
        file.sessions[0].projects[0].active_panel = Some(PanelId(98));
        file.sessions[0].projects[0].panels[1].focused_terminal = Some(TerminalId(97));
        file.validate();
        let s = &file.sessions[0];
        assert_eq!(s.active_project, Some(ProjectId(10)));
        assert_eq!(s.projects[0].active_panel, Some(PanelId(11)));
        assert_eq!(
            s.projects[0].panels[1].focused_terminal,
            Some(TerminalId(13))
        );
        assert_eq!(file.last_attached, [SessionId(1)]);
    }

    #[test]
    fn ratios_that_do_not_fit_the_count_become_the_defaults() {
        let mut file = StateFile::new(30, vec![], &sample());
        file.sessions[0].projects[0].panels[1].ratios = Ratios::default_for(1);
        file.validate();
        assert_eq!(
            file.sessions[0].projects[0].panels[1].ratios,
            Ratios::default_for(3)
        );
        // Valid custom ratios stay.
        let mut file = StateFile::new(30, vec![], &sample());
        file.validate();
        assert_eq!(
            file.sessions[0].projects[0].panels[1].ratios.main,
            Some(700)
        );
    }

    #[test]
    fn an_exited_kept_terminal_is_not_restored() {
        let mut kept = terminal(4, Some("sh"));
        kept.keep = true;
        kept.exit_status = Some(3);
        let mut other = terminal(9, Some("sh"));
        other.keep = true;
        other.exit_status = Some(0);
        let mut running = terminal(5, Some("sleep"));
        running.keep = true;
        let saved = TerminalState::from_model(&kept);
        assert!(saved.keep && saved.exited == Some(3));
        assert!(TerminalState::from_model(&running).keep);

        let sessions = [session(
            1,
            "main",
            vec![project(
                2,
                "p",
                vec![
                    panel(3, vec![terminal(6, None), kept.clone()]),
                    panel(7, vec![other]),
                    panel(8, vec![running]),
                ],
            )],
        )];
        let mut file = StateFile::new(30, vec![], &sessions);
        let notes = file.validate();
        assert!(notes.is_empty(), "{notes:?}");
        let panels = &file.sessions[0].projects[0].panels;
        assert_eq!(panels.len(), 2, "the panel of the exited terminal is gone");
        assert_eq!(panels[0].terminals.len(), 1);
        assert_eq!(panels[0].focused_terminal, Some(TerminalId(6)));
        assert_eq!(panels[1].terminals.len(), 1);
        assert!(panels[1].terminals[0].keep);
    }

    #[test]
    fn the_agent_is_saved_and_old_files_without_it_load() {
        use crate::agents::{AgentKind, AgentSource, AgentState, AgentStatus};
        let mut t = terminal(1, None);
        assert_eq!(TerminalState::from_model(&t).agent, None);
        t.agent = Some(AgentState {
            kind: AgentKind::Claude,
            status: AgentStatus::Idle,
            source: AgentSource::Hooks,
            session_id: Some("abc".into()),
            first_prompt: Some("hi".into()),
            since: std::time::Duration::ZERO,
            activity: None,
        });
        let saved = TerminalState::from_model(&t);
        let resume = AgentResume {
            kind: AgentKind::Claude,
            session_id: Some("abc".into()),
        };
        assert_eq!(saved.agent, Some(resume));
        let text = serde_json::to_string(&saved).unwrap();
        assert_eq!(serde_json::from_str::<TerminalState>(&text).unwrap(), saved);

        let old = r#"{"id":1,"program":null,"args":[],"env":[],"cwd":null}"#;
        let parsed: TerminalState = serde_json::from_str(old).unwrap();
        assert_eq!(parsed.agent, None);
    }
}
