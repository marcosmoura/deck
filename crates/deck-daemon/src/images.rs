//! Sending kitty images to the clients that can show them.
//!
//! A frame lists the placements of its terminal. Before a client gets a frame that lists an
//! image it was not sent, the image's pixels are queued ahead of the frame, once per
//! (terminal, screen, image, generation). An image that left its screen's storage is
//! announced with `ImageGone` behind the frame that stops listing it.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use deck_core::model::TerminalId;
use deck_core::protocol::{Frame, ImageScreen, ServerMsg};

use crate::emu::{Emu, ImagePixels};
use crate::queue::{ImageRef, Queue};
use crate::terminal::TermHandle;

/// What a client was sent: (terminal, screen, image, generation).
type Key = (TerminalId, ImageScreen, u32, u64);
/// An image of a terminal, whatever its generation.
type Slot = (TerminalId, ImageScreen, u32);

/// A frame, and what the emulator said at the same moment about the images the clients
/// were sent.
#[derive(Debug)]
pub struct Shot {
    pub frame: Frame,
    /// The screen that is active.
    screen: ImageScreen,
    /// The images the storage was asked about: the clients' sent ones and the listed ones.
    asked: HashSet<u32>,
    /// The generation and sent byte size of each asked-about image the active screen's
    /// storage holds.
    stored: HashMap<u32, (u64, usize)>,
}

impl Shot {
    /// Reads the storage state for `ids` on the emulator thread, next to `frame`.
    pub fn new(emu: &mut Emu, frame: Frame, ids: &[u32]) -> Self {
        let mut asked: HashSet<u32> = ids.iter().copied().collect();
        asked.extend(frame.images.iter().map(|p| p.image));
        let asked_ids: Vec<u32> = asked.iter().copied().collect();
        let (screen, stored) = emu.image_generations(&asked_ids);
        Self {
            frame,
            screen,
            asked,
            stored,
        }
    }
}

/// What the daemon remembers of one client's images.
#[derive(Debug, Default)]
pub struct Tracker {
    /// The client attached with `images`.
    pub enabled: bool,
    sent: HashSet<Key>,
    /// Terminals whose frame the client is owed after an image was held back.
    pub owed: BTreeSet<TerminalId>,
    /// Images (by generation) whose pixels could not be copied: not asked for again.
    unavailable: HashSet<Key>,
}

/// Remembered images that cannot be copied, at most.
const UNAVAILABLE_CAP: usize = 256;

impl Tracker {
    /// The ids of the images of `terminal` the client was sent.
    pub fn ids_of(&self, terminal: TerminalId) -> impl Iterator<Item = u32> + '_ {
        self.sent
            .iter()
            .filter(move |k| k.0 == terminal)
            .map(|k| k.2)
    }

    /// The terminal closed: the client drops its images with it.
    pub fn forget_terminal(&mut self, terminal: TerminalId) {
        self.sent.retain(|k| k.0 != terminal);
        self.unavailable.retain(|k| k.0 != terminal);
        self.owed.remove(&terminal);
    }
}

/// What asking for an image's pixels gave.
enum Fetched {
    Pixels(Arc<ImagePixels>),
    /// Not copied yet: [`Fetcher::copy_wanted`] does it, outside the state lock.
    Pending,
    /// Not there, or not the generation the frame lists.
    Gone,
}

/// Where pixels come from while frames are delivered: copied from the emulators on demand
/// (`live`, under whatever lock the caller holds), or only collected for a later
/// [`Fetcher::copy_wanted`].
pub struct Fetcher {
    handles: HashMap<TerminalId, Arc<TermHandle>>,
    cache: HashMap<Slot, Option<Arc<ImagePixels>>>,
    live: bool,
    wanted: Vec<Slot>,
}

impl Fetcher {
    pub fn live(handles: HashMap<TerminalId, Arc<TermHandle>>) -> Self {
        Self {
            handles,
            cache: HashMap::new(),
            live: true,
            wanted: Vec::new(),
        }
    }

    pub fn deferred(handles: HashMap<TerminalId, Arc<TermHandle>>) -> Self {
        Self {
            live: false,
            ..Self::live(handles)
        }
    }

    /// Copies the images asked for while deferred. Takes no lock of the daemon's.
    pub fn copy_wanted(&mut self) {
        for slot in std::mem::take(&mut self.wanted) {
            let copied = self.copy(slot);
            self.cache.insert(slot, copied);
        }
    }

    fn copy(&self, (terminal, screen, id): Slot) -> Option<Arc<ImagePixels>> {
        let handle = self.handles.get(&terminal)?;
        handle
            .with_emu(move |e| e.image(screen, id))
            .flatten()
            .map(Arc::new)
    }

    fn get(&mut self, key: Key) -> Fetched {
        let (terminal, screen, id, generation) = key;
        let slot = (terminal, screen, id);
        if !self.cache.contains_key(&slot) {
            if !self.live {
                if !self.wanted.contains(&slot) {
                    self.wanted.push(slot);
                }
                return Fetched::Pending;
            }
            let copied = self.copy(slot);
            self.cache.insert(slot, copied);
        }
        match self.cache.get(&slot) {
            Some(Some(px)) if px.generation == generation => Fetched::Pixels(Arc::clone(px)),
            _ => Fetched::Gone,
        }
    }
}

/// Queues the frame of `shot` for one client, with the pieces of the images it lists ahead
/// of it and `ImageGone` for what left the storage behind it. Returns whether a placement
/// was left out only because its pixels were not copied yet.
pub fn deliver(queue: &Queue, tracker: &mut Tracker, shot: &Shot, fetch: &mut Fetcher) -> bool {
    let terminal = shot.frame.terminal;
    let mut frame = shot.frame.clone();
    if !tracker.enabled {
        frame.images.clear();
        queue.push_frame(frame);
        return false;
    }
    let mut pending = false;
    // Images with a placement left out: their old generation stays until the new one is sent.
    let mut waiting: HashSet<(ImageScreen, u32)> = HashSet::new();
    frame.images.clear();
    for p in &shot.frame.images {
        let key = (terminal, p.screen, p.image, p.generation);
        if tracker.sent.contains(&key) {
            frame.images.push(*p);
            continue;
        }
        // Left out of this frame, until the image is sent.
        let mut leave_out = || {
            waiting.insert((p.screen, p.image));
        };
        if tracker.unavailable.contains(&key) {
            leave_out();
            continue;
        }
        // Held back before: the frame the client is owed retries it, without copying again.
        if tracker.owed.contains(&terminal) {
            leave_out();
            continue;
        }
        let Some(&(generation, bytes)) = shot.stored.get(&p.image) else {
            leave_out();
            continue;
        };
        if generation != p.generation {
            leave_out();
            continue;
        }
        // The budget is checked before the pixels are copied.
        if !queue.fits(bytes) {
            tracker.owed.insert(terminal);
            leave_out();
            continue;
        }
        match fetch.get(key) {
            Fetched::Pixels(pixels) => {
                let image = ImageRef {
                    terminal,
                    screen: p.screen,
                    image: p.image,
                    pixels,
                };
                if queue.push_image(image) {
                    tracker.sent.insert(key);
                    frame.images.push(*p);
                } else {
                    tracker.owed.insert(terminal);
                    leave_out();
                }
            }
            Fetched::Pending => {
                pending = true;
                leave_out();
            }
            Fetched::Gone => {
                if tracker.unavailable.len() >= UNAVAILABLE_CAP {
                    tracker.unavailable.clear();
                }
                tracker.unavailable.insert(key);
                leave_out();
            }
        }
    }
    queue.push_frame(frame);
    // Only the active screen's storage is looked at: the other screen's images are still
    // there, and come back when it does.
    let mut gone: Vec<Key> = tracker
        .sent
        .iter()
        .filter(|k| k.0 == terminal && k.1 == shot.screen)
        .filter(|k| shot.asked.contains(&k.2) && shot.stored.get(&k.2).map(|s| s.0) != Some(k.3))
        .filter(|k| !waiting.contains(&(k.1, k.2)))
        .copied()
        .collect();
    gone.sort_unstable_by_key(|k| (k.2, k.3));
    for key in gone {
        tracker.sent.remove(&key);
        queue.push_msg(ServerMsg::ImageGone {
            terminal,
            screen: key.1,
            image: key.2,
            generation: key.3,
        });
    }
    pending
}
