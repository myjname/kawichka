use super::*;
#[test]
fn disabling_exercise_ends_pending_and_repeats_but_manual_still_works() {
    let mut e = engine();
    e.schedule(
        vec![event(Category::Exercise, "auto-exercise", 0)],
        clock(0),
        false,
    );
    let first = tick(&mut e, 0).announce.unwrap();
    e.confirm(&first.key, clock(1)).unwrap();
    let mut p = e.prefs.clone();
    p.schedule.exercise_enabled = false;
    e.preferences(p, clock(2)).unwrap();
    assert!(e.state.pending.is_empty());
    assert!(e.state.cycles[0].finished);
    assert!(tick(&mut e, 62).announce.is_none());
    assert!(e.manual_stretch(clock(63)).announce.is_some());
    let manual = Engine::new(e.prefs.clone(), e.state.clone(), clock(64)).unwrap();
    assert_eq!(manual.state.pending.len(), 1);
    let mut stale = engine();
    stale.schedule(
        vec![event(Category::Exercise, "exercise:auto", 0)],
        clock(0),
        false,
    );
    tick(&mut stale, 0);
    stale.prefs.schedule.exercise_enabled = false;
    let resumed = Engine::new(stale.prefs, stale.state, clock(1)).unwrap();
    assert!(resumed.state.pending.is_empty());
    assert!(resumed.state.cycles[0].finished);
}
#[test]
fn large_unconfirmed_history_is_an_outbox_instead_of_an_invalid_active_state() {
    let mut e = engine();
    for i in 0..10001 {
        e.manual_stretch(clock(i));
    }
    e.state.validate().unwrap();
    assert_eq!(e.state.pending.len(), 128);
    assert_eq!(e.archived_pending.len(), 10001 - 128);
    assert!(e.state.cycles.len() <= 256);
    assert_eq!(e.archived_pending[0].text, e.state.pending[0].text);
}
#[test]
fn queued_primary_and_followup_keep_their_original_runtime_deadlines() {
    let mut e = engine();
    e.schedule(
        vec![event(Category::Exercise, "queued", 0)],
        clock(0),
        false,
    );
    assert!(
        e.tick(clock(0), Environment::default(), true)
            .announce
            .is_none()
    );
    let primary = e
        .tick(clock(20), Environment::default(), false)
        .announce
        .unwrap();
    assert_eq!(primary.generation_due, Some(clock(0).utc));
    assert_eq!(primary.created, clock(20).utc);
    assert!(
        e.tick(clock(80), Environment::default(), true)
            .announce
            .is_none()
    );
    let followup = e
        .tick(clock(90), Environment::default(), false)
        .announce
        .unwrap();
    assert_eq!(followup.generation_due, Some(clock(80).utc));
    assert_eq!(followup.created, clock(90).utc);
}
#[test]
fn backward_clock_jump_ends_old_cycles_but_keeps_a_new_explicit_request() {
    let mut e = engine();
    e.schedule(vec![event(Category::Break, "old", 0)], clock(0), false);
    e.tick(clock(0), Environment::default(), false);
    let mut back = clock(1);
    back.utc -= Duration::hours(1);
    back.date = back.utc.date_naive();
    e.manual_stretch(back);
    e.tick(back, Environment::default(), false);
    assert!(e.state.cycles[0].finished);
    assert!(!e.state.cycles[1].finished);
}
fn clock(seconds: i64) -> Clock {
    let utc = "2026-10-24T09:00:00Z".parse::<DateTime<Utc>>().unwrap() + Duration::seconds(seconds);
    Clock {
        utc,
        mono: 1_000_000 + (seconds as u64) * 1000,
        date: utc.date_naive(),
    }
}
fn engine() -> Engine {
    let p = Preferences {
        repeat_minutes: 1,
        ..Default::default()
    };
    Engine::new(p, State::default(), clock(0)).unwrap()
}
fn event(category: Category, id: &str, seconds: i64) -> Event {
    Event {
        id: id.into(),
        category,
        due: clock(seconds).utc,
        followup: false,
    }
}
fn tick(e: &mut Engine, s: i64) -> Effect {
    e.tick(clock(s), Environment::default(), false)
}
#[test]
fn combines_meal_break_and_confirmation_arms_weekend_exercise_without_trusting_rest() {
    let mut e = engine();
    e.prefs.schedule.exercise_after_minutes = 1;
    e.schedule(
        vec![
            event(Category::Break, "break", 0),
            event(Category::Breakfast, "meal", 0),
        ],
        clock(0),
        false,
    );
    let first = tick(&mut e, 0).announce.unwrap();
    assert_eq!(first.events.len(), 2);
    e.confirm(&first.key, clock(1)).unwrap();
    assert!(e.state.pending.is_empty());
    assert!(!e.state.cycles[0].finished);
    assert_eq!(e.state.exercise.as_ref().unwrap().due, clock(61).utc);
    assert_eq!(e.state.handled.len(), 2);
    e.breakfast(clock(20)).unwrap();
    assert_eq!(e.state.exercise.as_ref().unwrap().due, clock(61).utc);
    e.input(clock(61).mono, Input::Keyboard);
    assert_eq!(
        tick(&mut e, 61).announce.unwrap().events[0].category,
        Category::Exercise
    );
}
#[test]
fn three_followups_are_distinct_and_final_message_does_not_quit_or_disappear() {
    let mut e = engine();
    e.schedule(vec![event(Category::Break, "break", 0)], clock(0), false);
    let first = tick(&mut e, 0).announce.unwrap();
    e.confirm(&first.key, clock(0)).unwrap();
    let mut texts = BTreeSet::new();
    for s in [60, 120, 180] {
        e.input(clock(s).mono, Input::Keyboard);
        texts.insert(tick(&mut e, s).announce.unwrap().text);
    }
    assert_eq!(texts.len(), 3);
    assert!(e.state.cycles[0].finished);
    assert!(tick(&mut e, 240).announce.is_none());
    assert!(
        e.current(clock(240), Environment::default())
            .unwrap()
            .text
            .contains("Шучу")
    );
}
#[test]
fn incidental_clicks_allow_rest_and_activity_after_rest_does_not_restart_series() {
    let mut e = engine();
    e.schedule(vec![event(Category::Break, "break", 0)], clock(0), false);
    tick(&mut e, 0);
    for s in [10, 100, 200] {
        e.input(clock(s).mono, Input::Click);
    }
    e.input(clock(301).mono, Input::Keyboard);
    assert!(tick(&mut e, 301).announce.is_none());
    assert!(e.state.cycles[0].finished);
    assert!(tick(&mut e, 302).announce.is_none());
}
#[test]
fn pause_lock_and_long_sleep_do_not_produce_backlogs_or_resume_audio() {
    let mut e = engine();
    e.schedule(vec![event(Category::Lunch, "meal", 20)], clock(0), false);
    e.pause(Some(clock(60).utc));
    assert!(tick(&mut e, 20).announce.is_none());
    assert!(e.state.skipped.contains_key("meal"));
    assert!(tick(&mut e, 60).announce.is_none());
    e.schedule(vec![event(Category::Break, "b", 61)], clock(60), false);
    let first = tick(&mut e, 61).announce.unwrap();
    let blocked = e.tick(
        clock(62),
        Environment {
            locked: true,
            ..Default::default()
        },
        false,
    );
    assert!(blocked.stop_audio);
    assert!(
        e.current(
            clock(62),
            Environment {
                locked: true,
                ..Default::default()
            }
        )
        .is_none()
    );
    assert!(tick(&mut e, 63).announce.is_none());
    assert!(!e.state.pending[0].visible);
    e.review();
    assert_eq!(
        e.current(clock(64), Environment::default()).unwrap().key,
        first.key
    );
    assert!(tick(&mut e, 1000).announce.is_none());
    assert!(e.state.cycles[0].finished);
}
#[test]
fn restart_is_quiet_fresh_and_retains_consumed_ids_and_repeat_budget() {
    let mut e = engine();
    e.schedule(vec![event(Category::Break, "break", 0)], clock(0), false);
    tick(&mut e, 0);
    e.input(clock(60).mono, Input::Keyboard);
    tick(&mut e, 60);
    let bytes = serde_json::to_vec(&e.state).unwrap();
    let state = serde_json::from_slice(&bytes).unwrap();
    let mut resumed = Engine::new(e.prefs.clone(), state, clock(70)).unwrap();
    resumed.schedule(vec![event(Category::Break, "break", 0)], clock(70), false);
    assert_eq!(
        resumed
            .current(clock(70), Environment::default())
            .unwrap()
            .followup,
        1
    );
    assert!(tick(&mut resumed, 70).announce.is_none());
    assert_eq!(resumed.state.cycles[0].sent, 1);
    let old = Engine::new(e.prefs, e.state, clock(1000)).unwrap();
    assert!(old.current(clock(1000), Environment::default()).is_none());
}
#[test]
fn restoring_a_fresh_checkpoint_with_an_overdue_repeat_does_not_announce_it_immediately() {
    let mut original = engine();
    original.schedule(
        vec![event(Category::Break, "restore-overdue", 0)],
        clock(0),
        false,
    );
    assert!(tick(&mut original, 0).announce.is_some());
    let mut restored = Engine::new(original.prefs, original.state, clock(70)).unwrap();
    assert!(
        restored
            .current(clock(70), Environment::default())
            .is_some()
    );
    assert!(tick(&mut restored, 70).announce.is_none());
    assert_eq!(restored.state.cycles[0].sent, 0);
    assert_eq!(restored.state.cycles[0].next, clock(130).utc);
    assert!(tick(&mut restored, 129).announce.is_none());
    assert_eq!(tick(&mut restored, 130).announce.unwrap().followup, 1);
}
#[test]
fn settings_recompute_future_only_and_idle_ticks_do_not_rewrite_state() {
    let mut e = engine();
    e.schedule(vec![event(Category::Lunch, "old", 0)], clock(0), false);
    e.schedule(
        vec![
            event(Category::Dinner, "new-past", 0),
            event(Category::Dinner, "future", 50),
        ],
        clock(1),
        true,
    );
    assert_eq!(tick(&mut e, 1).announce.unwrap().events[0].id, "old");
    let before = serde_json::to_string(&e.state).unwrap();
    tick(&mut e, 2);
    tick(&mut e, 3);
    assert_eq!(serde_json::to_string(&e.state).unwrap(), before);
    assert_eq!(tick(&mut e, 50).announce.unwrap().events[0].id, "future");
}
#[test]
fn manual_stretch_works_during_pause_and_fullscreen_modes_are_separate() {
    let mut e = engine();
    e.pause(Some(clock(100).utc));
    let n = e.manual_stretch(clock(0)).announce.unwrap();
    assert!(n.explicit);
    tick(&mut e, 1);
    assert!(e.current(clock(1), Environment::default()).is_some());
    let env = Environment {
        fullscreen: true,
        ..Default::default()
    };
    assert!(e.current(clock(1), env).is_none());
    assert_eq!(e.audio_mode(env), AudioMode::Silent);
    e.prefs.reminders.fullscreen_visual = VisualMode::Message;
    assert!(e.current(clock(1), env).is_some());
    e.confirm(&n.key, clock(2)).unwrap();
    assert!(e.state.pending.is_empty());
}
#[test]
fn allowed_fullscreen_keeps_message_and_audio_but_hidden_modes_do_not_replay() {
    let mut e = engine();
    e.prefs.reminders.fullscreen_visual = VisualMode::Both;
    e.prefs.reminders.fullscreen_audio = AudioMode::All;
    e.schedule(vec![event(Category::Lunch, "food", 0)], clock(0), false);
    tick(&mut e, 0);
    let full = Environment {
        fullscreen: true,
        ..Default::default()
    };
    assert!(!e.tick(clock(1), full, true).stop_audio);
    assert!(e.current(clock(1), full).is_some());
    e.prefs.reminders.fullscreen_visual = VisualMode::Nothing;
    e.prefs.reminders.fullscreen_audio = AudioMode::Silent;
    e.tick(clock(2), Environment::default(), true);
    assert!(e.tick(clock(3), full, true).stop_audio);
    assert!(e.current(clock(3), full).is_none());
    tick(&mut e, 4);
    assert!(e.current(clock(4), Environment::default()).is_none());
}
#[test]
fn paused_idle_ticks_are_stable_and_missing_input_never_proves_rest() {
    let mut e = engine();
    e.schedule(
        vec![
            event(Category::Break, "break", 0),
            event(Category::Lunch, "later", 1),
        ],
        clock(0),
        false,
    );
    tick(&mut e, 0);
    e.pause(Some(clock(1000).utc));
    tick(&mut e, 1);
    let bytes = serde_json::to_string(&e.state).unwrap();
    assert!(!tick(&mut e, 2).stop_audio);
    tick(&mut e, 3);
    assert_eq!(serde_json::to_string(&e.state).unwrap(), bytes);
    e.pause(None);
    e.uncertain_input(clock(301).mono);
    tick(&mut e, 301);
    assert!(!e.state.cycles[0].finished);
}

#[test]
fn reported_notification_restriction_stops_initiative_without_replay_or_claiming_hidden_events() {
    use crate::engine::NotificationState;
    let mut e = engine();
    e.schedule(
        vec![
            event(Category::Lunch, "first", 0),
            event(Category::Dinner, "blocked", 2),
        ],
        clock(0),
        false,
    );
    assert!(tick(&mut e, 0).announce.is_some());
    let restricted = Environment {
        notifications: NotificationState::Blocked,
        ..Default::default()
    };
    assert!(e.tick(clock(1), restricted, true).stop_audio);
    assert!(e.current(clock(1), restricted).is_none());
    assert!(e.tick(clock(2), restricted, false).announce.is_none());
    assert!(!e.state.handled.contains_key("blocked"));
    assert!(e.state.skipped.contains_key("blocked"));
    assert!(tick(&mut e, 3).announce.is_none());
    assert!(e.current(clock(3), Environment::default()).is_none());
    assert_eq!(e.state.handled.len(), 1);
    e.review();
    assert!(e.current(clock(3), restricted).is_some()); // Explicit, silent review remains available.
}

#[test]
fn dnd_exception_is_used_by_real_engine_but_never_overrides_session_pause_or_fullscreen() {
    use crate::engine::NotificationState;
    for restriction in 0..5 {
        let mut e = engine();
        e.prefs.reminders.allow_during_dnd = true;
        e.schedule(vec![event(Category::Lunch, "lunch", 0)], clock(0), false);
        let mut env = Environment {
            notifications: NotificationState::Blocked,
            ..Default::default()
        };
        match restriction {
            1 => env.locked = true,
            2 => env.session_unknown = true,
            3 => e.pause(Some(clock(60).utc)),
            4 => {
                env.fullscreen = true;
                e.prefs.reminders.fullscreen_visual = VisualMode::Nothing;
                e.prefs.reminders.fullscreen_audio = AudioMode::Silent;
            }
            _ => {}
        }
        assert_eq!(
            e.tick(clock(0), env, false).announce.is_some(),
            restriction == 0
        );
    }
    let mut e = engine();
    e.prefs.reminders.allow_during_dnd = true;
    e.schedule(vec![event(Category::Lunch, "unknown", 0)], clock(0), false);
    assert!(
        e.tick(
            clock(0),
            Environment {
                notifications: NotificationState::Unknown,
                ..Default::default()
            },
            false
        )
        .announce
        .is_none()
    );
}

#[test]
fn unknown_session_does_not_accumulate_fake_rest_or_a_return_backlog() {
    let mut e = engine();
    e.schedule(
        vec![
            event(Category::Break, "break", 0),
            event(Category::Lunch, "while-away", 1),
        ],
        clock(0),
        false,
    );
    tick(&mut e, 0);
    let away = Environment {
        session_unknown: true,
        ..Default::default()
    };
    for second in [1, 61, 121, 181, 241, 301] {
        e.tick(clock(second), away, false);
    }
    assert!(
        !e.state.cycles[0].finished,
        "an unavailable input stream is not rest"
    );
    assert!(e.state.skipped.contains_key("while-away"));
    assert!(tick(&mut e, 302).announce.is_none());
    assert!(e.current(clock(302), Environment::default()).is_none());
    e.input(clock(600).mono, Input::Keyboard);
    assert!(
        tick(&mut e, 602).announce.is_some(),
        "future permitted followup must still work"
    );
}

#[test]
fn short_confirmed_sleep_stops_audio_skips_due_events_and_preserves_an_unfinished_cycle() {
    let mut e = engine();
    e.schedule(
        vec![
            event(Category::Break, "break", 0),
            event(Category::Lunch, "asleep", 20),
        ],
        clock(0),
        false,
    );
    tick(&mut e, 0);
    e.suspension(clock(30).mono, 30_000);
    let wake = tick(&mut e, 30);
    assert!(wake.stop_audio && wake.announce.is_none());
    assert!(e.state.skipped.contains_key("asleep"));
    assert!(!e.state.cycles[0].finished);
    assert!(e.current(clock(30), Environment::default()).is_none());
}

#[test]
fn confirmed_sleep_remains_proof_of_rest_when_the_session_is_still_locked() {
    let mut e = engine();
    let mut p = e.prefs.clone();
    p.idle_minutes = 1;
    e.preferences(p, clock(0)).unwrap();
    e.schedule(vec![event(Category::Break, "break", 0)], clock(0), false);
    tick(&mut e, 0);
    e.suspension(clock(120).mono, 120_000);
    e.tick(
        clock(120),
        Environment {
            locked: true,
            ..Default::default()
        },
        false,
    );
    assert!(e.state.cycles[0].finished);
}

#[test]
fn a_new_allowed_primary_does_not_erase_the_audio_stop_required_by_a_fullscreen_transition() {
    let mut e = engine();
    e.prefs.reminders.fullscreen_visual = VisualMode::Nothing;
    e.prefs.reminders.fullscreen_audio = AudioMode::Voice;
    e.schedule(vec![event(Category::Lunch, "lunch", 1)], clock(0), false);
    tick(&mut e, 0);
    let effect = e.tick(
        clock(1),
        Environment {
            fullscreen: true,
            ..Default::default()
        },
        false,
    );
    assert!(effect.announce.is_some());
    assert!(effect.stop_audio);
}
#[test]
fn saving_a_stale_settings_draft_preserves_unrelated_tray_changes() {
    let base = Preferences::default();
    let mut current = base.clone();
    current.reminders.pet_visible = false;
    let mut draft = base.clone();
    draft.schedule.break_every = 90;
    let merged = crate::preferences::merge(&current, &base, &draft).unwrap();
    assert!(!merged.reminders.pet_visible);
    assert_eq!(merged.schedule.break_every, 90);
}
#[test]
fn pending_messages_can_be_reviewed_without_confirming_or_spending_repeat_budget() {
    let mut e = engine();
    e.schedule(
        vec![
            event(Category::Lunch, "lunch", 0),
            event(Category::Break, "break", 1),
        ],
        clock(0),
        false,
    );
    tick(&mut e, 0);
    tick(&mut e, 1);
    assert_eq!(e.state.pending.len(), 2);
    let latest = e
        .current(clock(1), Environment::default())
        .unwrap()
        .key
        .clone();
    e.review();
    assert_ne!(
        e.current(clock(1), Environment::default()).unwrap().key,
        latest
    );
    e.pause(Some(clock(100).utc));
    tick(&mut e, 2);
    assert!(e.current(clock(2), Environment::default()).is_none());
    e.review();
    assert!(e.current(clock(2), Environment::default()).is_some());
    assert!(e.state.cycles.iter().all(|c| c.sent == 0));
}
#[test]
fn changing_the_breakfast_timer_respects_its_anchor_and_never_fires_a_past_timer() {
    let mut e = engine();
    e.prefs.schedule.exercise_after_minutes = 10;
    e.breakfast(clock(0)).unwrap();
    let mut p = e.prefs.clone();
    p.schedule.exercise_enabled = false;
    e.preferences(p.clone(), clock(10)).unwrap();
    assert!(e.state.exercise.is_none());
    p.schedule.exercise_enabled = true;
    p.schedule.exercise_after_minutes = 2;
    e.preferences(p.clone(), clock(20)).unwrap();
    assert_eq!(e.state.exercise.as_ref().unwrap().due, clock(120).utc);
    p.schedule.exercise_after_minutes = 1;
    e.preferences(p, clock(70)).unwrap();
    assert!(e.state.exercise.is_none());
    e.breakfast(clock(80)).unwrap();
    assert!(e.state.exercise.is_none());
}
#[test]
fn manual_actions_at_the_same_clock_instant_have_distinct_durable_ids() {
    let mut e = engine();
    let a = e.manual_stretch(clock(0)).announce.unwrap();
    let b = e.manual_stretch(clock(0)).announce.unwrap();
    assert_ne!(a.key, b.key);
    let mut resumed = Engine::new(e.prefs, e.state, clock(0)).unwrap();
    let c = resumed.manual_stretch(clock(0)).announce.unwrap();
    assert_ne!(b.key, c.key);
}
#[test]
fn rest_before_a_new_reminder_does_not_end_its_repeats_on_return_or_wake() {
    let mut e = engine();
    tick(&mut e, 300);
    e.input(clock(301).mono, Input::Keyboard);
    e.manual_stretch(clock(301));
    tick(&mut e, 301);
    assert!(!e.state.cycles[0].finished);
    e.input(clock(361).mono, Input::Keyboard);
    assert_eq!(tick(&mut e, 361).announce.unwrap().followup, 1);
    let mut resumed = engine();
    resumed.manual_stretch(clock(600));
    tick(&mut resumed, 600);
    assert!(!resumed.state.cycles[0].finished);
}
#[test]
fn ongoing_idle_span_can_complete_a_new_manual_reminder_after_its_own_five_minutes() {
    let mut e = engine();
    e.prefs.repeat_minutes = 10;
    e.manual_stretch(clock(600));
    tick(&mut e, 600);
    assert!(!e.state.cycles[0].finished);
    tick(&mut e, 899);
    assert!(!e.state.cycles[0].finished);
    assert!(tick(&mut e, 900).announce.is_none());
    assert!(e.state.cycles[0].finished);
}
