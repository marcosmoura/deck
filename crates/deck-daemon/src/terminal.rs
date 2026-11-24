//! A terminal: a PTY with its child process, reader, writer and wait threads, and its
//! emulator.

use std::cell::Cell;
use std::io::{Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
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
use crate::server::Shared;

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

    /// The foreground process of the PTY: its pid and argv.
    pub fn foreground(&self) -> Option<(u32, Vec<String>)> {
        crate::platform::foreground_process(lock(&self.master).as_ref())
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

    /// Records the exit code: the process ended and its output was read.
    pub fn finish(&self, code: i32) {
        *lock(&self.finished) = Some(code);
    }

    /// The exit code once the process ended and its output was read.
    pub fn finished(&self) -> Option<i32> {
        *lock(&self.finished)
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

/// Serves a process started by [`open`]: its emulator, threads and tasks.
pub fn start(
    shared: &Arc<Shared>,
    id: TerminalId,
    session: SessionId,
    req: &SpawnRequest,
    parts: Parts,
) -> Result<Arc<TermHandle>, String> {
    let (input_tx, input_rx) = mpsc::channel::<Vec<u8>>();
    let killer = parts.child.clone_killer();
    let pid = parts.child.process_id();
    let queued = Arc::new(InputBudget::default());
    let emu = match EmuThread::spawn(
        id,
        req.cols,
        req.rows,
        req.cell,
        shared.config().scrollback_lines,
    ) {
        Ok(emu) => emu,
        Err(e) => {
            parts.discard();
            return Err(format!("could not start {}: {e}", req.program));
        }
    };
    let handle = Arc::new(TermHandle {
        id,
        session,
        program: req.program.clone(),
        pid,
        started: shared.clock.now(),
        emu,
        master: Arc::new(Mutex::new(parts.master)),
        input: Mutex::new(Some(input_tx)),
        queued: Arc::clone(&queued),
        child: Arc::new(Mutex::new(parts.child)),
        killer: Mutex::new(killer),
        dirty: Notify::new(),
        activity: Notify::new(),
        last_data: AtomicU64::new(0),
        finished: Mutex::new(None),
        eof: AtomicBool::new(false),
        exited: AtomicBool::new(false),
        closed: AtomicBool::new(false),
        resizes: AtomicU64::new(0),
        reports_cwd: AtomicBool::new(false),
        agent_rt: Mutex::new(AgentRt::default()),
    });

    spawn_writer(parts.writer, input_rx, queued);
    spawn_reader(shared, &handle, parts.reader);
    let exit_rx = spawn_waiter(&handle);

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

fn spawn_reader(shared: &Arc<Shared>, handle: &Arc<TermHandle>, mut reader: Box<dyn Read + Send>) {
    let (s, h) = (Arc::clone(shared), Arc::clone(handle));
    std::thread::Builder::new()
        .name("pty-reader".into())
        .spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            let mut osc7 = Osc7Scanner::new(&s.hostname);
            // Reads the files kitty graphics commands name, here and not on the emulator thread.
            let mut files = crate::emu::files::Scanner::default();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let Some(bytes) = buf.get(..n) else {
                            break;
                        };
                        h.last_data.store(nanos(s.clock.now()), Ordering::Release);
                        files.scan(bytes, |piece| s.output(&h, piece));
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

fn spawn_waiter(handle: &Arc<TermHandle>) -> oneshot::Receiver<i32> {
    let (tx, rx) = oneshot::channel();
    let h = Arc::clone(handle);
    std::thread::Builder::new()
        .name("pty-wait".into())
        .spawn(move || {
            let mut poll = WAIT_POLL_MIN;
            loop {
                // The daemon reaps the child itself, so a death by signal has its code;
                // portable-pty reports it as 1. Without a pid, portable-pty's wait is all
                // there is.
                let polled = match h.pid {
                    Some(pid) => crate::platform::wait_status(pid),
                    None => lock(&h.child)
                        .try_wait()
                        .map(|s| s.map(|s| i32::try_from(s.exit_code()).unwrap_or(1))),
                };
                match polled {
                    Ok(Some(code)) => {
                        h.exited.store(true, Ordering::Release);
                        let _ = tx.send(code);
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
}
