//! Counts/times only; no key codes or typed content. All live times are monotonic.
use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct ActivityPolicy {
    pub idle_ms: u64,
    pub allowed_clicks: usize,
}
impl Default for ActivityPolicy {
    fn default() -> Self {
        Self {
            idle_ms: 300_000,
            allowed_clicks: 3,
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub enum Input {
    Keyboard,
    Scroll,
    Click,
    Move { pixels: u32 },
}

#[derive(Clone)]
pub struct Activity {
    policy: ActivityPolicy,
    last_significant: u64,
    last_observed: u64,
    motion_started: u64,
    distance: u32,
    clicks: VecDeque<u64>,
    completed_rest: Option<u64>,
    completed_interval: Option<(u64, u64)>,
}
impl Activity {
    pub fn new(now: u64, policy: ActivityPolicy) -> Self {
        Self {
            policy,
            last_significant: now,
            last_observed: now,
            motion_started: now,
            distance: 0,
            clicks: VecDeque::new(),
            completed_rest: None,
            completed_interval: None,
        }
    }
    fn observe(&mut self, now: u64) -> bool {
        if now < self.last_observed {
            return false;
        }
        self.last_observed = now;
        if now.saturating_sub(self.last_significant) >= self.policy.idle_ms {
            // Record when the idle threshold was crossed, rather than when a
            // later input/tick happened to observe it. Old rest must not finish
            // a new reminder created on returning to the keyboard.
            self.completed_rest = Some(self.last_significant.saturating_add(self.policy.idle_ms));
            self.completed_interval = Some((self.last_significant, now));
        }
        true
    }
    pub fn input(&mut self, now: u64, input: Input) {
        if !self.observe(now) {
            return;
        }
        let significant = match input {
            Input::Keyboard | Input::Scroll => true,
            Input::Click => {
                while self
                    .clicks
                    .front()
                    .is_some_and(|time| now.saturating_sub(*time) >= self.policy.idle_ms)
                {
                    self.clicks.pop_front();
                }
                self.clicks.push_back(now);
                self.clicks.len() > self.policy.allowed_clicks
            }
            Input::Move { pixels } => {
                if now.saturating_sub(self.motion_started) > 3000 {
                    self.motion_started = now;
                    self.distance = 0;
                }
                self.distance = self.distance.saturating_add(pixels);
                if self.distance >= 80 {
                    self.motion_started = now;
                    self.distance = 0;
                    true
                } else {
                    false
                }
            }
        };
        if significant {
            self.last_significant = now;
        }
    }
    pub fn idle(&mut self, now: u64) -> bool {
        self.observe(now) && now.saturating_sub(self.last_significant) >= self.policy.idle_ms
    }
    /// A missing stream resets the unproven idle span. Previously completed
    /// rest (including confirmed sleep) remains valid for its original cycle.
    pub fn uncertain(&mut self, now: u64) {
        if now < self.last_observed {
            return;
        }
        self.last_significant = now;
        self.last_observed = now;
        self.motion_started = now;
        self.distance = 0;
        self.clicks.clear();
    }
    pub fn completed_rest_at(&self) -> Option<u64> {
        self.completed_rest
    }
    /// The qualifying idle span must contain a full rest period after this
    /// reminder began. This also retains proof when activity has resumed.
    pub fn rested_since(&mut self, started: u64, now: u64) -> bool {
        self.observe(now)
            && self.completed_interval.is_some_and(|(from, to)| {
                to <= now && to.saturating_sub(from.max(started)) >= self.policy.idle_ms
            })
    }
    /// Called only with a confirmed system suspension interval, never just
    /// because another session's keyboard state became unavailable.
    pub fn observe_suspension(&mut self, resumed: u64, duration_ms: u64) {
        if self.observe(resumed) && duration_ms >= self.policy.idle_ms {
            self.completed_rest = Some(resumed);
            self.completed_interval = Some((resumed.saturating_sub(duration_ms), resumed));
        }
    }
}

#[derive(Debug, Clone)]
pub struct RepeatCycle {
    pub started: u64,
    pub next: u64,
    pub sent: u32,
    pub finished: bool,
    pub hidden_by_confirmation: bool,
    interval_ms: u64,
    maximum: u32,
}
impl RepeatCycle {
    pub fn new(now: u64, interval_ms: u64, maximum: u32) -> Self {
        Self {
            started: now,
            next: now.saturating_add(interval_ms),
            sent: 0,
            finished: maximum == 0,
            hidden_by_confirmation: false,
            interval_ms,
            maximum,
        }
    }
    pub fn confirm(&mut self, now: u64, trust: bool, is_break_or_stretch: bool) {
        self.hidden_by_confirmation = true;
        if trust || !is_break_or_stretch {
            self.finished = true;
        } else {
            self.next = now.saturating_add(self.interval_ms);
        }
    }
    /// Returns one follow-up. Never emit a catch-up burst after a delayed tick.
    /// `finished` ends repeats, not the separate pending confirmation message.
    pub fn tick(&mut self, now: u64, completed_rest: Option<u64>) -> bool {
        if self.finished {
            return false;
        }
        if completed_rest.is_some_and(|rest| rest > self.started && rest <= now) {
            self.finished = true;
            return false;
        }
        if now < self.next {
            return false;
        }
        self.sent += 1;
        self.hidden_by_confirmation = false;
        self.next = now.saturating_add(self.interval_ms);
        if self.sent >= self.maximum {
            self.finished = true;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incidental_clicks_do_not_cancel_rest_but_active_work_does() {
        let mut activity = Activity::new(0, ActivityPolicy::default());
        for time in [1, 2, 3] {
            activity.input(time, Input::Click);
        }
        assert!(activity.idle(300_000));
        activity.input(300_001, Input::Keyboard);
        assert!(!activity.idle(300_002));
        assert_eq!(activity.completed_rest_at(), Some(300_000));
    }
    #[test]
    fn fourth_click_and_significant_motion_count_as_activity() {
        let mut activity = Activity::new(0, ActivityPolicy::default());
        for time in [1, 2, 3, 4] {
            activity.input(time, Input::Click);
        }
        assert!(!activity.idle(300_000));
        activity.input(300_001, Input::Move { pixels: 80 });
        assert!(!activity.idle(300_002));
    }
    #[test]
    fn confirmation_does_not_end_untrusted_break_and_no_catch_up_burst_occurs() {
        let mut cycle = RepeatCycle::new(0, 300_000, 3);
        cycle.confirm(1, false, true);
        assert!(!cycle.finished);
        assert!(cycle.tick(900_000, None));
        assert_eq!(cycle.sent, 1);
        assert!(!cycle.tick(900_000, None));
        assert!(cycle.tick(1_200_000, None));
        assert!(cycle.tick(1_500_000, None));
        assert!(cycle.finished);
        assert_eq!(cycle.sent, 3);
    }
    #[test]
    fn completed_rest_ends_cycle_even_when_keyboard_is_active_again() {
        let mut activity = Activity::new(0, ActivityPolicy::default());
        activity.input(300_000, Input::Keyboard);
        let mut cycle = RepeatCycle::new(0, 300_000, 3);
        assert!(!cycle.tick(300_001, activity.completed_rest_at()));
        assert!(cycle.finished);
    }
}
