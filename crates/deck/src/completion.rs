//! Shell completion: `COMPLETE=<shell> deck` prints the script a shell sources, and the shell
//! then runs `deck` again to ask for the candidates of each word.
//!
//! Sessions, spaces, panels and terminals come from the running daemon; with no daemon (or
//! any error) they complete to nothing. Completion never starts a daemon.

use clap::CommandFactory;
use clap_complete::env::{Bash, Fish, Shells, Zsh};
use clap_complete::{CompleteEnv, CompletionCandidate};
use deck_core::model::SessionView;

use crate::Cli;
use crate::tree::Api;

/// Answers a completion request and exits when `COMPLETE` is set; returns otherwise.
pub fn complete() {
    CompleteEnv::with_factory(Cli::command)
        .shells(Shells(&[&Bash, &Zsh, &Fish]))
        .complete();
}

/// The daemon's tree, empty when it cannot be read.
fn tree() -> Vec<SessionView> {
    Api::connect()
        .and_then(|mut api| api.tree(None))
        .unwrap_or_default()
}

/// Session names.
pub fn sessions() -> Vec<CompletionCandidate> {
    session_candidates(&tree())
}

/// Space ids.
pub fn spaces() -> Vec<CompletionCandidate> {
    space_candidates(&tree(), "")
}

/// Panel ids.
pub fn panels() -> Vec<CompletionCandidate> {
    panel_candidates(&tree(), "")
}

/// Terminal ids.
pub fn terminals() -> Vec<CompletionCandidate> {
    terminal_candidates(&tree(), "")
}

/// Terminal, panel and space ids, each labelled with its kind.
pub fn any_ids() -> Vec<CompletionCandidate> {
    let tree = tree();
    let mut out = terminal_candidates(&tree, "terminal ");
    out.extend(panel_candidates(&tree, "panel "));
    out.extend(space_candidates(&tree, "space "));
    out
}

fn candidate(value: impl ToString, help: String) -> CompletionCandidate {
    CompletionCandidate::new(value.to_string()).help(Some(help.into()))
}

fn session_candidates(tree: &[SessionView]) -> Vec<CompletionCandidate> {
    tree.iter()
        .map(|s| {
            let spaces = match s.projects.len() {
                1 => "1 space".to_owned(),
                n => format!("{n} spaces"),
            };
            let attached = if s.attached_clients > 0 {
                ", attached"
            } else {
                ""
            };
            candidate(&s.name, format!("{spaces}{attached}"))
        })
        .collect()
}

fn space_candidates(tree: &[SessionView], kind: &str) -> Vec<CompletionCandidate> {
    tree.iter()
        .flat_map(|s| {
            s.projects
                .iter()
                .map(move |p| candidate(p.id.0, format!("{kind}{} ({})", p.name, s.name)))
        })
        .collect()
}

fn panel_candidates(tree: &[SessionView], kind: &str) -> Vec<CompletionCandidate> {
    tree.iter()
        .flat_map(|s| {
            s.projects.iter().flat_map(move |p| {
                p.panels.iter().map(move |panel| {
                    let help = format!("{kind}{} in {} ({})", panel.name, p.name, s.name);
                    candidate(panel.id.0, help)
                })
            })
        })
        .collect()
}

fn terminal_candidates(tree: &[SessionView], kind: &str) -> Vec<CompletionCandidate> {
    tree.iter()
        .flat_map(|s| {
            s.projects.iter().flat_map(move |p| {
                p.panels.iter().flat_map(move |panel| {
                    panel.terminals.iter().map(move |t| {
                        let help = format!("{kind}{} in {} ({})", t.program, panel.name, s.name);
                        candidate(t.id.0, help)
                    })
                })
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use deck_core::layout::Ratios;
    use deck_core::model::{
        NameSource, PanelId, PanelView, ProjectId, ProjectView, SessionId, TerminalId, TerminalView,
    };

    fn view() -> Vec<SessionView> {
        vec![SessionView {
            id: SessionId(1),
            name: "main".into(),
            active_project: Some(ProjectId(3)),
            active_panel: Some(PanelId(5)),
            layout_viewport: None,
            attached_clients: 1,
            projects: vec![ProjectView {
                id: ProjectId(3),
                name: "deck".into(),
                root: "/r".into(),
                collapsed: false,
                active_panel: Some(PanelId(5)),
                panels: vec![PanelView {
                    id: PanelId(5),
                    name: "build".into(),
                    name_source: NameSource::Manual,
                    focused_terminal: Some(TerminalId(7)),
                    ratios: Ratios::default_for(1),
                    unseen: false,
                    terminals: vec![TerminalView {
                        id: TerminalId(7),
                        program: "zsh".into(),
                        title: None,
                        cwd: None,
                        exit_status: None,
                        pid: None,
                        keep: false,
                        agent: None,
                        busy: false,
                    }],
                }],
            }],
        }]
    }

    fn pairs(candidates: &[CompletionCandidate]) -> Vec<(String, String)> {
        candidates
            .iter()
            .map(|c| {
                (
                    c.get_value().to_string_lossy().into_owned(),
                    c.get_help().map(ToString::to_string).unwrap_or_default(),
                )
            })
            .collect()
    }

    fn values(line: &[&str]) -> Vec<String> {
        let args = line.iter().map(Into::into).collect::<Vec<_>>();
        clap_complete::engine::complete(&mut Cli::command(), args, line.len() - 1, None)
            .unwrap_or_default()
            .iter()
            .map(|c| c.get_value().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn sessions_complete_to_their_names() {
        assert_eq!(
            pairs(&session_candidates(&view())),
            [("main".into(), "1 space, attached".into())]
        );
    }

    #[test]
    fn ids_carry_what_they_name() {
        let tree = view();
        assert_eq!(
            pairs(&space_candidates(&tree, "")),
            [("3".into(), "deck (main)".into())]
        );
        assert_eq!(
            pairs(&panel_candidates(&tree, "panel ")),
            [("5".into(), "panel build in deck (main)".into())]
        );
        assert_eq!(
            pairs(&terminal_candidates(&tree, "")),
            [("7".into(), "zsh in build (main)".into())]
        );
    }

    #[test]
    fn subcommands_and_flags_complete_without_the_hidden_ones() {
        let all = values(&["deck", ""]);
        assert!(all.contains(&"kill-session".to_owned()), "{all:?}");
        assert!(all.contains(&"new-space".to_owned()), "{all:?}");
        for hidden in ["daemon", "_probe", "agent-event"] {
            assert!(!all.contains(&hidden.to_owned()), "{all:?}");
        }
        assert_eq!(values(&["deck", "kill-ses"]), ["kill-session"]);
        assert!(values(&["deck", "hooks", "install", ""]).contains(&"claude".to_owned()));
        assert!(values(&["deck", "send", "--term"]).contains(&"--terminal".to_owned()));
    }
}
