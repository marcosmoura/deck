//! `deck hooks`: installs, removes and checks the hooks that make Claude Code and Codex
//! report to `deck agent-event`, and the plugin file that does it for OpenCode.
//!
//! Only the user's own agent config files are touched, and only here. A file is parsed and
//! rewritten with its key order kept, through its symlink, after a backup, by renaming a
//! temp file over it.

use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use clap::{Subcommand, ValueEnum};
use deck_core::agents::{AgentKind, OPENCODE_EVENTS, hook_events};
use deck_core::paths::{self, EnvLookup};
use serde_json::{Map, Value, json};

/// The text between the `deck` binary and the agent's name in a hook command; it is what
/// tells deck's entries from the user's.
const MARKER: &str = " agent-event --agent ";

#[derive(Debug, Subcommand)]
pub enum HooksCommand {
    /// Adds deck's hooks to the agents' config files (keeps a backup; safe to repeat).
    Install {
        /// The agents to install hooks for.
        #[arg(required = true, value_enum)]
        agents: Vec<HookAgent>,
        /// Print what would be written and write nothing.
        #[arg(long)]
        print: bool,
    },
    /// Removes only deck's hooks.
    Uninstall {
        /// The agents to remove hooks from.
        #[arg(required = true, value_enum)]
        agents: Vec<HookAgent>,
    },
    /// Says for each agent whether deck's hooks are installed.
    Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum HookAgent {
    Claude,
    Codex,
    Opencode,
}

impl HookAgent {
    const ALL: [Self; 3] = [Self::Claude, Self::Codex, Self::Opencode];

    fn kind(self) -> AgentKind {
        match self {
            Self::Claude => AgentKind::Claude,
            Self::Codex => AgentKind::Codex,
            Self::Opencode => AgentKind::OpenCode,
        }
    }

    fn name(self) -> &'static str {
        self.kind().name()
    }

    /// The file that holds the agent's hooks.
    fn file(self, env: EnvLookup<'_>) -> anyhow::Result<PathBuf> {
        let set = |name: &str| env(name).filter(|v| !v.is_empty()).map(PathBuf::from);
        let home = || set("HOME").context("cannot find the home directory: HOME is not set");
        Ok(match self {
            Self::Claude => set("CLAUDE_CONFIG_DIR")
                .map_or_else(|| home().map(|h| h.join(".claude")), Ok)?
                .join("settings.json"),
            Self::Codex => set("CODEX_HOME")
                .map_or_else(|| home().map(|h| h.join(".codex")), Ok)?
                .join("hooks.json"),
            Self::Opencode => set("OPENCODE_CONFIG_DIR")
                .map_or_else(
                    || {
                        set("XDG_CONFIG_HOME")
                            .map_or_else(|| home().map(|h| h.join(".config")), Ok)
                            .map(|c| c.join("opencode"))
                    },
                    Ok,
                )?
                .join("plugins")
                .join(PLUGIN_DIR)
                .join(PLUGIN_FILE),
        })
    }

    /// Codex also loads hooks from its `config.toml`.
    fn config_toml(self, env: EnvLookup<'_>) -> Option<PathBuf> {
        let file = self.file(env).ok()?;
        (self == Self::Codex).then(|| file.with_file_name("config.toml"))
    }

    /// One entry of an event's list, running `command`.
    fn group(self, command: &str) -> Value {
        match self {
            Self::Claude => json!({
                "hooks": [{ "type": "command", "command": command, "async": true }]
            }),
            Self::Codex => json!({
                "matcher": "",
                "hooks": [{ "type": "command", "command": command, "timeout": 2 }]
            }),
            Self::Opencode => Value::Null,
        }
    }

    /// A new file: Codex wants a wrapper with a description; Claude Code a bare object.
    fn empty(self) -> Value {
        match self {
            Self::Claude => json!({}),
            Self::Codex => json!({ "description": "Reports what Codex does to deck" }),
            Self::Opencode => Value::Null,
        }
    }
}

pub fn run(command: HooksCommand) -> anyhow::Result<ExitCode> {
    let env = |name: &str| std::env::var(name).ok();
    match command {
        HooksCommand::Install { agents, print } => install(&agents, print, &env),
        HooksCommand::Uninstall { agents } => uninstall(&agents, &env),
        HooksCommand::Status => status(&env),
    }
}

// --- the hook commands ---------------------------------------------------------------------

/// Whether a hook command is one of deck's.
fn is_deck_command(command: &str) -> bool {
    command.split_once(MARKER).is_some_and(|(before, _)| {
        let before = before.trim_end_matches('\'');
        before == "deck" || before.ends_with("/deck")
    })
}

/// The path of the binary a hook command runs, when it is an absolute one.
fn command_binary(command: &str) -> Option<PathBuf> {
    let path = command_program(command)?;
    path.starts_with('/').then(|| PathBuf::from(path))
}

/// The program of a deck hook command, with its shell quoting undone.
fn command_program(command: &str) -> Option<String> {
    let (before, _) = command.split_once(MARKER)?;
    Some(
        match before.strip_prefix('\'').and_then(|b| b.strip_suffix('\'')) {
            Some(inner) => inner.replace("'\\''", "'"),
            None => before.to_string(),
        },
    )
}

/// Whether the binary a hook command runs is gone: its absolute path does not exist, or a
/// bare `deck` is not on `PATH`.
fn binary_missing(command: &str, env: EnvLookup<'_>) -> bool {
    if let Some(binary) = command_binary(command) {
        return !binary.exists();
    }
    command_program(command).is_some_and(|p| p == "deck")
        && env("PATH").and_then(|p| deck_on_path(&p)).is_none()
}

fn shell_quote(path: &str) -> String {
    let plain = path
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-+@:,".contains(c));
    if plain {
        path.to_string()
    } else {
        format!("'{}'", path.replace('\'', "'\\''"))
    }
}

/// The first `deck` on `PATH`.
fn deck_on_path(path: &str) -> Option<PathBuf> {
    std::env::split_paths(path)
        .map(|dir| dir.join("deck"))
        .find(|p| p.is_file())
}

/// The program a hook runs for the binary `exe` (a canonical path), and whether it is that
/// binary itself (then a rebuild or a move makes the hook stale). When the first `deck` on
/// `PATH` is this binary, the program is that entry as it is written, which an upgrade keeps
/// pointing at the new version (`/opt/homebrew/bin/deck`). Never the bare name, and never a
/// relative entry: an agent resolves a hook with its own `PATH` in the project's directory,
/// where `./node_modules/.bin` or `.` would run a `deck` that the repository supplies.
fn hook_program(exe: &Path, path: Option<&str>) -> (PathBuf, bool) {
    let entry = path
        .and_then(deck_on_path)
        .filter(|entry| entry.is_absolute() && entry.canonicalize().is_ok_and(|real| real == exe));
    match entry {
        Some(entry) => (entry, false),
        None => (exe.to_path_buf(), true),
    }
}

/// The command a hook of `agent` runs, and whether it names the binary itself (then a
/// rebuild or a move makes it stale).
fn hook_command(agent: HookAgent, env: EnvLookup<'_>) -> anyhow::Result<(String, bool)> {
    let exe = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .context("cannot find the deck executable")?;
    let (program, absolute) = hook_program(&exe, env("PATH").as_deref());
    let program = shell_quote(&program.to_string_lossy());
    Ok((format!("{program}{MARKER}{}", agent.name()), absolute))
}

// --- editing the JSON ----------------------------------------------------------------------

fn is_deck_handler(handler: &Value) -> bool {
    handler
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(is_deck_command)
}

fn handlers_mut(group: &mut Value) -> Option<&mut Vec<Value>> {
    group.get_mut("hooks").and_then(Value::as_array_mut)
}

/// Puts deck's hook into an event's list: in the place of the one already there, else last.
fn place(groups: &mut Vec<Value>, agent: HookAgent, command: &str) {
    let fresh = agent.group(command);
    let handler = fresh
        .get("hooks")
        .and_then(Value::as_array)
        .and_then(|h| h.first())
        .cloned()
        .unwrap_or(Value::Null);
    let mut placed = false;
    let mut emptied = Vec::with_capacity(groups.len());
    for group in groups.iter_mut() {
        let mut removed = false;
        if let Some(list) = handlers_mut(group) {
            list.retain_mut(|h| {
                if !is_deck_handler(h) {
                    return true;
                }
                if placed {
                    removed = true;
                    return false;
                }
                *h = handler.clone();
                placed = true;
                true
            });
            removed &= list.is_empty();
        }
        emptied.push(removed);
    }
    let mut emptied = emptied.into_iter();
    groups.retain(|_| !emptied.next().unwrap_or(false));
    if !placed {
        groups.push(fresh);
    }
}

fn install_into(root: &mut Value, agent: HookAgent, command: &str) -> anyhow::Result<()> {
    let root = root
        .as_object_mut()
        .context("the file is not a JSON object")?;
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .context("\"hooks\" is not an object")?;
    for event in hook_events(agent.kind()) {
        let groups = hooks
            .entry(*event)
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .with_context(|| format!("hooks.{event} is not an array"))?;
        place(groups, agent, command);
    }
    Ok(())
}

/// Removes every deck hook; returns how many.
fn remove_from(root: &mut Value, agent: HookAgent) -> usize {
    let mut removed = 0;
    let Some(root) = root.as_object_mut() else {
        return 0;
    };
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        let mut pruned = Vec::new();
        for (event, groups) in hooks.iter_mut() {
            let Some(groups) = groups.as_array_mut() else {
                continue;
            };
            let mut touched = false;
            groups.retain_mut(|group| {
                let Some(list) = handlers_mut(group) else {
                    return true;
                };
                let before = list.len();
                list.retain(|h| !is_deck_handler(h));
                removed += before - list.len();
                touched |= list.len() != before;
                // Only a group deck emptied goes; the user's own empty ones stay.
                list.len() == before || !list.is_empty()
            });
            if touched && groups.is_empty() {
                pruned.push(event.clone());
            }
        }
        hooks.retain(|event, _| !pruned.contains(event));
    }
    let empty = root
        .get("hooks")
        .and_then(Value::as_object)
        .is_some_and(Map::is_empty);
    if empty && agent == HookAgent::Claude {
        root.remove("hooks");
    }
    removed
}

/// Every deck hook command of a file: its event and command.
fn deck_hooks(root: &Value) -> Vec<(String, String)> {
    let Some(hooks) = root.get("hooks").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for (event, groups) in hooks {
        for group in groups.as_array().into_iter().flatten() {
            for handler in group
                .get("hooks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if is_deck_handler(handler)
                    && let Some(command) = handler.get("command").and_then(Value::as_str)
                {
                    found.push((event.clone(), command.to_string()));
                }
            }
        }
    }
    found
}

// --- the files -----------------------------------------------------------------------------

/// The real file behind `path`, following symlinks; `path` itself when it does not exist.
/// A symlink to a missing file is refused: writing would replace the link.
fn real_path(path: &Path) -> anyhow::Result<PathBuf> {
    match path.canonicalize() {
        Ok(real) => Ok(real),
        Err(_) if fs::symlink_metadata(path).is_ok_and(|m| m.is_symlink()) => bail!(
            "{} is a symlink to a file that does not exist; fix or remove the link first",
            path.display()
        ),
        Err(_) => Ok(path.to_path_buf()),
    }
}

fn read_json(path: &Path, agent: HookAgent) -> anyhow::Result<(Value, Option<String>)> {
    match fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok((agent.empty(), Some(text))),
        Ok(text) => {
            let value = serde_json::from_str(&text)
                .with_context(|| format!("{} is not valid JSON; left untouched", path.display()))?;
            Ok((value, Some(text)))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((agent.empty(), None)),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
    }
}

fn render(value: &Value) -> anyhow::Result<String> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    Ok(text)
}

fn backup_dir(env: EnvLookup<'_>) -> anyhow::Result<PathBuf> {
    Ok(paths::state_dir(env)?.join("backups"))
}

/// The backup directory, created with the state directory above it when missing, both for
/// the user only: a backup is a copy of an agent's settings.
fn private_backup_dir(env: EnvLookup<'_>) -> anyhow::Result<PathBuf> {
    let dir = backup_dir(env)?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .with_context(|| format!("cannot create {}", dir.display()))?;
    Ok(dir)
}

/// How many backups of one file are kept: a backup is a whole copy of an agent's settings,
/// which can hold keys, so old ones do not stay forever.
const BACKUPS_KEPT: usize = 3;

/// Deletes the backups of `stem` (`<agent>-<file>`) but `written`, the one just made, and
/// the newest of the others, [`BACKUPS_KEPT`] in all. A backup is `<stem>-<unix seconds>`
/// or `<stem>-<unix seconds>-<n>`.
fn prune_backups(dir: &Path, stem: &str, written: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut found: Vec<((u64, u32), PathBuf)> = entries
        .flatten()
        .filter(|e| e.path() != written)
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let stamp = name.strip_prefix(stem)?.strip_prefix('-')?;
            let (secs, n) = stamp.split_once('-').unwrap_or((stamp, "0"));
            Some(((secs.parse().ok()?, n.parse().ok()?), e.path()))
        })
        .collect();
    found.sort_by_key(|(stamp, _)| std::cmp::Reverse(*stamp));
    for (_, old) in found.into_iter().skip(BACKUPS_KEPT - 1) {
        if let Err(e) = fs::remove_file(&old) {
            eprintln!("deck: cannot remove the old backup {}: {e}", old.display());
        }
    }
}

/// Writes a backup of `stem` (`<agent>-<file>`) that does not exist yet, `<stem>-<secs>`,
/// else `<stem>-<secs>-1`, `-2`..., then deletes the oldest ones of that file.
fn write_backup(dir: &Path, stem: &str, secs: u64, contents: &[u8]) -> anyhow::Result<()> {
    let name = format!("{stem}-{secs}");
    for n in 0u32.. {
        let file = if n == 0 {
            dir.join(&name)
        } else {
            dir.join(format!("{name}-{n}"))
        };
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&file)
        {
            Ok(mut f) => {
                f.write_all(contents)
                    .with_context(|| format!("cannot write the backup {}", file.display()))?;
                prune_backups(dir, stem, &file);
                return Ok(());
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => {
                return Err(e)
                    .with_context(|| format!("cannot write the backup {}", file.display()));
            }
        }
    }
    bail!("too many backups")
}

/// Writes `text` over `target` (a real path): backup first, then a temp file in the same
/// directory renamed into place. Returns whether anything changed.
fn write_safely(
    agent: HookAgent,
    target: &Path,
    old: Option<&str>,
    text: &str,
    env: EnvLookup<'_>,
) -> anyhow::Result<bool> {
    if old == Some(text) {
        return Ok(false);
    }
    let dir = target.parent().context("the file has no directory")?;
    fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .context("the file has no name")?;
    if old.is_some() {
        let backups = private_backup_dir(env)?;
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let contents =
            fs::read(target).with_context(|| format!("cannot read {}", target.display()))?;
        write_backup(
            &backups,
            &format!("{}-{name}", agent.name()),
            secs,
            &contents,
        )?;
    }
    let tmp = dir.join(format!(".{name}.deck-{}", std::process::id()));
    let mode = fs::metadata(target).map_or(0o644, |m| m.permissions().mode() & 0o7777);
    let result = (|| -> anyhow::Result<()> {
        // Created with the target's mode, so a private file is never readable by others.
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(mode)
            .open(&tmp)?;
        file.set_permissions(fs::Permissions::from_mode(mode))?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.with_context(|| format!("cannot write {}", target.display()))?;
    Ok(true)
}

// --- the OpenCode plugin ---------------------------------------------------------------------

/// OpenCode v2 loads a TUI plugin from `plugins/<dir>/tui.js`; the TUI runs once per terminal,
/// where `DECK_TERMINAL_ID` is the terminal's own (a server plugin would run in the one
/// shared background server).
const PLUGIN_DIR: &str = "deck";
const PLUGIN_FILE: &str = "tui.js";
const PLUGIN_TEMPLATE: &str = include_str!("opencode_plugin.js.tmpl");

/// The `plugins/deck.js` that earlier versions of deck wrote (an OpenCode v1 plugin).
fn legacy_plugin(path: &Path) -> Option<PathBuf> {
    Some(path.parent()?.parent()?.join("deck.js"))
}

/// First line of the plugin; what tells deck's file from one of the user's.
const PLUGIN_MARKER: &str = "// Written by \"deck hooks install opencode\"";

/// The plugin file: it forwards the events deck subscribes to, as they are, and nothing
/// else; deck's daemon does the interpreting. `program` is the deck binary.
fn plugin_source(program: &str) -> anyhow::Result<String> {
    let events = OPENCODE_EVENTS
        .iter()
        .map(|e| format!("  \"{e}\","))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(PLUGIN_TEMPLATE
        .replace("@MARKER@", PLUGIN_MARKER)
        .replace("@PROGRAM@", &serde_json::to_string(program)?)
        .replace("@EVENTS@", &events))
}

/// Whether a plugin file's text is deck's.
fn is_deck_plugin(text: &str) -> bool {
    text.starts_with(PLUGIN_MARKER)
}

/// The deck binary a plugin file runs.
fn plugin_program(text: &str) -> Option<String> {
    let line = text.lines().find_map(|l| l.strip_prefix("const DECK = "))?;
    serde_json::from_str(line.trim_end_matches(';')).ok()
}

fn read_plugin(path: &Path) -> anyhow::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
    }
}

fn install_plugin(
    path: &Path,
    command: &str,
    absolute: bool,
    print: bool,
    env: EnvLookup<'_>,
) -> anyhow::Result<()> {
    let real = real_path(path)?;
    let program = command_program(command).context("cannot find the deck executable")?;
    let text = plugin_source(&program)?;
    if print {
        println!("# {}", real.display());
        print!("{text}");
        return Ok(());
    }
    let old = read_plugin(&real)?;
    if old.as_deref().is_some_and(|o| !is_deck_plugin(o)) {
        bail!(
            "{} exists and was not written by deck; left untouched",
            real.display()
        );
    }
    let changed = write_safely(HookAgent::Opencode, &real, old.as_deref(), &text, env)?;
    let what = if changed {
        "installed in"
    } else {
        "already installed in"
    };
    println!("opencode: {what} {}", real.display());
    // The v1 file would make OpenCode v2 report a plugin error on every start.
    if let Some(legacy) = legacy_plugin(path)
        && let Some(old) = read_plugin(&legacy)?.filter(|t| is_deck_plugin(t))
    {
        write_safely_removing(&legacy, &old, env)?;
        println!("  removed the old plugin {}", legacy.display());
    }
    if changed && old.is_some() {
        println!(
            "  backup of the previous file in {}",
            backup_dir(env)?.display()
        );
    }
    if absolute {
        println!(
            "  the plugin runs {program}: moving or rebuilding deck makes it stale; \
             run \"deck hooks install opencode\" again then"
        );
    }
    Ok(())
}

/// Removes the plugin at `path` and the old `deck.js` when deck wrote it. When one is a
/// symlink, the link goes and its target stays.
fn uninstall_plugin(path: &Path, env: EnvLookup<'_>) -> anyhow::Result<()> {
    let mut removed = false;
    match read_plugin(path)? {
        Some(text) if is_deck_plugin(&text) => {
            write_safely_removing(path, &text, env)?;
            println!("opencode: removed {}", path.display());
            removed = true;
            if let Some(dir) = path.parent() {
                let _ = fs::remove_dir(dir);
            }
        }
        Some(_) => bail!("{} was not written by deck; left untouched", path.display()),
        None => {}
    }
    if let Some(legacy) = legacy_plugin(path)
        && let Some(text) = read_plugin(&legacy)?.filter(|t| is_deck_plugin(t))
    {
        write_safely_removing(&legacy, &text, env)?;
        println!("opencode: removed {}", legacy.display());
        removed = true;
    }
    if !removed {
        println!("opencode: not installed");
    }
    Ok(())
}

/// Removes a plugin file (or the symlink to it) after a backup of its contents.
fn write_safely_removing(path: &Path, text: &str, env: EnvLookup<'_>) -> anyhow::Result<()> {
    let backups = private_backup_dir(env)?;
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    write_backup(
        &backups,
        &format!(
            "opencode-{}",
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(PLUGIN_FILE)
        ),
        secs,
        text.as_bytes(),
    )?;
    fs::remove_file(path).with_context(|| format!("cannot remove {}", path.display()))
}

/// A dangling symlink reads as no file: `not installed`.
fn plugin_status(path: &Path, env: EnvLookup<'_>) -> anyhow::Result<String> {
    let old = legacy_plugin(path)
        .map(|legacy| read_plugin(&legacy))
        .transpose()?
        .flatten()
        .is_some_and(|t| is_deck_plugin(&t));
    let text = read_plugin(path)?.filter(|t| is_deck_plugin(t));
    if old {
        return Ok(
            "stale: an old plugin (OpenCode v1) is installed; run \"deck hooks install opencode\""
                .to_string(),
        );
    }
    let Some(text) = text else {
        return Ok("not installed".to_string());
    };
    let missing = plugin_program(&text).is_none_or(|program| {
        binary_missing(&format!("{}{MARKER}opencode", shell_quote(&program)), env)
    });
    if missing {
        return Ok(
            "stale: the deck binary of the plugin is gone; run \"deck hooks install opencode\" again"
                .to_string(),
        );
    }
    if !plugin_program(&text).is_some_and(|program| program.starts_with('/')) {
        return Ok(
            "stale: the plugin does not run deck by an absolute path; run \"deck hooks install opencode\" again"
                .to_string(),
        );
    }
    if !OPENCODE_EVENTS
        .iter()
        .all(|e| text.contains(&format!("\"{e}\"")))
    {
        return Ok(
            "stale: the plugin lacks some events; run \"deck hooks install opencode\" again"
                .to_string(),
        );
    }
    Ok("installed".to_string())
}

// --- the commands --------------------------------------------------------------------------

fn install(agents: &[HookAgent], print: bool, env: EnvLookup<'_>) -> anyhow::Result<ExitCode> {
    for agent in dedup(agents) {
        let (command, absolute) = hook_command(agent, env)?;
        let path = agent.file(env)?;
        if agent == HookAgent::Opencode {
            install_plugin(&path, &command, absolute, print, env)?;
            continue;
        }
        let real = real_path(&path)?;
        let (mut root, old) = read_json(&real, agent)?;
        install_into(&mut root, agent, &command)?;
        let text = render(&root)?;
        if print {
            println!("# {}", real.display());
            print!("{text}");
            continue;
        }
        let changed = write_safely(agent, &real, old.as_deref(), &text, env)?;
        let what = if changed {
            "installed in"
        } else {
            "already installed in"
        };
        println!("{}: {what} {}", agent.name(), real.display());
        if changed && old.is_some() {
            println!(
                "  backup of the previous file in {}; the file may have been reformatted",
                backup_dir(env)?.display()
            );
        }
        if absolute {
            println!(
                "  the hooks run {command}: moving or rebuilding deck makes them stale; \
                 run \"deck hooks install {}\" again then",
                agent.name()
            );
        }
        if agent == HookAgent::Codex {
            println!("  Codex asks you to review new hooks: open /hooks in Codex and trust them");
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn uninstall(agents: &[HookAgent], env: EnvLookup<'_>) -> anyhow::Result<ExitCode> {
    for agent in dedup(agents) {
        if agent == HookAgent::Opencode {
            uninstall_plugin(&agent.file(env)?, env)?;
            continue;
        }
        let real = real_path(&agent.file(env)?)?;
        let (mut root, old) = read_json(&real, agent)?;
        let Some(old) = old else {
            println!("{}: not installed", agent.name());
            continue;
        };
        if remove_from(&mut root, agent) == 0 {
            println!("{}: not installed", agent.name());
            continue;
        }
        let text = render(&root)?;
        write_safely(agent, &real, Some(&old), &text, env)?;
        println!("{}: removed from {}", agent.name(), real.display());
    }
    Ok(ExitCode::SUCCESS)
}

fn dedup(agents: &[HookAgent]) -> Vec<HookAgent> {
    HookAgent::ALL
        .into_iter()
        .filter(|a| agents.contains(a))
        .collect()
}

/// The status line of one agent.
fn agent_status(agent: HookAgent, env: EnvLookup<'_>) -> anyhow::Result<String> {
    if agent == HookAgent::Opencode {
        return Ok(format!(
            "opencode: {}",
            plugin_status(&agent.file(env)?, env)?
        ));
    }
    let real = real_path(&agent.file(env)?)?;
    let (root, old) = read_json(&real, agent)?;
    let found = if old.is_some() {
        deck_hooks(&root)
    } else {
        Vec::new()
    };
    let toml = agent
        .config_toml(env)
        .and_then(|p| fs::read_to_string(p).ok())
        .unwrap_or_default();
    let in_toml = toml
        .lines()
        .any(|l| l.contains("deck") && l.contains(MARKER.trim()));
    let wanted = hook_events(agent.kind());
    let events: Vec<&str> = found.iter().map(|(e, _)| e.as_str()).collect();
    let stale = found.iter().any(|(_, c)| binary_missing(c, env));
    let bare = found.iter().any(|(_, c)| command_binary(c).is_none());
    let complete = wanted.iter().all(|e| events.contains(e));
    let mut line = if found.is_empty() {
        "not installed".to_string()
    } else if stale {
        "stale: the deck binary of a hook is gone; run \"deck hooks install\" again".to_string()
    } else if bare {
        // What deck 0.1 and 0.2 wrote: the agent's `PATH` chooses the `deck` that runs.
        "stale: a hook does not run deck by an absolute path; run \"deck hooks install\" again"
            .to_string()
    } else if !complete {
        "stale: some events have no hook; run \"deck hooks install\" again".to_string()
    } else if agent == HookAgent::Codex {
        // Codex keys a trust entry by a hash deck cannot compute, so it cannot tell.
        "installed (trust them in /hooks if Codex asks)".to_string()
    } else {
        "installed".to_string()
    };
    if in_toml {
        line.push_str(" (deck entries also found in config.toml)");
    }
    Ok(format!("{}: {line}", agent.name()))
}

fn status(env: EnvLookup<'_>) -> anyhow::Result<ExitCode> {
    let mut failed = false;
    for agent in HookAgent::ALL {
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

    const CMD: &str = "deck agent-event --agent claude";

    fn installed(text: &str) -> Value {
        let mut root: Value = serde_json::from_str(text).unwrap();
        install_into(&mut root, HookAgent::Claude, CMD).unwrap();
        root
    }

    #[test]
    fn deck_commands_are_recognised() {
        assert!(is_deck_command("deck agent-event --agent claude"));
        assert!(is_deck_command("/opt/x/deck agent-event --agent codex"));
        assert!(is_deck_command(
            "'/opt/my dir/deck' agent-event --agent codex"
        ));
        assert!(!is_deck_command("echo agent-event --agent claude"));
        assert!(!is_deck_command("deckhand agent-event --agent claude"));
        assert_eq!(
            command_binary("'/opt/my dir/deck' agent-event --agent codex"),
            Some(PathBuf::from("/opt/my dir/deck"))
        );
        assert_eq!(command_binary(CMD), None);
    }

    #[test]
    fn install_keeps_existing_hooks_and_is_idempotent() {
        let before = r#"{"model":"x","hooks":{"Stop":[{"hooks":[{"type":"command","command":"mine"}]}],"Custom":[]},"zeta":1}"#;
        let once = installed(before);
        let twice = {
            let mut again = once.clone();
            install_into(&mut again, HookAgent::Claude, CMD).unwrap();
            again
        };
        assert_eq!(render(&once).unwrap(), render(&twice).unwrap());
        let keys: Vec<&String> = once.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["model", "hooks", "zeta"]);
        let stop = once["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop[0]["hooks"][0]["command"], "mine");
        assert_eq!(stop[1]["hooks"][0]["command"], CMD);
        assert_eq!(stop[1]["hooks"][0]["async"], true);
        assert_eq!(
            deck_hooks(&once).len(),
            hook_events(AgentKind::Claude).len()
        );
    }

    #[test]
    fn a_changed_command_replaces_the_old_one_in_place() {
        let mut root = installed("{}");
        install_into(
            &mut root,
            HookAgent::Claude,
            "/new/deck agent-event --agent claude",
        )
        .unwrap();
        let found = deck_hooks(&root);
        assert_eq!(found.len(), hook_events(AgentKind::Claude).len());
        assert!(found.iter().all(|(_, c)| c.starts_with("/new/deck")));
    }

    #[test]
    fn uninstall_restores_the_hooks() {
        let before =
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"mine"}]}]},"a":1}"#;
        let mut root = installed(before);
        assert_eq!(
            remove_from(&mut root, HookAgent::Claude),
            hook_events(AgentKind::Claude).len()
        );
        assert_eq!(root, serde_json::from_str::<Value>(before).unwrap());
        let mut bare = installed("{}");
        remove_from(&mut bare, HookAgent::Claude);
        assert_eq!(bare, json!({}));
    }

    #[test]
    fn the_codex_group_has_a_matcher_and_a_timeout() {
        let mut root = HookAgent::Codex.empty();
        install_into(
            &mut root,
            HookAgent::Codex,
            "deck agent-event --agent codex",
        )
        .unwrap();
        let group = &root["hooks"]["Interrupt"][0];
        assert_eq!(group["matcher"], "");
        assert_eq!(group["hooks"][0]["timeout"], 2);
        assert!(group["hooks"][0].get("async").is_none());
        assert!(root["hooks"].get("SessionEnd").is_some());
    }

    #[test]
    fn uninstall_leaves_the_users_own_empty_lists_and_groups() {
        let before = r#"{"hooks":{"Custom":[],"Stop":[{"hooks":[]}]}}"#;
        let mut root = installed(before);
        assert_eq!(
            remove_from(&mut root, HookAgent::Claude),
            hook_events(AgentKind::Claude).len()
        );
        assert_eq!(root, serde_json::from_str::<Value>(before).unwrap());
    }

    #[test]
    fn the_binary_of_a_quoted_command_is_unquoted() {
        let path = "/opt/it's here/deck";
        let command = format!("{} agent-event --agent codex", shell_quote(path));
        assert!(is_deck_command(&command));
        assert_eq!(command_binary(&command), Some(PathBuf::from(path)));
    }

    #[test]
    fn the_plugin_forwards_every_subscribed_event() {
        let text = plugin_source("/opt/it's/deck").unwrap();
        assert!(is_deck_plugin(&text));
        for event in OPENCODE_EVENTS {
            assert!(text.contains(&format!("\"{event}\"")), "{event}");
        }
        assert!(!text.contains('@'), "{text}");
        assert_eq!(plugin_program(&text).as_deref(), Some("/opt/it's/deck"));
    }

    #[test]
    fn the_plugin_is_a_v2_tui_plugin_that_chains_its_calls_without_awaiting_them() {
        let text = plugin_source("deck").unwrap();
        assert!(text.contains("export default {\n  id: \"deck\",\n  setup(ctx) {"));
        assert!(text.contains("ctx.data.listen(({ details: event }) => {"));
        assert!(text.contains("ctx.ui.router.current()"));
        assert!(text.contains(
            "spawn(DECK, [\"agent-event\", \"--agent\", \"opencode\", \"--event\", event.type]"
        ));
        assert!(text.contains("child.stdin.end(JSON.stringify(event));"));
        assert!(text.contains("let queue = Promise.resolve();"));
        assert!(text.contains("queue = queue.then(() => send(event)).then(() => {}, () => {});"));
        // Only the events of this TUI's own sessions go on.
        assert!(text.contains("if (!own.has(id)) return;"));
        // Not the tab list, which OpenCode shares between the TUIs of a directory.
        assert!(!text.contains("ui.tabs"));
        // v1 forms are gone, and the handler never awaits a call.
        assert!(!text.contains("export const"));
        assert!(!text.contains("$`"));
        assert!(!text.contains("await "));
    }

    #[test]
    fn the_plugin_sends_a_tool_once_and_never_a_text_chunk() {
        let text = plugin_source("deck").unwrap();
        for part in [
            "\"session.tool.called\"",
            "sent.has(call)",
            "const MAX = 256;",
            "\"deck.text\"",
            "\"session.text.ended\"",
        ] {
            assert!(text.contains(part), "{part}");
        }
        // Streamed chunks are not an event the plugin subscribes to.
        assert!(!text.contains("text.delta") && !text.contains("tool.input.delta"));
    }

    /// Runs the plugin under node (skipped when there is none) with a fake `deck` that logs
    /// its calls: a streamed answer of 500 chunks and 3 tool calls (one repeated) starts
    /// 3 + 1 processes beyond the status events, and only for the plugin's own session.
    #[test]
    fn a_streamed_answer_starts_few_processes() {
        use std::process::Command;
        if Command::new("node").arg("--version").output().is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("log");
        let fake = dir.path().join("deck");
        fs::write(
            &fake,
            format!(
                "#!/bin/sh\necho \"$@\" >> {0:?}\ncat >> {0:?}\necho >> {0:?}\n",
                log
            ),
        )
        .unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let plugin = dir.path().join("plugin.mjs");
        fs::write(&plugin, plugin_source(fake.to_str().unwrap()).unwrap()).unwrap();
        let driver = dir.path().join("driver.mjs");
        let import = format!("import plugin from {:?};", plugin.display().to_string());
        let body = r#"
let handler;
const ctx = {
  ui: { router: { current: () => ({ type: "session", sessionID: "s1" }) } },
  data: { session: { root: (id) => (id === "sub" ? "s1" : id) }, listen: (h) => { handler = h; return () => {}; } },
};
plugin.setup(ctx);
const ev = (type, data) => handler({ details: { type, data } });
ev("session.status", { sessionID: "s1", status: { type: "busy" } });
for (let n = 0; n < 500; n++) {
  ev("session.text.delta", { sessionID: "s1", delta: "x" });
  ev("session.tool.input.delta", { sessionID: "s1", id: "c0", delta: "x" });
}
for (const id of ["c1", "c2", "c3"]) {
  ev("session.tool.input.started", { sessionID: "s1", id, name: "bash" });
  ev("session.tool.called", { sessionID: "s1", id, input: { command: "ls", content: "SECRET" } });
  ev("session.tool.called", { sessionID: "s1", id, input: { command: "ls" } });
}
ev("session.tool.called", { sessionID: "other", id: "c9", input: {} });
ev("session.created", { sessionID: "sub", parentID: "s1" });
ev("session.tool.called", { sessionID: "sub", id: "c8", input: { command: "SUBSECRET" } });
ev("session.text.ended", { sessionID: "s1", text: "All done\nSECOND" });
ev("session.status", { sessionID: "s1", status: { type: "idle" } });
await new Promise((r) => setTimeout(r, 2500));
"#;
        fs::write(&driver, format!("{import}{body}")).unwrap();
        let out = Command::new("node").arg(&driver).output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let log = fs::read_to_string(&log).unwrap();
        assert_eq!(log.matches("agent-event").count(), 2 + 1 + 3 + 1, "{log}");
        assert_eq!(log.matches("session.tool.called").count() / 2, 3, "{log}");
        assert!(
            log.contains(r#""name":"bash""#) && log.contains("deck.text"),
            "{log}"
        );
        assert!(!log.contains("SECRET") && !log.contains("SECOND"), "{log}");
    }

    #[test]
    fn an_old_v1_plugin_is_stale_and_goes_with_an_install() {
        let dir = std::env::temp_dir().join(format!("deck-hooks-v1-{}", std::process::id()));
        let file = dir.join("plugins/deck/tui.js");
        let old = dir.join("plugins/deck.js");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(
            &old,
            format!("{PLUGIN_MARKER}; do not edit.\nconst DECK = \"/bin/sh\";\n"),
        )
        .unwrap();
        let env = |_: &str| None;
        assert_eq!(legacy_plugin(&file).as_deref(), Some(old.as_path()));
        assert!(plugin_status(&file, &env).unwrap().contains("OpenCode v1"));
        fs::remove_file(&old).unwrap();
        assert_eq!(plugin_status(&file, &env).unwrap(), "not installed");
        fs::write(&file, plugin_source("/bin/sh").unwrap()).unwrap();
        assert_eq!(plugin_status(&file, &env).unwrap(), "installed");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_plugin_without_the_activity_events_is_stale() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("plugins/deck/tui.js");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let env = |_: &str| None;
        let current = plugin_source("/bin/sh").unwrap();
        for event in [
            "session.tool.input.started",
            "session.tool.called",
            "session.text.ended",
        ] {
            // The plugin of an earlier deck: the same file without this event.
            fs::write(&file, current.replace(&format!("\"{event}\""), "\"x\"")).unwrap();
            let status = plugin_status(&file, &env).unwrap();
            assert!(
                status.starts_with("stale") && status.contains("lacks some events"),
                "{status}"
            );
        }
        fs::write(&file, &current).unwrap();
        assert_eq!(plugin_status(&file, &env).unwrap(), "installed");
    }

    #[test]
    fn the_hook_program_is_an_absolute_path_never_the_bare_name() {
        let tmp = tempfile::tempdir().unwrap();
        let file = |rest: &str| {
            let path = tmp.path().join(rest);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "").unwrap();
            path
        };
        let exe = file("cellar/1.0/deck").canonicalize().unwrap();
        // What a package manager puts on `PATH`: a link to the current version.
        let bin = tmp.path().join("bin");
        fs::create_dir(&bin).unwrap();
        std::os::unix::fs::symlink(&exe, bin.join("deck")).unwrap();
        let on_path = |dirs: &[&Path]| {
            let path = std::env::join_paths(dirs).unwrap();
            hook_program(&exe, path.to_str())
        };

        // The entry as written, not what it links to: it survives an upgrade.
        assert_eq!(on_path(&[&bin]), (bin.join("deck"), false));
        assert_ne!(bin.join("deck"), exe);
        // A relative entry that runs the same binary is never written.
        let cwd = std::env::current_dir().unwrap();
        let up: PathBuf = cwd.components().skip(1).map(|_| "..").collect();
        let relative = up.join(bin.strip_prefix("/").unwrap());
        assert_eq!(relative.join("deck").canonicalize().unwrap(), exe);
        assert_eq!(on_path(&[&relative, &bin]), (exe.clone(), true));
        // Another deck comes first, or none is there: this binary, by its own path.
        let other = file("other/deck");
        assert_eq!(
            on_path(&[other.parent().unwrap(), &bin]),
            (exe.clone(), true)
        );
        assert_eq!(
            on_path(&[Path::new("/nonexistent-dir")]),
            (exe.clone(), true)
        );
        assert_eq!(hook_program(&exe, None), (exe.clone(), true));
    }

    /// Privacy review, P-5: a backup is a copy of an agent's settings, keys included.
    #[test]
    fn only_the_newest_three_backups_of_a_file_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let names = || {
            let mut names: Vec<String> = fs::read_dir(dir.path())
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            names.sort();
            names
        };
        // Another file's backups, and a file that is no backup, are left alone.
        for other in [
            "claude-settings.local.json-100",
            "codex-hooks.json-100",
            "claude-settings.json-notes",
        ] {
            fs::write(dir.path().join(other), b"x").unwrap();
        }
        for secs in [100, 100, 100, 200, 90] {
            write_backup(dir.path(), "claude-settings.json", secs, b"{}").unwrap();
        }
        // A clock that went back: the backup just made is never the one deleted.
        assert!(dir.path().join("claude-settings.json-90").exists());
        write_backup(dir.path(), "claude-settings.json", 1_000, b"{}").unwrap();
        assert_eq!(
            names(),
            [
                "claude-settings.json-100-2",
                "claude-settings.json-1000",
                "claude-settings.json-200",
                "claude-settings.json-notes",
                "claude-settings.local.json-100",
                "codex-hooks.json-100",
            ],
            "the newest three, by time and then by number"
        );
    }

    #[test]
    fn a_bare_deck_is_missing_when_it_is_not_on_path() {
        let none = |_: &str| Some("/nonexistent-dir".to_string());
        assert!(binary_missing(CMD, &none));
        let here = |_: &str| None;
        assert!(binary_missing(CMD, &here));
    }
}
