//! Orbital clock and chat-dismissal rules shared by companion windows.
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

/// Match a display refresh without queuing old geometry behind the UI thread.
pub(crate) const FRAME_INTERVAL: Duration = Duration::from_nanos(16_666_667);
const TRANSITION_DURATION: Duration = Duration::from_millis(320);

/// Focus belongs to the whole companion group, not to one popup. In particular,
/// moving from one Sprite/chat to another must not look like a click-away.
#[derive(Default)]
pub(crate) struct CompanionDismissal {
    focused: HashSet<String>,
    armed: bool,
    revision: u64,
    closing: Option<u64>,
    buttons: u64,
    clicks: Option<u64>,
}

impl CompanionDismissal {
    pub fn focus(&mut self, label: &str, focused: bool) -> bool {
        if focused {
            let cancelled = self.cancel();
            self.focused.insert(label.into());
            self.armed = true;
            cancelled
        } else {
            self.focused.remove(label);
            // A native blur often arrives after the global outside-click
            // sample. It must not undo the close that sample just requested.
            if self.closing.is_none() {
                self.revision = self.revision.wrapping_add(1);
            }
            false
        }
    }

    pub fn remove(&mut self, label: &str) -> bool {
        self.focused.remove(label);
        self.cancel()
    }

    pub fn pending(&self) -> Option<u64> {
        (self.armed && self.focused.is_empty() && self.closing.is_none()).then_some(self.revision)
    }

    pub fn pressed(&mut self, buttons: u64, clicks: Option<u64>) -> bool {
        let pressed = buttons & !self.buttons != 0
            || self
                .clicks
                .zip(clicks)
                .is_some_and(|(before, after)| before != after);
        self.buttons = buttons;
        self.clicks = clicks;
        pressed
    }

    pub fn begin(&mut self, expected: Option<u64>, has_chats: bool) -> Option<u64> {
        if !has_chats
            || self.closing.is_some()
            || expected.is_some_and(|revision| self.pending() != Some(revision))
        {
            return None;
        }
        self.revision = self.revision.wrapping_add(1);
        self.closing = Some(self.revision);
        Some(self.revision)
    }

    pub fn cancel(&mut self) -> bool {
        self.revision = self.revision.wrapping_add(1);
        self.closing.take().is_some()
    }

    pub fn finish(&mut self, revision: u64) -> bool {
        if self.closing != Some(revision) {
            return false;
        }
        self.closing = None;
        self.armed = false;
        true
    }
}

pub(crate) fn transition_progress(elapsed: Duration, reduced_motion: bool) -> f64 {
    if reduced_motion {
        return 1.0;
    }
    let linear = (elapsed.as_secs_f64() / TRANSITION_DURATION.as_secs_f64()).clamp(0.0, 1.0);
    1.0 - (1.0 - linear).powi(3)
}

pub(crate) struct MotionState {
    pub roster: Vec<String>,
    pub elapsed: Duration,
    last_tick: Instant,
    hovering: HashSet<String>,
}
impl Default for MotionState {
    fn default() -> Self {
        Self::new(Instant::now())
    }
}
impl MotionState {
    fn new(now: Instant) -> Self {
        Self {
            roster: Vec::new(),
            elapsed: Duration::ZERO,
            last_tick: now,
            hovering: HashSet::new(),
        }
    }
    pub fn reset(&mut self, roster: Vec<String>, now: Instant) {
        self.roster = roster;
        self.elapsed = Duration::ZERO;
        self.last_tick = now;
        self.hovering.clear();
    }
    pub fn activity(&mut self, now: Instant) {
        self.last_tick = now;
    }
    pub fn resume(&mut self, now: Instant) {
        // Time spent expanding belongs to the transition, not the orbital
        // clock. Start moving from the exact final frame without catching up.
        self.last_tick = now;
    }
    pub fn hover(&mut self, label: &str, hovering: bool, now: Instant) {
        // Repeated misses must not reset the clock while the system is orbiting.
        let changed = if hovering {
            self.hovering.insert(label.into())
        } else {
            self.hovering.remove(label)
        };
        if Self::pauses_orbits(label) && (hovering || changed) {
            self.activity(now);
        }
    }
    /// Native window focus is authoritative. A newly shown webview can retain
    /// DOM focus even while its companion window is deliberately nonfocusable.
    pub fn focus(&mut self, label: &str, focused: bool, now: Instant) {
        self.hover(&format!("focus:{label}"), focused, now);
    }
    fn pauses_orbits(label: &str) -> bool {
        label
            .strip_prefix("focus:")
            .unwrap_or(label)
            .starts_with("orbit-")
    }
    pub fn is_hovered(&self, label: &str) -> bool {
        self.hovering.contains(label)
    }
    pub fn remove(&mut self, label: &str, now: Instant) {
        self.hovering.remove(label);
        self.hovering.remove(&format!("focus:{label}"));
        self.activity(now);
    }
    /// Open chats keep the entire formation at its current phase. This leaves
    /// the originating planet in a predictable place when a chat folds away.
    /// Returns whether orbital positions advanced; it never requests collapse.
    pub fn tick(
        &mut self,
        now: Instant,
        expanded: bool,
        chat_open: bool,
        reduced_motion: bool,
    ) -> bool {
        let delta = now
            .saturating_duration_since(self.last_tick)
            .min(Duration::from_millis(250));
        self.last_tick = now;
        if !expanded {
            return false;
        }
        let selecting_sprite = self.hovering.iter().any(|label| Self::pauses_orbits(label));
        if chat_open || selecting_sprite || reduced_motion {
            return false;
        }
        self.elapsed += delta;
        delta > Duration::ZERO
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_can_move_between_companions_without_dismissing_the_group() {
        let mut state = CompanionDismissal::default();
        assert!(state.pending().is_none());
        state.focus("chat-a", true);
        state.focus("chat-a", false);
        let old = state.pending().unwrap();
        state.focus("orbit-b", true);
        assert!(state.pending().is_none());
        assert!(state.begin(Some(old), true).is_none());
        state.focus("orbit-b", false);
        let revision = state.pending().unwrap();
        let closing = state.begin(Some(revision), true).unwrap();
        assert!(state.finish(closing));
        assert!(state.pending().is_none());
    }

    #[test]
    fn outside_click_can_dismiss_without_native_focus_and_inside_click_cancels_it() {
        let mut state = CompanionDismissal::default();
        // Borderless macOS companions need not become the key window.
        assert!(state.pressed(1, None));
        let closing = state.begin(None, true).unwrap();
        assert!(!state.pressed(1, None));
        assert!(!state.pressed(0, None));
        assert!(state.pressed(1, None));
        assert!(state.cancel());
        assert!(!state.finish(closing));
    }

    #[test]
    fn native_blur_after_an_outside_click_does_not_cancel_dismissal() {
        let mut state = CompanionDismissal::default();
        state.focus("chat-a", true);
        let closing = state.begin(None, true).unwrap();
        assert!(!state.focus("chat-a", false));
        assert!(state.finish(closing));
    }

    #[test]
    fn click_away_returns_chats_without_ever_gathering_the_formation() {
        let mut state = CompanionDismissal::default();
        state.focus("chat-a", true);
        assert!(state.pressed(1, Some(1)));
        let returning = state.begin(None, true).unwrap();
        // The native blur and another pointer sample during the fold cannot
        // start a second transition that hides the returning Sprite.
        state.focus("chat-a", false);
        assert!(state.begin(state.pending(), false).is_none());
        assert!(state.finish(returning));
        state.remove("chat-a");
        assert!(state.pending().is_none());

        // Later outside clicks with no chat have no dismissal transition.
        assert!(!state.pressed(0, Some(1)));
        assert!(state.pressed(1, Some(2)));
        assert!(state.begin(None, false).is_none());
    }

    #[test]
    fn focus_loss_with_a_chat_keeps_sprites_expanded() {
        let mut state = CompanionDismissal::default();
        state.focus("chat-a", true);
        state.focus("chat-a", false);
        let returning = state.begin(state.pending(), true).unwrap();
        assert!(state.finish(returning));
        // Explicit sun contraction closes chats without requesting a second
        // contraction from the dismissal controller.
        let sun = state.begin(None, true).unwrap();
        assert!(state.finish(sun));
    }

    #[test]
    fn restored_sprite_blur_does_not_gather_or_block_the_next_pointer_click() {
        let mut state = CompanionDismissal::default();
        state.focus("chat-a", true);
        state.focus("chat-a", false);
        let returning = state.begin(state.pending(), true).unwrap();
        assert!(state.finish(returning));
        state.remove("chat-a");

        // macOS may briefly focus the restored Sprite, then transfer to the
        // application clicked behind it. That late blur has no chat to close.
        state.focus("orbit-a", true);
        state.focus("orbit-a", false);
        assert!(state.begin(state.pending(), false).is_none());

        // A fresh chat still opens and closes normally after ignored clicks.
        state.focus("chat-b", true);
        state.focus("chat-b", false);
        let next = state.begin(state.pending(), true).unwrap();
        assert!(state.finish(next));
    }

    #[test]
    fn short_trackpad_taps_are_detected_between_pointer_samples() {
        let mut state = CompanionDismissal::default();
        assert!(!state.pressed(0, Some(10)));
        assert!(state.pressed(0, Some(11)));
        assert!(!state.pressed(0, Some(11)));
        assert!(state.pressed(1, Some(12)));
        assert!(!state.pressed(1, Some(12)));
    }

    #[test]
    fn stale_blur_and_closing_callbacks_cannot_dismiss_a_reopened_chat() {
        let mut state = CompanionDismissal::default();
        state.focus("chat-a", true);
        state.focus("chat-a", false);
        let blur = state.pending().unwrap();
        let closing = state.begin(Some(blur), true).unwrap();
        assert!(state.focus("chat-b", true));
        assert!(!state.finish(closing));
        assert!(state.begin(Some(blur), true).is_none());
        state.remove("chat-b");
        let next = state.begin(state.pending(), true).unwrap();
        assert_ne!(next, closing);
    }
    #[test]
    fn long_idle_expansions_keep_orbiting_until_explicitly_collapsed() {
        let now = Instant::now();
        let mut state = MotionState::new(now);
        for seconds in [30, 60, 3600, 86400] {
            let elapsed = state.elapsed;
            assert!(state.tick(now + Duration::from_secs(seconds), true, false, false));
            assert!(state.elapsed > elapsed);
        }
        let elapsed = state.elapsed;
        assert!(!state.tick(now + Duration::from_secs(86401), false, false, false));
        assert_eq!(state.elapsed, elapsed);
    }

    #[test]
    fn hover_pauses_motion_and_leaving_resumes_without_catching_up() {
        let now = Instant::now();
        let mut state = MotionState::new(now);
        state.tick(now + FRAME_INTERVAL, true, false, false);
        let elapsed = state.elapsed;
        state.hover("orbit-a", true, now + FRAME_INTERVAL);
        assert!(!state.tick(now + Duration::from_secs(100), true, false, false));
        assert_eq!(state.elapsed, elapsed);
        state.remove("orbit-a", now + Duration::from_secs(101));
        assert!(state.tick(
            now + Duration::from_secs(101) + FRAME_INTERVAL,
            true,
            false,
            false
        ));
        assert_eq!(state.elapsed, elapsed + FRAME_INTERVAL);
    }

    #[test]
    fn hovering_the_sun_after_expansion_keeps_the_planets_orbiting() {
        let now = Instant::now();
        let mut state = MotionState::new(now);
        state.hover("focus:sun", true, now);
        for frame in 1..=120 {
            let time = now + FRAME_INTERVAL * frame;
            state.hover("sun", true, time);
            assert!(state.tick(time, true, false, false));
        }
        assert_eq!(state.elapsed, FRAME_INTERVAL * 120);
        assert!(state.is_hovered("sun"));
        state.hover("orbit-a", true, now + FRAME_INTERVAL * 121);
        assert!(!state.tick(now + FRAME_INTERVAL * 122, true, false, false));
        assert_eq!(state.elapsed, FRAME_INTERVAL * 120);
    }

    #[test]
    fn a_new_expansion_restarts_deterministically_with_a_fresh_roster() {
        let now = Instant::now();
        let mut state = MotionState::new(now);
        state.hover("old", true, now);
        state.reset(vec!["first".into(), "second".into()], now);
        assert_eq!(state.roster, vec!["first", "second"]);
        assert_eq!(state.elapsed, Duration::ZERO);
        assert!(state.tick(now + Duration::from_secs(31), true, false, false));
    }

    #[test]
    fn repeated_native_pointer_misses_do_not_pause_orbits() {
        let now = Instant::now();
        let mut state = MotionState::new(now);
        for second in 1..=60 {
            let time = now + Duration::from_secs(second);
            state.hover("orbit-a", false, time);
            assert!(state.tick(time, true, false, false));
        }
        assert!(state.elapsed > Duration::ZERO);
    }

    #[test]
    fn keyboard_focus_survives_native_pointer_misses() {
        let now = Instant::now();
        let mut state = MotionState::new(now);
        state.focus("orbit-a", true, now);
        state.hover("orbit-a", false, now + Duration::from_secs(1));
        assert!(state.is_hovered("focus:orbit-a"));
        assert!(!state.tick(now + Duration::from_secs(40), true, false, false));
        assert_eq!(state.elapsed, Duration::ZERO);
        state.focus("orbit-a", false, now + Duration::from_secs(40));
        assert!(state.tick(
            now + Duration::from_secs(40) + FRAME_INTERVAL,
            true,
            false,
            false
        ));
    }

    #[test]
    fn first_expansion_moves_after_stale_webview_focus_is_reconciled() {
        let now = Instant::now();
        let mut state = MotionState::new(now);
        state.reset(vec!["a".into()], now);
        // Fresh WKWebViews can report DOM focus while loading. This must not
        // require a second expansion to clear a focus token and start motion.
        state.hover("focus:orbit-a", true, now);
        let shown = now + TRANSITION_DURATION;
        state.resume(shown);
        state.focus("orbit-a", false, shown);
        assert!(!state.is_hovered("focus:orbit-a"));
        for frame in 1..=120 {
            let time = shown + FRAME_INTERVAL * frame;
            state.hover("orbit-a", false, time);
            state.focus("orbit-a", false, time);
            assert!(state.tick(time, true, false, false));
        }
        assert_eq!(state.roster, vec!["a"]);
        assert_eq!(state.elapsed, FRAME_INTERVAL * 120);
    }

    #[test]
    fn unfocused_native_window_still_pauses_when_pointer_is_over_sprite() {
        let now = Instant::now();
        let mut state = MotionState::new(now);
        state.reset(vec!["a".into()], now);
        state.hover("orbit-a", true, now);
        state.focus("orbit-a", false, now);
        assert!(!state.tick(now + FRAME_INTERVAL, true, false, false));
        assert!(state.is_hovered("orbit-a"));
        let leaving = now + FRAME_INTERVAL;
        state.hover("orbit-a", false, leaving);
        state.focus("orbit-a", false, leaving);
        assert!(state.tick(leaving + FRAME_INTERVAL, true, false, false));
        assert_eq!(state.elapsed, FRAME_INTERVAL);
    }

    #[test]
    fn returning_chat_clears_stale_focus_and_resumes_orbits() {
        let now = Instant::now();
        let mut state = MotionState::new(now);
        state.hover("chat-a", true, now);
        state.hover("focus:chat-a", true, now);
        assert!(!state.tick(now + Duration::from_secs(100), true, true, false));
        state.remove("chat-a", now + Duration::from_secs(100));
        assert!(!state.is_hovered("chat-a"));
        assert!(!state.is_hovered("focus:chat-a"));
        assert!(state.tick(
            now + Duration::from_secs(100) + FRAME_INTERVAL,
            true,
            false,
            false
        ));
        assert_eq!(state.elapsed, FRAME_INTERVAL);
    }

    #[test]
    fn transitions_follow_elapsed_time_and_finish_exactly() {
        assert_eq!(transition_progress(Duration::ZERO, false), 0.0);
        assert_eq!(transition_progress(TRANSITION_DURATION, false), 1.0);
        assert_eq!(transition_progress(TRANSITION_DURATION * 2, false), 1.0);
        assert_eq!(transition_progress(Duration::ZERO, true), 1.0);
        let half = transition_progress(TRANSITION_DURATION / 2, false);
        assert!(half > 0.5 && half < 1.0);
    }

    #[test]
    fn completing_expansion_does_not_jump_ahead() {
        let now = Instant::now();
        let mut state = MotionState::new(now);
        state.resume(now + Duration::from_secs(2));
        state.tick(
            now + Duration::from_secs(2) + FRAME_INTERVAL,
            true,
            false,
            false,
        );
        assert_eq!(state.elapsed, FRAME_INTERVAL);
    }

    #[test]
    fn reduced_motion_stays_still_even_after_long_idle_time() {
        let now = Instant::now();
        let mut state = MotionState::new(now);
        assert!(!state.tick(now + Duration::from_secs(86400), true, false, true));
        assert_eq!(state.elapsed, Duration::ZERO);
    }
}
