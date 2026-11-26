//! What the live upgrade asks of the operating system: descriptors that survive `execve`,
//! the signal mask, `execve` itself, and the checks the new image makes on what it
//! inherited.

use std::ffi::{CString, OsStr, OsString};
use std::io;
use std::os::fd::{FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

/// The signals the old image does not act on while it hands over, and holds blocked across
/// the `execve`.
const HANDOFF_SIGNALS: [libc::c_int; 4] =
    [libc::SIGTERM, libc::SIGINT, libc::SIGHUP, libc::SIGUSR2];

/// Takes ownership of a descriptor this process inherited: one the handoff file names, which
/// the old image left open on purpose. Fails for a standard descriptor and for a number
/// that is not open.
///
/// That nothing else in the process owns the number cannot be checked here. It holds
/// because `prepare_adoption` refuses a handoff file that names a descriptor twice, and
/// each named descriptor is taken once.
pub fn own_fd(fd: RawFd) -> io::Result<OwnedFd> {
    if fd <= libc::STDERR_FILENO {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    // SAFETY: `fcntl` with `F_GETFD` takes only the descriptor.
    if unsafe { libc::fcntl(fd, libc::F_GETFD) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` is open (checked above) and, by the note above, owned by nobody else, so
    // the new `OwnedFd` is its only owner and closes it once.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Sets or clears `FD_CLOEXEC` on `fd`.
pub fn set_cloexec(fd: RawFd, on: bool) -> io::Result<()> {
    // SAFETY: `fcntl` with `F_GETFD` takes only the descriptor.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    let wanted = if on {
        flags | libc::FD_CLOEXEC
    } else {
        flags & !libc::FD_CLOEXEC
    };
    // SAFETY: `fcntl` with `F_SETFD` takes the descriptor and an integer of flags.
    if unsafe { libc::fcntl(fd, libc::F_SETFD, wanted) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The device and inode of what `fd` is open on.
pub fn fd_identity(fd: RawFd) -> io::Result<(u64, u64)> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::zeroed();
    // SAFETY: `stat` is a zeroed `stat`, valid for `fstat` to write.
    if unsafe { libc::fstat(fd, stat.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the call succeeded, so it filled the struct; zero bytes were valid before.
    let stat = unsafe { stat.assume_init() };
    #[allow(
        clippy::unnecessary_cast,
        reason = "the field types differ between macOS and Linux"
    )]
    Ok((stat.st_dev as u64, stat.st_ino as u64))
}

/// Whether `fd` is a socket that listens.
pub fn is_listening_socket(fd: RawFd) -> bool {
    let mut on: libc::c_int = 0;
    let mut len = libc::socklen_t::try_from(size_of::<libc::c_int>()).unwrap_or(4);
    // SAFETY: `on` and `len` are valid for `getsockopt` to write; `len` is `on`'s size.
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_ACCEPTCONN,
            (&raw mut on).cast(),
            &raw mut len,
        )
    };
    if rc == 0 {
        return on != 0;
    }
    // macOS does not answer `SO_ACCEPTCONN` for Unix sockets. `connect` tells instead and
    // changes nothing: a socket that listens refuses it with `EOPNOTSUPP` whatever the
    // address, and any other descriptor fails another way (`ENOTSOCK`, `EISCONN`, or for the
    // address, which names no socket). `listen` would answer too, but it makes a socket
    // that is only bound listen.
    // SAFETY: zero bytes are a valid `sockaddr_un`: an address with an empty path.
    let mut addr = unsafe { std::mem::MaybeUninit::<libc::sockaddr_un>::zeroed().assume_init() };
    addr.sun_family = libc::sa_family_t::try_from(libc::AF_UNIX).unwrap_or(1);
    let len = libc::socklen_t::try_from(size_of::<libc::sockaddr_un>()).unwrap_or(0);
    // SAFETY: `addr` is a valid `sockaddr_un` of `len` bytes that `connect` only reads.
    let rc = unsafe { libc::connect(fd, (&raw const addr).cast(), len) };
    rc != 0 && io::Error::last_os_error().raw_os_error() == Some(libc::EOPNOTSUPP)
}

fn mask(how: libc::c_int) -> io::Result<()> {
    let mut set = std::mem::MaybeUninit::<libc::sigset_t>::zeroed();
    // SAFETY: `set` is valid for `sigemptyset` and `sigaddset` to write.
    unsafe {
        libc::sigemptyset(set.as_mut_ptr());
        for signal in HANDOFF_SIGNALS {
            libc::sigaddset(set.as_mut_ptr(), signal);
        }
    }
    // SAFETY: `sigemptyset` initialised `set`; a null old set is allowed.
    let rc = unsafe { libc::pthread_sigmask(how, set.as_ptr(), std::ptr::null_mut()) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(rc))
    }
}

/// Blocks `SIGTERM`, `SIGINT`, `SIGHUP` and `SIGUSR2` on the calling thread. The mask
/// survives `execve`.
pub fn block_handoff_signals() -> io::Result<()> {
    mask(libc::SIG_BLOCK)
}

/// Unblocks the four signals on the calling thread.
pub fn unblock_handoff_signals() -> io::Result<()> {
    mask(libc::SIG_UNBLOCK)
}

static EARLY_USR2: AtomicBool = AtomicBool::new(false);
static EARLY_SIGNAL: AtomicI32 = AtomicI32::new(0);

extern "C" fn record_early(signal: libc::c_int) {
    if signal == libc::SIGUSR2 {
        EARLY_USR2.store(true, Ordering::SeqCst);
    } else {
        EARLY_SIGNAL.store(signal, Ordering::SeqCst);
    }
}

/// Installs handlers that only remember that one of the four signals came, then unblocks
/// them, so that a signal that was held across the `execve` is not lost before the real
/// handlers are in place.
pub fn catch_early_signals() -> io::Result<()> {
    for signal in HANDOFF_SIGNALS {
        let mut action = std::mem::MaybeUninit::<libc::sigaction>::zeroed();
        // SAFETY: `action` is a zeroed `sigaction`, valid for `sigemptyset` to write.
        unsafe { libc::sigemptyset(&raw mut (*action.as_mut_ptr()).sa_mask) };
        // SAFETY: the struct was zeroed and its mask initialised above, so it is a valid
        // `sigaction`; the handler only stores into atomics, which is async-signal-safe.
        let mut action = unsafe { action.assume_init() };
        action.sa_sigaction = record_early as extern "C" fn(libc::c_int) as usize;
        action.sa_flags = libc::SA_RESTART;
        // SAFETY: `action` is a valid `sigaction`; a null old action is allowed.
        if unsafe { libc::sigaction(signal, &raw const action, std::ptr::null_mut()) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    unblock_handoff_signals()
}

/// What [`catch_early_signals`] caught, and forgets it.
pub fn take_early_signals() -> (bool, Option<&'static str>) {
    let usr2 = EARLY_USR2.swap(false, Ordering::SeqCst);
    let name = match EARLY_SIGNAL.swap(0, Ordering::SeqCst) {
        libc::SIGTERM => Some("SIGTERM"),
        libc::SIGINT => Some("SIGINT"),
        libc::SIGHUP => Some("SIGHUP"),
        _ => None,
    };
    (usr2, name)
}

fn c_string(text: &OsStr) -> io::Result<CString> {
    CString::new(text.as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

/// Replaces this process with `path`, run with `args` (the first is `argv[0]`) and exactly
/// `env`. Returns only when it failed, with the reason.
pub fn execve(path: &Path, args: &[OsString], env: &[(OsString, OsString)]) -> io::Error {
    let prepared = (|| -> io::Result<_> {
        let path = c_string(path.as_os_str())?;
        let args = args
            .iter()
            .map(|a| c_string(a))
            .collect::<io::Result<Vec<_>>>()?;
        let env = env
            .iter()
            .map(|(k, v)| {
                let mut pair = k.as_bytes().to_vec();
                pair.push(b'=');
                pair.extend_from_slice(v.as_bytes());
                CString::new(pair).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
            })
            .collect::<io::Result<Vec<_>>>()?;
        Ok((path, args, env))
    })();
    let (path, args, env) = match prepared {
        Ok(parts) => parts,
        Err(e) => return e,
    };
    let argv: Vec<*const libc::c_char> = args
        .iter()
        .map(|a| a.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();
    let envp: Vec<*const libc::c_char> = env
        .iter()
        .map(|e| e.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();
    // SAFETY: `path`, `argv` and `envp` point into the `CString`s above, which outlive the
    // call, and both arrays end with a null pointer as `execve` requires.
    unsafe { libc::execve(path.as_ptr(), argv.as_ptr(), envp.as_ptr()) };
    io::Error::last_os_error()
}

/// Closes every descriptor above 2 except those in `keep`: what a new image does when it
/// cannot read the handoff file and starts over.
pub fn close_inherited(keep: &[RawFd]) {
    // SAFETY: `getdtablesize` takes no arguments.
    let limit = unsafe { libc::getdtablesize() }.clamp(0, 65_536);
    for fd in 3..limit {
        if !keep.contains(&fd) {
            // SAFETY: closing a descriptor number is sound; one that is not open fails with
            // `EBADF`, which is ignored. Callers run it first thing in a new image
            // (`deck daemon --adopt`, before the async runtime, tracing or any signal
            // setup), so every descriptor above 2 is one the old image left open and
            // nothing of this process owns one.
            unsafe { libc::close(fd) };
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::fd::AsRawFd;

    use super::*;

    #[test]
    fn cloexec_can_be_cleared_and_set_again() {
        let file = std::fs::File::open("/dev/null").unwrap();
        let fd = file.as_raw_fd();
        // SAFETY: `fcntl` with `F_GETFD` takes only a descriptor, here an open one.
        let flags = |fd| unsafe { libc::fcntl(fd, libc::F_GETFD) };
        assert_ne!(flags(fd) & libc::FD_CLOEXEC, 0);
        set_cloexec(fd, false).unwrap();
        assert_eq!(flags(fd) & libc::FD_CLOEXEC, 0);
        set_cloexec(fd, true).unwrap();
        assert_ne!(flags(fd) & libc::FD_CLOEXEC, 0);
    }

    #[test]
    fn the_identity_of_a_descriptor_matches_its_file() {
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        let file = std::fs::File::create(&path).unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        assert_eq!(
            fd_identity(file.as_raw_fd()).unwrap(),
            (meta.dev(), meta.ino())
        );
    }

    #[test]
    fn only_a_listening_socket_is_one() {
        let dir = tempfile::tempdir().unwrap();
        let listener = std::os::unix::net::UnixListener::bind(dir.path().join("s")).unwrap();
        assert!(is_listening_socket(listener.as_raw_fd()));
        let (a, _b) = std::os::unix::net::UnixStream::pair().unwrap();
        assert!(!is_listening_socket(a.as_raw_fd()));
        let file = std::fs::File::open("/dev/null").unwrap();
        assert!(!is_listening_socket(file.as_raw_fd()));
    }

    #[test]
    fn asking_whether_a_bound_socket_listens_does_not_make_it_listen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s");
        // A stream socket that is bound and does not listen, which std cannot make.
        // SAFETY: `socket` takes three integers.
        let socket = own_fd(unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) }).unwrap();
        // SAFETY: zero bytes are a valid `sockaddr_un`.
        let mut addr =
            unsafe { std::mem::MaybeUninit::<libc::sockaddr_un>::zeroed().assume_init() };
        addr.sun_family = libc::sa_family_t::try_from(libc::AF_UNIX).unwrap();
        let bytes = path.as_os_str().as_bytes();
        assert!(
            bytes.len() < addr.sun_path.len(),
            "the path fits with its NUL"
        );
        for (to, from) in addr.sun_path.iter_mut().zip(bytes) {
            *to = libc::c_char::from_ne_bytes([*from]);
        }
        let len = libc::socklen_t::try_from(size_of::<libc::sockaddr_un>()).unwrap();
        // SAFETY: `addr` is a valid `sockaddr_un` of `len` bytes that `bind` only reads.
        let rc = unsafe { libc::bind(socket.as_raw_fd(), (&raw const addr).cast(), len) };
        assert_eq!(rc, 0, "{}", io::Error::last_os_error());

        assert!(!is_listening_socket(socket.as_raw_fd()));
        // Still not listening: a connection is refused.
        let err = std::os::unix::net::UnixStream::connect(&path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::ConnectionRefused);
    }

    #[test]
    fn only_an_open_descriptor_above_the_standard_ones_can_be_owned() {
        use std::os::fd::IntoRawFd;
        for fd in [-1, 0, 1, 2] {
            assert_eq!(
                own_fd(fd).unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "{fd}"
            );
        }
        let fd = std::fs::File::open("/dev/null").unwrap().into_raw_fd();
        drop(own_fd(fd).unwrap());
        // Closed by the drop above: the number is no descriptor any more.
        assert_eq!(own_fd(fd).unwrap_err().raw_os_error(), Some(libc::EBADF));
    }

    #[test]
    fn execve_reports_a_missing_binary() {
        let err = execve(
            Path::new("/nonexistent/deck"),
            &[OsString::from("deck")],
            &[],
        );
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn the_signals_can_be_blocked_and_unblocked_on_a_thread() {
        std::thread::spawn(|| {
            block_handoff_signals().unwrap();
            unblock_handoff_signals().unwrap();
        })
        .join()
        .unwrap();
    }
}
