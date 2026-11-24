//! `deck skills`: installs, removes and checks the agent skill deck ships (`skills/deck`,
//! built into the binary) in the skills directories of Claude Code, Codex and OpenCode.
//!
//! A skill directory is deck's when it holds only a `SKILL.md` whose front matter names the
//! skill, or when it is a symlink to such a directory (the old `ln -s` install) or to nothing.
//! Anything else with the skill's name is the user's and is never touched.

use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, bail};
use clap::{Subcommand, ValueEnum};
use deck_core::paths::EnvLookup;

use crate::guide::SKILL;

/// The skill's name: its directory and the `name` of its front matter.
const NAME: &str = "deck";
const FILE: &str = "SKILL.md";

#[derive(Debug, Subcommand)]
pub enum SkillsCommand {
    /// Writes deck's skill into the agents' skills directories (safe to repeat; refreshes an
    /// older copy).
    Install {
        /// The agents to install the skill for.
        #[arg(required = true, value_enum)]
        agents: Vec<SkillAgent>,
        /// Print what would be written and write nothing.
        #[arg(long)]
        print: bool,
    },
    /// Removes only deck's skill.
    Uninstall {
        /// The agents to remove the skill from.
        #[arg(required = true, value_enum)]
        agents: Vec<SkillAgent>,
    },
    /// Says for each agent whether deck's skill is installed and current.
    Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SkillAgent {
    Claude,
    Codex,
    Opencode,
}

impl SkillAgent {
    const ALL: [Self; 3] = [Self::Claude, Self::Codex, Self::Opencode];

    fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Opencode => "opencode",
        }
    }

    /// The directory the agent reads the user's skills from.
    fn skills_dir(self, env: EnvLookup<'_>) -> anyhow::Result<PathBuf> {
        let set = |name: &str| env(name).filter(|v| !v.is_empty()).map(PathBuf::from);
        let home = || set("HOME").context("cannot find the home directory: HOME is not set");
        Ok(match self {
            Self::Claude => set("CLAUDE_CONFIG_DIR")
                .map_or_else(|| home().map(|h| h.join(".claude")), Ok)?
                .join("skills"),
            Self::Codex => home()?.join(".agents").join("skills"),
            Self::Opencode => set("OPENCODE_CONFIG_DIR")
                .map_or_else(
                    || {
                        set("XDG_CONFIG_HOME")
                            .map_or_else(|| home().map(|h| h.join(".config")), Ok)
                            .map(|c| c.join("opencode"))
                    },
                    Ok,
                )?
                .join("skills"),
        })
    }
}

pub fn run(command: SkillsCommand) -> anyhow::Result<ExitCode> {
    let env = |name: &str| std::env::var(name).ok();
    match command {
        SkillsCommand::Install { agents, print } => install(&agents, print, &env),
        SkillsCommand::Uninstall { agents } => uninstall(&agents, &env),
        SkillsCommand::Status => status(&env),
    }
}

/// What is at a skill's path.
#[derive(Debug, PartialEq, Eq)]
enum Found {
    Missing,
    /// deck's copy, the same as the binary's.
    Current,
    /// deck's copy, from another version.
    Stale,
    /// A symlink to a deck skill directory, or to nothing.
    Link(PathBuf),
    /// Not deck's.
    Foreign,
}

/// The `name` of a `SKILL.md`'s front matter.
fn front_matter_name(text: &str) -> Option<&str> {
    let (matter, _) = text.strip_prefix("---\n")?.split_once("\n---")?;
    matter
        .lines()
        .find_map(|l| l.strip_prefix("name:"))
        .map(|n| n.trim().trim_matches(['"', '\'']))
}

/// The text of `dir/SKILL.md` when `dir` holds that file only and it names the skill.
fn deck_skill_text(dir: &Path) -> Option<String> {
    let mut entries = fs::read_dir(dir).ok()?;
    let only = entries.next()?.ok()?;
    if entries.next().is_some() || only.file_name() != FILE || !only.file_type().ok()?.is_file() {
        return None;
    }
    let text = fs::read_to_string(only.path()).ok()?;
    (front_matter_name(&text) == Some(NAME)).then_some(text)
}

fn find(dir: &Path) -> anyhow::Result<Found> {
    let meta = match fs::symlink_metadata(dir) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Found::Missing),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", dir.display())),
    };
    if meta.is_symlink() {
        let target =
            fs::read_link(dir).with_context(|| format!("cannot read {}", dir.display()))?;
        let dangling = !dir.exists();
        return Ok(if dangling || deck_skill_text(dir).is_some() {
            Found::Link(target)
        } else {
            Found::Foreign
        });
    }
    if !meta.is_dir() {
        return Ok(Found::Foreign);
    }
    Ok(match deck_skill_text(dir) {
        Some(text) if text == SKILL => Found::Current,
        Some(_) => Found::Stale,
        None => Found::Foreign,
    })
}

fn foreign(dir: &Path) -> anyhow::Error {
    anyhow::anyhow!(
        "{} exists and was not written by deck; left untouched",
        dir.display()
    )
}

/// Writes the skill into `dir` (a new or deck's own directory): a temp file renamed over
/// `SKILL.md`.
fn write_skill(dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let target = dir.join(FILE);
    let tmp = dir.join(format!(".{FILE}.deck-{}", std::process::id()));
    let result = (|| -> anyhow::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o644)
            .open(&tmp)?;
        file.write_all(SKILL.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, &target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.with_context(|| format!("cannot write {}", target.display()))
}

/// Removes deck's own directory: its `SKILL.md`, then the directory.
fn remove_skill(dir: &Path) -> anyhow::Result<()> {
    let file = dir.join(FILE);
    fs::remove_file(&file).with_context(|| format!("cannot remove {}", file.display()))?;
    fs::remove_dir(dir).with_context(|| format!("cannot remove {}", dir.display()))
}

fn remove_link(dir: &Path) -> anyhow::Result<()> {
    fs::remove_file(dir).with_context(|| format!("cannot remove the link {}", dir.display()))
}

fn dedup(agents: &[SkillAgent]) -> Vec<SkillAgent> {
    SkillAgent::ALL
        .into_iter()
        .filter(|a| agents.contains(a))
        .collect()
}

fn install(agents: &[SkillAgent], print: bool, env: EnvLookup<'_>) -> anyhow::Result<ExitCode> {
    // Every agent is checked before any is written, so a refusal leaves nothing half done.
    let mut plan = Vec::new();
    for agent in dedup(agents) {
        let dir = agent.skills_dir(env)?.join(NAME);
        let found = find(&dir)?;
        if found == Found::Foreign {
            return Err(foreign(&dir));
        }
        plan.push((agent, dir, found));
    }
    for (agent, dir, found) in plan {
        if print {
            println!("# {}", dir.join(FILE).display());
            print!("{SKILL}");
            continue;
        }
        let name = agent.name();
        match found {
            Found::Current => println!("{name}: already installed in {}", dir.display()),
            Found::Link(target) => {
                remove_link(&dir)?;
                if let Err(e) = write_skill(&dir) {
                    // Put the link back rather than leave neither it nor a copy.
                    let _ = fs::remove_dir_all(&dir);
                    let _ = std::os::unix::fs::symlink(&target, &dir);
                    return Err(e);
                }
                println!(
                    "{name}: installed in {} (replaced the link to {})",
                    dir.display(),
                    target.display()
                );
            }
            Found::Missing | Found::Stale => {
                write_skill(&dir)?;
                println!("{name}: installed in {}", dir.display());
            }
            Found::Foreign => unreachable!("refused above"),
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn uninstall(agents: &[SkillAgent], env: EnvLookup<'_>) -> anyhow::Result<ExitCode> {
    for agent in dedup(agents) {
        let dir = agent.skills_dir(env)?.join(NAME);
        let name = agent.name();
        match find(&dir)? {
            Found::Missing => println!("{name}: not installed"),
            Found::Current | Found::Stale => {
                remove_skill(&dir)?;
                println!("{name}: removed {}", dir.display());
            }
            Found::Link(_) => {
                remove_link(&dir)?;
                println!("{name}: removed the link {}", dir.display());
            }
            Found::Foreign => return Err(foreign(&dir)),
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// The status of one agent.
fn agent_status(agent: SkillAgent, env: EnvLookup<'_>) -> anyhow::Result<String> {
    let dir = agent.skills_dir(env)?.join(NAME);
    let name = agent.name();
    let line = match find(&dir)? {
        Found::Current => "installed".to_string(),
        Found::Stale => {
            format!("stale: from another version of deck; run \"deck skills install {name}\"")
        }
        Found::Link(target) => format!(
            "linked to {}; \"deck skills install {name}\" replaces it with a copy",
            target.display()
        ),
        Found::Foreign => format!("{} is not deck's skill", dir.display()),
        Found::Missing => match opencode_reads_another(agent, env) {
            Some(other) => format!("not installed (OpenCode reads the copy in {other})"),
            None => "not installed".to_string(),
        },
    };
    Ok(format!("{name}: {line}"))
}

/// OpenCode also reads the skills of `~/.claude/skills` and `~/.agents/skills`.
fn opencode_reads_another(agent: SkillAgent, env: EnvLookup<'_>) -> Option<&'static str> {
    if agent != SkillAgent::Opencode {
        return None;
    }
    let home = PathBuf::from(env("HOME").filter(|h| !h.is_empty())?);
    ["~/.claude/skills", "~/.agents/skills"]
        .into_iter()
        .find(|other| {
            let dir = home.join(&other[2..]).join(NAME);
            matches!(
                find(&dir),
                Ok(Found::Current | Found::Stale | Found::Link(_))
            ) && dir.join(FILE).is_file()
        })
}

fn status(env: EnvLookup<'_>) -> anyhow::Result<ExitCode> {
    let mut failed = false;
    for agent in SkillAgent::ALL {
        match agent_status(agent, env) {
            Ok(line) => println!("{line}"),
            Err(e) => {
                failed = true;
                eprintln!("deck: {}: {e:#}", agent.name());
            }
        }
    }
    if failed {
        bail!("could not check every agent");
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_front_matter_name_is_read() {
        assert_eq!(front_matter_name(SKILL), Some(NAME));
        assert_eq!(front_matter_name("---\nname: \"x\"\n---\nbody"), Some("x"));
        assert_eq!(front_matter_name("# no front matter\nname: deck\n"), None);
    }
}
