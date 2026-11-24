//! The `unsafe` of the daemon: questions to the operating system. Everything else is safe
//! code.

#![allow(unsafe_code)]

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

use std::time::Duration;

#[cfg(target_os = "linux")]
pub use linux::{process_argv, process_cwd};
#[cfg(target_os = "macos")]
pub use macos::{process_argv, process_cwd};

/// What the daemon process uses; a figure that cannot be read is `None`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SelfStats {
    /// Resident memory in bytes.
    pub rss_bytes: Option<u64>,
    /// Open file descriptors.
    pub fds: Option<u32>,
    pub threads: Option<u32>,
    pub cpu_user: Option<Duration>,
    pub cpu_system: Option<Duration>,
}

/// Measures the daemon process: memory, descriptors, threads and CPU time. Reads files or
/// asks the kernel, so it runs off the async workers.
pub fn self_stats() -> SelfStats {
    let cpu = cpu_times();
    #[cfg(target_os = "linux")]
    let (rss_bytes, fds, threads) = (linux::rss_bytes(), linux::fds(), linux::threads());
    #[cfg(target_os = "macos")]
    let (rss_bytes, fds, threads) = (macos::rss_bytes(), macos::fds(), macos::threads());
    SelfStats {
        rss_bytes,
        fds,
        threads,
        cpu_user: cpu.map(|c| c.0),
        cpu_system: cpu.map(|c| c.1),
    }
}

/// The user and system CPU time of this process, from `getrusage`.
fn cpu_times() -> Option<(Duration, Duration)> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `usage` is a zeroed `rusage`, valid for `getrusage` to write.
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if rc != 0 {
        return None;
    }
    // SAFETY: the call succeeded, so it filled the struct; zero bytes were valid before.
    let usage = unsafe { usage.assume_init() };
    let of = |t: libc::timeval| {
        Duration::new(
            u64::try_from(t.tv_sec).unwrap_or(0),
            u32::try_from(t.tv_usec).unwrap_or(0).saturating_mul(1000),
        )
    };
    Some((of(usage.ru_utime), of(usage.ru_stime)))
}

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
    fn self_stats_are_plausible_and_cpu_does_not_go_back() {
        // A test process may hold only the three standard descriptors; two more make the
        // count's growth checkable.
        let before = self_stats();
        let open: Vec<_> = (0..2)
            .map(|_| std::fs::File::open("/dev/null").unwrap())
            .collect();
        let first = self_stats();
        assert!(first.fds.unwrap() >= before.fds.unwrap() + 2, "{first:?}");
        assert!(first.fds.unwrap() > 3, "{first:?}");
        assert!(first.rss_bytes.unwrap() > 1 << 20);
        assert!(first.threads.unwrap() >= 1);
        // Burn some CPU so the second reading can only be later.
        let mut x = 0u64;
        for i in 0..5_000_000u64 {
            x = x.wrapping_add(std::hint::black_box(i));
        }
        std::hint::black_box(x);
        let second = self_stats();
        drop(open);
        assert!(second.cpu_user.unwrap() >= first.cpu_user.unwrap());
        assert!(second.cpu_system.unwrap() >= first.cpu_system.unwrap());
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
