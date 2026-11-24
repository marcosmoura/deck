//! One handoff, from `SIGUSR2` to the `execve`: it either replaces the process or ends with
//! the daemon as it was.

use std::ffi::OsString;
use std::os::fd::{AsRawFd, RawFd};
use std::path::Path;
use std::sync::Arc;

use deck_core::protocol::NoticeLevel;

use super::{
    HANDOFF_VERSION, HandoffTest, Marker, PREFLIGHT_LIMIT, Upgrade, take_request, write_result,
};
use crate::server::Shared;
use crate::startup::DaemonPaths;
use crate::{TEST_PROTOCOL_VERSION_VAR, platform};

/// The descriptors the new image inherits besides the terminals' masters.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Inherited {
    pub listener: RawFd,
    pub lock: RawFd,
    pub state_lock: RawFd,
}

/// Why a handoff did not happen.
enum Stop {
    /// Nothing was touched.
    Refused(String),
    /// The daemon was frozen, and has been given back.
    Failed(String),
}

/// Runs a handoff. Returns only when it did not happen (the process is replaced when it
/// did): the result file, the log and a notice say why. The caller then acts on what
/// [`Upgrade::end`] returns.
pub(crate) async fn attempt(
    shared: &Arc<Shared>,
    upgrade: &Upgrade,
    paths: &DaemonPaths,
    uid: u32,
    generation: u64,
    inherited: Inherited,
    test: HandoffTest,
) {
    upgrade.begin();
    let Err(stop) = run(shared, upgrade, paths, uid, generation, inherited, test).await else {
        return;
    };
    let (line, text) = match stop {
        Stop::Refused(why) => (format!("refused: {why}"), format!("upgrade refused: {why}")),
        Stop::Failed(why) => (format!("failed: {why}"), format!("upgrade failed: {why}")),
    };
    write_result(&paths.upgrade_result_file(), &line);
    shared.notify(NoticeLevel::Error, text);
}

fn marker(shared: &Shared, generation: u64, upgrading: bool) -> Marker {
    Marker {
        handoff: HANDOFF_VERSION,
        pid: std::process::id(),
        generation,
        terminals: shared.terminal_count(),
        upgrading,
    }
}

async fn run(
    shared: &Arc<Shared>,
    upgrade: &Upgrade,
    paths: &DaemonPaths,
    uid: u32,
    generation: u64,
    inherited: Inherited,
    test: HandoffTest,
) -> Result<(), Stop> {
    let exe = take_request(&paths.upgrade_request_file(), uid).map_err(Stop::Refused)?;
    if shared.st_is_shutting_down() {
        return Err(Stop::Refused("the daemon is shutting down".into()));
    }
    preflight(&exe).await.map_err(Stop::Refused)?;
    tracing::info!("upgrading: {} (handoff {HANDOFF_VERSION})", exe.display());

    let marker_file = paths.marker_file();
    let write_marker = |upgrading: bool| {
        if let Err(e) = marker(shared, generation, upgrading).write(&marker_file) {
            tracing::warn!("cannot write {}: {e}", marker_file.display());
        }
    };
    write_marker(true);
    tokio::time::sleep(test.prefreeze_delay).await;

    let frozen = match shared.freeze(&exe).await {
        Ok(frozen) => frozen,
        Err((why, handles)) => {
            shared.thaw(&handles);
            write_marker(false);
            return Err(Stop::Failed(why));
        }
    };
    let handles = frozen.handles().to_vec();
    tokio::time::sleep(test.frozen_delay).await;
    let give_back = |why: String| {
        shared.thaw(&handles);
        write_marker(false);
        Stop::Failed(why)
    };

    // The layout first, so even an adoption that fails outright restores it.
    let saving = Arc::clone(shared);
    if let Err(e) = tokio::task::spawn_blocking(move || saving.save_now()).await {
        return Err(give_back(format!("cannot save the state: {e}")));
    }

    let control = upgrade.clone();
    let runtime_dir = paths.runtime_dir.clone();
    let building = Arc::clone(shared);
    let written = match tokio::task::spawn_blocking(move || {
        let file = building.collect(&frozen, Vec::new(), &inherited);
        file.and_then(|mut file| {
            file.generation = generation;
            // Last, so that what came while the snapshots were taken is in: termination
            // signals, and a shutdown asked for over the socket (`kill-server`, the last
            // session killed). The new image acts on them once it has adopted.
            file.signals = control.recorded();
            if building.st_is_shutting_down() {
                file.signals.push("a shutdown request".into());
            }
            let fds: Vec<RawFd> = file.terminals.iter().filter_map(|t| t.fd).collect();
            super::file::write(&runtime_dir, &file)
                .map(|f| (f, fds))
                .map_err(|e| format!("cannot write the handoff file: {e}"))
        })
    })
    .await
    {
        Ok(done) => done,
        Err(e) => {
            return Err(give_back(format!("the handoff did not finish: {e}")));
        }
    };
    let (handoff_file, terminal_fds) = match written {
        Ok(done) => done,
        Err(why) => return Err(give_back(why)),
    };

    // The marker keeps saying `upgrading` through the exec; the new image rewrites it.
    let mut fds = terminal_fds;
    fds.extend([inherited.listener, inherited.lock, inherited.state_lock]);
    let adopt_fd = handoff_file.as_raw_fd();
    fds.push(adopt_fd);
    let exe_for_exec = exe.clone();
    let failure =
        tokio::task::spawn_blocking(move || exec(&exe_for_exec, adopt_fd, &fds, test.exec_fail))
            .await
            .unwrap_or_else(|e| format!("the exec thread failed: {e}"));
    drop(handoff_file);
    Err(give_back(failure))
}

/// The last steps: lets the descriptors cross the exec, blocks the signals and replaces the
/// process. Returns the reason when it did not, with everything put back.
fn exec(exe: &Path, adopt_fd: RawFd, fds: &[RawFd], exec_fail: bool) -> String {
    let mut cleared = Vec::new();
    for &fd in fds {
        match platform::set_cloexec(fd, false) {
            Ok(()) => cleared.push(fd),
            Err(e) => {
                restore(&cleared);
                return format!("cannot pass descriptor {fd} on: {e}");
            }
        }
    }
    // The mask survives the exec: a signal that comes now waits for the new image.
    if let Err(e) = platform::block_handoff_signals() {
        restore(&cleared);
        return format!("cannot block signals: {e}");
    }
    let err = if exec_fail {
        std::io::Error::other("forced by DECK_TEST_EXEC_FAIL")
    } else {
        let args = [
            exe.as_os_str().to_owned(),
            OsString::from("daemon"),
            OsString::from("--adopt"),
            OsString::from(adopt_fd.to_string()),
        ];
        // A handoff always moves to the new binary's own protocol version.
        let env: Vec<(OsString, OsString)> = std::env::vars_os()
            .filter(|(k, _)| k != TEST_PROTOCOL_VERSION_VAR)
            .collect();
        platform::execve(exe, &args, &env)
    };
    if let Err(e) = platform::unblock_handoff_signals() {
        tracing::warn!("cannot unblock signals: {e}");
    }
    restore(&cleared);
    format!("cannot start {}: {err}", exe.display())
}

fn restore(cleared: &[RawFd]) {
    for &fd in cleared {
        if let Err(e) = platform::set_cloexec(fd, true) {
            tracing::warn!("cannot close descriptor {fd} on exec again: {e}");
        }
    }
}

/// Asks the new binary whether it reads this handoff: `<exe> daemon --handoff-check <n>`.
async fn preflight(exe: &Path) -> Result<(), String> {
    let mut command = tokio::process::Command::new(exe);
    command
        .args(["daemon", "--handoff-check", &HANDOFF_VERSION.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let output = match tokio::time::timeout(PREFLIGHT_LIMIT, command.output()).await {
        Err(_) => {
            return Err(format!(
                "{} did not answer the check in time",
                exe.display()
            ));
        }
        Ok(Err(e)) => return Err(format!("cannot run {}: {e}", exe.display())),
        Ok(Ok(output)) => output,
    };
    if output.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&output.stderr);
    let said = said.lines().next().unwrap_or("").trim();
    Err(format!(
        "{} cannot take over ({}){}{said}",
        exe.display(),
        output.status,
        if said.is_empty() { "" } else { ": " }
    ))
}
