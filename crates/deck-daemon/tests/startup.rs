//! Start-up, locks, the handshake and the control requests.

mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use deck_core::protocol::{
    ByeReason, ClientMsg, PROTOCOL_VERSION, Request, Response, Role, ServerMsg, encode,
};
use deck_daemon::{Daemon, DaemonError, DaemonPaths, Options};
use support::{Attach, Client, TestDaemon, config, hello, system_clock};

async fn run_once(paths: DaemonPaths) -> Result<(), DaemonError> {
    Daemon::run_with(paths, config(), std::future::ready(()), Options::default()).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_daemon_on_the_same_home_is_refused_and_the_first_keeps_serving() {
    let daemon = TestDaemon::start().await;
    let err = run_once(daemon.paths.clone()).await.unwrap_err();
    assert!(matches!(err, DaemonError::AlreadyRunning), "{err}");
    assert_eq!(
        fs::read_to_string(&daemon.paths.lock).unwrap(),
        std::process::id().to_string()
    );
    // Still serving, and its socket was not touched.
    let mut control = Client::control(&daemon.paths.socket).await;
    let Response::Sessions(sessions) = control.request(Request::ListSessions).await else {
        panic!("sessions")
    };
    assert!(sessions.is_empty());
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_daemon_with_another_runtime_dir_but_the_same_state_is_refused() {
    let daemon = TestDaemon::start().await;
    let other = tempfile::Builder::new()
        .prefix("dk")
        .tempdir_in("/tmp")
        .unwrap();
    let paths = DaemonPaths {
        runtime_dir: other.path().to_path_buf(),
        socket: other.path().join("deck.sock"),
        lock: other.path().join("deck.lock"),
        ..daemon.paths.clone()
    };
    fs::set_permissions(other.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let err = run_once(paths).await.unwrap_err();
    assert!(matches!(err, DaemonError::StateInUse(_)), "{err}");
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unsafe_runtime_dir_is_refused() {
    let home = tempfile::Builder::new()
        .prefix("dk")
        .tempdir_in("/tmp")
        .unwrap();
    let paths = support::paths_in(home.path());
    fs::create_dir_all(&paths.runtime_dir).unwrap();
    fs::set_permissions(&paths.runtime_dir, fs::Permissions::from_mode(0o755)).unwrap();
    let err = run_once(paths.clone()).await.unwrap_err();
    assert!(matches!(err, DaemonError::UnsafeRuntimeDir { .. }), "{err}");
    assert!(
        err.to_string()
            .contains(paths.runtime_dir.to_str().unwrap())
    );

    // A link to a good directory is not a runtime dir either (this is also how a
    // directory owned by someone else is simulated without a second user).
    let target: PathBuf = home.path().join("elsewhere");
    fs::create_dir(&target).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
    fs::remove_dir(&paths.runtime_dir).unwrap();
    std::os::unix::fs::symlink(&target, &paths.runtime_dir).unwrap();
    let err = run_once(paths).await.unwrap_err();
    assert!(matches!(err, DaemonError::UnsafeRuntimeDir { .. }), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_version_and_a_corrupted_hello_get_version_mismatch() {
    let daemon = TestDaemon::start().await;

    let mut wrong = daemon.connect().await;
    let ClientMsg::Hello {
        role,
        cwd,
        env,
        colors,
        ..
    } = hello(Role::Control, std::env::temp_dir(), Vec::new(), None)
    else {
        unreachable!()
    };
    wrong
        .send(&ClientMsg::Hello {
            version: PROTOCOL_VERSION + 1,
            role,
            cwd,
            env,
            colors,
        })
        .await;
    let msg = wrong.recv(support::TIMEOUT).await.expect("a bye");
    assert_eq!(
        msg,
        ServerMsg::Bye {
            reason: ByeReason::VersionMismatch {
                daemon: PROTOCOL_VERSION
            }
        }
    );

    // The right version, but a tail that does not decode.
    let mut corrupt = daemon.connect().await;
    let mut bytes = encode(&hello(
        Role::Control,
        std::env::temp_dir(),
        vec![("A".into(), "b".into())],
        None,
    ))
    .unwrap();
    let len = bytes.len();
    for b in &mut bytes[4 + 2..len] {
        *b = 0xff;
    }
    corrupt.send_raw(&bytes).await;
    let msg = corrupt.recv(support::TIMEOUT).await.expect("a bye");
    assert_eq!(
        msg,
        ServerMsg::Bye {
            reason: ByeReason::VersionMismatch {
                daemon: PROTOCOL_VERSION
            }
        }
    );

    // Not a Hello at all: the connection is just closed.
    let mut other = daemon.connect().await;
    other.send(&ClientMsg::Detach).await;
    assert!(other.recv(support::TIMEOUT).await.is_none());
    assert!(other.eof);
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_daemon_of_another_version_says_so_to_a_normal_client() {
    let daemon = TestDaemon::start_with(
        config(),
        Options {
            clock: system_clock(),
            protocol_version: 999,
            ..Options::default()
        },
    )
    .await;
    let mut client = daemon.connect().await;
    client
        .send(&hello(
            Role::Control,
            std::env::temp_dir(),
            Vec::new(),
            None,
        ))
        .await;
    let msg = client.recv(support::TIMEOUT).await.expect("a bye");
    assert_eq!(
        msg,
        ServerMsg::Bye {
            reason: ByeReason::VersionMismatch { daemon: 999 }
        }
    );
    daemon.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn kill_server_replies_ok_and_stops_the_daemon_and_removes_its_files() {
    let daemon = TestDaemon::start().await;
    let socket = daemon.paths.socket.clone();
    let mut control = Client::control(&daemon.paths.socket).await;
    assert_eq!(control.request(Request::KillServer).await, Response::Ok);
    // The control client is told the daemon is going away.
    control.wait(|c| c.bye.is_some()).await;
    assert_eq!(control.bye, Some(ByeReason::ServerShutdown));
    daemon.wait_exit().await.unwrap();
    assert!(!socket.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_sends_bye_to_attached_clients_and_frees_the_locks() {
    let daemon = TestDaemon::start().await;
    let mut client = Client::attach(&daemon.paths.socket, Attach::default()).await;
    let paths = daemon.paths.clone();
    daemon.stop().await.unwrap();
    client.wait(|c| c.bye.is_some()).await;
    assert_eq!(client.bye, Some(ByeReason::ServerShutdown));
    assert!(!paths.socket.exists());
    // Both locks are free again.
    let second = run_once(paths).await;
    assert!(second.is_ok(), "{second:?}");
}
