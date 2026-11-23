//! Every icon deck draws, with its fixed Unicode and Nerd Font glyphs, and which icon a panel
//! shows: a pure choice from its agent and its foreground program.

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

    /// The icon's name.
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

/// Every icon deck draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    /// What a panel's program is.
    Kind(IconKind),
    /// A space that shows its panels.
    Expanded,
    /// A space that hides its panels.
    Collapsed,
    /// The caret that marks where a dragged row would land.
    DropTarget,
    /// A working agent.
    Spinner,
    /// An agent that needs the user.
    Waiting,
    /// An agent that finished its turn.
    Idle,
    /// An agent found by its process that is not printing.
    Quiet,
    /// A panel whose agent needed the user or finished while nobody looked.
    Unseen,
    /// The `New` and `New space` chips of the sidebar.
    New,
    /// The `Resume` chip of the sidebar.
    Resume,
}

/// The frames of the spinner in the Unicode tier.
const UNICODE_SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// The frames of the spinner in the Nerd tier: nf-extra-progress_spinner_1 to 6.
const NERD_SPINNER: [&str; 6] = [
    "\u{ee06}", "\u{ee07}", "\u{ee08}", "\u{ee09}", "\u{ee0a}", "\u{ee0b}",
];

impl Icon {
    /// Every icon.
    pub const ALL: [Self; 21] = [
        Self::Kind(IconKind::Claude),
        Self::Kind(IconKind::Codex),
        Self::Kind(IconKind::OpenCode),
        Self::Kind(IconKind::Terminal),
        Self::Kind(IconKind::Editor),
        Self::Kind(IconKind::Git),
        Self::Kind(IconKind::Monitor),
        Self::Kind(IconKind::Remote),
        Self::Kind(IconKind::Build),
        Self::Kind(IconKind::Pager),
        Self::Kind(IconKind::Repl),
        Self::Expanded,
        Self::Collapsed,
        Self::DropTarget,
        Self::Spinner,
        Self::Waiting,
        Self::Idle,
        Self::Quiet,
        Self::Unseen,
        Self::New,
        Self::Resume,
    ];

    /// How many spinner frames the Unicode glyphs have.
    pub const UNICODE_FRAMES: usize = UNICODE_SPINNER.len();
    /// How many spinner frames the Nerd glyphs have.
    pub const NERD_FRAMES: usize = NERD_SPINNER.len();
}

/// The Unicode glyph of an icon, one cell wide; `frame` picks the spinner frame (modulo the
/// frame count) and is ignored by every other icon.
pub fn unicode(icon: Icon, frame: usize) -> &'static str {
    match icon {
        Icon::Kind(kind) => match kind {
            IconKind::Claude => "✻",
            IconKind::Codex => "◎",
            IconKind::OpenCode => "◇",
            IconKind::Terminal => "❯",
            IconKind::Editor => "✎",
            IconKind::Git => "±",
            IconKind::Monitor => "◔",
            IconKind::Remote => "⇄",
            IconKind::Build => "▲",
            IconKind::Pager => "¶",
            IconKind::Repl => "λ",
        },
        Icon::Expanded => "▾",
        Icon::Collapsed => "▸",
        Icon::DropTarget => "➤",
        Icon::Spinner => UNICODE_SPINNER
            .get(frame % UNICODE_SPINNER.len())
            .copied()
            .unwrap_or_default(),
        Icon::Waiting => "!",
        Icon::Idle => "●",
        Icon::Quiet => "○",
        Icon::Unseen => "•",
        Icon::New => "+",
        Icon::Resume => "↻",
    }
}

/// The Nerd Font glyph of an icon, one cell wide, checked against Nerd Fonts'
/// `glyphnames.json` (3.5.1); `frame` as for [`unicode`]. Nerd Fonts has no OpenCode logo:
/// `▣` stands in.
pub fn nerd(icon: Icon, frame: usize) -> &'static str {
    match icon {
        Icon::Kind(kind) => match kind {
            IconKind::Claude => "\u{ec82}",   // nf-cod-claude
            IconKind::Codex => "\u{ec81}",    // nf-cod-openai
            IconKind::OpenCode => "▣",        // no Nerd Font glyph
            IconKind::Terminal => "\u{f120}", // nf-fa-terminal
            IconKind::Editor => "\u{f040}",   // nf-fa-pencil
            IconKind::Git => "\u{f126}",      // nf-fa-code_fork
            IconKind::Monitor => "\u{f0e4}",  // nf-fa-tachometer
            IconKind::Remote => "\u{f0ec}",   // nf-fa-exchange
            IconKind::Build => "\u{f013}",    // nf-fa-cog
            IconKind::Pager => "\u{f02d}",    // nf-fa-book
            IconKind::Repl => "\u{f121}",     // nf-fa-code
        },
        Icon::Expanded => "\u{eab4}",   // nf-cod-chevron_down
        Icon::Collapsed => "\u{eab6}",  // nf-cod-chevron_right
        Icon::DropTarget => "\u{f0da}", // nf-fa-caret_right
        Icon::Spinner => NERD_SPINNER
            .get(frame % NERD_SPINNER.len())
            .copied()
            .unwrap_or_default(),
        Icon::Waiting => "\u{f06a}", // nf-fa-circle_exclamation
        Icon::Idle => "\u{f058}",    // nf-fa-check_circle
        Icon::Quiet => "\u{f10c}",   // nf-fa-circle_o
        Icon::Unseen => "\u{f444}",  // nf-oct-dot_fill
        Icon::New => "\u{ea60}",     // nf-cod-add
        Icon::Resume => "\u{ea82}",  // nf-cod-history
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

    #[test]
    fn every_glyph_of_every_frame_is_one_cell_wide() {
        use unicode_width::UnicodeWidthStr;
        for icon in Icon::ALL {
            for frame in 0..20 {
                assert_eq!(unicode(icon, frame).width(), 1, "unicode {icon:?} {frame}");
                assert_eq!(nerd(icon, frame).width(), 1, "nerd {icon:?} {frame}");
            }
        }
    }

    #[test]
    fn the_spinner_wraps_at_its_frame_count_and_others_ignore_the_frame() {
        assert_eq!(Icon::UNICODE_FRAMES, 10);
        assert_eq!(Icon::NERD_FRAMES, 6);
        assert_eq!(unicode(Icon::Spinner, 0), "⠋");
        assert_eq!(unicode(Icon::Spinner, 10), "⠋");
        assert_eq!(unicode(Icon::Spinner, 12), "⠹");
        assert_eq!(nerd(Icon::Spinner, 5), "\u{ee0b}");
        assert_eq!(nerd(Icon::Spinner, 6), "\u{ee06}");
        assert_eq!(nerd(Icon::Idle, 3), nerd(Icon::Idle, 0));
        assert_eq!(unicode(Icon::Idle, 3), unicode(Icon::Idle, 0));
    }

    #[test]
    fn the_glyphs_are_the_documented_ones() {
        assert_eq!(unicode(Icon::Kind(IconKind::Claude), 0), "✻");
        assert_eq!(nerd(Icon::Kind(IconKind::Claude), 0), "\u{ec82}");
        assert_eq!(nerd(Icon::Kind(IconKind::OpenCode), 0), "▣");
        assert_eq!(unicode(Icon::DropTarget, 0), "➤");
        assert_eq!(nerd(Icon::Expanded, 0), "\u{eab4}");
        assert_eq!(nerd(Icon::Collapsed, 0), "\u{eab6}");
        assert_eq!(unicode(Icon::New, 0), "+");
        assert_eq!(nerd(Icon::New, 0), "\u{ea60}");
        assert_eq!(unicode(Icon::Resume, 0), "↻");
        assert_eq!(nerd(Icon::Resume, 0), "\u{ea82}");
        assert_eq!(nerd(Icon::Waiting, 0), "\u{f06a}");
        assert_eq!(nerd(Icon::Idle, 0), "\u{f058}");
        assert_eq!(nerd(Icon::Quiet, 0), "\u{f10c}");
        assert_eq!(nerd(Icon::Unseen, 0), "\u{f444}");
    }

    #[test]
    fn the_program_icons_are_distinct_in_each_tier() {
        for glyph in [unicode, nerd] {
            let mut seen = Vec::new();
            for kind in IconKind::ALL {
                let g = glyph(Icon::Kind(kind), 0);
                assert!(!seen.contains(&g), "{g} repeats");
                seen.push(g);
            }
        }
    }
}
