//! A client's bounded outgoing queue.
//!
//! Frames may be dropped when the client is slow: the queue then drops every pending frame
//! and asks for a resync (a `full` frame per terminal) once it has drained. `Bye`,
//! `Response`, `Welcome`, `Clipboard`, `ImageGone` and the pieces of images are never
//! dropped, and only the latest `SessionView` is kept. Image pieces are bounded by bytes
//! instead of by count: an image that does not fit waits (the writer is told when the pieces
//! drain, with [`Next::Owed`]).

use std::collections::VecDeque;
use std::sync::Mutex;

use deck_core::model::SessionView;
use std::sync::Arc;

use deck_core::model::TerminalId;
use deck_core::protocol::{Frame, ImageScreen, MAX_IMAGE_PIECE, ServerMsg};
use tokio::sync::Notify;

use crate::emu::ImagePixels;
use crate::lock;

/// Frames kept for a client before it counts as stalled.
pub const MAX_PENDING_FRAMES: usize = 64;

/// Messages that are never dropped (responses, clipboard copies) a client may leave
/// unread before it is disconnected.
pub const MAX_PENDING_MSGS: usize = 1000;

/// Image bytes kept for a client before a further image waits (one image is always let
/// through when none is pending).
pub const MAX_PENDING_IMAGE_BYTES: usize = 64 << 20;

#[derive(Debug)]
enum Item {
    Msg(ServerMsg),
    /// A piece of an image: never dropped, counted in bytes.
    Piece(Box<Piece>),
    Frame(Frame),
    /// Where the pending `SessionView` goes in the order of messages.
    View,
}

/// An image of a terminal, to be sent in pieces.
#[derive(Debug, Clone)]
pub struct ImageRef {
    pub terminal: TerminalId,
    pub screen: ImageScreen,
    pub image: u32,
    pub pixels: Arc<ImagePixels>,
}

/// The bytes `offset..offset + len` of an image, copied into a message when the writer takes
/// it, so one copy of the pixels serves every client.
#[derive(Debug)]
struct Piece {
    image: ImageRef,
    offset: usize,
    len: usize,
}

impl Piece {
    fn message(&self) -> ServerMsg {
        let px = &self.image.pixels;
        ServerMsg::Image {
            terminal: self.image.terminal,
            screen: self.image.screen,
            image: self.image.image,
            generation: px.generation,
            width: px.width,
            height: px.height,
            format: px.format,
            offset: u32::try_from(self.offset).unwrap_or(u32::MAX),
            total: u32::try_from(px.data.len()).unwrap_or(u32::MAX),
            data: px
                .data
                .get(self.offset..self.offset + self.len)
                .unwrap_or_default()
                .to_vec(),
        }
    }
}

#[derive(Debug, Default)]
struct Inner {
    items: VecDeque<Item>,
    frames: usize,
    msgs: usize,
    /// Pixel bytes in the pending image pieces.
    pieces: usize,
    /// An image was held back: tell the writer when the pieces have drained.
    owed: bool,
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
    /// The pieces drained after an image was held back: the client is owed frames again.
    Owed,
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
            q.pieces = 0;
            q.owed = false;
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

    /// Whether an image of `bytes` would be queued now. When it would not, the writer is told
    /// when the pieces drain, as after a failed [`Queue::push_image`].
    pub fn fits(&self, bytes: usize) -> bool {
        let mut q = lock(&self.inner);
        if q.closed {
            return false;
        }
        if q.pieces > 0 && q.pieces.saturating_add(bytes) > MAX_PENDING_IMAGE_BYTES {
            q.owed = true;
            return false;
        }
        true
    }

    /// Queues one image as pieces of at most 4 MiB, in order, when it fits the byte budget
    /// (or nothing is pending). Otherwise queues nothing, and the writer is told when the
    /// pieces drain.
    pub fn push_image(&self, image: ImageRef) -> bool {
        let size = image.pixels.data.len();
        let mut q = lock(&self.inner);
        if q.closed {
            return false;
        }
        if q.pieces > 0 && q.pieces.saturating_add(size) > MAX_PENDING_IMAGE_BYTES {
            q.owed = true;
            return false;
        }
        q.pieces += size;
        let mut offset = 0;
        while offset < size {
            let len = MAX_IMAGE_PIECE.min(size - offset);
            q.items.push_back(Item::Piece(Box::new(Piece {
                image: image.clone(),
                offset,
                len,
            })));
            offset += len;
        }
        drop(q);
        self.notify.notify_one();
        true
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
                if q.owed && q.pieces == 0 {
                    q.owed = false;
                    return Next::Owed;
                }
                match q.items.pop_front() {
                    Some(Item::Piece(piece)) => {
                        q.pieces = q.pieces.saturating_sub(piece.len);
                        drop(q);
                        return Next::Msg(piece.message());
                    }
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
    fn pending_pieces(&self) -> usize {
        lock(&self.inner).pieces
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
    use deck_core::protocol::{ByeReason, Cursor, CursorShape, ImageFormat, ImageScreen, Modes};

    /// Image `id` of `bytes` bytes.
    fn img(id: u32, bytes: usize) -> ImageRef {
        ImageRef {
            terminal: TerminalId(1),
            screen: ImageScreen::Primary,
            image: id,
            pixels: Arc::new(ImagePixels {
                generation: 1,
                width: 1,
                height: 1,
                format: ImageFormat::Rgba,
                data: vec![0; bytes],
            }),
        }
    }

    fn frame(n: u16) -> Frame {
        Frame {
            terminal: TerminalId(1),
            size: (n, 1),
            cursor: Cursor {
                row: 0,
                col: 0,
                visible: true,
                shape: CursorShape::Block,
                blink: false,
            },
            modes: Modes::default(),
            display_offset: 0,
            history: 0,
            top_row: 0,
            numbering: 0,
            lines: vec![],
            full: false,
            images: vec![],
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
            attached_clients: 0,
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
                Next::Owed => panic!("no image was held back"),
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

    const MIB: usize = 1 << 20;

    #[tokio::test]
    async fn images_wait_when_the_byte_budget_is_full() {
        let q = Queue::new();
        assert!(q.push_image(img(1, 40 * MIB)));
        // 40 + 40 passes 64: held back, nothing queued.
        assert!(!q.push_image(img(2, 40 * MIB)));
        assert_eq!(q.pending_pieces(), 40 * MIB);
        // 40 + 24 is exactly the budget.
        assert!(q.push_image(img(3, 24 * MIB)));
        assert_eq!(q.pending_pieces(), MAX_PENDING_IMAGE_BYTES);
        assert!(!q.push_image(img(4, 1)));
        assert!(!q.fits(1));
    }

    #[tokio::test]
    async fn one_image_always_gets_through_when_nothing_is_pending() {
        let q = Queue::new();
        assert!(q.push_image(img(1, 70 * MIB)));
        assert!(!q.push_image(img(2, 1)));
    }

    #[tokio::test]
    async fn the_drain_hook_fires_once_the_pieces_are_sent() {
        let q = Queue::new();
        assert!(q.push_image(img(1, 48 * MIB + 1)));
        assert!(!q.push_image(img(2, 48 * MIB)));
        for n in 0..13 {
            let Next::Msg(ServerMsg::Image { offset, data, .. }) = q.next().await else {
                panic!("piece {n}")
            };
            assert_eq!(offset as usize, n * MAX_IMAGE_PIECE);
            assert_eq!(data.len(), if n < 12 { MAX_IMAGE_PIECE } else { 1 });
        }
        assert_eq!(q.pending_pieces(), 0);
        assert!(matches!(q.next().await, Next::Owed));
        // Once: the next wait is for new work.
        q.push_msg(bye());
        assert!(matches!(q.next().await, Next::Msg(ServerMsg::Bye { .. })));
        assert!(matches!(q.next().await, Next::Done));
    }

    #[tokio::test]
    async fn no_hook_without_a_held_back_image() {
        let q = Queue::new();
        assert!(q.push_image(img(1, 10)));
        q.push_msg(bye());
        assert!(matches!(q.next().await, Next::Msg(ServerMsg::Image { .. })));
        assert!(matches!(q.next().await, Next::Msg(ServerMsg::Bye { .. })));
        assert!(matches!(q.next().await, Next::Done));
    }

    #[tokio::test]
    async fn pieces_go_out_before_the_frame_queued_behind_them() {
        let q = Queue::new();
        q.push_frame(frame(1));
        assert!(q.push_image(img(1, 5)) && q.push_image(img(2, 6)));
        q.push_frame(frame(2));
        let mut kinds = Vec::new();
        for _ in 0..4 {
            match q.next().await {
                Next::Msg(ServerMsg::Image { data, .. }) => {
                    kinds.push(format!("piece{}", data.len()))
                }
                Next::Msg(ServerMsg::Frame(f)) => kinds.push(format!("frame{}", f.size.0)),
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(kinds, ["frame1", "piece5", "piece6", "frame2"]);
    }

    #[tokio::test]
    async fn dropped_frames_keep_the_pieces_and_resync_follows_them() {
        let q = Queue::new();
        assert!(q.push_image(img(1, 7)));
        for i in 0..=MAX_PENDING_FRAMES as u16 {
            q.push_frame(frame(i));
        }
        assert_eq!(q.pending_frames(), 0);
        // The pieces are sent while the resync waits, and still count until sent.
        assert_eq!(q.pending_pieces(), 7);
        assert!(matches!(q.next().await, Next::Msg(ServerMsg::Image { .. })));
        assert!(matches!(q.next().await, Next::Resync));
    }

    #[tokio::test]
    async fn image_gone_is_never_dropped() {
        let q = Queue::new();
        for i in 0..=MAX_PENDING_FRAMES as u16 {
            q.push_frame(frame(i));
        }
        q.push_msg(ServerMsg::ImageGone {
            terminal: TerminalId(1),
            screen: ImageScreen::Primary,
            image: 1,
            generation: 1,
        });
        assert!(matches!(
            q.next().await,
            Next::Msg(ServerMsg::ImageGone { .. })
        ));
    }
}
