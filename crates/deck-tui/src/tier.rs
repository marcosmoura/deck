//! Which form of icon the outer terminal gets: images, Nerd Font glyphs or Unicode glyphs.
//!
//! The decision is made in [`crate::caps`] from the terminal's answers, once per client run.

use deck_core::icons::{self, Icon};

/// The best form of icon the outer terminal is known to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Images for the agents (kitty graphics), Nerd Font glyphs for the other icons.
    Image,
    /// Nerd Font glyphs: the terminal bundles the symbols.
    Nerd,
    /// Unicode glyphs every font has.
    Unicode,
}

impl Tier {
    /// The glyph of `icon` in this tier: Unicode, or Nerd Font in the `Nerd` and `Image` tiers
    /// (an icon without an image is its Nerd glyph); `frame` picks the spinner's.
    pub fn glyph(self, icon: Icon, frame: usize) -> &'static str {
        match self {
            Self::Unicode => icons::unicode(icon, frame),
            Self::Nerd | Self::Image => icons::nerd(icon, frame),
        }
    }
}

impl Tier {
    /// The tier once the font check answered: `Unicode` becomes `Nerd` when it found Nerd Font
    /// glyphs; the other tiers stay (the tier never downgrades).
    pub fn with_font(self, found: bool) -> Self {
        if found && self == Self::Unicode {
            Self::Nerd
        } else {
            self
        }
    }
}
