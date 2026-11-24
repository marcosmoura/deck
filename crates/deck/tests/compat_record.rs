//! Recording real programs for the compat suite (`just record-compat <name>`), the checks on
//! scenarios, and the privacy check on every committed recording.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "integration-test helpers fail the test on error"
)]

mod support;

use support::record::{
    Scenario, Step, privacy_violations, recordings_dir, scenarios_dir, write_recording,
};

/// The recorder: plays `compat/scenarios/$DECK_RECORD.scenario` and writes the recording.
#[test]
#[ignore = "run through `just record-compat <name>`"]
fn record() {
    let name = std::env::var("DECK_RECORD").expect("DECK_RECORD names the scenario");
    let force = std::env::var("FORCE").is_ok_and(|v| v == "1");
    let path = recordings_dir().join(format!("{name}.vt"));
    // Refuse before running the program, not after.
    assert!(
        force || !path.exists(),
        "{} exists; run with FORCE=1 to replace it",
        path.display()
    );
    let bytes = support::record::play(&Scenario::named(&name));
    match write_recording(&name, &bytes, force) {
        Ok(path) => println!("recorded {} bytes to {}", bytes.len(), path.display()),
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn a_scenario_is_read_step_by_step() {
    let s = Scenario::parse(
        "t",
        "# a comment\nsize 80x20\nsetup echo hi > a\nrun less 'a b' c\nwait hi\nsend q\\r\nkey C-x\nsleep 5\nstop\n",
    );
    assert_eq!(s.size, (80, 20));
    assert_eq!(s.setup, ["echo hi > a"]);
    assert_eq!(s.argv, ["less", "a b", "c"]);
    assert!(matches!(&s.steps[0], Step::Wait(t) if t == "hi"));
    assert!(matches!(&s.steps[1], Step::Send(b) if b == b"q\r"));
    assert!(matches!(&s.steps[2], Step::Send(b) if b == b"\x18"));
    assert!(matches!(s.steps[3], Step::Sleep(_)));
    assert!(matches!(s.steps[4], Step::Stop));
}

#[test]
fn every_recording_has_a_scenario() {
    // A recording without a scenario cannot be replayed; a scenario without a recording is a
    // program the recording machine did not have (the compatibility doc lists those).
    for entry in std::fs::read_dir(recordings_dir()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "vt") {
            let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
            assert!(
                scenarios_dir().join(format!("{stem}.scenario")).exists(),
                "recording {stem} has no scenario"
            );
        }
    }
}

#[test]
fn committed_recordings_are_private() {
    for entry in std::fs::read_dir(recordings_dir()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "vt") {
            let found = privacy_violations(&std::fs::read(&path).unwrap());
            assert!(found.is_empty(), "{}: {found:?}", path.display());
        }
    }
}

#[test]
fn the_privacy_check_catches_a_home_and_a_users_path() {
    let home = std::env::var("HOME").unwrap();
    let with_home = format!("cwd {home}/project\r\n");
    assert!(!privacy_violations(with_home.as_bytes()).is_empty());
    assert!(!privacy_violations(b"/Users/somebody/code").is_empty());
    assert!(!privacy_violations(b"/home/somebody/code").is_empty());
    assert!(!privacy_violations(b"opened /srv/secret/key.pem").is_empty());
    assert!(!privacy_violations(b"cwd=/mnt/data/x").is_empty());
    assert!(
        privacy_violations(b"a/b src/main.rs https://example.com/x // note /usr/bin/env")
            .is_empty()
    );
    assert!(privacy_violations(b"/tmp/deck-record/x/.gitconfig deck@example.invalid").is_empty());
}

#[test]
fn the_recorder_refuses_a_private_recording_and_an_overwrite() {
    let err = write_recording("privacy_probe", b"/Users/somebody/code", true).unwrap_err();
    assert!(err.contains("private details"), "{err}");
    assert!(!recordings_dir().join("privacy_probe.vt").exists());

    // An existing recording is not replaced without FORCE=1 (checked before the content).
    let existing = std::fs::read_dir(recordings_dir())
        .unwrap()
        .filter_map(Result::ok)
        .find(|e| e.path().extension().is_some_and(|x| x == "vt"));
    if let Some(entry) = existing {
        let name = entry
            .path()
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let err = write_recording(&name, b"x", false).unwrap_err();
        assert!(err.contains("FORCE=1"), "{err}");
    }
}
