//! `deck skills`, end to end. Every test points `HOME` (and the agents' config variables)
//! at a temp directory: the real skills directories are never read or written.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "integration-test helpers fail the test on error"
)]

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const SKILL: &str = include_str!("../../../skills/deck/SKILL.md");

struct Sandbox {
    dir: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self, rest: &str) -> PathBuf {
        self.dir.path().join(rest)
    }

    fn run_with(&self, args: &[&str], vars: &[(&str, PathBuf)]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_deck"));
        cmd.args(args)
            .env_clear()
            .env("HOME", self.path("home"))
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::null());
        for (name, value) in vars {
            cmd.env(name, value);
        }
        cmd.output().unwrap()
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_with(args, &[])
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn claude(&self) -> PathBuf {
        self.path("home/.claude/skills/deck")
    }

    fn codex(&self) -> PathBuf {
        self.path("home/.agents/skills/deck")
    }

    fn opencode(&self) -> PathBuf {
        self.path("home/.config/opencode/skills/deck")
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn install_writes_the_skill_for_each_agent_and_is_idempotent() {
    let sb = Sandbox::new();
    assert_eq!(
        sb.ok(&["skills", "status"]),
        "claude: not installed\ncodex: not installed\nopencode: not installed\n"
    );
    let out = sb.ok(&["skills", "install", "claude", "codex", "opencode", "claude"]);
    assert_eq!(out.lines().count(), 3, "{out}");
    for dir in [sb.claude(), sb.codex(), sb.opencode()] {
        assert_eq!(listing(&dir), ["SKILL.md"]);
        assert_eq!(fs::read_to_string(dir.join("SKILL.md")).unwrap(), SKILL);
    }
    assert_eq!(
        sb.ok(&["skills", "status"]),
        "claude: installed\ncodex: installed\nopencode: installed\n"
    );
    let again = sb.ok(&["skills", "install", "claude"]);
    assert!(again.starts_with("claude: already installed in"), "{again}");
}

#[test]
fn print_writes_nothing() {
    let sb = Sandbox::new();
    let out = sb.ok(&["skills", "install", "codex", "--print"]);
    assert!(out.starts_with(&format!("# {}\n", sb.codex().join("SKILL.md").display())));
    assert!(out.ends_with(SKILL));
    assert!(!sb.path("home/.agents").exists());
}

#[test]
fn an_older_copy_is_stale_and_install_refreshes_it() {
    let sb = Sandbox::new();
    write(
        &sb.claude().join("SKILL.md"),
        "---\nname: deck\ndescription: old\n---\n# old\n",
    );
    let status = sb.ok(&["skills", "status"]);
    assert!(status.starts_with("claude: stale"), "{status}");
    sb.ok(&["skills", "install", "claude"]);
    assert_eq!(
        fs::read_to_string(sb.claude().join("SKILL.md")).unwrap(),
        SKILL
    );
}

#[test]
fn uninstall_removes_only_deck_s_skill() {
    let sb = Sandbox::new();
    sb.ok(&["skills", "install", "claude", "codex"]);
    write(&sb.path("home/.claude/skills/mine/SKILL.md"), "mine");
    let out = sb.ok(&["skills", "uninstall", "claude", "codex", "opencode"]);
    assert!(out.contains("opencode: not installed"), "{out}");
    assert!(!sb.claude().exists());
    assert!(!sb.codex().exists());
    assert!(sb.path("home/.claude/skills/mine/SKILL.md").is_file());
}

#[test]
fn an_old_link_is_reported_and_replaced_and_its_target_stays() {
    let sb = Sandbox::new();
    let clone = sb.path("clone/skills/deck");
    write(&clone.join("SKILL.md"), SKILL);
    fs::create_dir_all(sb.claude().parent().unwrap()).unwrap();
    symlink(&clone, sb.claude()).unwrap();
    let status = sb.ok(&["skills", "status"]);
    assert!(
        status.starts_with(&format!("claude: linked to {}", clone.display())),
        "{status}"
    );
    let out = sb.ok(&["skills", "install", "claude"]);
    assert!(out.contains("replaced the link"), "{out}");
    assert!(!fs::symlink_metadata(sb.claude()).unwrap().is_symlink());
    assert_eq!(listing(&clone), ["SKILL.md"]);

    // Uninstall removes a working link alone, too.
    fs::create_dir_all(sb.opencode().parent().unwrap()).unwrap();
    symlink(&clone, sb.opencode()).unwrap();
    sb.ok(&["skills", "uninstall", "opencode"]);
    assert!(fs::symlink_metadata(sb.opencode()).is_err());
    assert_eq!(listing(&clone), ["SKILL.md"]);

    // A link to nothing is deck's too; uninstall removes the link alone.
    fs::create_dir_all(sb.codex().parent().unwrap()).unwrap();
    symlink(sb.path("gone"), sb.codex()).unwrap();
    sb.ok(&["skills", "uninstall", "codex"]);
    assert!(fs::symlink_metadata(sb.codex()).is_err());
}

#[test]
fn a_skill_that_is_not_deck_s_is_refused() {
    let sb = Sandbox::new();
    // Another skill named deck, and deck's file next to a file of the user's.
    write(
        &sb.claude().join("SKILL.md"),
        "---\nname: other\n---\nmine\n",
    );
    write(&sb.codex().join("SKILL.md"), SKILL);
    write(&sb.codex().join("notes.md"), "mine");
    for agent in ["claude", "codex"] {
        for action in ["install", "uninstall"] {
            let out = sb.run(&["skills", action, agent]);
            assert!(!out.status.success(), "{action} {agent}");
            let err = String::from_utf8_lossy(&out.stderr);
            assert!(err.contains("was not written by deck"), "{err}");
        }
    }
    // A refusal of one agent writes none of the others.
    let out = sb.run(&["skills", "install", "claude", "opencode"]);
    assert!(!out.status.success());
    assert!(!sb.opencode().exists());
    let status = sb.ok(&["skills", "status"]);
    assert!(status.contains("is not deck's skill"), "{status}");
    assert_eq!(
        fs::read_to_string(sb.claude().join("SKILL.md")).unwrap(),
        "---\nname: other\n---\nmine\n"
    );
    assert_eq!(listing(&sb.codex()), ["SKILL.md", "notes.md"]);
}

#[test]
fn the_config_directories_of_the_agents_are_honoured() {
    let sb = Sandbox::new();
    let vars = [
        ("CLAUDE_CONFIG_DIR", sb.path("claude-dir")),
        ("OPENCODE_CONFIG_DIR", sb.path("opencode-dir")),
    ];
    let out = sb.run_with(&["skills", "install", "claude", "opencode"], &vars);
    assert!(out.status.success());
    assert!(sb.path("claude-dir/skills/deck/SKILL.md").is_file());
    assert!(sb.path("opencode-dir/skills/deck/SKILL.md").is_file());
    let xdg = [("XDG_CONFIG_HOME", sb.path("xdg"))];
    assert!(
        sb.run_with(&["skills", "install", "opencode"], &xdg)
            .status
            .success()
    );
    assert!(sb.path("xdg/opencode/skills/deck/SKILL.md").is_file());
}

#[test]
fn opencode_status_names_the_copy_it_reads_instead() {
    let sb = Sandbox::new();
    sb.ok(&["skills", "install", "codex"]);
    let status = sb.ok(&["skills", "status"]);
    assert!(
        status.contains("opencode: not installed (OpenCode reads the copy in ~/.agents/skills)"),
        "{status}"
    );
}
