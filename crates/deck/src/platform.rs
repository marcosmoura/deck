//! The `unsafe` of the binary: process identity, liveness, signals, detaching and reading stdin. Everything else is safe code.

#![allow(unsafe_code)]

use std::fs::File;
use std::io;
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// The effective user id of this process.
pub fn effective_uid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// True while a process with this pid exists (`kill(pid, 0)` sends no signal).
pub fn process_exists(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process can be signalled.
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Points stderr (fd 2) at `target` and returns a handle to the original stderr.
pub fn redirect_stderr(target: &File) -> io::Result<File> {
    let original = io::stderr().as_fd().try_clone_to_owned()?;
    // SAFETY: both descriptors are valid for the call; `dup2` replaces fd 2 and keeps
    // `target` open.
    let rc = unsafe { libc::dup2(target.as_raw_fd(), libc::STDERR_FILENO) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(File::from(original))
}

/// Sends `SIGTERM` to a process, which takes the daemon's clean shutdown path.
pub fn terminate(pid: i32) -> io::Result<()> {
    // SAFETY: plain `kill(2)`; no memory is involved.
    if unsafe { libc::kill(pid, libc::SIGTERM) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Sends `SIGUSR2` to the daemon: the request of an upgrade.
pub fn request_upgrade(pid: i32) -> io::Result<()> {
    // SAFETY: plain `kill(2)`; no memory is involved.
    if unsafe { libc::kill(pid, libc::SIGUSR2) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Starts `cmd` in a new session with stdin, stdout and stderr on `/dev/null`, so it
/// outlives this process and the terminal it was started from. A thread reaps it; the flag
/// it returns is set once the process has exited.
pub fn detach_spawn(cmd: &mut Command) -> io::Result<Arc<AtomicBool>> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: the closure runs between `fork` and `exec` and only calls `setsid`, which is
    // async-signal-safe.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn()?;
    let exited = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&exited);
    std::thread::spawn(move || {
        let _ = child.wait();
        flag.store(true, Ordering::Release);
    });
    Ok(exited)
}

/// Waits up to `timeout` for input on stdin and reads what is there: `Some(bytes)` (empty
/// after a timeout or an interrupt), `None` when stdin is closed or fails. It bypasses
/// std's buffering, so nothing stays behind for the event reader.
pub fn read_stdin(timeout: Duration) -> Option<Vec<u8>> {
    let mut fd = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    let millis = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
    // SAFETY: `fd` is one valid `pollfd` and the count says so.
    let ready = unsafe { libc::poll(&raw mut fd, 1, millis) };
    if ready < 0 {
        let interrupted = io::Error::last_os_error().kind() == io::ErrorKind::Interrupted;
        return interrupted.then(Vec::new);
    }
    if ready == 0 {
        return Some(Vec::new());
    }
    let mut buf = [0u8; 4096];
    // SAFETY: `buf` is valid for writes of its length.
    let n = unsafe { libc::read(libc::STDIN_FILENO, buf.as_mut_ptr().cast(), buf.len()) };
    match usize::try_from(n) {
        Ok(0) | Err(_) => None,
        Ok(n) => buf.get(..n).map(<[u8]>::to_vec),
    }
}
