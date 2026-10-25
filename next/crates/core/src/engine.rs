//! Reminder coordinator with injected clock, input and presentation state.
//! Callers must commit `state` before presenting its effects.
use crate::{
    activity::{Activity, ActivityPolicy, Input, RepeatCycle},
    preferences::{AudioMode, Language, Preferences, VisualMode},
    reminders::{Category, Event, next_group},
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy)]
pub struct Clock {
    pub utc: DateTime<Utc>,
    pub mono: u64,
    pub date: NaiveDate,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NotificationState {
    #[default]
    Allowed,
    Blocked,
    Unknown,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PowerSource {
    #[default]
    Ac,
    Battery,
    Unknown,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Environment {
    pub locked: bool,
    pub fullscreen: bool,
    pub notifications: NotificationState,
    pub session_unknown: bool,
    pub suspended: bool,
    pub power: PowerSource,
}
impl Environment {
    pub fn session_blocked(self) -> bool {
        self.locked || self.session_unknown || self.suspended
    }
    pub fn notifications_blocked(self, allow_during_dnd: bool) -> bool {
        match self.notifications {
            NotificationState::Allowed => false,
            NotificationState::Blocked => !allow_during_dnd,
            NotificationState::Unknown => true,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Notice {
    pub key: String,
    pub cycle: String,
    pub events: Vec<Event>,
    pub created: DateTime<Utc>,
    #[serde(default)]
    pub generation_due: Option<DateTime<Utc>>,
    pub text: String,
    pub followup: u32,
    pub visible: bool,
    pub explicit: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedCycle {
    pub events: Vec<Event>,
    pub started: DateTime<Utc>,
    pub next: DateTime<Utc>,
    pub sent: u32,
    pub maximum: u32,
    pub finished: bool,
    pub hidden: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub format: u32,
    pub handled: BTreeMap<String, DateTime<Utc>>,
    pub skipped: BTreeMap<String, DateTime<Utc>>,
    pub pending: Vec<Notice>,
    pub cycles: Vec<SavedCycle>,
    pub exercise: Option<Event>,
    pub breakfast_dates: BTreeSet<NaiveDate>,
    #[serde(default)]
    pub breakfast_times: BTreeMap<NaiveDate, DateTime<Utc>>,
    pub pause_until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub pause_forever: bool,
    pub pet_position: Option<(i32, i32)>,
    #[serde(default)]
    pub manual_serial: u64,
    #[serde(default)]
    pub checkpoint_utc: Option<DateTime<Utc>>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            format: 1,
            handled: BTreeMap::new(),
            skipped: BTreeMap::new(),
            pending: vec![],
            cycles: vec![],
            exercise: None,
            breakfast_dates: BTreeSet::new(),
            breakfast_times: BTreeMap::new(),
            pause_until: None,
            pause_forever: false,
            pet_position: None,
            manual_serial: 0,
            checkpoint_utc: None,
        }
    }
}
impl State {
    pub fn validate(&self) -> Result<(), String> {
        if self.format != 1
            || self.manual_serial == u64::MAX
            || self.pending.len() > 10_000
            || self.cycles.len() > 10_000
            || self
                .cycles
                .iter()
                .any(|c| c.events.is_empty() || c.sent > c.maximum || c.maximum > 20)
            || self.pending.iter().any(|n| {
                n.events.is_empty()
                    || n.text.len() > 8192
                    || n.generation_due.is_some_and(|due| due > n.created)
            })
        {
            return Err("invalid/newer reminder state; not resetting it".into());
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct Engine {
    pub prefs: Preferences,
    pub state: State,
    /// Transactional outbox: commit these together with state, then clear.
    pub archived_pending: Vec<Notice>,
    scheduled: Vec<Event>,
    activity: Activity,
    repeats: Vec<RepeatCycle>,
    last: Clock,
    environment: Environment,
    was_blocked: bool,
    review_override: Option<String>,
    resumed_from_sleep: bool,
}
#[derive(Clone, Debug, Default)]
pub struct Effect {
    pub announce: Option<Notice>,
    pub stop_audio: bool,
}
impl Engine {
    pub fn new(prefs: Preferences, mut state: State, clock: Clock) -> Result<Self, String> {
        prefs.validate()?;
        state.validate()?;
        let interval = u64::from(prefs.repeat_minutes) * 60_000;
        // Recovery is text-only and limited to five minutes. A long absence is not
        // evidence of rest, and never creates a backlog of repeat audio.
        for notice in &mut state.pending {
            let checkpoint = state.checkpoint_utc.unwrap_or(notice.created);
            notice.visible &= notice.created <= clock.utc
                && checkpoint <= clock.utc
                && clock.utc - checkpoint <= Duration::minutes(5);
        }
        let repeats = state
            .cycles
            .iter_mut()
            .map(|saved| {
                if saved.started > clock.utc || clock.utc - saved.next > Duration::minutes(5) {
                    saved.finished = true;
                }
                let mut cycle = RepeatCycle::new(clock.mono, interval, saved.maximum);
                // A missed repeat is not a fresh notification after a restore
                // or restart. Keep the budget and resume at a future interval.
                if saved.next <= clock.utc && !saved.finished {
                    saved.next = clock.utc + Duration::milliseconds(interval as i64);
                }
                cycle.next = clock
                    .mono
                    .saturating_add((saved.next - clock.utc).num_milliseconds().max(0) as u64);
                cycle.sent = saved.sent;
                cycle.finished = saved.finished;
                cycle.hidden_by_confirmation = saved.hidden;
                cycle
            })
            .collect();
        let mut engine = Self {
            activity: Activity::new(
                clock.mono,
                ActivityPolicy {
                    idle_ms: u64::from(prefs.idle_minutes) * 60_000,
                    allowed_clicks: prefs.allowed_clicks.into(),
                },
            ),
            prefs,
            state,
            archived_pending: vec![],
            repeats,
            scheduled: vec![],
            last: clock,
            environment: Environment::default(),
            was_blocked: false,
            review_override: None,
            resumed_from_sleep: false,
        };
        if !engine.prefs.schedule.exercise_enabled {
            engine.cancel_exercise(clock, false);
        }
        Ok(engine)
    }
    pub fn input(&mut self, at: u64, input: Input) {
        self.activity.input(at, input);
    }
    pub fn suspension(&mut self, now: u64, elapsed_ms: u64) {
        self.activity.observe_suspension(now, elapsed_ms);
        self.resumed_from_sleep = true;
    }
    pub fn schedule(&mut self, events: Vec<Event>, clock: Clock, preserve_past: bool) {
        if preserve_past {
            self.scheduled.retain(|e| e.due <= clock.utc);
            self.scheduled
                .extend(events.into_iter().filter(|e| e.due > clock.utc));
        } else {
            self.scheduled = events;
        }
    }
    fn cancel_exercise(&mut self, clock: Clock, include_manual: bool) {
        let matches = |e: &Event| {
            e.category == Category::Exercise && (include_manual || !e.id.starts_with("manual:"))
        };
        self.state.exercise = None;
        let removed: BTreeSet<_> = self
            .state
            .pending
            .iter()
            .filter(|n| (include_manual || !n.explicit) && n.events.iter().any(matches))
            .map(|n| n.key.clone())
            .collect();
        self.state.pending.retain(|n| !removed.contains(&n.key));
        if self
            .review_override
            .as_ref()
            .is_some_and(|key| removed.contains(key))
        {
            self.review_override = None;
        }
        self.scheduled.retain(|e| !matches(e));
        for i in 0..self.repeats.len() {
            if self.state.cycles[i].events.iter().any(matches) {
                self.repeats[i].finished = true;
                self.sync_cycle(i, clock);
            }
        }
    }
    pub fn preferences(&mut self, prefs: Preferences, clock: Clock) -> Result<(), String> {
        prefs.validate()?;
        if self.prefs.schedule.exercise_enabled && !prefs.schedule.exercise_enabled {
            self.cancel_exercise(clock, true);
        }
        if prefs.schedule.exercise_enabled != self.prefs.schedule.exercise_enabled
            || prefs.schedule.exercise_after_minutes != self.prefs.schedule.exercise_after_minutes
        {
            let anchor = self
                .state
                .breakfast_times
                .get(&clock.date)
                .copied()
                .or_else(|| {
                    self.state
                        .exercise
                        .as_ref()
                        .filter(|e| e.due > clock.utc)
                        .map(|e| {
                            e.due
                                - Duration::minutes(
                                    self.prefs.schedule.exercise_after_minutes.into(),
                                )
                        })
                });
            if let Some(anchor) = anchor {
                self.state.exercise = prefs
                    .schedule
                    .mark_breakfast(anchor)
                    .map_err(|e| e.to_string())?
                    .filter(|e| e.due > clock.utc);
            } else if !prefs.schedule.exercise_enabled {
                self.state.exercise = None;
            }
        }
        if prefs.idle_minutes != self.prefs.idle_minutes
            || prefs.allowed_clicks != self.prefs.allowed_clicks
        {
            self.activity = Activity::new(
                clock.mono,
                ActivityPolicy {
                    idle_ms: u64::from(prefs.idle_minutes) * 60_000,
                    allowed_clicks: prefs.allowed_clicks.into(),
                },
            );
        }
        // Existing series retain their original interval/count; edits govern new ones.
        self.prefs = prefs;
        Ok(())
    }
    pub fn paused(&self, clock: Clock) -> bool {
        self.state.pause_forever
            || self
                .state
                .pause_until
                .is_some_and(|until| until > clock.utc)
    }
    pub fn pause(&mut self, until: Option<DateTime<Utc>>) {
        self.state.pause_forever = false;
        self.state.pause_until = until;
    }
    pub fn breakfast(&mut self, clock: Clock) -> Result<(), String> {
        if self.state.breakfast_dates.insert(clock.date) {
            self.state.breakfast_times.insert(clock.date, clock.utc);
            self.state.exercise = self
                .prefs
                .schedule
                .mark_breakfast(clock.utc)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    pub fn confirm(&mut self, key: &str, clock: Clock) -> Result<(), String> {
        let Some(index) = self.state.pending.iter().position(|n| n.key == key) else {
            return Ok(());
        };
        let notice = self.state.pending.remove(index);
        if self.review_override.as_deref() == Some(key) {
            self.review_override = None;
        }
        if notice
            .events
            .iter()
            .any(|e| e.category == Category::Breakfast)
        {
            self.breakfast(clock)?;
        }
        if let Some(index) = self
            .state
            .cycles
            .iter()
            .position(|c| c.events[0].id == notice.cycle)
        {
            let rest = self.state.cycles[index]
                .events
                .iter()
                .any(|e| matches!(e.category, Category::Break | Category::Exercise));
            self.repeats[index].confirm(clock.mono, self.prefs.trust_confirmation, rest);
            self.sync_cycle(index, clock);
        }
        Ok(())
    }
    pub fn manual_stretch(&mut self, clock: Clock) -> Effect {
        self.state.manual_serial = self.state.manual_serial.saturating_add(1);
        let event = Event {
            id: format!(
                "manual:{}:{}",
                clock.utc.timestamp_millis(),
                self.state.manual_serial
            ),
            due: clock.utc,
            category: Category::Exercise,
            followup: false,
        };
        self.start(vec![event], clock, true)
    }
    pub fn review(&mut self) {
        if self.state.pending.is_empty() {
            return;
        }
        let index = self
            .state
            .pending
            .iter()
            .rposition(|n| !n.visible)
            .unwrap_or_else(|| self.state.pending.len().saturating_sub(2));
        let mut notice = self.state.pending.remove(index);
        notice.visible = true;
        self.review_override = Some(notice.key.clone());
        self.state.pending.push(notice);
    }
    pub fn current(&self, clock: Clock, env: Environment) -> Option<&Notice> {
        if env.session_blocked()
            || (env.fullscreen && self.prefs.reminders.fullscreen_visual == VisualMode::Nothing)
        {
            return None;
        }
        self.state.pending.iter().rev().find(|n| {
            n.visible
                && (!env.notifications_blocked(self.prefs.reminders.allow_during_dnd)
                    || n.explicit
                    || self.review_override.as_ref() == Some(&n.key))
                && (!self.paused(clock)
                    || n.explicit
                    || self.review_override.as_ref() == Some(&n.key))
        })
    }
    pub fn audio_mode(&self, env: Environment) -> AudioMode {
        if env.fullscreen {
            self.prefs.reminders.fullscreen_audio
        } else {
            self.prefs.reminders.audio
        }
    }
    pub fn next_event(&self, clock: Clock) -> Option<&Event> {
        self.scheduled
            .iter()
            .chain(self.state.exercise.iter())
            .filter(|e| {
                e.due > clock.utc
                    && !self.state.handled.contains_key(&e.id)
                    && !self.state.skipped.contains_key(&e.id)
            })
            .min_by_key(|e| e.due)
    }
    pub fn tick(&mut self, clock: Clock, env: Environment, audio_busy: bool) -> Effect {
        let mut effect = Effect::default();
        let notification_blocked = env.notifications_blocked(self.prefs.reminders.allow_during_dnd);
        let blocked = env.session_blocked()
            || notification_blocked
            || self.paused(clock)
            || (env.fullscreen
                && self.prefs.reminders.fullscreen_visual == VisualMode::Nothing
                && self.prefs.reminders.fullscreen_audio == AudioMode::Silent);
        let resumed = clock.mono.saturating_sub(self.last.mono) > 300_000
            || (clock.utc - self.last.utc)
                .num_milliseconds()
                .abs_diff(clock.mono.saturating_sub(self.last.mono) as i64)
                > 300_000;
        let wake = std::mem::take(&mut self.resumed_from_sleep);
        let visual_blocked = env.fullscreen
            && !self.environment.fullscreen
            && self.prefs.reminders.fullscreen_visual == VisualMode::Nothing;
        let just_blocked = (blocked && !self.was_blocked) || visual_blocked;
        effect.stop_audio = (blocked && !self.was_blocked)
            || resumed
            || wake
            || (env.fullscreen != self.environment.fullscreen
                && self.audio_mode(env) != self.audio_mode(self.environment));
        if just_blocked || resumed || wake {
            self.review_override = None;
            for n in &mut self.state.pending {
                if !n.explicit || env.session_blocked() || visual_blocked {
                    n.visible = false;
                }
            }
        }
        if self
            .state
            .pause_until
            .is_some_and(|until| until <= clock.utc)
        {
            self.state.pause_until = None;
        }
        if env.session_blocked() {
            self.activity.uncertain(clock.mono);
        }
        self.activity.idle(clock.mono);
        let mut events = self.scheduled.clone();
        if let Some(event) = &self.state.exercise {
            events.push(event.clone());
        }
        if !blocked && self.was_blocked {
            for i in 0..self.repeats.len() {
                self.repeats[i].next = clock
                    .mono
                    .saturating_add(u64::from(self.prefs.repeat_minutes) * 60_000);
                self.sync_cycle(i, clock);
            }
        }
        if blocked || resumed || wake {
            for event in events.iter().filter(|e| e.due <= clock.utc) {
                if !self.state.handled.contains_key(&event.id) {
                    self.state
                        .skipped
                        .entry(event.id.clone())
                        .or_insert(clock.utc);
                }
            }
            for i in 0..self.repeats.len() {
                if (blocked && !self.was_blocked && !resumed) || wake {
                    self.repeats[i].next = clock
                        .mono
                        .saturating_add(u64::from(self.prefs.repeat_minutes) * 60_000);
                }
                if resumed && self.repeats[i].started != clock.mono {
                    self.repeats[i].finished = true;
                }
                if resumed || wake || !self.was_blocked {
                    self.sync_cycle(i, clock);
                }
            }
        } else if !audio_busy {
            let excluded = self
                .state
                .handled
                .keys()
                .chain(self.state.skipped.keys())
                .cloned()
                .collect();
            let mut group = next_group(&events, &excluded, clock.utc);
            if self.activity.idle(clock.mono) {
                group.retain(|e| {
                    if e.category == Category::Break {
                        self.state.skipped.insert(e.id.clone(), clock.utc);
                        false
                    } else {
                        true
                    }
                });
            }
            if !group.is_empty() {
                let next = self.start(group, clock, false);
                effect.announce = next.announce;
                effect.stop_audio |= next.stop_audio;
            } else {
                for i in 0..self.repeats.len() {
                    let rest = self
                        .activity
                        .rested_since(self.repeats[i].started, clock.mono)
                        .then_some(clock.mono);
                    let due = clock.utc
                        - Duration::milliseconds(
                            clock.mono.saturating_sub(self.repeats[i].next).min(300_000) as i64,
                        );
                    let followup = self.repeats[i].tick(clock.mono, rest);
                    self.sync_cycle(i, clock);
                    if followup {
                        let next = self.followup(i, clock, due);
                        effect.announce = next.announce;
                        effect.stop_audio |= next.stop_audio;
                        break;
                    }
                }
            }
        }
        // End a rest cycle even while another primary event is being delivered.
        for i in 0..self.repeats.len() {
            if self
                .activity
                .rested_since(self.repeats[i].started, clock.mono)
            {
                self.repeats[i].finished = true;
                self.sync_cycle(i, clock);
            }
        }
        self.last = clock;
        self.environment = env;
        self.was_blocked = blocked;
        let cutoff = clock.utc - Duration::days(14);
        self.state.handled.retain(|_, at| *at >= cutoff);
        self.state.skipped.retain(|_, at| *at >= cutoff);
        self.state
            .breakfast_dates
            .retain(|date| *date >= clock.date - chrono::Days::new(14));
        self.state
            .breakfast_times
            .retain(|date, _| *date >= clock.date - chrono::Days::new(14));
        for i in (0..self.repeats.len()).rev() {
            if self.state.cycles[i].finished && self.state.cycles[i].started < cutoff {
                self.state.cycles.remove(i);
                self.repeats.remove(i);
            }
        }
        self.compact_pending();
        effect
    }
    fn compact_pending(&mut self) {
        const ACTIVE: usize = 128;
        if self.state.pending.len() > ACTIVE {
            self.archived_pending.extend(
                self.state
                    .pending
                    .drain(..self.state.pending.len() - ACTIVE),
            );
        }
        let keep: BTreeSet<_> = self
            .state
            .pending
            .iter()
            .map(|n| n.cycle.as_str())
            .collect();
        for i in (0..self.repeats.len()).rev() {
            if self.state.cycles.len() <= ACTIVE * 2 {
                break;
            }
            if self.repeats[i].finished
                && !keep.contains(self.state.cycles[i].events[0].id.as_str())
            {
                self.state.cycles.remove(i);
                self.repeats.remove(i);
            }
        }
    }
    pub fn uncertain_input(&mut self, now: u64) {
        self.activity.uncertain(now);
    }
    fn sync_cycle(&mut self, i: usize, clock: Clock) {
        let live = &self.repeats[i];
        let saved = &mut self.state.cycles[i];
        if !live.finished {
            let next =
                clock.utc + Duration::milliseconds(live.next.saturating_sub(clock.mono) as i64);
            if (next - saved.next).num_milliseconds().abs() > 1000 {
                saved.next = next;
            }
        }
        saved.sent = live.sent;
        saved.finished = live.finished;
        saved.hidden = live.hidden_by_confirmation;
    }
    fn start(&mut self, events: Vec<Event>, clock: Clock, explicit: bool) -> Effect {
        let kind = events[0].category;
        let maximum = if events.iter().any(|e| e.category == Category::Break) {
            self.prefs.reminders.break_repeats
        } else if kind == Category::Exercise {
            self.prefs.reminders.exercise_repeats
        } else {
            self.prefs.reminders.meal_repeats
        };
        for event in &events {
            self.state.handled.insert(event.id.clone(), clock.utc);
        }
        for i in 0..self.state.cycles.len() {
            if self.state.cycles[i]
                .events
                .iter()
                .any(|old| events.iter().any(|new| old.category == new.category))
            {
                self.repeats[i].finished = true;
                self.sync_cycle(i, clock);
            }
        }
        let cycle = RepeatCycle::new(
            clock.mono,
            u64::from(self.prefs.repeat_minutes) * 60_000,
            maximum,
        );
        self.state.cycles.push(SavedCycle {
            events: events.clone(),
            started: clock.utc,
            next: clock.utc + Duration::minutes(self.prefs.repeat_minutes.into()),
            sent: 0,
            maximum,
            finished: cycle.finished,
            hidden: false,
        });
        self.repeats.push(cycle);
        let n = self.make_notice(events, clock, 0, explicit);
        self.state.pending.push(n.clone());
        self.compact_pending();
        Effect {
            announce: Some(n),
            stop_audio: false,
        }
    }
    fn followup(&mut self, i: usize, clock: Clock, due: DateTime<Utc>) -> Effect {
        let events = self.state.cycles[i].events.clone();
        let count = self.repeats[i].sent;
        let explicit = self
            .state
            .pending
            .iter()
            .any(|n| n.cycle == events[0].id && n.explicit);
        self.state.pending.retain(|n| n.cycle != events[0].id);
        let mut n = self.make_notice(events, clock, count, explicit);
        n.generation_due = Some(due);
        self.state.pending.push(n.clone());
        Effect {
            announce: Some(n),
            stop_audio: false,
        }
    }
    fn make_notice(&self, events: Vec<Event>, clock: Clock, count: u32, explicit: bool) -> Notice {
        let category = events[0].category;
        let combined = events.len() > 1;
        let text = builtin(category, combined, count, self.prefs.speech_language);
        let cycle = events[0].id.clone();
        let generation_due = events.iter().map(|e| e.due).min();
        Notice {
            key: format!("{cycle}#{count}"),
            cycle,
            events,
            created: clock.utc,
            generation_due,
            text: text.into(),
            followup: count,
            visible: true,
            explicit,
        }
    }
}
pub fn builtin(
    category: Category,
    combined: bool,
    followup: u32,
    language: Language,
) -> &'static str {
    if followup > 0 {
        return match (language, followup) {
            (Language::Ru, 1) => {
                "Мы уже договорились на перерыв. Или кресло держит тебя в заложниках?"
            }
            (Language::Ru, 2) => {
                "Кнопку нажать получилось. Теперь покажи такой же талант встать и размяться."
            }
            (Language::Ru, _) => {
                "Ещё немного, и придётся меня отключить. Шучу. Но твоему креслу я уже не доверяю."
            }
            (Language::En, 1) => "We agreed on a break. Is the chair holding you hostage?",
            (Language::En, 2) => {
                "You managed to click the button. Now try standing up with the same enthusiasm."
            }
            (Language::En, _) => {
                "Keep this up and you may have to turn me off. Kidding. I still do not trust that chair."
            }
        };
    }
    match (language, category, combined) {
        (Language::Ru, Category::Breakfast, true) => {
            "Завтрак уже заслужил твоё внимание. Отойди от экрана и устрой себе перерыв."
        }
        (Language::Ru, Category::Lunch, true) => {
            "Работа подождёт. Пообедай и дай глазам отдохнуть от экрана."
        }
        (Language::Ru, Category::Dinner, true) => {
            "Ужин и небольшой перерыв звучат как хороший план. Отложи дела."
        }
        (Language::Ru, Category::Breakfast, _) => {
            "Доброе утро. Отложи дела и спокойно позавтракай."
        }
        (Language::Ru, Category::Lunch, _) => {
            "У обеда тоже есть право на твоё внимание. Сделай паузу и поешь."
        }
        (Language::Ru, Category::Dinner, _) => "День был длинным. Теперь можно спокойно поужинать.",
        (Language::Ru, Category::Exercise, _) => {
            "Плечи вниз, спину расправь. Давай встанем и немного разомнёмся."
        }
        (Language::Ru, Category::Break, _) => {
            "Экран никуда не убежит. Встань, пройдись и посмотри вдаль."
        }
        (Language::En, Category::Breakfast, _) => {
            "Breakfast deserves your attention. Step away from the screen and enjoy it."
        }
        (Language::En, Category::Lunch, _) => {
            "Work can wait. Have some lunch and give your eyes a break."
        }
        (Language::En, Category::Dinner, _) => {
            "It has been a long day. Take a break and enjoy dinner."
        }
        (Language::En, Category::Exercise, _) => {
            "Relax your shoulders, straighten your back. Let us stand up and stretch."
        }
        (Language::En, Category::Break, _) => {
            "The screen is not going anywhere. Stand up, walk a little and look into the distance."
        }
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
