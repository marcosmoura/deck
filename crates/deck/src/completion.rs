//! Shell completion: `COMPLETE=<shell> deck` prints the script a shell sources, and the shell
//! then runs `deck` again to ask for the candidates of each word.
//!
//! Sessions, spaces, panels and terminals come from the running daemon; with no daemon (or
//! any error) they complete to nothing. Completion never starts a daemon.

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::Path;

use clap::CommandFactory;
use clap_complete::env::{Bash, EnvCompleter, Fish, Shells, Zsh};
use clap_complete::{CompleteEnv, CompletionCandidate};
use deck_core::model::SessionView;

use crate::Cli;
use crate::tree::{Api, printable};

/// Answers a completion request and exits when `COMPLETE` is set; returns otherwise.
pub fn complete() {
    CompleteEnv::with_factory(Cli::command)
        .shells(Shells(&[&PlainBash, &Zsh, &Fish]))
        .complete();
}

/// Bash, offered only the candidates it can insert as they are.
///
/// clap's bash script splits the answer on one separator, expands globs in each word and
/// inserts the result unquoted, and its adapter writes the values raw: a directory called
/// `src$(touch x)` would complete to a command substitution that runs on Enter. zsh and fish
/// quote what they insert; for bash, a value that would need quoting is left out.
struct PlainBash;

impl EnvCompleter for PlainBash {
    fn name(&self) -> &'static str {
        Bash.name()
    }

    fn is(&self, name: &str) -> bool {
        Bash.is(name)
    }

    fn write_registration(
        &self,
        var: &str,
        name: &str,
        bin: &str,
        completer: &str,
        buf: &mut dyn Write,
    ) -> io::Result<()> {
        Bash.write_registration(var, name, bin, completer, buf)
    }

    /// As clap's adapter: the word's index and the separator come from the script.
    fn write_complete(
        &self,
        cmd: &mut clap::Command,
        args: Vec<OsString>,
        current_dir: Option<&Path>,
        buf: &mut dyn Write,
    ) -> io::Result<()> {
        let index = std::env::var("_CLAP_COMPLETE_INDEX")
            .ok()
            .and_then(|index| index.parse().ok())
            .unwrap_or_default();
        let separator = std::env::var("_CLAP_IFS").unwrap_or_else(|_| "\n".into());
        let candidates = clap_complete::engine::complete(cmd, args, index, current_dir)?;
        buf.write_all(plain_values(&candidates).join(&separator).as_bytes())
    }
}

/// The values of `candidates` that bash inserts unchanged when they are not quoted.
fn plain_values(candidates: &[CompletionCandidate]) -> Vec<&str> {
    candidates
        .iter()
        .filter_map(|candidate| candidate.get_value().to_str())
        .filter(|value| value.chars().all(plain_in_bash))
        .collect()
}

/// A character bash never treats specially in a word: no quoting, expansion, glob, history
/// or word separator, and nothing a terminal acts on.
fn plain_in_bash(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_alphanumeric() || "_-./:=+@,%~".contains(c)
    } else {
        !c.is_control() && !c.is_whitespace()
    }
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

/// A candidate with its description. zsh and fish print the description, and a program's
/// name in it comes from `argv[0]`: it leaves without control characters.
fn candidate(value: impl ToString, help: String) -> CompletionCandidate {
    CompletionCandidate::new(value.to_string()).help(Some(printable(&help).into()))
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
    fn help_carries_no_control_characters() {
        let mut tree = view();
        tree[0].projects[0].panels[0].terminals[0].program = "x\x1b]52;c;QQ==\x07\ty".into();
        assert_eq!(
            pairs(&terminal_candidates(&tree, "terminal ")),
            [(
                "7".into(),
                "terminal x?]52;c;QQ==??y in build (main)".into()
            )]
        );
    }

    #[test]
    fn bash_gets_only_candidates_that_need_no_quoting() {
        let dir = tempfile::tempdir().unwrap();
        let hostile = [
            "a b",
            "src$(touch PWNED)",
            "vt\x0bsplit",
            "[p-w]*",
            "n\u{a0}b",
        ];
        for name in ["ok", "日本"].iter().chain(&hostile) {
            std::fs::create_dir(dir.path().join(name)).unwrap();
        }
        let line = ["deck", "new-terminal", "--cwd", ""].map(OsString::from);
        let all = clap_complete::engine::complete(
            &mut Cli::command(),
            line.to_vec(),
            3,
            Some(dir.path()),
        )
        .unwrap();
        // The engine offers every directory; bash gets the ones that are safe unquoted.
        let offered: Vec<_> = all.iter().map(|c| c.get_value().to_owned()).collect();
        for name in hostile {
            assert!(
                offered.contains(&OsString::from(format!("{name}/"))),
                "{offered:?}"
            );
        }
        let mut plain = plain_values(&all);
        plain.sort_unstable();
        assert_eq!(plain, [".", "ok/", "日本/"]);

        // A session name may hold shell syntax.
        let mut tree = view();
        tree.push(SessionView {
            name: "$(x)".into(),
            ..tree[0].clone()
        });
        assert_eq!(plain_values(&session_candidates(&tree)), ["main"]);

        // Flags, ids and paths pass; everything bash would act on does not.
        for value in ["--cwd", "kill-session", "12", "/a/b.c/", "x=y:z,w+v@u%t~s"] {
            assert!(value.chars().all(plain_in_bash), "{value}");
        }
        for c in " \t\n\x0b\x1b\x7f\u{9b}$`\\\"'*?[]{}()<>|&;!#^".chars() {
            assert!(!plain_in_bash(c), "{c:?}");
        }
    }

    #[test]
    fn the_bash_script_is_claps_and_asks_for_bash() {
        let mut script = Vec::new();
        PlainBash
            .write_registration("COMPLETE", "deck", "deck", "deck", &mut script)
            .unwrap();
        let mut upstream = Vec::new();
        Bash.write_registration("COMPLETE", "deck", "deck", "deck", &mut upstream)
            .unwrap();
        assert_eq!(script, upstream);
        assert!(PlainBash.is(PlainBash.name()));
        assert!(
            String::from_utf8(script)
                .unwrap()
                .contains("COMPLETE=\"bash\"")
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
