//! A client's bounded outgoing queue.
//!
//! Frames may be dropped when the client is slow: the queue then drops every pending frame
//! and asks for a resync (a `full` frame per terminal) once it has drained. `Bye`,
//! `Response`, `Welcome` and `Clipboard` are never dropped, and only the latest
//! `SessionView` is kept.

use std::collections::VecDeque;
use std::sync::Mutex;

use deck_core::model::SessionView;
use deck_core::protocol::{Frame, ServerMsg};
use tokio::sync::Notify;

use crate::lock;

/// Frames kept for a client before it counts as stalled.
pub const MAX_PENDING_FRAMES: usize = 64;

/// Messages that are never dropped (responses, clipboard copies) a client may leave
/// unread before it is disconnected.
pub const MAX_PENDING_MSGS: usize = 1000;

#[derive(Debug)]
enum Item {
    Msg(ServerMsg),
    Frame(Frame),
    /// Where the pending `SessionView` goes in the order of messages.
    View,
}

#[derive(Debug, Default)]
struct Inner {
    items: VecDeque<Item>,
    frames: usize,
    msgs: usize,
    view: Option<SessionView>,
    need_resync: bool,
    closed: bool,
}

/// What the writer does next.
#[derive(Debug)]
pub enum Next {
    Msg(ServerMsg),
    /// The queue drained after dropping frames: send the current view and full frames.
    Resync,
    /// Closed and empty.
    Done,
}

#[derive(Debug, Default)]
pub struct Queue {
    inner: Mutex<Inner>,
    notify: Notify,
}

impl Queue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues a message that is never dropped. A `Bye` closes the queue behind it.
    pub fn push_msg(&self, msg: ServerMsg) {
        let mut q = lock(&self.inner);
        if q.closed {
            return;
        }
        if matches!(msg, ServerMsg::Bye { .. }) {
            q.closed = true;
        } else if q.msgs >= MAX_PENDING_MSGS {
            // A client that never reads is cut off rather than allowed to grow the queue.
            q.items.clear();
            q.frames = 0;
            q.msgs = 0;
            q.view = None;
            q.closed = true;
            drop(q);
            self.notify.notify_one();
            return;
        }
        q.msgs += 1;
        q.items.push_back(Item::Msg(msg));
        drop(q);
        self.notify.notify_one();
    }

    pub fn push_view(&self, view: SessionView) {
        let mut q = lock(&self.inner);
        if q.closed {
            return;
        }
        if q.view.replace(view).is_none() {
            q.items.push_back(Item::View);
        }
        drop(q);
        self.notify.notify_one();
    }

    pub fn push_frame(&self, frame: Frame) {
        let mut q = lock(&self.inner);
        if q.closed || q.need_resync {
            return;
        }
        if q.frames >= MAX_PENDING_FRAMES {
            q.items.retain(|i| !matches!(i, Item::Frame(_)));
            q.frames = 0;
            q.need_resync = true;
        } else {
            q.items.push_back(Item::Frame(frame));
            q.frames += 1;
        }
        drop(q);
        self.notify.notify_one();
    }

    /// Stops accepting messages; what is queued is still delivered.
    pub fn close(&self) {
        lock(&self.inner).closed = true;
        self.notify.notify_one();
    }

    pub async fn next(&self) -> Next {
        loop {
            {
                let mut q = lock(&self.inner);
                match q.items.pop_front() {
                    Some(Item::Msg(m)) => {
                        q.msgs = q.msgs.saturating_sub(1);
                        return Next::Msg(m);
                    }
                    Some(Item::Frame(f)) => {
                        q.frames -= 1;
                        return Next::Msg(ServerMsg::Frame(f));
                    }
                    Some(Item::View) => {
                        if let Some(v) = q.view.take() {
                            return Next::Msg(ServerMsg::SessionView(v));
                        }
                        continue;
                    }
                    None => {}
                }
                if q.need_resync {
                    q.need_resync = false;
                    return Next::Resync;
                }
                if q.closed {
                    return Next::Done;
                }
            }
            self.notify.notified().await;
        }
    }

    #[cfg(test)]
    fn pending_frames(&self) -> usize {
        lock(&self.inner).frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deck_core::model::{SessionId, TerminalId};
    use deck_core::protocol::{ByeReason, Cursor, CursorShape, Modes};

    fn frame(n: u16) -> Frame {
        Frame {
            terminal: TerminalId(1),
            size: (n, 1),
            cursor: Cursor {
                row: 0,
                col: 0,
                visible: true,
                shape: CursorShape::Block,
            },
            modes: Modes::default(),
            display_offset: 0,
            history: 0,
            lines: vec![],
            full: false,
        }
    }

    fn view(name: &str) -> SessionView {
        SessionView {
            id: SessionId(1),
            name: name.into(),
            projects: vec![],
            active_project: None,
            active_panel: None,
            layout_viewport: None,
        }
    }

    fn bye() -> ServerMsg {
        ServerMsg::Bye {
            reason: ByeReason::ServerShutdown,
        }
    }

    #[tokio::test]
    async fn keeps_order_and_replaces_the_pending_view() {
        let q = Queue::new();
        q.push_msg(ServerMsg::Welcome {
            session: SessionId(1),
            config: Box::default(),
        });
        q.push_view(view("a"));
        q.push_frame(frame(1));
        q.push_view(view("b"));
        let Next::Msg(ServerMsg::Welcome { .. }) = q.next().await else {
            panic!("welcome first")
        };
        let Next::Msg(ServerMsg::SessionView(v)) = q.next().await else {
            panic!("view second")
        };
        assert_eq!(v.name, "b");
        let Next::Msg(ServerMsg::Frame(_)) = q.next().await else {
            panic!("frame third")
        };
    }

    #[tokio::test]
    async fn a_full_queue_drops_frames_and_resyncs_once_drained() {
        let q = Queue::new();
        q.push_view(view("v"));
        for i in 0..=MAX_PENDING_FRAMES as u16 + 5 {
            q.push_frame(frame(i));
        }
        assert_eq!(q.pending_frames(), 0);
        q.push_msg(ServerMsg::Clipboard {
            selection: "c".into(),
            text: "t".into(),
        });
        q.push_msg(bye());
        // The view, the clipboard text and the bye survive; the frames do not.
        let mut kinds = Vec::new();
        loop {
            match q.next().await {
                Next::Msg(ServerMsg::SessionView(_)) => kinds.push("view"),
                Next::Msg(ServerMsg::Clipboard { .. }) => kinds.push("clip"),
                Next::Msg(ServerMsg::Bye { .. }) => kinds.push("bye"),
                Next::Msg(other) => panic!("{other:?}"),
                Next::Resync => kinds.push("resync"),
                Next::Done => break,
            }
        }
        assert_eq!(kinds, ["view", "clip", "bye", "resync"]);
    }

    #[tokio::test]
    async fn frames_after_a_drop_wait_for_the_resync() {
        let q = Queue::new();
        for i in 0..=MAX_PENDING_FRAMES as u16 {
            q.push_frame(frame(i));
        }
        q.push_frame(frame(999));
        assert!(matches!(q.next().await, Next::Resync));
        q.push_frame(frame(1000));
        let Next::Msg(ServerMsg::Frame(f)) = q.next().await else {
            panic!("frame after resync")
        };
        assert_eq!(f.size.0, 1000);
    }

    #[tokio::test]
    async fn a_client_that_never_reads_is_cut_off() {
        let q = Queue::new();
        for _ in 0..=MAX_PENDING_MSGS {
            q.push_msg(ServerMsg::Clipboard {
                selection: "c".into(),
                text: "t".into(),
            });
        }
        assert!(matches!(q.next().await, Next::Done));
        // A reader that keeps up is never cut off.
        let q = Queue::new();
        for _ in 0..3 * MAX_PENDING_MSGS {
            q.push_msg(ServerMsg::Response(deck_core::protocol::Response::Ok));
            assert!(matches!(q.next().await, Next::Msg(_)));
        }
    }

    #[tokio::test]
    async fn bye_closes_the_queue() {
        let q = Queue::new();
        q.push_msg(bye());
        q.push_msg(ServerMsg::Welcome {
            session: SessionId(2),
            config: Box::default(),
        });
        assert!(matches!(q.next().await, Next::Msg(ServerMsg::Bye { .. })));
        assert!(matches!(q.next().await, Next::Done));
    }
}
