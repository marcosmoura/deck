//! Image pixels and placements reaching clients: order, pieces, forgetting, pacing.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use std::path::PathBuf;
use std::time::Duration;

use deck_core::action::Action;
use deck_core::model::TerminalId;
use deck_core::protocol::{
    CellPx, ClientMsg, Frame, ImageScreen, MAX_IMAGE_PIECE, Placement, PlacementKind, ServerMsg,
};
use support::{Attach, Client, TestDaemon};

const CELL: CellPx = CellPx {
    width: 9,
    height: 18,
};

/// Sends a kitty image with the given keys: `n` bytes of a ramp or of zeros, optionally
/// zlib-compressed (a big image of zeros is then a few kilobytes on the wire).
const SCRIPT: &str = r"import sys, zlib, base64
keys, n, mode, comp = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4] == '1'
d = bytes(n) if mode == 'zero' else bytes((j * 7 + int(mode)) % 256 for j in range(n))
if comp:
    d = zlib.compress(d)
    keys += ',o=z'
e = base64.b64encode(d).decode()
cs = [e[i:i + 4096] for i in range(0, len(e), 4096)] or ['']
for i, c in enumerate(cs):
    k = (keys + ',' if i == 0 else '') + 'm=' + str(int(i < len(cs) - 1))
    sys.stdout.write('\x1b_G' + k + ';' + c + '\x1b\\')
sys.stdout.flush()
";

struct Fixture {
    daemon: TestDaemon,
    script: PathBuf,
}

impl Fixture {
    async fn new() -> Self {
        let daemon = TestDaemon::start().await;
        let script = daemon.home.path().join("img.py");
        std::fs::write(&script, SCRIPT).unwrap();
        Self { daemon, script }
    }

    async fn attach(&self, images: bool) -> Client {
        Client::attach(
            &self.daemon.paths.socket,
            Attach {
                cell: Some(CELL),
                images,
                ..Attach::default()
            },
        )
        .await
    }

    /// The shell command that sends an image.
    fn send(&self, keys: &str, n: usize, mode: &str, compress: bool) -> String {
        format!(
            "python3 {} '{keys},q=2' {n} {mode} {}",
            self.script.display(),
            u8::from(compress)
        )
    }
}

async fn run(c: &mut Client, terminal: TerminalId, line: &str, tag: &str) {
    // The tag is computed by the shell, so the echoed command line does not contain it.
    c.send(&ClientMsg::Input {
        terminal,
        bytes: format!("\x15{line}; echo {tag}-$((1+1))\n").into_bytes(),
    })
    .await;
    let done = format!("{tag}-2");
    c.wait(|c| c.has_text(&done)).await;
}

/// Reads until `done` holds, with a long limit for the big images.
async fn wait_long(c: &mut Client, mut done: impl FnMut(&Client) -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while !done(c) {
        let left = deadline
            .checked_duration_since(std::time::Instant::now())
            .expect("in time");
        assert!(
            c.recv(left).await.is_some(),
            "nothing more came; got {:?}",
            c.log
                .iter()
                .map(|m| match m {
                    ServerMsg::Image { image, offset, .. } => format!("P{image}@{offset}"),
                    ServerMsg::Frame(f) => format!("F{}", f.images.len()),
                    ServerMsg::ImageGone { image, .. } => format!("G{image}"),
                    _ => "-".into(),
                })
                .collect::<Vec<_>>()
        );
    }
}

/// What the client got about images, in order: `P(id, gen, offset)`, `F(placed images)`,
/// `G(id, gen)`.
#[derive(Debug, PartialEq, Eq)]
enum Ev {
    Piece(u32, u64, u32),
    Frame(Vec<(u32, u64)>),
    Gone(u32, u64),
}

fn events(c: &Client, terminal: TerminalId) -> Vec<Ev> {
    c.log
        .iter()
        .filter_map(|m| match m {
            ServerMsg::Image {
                terminal: t,
                image,
                generation,
                offset,
                ..
            } if *t == terminal => Some(Ev::Piece(*image, *generation, *offset)),
            ServerMsg::Frame(f) if f.terminal == terminal => Some(Ev::Frame(
                f.images.iter().map(|p| (p.image, p.generation)).collect(),
            )),
            ServerMsg::ImageGone {
                terminal: t,
                image,
                generation,
                ..
            } if *t == terminal => Some(Ev::Gone(*image, *generation)),
            _ => None,
        })
        .collect()
}

fn last_frame(c: &Client, terminal: TerminalId) -> &Frame {
    c.frames()
        .filter(|f| f.terminal == terminal)
        .last()
        .expect("a frame")
}

fn placement(c: &Client, image: u32) -> Placement {
    c.frames()
        .flat_map(|f| &f.images)
        .find(|p| p.image == image)
        .copied()
        .expect("a placement")
}

fn pieces_of(c: &Client, image: u32) -> Vec<(u32, u32, usize)> {
    c.log
        .iter()
        .filter_map(|m| match m {
            ServerMsg::Image {
                image: i,
                offset,
                total,
                data,
                ..
            } if *i == image => Some((*offset, *total, data.len())),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn pieces_come_before_the_frame_that_lists_the_image_and_gone_follows_a_delete() {
    let fx = Fixture::new().await;
    let mut c = fx.attach(true).await;
    let t = c.terminal();
    let line = fx.send("a=T,f=32,s=20,v=10,i=5,c=2,r=1,C=1", 800, "1", false);
    run(&mut c, t, &line, "sent").await;
    c.wait(|c| !last_frame(c, t).images.is_empty()).await;

    let ev = events(&c, t);
    let first_piece = ev
        .iter()
        .position(|e| matches!(e, Ev::Piece(5, _, 0)))
        .expect("a piece");
    let first_listing = ev
        .iter()
        .position(|e| matches!(e, Ev::Frame(l) if !l.is_empty()))
        .expect("a frame listing the image");
    assert!(first_piece < first_listing, "{ev:?}");
    assert_eq!(pieces_of(&c, 5), [(0, 800, 800)]);
    let image = c.log.iter().find_map(|m| match m {
        ServerMsg::Image {
            width,
            height,
            format,
            data,
            ..
        } => Some((*width, *height, *format, data.clone())),
        _ => None,
    });
    let (w, h, format, data) = image.unwrap();
    assert_eq!((w, h), (20, 10));
    assert_eq!(format, deck_core::protocol::ImageFormat::Rgba);
    assert_eq!(data[..4], [1, 8, 15, 22]);
    let p = placement(&c, 5);
    assert_eq!((p.cols, p.rows), (2, 1));
    assert!(
        matches!(p.kind, PlacementKind::Placed { col: 0, .. }),
        "{p:?}"
    );
    assert_eq!(p.screen, ImageScreen::Primary);

    // The delete: the frame without the placement, then the image is gone.
    run(&mut c, t, r"printf '\033_Ga=d,d=A,q=2\033\\'", "del").await;
    c.wait(|c| events(c, t).iter().any(|e| matches!(e, Ev::Gone(5, _))))
        .await;
    let ev = events(&c, t);
    let gone = ev.iter().position(|e| matches!(e, Ev::Gone(5, _))).unwrap();
    let unlisted = ev
        .iter()
        .rposition(|e| matches!(e, Ev::Frame(l) if l.is_empty()))
        .unwrap();
    assert!(unlisted < gone, "{ev:?}");
    assert_eq!(ev.iter().filter(|e| matches!(e, Ev::Piece(..))).count(), 1);
    c.send(&ClientMsg::Detach).await;
}

#[tokio::test]
async fn an_image_over_4_mib_arrives_in_pieces_that_cover_it() {
    let fx = Fixture::new().await;
    let mut c = fx.attach(true).await;
    let t = c.terminal();
    // 1000 x 1500 RGB: 4,500,000 bytes.
    let line = fx.send(
        "a=T,f=24,s=1000,v=1500,i=7,c=2,r=1,C=1",
        4_500_000,
        "zero",
        true,
    );
    run(&mut c, t, &line, "big").await;
    wait_long(&mut c, |c| !last_frame(c, t).images.is_empty()).await;
    let pieces = pieces_of(&c, 7);
    assert_eq!(pieces.len(), 2, "{pieces:?}");
    let mut next = 0u32;
    for (offset, total, len) in &pieces {
        assert_eq!(*offset, next);
        assert_eq!(*total, 4_500_000);
        assert!(*len <= MAX_IMAGE_PIECE);
        next += u32::try_from(*len).unwrap();
    }
    assert_eq!(next, 4_500_000);
}

#[tokio::test]
async fn retransmitting_an_id_sends_the_new_generation_before_the_old_one_is_gone() {
    let fx = Fixture::new().await;
    let mut c = fx.attach(true).await;
    let t = c.terminal();
    let keys = "a=T,f=32,s=20,v=10,i=5,c=2,r=1,C=1";
    run(&mut c, t, &fx.send(keys, 800, "1", false), "one").await;
    c.wait(|c| !last_frame(c, t).images.is_empty()).await;
    let first = last_frame(&c, t).images[0].generation;
    run(&mut c, t, &fx.send(keys, 800, "2", false), "two").await;
    c.wait(|c| {
        events(c, t)
            .iter()
            .any(|e| matches!(e, Ev::Gone(5, g) if *g == first))
    })
    .await;
    let ev = events(&c, t);
    let gone = ev
        .iter()
        .position(|e| matches!(e, Ev::Gone(5, g) if *g == first))
        .unwrap();
    let second_piece = ev
        .iter()
        .position(|e| matches!(e, Ev::Piece(5, g, 0) if *g != first))
        .expect("pieces of the new generation");
    let listing = ev
        .iter()
        .position(|e| matches!(e, Ev::Frame(l) if l.iter().any(|(i, g)| *i == 5 && *g != first)))
        .expect("a frame listing the new generation");
    assert!(second_piece < listing && listing < gone, "{ev:?}");
}

#[tokio::test]
async fn the_alternate_screen_neither_forgets_nor_resends_primary_images() {
    let fx = Fixture::new().await;
    let mut c = fx.attach(true).await;
    let t = c.terminal();
    let keys = "a=T,f=32,s=20,v=10,i=5,c=2,r=1,C=1";
    run(&mut c, t, &fx.send(keys, 800, "1", false), "pri").await;
    c.wait(|c| !last_frame(c, t).images.is_empty()).await;
    // Into the alternate screen, with an image of its own.
    run(&mut c, t, r"printf '\033[?1049h'", "enter").await;
    let alt = fx.send("a=T,f=32,s=20,v=10,i=6,c=2,r=1,C=1", 800, "3", false);
    run(&mut c, t, &alt, "alt").await;
    c.wait(|c| last_frame(c, t).images.iter().any(|p| p.image == 6))
        .await;
    assert_eq!(placement(&c, 6).screen, ImageScreen::Alternate);
    assert!(
        last_frame(&c, t)
            .images
            .iter()
            .all(|p| p.screen == ImageScreen::Alternate)
    );
    run(&mut c, t, r"printf '\033[?1049l'", "leave").await;
    c.wait(|c| last_frame(c, t).images.iter().any(|p| p.image == 5))
        .await;
    let ev = events(&c, t);
    assert!(!ev.iter().any(|e| matches!(e, Ev::Gone(5, _))), "{ev:?}");
    assert_eq!(pieces_of(&c, 5).len(), 1, "the primary image is sent once");
    assert_eq!(pieces_of(&c, 6).len(), 1);
}

#[tokio::test]
async fn a_client_without_images_gets_no_image_messages() {
    let fx = Fixture::new().await;
    let mut c = fx.attach(false).await;
    let t = c.terminal();
    let keys = "a=T,f=32,s=20,v=10,i=5,c=2,r=1,C=1";
    run(&mut c, t, &fx.send(keys, 800, "1", false), "off").await;
    run(&mut c, t, r"printf '\033_Ga=d,d=A,q=2\033\\'", "offdel").await;
    assert!(c.frames().all(|f| f.images.is_empty()));
    assert!(
        !c.log
            .iter()
            .any(|m| matches!(m, ServerMsg::Image { .. } | ServerMsg::ImageGone { .. }))
    );
}

/// 4096 x 4096 RGB: 48 MiB.
const BIG: usize = 48 << 20;

fn complete(c: &Client, image: u32) -> bool {
    let pieces = pieces_of(c, image);
    pieces.last().is_some_and(|(offset, total, len)| {
        usize::try_from(*offset).unwrap() + len == usize::try_from(*total).unwrap()
    })
}

#[tokio::test]
async fn images_held_back_for_a_slow_client_arrive_when_it_reads_even_when_idle() {
    let fx = Fixture::new().await;
    let mut c = fx.attach(true).await;
    let ids = two_terminals(&mut c).await;
    // The client does not read while both programs send: the second image does not fit the
    // pending budget next to the first.
    for (n, t) in ids.iter().enumerate() {
        let line = fx.send(
            &format!("a=T,f=24,s=4096,v=4096,i={},c=2,r=1,C=1", n + 1),
            BIG,
            "zero",
            true,
        );
        c.send(&ClientMsg::Input {
            terminal: *t,
            bytes: format!("\x15{line}\n").into_bytes(),
        })
        .await;
    }
    tokio::time::sleep(Duration::from_secs(4)).await;
    wait_long(&mut c, |c| {
        ids.iter().enumerate().all(|(n, t)| {
            complete(c, u32::try_from(n + 1).unwrap()) && last_frame(c, *t).images.len() == 1
        })
    })
    .await;
    for (n, t) in ids.iter().enumerate() {
        let id = u32::try_from(n + 1).unwrap();
        let ev = events(&c, *t);
        let listing = ev
            .iter()
            .position(|e| matches!(e, Ev::Frame(l) if l.iter().any(|(i, _)| *i == id)))
            .unwrap();
        let last_piece = ev
            .iter()
            .rposition(|e| matches!(e, Ev::Piece(i, ..) if *i == id))
            .unwrap();
        assert!(last_piece < listing, "{ev:?}");
    }
    // Whichever terminal's pump ran first got its image queued first; the other was held
    // back: its pieces start only after the first image's last piece, and a frame of its
    // terminal that left it out came before them.
    let start = |id: u32| {
        c.log
            .iter()
            .position(|m| matches!(m, ServerMsg::Image { image, .. } if *image == id))
            .unwrap()
    };
    let (early, late) = if start(1) < start(2) { (1, 2) } else { (2, 1) };
    let early_done = c
        .log
        .iter()
        .rposition(|m| matches!(m, ServerMsg::Image { image, .. } if *image == early))
        .unwrap();
    let late_start = start(late);
    assert!(early_done < late_start);
    let late_terminal = ids[usize::try_from(late - 1).unwrap()];
    assert!(
        c.log[early_done..late_start].iter().any(
            |m| matches!(m, ServerMsg::Frame(f) if f.terminal == late_terminal && f.images.is_empty())
        ),
        "a frame without the held-back image comes between"
    );
}

/// A panel with two terminals.
async fn two_terminals(c: &mut Client) -> Vec<TerminalId> {
    c.action(Action::NewTerminal { panel: None }).await.unwrap();
    c.wait(|c| {
        c.view.as_ref().unwrap().projects[0].panels[0]
            .terminals
            .len()
            == 2
    })
    .await;
    c.view.as_ref().unwrap().projects[0].panels[0]
        .terminals
        .iter()
        .map(|t| t.id)
        .collect()
}

#[tokio::test]
async fn attaching_to_two_terminals_with_big_images_delivers_both() {
    let fx = Fixture::new().await;
    let mut first = fx.attach(false).await;
    first
        .action(Action::NewTerminal { panel: None })
        .await
        .unwrap();
    first
        .wait(|c| {
            let v = c.view.as_ref().unwrap();
            v.projects[0].panels[0].terminals.len() == 2
        })
        .await;
    let ids: Vec<TerminalId> = first.view.as_ref().unwrap().projects[0].panels[0]
        .terminals
        .iter()
        .map(|t| t.id)
        .collect();
    for (n, t) in ids.iter().enumerate() {
        let line = fx.send(
            &format!("a=T,f=24,s=4096,v=4096,i={},c=2,r=1,C=1", n + 1),
            BIG,
            "zero",
            true,
        );
        run(&mut first, *t, &line, &format!("t{n}")).await;
    }
    // A second client attaches to the session, and gets everything.
    let session = first.view.as_ref().unwrap().name.clone();
    let mut second = Client::attach(
        &fx.daemon.paths.socket,
        Attach {
            session: Some(session),
            create: false,
            cell: Some(CELL),
            images: true,
            outer_name: None,
            ..Attach::default()
        },
    )
    .await;
    wait_long(&mut second, |c| {
        ids.iter().enumerate().all(|(n, t)| {
            complete(c, u32::try_from(n + 1).unwrap())
                && c.frames()
                    .filter(|f| f.terminal == *t)
                    .any(|f| !f.images.is_empty())
        })
    })
    .await;
    // Each terminal's frame lists its own image, after its pieces.
    for (n, t) in ids.iter().enumerate() {
        let ev = events(&second, *t);
        let id = u32::try_from(n + 1).unwrap();
        let listing = ev
            .iter()
            .position(|e| matches!(e, Ev::Frame(l) if l.iter().any(|(i, _)| *i == id)))
            .unwrap();
        assert!(
            ev[..listing]
                .iter()
                .any(|e| matches!(e, Ev::Piece(i, ..) if *i == id))
        );
    }
}
