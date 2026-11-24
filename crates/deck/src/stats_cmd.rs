//! `deck stats`: what the daemon uses and holds.

use std::process::ExitCode;

use anyhow::bail;
use deck_core::protocol::{Request, Response, Stats};
use serde_json::{Value, json};

use crate::tree::{Api, NO_DAEMON, finish};

const MIB: f64 = 1024.0 * 1024.0;

pub fn run(json: bool) -> ExitCode {
    let result: anyhow::Result<Option<Value>> = (|| {
        let mut api = match Api::connect() {
            Ok(api) => api,
            // No daemon: nothing is used, and that is not an error here.
            Err(e) if e.to_string() == NO_DAEMON => {
                if json {
                    return Ok(Some(json!({ "running": false })));
                }
                println!("{NO_DAEMON}");
                return Ok(None);
            }
            Err(e) => return Err(e),
        };
        let Response::Stats(stats) = api.ask(Request::Stats)? else {
            bail!("unexpected answer from the daemon");
        };
        if json {
            return Ok(Some(stats_json(&stats)));
        }
        print!("{}", stats_text(&stats));
        Ok(None)
    })();
    finish(json, result)
}

#[allow(clippy::cast_precision_loss, reason = "figures for people to read")]
fn mib(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / MIB)
}

#[allow(clippy::cast_precision_loss, reason = "figures for people to read")]
fn secs(ms: u64) -> f64 {
    ms as f64 / 1000.0
}

/// `45s`, `12m`, `3h 5m`, `3d 4h`.
fn uptime(total: u64) -> String {
    let (d, h, m) = (total / 86_400, total / 3600 % 24, total / 60 % 60);
    match (d, h, m) {
        (0, 0, 0) => format!("{total}s"),
        (0, 0, m) => format!("{m}m"),
        (0, h, m) => format!("{h}h {m}m"),
        (d, h, _) => format!("{d}d {h}h"),
    }
}

fn plural(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn stats_text(s: &Stats) -> String {
    let or_dash = |v: Option<String>| v.unwrap_or_else(|| "?".into());
    let cpu = |ms: Option<u64>| or_dash(ms.map(|ms| format!("{:.1}", secs(ms))));
    let mut out = format!(
        "deck daemon pid {}, up {}, version {} (protocol {})\n",
        s.pid,
        uptime(s.uptime_secs),
        s.version,
        s.protocol
    );
    out.push_str(&format!(
        "memory {}  fds {}  threads {}  cpu {}s user, {}s system\n",
        or_dash(s.rss_bytes.map(mib)),
        or_dash(s.fds.map(|n| n.to_string())),
        or_dash(s.threads.map(|n| n.to_string())),
        cpu(s.cpu_user_ms),
        cpu(s.cpu_system_ms),
    ));
    let exited = if s.exited > 0 {
        format!(" ({} exited)", s.exited)
    } else {
        String::new()
    };
    out.push_str(&format!(
        "{}, {}{exited}, {}\n",
        plural(s.sessions, "session", "sessions"),
        plural(s.live + s.exited, "terminal", "terminals"),
        plural(s.clients, "client", "clients"),
    ));
    if s.per_terminal.is_empty() {
        return out;
    }
    let mut rows: Vec<[String; 5]> = vec![[
        "terminal".into(),
        "session".into(),
        "size".into(),
        "history".into(),
        "read".into(),
    ]];
    let mut terminals: Vec<_> = s.per_terminal.iter().collect();
    terminals.sort_by_key(|t| t.id.0);
    for t in terminals {
        rows.push([
            t.id.0.to_string(),
            t.session.clone(),
            format!("{}x{}", t.size.0, t.size.1),
            t.history.to_string(),
            mib(t.bytes_read),
        ]);
    }
    let widths = rows.iter().fold([0usize; 5], |mut w, row| {
        for (width, cell) in w.iter_mut().zip(row) {
            *width = (*width).max(cell.len());
        }
        w
    });
    out.push('\n');
    for row in &rows {
        let cells: Vec<String> = row
            .iter()
            .zip(&widths)
            .map(|(cell, w)| format!("{cell:<w$}"))
            .collect();
        out.push_str(cells.join("  ").trim_end());
        out.push('\n');
    }
    out
}

/// Sizes in bytes, durations in seconds; an unread figure is `null`.
fn stats_json(s: &Stats) -> Value {
    let list: Vec<Value> = s
        .per_terminal
        .iter()
        .map(|t| {
            json!({
                "id": t.id.0,
                "session": t.session,
                "history": t.history,
                "cols": t.size.0,
                "rows": t.size.1,
                "bytes_read": t.bytes_read,
            })
        })
        .collect();
    json!({
        "running": true,
        "pid": s.pid,
        "uptime_secs": s.uptime_secs,
        "version": s.version,
        "protocol": s.protocol,
        "rss_bytes": s.rss_bytes,
        "fds": s.fds,
        "threads": s.threads,
        "cpu_user_secs": s.cpu_user_ms.map(secs),
        "cpu_system_secs": s.cpu_system_ms.map(secs),
        "sessions": s.sessions,
        "terminals": s.live + s.exited,
        "exited": s.exited,
        "clients": s.clients,
        "terminal_list": list,
    })
}

#[cfg(test)]
mod tests {
    use deck_core::model::TerminalId;
    use deck_core::protocol::TerminalStats;

    use super::*;

    fn sample() -> Stats {
        Stats {
            pid: 4312,
            uptime_secs: 3 * 86_400 + 4 * 3600 + 5,
            version: "0.4.0".into(),
            protocol: 22,
            rss_bytes: Some(88_276_787),
            fds: Some(61),
            threads: Some(38),
            cpu_user_ms: Some(312_400),
            cpu_system_ms: Some(41_000),
            sessions: 2,
            live: 13,
            exited: 1,
            clients: 1,
            per_terminal: vec![TerminalStats {
                id: TerminalId(4),
                session: "main".into(),
                history: 10_000,
                size: (120, 40),
                bytes_read: 50_450_000,
            }],
        }
    }

    #[test]
    fn the_text_has_the_documented_shape() {
        let text = stats_text(&sample());
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[..4],
            [
                "deck daemon pid 4312, up 3d 4h, version 0.4.0 (protocol 22)",
                "memory 84.2 MiB  fds 61  threads 38  cpu 312.4s user, 41.0s system",
                "2 sessions, 14 terminals (1 exited), 1 client",
                "",
            ]
        );
        assert_eq!(lines[4], "terminal  session  size    history  read");
        assert_eq!(lines[5], "4         main     120x40  10000    48.1 MiB");
    }

    #[test]
    fn an_unread_figure_shows_as_a_question_mark_and_null() {
        let mut s = sample();
        s.rss_bytes = None;
        s.threads = None;
        s.per_terminal.clear();
        assert!(stats_text(&s).contains("memory ?  fds 61  threads ?"));
        let j = stats_json(&s);
        assert!(j["rss_bytes"].is_null() && j["threads"].is_null());
        assert_eq!(j["cpu_user_secs"], 312.4);
        assert_eq!(j["terminals"], 14);
    }

    #[test]
    fn uptime_reads_in_two_units() {
        assert_eq!(uptime(45), "45s");
        assert_eq!(uptime(12 * 60 + 3), "12m");
        assert_eq!(uptime(3 * 3600 + 5 * 60), "3h 5m");
        assert_eq!(uptime(86_400 + 3600), "1d 1h");
    }
}
