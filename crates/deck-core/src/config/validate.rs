//! `validate`: every problem of a config text, each with the line and column it is at.

use toml::de::DeTable;

use super::{ConfigError, load};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// One problem, at a 1-based line and column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub severity: Severity,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

/// What `validate` found, in file order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Report {
    pub findings: Vec<Finding>,
}

impl Report {
    pub fn has_errors(&self) -> bool {
        self.findings.iter().any(|f| f.severity == Severity::Error)
    }
}

/// The 1-based line and column (in characters) of a byte offset.
fn position(text: &str, offset: usize) -> (usize, usize) {
    let before = text.get(..offset).unwrap_or("");
    let line = before.matches('\n').count() + 1;
    let column = before
        .rsplit('\n')
        .next()
        .map_or(0, |last| last.chars().count())
        + 1;
    (line, column)
}

/// The byte offset of the value at `path` (`theme.accent`), or of its key when `key` is
/// set; the deepest part that exists when the path does not.
fn locate(root: &DeTable<'_>, path: &str, key: bool) -> usize {
    let parts: Vec<&str> = path.split('.').collect();
    let mut table = Some(root);
    let mut best = 0;
    for (i, part) in parts.iter().enumerate() {
        let Some(found) = table.and_then(|t| t.iter().find(|(k, _)| k.get_ref().as_ref() == *part))
        else {
            break;
        };
        let (name, value) = found;
        best = if key && i + 1 == parts.len() {
            name.span().start
        } else {
            value.span().start
        };
        table = value.get_ref().as_table();
    }
    best
}

/// Checks a config text: TOML syntax, bad values, unknown keys and key conflicts. Keys
/// before a syntax error are still checked.
pub fn validate(text: &str) -> Report {
    let mut findings = Vec::new();
    let mut push = |severity, offset: usize, message: String| {
        let (line, column) = position(text, offset);
        findings.push(Finding {
            severity,
            line,
            column,
            message,
        });
    };

    let body = match text.parse::<toml::Table>() {
        Ok(_) => text,
        Err(e) => {
            let start = e.span().map_or(0, |s| s.start).min(text.len());
            push(
                Severity::Error,
                start,
                format!("config is not valid TOML: {}", e.message()),
            );
            let line_start = text
                .get(..start)
                .map_or(0, |b| b.rfind('\n').map_or(0, |i| i + 1));
            let before = &text[..line_start];
            if before.parse::<toml::Table>().is_err() {
                return finish(findings);
            }
            before
        }
    };
    let (Ok(table), Ok(spans)) = (body.parse::<toml::Table>(), DeTable::parse(body)) else {
        return finish(findings);
    };
    let loaded = load(&table);
    for error in &loaded.errors {
        let (path, message) = match error {
            ConfigError::Value { key, .. } => (key.as_str(), error.to_string()),
            ConfigError::Syntax(_) => ("", error.to_string()),
        };
        push(
            Severity::Error,
            locate(spans.get_ref(), path, false),
            message,
        );
    }
    for warning in &loaded.warnings {
        push(
            Severity::Warning,
            locate(spans.get_ref(), &warning.key, true),
            warning.message.clone(),
        );
    }
    finish(findings)
}

fn finish(mut findings: Vec<Finding>) -> Report {
    findings.sort_by_key(|f| (f.line, f.column));
    Report { findings }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(report: &Report, severity: Severity, line: usize, column: usize) -> &Finding {
        report
            .findings
            .iter()
            .find(|f| f.severity == severity && f.line == line && f.column == column)
            .unwrap_or_else(|| panic!("no {severity:?} at {line}:{column} in {report:#?}"))
    }

    #[test]
    fn a_clean_file_and_an_empty_one_have_no_findings() {
        assert!(validate("").findings.is_empty());
        assert!(
            validate(super::super::DEFAULT_CONFIG_TOML)
                .findings
                .is_empty()
        );
    }

    #[test]
    fn each_kind_of_finding_has_its_line_and_column() {
        let text = "prefix = \"C-\"\n\
                    bogus = 1\n\
                    sidebar = true\n\
                    [theme]\n\
                    accent = \"purple\"\n\
                    [keys]\n\
                    teleport = \"t\"\n\
                    detach = \"n\"\n\
                    scroll = = \"x\"\n";
        let r = validate(text);
        assert!(r.has_errors());
        assert!(at(&r, Severity::Error, 1, 10).message.contains("`prefix`"));
        assert_eq!(
            at(&r, Severity::Warning, 2, 1).message,
            "unknown config key `bogus`"
        );
        assert!(
            at(&r, Severity::Error, 5, 10)
                .message
                .contains("`theme.accent`")
        );
        assert!(
            at(&r, Severity::Warning, 7, 1)
                .message
                .contains("keys.teleport")
        );
        let w = at(&r, Severity::Warning, 8, 1);
        assert!(w.message.contains("detach") && w.message.contains("next_panel"));
        assert!(w.message.contains("\"n\""));
        assert!(
            at(&r, Severity::Error, 9, 10)
                .message
                .contains("not valid TOML")
        );
        assert_eq!(r.findings.len(), 6, "{r:#?}");
    }

    #[test]
    fn two_actions_on_one_key_is_an_error_at_the_second_action() {
        let r = validate("[keys]\ndetach = \"n\"\nnext_panel = \"n\"\n");
        let e = at(&r, Severity::Error, 3, 14);
        assert!(e.message.contains("detach") && e.message.contains("next_panel"));
        assert!(e.message.contains("\"n\""));
    }

    #[test]
    fn only_an_unknown_key_is_a_warning() {
        let r = validate("colour = 1\n");
        assert!(!r.has_errors());
        assert_eq!(r.findings.len(), 1);
    }

    #[test]
    fn columns_count_characters_and_an_unreadable_prefix_reports_only_the_syntax() {
        let r = validate("[theme]\nactive = \"é\" bogus\n");
        assert_eq!(r.findings.len(), 1, "{r:#?}");
        assert_eq!(r.findings[0].line, 2);
        assert!(r.findings[0].column > 1);
        assert_eq!(position("é = 1\nx", 2), (1, 2));
    }
}
