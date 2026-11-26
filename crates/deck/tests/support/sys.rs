//! The only `unsafe` of the end-to-end tests: terminal modes and process checks.

#![allow(unsafe_code)]

use std::os::fd::RawFd;

/// True while a process with this pid exists (`kill(pid, 0)`).
pub fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists; nothing is sent.
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// True when canonical mode (`ICANON`) is on for the terminal behind `fd`.
pub fn canonical(fd: RawFd) -> std::io::Result<bool> {
    let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
    // SAFETY: `termios` points to writable memory of the right type; `tcgetattr` only
    // fills it and only reads `fd`, which the caller keeps open for the call.
    let rc = unsafe { libc::tcgetattr(fd, termios.as_mut_ptr()) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `tcgetattr` returned 0, so it initialised the structure.
    let termios = unsafe { termios.assume_init() };
    Ok(termios.c_lflag & libc::ICANON != 0)
}

/// Sends `SIGTERM` to a process.
pub fn terminate(pid: i32) {
    // SAFETY: plain `kill(2)`; no memory is involved.
    let rc = unsafe { libc::kill(pid, libc::SIGTERM) };
    assert_eq!(rc, 0, "kill: {}", std::io::Error::last_os_error());
}

/// Sends a signal to a process.
pub fn signal(pid: i32, signal: i32) {
    // SAFETY: plain `kill(2)`; no memory is involved.
    let rc = unsafe { libc::kill(pid, signal) };
    assert_eq!(rc, 0, "kill: {}", std::io::Error::last_os_error());
}

/// Waits up to `millis` for `fd` to be readable, hung up or failing; false after the wait.
pub fn wait_readable(fd: RawFd, millis: i32) -> bool {
    let mut poll = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: `poll` is one valid `pollfd` and the count says so; the caller keeps `fd` open.
    let ready = unsafe { libc::poll(&raw mut poll, 1, millis) };
    ready > 0
}

/// What a process has used so far; the difference of two readings is a window's cost.
#[derive(Debug, Clone, Copy, Default)]
pub struct Usage {
    /// User and system CPU time, in seconds.
    pub cpu_secs: f64,
    /// Wake-ups from idle: macOS `ri_interrupt_wkups`; on Linux the
    /// voluntary context switches of all its threads (a thread that sleeps and is woken
    /// counts one), the nearest figure `/proc` has.
    pub wakeups: u64,
    /// Memory the process is charged for, in bytes: macOS `ri_phys_footprint`, Linux `VmRSS`.
    pub memory: u64,
}

impl std::ops::Sub for Usage {
    type Output = Usage;

    /// The CPU time and wake-ups between two readings; `memory` is the later one's.
    fn sub(self, earlier: Usage) -> Usage {
        Usage {
            cpu_secs: self.cpu_secs - earlier.cpu_secs,
            wakeups: self.wakeups.saturating_sub(earlier.wakeups),
            memory: self.memory,
        }
    }
}

/// The usage of the process `pid` (not only of its reaped children, which is all
/// `getrusage(RUSAGE_CHILDREN)` counts); `None` when it is gone or not ours.
#[cfg(target_os = "macos")]
#[allow(
    deprecated,
    reason = "the `mach2` crate is not a dependency; libc still declares the call"
)]
pub fn usage(pid: i32) -> Option<Usage> {
    let mut info = std::mem::MaybeUninit::<libc::rusage_info_v4>::zeroed();
    // SAFETY: `info` is writable memory of the size the V4 flavour fills; the call only
    // writes into it.
    let rc = unsafe {
        libc::proc_pid_rusage(
            pid,
            libc::RUSAGE_INFO_V4,
            info.as_mut_ptr().cast::<libc::rusage_info_t>(),
        )
    };
    if rc != 0 {
        return None;
    }
    // SAFETY: `proc_pid_rusage` returned 0, so it filled the structure.
    let info = unsafe { info.assume_init() };
    let mut timebase = libc::mach_timebase_info { numer: 0, denom: 0 };
    // SAFETY: `timebase` is a valid out-pointer for the call.
    unsafe { libc::mach_timebase_info(&raw mut timebase) };
    // The times are in Mach absolute time units, not nanoseconds (125/3 on Apple Silicon).
    let nanos = (info.ri_user_time + info.ri_system_time) as f64 * f64::from(timebase.numer)
        / f64::from(timebase.denom.max(1));
    Some(Usage {
        cpu_secs: nanos / 1e9,
        // `ri_interrupt_wkups` only: `ri_pkg_idle_wkups` adds nothing on Apple silicon and
        // counts the same wake-ups again on Intel.
        wakeups: info.ri_interrupt_wkups,
        memory: info.ri_phys_footprint,
    })
}

/// See the macOS version.
#[cfg(target_os = "linux")]
pub fn usage(pid: i32) -> Option<Usage> {
    use std::fs;

    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name is in parentheses and may hold spaces: count fields after the last
    // `)`. `utime` and `stime` are fields 14 and 15; the state (field 3) is the first.
    let rest: Vec<&str> = stat
        .get(stat.rfind(')')? + 1..)?
        .split_whitespace()
        .collect();
    let ticks: u64 = rest.get(11)?.parse::<u64>().ok()? + rest.get(12)?.parse::<u64>().ok()?;
    // SAFETY: `sysconf` only reads a system constant.
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1);
    let mut wakeups = 0u64;
    for task in fs::read_dir(format!("/proc/{pid}/task")).ok()?.flatten() {
        let Ok(text) = fs::read_to_string(task.path().join("status")) else {
            continue;
        };
        wakeups += field(&text, "voluntary_ctxt_switches:");
    }
    let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    Some(Usage {
        cpu_secs: ticks as f64 / hz as f64,
        wakeups,
        memory: field(&status, "VmRSS:") * 1024,
    })
}

#[cfg(target_os = "linux")]
fn field(text: &str, name: &str) -> u64 {
    text.lines()
        .find_map(|l| l.strip_prefix(name))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}
