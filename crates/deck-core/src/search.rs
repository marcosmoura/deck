//! Finding a query in one row of a terminal's text: the rule the daemon's history search and
//! the client's highlights share, so a highlight lies where the search finds a match.
//!
//! A row is a list of units: a character with its combining marks, in one cell or, for a
//! wide character, in two. Matching is plain text, by unit, within the row, and smart case.

/// The characters of `text`, lowercased when `fold`.
fn fold_chars(text: &str, fold: bool) -> Vec<char> {
    if fold {
        text.chars().flat_map(char::to_lowercase).collect()
    } else {
        text.chars().collect()
    }
}

/// The columns `(first, last)`, both included, of each occurrence of `query` in a row.
///
/// `units` are the row's units as `(col, end, text)`: the first and last column a unit covers
/// and its text (`" "` for an empty cell); a wide character's second cell is not a unit. The
/// row's trailing blank units are not part of its text. Smart case: a query with no capital
/// letter ignores case, one with a capital letter matches exactly. A match starts at the
/// first character of a unit and ends at the last of one, so `e` is not found inside `é`
/// written as `e` and a combining accent. An empty query finds nothing.
pub fn find<'a>(
    units: impl IntoIterator<Item = (u16, u16, &'a str)>,
    query: &str,
) -> Vec<(u16, u16)> {
    if query.is_empty() {
        return Vec::new();
    }
    let fold = !query.chars().any(char::is_uppercase);
    let needle = fold_chars(query, fold);
    let mut units: Vec<(u16, u16, &str)> = units.into_iter().collect();
    while units.last().is_some_and(|(_, _, text)| *text == " ") {
        units.pop();
    }
    // Each character, with whether it starts and whether it ends its unit, and the unit.
    let mut hay: Vec<(char, bool, bool, usize)> = Vec::new();
    for (index, (_, _, text)) in units.iter().enumerate() {
        let chars = fold_chars(text, fold);
        let len = chars.len();
        hay.extend(
            chars
                .into_iter()
                .enumerate()
                .map(|(i, c)| (c, i == 0, i + 1 == len, index)),
        );
    }
    let mut out = Vec::new();
    for start in 0..hay.len() {
        let Some(window) = hay.get(start..start + needle.len()) else {
            break;
        };
        if !window.iter().zip(&needle).all(|(h, n)| h.0 == *n) {
            continue;
        }
        if let (Some(first), Some(last)) = (window.first(), window.last())
            && first.1
            && last.2
            && let (Some(a), Some(b)) = (units.get(first.3), units.get(last.3))
        {
            out.push((a.0, b.1));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Units of one cell each from `text`.
    fn row(text: &str) -> Vec<(u16, u16, String)> {
        text.chars()
            .enumerate()
            .map(|(i, c)| {
                let col = u16::try_from(i).unwrap();
                (col, col, c.to_string())
            })
            .collect()
    }

    fn hits(units: &[(u16, u16, String)], query: &str) -> Vec<(u16, u16)> {
        find(units.iter().map(|(a, b, t)| (*a, *b, t.as_str())), query)
    }

    #[test]
    fn smart_case_ignores_case_until_the_query_has_a_capital() {
        let units = row("Error: an error, ERROR");
        assert_eq!(hits(&units, "error"), [(0, 4), (10, 14), (17, 21)]);
        assert_eq!(hits(&units, "Error"), [(0, 4)]);
        assert_eq!(hits(&units, "eRror"), Vec::<(u16, u16)>::new());
        assert_eq!(hits(&units, ""), Vec::<(u16, u16)>::new());
    }

    #[test]
    fn a_wide_character_covers_two_columns_and_a_match_ends_on_the_last() {
        // 日本語 in columns 0-5, a blank, then `の` in 7-8.
        let units = vec![
            (0, 1, "日".to_string()),
            (2, 3, "本".to_string()),
            (4, 5, "語".to_string()),
            (6, 6, " ".to_string()),
            (7, 8, "の".to_string()),
        ];
        assert_eq!(hits(&units, "本語"), [(2, 5)]);
        assert_eq!(hits(&units, "語 の"), [(4, 8)]);
    }

    #[test]
    fn a_combining_mark_belongs_to_its_base_character() {
        let units = vec![
            (0, 0, "e\u{301}".to_string()),
            (1, 1, "c".to_string()),
            (2, 2, "e".to_string()),
        ];
        // `e` alone is not found inside the first unit; the whole unit is.
        assert_eq!(hits(&units, "e"), [(2, 2)]);
        assert_eq!(hits(&units, "e\u{301}c"), [(0, 1)]);
        // A match cannot begin on a mark.
        assert_eq!(hits(&units, "\u{301}c"), Vec::<(u16, u16)>::new());
    }

    #[test]
    fn trailing_blanks_are_not_text_but_inner_ones_are() {
        let units = row("a b   ");
        assert_eq!(hits(&units, "a b"), [(0, 2)]);
        assert_eq!(hits(&units, "b "), Vec::<(u16, u16)>::new());
        assert_eq!(hits(&units, " "), [(1, 1)]);
    }
}
