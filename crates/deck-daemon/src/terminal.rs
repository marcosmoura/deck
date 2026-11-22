//! A terminal: a PTY with its child process, reader, writer and wait threads, and its
//! emulator.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use deck_core::model::{SessionId, TerminalId};
use deck_core::osc7::Osc7Scanner;
use portable_pty::{Child, ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::{Notify, oneshot};

use crate::clock::Drain;
use crate::emu::Emu;
use crate::env::Env;
use crate::lock;
use crate::server::Shared;

/// At most one frame per terminal in this interval.
pub const FRAME_INTERVAL: Duration = Duration::from_millis(16);
/// A terminal whose process is gone is dropped after this much silence on its PTY.
pub const DRAIN_SILENCE: Duration = Duration::from_millis(200);

/// How the child is polled for its exit (no `unsafe` wait, and the child stays killable).
const WAIT_POLL_MIN: Duration = Duration::from_millis(10);
const WAIT_POLL_MAX: Duration = Duration::from_millis(250);
/// Input queued for a program that does not read it is dropped past this many bytes.
pub const INPUT_CAP: usize = 1 << 20;

#[derive(Debug, Clone)]
pub struct SpawnRequest {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Env,
    pub cols: u16,
    pub rows: u16,
}

/// A running terminal.
pub struct TermHandle {
    pub id: TerminalId,
    pub session: SessionId,
    pub program: String,
    pub pid: Option<u32>,
    /// When the process started, on the daemon's clock.
    pub started: Duration,
    emu: Mutex<Option<Emu>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    input: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    queued: Arc<InputBudget>,
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    /// Woken when the emulator has something new to show.
    pub dirty: Notify,
    /// Woken on every chunk of output and at the end of the stream.
    pub activity: Notify,
    last_data: AtomicU64,
    eof: AtomicBool,
    exited: AtomicBool,
    closed: AtomicBool,
    /// The shell reported its directory with OSC 7 at least once.
    reports_cwd: AtomicBool,
}

impl std::fmt::Debug for TermHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TermHandle")
            .field("id", &self.id)
            .field("pid", &self.pid)
            .finish_non_exhaustive()
    }
}

fn nanos(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
}

impl TermHandle {
    /// Sends bytes to the program, without blocking. Past [`INPUT_CAP`] bytes still
    /// waiting for the program, the input is dropped.
    pub fn write(&self, bytes: Vec<u8>) {
        let len = bytes.len();
        if !self.queued.take(len) {
            if self.queued.start_dropping() {
                tracing::warn!(terminal = %self.id, "the program is not reading its input; dropping input");
            }
            return;
        }
        match lock(&self.input).as_ref() {
            Some(tx) if tx.send(bytes).is_ok() => {}
            _ => {
                self.queued.give_back(len);
                tracing::debug!(terminal = %self.id, "input for a terminal whose writer ended");
            }
        }
    }

    pub fn resize_pty(&self, cols: u16, rows: u16) {
        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        if let Err(e) = lock(&self.master).resize(size) {
            tracing::warn!(terminal = %self.id, "cannot resize the PTY: {e}");
        }
        self.with_emu(|e| e.resize(cols, rows));
    }

    /// Runs `f` on the emulator; `None` once the terminal was released.
    pub fn with_emu<R>(&self, f: impl FnOnce(&mut Emu) -> R) -> Option<R> {
        lock(&self.emu).as_mut().map(f)
    }

    /// Drops the emulator and the input channel (its writer thread ends); the reader
    /// then discards whatever a background job still prints.
    pub fn release(&self) {
        *lock(&self.emu) = None;
        *lock(&self.input) = None;
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.dirty.notify_one();
    }

    pub fn reports_cwd(&self) -> bool {
        self.reports_cwd.load(Ordering::Acquire)
    }

    pub fn set_reports_cwd(&self) {
        self.reports_cwd.store(true, Ordering::Release);
    }

    pub fn has_exited(&self) -> bool {
        self.exited.load(Ordering::Acquire)
    }

    /// Asks the process to hang up (`SIGHUP`).
    pub fn hang_up(&self) {
        if !self.has_exited()
            && let Err(e) = lock(&self.killer).kill()
        {
            tracing::debug!(terminal = %self.id, "cannot hang up the child: {e}");
        }
    }

    /// Kills a child that ignored the hang-up: `SIGHUP` once more, then `SIGKILL`.
    pub fn kill(&self) {
        if !self.has_exited()
            && let Err(e) = lock(&self.child).kill()
        {
            tracing::debug!(terminal = %self.id, "cannot kill the child: {e}");
        }
    }

    /// Waits until the process exited and its output was read to the end or went quiet for
    /// [`DRAIN_SILENCE`], measured on the daemon's clock.
    pub async fn drain(&self, shared: &Shared) {
        let drain = Drain::new(DRAIN_SILENCE);
        let exited_at = shared.clock.now();
        loop {
            if self.eof.load(Ordering::Acquire) {
                return;
            }
            let last = Duration::from_nanos(self.last_data.load(Ordering::Acquire)).max(exited_at);
            let Some(wait) = drain.remaining(last, shared.clock.now()) else {
                return;
            };
            tokio::select! {
                () = self.activity.notified() => {}
                () = tokio::time::sleep(wait) => {}
            }
        }
    }
}

/// Counts the input bytes queued for the writer thread.
#[derive(Debug, Default)]
struct InputBudget {
    queued: AtomicUsize,
    dropping: AtomicBool,
}

impl InputBudget {
    /// Reserves `len` bytes; refused past [`INPUT_CAP`], except for one write into an empty
    /// queue (a large paste still goes through).
    fn take(&self, len: usize) -> bool {
        let before = self.queued.fetch_add(len, Ordering::AcqRel);
        if before > 0 && before.saturating_add(len) > INPUT_CAP {
            self.queued.fetch_sub(len, Ordering::AcqRel);
            return false;
        }
        self.dropping.store(false, Ordering::Release);
        true
    }

    fn give_back(&self, len: usize) {
        self.queued.fetch_sub(len, Ordering::AcqRel);
    }

    /// `true` for the first refused write since input last went through.
    fn start_dropping(&self) -> bool {
        !self.dropping.swap(true, Ordering::AcqRel)
    }
}

/// A started process and its PTY, not yet served by any thread.
pub struct Parts {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
}

impl Parts {
    /// Kills a process that will never be served, and reaps it, on a thread of its own:
    /// the kill waits up to 200 ms for `SIGHUP` before `SIGKILL`.
    pub fn discard(self) {
        let mut child = self.child;
        std::thread::Builder::new()
            .name("pty-discard".into())
            .spawn(move || {
                let _ = child.kill();
                let _ = child.wait();
            })
            .map_err(|e| tracing::error!("cannot start a PTY reaper thread: {e}"))
            .ok();
    }
}

/// Opens a PTY and starts the process in it. Blocking (`fork` and `exec`): call it from
/// `spawn_blocking`, never on an async task.
pub fn open(req: &SpawnRequest) -> Result<Parts, String> {
    open_pty(req).map_err(|e| format!("could not start {}: {e}", req.program))
}

fn open_pty(req: &SpawnRequest) -> Result<Parts, String> {
    if !req.cwd.is_dir() {
        return Err(format!(
            "working directory {} is not a directory",
            req.cwd.display()
        ));
    }
    let size = PtySize {
        rows: req.rows,
        cols: req.cols,
        pixel_width: 0,
        pixel_height: 0,
    };
    let pair = native_pty_system()
        .openpty(size)
        .map_err(|e| format!("cannot open a PTY: {e}"))?;
    let mut cmd = CommandBuilder::new(&req.program);
    cmd.args(&req.args);
    cmd.env_clear();
    for (k, v) in &req.env {
        cmd.env(k, v);
    }
    cmd.cwd(&req.cwd);
    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("{e:#}"))?;
    drop(pair.slave);
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("cannot read the PTY: {e}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("cannot write the PTY: {e}"))?;
    Ok(Parts {
        master: pair.master,
        child,
        reader,
        writer,
    })
}

/// Serves a process started by [`open`]: its emulator, threads and tasks.
pub fn start(
    shared: &Arc<Shared>,
    id: TerminalId,
    session: SessionId,
    req: &SpawnRequest,
    parts: Parts,
) -> Arc<TermHandle> {
    let (input_tx, input_rx) = mpsc::channel::<Vec<u8>>();
    let killer = parts.child.clone_killer();
    let pid = parts.child.process_id();
    let queued = Arc::new(InputBudget::default());
    let handle = Arc::new(TermHandle {
        id,
        session,
        program: req.program.clone(),
        pid,
        started: shared.clock.now(),
        emu: Mutex::new(Some(Emu::new(
            req.cols,
            req.rows,
            shared.config().scrollback_lines,
        ))),
        master: Mutex::new(parts.master),
        input: Mutex::new(Some(input_tx)),
        queued: Arc::clone(&queued),
        child: Arc::new(Mutex::new(parts.child)),
        killer: Mutex::new(killer),
        dirty: Notify::new(),
        activity: Notify::new(),
        last_data: AtomicU64::new(0),
        eof: AtomicBool::new(false),
        exited: AtomicBool::new(false),
        closed: AtomicBool::new(false),
        reports_cwd: AtomicBool::new(false),
    });

    spawn_writer(parts.writer, input_rx, queued);
    spawn_reader(shared, &handle, parts.reader);
    let exit_rx = spawn_waiter(&handle);

    // Frames: one per interval while the emulator has news.
    let (s, h) = (Arc::clone(shared), Arc::clone(&handle));
    shared.handle.spawn(async move {
        loop {
            h.dirty.notified().await;
            if h.is_closed() {
                return;
            }
            s.pump(&h);
            tokio::time::sleep(FRAME_INTERVAL).await;
        }
    });

    // Exit: read what is left, then drop the terminal.
    let (s, h) = (Arc::clone(shared), Arc::clone(&handle));
    shared.handle.spawn(async move {
        let status = exit_rx.await.unwrap_or(1);
        h.drain(&s).await;
        s.terminal_gone(&h, status);
    });
    handle
}

fn spawn_writer(
    mut writer: Box<dyn Write + Send>,
    rx: mpsc::Receiver<Vec<u8>>,
    queued: Arc<InputBudget>,
) {
    std::thread::Builder::new()
        .name("pty-writer".into())
        .spawn(move || {
            while let Ok(bytes) = rx.recv() {
                let written = writer.write_all(&bytes).and_then(|()| writer.flush());
                queued.give_back(bytes.len());
                if written.is_err() {
                    return;
                }
            }
        })
        .map_err(|e| tracing::error!("cannot start a PTY writer thread: {e}"))
        .ok();
}

fn spawn_reader(shared: &Arc<Shared>, handle: &Arc<TermHandle>, mut reader: Box<dyn Read + Send>) {
    let (s, h) = (Arc::clone(shared), Arc::clone(handle));
    std::thread::Builder::new()
        .name("pty-reader".into())
        .spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            let mut osc7 = Osc7Scanner::new(&s.hostname);
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let Some(bytes) = buf.get(..n) else {
                            break;
                        };
                        h.last_data.store(nanos(s.clock.now()), Ordering::Release);
                        s.output(&h, bytes);
                        for path in osc7.feed(bytes) {
                            s.osc7_cwd(&h, path);
                        }
                        h.activity.notify_one();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    // EIO once the child is gone (Linux), or any other end of the stream.
                    Err(_) => break,
                }
            }
            h.eof.store(true, Ordering::Release);
            h.activity.notify_one();
        })
        .map_err(|e| tracing::error!("cannot start a PTY reader thread: {e}"))
        .ok();
}

fn spawn_waiter(handle: &Arc<TermHandle>) -> oneshot::Receiver<u32> {
    let (tx, rx) = oneshot::channel();
    let h = Arc::clone(handle);
    std::thread::Builder::new()
        .name("pty-wait".into())
        .spawn(move || {
            let mut poll = WAIT_POLL_MIN;
            loop {
                let polled = lock(&h.child).try_wait();
                match polled {
                    Ok(Some(status)) => {
                        h.exited.store(true, Ordering::Release);
                        let _ = tx.send(status.exit_code());
                        return;
                    }
                    Ok(None) => {
                        std::thread::sleep(poll);
                        poll = (poll * 2).min(WAIT_POLL_MAX);
                    }
                    Err(e) => {
                        tracing::warn!(terminal = %h.id, "cannot wait for the child: {e}");
                        h.exited.store(true, Ordering::Release);
                        let _ = tx.send(1);
                        return;
                    }
                }
            }
        })
        .map_err(|e| tracing::error!("cannot start a PTY wait thread: {e}"))
        .ok();
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_past_the_cap_is_refused_until_the_writer_catches_up() {
        let budget = InputBudget::default();
        assert!(budget.take(INPUT_CAP - 10));
        assert!(budget.take(10));
        assert!(!budget.take(1));
        assert!(budget.start_dropping());
        assert!(!budget.take(1));
        assert!(!budget.start_dropping(), "warned once per episode");
        budget.give_back(INPUT_CAP - 10);
        assert!(budget.take(1));
        assert!(budget.start_dropping(), "a new episode warns again");
    }

    #[test]
    fn one_write_larger_than_the_cap_goes_through_an_empty_queue() {
        let budget = InputBudget::default();
        assert!(budget.take(4 * INPUT_CAP));
        assert!(!budget.take(1));
        budget.give_back(4 * INPUT_CAP);
        assert!(budget.take(1));
    }
}
