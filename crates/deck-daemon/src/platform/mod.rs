//! The `unsafe` of the daemon: questions to the operating system. Everything else is safe
//! code.

#![allow(unsafe_code)]

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "linux")]
pub use linux::{process_argv, process_cwd};
#[cfg(target_os = "macos")]
pub use macos::{process_argv, process_cwd};

/// The foreground process of a PTY: the leader of its foreground process group (`tcgetpgrp`
/// on the master) and its argv as the kernel holds it. `None` when either cannot be read,
/// for example when the process just ended.
pub fn foreground_process(master: &dyn portable_pty::MasterPty) -> Option<(u32, Vec<String>)> {
    let pid = u32::try_from(master.process_group_leader()?).ok()?;
    let argv = process_argv(pid)?;
    (!argv.is_empty()).then_some((pid, argv))
}

/// The exit code in a raw `waitpid` status: the code of a normal exit, `128 + n` for a death
/// by signal `n`.
fn decode_status(raw: libc::c_int) -> i32 {
    if libc::WIFEXITED(raw) {
        libc::WEXITSTATUS(raw)
    } else if libc::WIFSIGNALED(raw) {
        128 + libc::WTERMSIG(raw)
    } else {
        1
    }
}

/// Reaps the child `pid` when it has ended, without blocking: its exit code, or `None` while
/// it runs. portable-pty reports a death by signal as code 1 with only a text name, so the
/// daemon asks the operating system itself. The same POSIX call on macOS and Linux.
pub fn wait_status(pid: u32) -> std::io::Result<Option<i32>> {
    let pid = libc::pid_t::try_from(pid)
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    loop {
        let mut raw: libc::c_int = 0;
        // SAFETY: `raw` is a valid `c_int` that `waitpid` writes the status to; `WNOHANG`
        // makes the call return at once.
        let rc = unsafe { libc::waitpid(pid, &mut raw, libc::WNOHANG) };
        if rc == 0 {
            return Ok(None);
        }
        if rc == pid {
            return Ok(Some(decode_status(raw)));
        }
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::EINTR) {
            return Err(err);
        }
    }
}

/// The machine's host name, empty when it cannot be read.
pub fn hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: `buf` is valid for `buf.len()` bytes; `gethostname` writes at most that many
    // and `buf` is zeroed, so a name cut at the limit still ends at a NUL below.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len() - 1) };
    if rc != 0 {
        return String::new();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(0);
    buf.get(..end)
        .map(|name| String::from_utf8_lossy(name).into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    #[test]
    fn a_raw_status_maps_to_an_exit_code() {
        // Raw statuses as `waitpid` returns them: the code in the second byte, or the
        // signal in the low seven bits.
        assert_eq!(decode_status(0), 0);
        assert_eq!(decode_status(3 << 8), 3);
        assert_eq!(decode_status(101 << 8), 101);
        assert_eq!(decode_status(libc::SIGTERM), 143);
        assert_eq!(decode_status(libc::SIGKILL), 137);
        assert_eq!(decode_status(libc::SIGHUP), 129);
    }

    /// The code of a child that ends by itself, as `wait_status` reads it on this OS.
    #[allow(clippy::zombie_processes, reason = "`wait_status` reaps the child")]
    fn code_of(script: &str) -> Option<i32> {
        let child = Command::new("/bin/sh")
            .args(["-c", script])
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if let Some(code) = wait_status(child.id()).unwrap() {
                return Some(code);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        None
    }

    #[test]
    #[allow(clippy::zombie_processes, reason = "`wait_status` reaps the child")]
    fn wait_status_reads_an_exit_and_a_signal_death() {
        assert_eq!(code_of("exit 3"), Some(3));
        assert_eq!(code_of("kill -TERM $$"), Some(143));
        // Reaped: a second ask is an error, not a status.
        let child = Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .spawn()
            .unwrap();
        let pid = child.id();
        while wait_status(pid).unwrap().is_none() {}
        assert!(wait_status(pid).is_err());
    }

    #[test]
    fn the_hostname_matches_the_hostname_command() {
        let expected = if cfg!(target_os = "linux") {
            std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap()
        } else {
            let out = Command::new("hostname").output().unwrap();
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        assert_eq!(hostname(), expected.trim_end());
    }
}
