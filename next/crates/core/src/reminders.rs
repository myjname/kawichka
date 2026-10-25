//! Clock-injected queue and delivery rules from D02/D03/D04/D09/D10.
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Breakfast,
    Lunch,
    Dinner,
    Exercise,
    Break,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub category: Category,
    pub due: DateTime<Utc>,
    pub followup: bool,
}

impl Event {
    fn priority(&self) -> (bool, u8, DateTime<Utc>, &str) {
        (
            self.followup,
            match self.category {
                Category::Breakfast | Category::Lunch | Category::Dinner => 0,
                Category::Exercise => 1,
                Category::Break => 2,
            },
            self.due,
            &self.id,
        )
    }
    pub fn fresh(&self, now: DateTime<Utc>) -> bool {
        self.due <= now && now - self.due <= Duration::minutes(5)
    }
}

/// One dispatch may contain meal+break. Other pairs remain separate until an
/// explicitly agreed compatibility rule exists. IDs/confirmation stay separate.
pub fn next_group(events: &[Event], handled: &BTreeSet<String>, now: DateTime<Utc>) -> Vec<Event> {
    let mut waiting: Vec<_> = events
        .iter()
        .filter(|event| event.fresh(now) && !handled.contains(&event.id))
        .cloned()
        .collect();
    waiting.sort_by(|a, b| a.priority().cmp(&b.priority()));
    let Some(first) = waiting.first().cloned() else {
        return Vec::new();
    };
    let mut result = vec![first.clone()];
    if !first.followup
        && matches!(
            first.category,
            Category::Breakfast | Category::Lunch | Category::Dinner
        )
        && let Some(pause) = waiting.into_iter().find(|event| {
            !event.followup && event.category == Category::Break && event.due == first.due
        })
    {
        result.push(pause);
    }
    result
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeDeadline {
    pub due: DateTime<Utc>,
    pub max_wait_seconds: u32,
}
impl RuntimeDeadline {
    pub fn new(due: DateTime<Utc>) -> Self {
        Self {
            due,
            max_wait_seconds: 10,
        }
    }
    pub fn fallback_required(&self, now: DateTime<Utc>) -> bool {
        now >= self.due + Duration::seconds(self.max_wait_seconds.into())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeliveryState {
    pub event_ids: Vec<String>,
    pub checkpoint: DateTime<Utc>,
    pub shown: bool,
    pub confirmed: bool,
    pub audio_started: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovery {
    None,
    QuietMessage,
}
impl DeliveryState {
    /// Recovery never requests sound: physical playback cannot be transactional
    /// with SQLite. A visible pending message does not expire during normal use.
    pub fn recover(&self, now: DateTime<Utc>) -> Recovery {
        if !self.confirmed
            && self.shown
            && self.checkpoint <= now
            && now - self.checkpoint <= Duration::minutes(5)
        {
            Recovery::QuietMessage
        } else {
            Recovery::None
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PresentationPolicy {
    pub paused: bool,
    pub locked: bool,
    pub do_not_disturb: bool,
    pub allow_during_do_not_disturb: bool,
}
impl PresentationPolicy {
    pub fn permitted(self) -> bool {
        !self.paused && !self.locked && (!self.do_not_disturb || self.allow_during_do_not_disturb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn now() -> DateTime<Utc> {
        "2026-10-23T10:00:00Z".parse().unwrap()
    }
    fn event(id: &str, category: Category, followup: bool, ago: i64) -> Event {
        Event {
            id: id.into(),
            category,
            followup,
            due: now() - Duration::seconds(ago),
        }
    }
    #[test]
    fn primary_events_outrank_repeats_and_compatible_meal_break_keep_both_ids() {
        let events = vec![
            event("repeat", Category::Dinner, true, 30),
            event("stretch", Category::Exercise, false, 0),
            event("pause", Category::Break, false, 0),
            event("meal", Category::Lunch, false, 0),
        ];
        let group = next_group(&events, &BTreeSet::new(), now());
        assert_eq!(
            group.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["meal", "pause"]
        );
        let handled = group.into_iter().map(|e| e.id).collect();
        assert_eq!(next_group(&events, &handled, now())[0].id, "stretch");
    }
    #[test]
    fn old_or_handled_events_do_not_reappear_but_five_minute_boundary_is_inclusive() {
        let events = vec![
            event("old", Category::Lunch, false, 301),
            event("edge", Category::Break, false, 300),
        ];
        assert_eq!(next_group(&events, &BTreeSet::new(), now())[0].id, "edge");
        assert!(next_group(&events, &BTreeSet::from(["edge".into()]), now()).is_empty());
    }
    #[test]
    fn runtime_deadline_does_not_reset_for_retry() {
        let deadline = RuntimeDeadline::new(now());
        assert!(!deadline.fallback_required(now() + Duration::seconds(9)));
        assert!(deadline.fallback_required(now() + Duration::seconds(10)));
        assert!(deadline.fallback_required(now() + Duration::minutes(1)));
    }
    #[test]
    fn recovery_is_quiet_fresh_and_not_confirmed() {
        let mut state = DeliveryState {
            event_ids: vec!["meal".into()],
            checkpoint: now(),
            shown: true,
            confirmed: false,
            audio_started: true,
        };
        assert_eq!(
            state.recover(now() + Duration::minutes(5)),
            Recovery::QuietMessage
        );
        assert_eq!(state.recover(now() + Duration::minutes(6)), Recovery::None);
        state.confirmed = true;
        assert_eq!(state.recover(now()), Recovery::None);
    }
    #[test]
    fn dnd_exception_does_not_override_lock_or_pause() {
        let mut policy = PresentationPolicy {
            do_not_disturb: true,
            ..Default::default()
        };
        assert!(!policy.permitted());
        policy.allow_during_do_not_disturb = true;
        assert!(policy.permitted());
        policy.locked = true;
        assert!(!policy.permitted());
    }
}
