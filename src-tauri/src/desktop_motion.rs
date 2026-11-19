//! Clock and inactivity rules shared by all companion windows.
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

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
    pub fn hover(&mut self, label: &str, hovering: bool, now: Instant) {
        self.activity(now);
        if hovering {
            self.hovering.insert(label.into());
        } else {
            self.hovering.remove(label);
        }
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
    fn reduced_motion_stays_still_without_disabling_idle_collapse() {
        let now = Instant::now();
        let mut state = MotionState::new(now);
        assert!(state.tick(now + Duration::from_secs(31), true, false, true));
        assert_eq!(state.elapsed, Duration::ZERO);
    }
}
