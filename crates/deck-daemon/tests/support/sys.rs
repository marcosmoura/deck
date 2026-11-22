//! The only `unsafe` of the daemon's tests: process checks and signals by pid.

#![allow(unsafe_code)]

/// True while a process with this pid exists (`kill(pid, 0)`).
pub fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists; nothing is sent.
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Sends `SIGKILL` to a process the test started.
pub fn kill9(pid: i32) {
    // SAFETY: plain `kill(2)` on a pid the test recorded from its own child.
    unsafe {
        libc::kill(pid, libc::SIGKILL);
    }
}
