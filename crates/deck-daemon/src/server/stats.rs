//! `Request::Stats` and the periodic log line: the daemon's own figures and what it holds.

use std::sync::Arc;
use std::time::Duration;

use deck_core::model::SessionId;
use deck_core::protocol::{Stats, TerminalStats};

use super::Shared;
use crate::platform;
use crate::terminal::TermHandle;

/// MiB per byte, for the log line.
const MIB: f64 = 1024.0 * 1024.0;

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

impl Shared {
    /// The daemon's figures. The measuring and the questions to the emulators run on a
    /// blocking thread, never under the state lock; `per_terminal` asks for the terminal
    /// list too.
    pub async fn stats(self: &Arc<Self>, per_terminal: bool) -> Stats {
        let (sessions, clients, terminals): (Vec<(SessionId, String)>, usize, Vec<_>) = {
            let st = self.st();
            (
                st.sessions
                    .iter()
                    .map(|s| (s.model.id, s.model.name.clone()))
                    .collect(),
                st.clients.values().filter(|c| c.session.is_some()).count(),
                st.sessions
                    .iter()
                    .flat_map(|s| s.terminals.values().cloned())
                    .collect(),
            )
        };
        let uptime = self.started.elapsed().as_secs();
        let version = self.version;
        let measured = tokio::task::spawn_blocking(move || {
            let own = platform::self_stats();
            let exited = terminals.iter().filter(|h| h.has_exited()).count();
            let live = terminals.len() - exited;
            let list = if per_terminal {
                terminals
                    .iter()
                    .map(|h| terminal_stats(h, &sessions))
                    .collect()
            } else {
                Vec::new()
            };
            (own, live, exited, sessions.len(), list)
        })
        .await;
        let (own, live, exited, session_count, per_terminal) = measured.unwrap_or_default();
        Stats {
            pid: std::process::id(),
            uptime_secs: uptime,
            version: env!("CARGO_PKG_VERSION").to_string(),
            protocol: version,
            rss_bytes: own.rss_bytes,
            fds: own.fds,
            threads: own.threads,
            cpu_user_ms: own.cpu_user.map(millis),
            cpu_system_ms: own.cpu_system.map(millis),
            sessions: count(session_count),
            live: count(live),
            exited: count(exited),
            clients: count(clients),
            per_terminal,
        }
    }

    /// Logs the one-line summary at INFO; a figure that could not be read is left out.
    pub async fn log_stats(self: &Arc<Self>) {
        let s = self.stats(false).await;
        let mut line = String::from("stats:");
        if let Some(rss) = s.rss_bytes {
            #[allow(clippy::cast_precision_loss, reason = "a size for people to read")]
            let mib = rss as f64 / MIB;
            line.push_str(&format!(" rss={mib:.1}"));
        }
        if let Some(fds) = s.fds {
            line.push_str(&format!(" fds={fds}"));
        }
        if let Some(threads) = s.threads {
            line.push_str(&format!(" threads={threads}"));
        }
        if let (Some(user), Some(system)) = (s.cpu_user_ms, s.cpu_system_ms) {
            #[allow(clippy::cast_precision_loss, reason = "a time for people to read")]
            let secs = (user + system) as f64 / 1000.0;
            line.push_str(&format!(" cpu={secs:.1}s"));
        }
        line.push_str(&format!(
            " sessions={} terminals={} clients={}",
            s.sessions,
            s.live + s.exited,
            s.clients
        ));
        tracing::info!("{line}");
    }
}

fn terminal_stats(h: &Arc<TermHandle>, sessions: &[(SessionId, String)]) -> TerminalStats {
    let (history, size) = h
        .with_emu(|e| (count(e.history()), e.size()))
        .unwrap_or((0, (0, 0)));
    TerminalStats {
        id: h.id,
        session: sessions
            .iter()
            .find(|(id, _)| *id == h.session)
            .map(|(_, name)| name.clone())
            .unwrap_or_default(),
        history,
        size,
        bytes_read: h.bytes_read(),
    }
}

/// Logs the stats line every `period`, the first one after the first period.
pub async fn log_every(shared: Arc<Shared>, period: Duration) {
    let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
    loop {
        tick.tick().await;
        shared.log_stats().await;
    }
}
