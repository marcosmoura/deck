//! Which icon a panel shows: a pure choice from its agent and its foreground program. The
//! glyph of each kind is theme (`[theme.icon_glyphs]`).

use crate::agents::AgentKind;

/// What a panel's icon stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconKind {
    Claude,
    Codex,
    OpenCode,
    Terminal,
    Editor,
    Git,
    Monitor,
    Remote,
    Build,
    Pager,
    Repl,
}

impl IconKind {
    pub const ALL: [Self; 11] = [
        Self::Claude,
        Self::Codex,
        Self::OpenCode,
        Self::Terminal,
        Self::Editor,
        Self::Git,
        Self::Monitor,
        Self::Remote,
        Self::Build,
        Self::Pager,
        Self::Repl,
    ];

    /// The name used in `[theme.icon_glyphs]`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::Terminal => "terminal",
            Self::Editor => "editor",
            Self::Git => "git",
            Self::Monitor => "monitor",
            Self::Remote => "remote",
            Self::Build => "build",
            Self::Pager => "pager",
            Self::Repl => "repl",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.name() == name)
    }
}

/// The programs of each kind that is chosen by program; everything else is `terminal`.
const PROGRAMS: [(IconKind, &[&str]); 7] = [
    (
        IconKind::Editor,
        &[
            "vi", "vim", "nvim", "hx", "helix", "nano", "emacs", "micro", "kak",
        ],
    ),
    (IconKind::Git, &["git", "lazygit", "tig", "gitui"]),
    (IconKind::Monitor, &["top", "htop", "btop", "glances"]),
    (IconKind::Remote, &["ssh", "mosh", "et"]),
    (
        IconKind::Build,
        &["cargo", "make", "just", "npm", "pnpm", "yarn", "go"],
    ),
    (IconKind::Pager, &["less", "more", "man", "bat"]),
    (
        IconKind::Repl,
        &["python", "python3", "node", "irb", "ghci", "iex"],
    ),
];

/// The icon of a terminal: its agent's own, else the one of its program (compared without
/// regard to case), else the terminal icon.
pub fn icon_for(agent: Option<AgentKind>, program: Option<&str>) -> IconKind {
    if let Some(agent) = agent {
        return match agent {
            AgentKind::Claude => IconKind::Claude,
            AgentKind::Codex => IconKind::Codex,
            AgentKind::OpenCode => IconKind::OpenCode,
        };
    }
    let Some(program) = program else {
        return IconKind::Terminal;
    };
    PROGRAMS
        .iter()
        .find(|(_, names)| names.iter().any(|n| n.eq_ignore_ascii_case(program)))
        .map_or(IconKind::Terminal, |(kind, _)| *kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_agent_has_its_own_icon() {
        assert_eq!(icon_for(Some(AgentKind::Claude), None), IconKind::Claude);
        assert_eq!(
            icon_for(Some(AgentKind::Codex), Some("sh")),
            IconKind::Codex
        );
        assert_eq!(
            icon_for(Some(AgentKind::OpenCode), Some("")),
            IconKind::OpenCode
        );
    }

    #[test]
    fn an_agent_wins_over_its_program() {
        assert_eq!(
            icon_for(Some(AgentKind::Claude), Some("node")),
            IconKind::Claude
        );
    }

    #[test]
    fn every_program_of_the_table_has_its_kind() {
        let table: [(IconKind, &[&str]); 8] = [
            (
                IconKind::Terminal,
                &["sh", "bash", "zsh", "fish", "dash", "nu", "ksh", "tcsh"],
            ),
            (
                IconKind::Editor,
                &[
                    "vi", "vim", "nvim", "hx", "helix", "nano", "emacs", "micro", "kak",
                ],
            ),
            (IconKind::Git, &["git", "lazygit", "tig", "gitui"]),
            (IconKind::Monitor, &["top", "htop", "btop", "glances"]),
            (IconKind::Remote, &["ssh", "mosh", "et"]),
            (
                IconKind::Build,
                &["cargo", "make", "just", "npm", "pnpm", "yarn", "go"],
            ),
            (IconKind::Pager, &["less", "more", "man", "bat"]),
            (
                IconKind::Repl,
                &["python", "python3", "node", "irb", "ghci", "iex"],
            ),
        ];
        for (kind, programs) in table {
            for program in programs {
                assert_eq!(icon_for(None, Some(program)), kind, "{program}");
            }
        }
    }

    #[test]
    fn the_program_is_compared_without_case() {
        assert_eq!(icon_for(None, Some("NVIM")), IconKind::Editor);
        assert_eq!(icon_for(None, Some("Htop")), IconKind::Monitor);
    }

    #[test]
    fn unknown_or_missing_programs_are_a_terminal() {
        assert_eq!(icon_for(None, Some("weirdtool")), IconKind::Terminal);
        assert_eq!(icon_for(None, Some("")), IconKind::Terminal);
        assert_eq!(icon_for(None, None), IconKind::Terminal);
    }

    #[test]
    fn names_round_trip() {
        for kind in IconKind::ALL {
            assert_eq!(IconKind::parse(kind.name()), Some(kind));
        }
        assert_eq!(IconKind::parse("bogus"), None);
    }
}
