//! The `unsafe` of the binary: process identity, the user at the other end of a socket, liveness,
//! signals, detaching, reading stdin and telling when it is gone. Everything else is safe code.

#![allow(unsafe_code)]

use std::fs::File;
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use deck_tui::StdinState;

/// The effective user id of this process.
pub fn effective_uid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// The effective user id of the process that listens at the other end of a connected Unix
/// socket, as it was when the socket was made.
#[cfg(target_os = "linux")]
pub fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let size = size_of::<libc::ucred>();
    let mut len = libc::socklen_t::try_from(size).map_err(io::Error::other)?;
    // SAFETY: the descriptor is open, and `cred` is valid for writes of `len` bytes, its size.
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut cred).cast(),
            &raw mut len,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    if usize::try_from(len).ok() != Some(size) {
        return Err(io::Error::other("the peer credentials are incomplete"));
    }
    Ok(cred.uid)
}

/// The effective user id of the process that listens at the other end of a connected Unix
/// socket, as it was when the socket was made.
#[cfg(target_os = "macos")]
pub fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    let (mut uid, mut gid): (libc::uid_t, libc::gid_t) = (0, 0);
    // SAFETY: the descriptor is open and both pointers are valid for one write each.
    let rc = unsafe { libc::getpeereid(stream.as_raw_fd(), &raw mut uid, &raw mut gid) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(uid)
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

/// Re-applies stdin's terminal settings unchanged. On Linux that wakes every waiter on the
/// terminal, so input left unread is reported again (`deck_tui::RearmStdin`); elsewhere it does
/// nothing.
pub fn rearm_stdin() {
    #[cfg(target_os = "linux")]
    {
        let mut settings = std::mem::MaybeUninit::<libc::termios>::zeroed();
        // SAFETY: `settings` is a zeroed `termios`, valid for `tcgetattr` to fill.
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, settings.as_mut_ptr()) } != 0 {
            return;
        }
        // SAFETY: the call succeeded, so it filled the struct.
        let settings = unsafe { settings.assume_init() };
        // SAFETY: the terminal's own settings, set back with `TCSANOW` (nothing is flushed).
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw const settings) };
    }
}

/// Waits up to `timeout` for input on stdin without reading it, or for `wake` to be readable
/// (that is `Ready` too), and tells when the terminal is gone: `POLLHUP` or `POLLERR`;
/// `POLLIN` with nothing queued (`FIONREAD` 0 or failing: a revoked tty reads as end of file);
/// or, after a quiet wait, a failing `tcgetattr`. Stdin that is not a terminal is never
/// reported gone: this sleeps for `timeout`.
pub fn wait_stdin(timeout: Duration, wake: BorrowedFd<'_>) -> StdinState {
    // SAFETY: `isatty` only looks at a descriptor number.
    if unsafe { libc::isatty(libc::STDIN_FILENO) } != 1 {
        // Nothing wakes this sleep, so it stays short enough for the thread to see its stop flag.
        std::thread::sleep(timeout.min(Duration::from_millis(50)));
        return StdinState::Idle;
    }
    let mut fds = [
        libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: wake.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    let millis = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
    // SAFETY: `fds` is two valid `pollfd`s and the count says so; `wake` is open for the call.
    let ready = unsafe { libc::poll(fds.as_mut_ptr(), 2, millis) };
    if ready < 0 {
        let interrupted = io::Error::last_os_error().kind() == io::ErrorKind::Interrupted;
        return if interrupted {
            StdinState::Idle
        } else {
            StdinState::HungUp
        };
    }
    let [stdin, woken] = fds;
    if ready > 0 {
        if stdin.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
            return StdinState::HungUp;
        }
        if stdin.revents & libc::POLLIN != 0 {
            let mut queued: libc::c_int = 0;
            // SAFETY: `FIONREAD` writes one `c_int` through the pointer, which is valid.
            let rc = unsafe { libc::ioctl(libc::STDIN_FILENO, libc::FIONREAD, &raw mut queued) };
            return if rc == 0 && queued > 0 {
                StdinState::Ready
            } else {
                StdinState::HungUp
            };
        }
        // The main loop dropped its end: the client is leaving.
        if woken.revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            return StdinState::HungUp;
        }
        if woken.revents & libc::POLLIN != 0 {
            return StdinState::Ready;
        }
    }
    let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
    // SAFETY: `termios` is writable memory of the right type; `tcgetattr` only fills it.
    let rc = unsafe { libc::tcgetattr(libc::STDIN_FILENO, termios.as_mut_ptr()) };
    if rc == 0 {
        StdinState::Idle
    } else {
        StdinState::HungUp
    }
}
