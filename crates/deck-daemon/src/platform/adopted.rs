//! A PTY master and a child process the daemon did not start itself: the ones a new daemon
//! inherits from the old one across the live upgrade, as a file descriptor and a pid. They
//! plug into the `portable_pty` traits, so a terminal does not care where its PTY came from.

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use portable_pty::{Child, ChildKiller, ExitStatus, MasterPty, PtySize};

/// The wait between the checks after `SIGHUP`, and their number: the grace period of
/// portable-pty's own kill (it checks five times, 50 ms apart).
const KILL_GRACE_STEP: Duration = Duration::from_millis(50);
const KILL_GRACE_CHECKS: u32 = 5;

/// The master side of a PTY, from a descriptor.
#[derive(Debug)]
pub struct AdoptedMaster {
    fd: OwnedFd,
    took_writer: AtomicBool,
}

impl AdoptedMaster {
    pub fn from_fd(fd: OwnedFd) -> Self {
        Self {
            fd,
            took_writer: AtomicBool::new(false),
        }
    }

    fn dup(&self) -> io::Result<File> {
        Ok(File::from(self.fd.try_clone()?))
    }
}

/// Reads a PTY master: the `EIO` of a closed slave side is the end of the stream (as in
/// portable-pty's own reader).
struct Reader(File);

impl Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.0.read(buf) {
            Err(e) if e.raw_os_error() == Some(libc::EIO) => Ok(0),
            other => other,
        }
    }
}

impl MasterPty for AdoptedMaster {
    fn resize(&self, size: PtySize) -> Result<(), anyhow::Error> {
        let ws = libc::winsize {
            ws_row: size.rows,
            ws_col: size.cols,
            ws_xpixel: size.pixel_width,
            ws_ypixel: size.pixel_height,
        };
        // SAFETY: `ws` is a valid `winsize` that outlives the call, and the descriptor is
        // open for as long as `self`.
        let rc = unsafe { libc::ioctl(self.fd.as_raw_fd(), libc::TIOCSWINSZ as _, &raw const ws) };
        if rc != 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(())
    }

    fn get_size(&self) -> Result<PtySize, anyhow::Error> {
        let mut ws = libc::winsize {
            ws_row: 0,
            ws_col: 0,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: `ws` is a valid `winsize` for the call to write, and the descriptor is
        // open for as long as `self`.
        let rc = unsafe { libc::ioctl(self.fd.as_raw_fd(), libc::TIOCGWINSZ as _, &raw mut ws) };
        if rc != 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(PtySize {
            rows: ws.ws_row,
            cols: ws.ws_col,
            pixel_width: ws.ws_xpixel,
            pixel_height: ws.ws_ypixel,
        })
    }

    fn try_clone_reader(&self) -> Result<Box<dyn Read + Send>, anyhow::Error> {
        Ok(Box::new(Reader(self.dup()?)))
    }

    /// A duplicate of the descriptor. Unlike portable-pty's writer it sends no newline and
    /// end-of-file character when dropped.
    fn take_writer(&self) -> Result<Box<dyn Write + Send>, anyhow::Error> {
        if self.took_writer.swap(true, Ordering::AcqRel) {
            anyhow::bail!("cannot take writer more than once");
        }
        Ok(Box::new(self.dup()?))
    }

    fn process_group_leader(&self) -> Option<libc::pid_t> {
        // SAFETY: `tcgetpgrp` only takes the descriptor, which is open for as long as `self`.
        let pid = unsafe { libc::tcgetpgrp(self.fd.as_raw_fd()) };
        (pid > 0).then_some(pid)
    }

    fn as_raw_fd(&self) -> Option<RawFd> {
        Some(AsRawFd::as_raw_fd(&self.fd))
    }

    fn tty_name(&self) -> Option<PathBuf> {
        None
    }
}

/// A child process that is not a `std::process::Child` of this process object, only a pid
/// of one this process started (and so can still wait for).
#[derive(Debug)]
pub struct AdoptedChild {
    pid: u32,
}

impl AdoptedChild {
    pub fn new(pid: u32) -> Self {
        Self { pid }
    }
}

fn status(code: i32) -> ExitStatus {
    ExitStatus::with_exit_code(u32::try_from(code).unwrap_or(1))
}

impl Child for AdoptedChild {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        Ok(super::wait_status(self.pid)?.map(status))
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        super::wait_exit(self.pid).map(status)
    }

    fn process_id(&self) -> Option<u32> {
        Some(self.pid)
    }
}

impl ChildKiller for AdoptedChild {
    /// `SIGHUP`, then `SIGKILL` when the child is still there after the grace period.
    fn kill(&mut self) -> io::Result<()> {
        super::signal(self.pid, libc::SIGHUP)?;
        for attempt in 0..KILL_GRACE_CHECKS {
            if attempt > 0 {
                std::thread::sleep(KILL_GRACE_STEP);
            }
            if matches!(self.try_wait(), Ok(Some(_))) {
                return Ok(());
            }
        }
        super::signal(self.pid, libc::SIGKILL)
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(Hangup { pid: self.pid })
    }
}

/// What `clone_killer` gives out: it only hangs up, like portable-pty's.
#[derive(Debug)]
struct Hangup {
    pid: u32,
}

impl ChildKiller for Hangup {
    fn kill(&mut self) -> io::Result<()> {
        super::signal(self.pid, libc::SIGHUP)
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(Self { pid: self.pid })
    }
}

/// The PTY and process of a terminal whose program ended before the upgrade and that stays
/// on screen (`--keep`): nothing to read, write or signal, and a fixed size.
pub fn finished(size: PtySize) -> (Box<dyn MasterPty + Send>, Box<dyn Child + Send + Sync>) {
    (Box::new(Gone { size }), Box::new(Gone { size }))
}

/// See [`finished`].
#[derive(Debug, Clone, Copy)]
struct Gone {
    size: PtySize,
}

impl MasterPty for Gone {
    fn resize(&self, _size: PtySize) -> Result<(), anyhow::Error> {
        Ok(())
    }

    fn get_size(&self) -> Result<PtySize, anyhow::Error> {
        Ok(self.size)
    }

    fn try_clone_reader(&self) -> Result<Box<dyn Read + Send>, anyhow::Error> {
        Ok(Box::new(io::empty()))
    }

    fn take_writer(&self) -> Result<Box<dyn Write + Send>, anyhow::Error> {
        Ok(Box::new(io::sink()))
    }

    fn process_group_leader(&self) -> Option<libc::pid_t> {
        None
    }

    fn as_raw_fd(&self) -> Option<RawFd> {
        None
    }

    fn tty_name(&self) -> Option<PathBuf> {
        None
    }
}

impl Child for Gone {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        Ok(Some(status(0)))
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        Ok(status(0))
    }

    fn process_id(&self) -> Option<u32> {
        None
    }
}

impl ChildKiller for Gone {
    fn kill(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(*self)
    }
}

#[cfg(test)]
mod tests;
