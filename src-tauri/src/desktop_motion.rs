//! Clock and inactivity rules shared by all companion windows.
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

    pub fn begin(&mut self, expected: Option<u64>) -> Option<u64> {
        if self.closing.is_some()
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

/// The launcher button occupies a small circle near the sun window's lower
/// right corner. Off-window points must never keep the whole system paused.
pub(crate) fn launcher_hit(x: f64, y: f64, width: f64, height: f64, scale: f64) -> bool {
    if x < 0.0 || y < 0.0 || x > width || y > height {
        return false;
    }
    let dx = x - (width - 24.0 * scale);
    let dy = y - (height - 23.0 * scale);
    dx * dx + dy * dy <= (16.0 * scale).powi(2)
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
    last_activity: Instant,
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
            last_activity: now,
            hovering: HashSet::new(),
        }
    }
    pub fn reset(&mut self, roster: Vec<String>, now: Instant) {
        self.roster = roster;
        self.elapsed = Duration::ZERO;
        self.last_tick = now;
        self.last_activity = now;
        self.hovering.clear();
    }
    pub fn activity(&mut self, now: Instant) {
        self.last_activity = now;
    }
    pub fn resume(&mut self, now: Instant) {
        // Time spent expanding belongs to the transition, not the orbital
        // clock. Start moving from the exact final frame without catching up.
        self.last_tick = now;
        self.last_activity = now;
    }
    pub fn hover(&mut self, label: &str, hovering: bool, now: Instant) {
        // A native hit-test also supplies hover state. Repeated misses must not
        // count as activity, otherwise an empty desktop can never auto-collapse.
        let changed = if hovering {
            self.hovering.insert(label.into())
        } else {
            self.hovering.remove(label)
        };
        if hovering || changed {
            self.activity(now);
        }
    }
    pub fn is_hovered(&self, label: &str) -> bool {
        self.hovering.contains(label)
    }
    pub fn remove(&mut self, label: &str, now: Instant) {
        self.hovering.remove(label);
        self.activity(now);
    }
    /// Open chats keep the entire formation at its current phase. This leaves
    /// the originating planet in a predictable place when a chat folds away.
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
        if chat_open || !self.hovering.is_empty() {
            self.last_activity = now;
        } else if !reduced_motion {
            self.elapsed += delta;
        }
        !chat_open
            && self.hovering.is_empty()
            && now.saturating_duration_since(self.last_activity) >= Duration::from_secs(30)
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
        assert!(state.begin(Some(old)).is_none());
        state.focus("orbit-b", false);
        let revision = state.pending().unwrap();
        let closing = state.begin(Some(revision)).unwrap();
        assert!(state.finish(closing));
        assert!(state.pending().is_none());
    }

    #[test]
    fn outside_click_can_dismiss_without_native_focus_and_inside_click_cancels_it() {
        let mut state = CompanionDismissal::default();
        // Borderless macOS companions need not become the key window.
        assert!(state.pressed(1, None));
        let closing = state.begin(None).unwrap();
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
        let closing = state.begin(None).unwrap();
        assert!(!state.focus("chat-a", false));
        assert!(state.finish(closing));
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
        let closing = state.begin(Some(blur)).unwrap();
        assert!(state.focus("chat-b", true));
        assert!(!state.finish(closing));
        assert!(state.begin(Some(blur)).is_none());
        state.remove("chat-b");
        let next = state.begin(state.pending()).unwrap();
        assert_ne!(next, closing);
    }
    #[test]
    fn launcher_hover_is_bounded_to_its_button_at_any_scale() {
        for scale in [1.0, 1.5, 2.0] {
            let size = 192.0 * scale;
            assert!(launcher_hit(
                168.0 * scale,
                169.0 * scale,
                size,
                size,
                scale
            ));
            assert!(!launcher_hit(
                200.0 * scale,
                200.0 * scale,
                size,
                size,
                scale
            ));
            assert!(!launcher_hit(
                150.0 * scale,
                150.0 * scale,
                size,
                size,
                scale
            ));
            assert!(!launcher_hit(-1.0, 169.0 * scale, size, size, scale));
        }
    }

    #[test]
    fn only_idle_expansions_collapse_and_chats_get_a_fresh_thirty_seconds() {
        let now = Instant::now();
        let mut state = MotionState::new(now);
        assert!(!state.tick(now + Duration::from_secs(29), true, false, false));
        assert!(!state.tick(now + Duration::from_secs(40), true, true, false));
        assert!(!state.tick(now + Duration::from_secs(69), true, false, false));
        assert!(state.tick(now + Duration::from_secs(70), true, false, false));
        assert!(!state.tick(now + Duration::from_secs(100), false, false, false));
    }
    #[test]
    fn hover_pauses_motion_and_interaction_restarts_the_idle_clock() {
        let now = Instant::now();
        let mut state = MotionState::new(now);
        state.tick(now + Duration::from_millis(80), true, false, false);
        let elapsed = state.elapsed;
        state.hover("orbit-a", true, now + Duration::from_secs(1));
        assert!(!state.tick(now + Duration::from_secs(100), true, false, false));
        assert_eq!(state.elapsed, elapsed);
        state.remove("orbit-a", now + Duration::from_secs(101));
        assert!(!state.tick(now + Duration::from_secs(130), true, false, false));
        assert!(state.tick(now + Duration::from_secs(131), true, false, false));
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
    fn repeated_native_pointer_misses_do_not_keep_the_system_awake() {
        let now = Instant::now();
        let mut state = MotionState::new(now);
        for second in 1..=30 {
            state.hover("orbit-a", false, now + Duration::from_secs(second));
        }
        assert!(state.tick(now + Duration::from_secs(30), true, false, false));
    }
    #[test]
    fn leaving_the_hover_target_resumes_without_jumping_ahead() {
        let now = Instant::now();
        let mut state = MotionState::new(now);
        state.tick(now + FRAME_INTERVAL, true, false, false);
        state.hover("orbit-a", true, now + FRAME_INTERVAL);
        let paused = state.elapsed;
        state.tick(now + Duration::from_secs(5), true, false, false);
        assert_eq!(state.elapsed, paused);
        state.hover("orbit-a", false, now + Duration::from_secs(5));
        state.tick(
            now + Duration::from_secs(5) + FRAME_INTERVAL,
            true,
            false,
            false,
        );
        assert_eq!(state.elapsed, paused + FRAME_INTERVAL);
    }
    #[test]
    fn keyboard_focus_survives_native_pointer_misses() {
        let now = Instant::now();
        let mut state = MotionState::new(now);
        state.hover("focus:orbit-a", true, now);
        state.hover("orbit-a", false, now + Duration::from_secs(1));
        assert!(state.is_hovered("focus:orbit-a"));
        assert!(!state.tick(now + Duration::from_secs(40), true, false, false));
        assert_eq!(state.elapsed, Duration::ZERO);
        state.hover("focus:orbit-a", false, now + Duration::from_secs(40));
        assert!(state.tick(now + Duration::from_secs(70), true, false, false));
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
    fn completing_expansion_does_not_jump_ahead_or_shorten_idle_time() {
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
        assert!(!state.tick(now + Duration::from_secs(31), true, false, false));
        assert!(state.tick(now + Duration::from_secs(32), true, false, false));
    }
    #[test]
    fn reduced_motion_stays_still_without_disabling_idle_collapse() {
        let now = Instant::now();
        let mut state = MotionState::new(now);
        assert!(state.tick(now + Duration::from_secs(31), true, false, true));
        assert_eq!(state.elapsed, Duration::ZERO);
    }
}
