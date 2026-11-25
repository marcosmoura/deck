//! A terminal: a PTY with its child process, reader, writer and wait threads, and its
//! emulator.

use std::cell::Cell;
use std::io::{Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use deck_core::model::{SessionId, TerminalId};
use deck_core::osc7::Osc7Scanner;
use deck_core::protocol::CellPx;
use portable_pty::{Child, ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::{Notify, oneshot};

use crate::agents::AgentRt;
use crate::clock::Drain;
use crate::emu::Emu;
use crate::env::Env;
use crate::lock;
use crate::platform::{AdoptedChild, AdoptedMaster};
use crate::server::Shared;

mod carry;
mod gate;
use gate::Gate;
pub use gate::PauseTimeout;

/// The daemon's own argv, as [`crate::platform::foreground_process`] reads a process's.
fn daemon_argv() -> &'static Vec<String> {
    static ARGV: OnceLock<Vec<String>> = OnceLock::new();
    ARGV.get_or_init(|| {
        std::env::args_os()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    })
}

/// At most one frame per terminal in this interval.
pub const FRAME_INTERVAL: Duration = Duration::from_millis(16);
/// The least wait before the pass that follows a synchronized output's timeout.
const HOLD_POLL: Duration = Duration::from_millis(5);

/// Waits (each after the one before) until a resized PTY's real size is checked again.
const PTY_RECHECKS: [Duration; 3] = [
    Duration::from_millis(50),
    Duration::from_millis(200),
    Duration::from_millis(750),
];
/// A terminal whose process is gone is dropped after this much silence on its PTY.
pub const DRAIN_SILENCE: Duration = Duration::from_millis(200);

/// How the child is polled for its exit (the child stays killable).
const WAIT_POLL_MIN: Duration = Duration::from_millis(10);
const WAIT_POLL_MAX: Duration = Duration::from_millis(250);
/// How often the reader thread looks at its pause switch while the PTY is quiet.
const READ_POLL: Duration = Duration::from_millis(50);
/// The longest `pause_reader` and `pause_waiter` wait for their thread to stop.
const PAUSE_LIMIT: Duration = Duration::from_secs(1);
/// How long a nudged PTY keeps its changed size.
const NUDGE_BACK: Duration = Duration::from_millis(100);
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
    /// The outer terminal's cell size in pixels, when a client reported it.
    pub cell: Option<CellPx>,
}

/// A running terminal.
pub struct TermHandle {
    pub id: TerminalId,
    pub session: SessionId,
    pub program: String,
    pub pid: Option<u32>,
    /// When the process started, on the daemon's clock.
    pub started: Duration,
    emu: EmuThread,
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    input: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    queued: Arc<InputBudget>,
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    /// Woken when the emulator has something new to show.
    pub dirty: Notify,
    /// Woken on every chunk of output and at the end of the stream.
    pub activity: Notify,
    last_data: AtomicU64,
    /// Bytes the reader thread read from the PTY since it started.
    bytes_read: AtomicU64,
    /// The exit code, set once the process ended and its output was read.
    finished: Mutex<Option<i32>>,
    eof: AtomicBool,
    exited: AtomicBool,
    closed: AtomicBool,
    /// Counts `resize_pty` calls, so a recheck knows a newer resize replaced it.
    resizes: AtomicU64,
    /// The shell reported its directory with OSC 7 at least once.
    reports_cwd: AtomicBool,
    /// What the agent state machine remembers between events and ticks.
    pub(crate) agent_rt: Mutex<AgentRt>,
    /// Pauses the `pty-reader` thread.
    reader_gate: Gate,
    /// Pauses the `pty-wait` thread.
    waiter_gate: Gate,
    /// The exit status the waiter reaped, kept for `pause_waiter`.
    reaped: Mutex<Option<i32>>,
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

    /// Resizes the PTY and the emulator as one step, on the emulator thread, so
    /// `sync_pty` never sees one without the other.
    pub fn resize_pty(&self, cols: u16, rows: u16, cell: Option<CellPx>) {
        self.resizes.fetch_add(1, Ordering::AcqRel);
        let master = Arc::clone(&self.master);
        let id = self.id;
        let done = self.emu.call(move |e| {
            set_pty_size(&master, id, cols, rows, cell);
            e.resize_with_cell(cols, rows, cell);
        });
        if done.is_none() {
            // Released: only the PTY is left to resize.
            set_pty_size(&self.master, self.id, cols, rows, cell);
        }
    }

    /// The size and pixel size the kernel holds for the PTY (`TIOCGWINSZ`).
    pub fn pty_size(&self) -> Option<PtyGeometry> {
        pty_size(&self.master)
    }

    /// Checks the PTY's real size shortly after the last resize and sets it again when
    /// it differs. On macOS under load a resize can be lost or undone although the ioctl
    /// succeeded, which would leave the program with a stale size for good. A newer
    /// resize restarts the schedule; there is no timer while the size is steady.
    pub fn recheck_pty(self: &Arc<Self>, rt: &tokio::runtime::Handle) {
        let h = Arc::clone(self);
        let n = self.resizes.load(Ordering::Acquire);
        rt.spawn(async move {
            for wait in PTY_RECHECKS {
                tokio::time::sleep(wait).await;
                if h.is_closed() || h.resizes.load(Ordering::Acquire) != n {
                    return;
                }
                h.sync_pty();
            }
        });
    }

    /// Sets the PTY to the emulator's size and cell size when they differ.
    fn sync_pty(&self) {
        let master = Arc::clone(&self.master);
        let id = self.id;
        self.emu.call(move |e| {
            let (cols, rows) = e.size();
            let cell = e.cell();
            if pty_size(&master) != Some(((cols, rows), pixels_of(cols, rows, cell))) {
                set_pty_size(&master, id, cols, rows, cell);
            }
        });
    }

    /// The foreground process of the PTY: its pid and argv. `None` while the terminal's own
    /// process has not run its program yet: between fork and exec it still holds the
    /// daemon's argv.
    pub fn foreground(&self) -> Option<(u32, Vec<String>)> {
        let (pid, argv) = crate::platform::foreground_process(lock(&self.master).as_ref())?;
        (Some(pid) != self.pid || argv != *daemon_argv()).then_some((pid, argv))
    }

    /// Runs `f` on the emulator; `None` once the terminal was released.
    /// It blocks until the result is back and must not be called from the emulator
    /// thread itself.
    pub fn with_emu<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Emu) -> R + Send + 'static,
    ) -> Option<R> {
        self.emu.call(f)
    }

    /// Drops the emulator and the input channel (its writer thread ends); the reader
    /// then discards whatever a background job still prints.
    pub fn release(&self) {
        self.emu.release();
        *lock(&self.input) = None;
    }

    /// Ends the input channel: what is written to the terminal from now on is dropped. A
    /// kept terminal whose process ended keeps its emulator, so its screen stays.
    pub fn stop_input(&self) {
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

    /// The descriptor of the PTY master; `None` for a terminal that has no PTY any more.
    pub fn master_fd(&self) -> Option<std::os::fd::RawFd> {
        lock(&self.master).as_raw_fd()
    }

    /// Input bytes queued for the program and not yet written to the PTY.
    pub fn pending_input(&self) -> usize {
        self.queued.queued.load(Ordering::Acquire)
    }

    /// Makes the program see a real size change, so a full-screen program redraws: the PTY
    /// gets one row less (one more for a one-row terminal) now and its size back 100 ms
    /// later, unless a resize came meanwhile. The emulator is not touched.
    pub fn nudge_pty(self: &Arc<Self>, rt: &tokio::runtime::Handle) {
        let Some(((cols, rows), (pixel_width, pixel_height))) = self.pty_size() else {
            return;
        };
        let n = self.resizes.load(Ordering::Acquire);
        let nudged = PtySize {
            rows: if rows > 1 { rows - 1 } else { rows + 1 },
            cols,
            pixel_width,
            pixel_height,
        };
        if let Err(e) = lock(&self.master).resize(nudged) {
            tracing::warn!(terminal = %self.id, "cannot nudge the PTY: {e}");
            return;
        }
        let h = Arc::clone(self);
        rt.spawn(async move {
            tokio::time::sleep(NUDGE_BACK).await;
            if h.is_closed() || h.resizes.load(Ordering::Acquire) != n {
                return;
            }
            let size = PtySize {
                rows,
                cols,
                pixel_width,
                pixel_height,
            };
            if let Err(e) = lock(&h.master).resize(size) {
                tracing::warn!(terminal = %h.id, "cannot restore the PTY size: {e}");
            }
        });
    }

    /// Records the exit code: the process ended and its output was read.
    pub fn finish(&self, code: i32) {
        *lock(&self.finished) = Some(code);
    }

    /// The exit code once the process ended and its output was read.
    pub fn finished(&self) -> Option<i32> {
        *lock(&self.finished)
    }

    /// Bytes read from the PTY since the terminal started.
    pub fn bytes_read(&self) -> u64 {
        self.bytes_read.load(Ordering::Relaxed)
    }

    /// When the program last printed, on the daemon's clock (zero before its first output).
    pub fn last_output(&self) -> Duration {
        Duration::from_nanos(self.last_data.load(Ordering::Acquire))
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

    /// Stops the reader thread and returns once it has fed what it read to the emulator, or
    /// at the end of the stream. Output written meanwhile stays in the kernel.
    ///
    /// Returns the bytes the reader holds back: an unfinished escape sequence or character
    /// at the end of what it read. The emulator is at the start of a sequence, so a copy of
    /// the terminal made from a snapshot must be fed these bytes next, before the output
    /// still in the kernel. The reader keeps them: after `resume_reader` it goes on as if
    /// there had been no pause.
    pub fn pause_reader(&self) -> Result<Vec<u8>, PauseTimeout> {
        self.reader_gate.pause(PAUSE_LIMIT).inspect_err(|_| {
            tracing::warn!(terminal = %self.id, "the PTY reader did not stop");
        })
    }

    pub fn resume_reader(&self) {
        self.reader_gate.resume();
    }

    /// Stops the wait thread from calling `waitpid`, and returns the exit status it had
    /// already reaped, if any. A child that exits meanwhile stays unreaped.
    pub fn pause_waiter(&self) -> Result<Option<i32>, PauseTimeout> {
        self.waiter_gate.pause(PAUSE_LIMIT).inspect_err(|_| {
            tracing::warn!(terminal = %self.id, "the PTY wait thread did not stop");
        })?;
        Ok(*lock(&self.reaped))
    }

    pub fn resume_waiter(&self) {
        self.waiter_gate.resume();
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

/// The pixel size of a PTY of `cols` by `rows` cells; 0 by 0 while the cell size is unknown.
pub fn pixels_of(cols: u16, rows: u16, cell: Option<CellPx>) -> (u16, u16) {
    cell.map_or((0, 0), |c| {
        (cols.saturating_mul(c.width), rows.saturating_mul(c.height))
    })
}

fn set_pty_size(
    master: &Mutex<Box<dyn MasterPty + Send>>,
    id: TerminalId,
    cols: u16,
    rows: u16,
    cell: Option<CellPx>,
) {
    let (pixel_width, pixel_height) = pixels_of(cols, rows, cell);
    let size = PtySize {
        rows,
        cols,
        pixel_width,
        pixel_height,
    };
    if let Err(e) = lock(master).resize(size) {
        tracing::warn!(terminal = %id, "cannot resize the PTY: {e}");
    }
}

type PtyGeometry = ((u16, u16), (u16, u16));

fn pty_size(master: &Mutex<Box<dyn MasterPty + Send>>) -> Option<PtyGeometry> {
    lock(master)
        .get_size()
        .ok()
        .map(|s| ((s.cols, s.rows), (s.pixel_width, s.pixel_height)))
}

type Job = Box<dyn FnOnce(&mut Emu) + Send>;

thread_local! {
    /// The terminal whose emulator thread this is, on that thread.
    static EMU_OF: Cell<Option<TerminalId>> = const { Cell::new(None) };
}

/// The thread that owns a terminal's emulator and runs, in order, the closures sent to it.
/// It ends after `release` (or the drop of the handle) once the queued closures ran; it is
/// never joined.
struct EmuThread {
    id: TerminalId,
    jobs: Mutex<Option<mpsc::Sender<Job>>>,
}

impl EmuThread {
    fn spawn(
        id: TerminalId,
        cols: u16,
        rows: u16,
        cell: Option<CellPx>,
        scrollback: usize,
    ) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel::<Job>();
        let (ready_tx, ready) = mpsc::sync_channel::<Result<(), String>>(1);
        std::thread::Builder::new()
            .name(format!("emu-{id}"))
            .spawn(move || {
                EMU_OF.with(|c| c.set(Some(id)));
                let mut emu = match Emu::new(cols, rows, scrollback) {
                    Ok(mut emu) => {
                        if cell.is_some() {
                            emu.resize_with_cell(cols, rows, cell);
                        }
                        let _ = ready_tx.send(Ok(()));
                        emu
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e.to_string()));
                        return;
                    }
                };
                while let Ok(job) = rx.recv() {
                    job(&mut emu);
                }
            })?;
        // The emulator is built on its thread; wait for it, so a failure ends `start`.
        match ready.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(std::io::Error::other(e)),
            Err(_) => return Err(std::io::Error::other("the emulator thread ended")),
        }
        Ok(Self {
            id,
            jobs: Mutex::new(Some(tx)),
        })
    }

    /// Runs `f` on the emulator and waits for its result; `None` once released, or when
    /// `f` panicked.
    fn call<R: Send + 'static>(&self, f: impl FnOnce(&mut Emu) -> R + Send + 'static) -> Option<R> {
        debug_assert!(
            EMU_OF.with(Cell::get) != Some(self.id),
            "with_emu called from the emulator thread of terminal {}",
            self.id
        );
        let tx = lock(&self.jobs).clone()?;
        let (done, result) = mpsc::sync_channel(1);
        let id = self.id;
        let job: Job = Box::new(move |e| {
            let outcome = catch_unwind(AssertUnwindSafe(|| f(e)));
            if outcome.is_err() {
                tracing::error!(terminal = %id, "a closure panicked on the emulator thread");
            }
            let _ = done.send(outcome.ok());
        });
        tx.send(job).ok()?;
        result.recv().ok().flatten()
    }

    /// Ends the thread after the closures already queued.
    fn release(&self) {
        *lock(&self.jobs) = None;
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
    /// The PTY and process of a terminal the old daemon handed over: the master's descriptor
    /// (which this takes over) and the pid of its child.
    pub fn adopted(fd: std::os::fd::RawFd, pid: u32) -> Result<Self, String> {
        let master = AdoptedMaster::from_fd(crate::platform::own_fd(fd));
        let reader = master
            .try_clone_reader()
            .map_err(|e| format!("cannot read the PTY: {e}"))?;
        let writer = master
            .take_writer()
            .map_err(|e| format!("cannot write the PTY: {e}"))?;
        Ok(Self {
            master: Box::new(master),
            child: Box::new(AdoptedChild::new(pid)),
            reader,
            writer,
        })
    }

    /// The parts of a terminal whose program ended before the handoff: nothing to serve.
    pub fn finished(cols: u16, rows: u16, cell: Option<CellPx>) -> Self {
        let (pixel_width, pixel_height) = pixels_of(cols, rows, cell);
        let (master, child) = crate::platform::finished(PtySize {
            rows,
            cols,
            pixel_width,
            pixel_height,
        });
        Self {
            master,
            child,
            reader: Box::new(std::io::empty()),
            writer: Box::new(std::io::sink()),
        }
    }

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
    let (pixel_width, pixel_height) = pixels_of(req.cols, req.rows, req.cell);
    let size = PtySize {
        rows: req.rows,
        cols: req.cols,
        pixel_width,
        pixel_height,
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

/// What a terminal brings from the daemon that handed it over.
#[derive(Debug)]
pub struct Seed {
    pub cols: u16,
    pub rows: u16,
    pub cell: Option<CellPx>,
    /// Escape sequences that rebuild the screens ([`Emu::snapshot`]).
    pub snapshot: Vec<u8>,
    /// What the old reader held back; fed to the emulator after the snapshot.
    pub held: Vec<u8>,
    /// An exit status the old daemon had already reaped.
    pub reaped: Option<i32>,
    pub reports_cwd: bool,
    pub started: Duration,
    pub last_data: Duration,
    /// The exit code of a terminal whose program ended and was read to the end: it only
    /// keeps its screen, with no thread and no process.
    pub finished: Option<i32>,
}

/// Serves a process started by [`open`]: its emulator, threads and tasks.
pub fn start(
    shared: &Arc<Shared>,
    id: TerminalId,
    session: SessionId,
    req: &SpawnRequest,
    parts: Parts,
) -> Result<Arc<TermHandle>, String> {
    start_with(
        shared,
        id,
        session,
        &req.program,
        (req.cols, req.rows, req.cell),
        parts,
        None,
    )
}

/// Serves a terminal handed over by the old daemon, from its parts ([`Parts::adopted`] or
/// [`Parts::finished`]) and what it brought.
pub fn adopt(
    shared: &Arc<Shared>,
    id: TerminalId,
    session: SessionId,
    program: &str,
    parts: Parts,
    seed: Seed,
) -> Result<Arc<TermHandle>, String> {
    let geometry = (seed.cols, seed.rows, seed.cell);
    start_with(shared, id, session, program, geometry, parts, Some(seed))
}

fn start_with(
    shared: &Arc<Shared>,
    id: TerminalId,
    session: SessionId,
    program: &str,
    (cols, rows, cell): (u16, u16, Option<CellPx>),
    parts: Parts,
    seed: Option<Seed>,
) -> Result<Arc<TermHandle>, String> {
    if std::env::var_os(crate::TEST_LEAK_FD_VAR).is_some_and(|v| v == "1") {
        // Test hook: a descriptor that is never closed.
        std::mem::forget(std::fs::File::open("/dev/null"));
    }
    let (input_tx, input_rx) = mpsc::channel::<Vec<u8>>();
    let read_fd = parts.master.as_raw_fd();
    let killer = parts.child.clone_killer();
    let pid = parts.child.process_id();
    let queued = Arc::new(InputBudget::default());
    let emu = match EmuThread::spawn(id, cols, rows, cell, shared.config().scrollback_lines) {
        Ok(emu) => emu,
        Err(e) => {
            parts.discard();
            return Err(format!("could not start {program}: {e}"));
        }
    };
    let seed = seed.map(|mut seed| {
        // The new emulator takes up where the old one was: the snapshot, then the sequence
        // the old reader had not finished.
        let (snapshot, held) = (
            std::mem::take(&mut seed.snapshot),
            std::mem::take(&mut seed.held),
        );
        emu.call(move |e| {
            e.feed(&snapshot);
            e.feed(&held);
        });
        seed
    });
    let finished = seed.as_ref().and_then(|s| s.finished);
    let handle = Arc::new(TermHandle {
        id,
        session,
        program: program.to_string(),
        pid,
        started: seed
            .as_ref()
            .map_or_else(|| shared.clock.now(), |s| s.started),
        emu,
        master: Arc::new(Mutex::new(parts.master)),
        input: Mutex::new(Some(input_tx)),
        queued: Arc::clone(&queued),
        child: Arc::new(Mutex::new(parts.child)),
        killer: Mutex::new(killer),
        dirty: Notify::new(),
        activity: Notify::new(),
        last_data: AtomicU64::new(seed.as_ref().map_or(0, |s| nanos(s.last_data))),
        bytes_read: AtomicU64::new(0),
        finished: Mutex::new(finished),
        eof: AtomicBool::new(finished.is_some()),
        exited: AtomicBool::new(finished.is_some()),
        closed: AtomicBool::new(false),
        resizes: AtomicU64::new(0),
        reports_cwd: AtomicBool::new(seed.as_ref().is_some_and(|s| s.reports_cwd)),
        agent_rt: Mutex::new(AgentRt::default()),
        reader_gate: Gate::default(),
        waiter_gate: Gate::default(),
        reaped: Mutex::new(None),
    });

    if finished.is_some() {
        // Nothing runs: the screen stays, and what is typed to it is dropped.
        handle.reader_gate.finish();
        handle.waiter_gate.finish();
        *lock(&handle.input) = None;
        return Ok(handle);
    }
    spawn_writer(parts.writer, input_rx, queued);
    spawn_reader(shared, &handle, parts.reader, read_fd);
    let exit_rx = spawn_waiter(&handle, seed.and_then(|s| s.reaped));

    // Frames: one per interval while the emulator has news.
    let (s, h) = (Arc::clone(shared), Arc::clone(&handle));
    shared.handle.spawn(async move {
        // While the program holds synchronized output, the pass that follows its timeout.
        let mut hold: Option<Duration> = None;
        loop {
            match hold.take() {
                Some(wait) => {
                    tokio::select! {
                        () = h.dirty.notified() => {}
                        () = tokio::time::sleep(wait.max(HOLD_POLL)) => {}
                    }
                }
                None => h.dirty.notified().await,
            }
            if h.is_closed() {
                return;
            }
            hold = s.pump(&h);
            tokio::time::sleep(FRAME_INTERVAL).await;
            // The interval counts towards the wait for the hold's timeout.
            hold = hold.map(|wait| wait.saturating_sub(FRAME_INTERVAL));
        }
    });

    // Exit: read what is left, then drop the terminal.
    let (s, h) = (Arc::clone(shared), Arc::clone(&handle));
    shared.handle.spawn(async move {
        let status = exit_rx.await.unwrap_or(1);
        h.drain(&s).await;
        s.terminal_gone(&h, status);
    });
    Ok(handle)
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

/// What a chunk of output goes through on its way to the emulator.
struct Pump {
    osc7: Osc7Scanner,
    files: crate::emu::files::Scanner,
}

impl Pump {
    fn feed(&mut self, s: &Arc<Shared>, h: &Arc<TermHandle>, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        h.last_data.store(nanos(s.clock.now()), Ordering::Release);
        self.files.scan(bytes, |piece| s.output(h, piece));
        for path in self.osc7.feed(bytes) {
            s.osc7_cwd(h, path);
        }
        h.activity.notify_one();
    }
}

fn spawn_reader(
    shared: &Arc<Shared>,
    handle: &Arc<TermHandle>,
    mut reader: Box<dyn Read + Send>,
    fd: Option<std::os::fd::RawFd>,
) {
    let (s, h) = (Arc::clone(shared), Arc::clone(handle));
    std::thread::Builder::new()
        .name("pty-reader".into())
        .spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            let mut pump = Pump {
                osc7: Osc7Scanner::new(&s.hostname),
                // Reads the files kitty graphics commands name, here and not on the emulator
                // thread.
                files: crate::emu::files::Scanner::default(),
            };
            // An unfinished escape sequence or character at the end of the last chunk, which
            // waits for the rest of it.
            let mut carry: Vec<u8> = Vec::new();
            loop {
                // Everything read so far is in the emulator here, bar what the carry and the
                // scanner hold, so this is where a pause takes effect; the poll bounds how
                // long a pause waits for it.
                h.reader_gate.checkpoint(|| {
                    let mut held = pump.files.pending().to_vec();
                    held.extend_from_slice(&carry);
                    held
                });
                if let Some(fd) = fd
                    && !crate::platform::poll_readable(fd, READ_POLL)
                {
                    // Quiet: the rest of a held sequence is not coming soon. Unless a pause
                    // began meanwhile, which takes the carry as it is.
                    if !carry.is_empty() {
                        h.reader_gate.unless_requested(|| {
                            pump.feed(&s, &h, &carry);
                            carry.clear();
                        });
                    }
                    continue;
                }
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let Some(bytes) = buf.get(..n) else {
                            break;
                        };
                        h.bytes_read.fetch_add(n as u64, Ordering::Relaxed);
                        // The unfinished tail waits for the next chunk (or the quiet above);
                        // without a poll there is no quiet to wait for, so nothing is held.
                        let data = if carry.is_empty() {
                            std::borrow::Cow::Borrowed(bytes)
                        } else {
                            carry.extend_from_slice(bytes);
                            std::borrow::Cow::Owned(std::mem::take(&mut carry))
                        };
                        let end = if fd.is_some() {
                            carry::incomplete_tail(&data)
                        } else {
                            data.len()
                        };
                        let (now, held) = data.split_at(end);
                        carry.extend_from_slice(held);
                        pump.feed(&s, &h, now);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    // EIO once the child is gone (Linux), or any other end of the stream.
                    Err(_) => break,
                }
            }
            // The end of the stream: what was held is all there will be.
            pump.feed(&s, &h, &carry);
            h.eof.store(true, Ordering::Release);
            h.reader_gate.finish();
            h.activity.notify_one();
        })
        .map_err(|e| tracing::error!("cannot start a PTY reader thread: {e}"))
        .ok();
}

/// Starts the wait thread. `reaped` is an exit status the old daemon already took from the
/// child, which is then not waited for again.
fn spawn_waiter(handle: &Arc<TermHandle>, mut reaped: Option<i32>) -> oneshot::Receiver<i32> {
    let (tx, rx) = oneshot::channel();
    let h = Arc::clone(handle);
    std::thread::Builder::new()
        .name("pty-wait".into())
        .spawn(move || {
            let mut poll = WAIT_POLL_MIN;
            loop {
                h.waiter_gate.checkpoint(Vec::new);
                // The daemon reaps the child itself, so a death by signal has its code;
                // portable-pty reports it as 1. Without a pid, portable-pty's wait is all
                // there is.
                let polled = match h.pid {
                    _ if reaped.is_some() => Ok(reaped.take()),
                    Some(pid) => crate::platform::wait_status(pid),
                    None => lock(&h.child)
                        .try_wait()
                        .map(|s| s.map(|s| i32::try_from(s.exit_code()).unwrap_or(1))),
                };
                match polled {
                    Ok(Some(code)) => {
                        *lock(&h.reaped) = Some(code);
                        h.exited.store(true, Ordering::Release);
                        h.waiter_gate.finish();
                        let _ = tx.send(code);
                        return;
                    }
                    Ok(None) => {
                        h.waiter_gate.sleep(poll);
                        poll = (poll * 2).min(WAIT_POLL_MAX);
                    }
                    Err(e) => {
                        tracing::warn!(terminal = %h.id, "cannot wait for the child: {e}");
                        *lock(&h.reaped) = Some(1);
                        h.exited.store(true, Ordering::Release);
                        h.waiter_gate.finish();
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

    fn thread() -> EmuThread {
        EmuThread::spawn(TerminalId(7), 10, 4, None, 100).expect("spawn")
    }

    #[test]
    fn closures_run_in_order_on_a_named_thread() {
        let t = thread();
        assert_eq!(t.call(|e| e.feed(b"ab").len()), Some(0));
        assert_eq!(t.call(|e| e.size()), Some((10, 4)));
        t.call(|e| e.resize(20, 5));
        assert_eq!(t.call(|e| e.size()), Some((20, 5)));
        let name = t.call(|_| std::thread::current().name().map(str::to_owned));
        assert_eq!(name, Some(Some(format!("emu-{}", TerminalId(7)))));
    }

    #[test]
    fn the_pty_gets_the_cell_size_as_pixels() {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 4,
                cols: 10,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("pty");
        let master = Mutex::new(pair.master);
        let cell = Some(CellPx {
            width: 9,
            height: 18,
        });
        assert_eq!(pixels_of(10, 4, cell), (90, 72));
        set_pty_size(&master, TerminalId(1), 10, 4, cell);
        assert_eq!(pty_size(&master), Some(((10, 4), (90, 72))));
        set_pty_size(&master, TerminalId(1), 12, 5, None);
        assert_eq!(pty_size(&master), Some(((12, 5), (0, 0))));
    }

    struct EndSignal(mpsc::Sender<()>);

    impl Drop for EndSignal {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }

    thread_local! {
        static END: Cell<Option<EndSignal>> = const { Cell::new(None) };
    }

    #[test]
    fn release_ends_the_thread_and_later_calls_return_none() {
        let t = thread();
        let (tx, ended) = mpsc::channel();
        // Dropped when the thread exits.
        t.call(move |_| END.with(|c| c.set(Some(EndSignal(tx)))));
        t.release();
        assert_eq!(t.call(|e| e.size()), None);
        ended
            .recv_timeout(Duration::from_secs(10))
            .expect("the emulator thread ends after release");
    }

    #[test]
    fn a_panicking_closure_returns_none_and_the_thread_keeps_serving() {
        let t = thread();
        assert_eq!(t.call(|_| -> u8 { panic!("boom") }), None);
        assert_eq!(t.call(|e| e.size()), Some((10, 4)));
    }

    #[test]
    fn concurrent_callers_run_one_at_a_time_each_with_its_own_result() {
        let t = Arc::new(thread());
        let running = Arc::new(AtomicUsize::new(0));
        let workers: Vec<_> = (0..8usize)
            .map(|n| {
                let (t, running) = (Arc::clone(&t), Arc::clone(&running));
                std::thread::spawn(move || {
                    for i in 0..50usize {
                        let running = Arc::clone(&running);
                        let got = t.call(move |_| {
                            assert_eq!(running.fetch_add(1, Ordering::SeqCst), 0);
                            running.fetch_sub(1, Ordering::SeqCst);
                            n * 1000 + i
                        });
                        assert_eq!(got, Some(n * 1000 + i));
                    }
                })
            })
            .collect();
        for w in workers {
            w.join().expect("worker");
        }
    }

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

    /// A started terminal running `script` in `/bin/sh`, on a runtime of its own.
    struct Running {
        shared: Arc<Shared>,
        handle: Arc<TermHandle>,
        _dir: tempfile::TempDir,
    }

    fn running(script: &str) -> Running {
        let dir = tempfile::tempdir().expect("dir");
        let path = |name: &str| dir.path().join(name);
        let paths = crate::DaemonPaths {
            runtime_dir: dir.path().to_path_buf(),
            socket: path("deck.sock"),
            lock: path("deck.lock"),
            state_dir: dir.path().to_path_buf(),
            state_lock: path("state.lock"),
            log_file: path("deck.log"),
        };
        let (shared, _shutdown) = Shared::new(
            paths,
            deck_core::config::Config::default(),
            Vec::new(),
            Arc::new(crate::SystemClock::new()),
            0,
            crate::server::Probes {
                cwd: crate::CwdQuery::default(),
                foreground: crate::ForegroundQuery::default(),
                listing: Arc::default(),
            },
            Arc::default(),
        );
        let req = SpawnRequest {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            cwd: dir.path().to_path_buf(),
            env: vec![("PATH".into(), "/usr/bin:/bin".into())],
            cols: 80,
            rows: 24,
            cell: None,
        };
        let parts = open(&req).expect("open");
        let handle = start(&shared, TerminalId(1), SessionId(1), &req, parts).expect("start");
        Running {
            shared,
            handle,
            _dir: dir,
        }
    }

    fn until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while !done() {
            assert!(std::time::Instant::now() < deadline, "{what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn lines(h: &TermHandle) -> Vec<String> {
        let id = h.id;
        h.with_emu(move |e| e.capture(id, None, true, false).lines)
            .unwrap_or_default()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_paused_reader_leaves_the_output_in_the_kernel() {
        let t = running(
            "read go; i=0; while [ $i -lt 2500 ]; do printf 'line%05d xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\\n' $i; i=$((i+1)); done; sleep 60",
        );
        let h = &t.handle;
        assert!(h.pause_reader().is_ok());
        let before = h.bytes_read();
        h.write(b"go\n".to_vec());
        // 2,500 lines are more than a PTY holds: the child blocks, nothing is read.
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(h.bytes_read(), before, "a paused reader read");
        h.resume_reader();
        until("the last line", || {
            lines(h).last().is_some_and(|l| l.starts_with("line02499"))
        });
        let got: Vec<String> = lines(h)
            .into_iter()
            .filter(|l| l.starts_with("line"))
            .collect();
        assert_eq!(got.len(), 2500, "bytes were lost");
        for (n, line) in got.iter().enumerate() {
            assert!(line.starts_with(&format!("line{n:05} ")), "{n}: {line}");
        }
        h.hang_up();
        drop(t.shared);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pausing_the_reader_is_quick_when_idle_and_after_the_child_exited() {
        let idle = running("sleep 60");
        // Let the reader get going, then ask.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let started = std::time::Instant::now();
        assert!(idle.handle.pause_reader().is_ok());
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "{:?}",
            started.elapsed()
        );
        // Pausing twice and resuming are harmless.
        assert!(idle.handle.pause_reader().is_ok());
        idle.handle.resume_reader();
        idle.handle.resume_reader();
        idle.handle.hang_up();

        let gone = running("echo bye");
        until("the end of the stream", || {
            gone.handle.eof.load(Ordering::Acquire)
        });
        let started = std::time::Instant::now();
        assert!(gone.handle.pause_reader().is_ok());
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "{:?}",
            started.elapsed()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_paused_waiter_does_not_reap_and_a_reaped_status_is_returned() {
        let t = running("read go; exit 5");
        let h = &t.handle;
        assert_eq!(h.pause_waiter(), Ok(None));
        h.write(b"go\n".to_vec());
        // The waiter polls at least every 250 ms: it would have reaped by now.
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert!(!h.has_exited(), "the waiter reaped while paused");
        h.resume_waiter();
        until("the child to be reaped", || h.has_exited());
        assert_eq!(h.pause_waiter(), Ok(Some(5)));
        // Resuming a waiter that ended is harmless.
        h.resume_waiter();
        until("the terminal to finish", || h.finished() == Some(5));
        drop(t.shared);
    }

    /// A child prints `first`, waits, then prints `second`, with no echo. The reader is
    /// paused after `first` was read, then `second` is left in the kernel: the held bytes,
    /// the snapshot and the kernel's rest make a copy that looks like the source.
    async fn split_stream(first: &[u8], second: &[u8], held: &[u8]) {
        let octal = |bytes: &[u8]| {
            bytes
                .iter()
                .map(|b| format!("\\{b:03o}"))
                .collect::<String>()
        };
        let t = running(&format!(
            "stty -echo; printf R; read a; printf '{}'; read b; printf '{}'; sleep 60",
            octal(first),
            octal(second)
        ));
        let h = &t.handle;
        until("the start", || lines(h).first().is_some_and(|l| l == "R"));
        h.write(b"a\n".to_vec());
        // The reader gives what it holds to the emulator after `READ_POLL` of quiet, so the
        // pause has to follow the read at once: this wait does not sleep.
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while h.bytes_read() != 1 + first.len() as u64 {
            assert!(std::time::Instant::now() < deadline, "the first part");
            std::thread::yield_now();
        }
        let pending = h.pause_reader().expect("paused");
        assert_eq!(pending, held, "held bytes");
        let snapshot = h.with_emu(|e| e.snapshot()).expect("snapshot");
        h.write(b"b\n".to_vec());
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            h.bytes_read(),
            1 + first.len() as u64,
            "a paused reader read"
        );

        // What a new daemon does: the snapshot, the held bytes, then what the kernel has.
        let mut copy = Emu::new(80, 24, 10_000).expect("emulator");
        copy.feed(&snapshot);
        copy.feed(&pending);
        copy.feed(second);
        let mut whole = Emu::new(80, 24, 10_000).expect("emulator");
        whole.feed(&[b"R", first, second].concat());
        let id = h.id;
        h.resume_reader();
        until("the second part", || {
            h.bytes_read() >= (1 + first.len() + second.len()) as u64
        });
        let mut frame = None;
        until("the frame", || {
            frame = h.with_emu(move |e| e.full_frame(id));
            frame == Some(whole.full_frame(id))
        });
        assert_eq!(copy.full_frame(id), whole.full_frame(id), "the copy");
        h.hang_up();
        drop(t.shared);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_pause_after_a_lone_escape_holds_it() {
        split_stream(b"hi \x1b", b"[31mred\x1b[0m done", b"\x1b").await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_pause_inside_a_csi_holds_it() {
        split_stream(b"hi \x1b[31", b"mred\x1b[0m done", b"\x1b[31").await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_pause_inside_an_osc_title_holds_it() {
        split_stream(b"hi \x1b]2;tit", b"le\x07 done", b"\x1b]2;tit").await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_pause_inside_a_utf8_character_holds_it() {
        split_stream(b"caf\xc3", b"\xa9 done", b"\xc3").await;
        split_stream(b"x\xe6\x97", b"\xa5 done", b"\xe6\x97").await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_pause_inside_a_kitty_command_holds_it() {
        split_stream(
            b"a\x1b_Gi=1,a=q;AAAA",
            b"\x1b\\ done",
            b"\x1b_Gi=1,a=q;AAAA",
        )
        .await;
    }
}
