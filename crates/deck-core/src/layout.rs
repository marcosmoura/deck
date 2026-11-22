//! Where things go in the outer window.

use crate::config::Config;

/// The panel area keeps at least this many columns, or the sidebar is hidden.
pub const MIN_PANEL_COLS: u16 = 20;

/// Columns the sidebar and its separator take in a window `cols` wide: none when the user
/// hid it (`shown` false) or when the panel area would be narrower than [`MIN_PANEL_COLS`].
pub fn sidebar_cols(cols: u16, sidebar_width: u16, shown: bool) -> u16 {
    if shown && cols >= sidebar_width + 1 + MIN_PANEL_COLS {
        sidebar_width + 1
    } else {
        0
    }
}

/// The panel area of an `outer` window (columns, rows): what is left of it after the
/// sidebar and the status-bar row. `sidebar_visible` says the user has not hidden the
/// sidebar; a window too narrow for it hides it anyway.
pub fn panel_area(outer: (u16, u16), config: &Config, sidebar_visible: bool) -> (u16, u16) {
    let (cols, rows) = outer;
    (
        cols - sidebar_cols(cols, config.sidebar_width, sidebar_visible),
        rows.saturating_sub(1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_area_leaves_the_sidebar_and_the_status_bar() {
        let config = Config::default();
        assert_eq!(panel_area((100, 24), &config, true), (71, 23));
        assert_eq!(panel_area((100, 24), &config, false), (100, 23));
        assert_eq!(panel_area((100, 0), &config, false), (100, 0));
    }

    #[test]
    fn a_narrow_window_hides_the_sidebar() {
        let config = Config::default();
        // 28 + separator + 20 columns is the narrowest that keeps it.
        assert_eq!(panel_area((49, 24), &config, true), (20, 23));
        assert_eq!(panel_area((48, 24), &config, true), (48, 23));
        let wide = Config {
            sidebar_width: 40,
            ..Config::default()
        };
        assert_eq!(panel_area((100, 24), &wide, true), (59, 23));
    }
}
