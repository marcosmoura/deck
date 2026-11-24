//! Prompts, confirmations and the help overlay: what takes the keyboard before anything
//! else, and the one-line editor of the prompts.

use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use deck_core::action::{Action as Act, ActionError, default_project_name};
use deck_core::keys::{KeyCode, KeyEvent};
use deck_core::model::{PanelId, ProjectId, TerminalId};
use deck_core::protocol::{ClientMsg, Request};

use crate::app::{App, Out};
use crate::picker::Req;

/// What a prompt or confirmation is about; when it goes away meanwhile the dialog closes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Panel(PanelId, String),
    Project(ProjectId, String),
    /// A terminal and its 1-based index in the panel.
    Terminal(TerminalId, usize),
    /// A session, by name; it is not tracked while the dialog is open.
    Session(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptKind {
    /// `Directory:` of a new project.
    ProjectDir,
    /// `Project name:`, for the directory the first prompt resolved.
    ProjectName {
        root: PathBuf,
        dir_text: String,
    },
    RenamePanel(PanelId),
    RenameProject(ProjectId),
    /// `New session:`, from the picker; an empty name lets the daemon choose.
    NewSession,
    /// `Rename session:`; `current` is the attached session, renamed by an action.
    RenameSession {
        from: String,
        current: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub kind: PromptKind,
    pub editor: Editor,
    pub target: Option<Target>,
}

impl Prompt {
    pub fn label(&self) -> &'static str {
        match self.kind {
            PromptKind::ProjectDir => "Directory:",
            PromptKind::ProjectName { .. } => "Space name:",
            PromptKind::RenamePanel(_) => "Rename panel:",
            PromptKind::RenameProject(_) => "Rename space:",
            PromptKind::NewSession => "New session:",
            PromptKind::RenameSession { .. } => "Rename session:",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm {
    pub target: Target,
    pub question: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    Prompt(Prompt),
    Confirm(Confirm),
    Help,
}

/// What to reopen when the daemon refuses the action a prompt submitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retry {
    pub prompt: Prompt,
    /// For a new project: the directory prompt, reopened when the directory is the problem.
    pub path_prompt: Option<Prompt>,
}

/// A one-line text editor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Editor {
    chars: Vec<char>,
    cursor: usize,
}

impl Editor {
    /// An editor with `text`, the cursor at its end.
    pub fn new(text: &str) -> Self {
        let chars: Vec<char> = text.chars().filter(|c| !c.is_control()).collect();
        Self {
            cursor: chars.len(),
            chars,
        }
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    /// The text before the cursor and the text from it on.
    pub fn split(&self) -> (String, String) {
        let (a, b) = self.chars.split_at(self.cursor);
        (a.iter().collect(), b.iter().collect())
    }

    /// Inserts text at the cursor; control characters (newlines among them) are dropped.
    pub fn insert_str(&mut self, text: &str) {
        for c in text.chars().filter(|c| !c.is_control()) {
            self.chars.insert(self.cursor, c);
            self.cursor += 1;
        }
    }

    /// Applies an editing key; false for a key that edits nothing.
    pub fn on_key(&mut self, key: &KeyEvent) -> bool {
        let (ctrl, alt) = (key.mods.ctrl, key.mods.alt);
        match key.code {
            KeyCode::Char('u') if ctrl && !alt => {
                self.chars.clear();
                self.cursor = 0;
            }
            KeyCode::Char('w') if ctrl && !alt => {
                // The word before the cursor, and the blanks between it and the cursor.
                let mut from = self.cursor;
                while from > 0 && self.chars.get(from - 1) == Some(&' ') {
                    from -= 1;
                }
                while from > 0 && self.chars.get(from - 1).is_some_and(|c| *c != ' ') {
                    from -= 1;
                }
                self.chars.drain(from..self.cursor);
                self.cursor = from;
            }
            KeyCode::Char(c) if !ctrl && !alt => self.insert_str(c.encode_utf8(&mut [0; 4])),
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.chars.len()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.chars.len(),
            KeyCode::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                self.chars.remove(self.cursor);
            }
            KeyCode::Delete if self.cursor < self.chars.len() => {
                self.chars.remove(self.cursor);
            }
            _ => return false,
        }
        true
    }
}

/// A directory the user typed, made absolute: `~` is the home directory and a relative path
/// is against `cwd`; `.` and `..` are resolved in the text.
pub fn resolve_dir(text: &str, cwd: &Path, home: Option<&Path>) -> PathBuf {
    let text = text.trim();
    let path = if text.is_empty() {
        cwd.to_path_buf()
    } else if let (Some(home), true) = (home, text == "~") {
        home.to_path_buf()
    } else if let (Some(home), Some(rest)) = (home, text.strip_prefix("~/")) {
        home.join(rest)
    } else if Path::new(text).is_absolute() {
        PathBuf::from(text)
    } else {
        cwd.join(text)
    };
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

impl App {
    pub(crate) fn open_prompt(&mut self, kind: PromptKind, text: &str, target: Option<Target>) {
        self.modal = Some(Modal::Prompt(Prompt {
            kind,
            editor: Editor::new(text),
            target,
        }));
    }

    pub(crate) fn open_new_project(&mut self) {
        let cwd = self.cwd.to_string_lossy().into_owned();
        self.open_prompt(PromptKind::ProjectDir, &cwd, None);
    }

    pub(crate) fn open_rename_panel(&mut self, panel: PanelId) {
        let Some((_, p)) = self.mirror.find_panel(panel) else {
            return;
        };
        let name = p.name.clone();
        self.open_prompt(
            PromptKind::RenamePanel(panel),
            &name,
            Some(Target::Panel(panel, name.clone())),
        );
    }

    pub(crate) fn open_rename_project(&mut self, project: ProjectId) {
        let Some(p) = self.mirror.find_project(project) else {
            return;
        };
        let name = p.name.clone();
        self.open_prompt(
            PromptKind::RenameProject(project),
            &name,
            Some(Target::Project(project, name.clone())),
        );
    }

    pub(crate) fn confirm_close_panel(&mut self, panel: PanelId) {
        let Some((_, p)) = self.mirror.find_panel(panel) else {
            return;
        };
        let question = format!(
            "Close panel \"{}\" and its {}?",
            p.name,
            if p.terminals.len() == 1 {
                "terminal".to_string()
            } else {
                plural(p.terminals.len(), "terminal", "terminals")
            }
        );
        let target = Target::Panel(panel, p.name.clone());
        self.modal = Some(Modal::Confirm(Confirm { target, question }));
    }

    /// Asks before closing a terminal; a kept terminal whose process already ended closes at
    /// once.
    pub(crate) fn confirm_close_terminal(
        &mut self,
        terminal: TerminalId,
        now: Instant,
    ) -> Vec<Out> {
        let found = self
            .mirror
            .find_terminal(terminal)
            .map(|(_, index, t)| (terminal, index, t.exit_status.is_some()));
        let Some((focused, index, exited)) = found else {
            self.set_flash("no terminal to close".into(), now, false);
            return Vec::new();
        };
        if exited {
            return vec![self.act(Act::CloseTerminal { terminal: focused }, None)];
        }
        let question = format!("Close terminal {}?", index + 1);
        let target = Target::Terminal(focused, index + 1);
        self.modal = Some(Modal::Confirm(Confirm { target, question }));
        Vec::new()
    }

    pub(crate) fn confirm_close_project(&mut self, project: ProjectId) {
        let Some(p) = self.mirror.find_project(project) else {
            return;
        };
        let terminals: usize = p.panels.iter().map(|x| x.terminals.len()).sum();
        let question = format!(
            "Close space \"{}\" and its {}?",
            p.name,
            plural(terminals, "terminal", "terminals")
        );
        let target = Target::Project(project, p.name.clone());
        self.modal = Some(Modal::Confirm(Confirm { target, question }));
    }

    /// Text pasted into an open prompt.
    pub(crate) fn modal_paste(&mut self, text: &str) {
        if let Some(Modal::Prompt(p)) = &mut self.modal {
            p.editor.insert_str(text);
        }
    }

    /// A key while a dialog is open; it never goes anywhere else.
    pub(crate) fn modal_key(&mut self, key: KeyEvent) -> Vec<Out> {
        let Some(modal) = self.modal.take() else {
            return Vec::new();
        };
        let plain = !key.mods.ctrl && !key.mods.alt;
        match modal {
            Modal::Help => {
                let close = plain && matches!(key.code, KeyCode::Esc | KeyCode::Char('q' | '?'));
                if !close {
                    self.modal = Some(Modal::Help);
                    let (room, total) = crate::draw::help_room(self);
                    let max = total.saturating_sub(room);
                    let page = room.max(1);
                    let at = self.help_scroll;
                    self.help_scroll = match key.code {
                        KeyCode::Up | KeyCode::Char('k') => at.saturating_sub(1),
                        KeyCode::Down | KeyCode::Char('j') => at + 1,
                        KeyCode::PageUp => at.saturating_sub(page),
                        KeyCode::PageDown => at + page,
                        KeyCode::Home => 0,
                        KeyCode::End => max,
                        _ => at,
                    }
                    .min(max);
                }
                Vec::new()
            }
            Modal::Confirm(confirm) => match key.code {
                KeyCode::Char('y') if plain => {
                    let action = match confirm.target {
                        Target::Panel(panel, _) => Act::ClosePanel { panel },
                        Target::Project(project, _) => Act::CloseProject { project },
                        Target::Terminal(terminal, _) => Act::CloseTerminal { terminal },
                        Target::Session(name) => {
                            if self.mirror.session_name() == Some(name.as_str()) {
                                Act::KillSession
                            } else {
                                return vec![
                                    self.request(Request::KillSession { name }, Req::Kill),
                                ];
                            }
                        }
                    };
                    vec![self.act(action, None)]
                }
                KeyCode::Char('n') | KeyCode::Esc if plain => Vec::new(),
                _ => {
                    self.modal = Some(Modal::Confirm(confirm));
                    Vec::new()
                }
            },
            Modal::Prompt(mut prompt) => match key.code {
                KeyCode::Esc if plain => Vec::new(),
                KeyCode::Enter => self.submit(prompt),
                _ => {
                    prompt.editor.on_key(&key);
                    self.modal = Some(Modal::Prompt(prompt));
                    Vec::new()
                }
            },
        }
    }

    fn submit(&mut self, prompt: Prompt) -> Vec<Out> {
        let text = prompt.editor.text();
        match prompt.kind.clone() {
            PromptKind::ProjectDir => {
                let root = resolve_dir(&text, &self.cwd, self.home.as_deref());
                let name = default_project_name(&root, self.home.as_deref());
                let kind = PromptKind::ProjectName {
                    root,
                    dir_text: text,
                };
                self.open_prompt(kind, &name, None);
                Vec::new()
            }
            PromptKind::ProjectName { root, dir_text } => {
                let path_prompt = Prompt {
                    kind: PromptKind::ProjectDir,
                    editor: Editor::new(&dir_text),
                    target: None,
                };
                let action = Act::NewProject { name: text, root };
                vec![self.act(
                    action,
                    Some(Retry {
                        prompt,
                        path_prompt: Some(path_prompt),
                    }),
                )]
            }
            PromptKind::RenamePanel(panel) => {
                let action = Act::RenamePanel { panel, name: text };
                vec![self.act(
                    action,
                    Some(Retry {
                        prompt,
                        path_prompt: None,
                    }),
                )]
            }
            PromptKind::NewSession => {
                let name = (!text.trim().is_empty()).then(|| text.clone());
                let request = Request::NewSession {
                    name,
                    cwd: self.cwd.clone(),
                };
                let retry = Retry {
                    prompt,
                    path_prompt: None,
                };
                vec![self.request(request, Req::Create(Some(retry)))]
            }
            PromptKind::RenameSession { from, current } => {
                let retry = Retry {
                    prompt,
                    path_prompt: None,
                };
                if current {
                    let action = Act::RenameSession { name: text };
                    vec![self.act(action, Some(retry))]
                } else {
                    let request = Request::RenameSession { from, to: text };
                    vec![self.request(request, Req::Rename(retry))]
                }
            }
            PromptKind::RenameProject(project) => {
                let action = Act::RenameProject {
                    project,
                    name: text,
                };
                vec![self.act(
                    action,
                    Some(Retry {
                        prompt,
                        path_prompt: None,
                    }),
                )]
            }
        }
    }

    /// Sends an action; its answer comes back in order, with `retry` to reopen on an error.
    pub(crate) fn act(&mut self, action: Act, retry: Option<Retry>) -> Out {
        self.pending.push_back((retry, false));
        Out::Send(ClientMsg::Action(action))
    }

    /// Like [`App::act`], for an action whose refusal is not shown.
    pub(crate) fn act_silent(&mut self, action: Act) -> Out {
        self.pending.push_back((None, true));
        Out::Send(ClientMsg::Action(action))
    }

    /// The daemon's answer to the oldest action sent.
    pub(crate) fn action_result(
        &mut self,
        result: Result<(), ActionError>,
        now: std::time::Instant,
    ) {
        let (retry, silent) = self.pending.pop_front().unwrap_or((None, false));
        // The daemon's view came before the move's answer: the preview ends with it, and a
        // refused move falls back to the daemon's order.
        self.row_drop = match self.row_drop {
            Some((_, 0)) => {
                self.dirty = true;
                None
            }
            Some((reorder, ahead)) => Some((reorder, ahead - 1)),
            None => None,
        };
        match self.launch_result {
            Some(0) => {
                self.launch_result = None;
                if result.is_ok() {
                    self.sidebar_focus = false;
                }
            }
            Some(n) => self.launch_result = Some(n - 1),
            None => {}
        }
        let Err(error) = result else { return };
        if silent {
            return;
        }
        self.set_flash(error.to_string(), now, true);
        if let (Some(retry), None) = (retry, &self.modal) {
            let path_problem = matches!(
                error,
                ActionError::NotADirectory(_) | ActionError::NotAbsolute(_)
            );
            let prompt = match retry.path_prompt {
                Some(path) if path_problem => path,
                _ => retry.prompt,
            };
            self.modal = Some(Modal::Prompt(prompt));
        }
    }

    fn has_terminal(&self, id: TerminalId) -> bool {
        self.mirror.view().is_some_and(|v| {
            v.projects
                .iter()
                .flat_map(|p| &p.panels)
                .flat_map(|p| &p.terminals)
                .any(|t| t.id == id)
        })
    }

    /// Closes a dialog whose panel or project is gone, with a flash.
    pub(crate) fn check_modal_target(&mut self, now: std::time::Instant) {
        let target = match &self.modal {
            Some(Modal::Prompt(p)) => p.target.clone(),
            Some(Modal::Confirm(c)) => Some(c.target.clone()),
            _ => None,
        };
        let text = match target {
            Some(Target::Panel(id, name)) if self.mirror.find_panel(id).is_none() => {
                format!("panel \"{name}\" was closed")
            }
            Some(Target::Project(id, name)) if self.mirror.find_project(id).is_none() => {
                format!("space \"{name}\" was closed")
            }
            Some(Target::Terminal(id, index)) if !self.has_terminal(id) => {
                format!("terminal {index} was closed")
            }
            _ => return,
        };
        self.modal = None;
        self.set_flash(text, now, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deck_core::keys::Modifiers;

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, Modifiers::NONE)
    }

    fn type_str(e: &mut Editor, s: &str) {
        for c in s.chars() {
            e.on_key(&k(KeyCode::Char(c)));
        }
    }

    #[test]
    fn the_editor_inserts_moves_and_deletes() {
        let mut e = Editor::new("abc");
        assert_eq!(e.split(), ("abc".into(), String::new()));
        e.on_key(&k(KeyCode::Left));
        e.on_key(&k(KeyCode::Char('X')));
        assert_eq!(e.text(), "abXc");
        assert_eq!(e.split(), ("abX".into(), "c".into()));
        e.on_key(&k(KeyCode::Home));
        e.on_key(&k(KeyCode::Delete));
        assert_eq!(e.text(), "bXc");
        e.on_key(&k(KeyCode::Backspace));
        assert_eq!(e.text(), "bXc");
        e.on_key(&k(KeyCode::End));
        e.on_key(&k(KeyCode::Backspace));
        assert_eq!(e.text(), "bX");
        e.on_key(&k(KeyCode::Right));
        e.on_key(&k(KeyCode::Delete));
        assert_eq!(e.text(), "bX");
        type_str(&mut e, "é日");
        assert_eq!(e.text(), "bXé日");
        e.on_key(&k(KeyCode::Left));
        e.on_key(&k(KeyCode::Backspace));
        assert_eq!(e.text(), "bX日");
    }

    #[test]
    fn ctrl_u_clears_and_other_chords_edit_nothing() {
        let mut e = Editor::new("abc");
        assert!(e.on_key(&KeyEvent::new(KeyCode::Char('u'), Modifiers::CTRL)));
        assert_eq!(e.text(), "");
        assert!(!e.on_key(&KeyEvent::new(KeyCode::Char('a'), Modifiers::CTRL)));
        // The prefix, Alt and Space, inserts nothing.
        assert!(!e.on_key(&KeyEvent::new(KeyCode::Char(' '), Modifiers::ALT)));
        assert!(!e.on_key(&k(KeyCode::Up)));
        assert!(!e.on_key(&k(KeyCode::Enter)));
        assert_eq!(e.text(), "");
    }

    #[test]
    fn pasted_text_drops_newlines() {
        let mut e = Editor::new("a");
        e.insert_str("b\nc\r\nd\te");
        assert_eq!(e.text(), "abcde");
    }

    #[test]
    fn directories_resolve_on_the_client() {
        let cwd = Path::new("/work/proj");
        let home = Some(Path::new("/home/me"));
        let r = |t: &str| resolve_dir(t, cwd, home);
        assert_eq!(r("~"), Path::new("/home/me"));
        assert_eq!(r("~/code"), Path::new("/home/me/code"));
        assert_eq!(r("/srv/api/"), Path::new("/srv/api"));
        assert_eq!(r("sub"), Path::new("/work/proj/sub"));
        assert_eq!(r("../other"), Path::new("/work/other"));
        assert_eq!(r("./a/./b"), Path::new("/work/proj/a/b"));
        assert_eq!(r(""), cwd);
        assert_eq!(r("  /x  "), Path::new("/x"));
        assert_eq!(resolve_dir("~", cwd, None), Path::new("/work/proj/~"));
    }
}
