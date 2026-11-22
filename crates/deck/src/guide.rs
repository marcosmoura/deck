//! `deck guide`: the agent skill's text, for agents without skill support.

use std::process::ExitCode;

/// The whole skill file, front matter included.
const SKILL: &str = include_str!("../../../skills/deck/SKILL.md");

/// The skill without its front matter.
fn body() -> &'static str {
    SKILL
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .map_or(SKILL, |(_, body)| body)
        .trim_start_matches('\n')
}

pub fn run() -> ExitCode {
    print!("{}", body());
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Cli;
    use clap::Parser;

    const README: &str = include_str!("../../../README.md");

    #[test]
    fn the_body_has_no_front_matter() {
        let text = body();
        assert!(text.starts_with("# deck"), "{text}");
        assert!(!text.contains("name: deck"));
    }

    /// The commands of every fenced code block, as argv: quotes group words (and may
    /// span lines), an unquoted newline ends a command, `#` starts a comment and `>`
    /// ends the arguments (a redirection).
    fn commands(markdown: &str) -> Vec<Vec<String>> {
        let mut out = Vec::new();
        let mut in_fence = false;
        let mut block = String::new();
        for line in markdown.lines() {
            if line.trim_start().starts_with("```") {
                if in_fence {
                    out.extend(split_block(&block));
                    block.clear();
                }
                in_fence = !in_fence;
            } else if in_fence {
                block.push_str(line);
                block.push('\n');
            }
        }
        out
    }

    fn split_block(block: &str) -> Vec<Vec<String>> {
        let mut commands = Vec::new();
        let mut words: Vec<String> = Vec::new();
        let mut word = String::new();
        let (mut quote, mut has_word, mut comment) = (None::<char>, false, false);
        let flush_word = |words: &mut Vec<String>, word: &mut String, has: &mut bool| {
            if *has {
                words.push(std::mem::take(word));
                *has = false;
            }
        };
        for c in block.chars() {
            match (quote, c) {
                (Some(q), c) if c == q => quote = None,
                (Some(_), c) => word.push(c),
                (None, '\n') => {
                    flush_word(&mut words, &mut word, &mut has_word);
                    commands.push(std::mem::take(&mut words));
                    comment = false;
                }
                (None, _) if comment => {}
                (None, '#') if !has_word => comment = true,
                (None, '"' | '\'') => {
                    quote = Some(c);
                    has_word = true;
                }
                (None, c) if c.is_whitespace() => flush_word(&mut words, &mut word, &mut has_word),
                (None, c) => {
                    word.push(c);
                    has_word = true;
                }
            }
        }
        commands
            .into_iter()
            .map(|mut words| {
                if let Some(at) = words.iter().position(|w| w == ">") {
                    words.truncate(at);
                }
                words
            })
            .collect()
    }

    fn check_commands(name: &str, markdown: &str) {
        let mut seen = 0;
        for argv in commands(markdown) {
            if argv.first().map(String::as_str) != Some("deck") {
                continue;
            }
            seen += 1;
            if let Err(e) = Cli::try_parse_from(&argv) {
                panic!("{name}: `{}` does not parse: {e}", argv.join(" "));
            }
        }
        assert!(seen > 0, "{name} has no deck command");
    }

    #[test]
    fn every_deck_command_in_the_skill_parses() {
        check_commands("SKILL.md", SKILL);
    }

    #[test]
    fn every_deck_command_in_the_readme_parses() {
        check_commands("README.md", README);
    }
}
