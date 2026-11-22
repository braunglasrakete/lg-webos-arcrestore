//! The restore decision, ported from `app/scripts/watch.sh`.
//!
//! The engine is fed a fresh pair of readings (the configured Sound Out from
//! the settings service and the live route from the audio service) and
//! decides whether to ask for the previous output back. It holds no I/O, so
//! every transition is covered by the tests below.

use std::time::{Duration, Instant};

/// Restore attempts per Bluetooth disconnect, as in watch.sh.
pub const MAX_ATTEMPTS: u32 = 3;
/// Minimum time between restore attempts. watch.sh got this spacing from its
/// 2 s poll; an event-driven watcher has to keep it explicitly, or a burst of
/// events would use up all attempts before the TV applied the first one.
pub const RETRY_GAP: Duration = Duration::from_secs(2);

pub const TV_SPEAKER: &str = "tv_speaker";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    Waiting,
    Bluetooth,
    CheckingRestore,
    Retrying,
    RestoreFailed,
    Restored,
}

impl Detail {
    /// The words app.js looks up in its `details` table.
    pub fn as_str(self) -> &'static str {
        match self {
            Detail::Waiting => "Waiting",
            Detail::Bluetooth => "Bluetooth",
            Detail::CheckingRestore => "CheckingRestore",
            Detail::Retrying => "Retrying",
            Detail::RestoreFailed => "RestoreFailed",
            Detail::Restored => "Restored",
        }
    }
}

/// Same patterns as watch.sh's `is_bluetooth`.
pub fn is_bluetooth(output: &str) -> bool {
    output == "bt_soundbar"
        || output == "tv_speaker_bluetooth"
        || output.starts_with("bluetooth_speaker")
        || output.contains("bluetooth")
}

#[derive(Debug)]
pub struct Engine {
    pub previous: Option<String>,
    pub configured: Option<String>,
    pub live: Option<String>,
    pub detail: Detail,
    bluetooth_seen: bool,
    attempts: u32,
    last_attempt: Option<Instant>,
    /// A restore was requested and its result has not been read yet.
    verify_pending: bool,
}

impl Default for Engine {
    fn default() -> Self {
        Engine {
            previous: None,
            configured: None,
            live: None,
            detail: Detail::Waiting,
            bluetooth_seen: false,
            attempts: 0,
            last_attempt: None,
            verify_pending: false,
        }
    }
}

fn some_bt(value: &Option<String>) -> bool {
    value.as_deref().is_some_and(is_bluetooth)
}

impl Engine {
    /// Takes one pair of readings. Returns the output to restore, if any.
    /// The caller reports the request's outcome with [`Engine::restore_sent`].
    pub fn observe(&mut self, configured: Option<String>, live: Option<String>, now: Instant) -> Option<String> {
        self.configured = configured;
        self.live = live;
        let verifying = std::mem::take(&mut self.verify_pending);
        let output = self.live.clone().or_else(|| self.configured.clone());

        if some_bt(&self.configured) || some_bt(&self.live) {
            if !self.bluetooth_seen {
                self.attempts = 0;
            }
            self.bluetooth_seen = true;
            self.detail = Detail::Bluetooth;
            return None;
        }

        let previous = self.previous.clone();
        let wants_restore = output.as_deref() == Some(TV_SPEAKER)
            && self.bluetooth_seen
            && previous.as_deref().is_some_and(|p| p != TV_SPEAKER)
            && match self.configured.as_deref() {
                None => true,
                Some(c) => c == TV_SPEAKER || Some(c) == previous.as_deref(),
            };

        if wants_restore {
            if self.attempts < MAX_ATTEMPTS {
                if self.last_attempt.is_some_and(|t| now < t + RETRY_GAP) {
                    // Too soon after the last request; the caller checks again
                    // at `next_deadline`.
                    self.verify_pending = verifying;
                    return None;
                }
                self.attempts += 1;
                self.last_attempt = Some(now);
                self.verify_pending = true;
                return previous;
            }
            self.bluetooth_seen = false;
            self.detail = Detail::RestoreFailed;
            return None;
        }

        if let Some(output) = output {
            if self.bluetooth_seen {
                self.detail = if self.attempts > 0 && Some(&output) == previous.as_ref() {
                    Detail::Restored
                } else {
                    Detail::Waiting
                };
            }
            self.bluetooth_seen = false;
            self.previous = Some(output);
        }
        None
    }

    /// Records whether the settings service accepted the restore request.
    pub fn restore_sent(&mut self, accepted: bool) {
        self.detail = if accepted { Detail::CheckingRestore } else { Detail::Retrying };
    }

    /// Bluetooth is (or was last seen) carrying the audio.
    pub fn bluetooth_active(&self) -> bool {
        self.bluetooth_seen
    }

    /// When the readings must be taken again to verify or retry a restore.
    pub fn next_deadline(&self) -> Option<Instant> {
        if self.verify_pending {
            self.last_attempt.map(|t| t + RETRY_GAP)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Option<String> {
        Some(v.to_string())
    }

    struct Clock(Instant);
    impl Clock {
        fn tick(&mut self, ms: u64) -> Instant {
            self.0 += Duration::from_millis(ms);
            self.0
        }
    }

    fn arc_then_bt(e: &mut Engine, c: &mut Clock) {
        assert_eq!(e.observe(s("external_arc"), s("external_arc"), c.tick(0)), None);
        assert_eq!(e.previous.as_deref(), Some("external_arc"));
        assert_eq!(e.observe(s("bt_soundbar"), s("bt_soundbar"), c.tick(2000)), None);
        assert_eq!(e.detail, Detail::Bluetooth);
    }

    #[test]
    fn restores_after_bluetooth_disconnect() {
        let mut c = Clock(Instant::now());
        let mut e = Engine::default();
        arc_then_bt(&mut e, &mut c);
        let restore = e.observe(s("tv_speaker"), s("tv_speaker"), c.tick(2000));
        assert_eq!(restore.as_deref(), Some("external_arc"));
        e.restore_sent(true);
        assert_eq!(e.detail, Detail::CheckingRestore);
        assert!(e.next_deadline().is_some());
        assert_eq!(e.observe(s("external_arc"), s("external_arc"), c.tick(2000)), None);
        assert_eq!(e.detail, Detail::Restored);
        assert_eq!(e.next_deadline(), None);
    }

    #[test]
    fn settings_lagging_behind_live_route_still_restores() {
        let mut c = Clock(Instant::now());
        let mut e = Engine::default();
        arc_then_bt(&mut e, &mut c);
        // Live already fell back, settings still says Bluetooth: keep waiting.
        assert_eq!(e.observe(s("bt_soundbar"), s("tv_speaker"), c.tick(100)), None);
        assert_eq!(e.detail, Detail::Bluetooth);
        // Settings still holds the old ARC choice: that is allowed.
        let r = e.observe(s("external_arc"), s("tv_speaker"), c.tick(100));
        assert_eq!(r.as_deref(), Some("external_arc"));
    }

    #[test]
    fn missing_readings_fall_back_like_the_shell() {
        let mut c = Clock(Instant::now());
        let mut e = Engine::default();
        arc_then_bt(&mut e, &mut c);
        // No live route: the configured value stands in for it.
        let r = e.observe(s("tv_speaker"), None, c.tick(100));
        assert_eq!(r.as_deref(), Some("external_arc"));
    }

    #[test]
    fn burst_of_events_does_not_spend_all_attempts() {
        let mut c = Clock(Instant::now());
        let mut e = Engine::default();
        arc_then_bt(&mut e, &mut c);
        assert!(e.observe(s("tv_speaker"), s("tv_speaker"), c.tick(100)).is_some());
        e.restore_sent(true);
        for _ in 0..5 {
            assert_eq!(e.observe(s("tv_speaker"), s("tv_speaker"), c.tick(200)), None);
        }
        assert_eq!(e.detail, Detail::CheckingRestore);
        assert!(e.observe(s("tv_speaker"), s("tv_speaker"), c.tick(1000)).is_some());
    }

    #[test]
    fn gives_up_after_three_attempts() {
        let mut c = Clock(Instant::now());
        let mut e = Engine::default();
        arc_then_bt(&mut e, &mut c);
        for _ in 0..MAX_ATTEMPTS {
            assert!(e.observe(s("tv_speaker"), s("tv_speaker"), c.tick(2000)).is_some());
            e.restore_sent(false);
            assert_eq!(e.detail, Detail::Retrying);
        }
        assert_eq!(e.observe(s("tv_speaker"), s("tv_speaker"), c.tick(2000)), None);
        assert_eq!(e.detail, Detail::RestoreFailed);
        assert_eq!(e.next_deadline(), None);
        // No restore loop afterwards; TV speaker now becomes "previous".
        assert_eq!(e.observe(s("tv_speaker"), s("tv_speaker"), c.tick(2000)), None);
        assert_eq!(e.previous.as_deref(), Some("tv_speaker"));
        assert_eq!(e.detail, Detail::RestoreFailed);
    }

    #[test]
    fn user_choosing_another_output_is_respected() {
        let mut c = Clock(Instant::now());
        let mut e = Engine::default();
        arc_then_bt(&mut e, &mut c);
        // The user picked optical while the route briefly reads TV speaker.
        assert_eq!(e.observe(s("external_optical"), s("tv_speaker"), c.tick(2000)), None);
        assert_eq!(e.previous.as_deref(), Some("tv_speaker"));
        assert_eq!(e.detail, Detail::Waiting);
        assert_eq!(e.observe(s("external_optical"), s("external_optical"), c.tick(2000)), None);
        assert_eq!(e.previous.as_deref(), Some("external_optical"));
    }

    #[test]
    fn tv_speaker_before_bluetooth_is_never_restored_to() {
        let mut c = Clock(Instant::now());
        let mut e = Engine::default();
        assert_eq!(e.observe(s("tv_speaker"), s("tv_speaker"), c.tick(0)), None);
        assert_eq!(e.observe(s("bt_soundbar"), s("bt_soundbar"), c.tick(2000)), None);
        assert_eq!(e.observe(s("tv_speaker"), s("tv_speaker"), c.tick(2000)), None);
        assert_eq!(e.detail, Detail::Waiting);
    }

    #[test]
    fn nothing_known_before_bluetooth_does_nothing() {
        let mut c = Clock(Instant::now());
        let mut e = Engine::default();
        assert_eq!(e.observe(s("bt_soundbar"), s("bt_soundbar"), c.tick(0)), None);
        assert_eq!(e.observe(s("tv_speaker"), s("tv_speaker"), c.tick(2000)), None);
    }

    #[test]
    fn bluetooth_reconnecting_mid_retry_resets_attempts_later() {
        let mut c = Clock(Instant::now());
        let mut e = Engine::default();
        arc_then_bt(&mut e, &mut c);
        assert!(e.observe(s("tv_speaker"), s("tv_speaker"), c.tick(2000)).is_some());
        e.restore_sent(false);
        // Speaker comes back before the retry: wait for it again.
        assert_eq!(e.observe(s("bt_soundbar"), s("bt_soundbar"), c.tick(2000)), None);
        assert_eq!(e.detail, Detail::Bluetooth);
        // Back on ARC normally, then another Bluetooth session gets three fresh tries.
        assert_eq!(e.observe(s("external_arc"), s("external_arc"), c.tick(2000)), None);
        assert_eq!(e.observe(s("bt_soundbar"), s("bt_soundbar"), c.tick(2000)), None);
        for _ in 0..MAX_ATTEMPTS {
            assert!(e.observe(s("tv_speaker"), s("tv_speaker"), c.tick(2000)).is_some());
            e.restore_sent(false);
        }
    }

    #[test]
    fn no_verify_deadline_once_bluetooth_is_back() {
        let mut c = Clock(Instant::now());
        let mut e = Engine::default();
        arc_then_bt(&mut e, &mut c);
        assert!(e.observe(s("tv_speaker"), s("tv_speaker"), c.tick(2000)).is_some());
        e.restore_sent(true);
        assert!(e.next_deadline().is_some());
        // Speaker reconnects: nothing to verify, so no deadline in the past.
        assert_eq!(e.observe(s("bt_soundbar"), s("bt_soundbar"), c.tick(2500)), None);
        assert_eq!(e.next_deadline(), None);
    }

    #[test]
    fn tv_plus_bluetooth_counts_as_bluetooth() {
        let mut c = Clock(Instant::now());
        let mut e = Engine::default();
        assert_eq!(e.observe(s("external_arc"), s("external_arc"), c.tick(0)), None);
        assert_eq!(e.observe(s("tv_speaker_bluetooth"), s("tv_speaker"), c.tick(2000)), None);
        assert_eq!(e.detail, Detail::Bluetooth);
        assert!(e.observe(s("tv_speaker"), s("tv_speaker"), c.tick(2000)).is_some());
    }

    #[test]
    fn bluetooth_patterns_match_the_shell() {
        for bt in ["bt_soundbar", "tv_speaker_bluetooth", "bluetooth_speaker", "bluetooth_speaker_2", "x_bluetooth_y"] {
            assert!(is_bluetooth(bt), "{bt}");
        }
        for other in ["tv_speaker", "external_arc", "external_optical", "headphone", "lineout"] {
            assert!(!is_bluetooth(other), "{other}");
        }
    }
}
