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
