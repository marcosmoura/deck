//! The agent state machine of one terminal: what process detection and hook events do to
//! its [`AgentState`]. Pure: every rule takes the daemon clock's `now`, so tests drive the
//! 2 s, 3 s and 60 s rules exactly.

use std::time::Duration;

use deck_core::agents::{
    Activity, AgentKind, AgentSource, AgentState, AgentStatus, OpenCodeTracker, clean_activity,
    clip_prompt, valid_session_id,
};

/// Output within this long makes a process-detected agent `active`.
pub const ACTIVE_WINDOW: Duration = Duration::from_secs(2);
/// A hook-sourced state is cleared once its agent has been out of the foreground this long.
pub const GONE_GRACE: Duration = Duration::from_secs(3);
/// A hook-sourced `working` without an event or output for this long falls back to the
/// process-only state.
pub const STUCK_WORKING: Duration = Duration::from_secs(60);
/// A command makes its terminal busy once it has run this long, so a quick one never shows.
pub const BUSY_AFTER: Duration = Duration::from_secs(1);
/// An event this much older than the last applied activity is taken as a clock that stepped back.
const CLOCK_STEP_MS: u64 = 60_000;

/// What the daemon remembers about a terminal's agent besides the state itself.
#[derive(Debug, Default)]
pub struct AgentRt {
    /// When the last hook event (or answered prompt) came.
    last_event: Duration,
    /// Since when the agent's process is not in the foreground.
    gone_since: Option<Duration>,
    /// Sorts the raw events of OpenCode's plugin.
    opencode: OpenCodeTracker,
    /// Since when a command runs in the foreground.
    running_since: Option<Duration>,
    /// Whether the terminal counts as busy.
    pub busy: bool,
    /// The `at_ms` of the last event whose activity was applied (0: none yet).
    activity_at: u64,
}

/// One hook event.
#[derive(Debug, Clone)]
pub struct Event {
    pub kind: AgentKind,
    pub status: Option<AgentStatus>,
    pub session_id: Option<String>,
    pub prompt: Option<String>,
    pub activity: Activity,
    /// When the event began, Unix milliseconds; 0 when unknown (never older than anything).
    pub at_ms: u64,
}

fn process_status(last_output: Duration, now: Duration) -> AgentStatus {
    if now.saturating_sub(last_output) < ACTIVE_WINDOW {
        AgentStatus::Active
    } else {
        AgentStatus::Quiet
    }
}

/// One detection tick for a terminal: `found` is the agent its foreground process is, if
/// any. Returns whether the state changed.
pub fn detect(
    agent: &mut Option<AgentState>,
    rt: &mut AgentRt,
    found: Option<AgentKind>,
    last_output: Duration,
    now: Duration,
) -> bool {
    let status = process_status(last_output, now);
    let Some(state) = agent else {
        let Some(kind) = found else {
            return false;
        };
        *agent = Some(AgentState {
            kind,
            status,
            source: AgentSource::Process,
            session_id: None,
            first_prompt: None,
            since: now,
            activity: None,
        });
        return true;
    };
    match state.source {
        AgentSource::Process => match found {
            None => {
                *agent = None;
                true
            }
            Some(kind) if kind != state.kind => {
                *agent = Some(AgentState {
                    kind,
                    status,
                    source: AgentSource::Process,
                    session_id: None,
                    first_prompt: None,
                    since: now,
                    activity: None,
                });
                true
            }
            Some(_) if state.status != status => {
                state.status = status;
                state.since = now;
                true
            }
            Some(_) => false,
        },
        AgentSource::Hooks => {
            if found == Some(state.kind) {
                rt.gone_since = None;
            } else {
                let since = *rt.gone_since.get_or_insert(now);
                if now.saturating_sub(since) >= GONE_GRACE {
                    *agent = None;
                    rt.gone_since = None;
                    return true;
                }
            }
            let quiet_for = now.saturating_sub(rt.last_event.max(last_output));
            if found.is_some() && state.status == AgentStatus::Working && quiet_for >= STUCK_WORKING
            {
                state.source = AgentSource::Process;
                state.status = status;
                state.since = now;
                state.activity = None;
                return true;
            }
            false
        }
    }
}

/// A hook event. Returns whether the state changed.
pub fn apply_event(
    agent: &mut Option<AgentState>,
    rt: &mut AgentRt,
    event: &Event,
    now: Duration,
) -> bool {
    rt.last_event = now;
    rt.gone_since = None;
    // An event older than the last activity applied only gives its status. One more than
    // `CLOCK_STEP_MS` older is not late (no hook is): the wall clock stepped back.
    let fresh = event.at_ms == 0
        || event.at_ms >= rt.activity_at
        || rt.activity_at - event.at_ms > CLOCK_STEP_MS;
    if fresh && event.activity != Activity::Keep && event.at_ms != 0 {
        rt.activity_at = event.at_ms;
    }
    if event.status == Some(AgentStatus::None) {
        return agent.take().is_some();
    }
    let existing = agent.as_ref().filter(|s| s.kind == event.kind);
    // Without a status the event only adds to a state that exists.
    let Some(status) = event.status.or_else(|| existing.map(|s| s.status)) else {
        return false;
    };
    let mut state = existing.cloned().unwrap_or(AgentState {
        kind: event.kind,
        status,
        source: AgentSource::Hooks,
        session_id: None,
        first_prompt: None,
        since: now,
        activity: None,
    });
    if fresh {
        // The daemon trusts no hook to have cleaned the line.
        match &event.activity {
            Activity::Keep => {}
            Activity::Clear => state.activity = None,
            Activity::Set(line) => state.activity = clean_activity(line),
        }
    }
    if state.status != status || state.source != AgentSource::Hooks {
        state.since = now;
    }
    state.status = status;
    state.source = AgentSource::Hooks;
    // An id that could never be resumed is not kept: it would be copied into every view and
    // into `state.json`, whatever its size.
    if let Some(id) = event.session_id.as_ref().filter(|id| valid_session_id(id))
        && state.session_id.as_ref() != Some(id)
    {
        state.session_id = Some(id.clone());
        state.first_prompt = None;
    }
    if state.first_prompt.is_none() {
        state.first_prompt = event.prompt.as_deref().and_then(clip_prompt);
    }
    let changed = agent.as_ref() != Some(&state);
    *agent = Some(state);
    changed
}

/// An event of OpenCode's plugin, already parsed: the terminal's tracker decides what it means.
pub fn apply_opencode(
    agent: &mut Option<AgentState>,
    rt: &mut AgentRt,
    doc: &serde_json::Value,
    now: Duration,
) -> bool {
    let Some(update) = rt.opencode.feed(doc) else {
        return false;
    };
    let event = Event {
        kind: AgentKind::OpenCode,
        status: update.status,
        session_id: update.session_id,
        prompt: update.prompt,
        activity: update.activity,
        at_ms: 0,
    };
    apply_event(agent, rt, &event, now)
}

/// The user typed into the terminal. A hook-sourced `waiting` agent is answered: it works
/// again. Returns whether the state changed.
pub fn answered(agent: &mut Option<AgentState>, rt: &mut AgentRt, now: Duration) -> bool {
    match agent {
        Some(state)
            if state.source == AgentSource::Hooks && state.status == AgentStatus::Waiting =>
        {
            state.status = AgentStatus::Working;
            state.since = now;
            state.activity = None;
            rt.last_event = now;
            true
        }
        _ => false,
    }
}

/// One detection tick of a terminal's command: `running` is whether a command is in the
/// foreground now. The terminal is busy once the command has run for [`BUSY_AFTER`].
/// Returns whether `busy` changed.
pub fn detect_busy(rt: &mut AgentRt, running: bool, now: Duration) -> bool {
    let busy = if running {
        let since = *rt.running_since.get_or_insert(now);
        now.saturating_sub(since) >= BUSY_AFTER
    } else {
        rt.running_since = None;
        false
    };
    std::mem::replace(&mut rt.busy, busy) != busy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_is_busy_after_a_second_and_not_once_it_ends() {
        let mut rt = AgentRt::default();
        assert!(!detect_busy(&mut rt, true, secs(10)));
        assert!(!rt.busy);
        assert!(detect_busy(&mut rt, true, secs(11)));
        assert!(rt.busy);
        assert!(!detect_busy(&mut rt, true, secs(12)));
        assert!(detect_busy(&mut rt, false, secs(13)));
        assert!(!rt.busy);
        // A command seen once and gone by the next tick never shows.
        assert!(!detect_busy(&mut rt, true, secs(14)));
        assert!(!detect_busy(&mut rt, false, secs(15)));
        assert!(!detect_busy(&mut rt, true, secs(16)));
        assert!(!rt.busy);
    }

    const fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn event(status: Option<AgentStatus>) -> Event {
        Event {
            kind: AgentKind::Claude,
            status,
            session_id: None,
            prompt: None,
            activity: Activity::Keep,
            at_ms: 0,
        }
    }

    #[test]
    fn a_process_is_active_while_printing_and_quiet_after() {
        let (mut agent, mut rt) = (None, AgentRt::default());
        assert!(!detect(&mut agent, &mut rt, None, secs(0), secs(0)));
        assert!(detect(
            &mut agent,
            &mut rt,
            Some(AgentKind::Codex),
            secs(10),
            secs(11)
        ));
        let state = agent.clone().unwrap();
        assert_eq!(state.kind, AgentKind::Codex);
        assert_eq!(state.source, AgentSource::Process);
        assert_eq!(state.status, AgentStatus::Active);
        // Still printing: nothing changes.
        assert!(!detect(
            &mut agent,
            &mut rt,
            Some(AgentKind::Codex),
            secs(12),
            secs(13)
        ));
        // Two seconds of silence.
        assert!(detect(
            &mut agent,
            &mut rt,
            Some(AgentKind::Codex),
            secs(12),
            secs(14)
        ));
        let state = agent.clone().unwrap();
        assert_eq!(state.status, AgentStatus::Quiet);
        assert_eq!(state.since, secs(14));
        // The agent leaves the foreground.
        assert!(detect(&mut agent, &mut rt, None, secs(12), secs(15)));
        assert_eq!(agent, None);
    }

    #[test]
    fn another_agent_in_the_foreground_replaces_a_process_state() {
        let (mut agent, mut rt) = (None, AgentRt::default());
        detect(
            &mut agent,
            &mut rt,
            Some(AgentKind::Claude),
            secs(0),
            secs(0),
        );
        assert!(detect(
            &mut agent,
            &mut rt,
            Some(AgentKind::Codex),
            secs(0),
            secs(0)
        ));
        assert_eq!(agent.unwrap().kind, AgentKind::Codex);
    }

    #[test]
    fn hook_events_set_the_state_and_win_over_the_process() {
        let (mut agent, mut rt) = (None, AgentRt::default());
        assert!(apply_event(
            &mut agent,
            &mut rt,
            &event(Some(AgentStatus::Working)),
            secs(1)
        ));
        let state = agent.clone().unwrap();
        assert_eq!(state.source, AgentSource::Hooks);
        assert_eq!(state.since, secs(1));
        // The same status again changes nothing, and keeps `since`.
        assert!(!apply_event(
            &mut agent,
            &mut rt,
            &event(Some(AgentStatus::Working)),
            secs(2)
        ));
        assert_eq!(agent.as_ref().unwrap().since, secs(1));
        // Detection does not touch it while the agent runs.
        assert!(!detect(
            &mut agent,
            &mut rt,
            Some(AgentKind::Claude),
            secs(0),
            secs(30)
        ));
        assert_eq!(agent.as_ref().unwrap().status, AgentStatus::Working);
        // `none` clears it.
        assert!(apply_event(
            &mut agent,
            &mut rt,
            &event(Some(AgentStatus::None)),
            secs(31)
        ));
        assert_eq!(agent, None);
    }

    #[test]
    fn an_event_without_a_status_only_adds_to_an_existing_state() {
        let (mut agent, mut rt) = (None, AgentRt::default());
        let mut ev = event(None);
        ev.session_id = Some("s".into());
        assert!(!apply_event(&mut agent, &mut rt, &ev, secs(1)));
        assert_eq!(agent, None);
        apply_event(
            &mut agent,
            &mut rt,
            &event(Some(AgentStatus::Idle)),
            secs(1),
        );
        assert!(apply_event(&mut agent, &mut rt, &ev, secs(2)));
        let state = agent.unwrap();
        assert_eq!(state.status, AgentStatus::Idle);
        assert_eq!(state.session_id.as_deref(), Some("s"));
    }

    #[test]
    fn a_session_id_that_could_not_be_resumed_is_dropped() {
        let (mut agent, mut rt) = (None, AgentRt::default());
        let with = |id: String| Event {
            session_id: Some(id),
            ..event(Some(AgentStatus::Working))
        };
        // Oversized, or with characters no agent's id has: the status still applies.
        for bad in [
            "a".repeat(129),
            "x".repeat(1 << 20),
            "a b".into(),
            "-a".into(),
        ] {
            apply_event(&mut agent, &mut rt, &with(bad), secs(1));
            let state = agent.clone().unwrap();
            assert_eq!(state.status, AgentStatus::Working);
            assert_eq!(state.session_id, None);
        }
        apply_event(&mut agent, &mut rt, &with("ses_1".into()), secs(2));
        assert_eq!(agent.clone().unwrap().session_id.as_deref(), Some("ses_1"));
        // A bad id after a good one changes nothing.
        apply_event(&mut agent, &mut rt, &with("a b".into()), secs(3));
        assert_eq!(agent.unwrap().session_id.as_deref(), Some("ses_1"));
    }

    #[test]
    fn a_new_session_id_resets_the_first_prompt() {
        let (mut agent, mut rt) = (None, AgentRt::default());
        let with = |id: &str, prompt: &str| Event {
            session_id: Some(id.into()),
            prompt: Some(prompt.into()),
            ..event(Some(AgentStatus::Working))
        };
        apply_event(
            &mut agent,
            &mut rt,
            &with("a", "first\nsecond line"),
            secs(1),
        );
        let state = agent.clone().unwrap();
        assert_eq!(state.session_id.as_deref(), Some("a"));
        assert_eq!(state.first_prompt.as_deref(), Some("first"));
        // Same session: the first prompt stays.
        apply_event(&mut agent, &mut rt, &with("a", "later"), secs(2));
        assert_eq!(
            agent.as_ref().unwrap().first_prompt.as_deref(),
            Some("first")
        );
        // A blank prompt is not a first prompt.
        apply_event(&mut agent, &mut rt, &with("b", "  \n"), secs(3));
        let state = agent.clone().unwrap();
        assert_eq!(state.session_id.as_deref(), Some("b"));
        assert_eq!(state.first_prompt, None);
        apply_event(&mut agent, &mut rt, &with("b", "next"), secs(4));
        assert_eq!(
            agent.as_ref().unwrap().first_prompt.as_deref(),
            Some("next")
        );
    }

    fn line(agent: &Option<AgentState>) -> Option<&str> {
        agent.as_ref().and_then(|a| a.activity.as_deref())
    }

    fn with_line(status: Option<AgentStatus>, activity: Activity, at_ms: u64) -> Event {
        Event {
            activity,
            at_ms,
            ..event(status)
        }
    }

    #[test]
    fn the_activity_is_kept_cleared_or_set_by_events() {
        let (mut agent, mut rt) = (None, AgentRt::default());
        let working = Some(AgentStatus::Working);
        let set = |s: &str| Activity::Set(s.into());
        assert!(apply_event(
            &mut agent,
            &mut rt,
            &with_line(working, set("thinking"), 10),
            secs(1)
        ));
        assert_eq!(line(&agent), Some("thinking"));
        // Keep leaves it, and changes nothing.
        assert!(!apply_event(
            &mut agent,
            &mut rt,
            &with_line(working, Activity::Keep, 11),
            secs(2)
        ));
        assert_eq!(line(&agent), Some("thinking"));
        // Set replaces it, with the line cleaned and cut again.
        let raw = format!("\u{1b}[31m{}\nsecond line", "x".repeat(200));
        assert!(apply_event(
            &mut agent,
            &mut rt,
            &with_line(working, Activity::Set(raw), 12),
            secs(3)
        ));
        let shown = line(&agent).unwrap();
        assert_eq!(shown.chars().count(), 80);
        assert!(shown.starts_with("[31mxxx") && !shown.contains('\n'));
        // A blank line clears; so does Clear.
        apply_event(
            &mut agent,
            &mut rt,
            &with_line(working, set("a"), 13),
            secs(4),
        );
        assert!(apply_event(
            &mut agent,
            &mut rt,
            &with_line(working, set(" \n "), 14),
            secs(5)
        ));
        assert_eq!(line(&agent), None);
        apply_event(
            &mut agent,
            &mut rt,
            &with_line(working, set("a"), 15),
            secs(6),
        );
        assert!(apply_event(
            &mut agent,
            &mut rt,
            &with_line(working, Activity::Clear, 16),
            secs(7)
        ));
        assert_eq!(line(&agent), None);
    }

    #[test]
    fn an_older_event_does_not_change_the_activity() {
        let (mut agent, mut rt) = (None, AgentRt::default());
        let set = |s: &str| Activity::Set(s.into());
        apply_event(
            &mut agent,
            &mut rt,
            &with_line(Some(AgentStatus::Idle), set("all done"), 200),
            secs(1),
        );
        // A PreToolUse that started before the Stop: its status applies, its line does not.
        assert!(apply_event(
            &mut agent,
            &mut rt,
            &with_line(Some(AgentStatus::Working), set("running late"), 150),
            secs(2)
        ));
        assert_eq!(line(&agent), Some("all done"));
        assert_eq!(agent.as_ref().unwrap().status, AgentStatus::Working);
        // An event from a caller that has no clock is never older; an equal one is not either.
        apply_event(
            &mut agent,
            &mut rt,
            &with_line(None, set("now"), 0),
            secs(3),
        );
        assert_eq!(line(&agent), Some("now"));
        apply_event(
            &mut agent,
            &mut rt,
            &with_line(None, set("same time"), 200),
            secs(4),
        );
        assert_eq!(line(&agent), Some("same time"));
        // A Clear that is older does not clear.
        apply_event(
            &mut agent,
            &mut rt,
            &with_line(None, Activity::Clear, 100),
            secs(5),
        );
        assert_eq!(line(&agent), Some("same time"));
    }

    #[test]
    fn a_clock_that_stepped_back_does_not_freeze_the_line() {
        let (mut agent, mut rt) = (None, AgentRt::default());
        let set = |s: &str| Activity::Set(s.into());
        let at = |t, a| with_line(Some(AgentStatus::Working), a, t);
        apply_event(&mut agent, &mut rt, &at(1_000_000, set("before")), secs(1));
        // Late by a minute or less: ignored.
        apply_event(&mut agent, &mut rt, &at(940_000, set("late")), secs(2));
        assert_eq!(line(&agent), Some("before"));
        // Over a minute behind: the clock stepped back, so the line applies and the order restarts.
        apply_event(
            &mut agent,
            &mut rt,
            &at(939_999, set("after step")),
            secs(3),
        );
        assert_eq!(line(&agent), Some("after step"));
        apply_event(&mut agent, &mut rt, &at(939_000, set("older")), secs(4));
        assert_eq!(line(&agent), Some("after step"));
        apply_event(&mut agent, &mut rt, &at(940_500, set("newer")), secs(5));
        assert_eq!(line(&agent), Some("newer"));
    }

    #[test]
    fn the_activity_ends_with_the_state_that_carried_it() {
        let set = Activity::Set("running x".into());
        // SessionEnd.
        let (mut agent, mut rt) = (None, AgentRt::default());
        apply_event(
            &mut agent,
            &mut rt,
            &with_line(Some(AgentStatus::Working), set.clone(), 1),
            secs(1),
        );
        apply_event(
            &mut agent,
            &mut rt,
            &with_line(Some(AgentStatus::None), Activity::Clear, 2),
            secs(2),
        );
        assert_eq!(agent, None);
        // The agent leaves the foreground.
        apply_event(
            &mut agent,
            &mut rt,
            &with_line(Some(AgentStatus::Working), set.clone(), 3),
            secs(3),
        );
        detect(&mut agent, &mut rt, None, secs(0), secs(4));
        assert!(detect(&mut agent, &mut rt, None, secs(0), secs(7)));
        assert_eq!(agent, None);
        // A working state with no news for a minute falls back to the process state.
        apply_event(
            &mut agent,
            &mut rt,
            &with_line(Some(AgentStatus::Working), set.clone(), 4),
            secs(100),
        );
        let found = Some(AgentKind::Claude);
        assert!(detect(&mut agent, &mut rt, found, secs(100), secs(160)));
        assert_eq!(line(&agent), None);
        // A typed answer to `waiting`.
        let (mut agent, mut rt) = (None, AgentRt::default());
        apply_event(
            &mut agent,
            &mut rt,
            &with_line(Some(AgentStatus::Waiting), set, 5),
            secs(1),
        );
        assert_eq!(line(&agent), Some("running x"));
        assert!(answered(&mut agent, &mut rt, secs(2)));
        assert_eq!(line(&agent), None);
    }

    #[test]
    fn typing_answers_a_waiting_agent_only() {
        let (mut agent, mut rt) = (None, AgentRt::default());
        assert!(!answered(&mut agent, &mut rt, secs(1)));
        apply_event(
            &mut agent,
            &mut rt,
            &event(Some(AgentStatus::Waiting)),
            secs(1),
        );
        assert!(answered(&mut agent, &mut rt, secs(2)));
        assert_eq!(agent.as_ref().unwrap().status, AgentStatus::Working);
        assert!(!answered(&mut agent, &mut rt, secs(3)));
        // A process-detected agent is never `waiting`, and is left alone.
        let mut process = None;
        detect(
            &mut process,
            &mut rt,
            Some(AgentKind::Claude),
            secs(0),
            secs(0),
        );
        assert!(!answered(&mut process, &mut rt, secs(1)));
    }

    #[test]
    fn a_working_agent_with_no_news_for_a_minute_falls_back_to_the_process_state() {
        let (mut agent, mut rt) = (None, AgentRt::default());
        apply_event(
            &mut agent,
            &mut rt,
            &event(Some(AgentStatus::Working)),
            secs(100),
        );
        let found = Some(AgentKind::Claude);
        // Output keeps it working.
        assert!(!detect(&mut agent, &mut rt, found, secs(150), secs(159)));
        assert_eq!(agent.as_ref().unwrap().source, AgentSource::Hooks);
        // 60 s after the last output.
        assert!(!detect(&mut agent, &mut rt, found, secs(150), secs(209)));
        assert!(detect(&mut agent, &mut rt, found, secs(150), secs(210)));
        let state = agent.clone().unwrap();
        assert_eq!(state.source, AgentSource::Process);
        assert_eq!(state.status, AgentStatus::Quiet);
        // A waiting agent is not given up on: the user may be away.
        let (mut agent, mut rt) = (None, AgentRt::default());
        apply_event(
            &mut agent,
            &mut rt,
            &event(Some(AgentStatus::Waiting)),
            secs(0),
        );
        assert!(!detect(&mut agent, &mut rt, found, secs(0), secs(500)));
        assert_eq!(agent.unwrap().source, AgentSource::Hooks);
    }

    #[test]
    fn a_hook_state_is_cleared_three_seconds_after_the_agent_left() {
        let (mut agent, mut rt) = (None, AgentRt::default());
        apply_event(
            &mut agent,
            &mut rt,
            &event(Some(AgentStatus::Idle)),
            secs(10),
        );
        let found = Some(AgentKind::Claude);
        assert!(!detect(&mut agent, &mut rt, found, secs(0), secs(11)));
        // Gone at 12 s: the grace counts from there.
        assert!(!detect(&mut agent, &mut rt, None, secs(0), secs(12)));
        assert!(!detect(
            &mut agent,
            &mut rt,
            None,
            secs(0),
            Duration::from_millis(14_900)
        ));
        // Back in the foreground: the clock starts over.
        assert!(!detect(&mut agent, &mut rt, found, secs(0), secs(15)));
        assert!(!detect(&mut agent, &mut rt, None, secs(0), secs(16)));
        assert!(!detect(&mut agent, &mut rt, None, secs(0), secs(18)));
        assert!(detect(&mut agent, &mut rt, None, secs(0), secs(19)));
        assert_eq!(agent, None);
    }

    #[test]
    fn another_agent_in_the_foreground_starts_the_gone_clock_of_a_hook_state() {
        let (mut agent, mut rt) = (None, AgentRt::default());
        apply_event(
            &mut agent,
            &mut rt,
            &event(Some(AgentStatus::Idle)),
            secs(10),
        );
        let codex = Some(AgentKind::Codex);
        assert!(!detect(&mut agent, &mut rt, codex, secs(0), secs(11)));
        assert!(!detect(&mut agent, &mut rt, codex, secs(0), secs(13)));
        assert!(detect(&mut agent, &mut rt, codex, secs(0), secs(14)));
        assert_eq!(agent, None);
    }

    #[test]
    fn each_terminal_has_its_own_opencode_tracker() {
        let json = |raw: &str| serde_json::from_str::<serde_json::Value>(raw).unwrap();
        let (mut a, mut rt_a) = (None, AgentRt::default());
        let (mut b, mut rt_b) = (None, AgentRt::default());
        let sub = r#"{"type":"session.created","properties":{"info":{"id":"x","parentID":"r"}}}"#;
        let idle =
            r#"{"type":"session.status","properties":{"sessionID":"x","status":{"type":"idle"}}}"#;
        assert!(!apply_opencode(&mut a, &mut rt_a, &json(sub), secs(1)));
        // "x" is a subagent in the first terminal only.
        assert!(!apply_opencode(&mut a, &mut rt_a, &json(idle), secs(2)));
        assert!(apply_opencode(&mut b, &mut rt_b, &json(idle), secs(2)));
        assert_eq!(b.map(|s| s.status), Some(AgentStatus::Idle));
        assert_eq!(a, None);
    }
}
