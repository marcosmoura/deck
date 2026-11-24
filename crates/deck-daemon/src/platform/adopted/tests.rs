use std::io::BufRead;
use std::os::fd::BorrowedFd;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Instant;

use portable_pty::{CommandBuilder, native_pty_system};

use super::*;

fn size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// Collects what a reader gives, so a test can wait for a word without blocking.
struct Output(mpsc::Receiver<Vec<u8>>, Vec<u8>);

impl Output {
    fn of(mut reader: Box<dyn Read + Send>) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    return;
                }
            }
        });
        Self(rx, Vec::new())
    }

    fn until(&mut self, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let text = String::from_utf8_lossy(&self.1).into_owned();
            if text.contains(needle) {
                return text;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            match self.0.recv_timeout(left) {
                Ok(bytes) => self.1.extend(bytes),
                Err(_) => panic!("{needle:?} never came; got {text:?}"),
            }
        }
    }
}

#[test]
fn an_adopted_master_reads_writes_resizes_and_names_the_foreground() {
    let pair = native_pty_system().openpty(size(24, 80)).expect("pty");
    let mut cmd = CommandBuilder::new("/bin/sh");
    cmd.args([
        "-c",
        "echo ready; read a; echo got:$a; read b; echo size:$(stty size); read c",
    ]);
    let mut child = pair.slave.spawn_command(cmd).expect("child");
    drop(pair.slave);
    let raw = pair.master.as_raw_fd().expect("fd");
    // SAFETY: `raw` is the descriptor of `pair.master`, which is alive for this whole test.
    let dup = unsafe { BorrowedFd::borrow_raw(raw) }
        .try_clone_to_owned()
        .expect("dup");
    let master = AdoptedMaster::from_fd(dup);
    assert!(master.tty_name().is_none());
    assert!(master.as_raw_fd().is_some());

    let mut out = Output::of(master.try_clone_reader().expect("reader"));
    let mut writer = master.take_writer().expect("writer");
    assert!(master.take_writer().is_err(), "a second writer is refused");
    out.until("ready");
    assert_eq!(master.get_size().expect("size").rows, 24);
    assert_eq!(
        master.process_group_leader(),
        child.process_id().and_then(|p| i32::try_from(p).ok())
    );

    writer.write_all(b"abc\n").expect("write");
    out.until("got:abc");
    master
        .resize(PtySize {
            rows: 10,
            cols: 40,
            pixel_width: 400,
            pixel_height: 100,
        })
        .expect("resize");
    let now = master.get_size().expect("size");
    assert_eq!(
        (now.rows, now.cols, now.pixel_width, now.pixel_height),
        (10, 40, 400, 100)
    );
    writer.write_all(b"\n").expect("write");
    out.until("size:10 40");
    // The original master sees the same size, and dropping the adopted writer sends nothing
    // (the script is still waiting for its last line).
    assert_eq!(pair.master.get_size().expect("size").cols, 40);
    drop(writer);
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        child.try_wait().expect("wait").is_none(),
        "the writer's drop sent an EOF"
    );
    child.kill().expect("kill");
    child.wait().expect("reap");
}

/// A child of this test process, for the adopted types to wait for.
fn spawn(script: &str) -> std::process::Child {
    Command::new("/bin/sh")
        .args(["-c", script])
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn")
}

fn wait_for_ready(child: &mut std::process::Child) {
    let stdout = child.stdout.take().expect("stdout");
    let mut line = String::new();
    std::io::BufReader::new(stdout)
        .read_line(&mut line)
        .expect("line");
    assert_eq!(line.trim(), "ready");
}

#[test]
#[allow(clippy::zombie_processes, reason = "the adopted child reaps them")]
fn an_adopted_child_reports_an_exit_code_and_a_signal_death() {
    let exited = spawn("exit 7");
    let mut child = AdoptedChild::new(exited.id());
    assert_eq!(child.process_id(), Some(exited.id()));
    assert_eq!(child.wait().expect("wait").exit_code(), 7);

    let signalled = spawn("kill -TERM $$");
    let mut child = AdoptedChild::new(signalled.id());
    let deadline = Instant::now() + Duration::from_secs(10);
    let code = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status.exit_code();
        }
        assert!(Instant::now() < deadline, "the child never ended");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(code, 128 + u32::try_from(libc::SIGTERM).expect("signal"));

    let running = spawn("echo ready; exec sleep 30");
    let mut running = running;
    wait_for_ready(&mut running);
    let mut child = AdoptedChild::new(running.id());
    assert!(child.try_wait().expect("try_wait").is_none());
    child.kill().expect("kill");
    let _ = child.wait();
}

#[test]
fn kill_stops_a_child_that_ignores_sighup() {
    let mut proc = spawn("trap '' HUP; echo ready; exec sleep 30");
    wait_for_ready(&mut proc);
    let mut child = AdoptedChild::new(proc.id());
    let started = Instant::now();
    child.kill().expect("kill");
    assert!(
        started.elapsed() >= Duration::from_millis(150),
        "SIGHUP got a grace period"
    );
    // The grace period passed with the child alive, so it died of `SIGKILL`.
    assert_eq!(
        child.wait().expect("wait").exit_code(),
        128 + u32::try_from(libc::SIGKILL).expect("signal")
    );
}

#[test]
fn a_cloned_killer_only_hangs_up() {
    let mut proc = spawn("trap '' HUP; echo ready; exec sleep 5");
    wait_for_ready(&mut proc);
    let child = AdoptedChild::new(proc.id());
    child.clone_killer().kill().expect("hang up");
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        super::super::signal(proc.id(), 0).is_ok(),
        "SIGHUP alone does not kill it"
    );
    proc.kill().expect("kill");
    proc.wait().expect("wait");
}
