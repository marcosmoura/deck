//! One client connection: peer check, handshake, then the message loop.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use deck_core::protocol::{
    ByeReason, CellPx, ClientMsg, FrameReader, Request, Response, Role, ServerMsg, decode_payload,
    encode, hello_version,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::task::JoinSet;

use crate::env::{Env, lookup};
use crate::queue::{Next, Queue};
use crate::server::{AttachRequest, ClientId, Shared, WriterGuard};

const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
const BEST_EFFORT_WRITE: Duration = Duration::from_secs(1);

struct Reader {
    half: OwnedReadHalf,
    frames: FrameReader,
    buf: Vec<u8>,
}

impl Reader {
    fn new(half: OwnedReadHalf) -> Self {
        Self {
            half,
            frames: FrameReader::new(),
            buf: vec![0; 64 * 1024],
        }
    }

    /// The next whole payload; `None` at the end of the stream.
    async fn payload(&mut self) -> io::Result<Option<Vec<u8>>> {
        loop {
            if let Some(p) = self
                .frames
                .next_payload()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
            {
                return Ok(Some(p));
            }
            let n = self.half.read(&mut self.buf).await?;
            if n == 0 {
                return Ok(None);
            }
            let payload = self.buf.get(..n).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "read exceeds the buffer length")
            })?;
            self.frames.push(payload);
        }
    }
}

/// Writes a `Bye` straight to the socket, for a client that never got a queue.
async fn bye_direct(half: &mut OwnedWriteHalf, reason: ByeReason) {
    let Ok(bytes) = encode(&ServerMsg::Bye { reason }) else {
        return;
    };
    let _ = tokio::time::timeout(BEST_EFFORT_WRITE, half.write_all(&bytes)).await;
    let _ = half.shutdown().await;
}

pub async fn handle(shared: Arc<Shared>, stream: UnixStream, uid: u32) {
    let pid = match stream.peer_cred() {
        Ok(cred) if cred.uid() == uid => cred.pid(),
        Ok(cred) => {
            tracing::warn!(
                peer_uid = cred.uid(),
                "rejected a connection from another user"
            );
            return;
        }
        Err(e) => {
            tracing::warn!("cannot read the peer's credentials: {e}");
            return;
        }
    };
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = Reader::new(read_half);

    let first = match tokio::time::timeout(HELLO_TIMEOUT, reader.payload()).await {
        Ok(Ok(Some(payload))) => payload,
        _ => return,
    };
    let mismatch = ByeReason::VersionMismatch {
        daemon: shared.version,
    };
    let (role, cwd, env, colors) = match decode_payload::<ClientMsg>(&first) {
        Ok(ClientMsg::Hello { version, .. }) if version != shared.version => {
            bye_direct(&mut write_half, mismatch).await;
            return;
        }
        Ok(ClientMsg::Hello {
            role,
            cwd,
            env,
            colors,
            ..
        }) => (role, cwd, env, colors),
        Ok(_) => return,
        Err(_) => {
            if hello_version(&first).is_some() {
                bye_direct(&mut write_half, mismatch).await;
            }
            return;
        }
    };

    let Some((cid, queue)) = shared.add_client(colors) else {
        bye_direct(&mut write_half, ByeReason::ServerShutdown).await;
        return;
    };
    let guard = shared.writer_guard();
    let mut writer = tokio::spawn(write_loop(
        Arc::clone(&shared),
        cid,
        Arc::clone(&queue),
        write_half,
        guard,
    ));

    let hello = Hello {
        env: env.clone(),
        pid,
    };
    if let Role::Attach {
        session,
        create,
        cols,
        rows,
        cell,
        images,
        outer_name,
    } = role
    {
        let request = AttachRequest {
            session,
            create,
            cols,
            rows,
            cell: known_cell(cell),
            images,
            outer_name: outer_name.filter(|n| valid_name(n)),
            cwd: cwd.clone(),
            env: env.clone(),
        };
        if let Err(message) = shared.attach(cid, &request).await {
            queue.push_msg(ServerMsg::Bye {
                reason: ByeReason::Error(message),
            });
        }
    }

    // A pending `wait` ends with its connection: dropping the set aborts it.
    let mut waits = JoinSet::new();
    let mut writer_done = false;
    loop {
        tokio::select! {
            biased;
            _ = &mut writer => {
                writer_done = true;
                break;
            }
            next = reader.payload() => match next {
                Ok(Some(payload)) => {
                    if !dispatch(&shared, cid, &queue, &hello, &mut waits, &payload).await {
                        break;
                    }
                }
                _ => break,
            },
        }
    }
    shared.remove_client(cid);
    if !writer_done {
        writer.abort();
    }
}

/// The outer terminal's name as the TUI sends it: printable text of a fitting length.
fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.chars().count() <= 64 && !name.chars().any(char::is_control)
}

/// A cell size with a zero side is no size: the client does not know it.
fn known_cell(cell: Option<CellPx>) -> Option<CellPx> {
    cell.filter(|c| c.width > 0 && c.height > 0)
}

/// What a connection's `Hello` said about the client's environment, and the client's pid.
struct Hello {
    env: Env,
    pid: Option<i32>,
}

impl Hello {
    /// Who sent a request, for the log: `from pid 12 (session 1, panel 3, terminal 4)`, or
    /// `(outside deck)` for a client that runs in no deck terminal. Only the ids are read
    /// from the environment.
    fn requester(&self) -> String {
        let pid = self
            .pid
            .map_or_else(|| "an unknown pid".to_string(), |p| format!("pid {p}"));
        let id = |key| lookup(&self.env, key).map(|v| v.to_string_lossy().into_owned());
        let place = match (
            id("DECK_SESSION_ID"),
            id("DECK_PANEL_ID"),
            id("DECK_TERMINAL_ID"),
        ) {
            (Some(s), Some(p), Some(t)) => format!("session {s}, panel {p}, terminal {t}"),
            _ => "outside deck".to_string(),
        };
        format!("from {pid} ({place})")
    }
}

/// Handles one message; false ends the connection.
async fn dispatch(
    shared: &Arc<Shared>,
    cid: ClientId,
    queue: &Arc<Queue>,
    hello: &Hello,
    waits: &mut JoinSet<()>,
    payload: &[u8],
) -> bool {
    match decode_payload::<ClientMsg>(payload) {
        Ok(ClientMsg::Viewport { cols, rows, cell }) => {
            shared.viewport(cid, cols, rows, known_cell(cell))
        }
        Ok(ClientMsg::ReloadViewport { cols, rows, cell }) => {
            shared.reload_viewport(cid, cols, rows, known_cell(cell));
        }
        Ok(ClientMsg::Input { terminal, bytes }) => shared.input(cid, terminal, bytes),
        Ok(ClientMsg::Scroll { terminal, to }) => shared.scroll(cid, terminal, to),
        Ok(ClientMsg::Focus(focused)) => shared.focus(cid, focused),
        Ok(ClientMsg::Detach) => return false,
        Ok(ClientMsg::Request(Request::ListSessions)) => {
            queue.push_msg(ServerMsg::Response(Response::Sessions(
                shared.list_sessions(),
            )));
        }
        Ok(ClientMsg::Request(Request::KillServer)) => {
            queue.push_msg(ServerMsg::Response(Response::Ok));
            tracing::info!("shutting down: kill-server {}", hello.requester());
            shared.request_shutdown();
        }
        Ok(ClientMsg::Request(Request::NewSession { name, cwd })) => {
            let response = match shared.new_session(name, cwd, hello.env.clone()).await {
                Ok((id, name)) => Response::Session { id, name },
                Err(e) => Response::Error(e.to_string()),
            };
            queue.push_msg(ServerMsg::Response(response));
        }
        Ok(ClientMsg::Request(Request::KillSession { name })) => {
            let answer = || queue.push_msg(ServerMsg::Response(Response::Ok));
            match shared.kill_session_named(&name, answer) {
                Ok(true) => {
                    tracing::info!(
                        "shutting down: last session {name:?} killed {}",
                        hello.requester()
                    );
                    shared.request_shutdown();
                }
                Ok(false) => {}
                Err(e) => queue.push_msg(ServerMsg::Response(Response::Error(e.to_string()))),
            }
        }
        Ok(ClientMsg::Request(Request::RenameSession { from, to })) => {
            queue.push_msg(ServerMsg::Response(
                shared
                    .rename_session_named(&from, &to)
                    .map_or_else(|e| Response::Error(e.to_string()), |()| Response::Ok),
            ));
        }
        Ok(ClientMsg::Request(
            request @ (Request::Tree { .. }
            | Request::NewProject { .. }
            | Request::NewPanel { .. }
            | Request::NewTerminal { .. }
            | Request::Focus { .. }
            | Request::Close { .. }
            | Request::RenamePanel { .. }
            | Request::AutoNamePanel { .. }),
        )) => {
            let response = shared.control(request).await;
            queue.push_msg(ServerMsg::Response(response));
        }
        Ok(ClientMsg::Request(
            request @ (Request::Send { .. } | Request::SendKeys { .. } | Request::Capture { .. }),
        )) => {
            queue.push_msg(ServerMsg::Response(shared.terminal_io(request)));
        }
        Ok(ClientMsg::Request(Request::Wait {
            terminal,
            until,
            timeout_ms,
        })) => {
            while waits.try_join_next().is_some() {}
            let (shared, queue) = (Arc::clone(shared), Arc::clone(queue));
            waits.spawn(async move {
                let response = shared.wait(terminal, until, timeout_ms).await;
                queue.push_msg(ServerMsg::Response(response));
            });
        }
        Ok(ClientMsg::Request(Request::ListAgentSessions { kind, limit })) => {
            // Answered when the listing is done, from its own task: never in order.
            shared.spawn_listing(cid, kind, limit, queue, waits);
        }
        Ok(ClientMsg::Request(Request::AgentEvent {
            terminal,
            kind,
            status,
            session_id,
            prompt,
            raw,
        })) => {
            queue.push_msg(ServerMsg::Response(
                shared.agent_event(terminal, kind, status, session_id, prompt, raw),
            ));
        }
        Ok(ClientMsg::Action(action)) => {
            let result = shared.action(cid, action).await;
            queue.push_msg(ServerMsg::ActionResult(result));
        }
        Ok(ClientMsg::Hello { .. }) => tracing::warn!("a second Hello was ignored"),
        Err(e) => {
            tracing::warn!("cannot decode a client message: {e}");
            return false;
        }
    }
    true
}

async fn write_loop(
    shared: Arc<Shared>,
    cid: ClientId,
    queue: Arc<Queue>,
    mut half: OwnedWriteHalf,
    _guard: WriterGuard,
) {
    loop {
        match queue.next().await {
            Next::Msg(msg) => {
                let bytes = match encode(&msg) {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        tracing::error!("cannot encode a message for a client: {e}");
                        continue;
                    }
                };
                if half.write_all(&bytes).await.is_err() {
                    break;
                }
            }
            Next::Resync => shared.resync(cid),
            Next::Owed => shared.send_owed(cid),
            Next::Done => break,
        }
    }
    let _ = half.shutdown().await;
    queue.close();
}
