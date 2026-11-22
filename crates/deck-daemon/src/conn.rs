//! One client connection: peer check, handshake, then the message loop.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use deck_core::protocol::{
    ByeReason, ClientMsg, FrameReader, Request, Response, Role, ServerMsg, decode_payload, encode,
    hello_version,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

use crate::env::Env;
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
            self.frames.push(&self.buf[..n]);
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
    match stream.peer_cred() {
        Ok(cred) if cred.uid() == uid => {}
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
    }
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

    let hello = Hello { env: env.clone() };
    if let Role::Attach {
        session,
        create,
        cols,
        rows,
    } = role
    {
        let request = AttachRequest {
            session,
            create,
            cols,
            rows,
            cwd: cwd.clone(),
            env: env.clone(),
        };
        if let Err(message) = shared.attach(cid, &request).await {
            queue.push_msg(ServerMsg::Bye {
                reason: ByeReason::Error(message),
            });
        }
    }

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
                    if !dispatch(&shared, cid, &queue, &hello, &payload).await {
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

/// What a connection's `Hello` said about the client's environment.
struct Hello {
    env: Env,
}

/// Handles one message; false ends the connection.
async fn dispatch(
    shared: &Arc<Shared>,
    cid: ClientId,
    queue: &Queue,
    hello: &Hello,
    payload: &[u8],
) -> bool {
    match decode_payload::<ClientMsg>(payload) {
        Ok(ClientMsg::Viewport { cols, rows }) => shared.viewport(cid, cols, rows),
        Ok(ClientMsg::Input { terminal, bytes }) => shared.input(cid, terminal, bytes),
        Ok(ClientMsg::Scroll { terminal, to }) => shared.scroll(cid, terminal, to),
        Ok(ClientMsg::Detach) => return false,
        Ok(ClientMsg::Request(Request::ListSessions)) => {
            queue.push_msg(ServerMsg::Response(Response::Sessions(
                shared.list_sessions(),
            )));
        }
        Ok(ClientMsg::Request(Request::KillServer)) => {
            queue.push_msg(ServerMsg::Response(Response::Ok));
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
                Ok(true) => shared.request_shutdown(),
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
            Next::Done => break,
        }
    }
    let _ = half.shutdown().await;
    queue.close();
}
