use std::path::Path;

use super::*;

fn command(keys: &str, path: &Path) -> Vec<u8> {
    format!("\x1b_G{keys};{}\x1b\\", encode(path.as_os_str().as_bytes())).into_bytes()
}

fn scan_with(scanner: &mut Scanner, input: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    scanner.scan(input, |piece| {
        assert!(piece.len() <= PIECE, "a piece of {} bytes", piece.len());
        out.extend_from_slice(piece);
    });
    out
}

fn scan_whole(input: &[u8]) -> Vec<u8> {
    scan_with(&mut Scanner::default(), input)
}

/// The data of a rewritten command: its control strings and the decoded payloads joined.
fn parts(out: &[u8]) -> (Vec<String>, Vec<u8>) {
    let text = std::str::from_utf8(out).expect("utf-8");
    let mut controls = Vec::new();
    let mut data = Vec::new();
    for apc in text.split("\x1b\\").filter(|a| !a.is_empty()) {
        let body = apc.strip_prefix("\x1b_G").expect("an APC");
        let (control, payload) = body.split_once(';').expect("a payload");
        controls.push(control.to_string());
        data.extend(decode(payload.as_bytes()).expect("base64"));
    }
    (controls, data)
}

fn file(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("write");
    path
}

#[test]
fn a_file_command_becomes_direct_transmission() {
    let dir = tempfile::tempdir().expect("dir");
    let path = file(dir.path(), "a.png", b"PNGDATA");
    let out = scan_whole(&command("a=T,U=1,q=1,f=100,t=f,i=5,c=2,r=1", &path));
    let (controls, data) = parts(&out);
    assert_eq!(controls, ["a=T,U=1,q=1,f=100,i=5,c=2,r=1,t=d"]);
    assert_eq!(data, b"PNGDATA");
}

#[test]
fn other_bytes_pass_in_order_around_a_command_split_across_chunks() {
    let dir = tempfile::tempdir().expect("dir");
    let path = file(dir.path(), "a.bin", b"0123");
    let mut input = b"before\x1b[31m".to_vec();
    input.extend(command("a=T,f=32,t=f,s=1,v=1", &path));
    input.extend(b"after\x1b_Xnot kitty\x1b\\\x1b]0;t\x07");
    let whole = scan_whole(&input);
    for size in [1, 2, 3, 7] {
        let mut scanner = Scanner::default();
        let got: Vec<u8> = input
            .chunks(size)
            .flat_map(|c| scan_with(&mut scanner, c))
            .collect();
        assert_eq!(got, whole, "chunks of {size}");
    }
    let text = String::from_utf8(whole).expect("utf-8");
    assert!(text.starts_with("before\x1b[31m\x1b_Ga=T,f=32,s=1,v=1,t=d;MDEyMw==\x1b\\after"));
    assert!(text.ends_with("after\x1b_Xnot kitty\x1b\\\x1b]0;t\x07"));
}

#[test]
fn size_and_offset_pick_the_bytes() {
    let dir = tempfile::tempdir().expect("dir");
    let path = file(dir.path(), "a.bin", b"0123456789");
    let (controls, data) = parts(&scan_whole(&command("a=t,f=24,t=f,S=4,O=3", &path)));
    assert_eq!(data, b"3456");
    assert_eq!(controls, ["a=t,f=24,t=d"]);
    // An offset past the end, or a size past the end.
    let past = scan_whole(&command("a=t,t=f,O=11", &path));
    assert!(past.starts_with(b"\x1b_Ga=t,t=f,O=11;"));
    assert_eq!(
        parts(&scan_whole(&command("a=t,t=f,S=99,O=8", &path))).1,
        b"89"
    );
}

#[test]
fn a_long_file_goes_in_chunks_that_keep_only_m_and_q() {
    let dir = tempfile::tempdir().expect("dir");
    let bytes: Vec<u8> = (0..10_000u32).map(|i| i as u8).collect();
    let path = file(dir.path(), "big.bin", &bytes);
    let (controls, data) = parts(&scan_whole(&command("a=t,f=32,s=50,v=50,q=2,t=f", &path)));
    assert_eq!(
        controls,
        [
            "a=t,f=32,s=50,v=50,q=2,t=d,m=1",
            "q=2,m=1",
            "q=2,m=1",
            "q=2,m=0"
        ]
    );
    assert_eq!(data, bytes);
}

#[test]
fn a_fifo_a_missing_file_a_directory_and_a_relative_path_pass_unchanged() {
    let dir = tempfile::tempdir().expect("dir");
    let fifo = dir.path().join("pipe");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo")
            .success()
    );
    let (done, finished) = std::sync::mpsc::channel();
    let dir_path = dir.path().to_path_buf();
    std::os::unix::fs::symlink("/dev/null", dir_path.join("null-link")).expect("symlink");
    std::thread::spawn(move || {
        let cases = [
            command("a=T,t=f", &fifo),
            command("a=T,t=f", &dir_path.join("missing")),
            command("a=T,t=f", &dir_path),
            command("a=T,t=f", Path::new("relative.png")),
            command("a=T,t=f", Path::new("/dev/null")),
            command("a=T,t=f", Path::new("/proc/self/status")),
            // `..` and a symlink do not get a device past the check.
            command("a=T,t=f", &dir_path.join("../../../../../../dev/null")),
            command("a=T,t=f", &dir_path.join("null-link")),
        ];
        let same = cases.iter().all(|c| scan_whole(c) == *c);
        let _ = done.send(same);
    });
    let same = finished
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the scan returned");
    assert!(same);
}

#[test]
fn a_file_over_the_limit_passes_unchanged() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("huge");
    std::fs::File::create(&path)
        .and_then(|f| f.set_len(MAX_FILE + 1))
        .expect("sparse file");
    let cmd = command("a=T,t=f", &path);
    assert_eq!(scan_whole(&cmd), cmd);
    // Part of it, within the limit, is read.
    let part = scan_whole(&command("a=t,t=f,S=3", &path));
    assert_eq!(parts(&part).1, [0, 0, 0]);
}

#[test]
fn other_media_chunks_and_malformed_commands_pass_unchanged() {
    let dir = tempfile::tempdir().expect("dir");
    let path = file(dir.path(), "a.bin", b"x");
    for keys in ["a=T,t=d", "a=T,t=s", "a=T,t=f,m=1", "a=T,t=f,S=x", ""] {
        let cmd = command(keys, &path);
        assert_eq!(scan_whole(&cmd), cmd, "{keys}");
    }
    let bad = b"\x1b_Ga=T,t=f;!!!\x1b\\".to_vec();
    assert_eq!(scan_whole(&bad), bad);
    let no_payload = b"\x1b_Ga=d\x1b\\".to_vec();
    assert_eq!(scan_whole(&no_payload), no_payload);
}

#[test]
fn an_apc_past_the_limit_is_flushed_unchanged() {
    let mut input = b"\x1b_Ga=t,f=24,s=1,v=1;".to_vec();
    input.extend(vec![b'A'; MAX_APC + 100]);
    input.extend(b"\x1b\\ok");
    assert_eq!(scan_whole(&input), input);
}

fn kitty_temp(name: &str, bytes: &[u8]) -> tempfile::NamedTempFile {
    let mut f = tempfile::Builder::new()
        .prefix(&format!("tty-graphics-protocol-{name}-"))
        .tempfile()
        .expect("temp file");
    std::io::Write::write_all(&mut f, bytes).expect("write");
    f
}

#[test]
fn a_byte_that_ends_an_apc_flushes_it_unchanged() {
    for stop in [0x18u8, 0x1a, 0x9c, b'\n'] {
        let mut input = b"\x1b_Ga=t,f=24;AAAA".to_vec();
        input.push(stop);
        input.extend(b"BBBB\x1b\\ok");
        assert_eq!(scan_whole(&input), input, "{stop:#x}");
        // Byte by byte, the same.
        let mut scanner = Scanner::default();
        let got: Vec<u8> = input
            .chunks(1)
            .flat_map(|c| scan_with(&mut scanner, c))
            .collect();
        assert_eq!(got, input, "{stop:#x} in single bytes");
    }
}

#[test]
fn a_temp_file_is_deleted_only_when_it_is_kitty_style_in_a_temp_dir() {
    let named = kitty_temp("a", b"abc");
    let path = named.path().to_path_buf();
    assert_eq!(parts(&scan_whole(&command("a=t,t=t", &path))).1, b"abc");
    assert!(!path.exists(), "deleted");

    let plain = tempfile::Builder::new()
        .prefix("deck-test-plain-")
        .tempfile()
        .expect("temp file");
    std::fs::write(plain.path(), b"abc").expect("write");
    scan_whole(&command("a=t,t=t", plain.path()));
    assert!(plain.path().exists(), "kept: the name is not kitty's");

    // `t=f` never deletes.
    let named = kitty_temp("b", b"abc");
    scan_whole(&command("a=t,t=f", named.path()));
    assert!(named.path().exists());

    // Outside a temp dir: a kitty name in a directory of ours.
    let dir = tempfile::tempdir_in(".").expect("dir");
    let kept = file(dir.path(), "tty-graphics-protocol-c", b"abc");
    let kept = kept.canonicalize().expect("path");
    scan_whole(&command("a=t,t=t", &kept));
    assert!(kept.exists(), "kept: not in a temp dir");
}

#[test]
fn a_kitty_temp_file_that_is_refused_is_still_deleted() {
    let named = kitty_temp("big", b"0123456789");
    let path = named.path().to_path_buf();
    // An offset past the end.
    let cmd = command("a=t,t=t,O=99", &path);
    assert_eq!(scan_whole(&cmd), cmd);
    assert!(!path.exists(), "deleted");
}

#[test]
fn a_big_file_becomes_pieces_of_bounded_size() {
    let dir = tempfile::tempdir().expect("dir");
    let path = file(dir.path(), "big", &vec![7u8; 1 << 20]);
    let (mut max, mut total) = (0, 0);
    Scanner::default().scan(&command("a=t,f=32,s=1,v=1,t=f", &path), |p| {
        max = max.max(p.len());
        total += p.len();
    });
    assert_eq!(max, PIECE);
    assert!(total > 1 << 20);
}

#[test]
fn many_file_commands_in_one_chunk_run_out_of_budget_and_pass_unchanged() {
    let dir = tempfile::tempdir().expect("dir");
    let size = 100 * 1024;
    let path = file(dir.path(), "big", &vec![1u8; size]);
    let one = command("a=t,f=32,s=1,v=1,t=f", &path);
    let input = one.repeat(300);
    // Room for two files; the window never ends during the test.
    let mut scanner = Scanner {
        budget: 2 * size as u64,
        window: Duration::from_secs(3600),
        ..Scanner::default()
    };
    let (mut max, mut total) = (0, 0);
    scanner.scan(&input, |p| {
        max = max.max(p.len());
        total += p.len();
    });
    assert!(max <= PIECE);
    // Two files were read (4/3 of their size each); the other 298 commands passed as they were.
    let read = 2 * size * 4 / 3;
    assert!(total < read + 300 * one.len() + 4096, "{total}");
    assert!(total >= 298 * one.len() + read);
}

#[test]
fn base64_round_trips() {
    for len in 0..8 {
        let bytes: Vec<u8> = (0..len).map(|i| (i * 37) as u8).collect();
        assert_eq!(decode(encode(&bytes).as_bytes()), Some(bytes.clone()));
        assert_eq!(
            decode(encode(&bytes).trim_end_matches('=').as_bytes()),
            Some(bytes)
        );
    }
    assert_eq!(decode(b"a b"), None);
}
