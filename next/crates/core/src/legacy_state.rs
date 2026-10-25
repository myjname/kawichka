//! Explicit .NET date/checkpoint conversion. Recovery never requests audio.
use crate::{
    engine::{Clock, Notice, SavedCycle, State},
    legacy::SettingsSnapshot,
    preferences::Preferences,
    reminders::{Category, Event},
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde_json::Value;

pub fn date(v: &Value) -> Result<Option<DateTime<Utc>>, String> {
    if v.is_null() {
        return Ok(None);
    }
    let s = v.as_str().ok_or("invalid legacy date")?;
    if s.starts_with("0001-") {
        return Ok(None);
    }
    if let Some(ms) = s.strip_prefix("/Date(").and_then(|s| s.strip_suffix(")/")) {
        let end = ms
            .char_indices()
            .skip(1)
            .find(|(_, c)| *c == '+' || *c == '-')
            .map_or(ms.len(), |(i, _)| i);
        return DateTime::from_timestamp_millis(
            ms[..end].parse().map_err(|_| "invalid Microsoft date")?,
        )
        .map(Some)
        .ok_or("date out of range".into());
    }
    s.parse().map(Some).map_err(|_| "invalid UTC date".into())
}
fn category(s: &str) -> Result<Category, String> {
    serde_json::from_value(Value::String(s.into())).map_err(|_| "invalid legacy category".into())
}
fn stamp(v: &Value, key: &str) -> Result<DateTime<Utc>, String> {
    date(&v[key])?.ok_or_else(|| format!("missing {key}"))
}
pub fn convert(
    snapshot: &SettingsSnapshot,
    checkpoint: &Value,
    prefs: &Preferences,
    clock: Clock,
    mut resolve: impl FnMut(NaiveDate, u16) -> Result<DateTime<Utc>, String>,
) -> Result<State, String> {
    let d = &snapshot.desktop;
    let mut state = State::default();
    let paused = d.get("Paused").and_then(Value::as_bool).unwrap_or(false);
    if paused {
        state.pause_until = d
            .get("PauseUntilUtc")
            .map(date)
            .transpose()?
            .flatten()
            .filter(|t| *t > clock.utc);
        state.pause_forever = d.get("PauseUntilUtc").is_none_or(Value::is_null);
    }
    let x = d.get("PetX").and_then(Value::as_i64);
    let y = d.get("PetY").and_then(Value::as_i64);
    if let (Some(x), Some(y)) = (x, y)
        && x != i32::MIN as i64
        && y != i32::MIN as i64
    {
        state.pet_position = Some((
            i32::try_from(x).map_err(|_| "invalid pet x")?,
            i32::try_from(y).map_err(|_| "invalid pet y")?,
        ));
    }
    for id in d
        .get("HandledEvents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let due = if let Some(s) = id.strip_prefix("exercise:") {
            date(&Value::String(s.into()))?
        } else {
            let parts: Vec<_> = id.split(':').collect();
            if parts.len() == 3 {
                let day = parts[0]
                    .parse::<NaiveDate>()
                    .map_err(|_| "invalid handled day")?;
                let minute = parts[1]
                    .parse::<u16>()
                    .map_err(|_| "invalid handled minute")?;
                category(parts[2])?;
                if minute >= 1440 {
                    return Err("invalid handled time".into());
                }
                Some(resolve(day, minute)?)
            } else {
                return Err("invalid handled ID".into());
            }
        };
        if let Some(due) = due {
            state.handled.insert(id.into(), due);
        }
    }
    if prefs.schedule.exercise_enabled
        && let Some(due) = d.get("ExerciseUtc").map(date).transpose()?.flatten()
    {
        let id = format!(
            "exercise:{}.{:07}Z",
            due.format("%Y-%m-%dT%H:%M:%S"),
            due.timestamp_subsec_nanos() / 100
        );
        let event = Event {
            id,
            category: Category::Exercise,
            due,
            followup: false,
        };
        let breakfast = due - Duration::minutes(prefs.schedule.exercise_after_minutes.into());
        if breakfast <= clock.utc && clock.utc - breakfast < Duration::days(1) {
            let day = if breakfast >= resolve(clock.date, 0)? {
                clock.date
            } else {
                clock.date.pred_opt().ok_or("breakfast date out of range")?
            };
            state.breakfast_dates.insert(day);
            state.breakfast_times.insert(day, breakfast);
        }
        if !state.handled.contains_key(&event.id) {
            state.exercise = Some(event);
        }
    }
    // Pre-v2 meal/break markers did not have HandledEvents.
    if let Some(s) = d.get("LastMeal").and_then(Value::as_str)
        && s.len() > 11
    {
        let day = s
            .get(..10)
            .ok_or("invalid last meal marker")?
            .parse::<NaiveDate>()
            .map_err(|_| "invalid last meal day")?;
        let kind = s.get(11..).ok_or("invalid last meal marker")?;
        let minute = match kind {
            "breakfast" => prefs.schedule.breakfast,
            "lunch" => prefs.schedule.lunch,
            "dinner" => prefs.schedule.dinner,
            _ => return Err("invalid last meal".into()),
        };
        state
            .handled
            .insert(format!("{day}:{minute}:{kind}"), resolve(day, minute)?);
    }
    if let Some(s) = d.get("LastBreak").and_then(Value::as_str)
        && s.len() == 13
    {
        let day = s
            .get(..10)
            .ok_or("invalid last break marker")?
            .parse::<NaiveDate>()
            .map_err(|_| "invalid last break day")?;
        let hour = s
            .get(11..)
            .ok_or("invalid last break marker")?
            .parse::<u16>()
            .map_err(|_| "invalid last break hour")?;
        if hour > 23 {
            return Err("invalid last break hour".into());
        }
        state.handled.insert(
            format!("{day}:{}:break", hour * 60),
            resolve(day, hour * 60)?,
        );
    }
    if checkpoint.is_null() {
        state.validate()?;
        return Ok(state);
    }
    if checkpoint["Version"].as_u64() != Some(1) {
        return Err("unsupported legacy checkpoint".into());
    }
    let c = category(
        checkpoint["Category"]
            .as_str()
            .ok_or("missing checkpoint category")?,
    )?;
    let text = checkpoint["Text"]
        .as_str()
        .ok_or("missing checkpoint text")?;
    let message = stamp(checkpoint, "MessageUtc")?;
    let checked = stamp(checkpoint, "CheckpointUtc")?;
    state.checkpoint_utc = Some(checked);
    let pending = checkpoint["Pending"].as_bool().ok_or("invalid Pending")?;
    let manual = checkpoint["Manual"].as_bool().ok_or("invalid Manual")?;
    let followup = checkpoint["Followup"].as_bool().ok_or("invalid Followup")?;
    if text.chars().count() > 400
        || pending && text.is_empty()
        || message > checked
        || checked > clock.utc + Duration::minutes(5)
    {
        return Err("invalid checkpoint bounds".into());
    }
    let cycle = &checkpoint["Cycle"];
    let suspended = date(&checkpoint["SuspendedUtc"])?;
    if suspended.is_some_and(|s| s > checked) {
        return Err("invalid suspension".into());
    }
    let mut event = Event {
        id: format!(
            "legacy:{}:{}",
            serde_json::to_value(c).unwrap().as_str().unwrap(),
            message.timestamp_millis()
        ),
        category: c,
        due: message,
        followup: false,
    };
    if !cycle.is_null() {
        if manual || category(cycle["Category"].as_str().ok_or("missing cycle category")?)? != c {
            return Err("invalid checkpoint cycle".into());
        }
        let started = stamp(cycle, "StartedUtc")?;
        let next = stamp(cycle, "NextRepeatUtc")?;
        let sent = cycle["RepeatsSent"]
            .as_u64()
            .filter(|n| *n <= 20)
            .ok_or("invalid repeat count")? as u32;
        let maximum = match c {
            Category::Break => prefs.reminders.break_repeats,
            Category::Exercise => prefs.reminders.exercise_repeats,
            _ => prefs.reminders.meal_repeats,
        }
        .max(sent);
        let hidden = cycle["HiddenByConfirmation"]
            .as_bool()
            .ok_or("invalid hidden flag")?;
        if started > checked
            || next < started
            || next > checked + Duration::hours(2)
            || hidden && pending
        {
            return Err("invalid cycle bounds".into());
        }
        event.due = started;
        let finished = cycle["Finished"].as_bool().ok_or("invalid finished flag")?
            || sent >= maximum
            || clock.utc - next > Duration::minutes(5)
            || suspended.is_some_and(|s| {
                started <= s && clock.utc - s >= Duration::minutes(prefs.idle_minutes.into())
            });
        state.cycles.push(SavedCycle {
            events: vec![event.clone()],
            started,
            next,
            sent,
            maximum,
            finished,
            hidden,
        });
    }
    if pending {
        state.pending.push(Notice {
            key: format!("legacy-notice:{}", message.timestamp_millis()),
            cycle: event.id.clone(),
            events: vec![event],
            created: message,
            generation_due: None,
            text: text.into(),
            followup: u32::from(followup),
            visible: (!paused || manual)
                && checked <= clock.utc
                && clock.utc - checked <= Duration::minutes(5),
            explicit: manual,
        });
    }
    state.validate()?;
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn legacy_manual_pending_survives_restart_with_scheduled_exercise_disabled() {
        let utc = "2026-10-25T12:02:00Z".parse::<DateTime<Utc>>().unwrap();
        let clock = Clock {
            utc,
            mono: 1000,
            date: utc.date_naive(),
        };
        let mut prefs = Preferences::default();
        prefs.schedule.exercise_enabled = false;
        let source = crate::legacy::reconcile(&Default::default(), &Default::default()).unwrap();
        let checkpoint = json!({"Version":1,"Category":"exercise","Text":"Saved manual exercise reminder","MessageUtc":"2026-10-25T12:00:00Z","CheckpointUtc":"2026-10-25T12:01:00Z","Pending":true,"Manual":true,"Followup":false,"Cycle":null});
        let state = convert(&source, &checkpoint, &prefs, clock, |_, _| unreachable!()).unwrap();
        assert_eq!(state.pending.len(), 1);
        assert!(state.pending[0].explicit);
        let engine = crate::engine::Engine::new(prefs, state, clock).unwrap();
        assert_eq!(
            engine.state.pending.len(),
            1,
            "scheduled exercise being disabled must not discard an explicit legacy message"
        );
        assert_eq!(
            engine.state.pending[0].text,
            "Saved manual exercise reminder"
        );
    }
    #[test]
    fn breakfast_timer_crosses_midnight_but_last_meal_notification_is_not_confirmation() {
        let now = "2026-10-24T00:10:00Z".parse::<DateTime<Utc>>().unwrap();
        let clock = Clock {
            utc: now,
            mono: 0,
            date: now.date_naive(),
        };
        let mut p = Preferences::default();
        p.schedule.exercise_enabled = true;
        p.schedule.exercise_after_minutes = 90;
        let resolver = |d: NaiveDate, m: u16| {
            Ok(d.and_hms_opt(u32::from(m / 60), u32::from(m % 60), 0)
                .unwrap()
                .and_utc())
        };
        let s = crate::legacy::reconcile(
            json!({"LastMeal":"2026-10-24:breakfast"})
                .as_object()
                .unwrap(),
            &Default::default(),
        )
        .unwrap();
        let a = convert(&s, &Value::Null, &p, clock, resolver).unwrap();
        assert!(a.breakfast_dates.is_empty());
        let s = crate::legacy::reconcile(
            json!({"ExerciseUtc":"2026-10-24T00:30:00Z"})
                .as_object()
                .unwrap(),
            &Default::default(),
        )
        .unwrap();
        let a = convert(&s, &Value::Null, &p, clock, resolver).unwrap();
        assert!(a.breakfast_dates.contains(&clock.date.pred_opt().unwrap()));
        assert!(!a.breakfast_dates.contains(&clock.date));
        assert_eq!(
            a.exercise.unwrap().due,
            "2026-10-24T00:30:00Z".parse::<DateTime<Utc>>().unwrap()
        );
    }
    #[test]
    fn checkpoint_keeps_hidden_budget_pause_and_utc_formats() {
        let now = "2026-10-23T12:02:00Z".parse().unwrap();
        let clock = Clock {
            utc: now,
            mono: 1000,
            date: now.date_naive(),
        };
        let s = crate::legacy::reconcile(
            json!({"Paused":true,"PetX":120,"PetY":200,"HandledEvents":["2026-10-23:600:break"]})
                .as_object()
                .unwrap(),
            &Default::default(),
        )
        .unwrap();
        let checkpoint = json!({"Version":1,"Category":"break","Text":"A saved reminder","MessageUtc":"2026-10-23T12:00:00Z","CheckpointUtc":"2026-10-23T12:01:00Z","Pending":false,"Manual":false,"Followup":true,"Cycle":{"Category":"break","StartedUtc":"2026-10-23T12:00:00Z","NextRepeatUtc":"2026-10-23T12:05:00Z","RepeatsSent":2,"Finished":false,"HiddenByConfirmation":true}});
        let state = convert(&s, &checkpoint, &Preferences::default(), clock, |d, m| {
            Ok(d.and_hms_opt(u32::from(m / 60), u32::from(m % 60), 0)
                .unwrap()
                .and_utc())
        })
        .unwrap();
        assert!(state.pause_forever);
        assert_eq!(state.cycles[0].sent, 2);
        assert!(state.cycles[0].hidden);
        assert_eq!(state.pet_position, Some((120, 200)));
        assert_eq!(state.handled.len(), 1);
        assert!(state.pending.is_empty());
        assert_eq!(
            date(&json!("/Date(1792756800000+0200)/")).unwrap(),
            Some("2026-10-23T12:00:00Z".parse().unwrap())
        );
    }
    #[test]
    fn fresh_pending_is_restored_silently_and_future_schema_rejected() {
        let now = "2026-10-23T12:02:00Z".parse().unwrap();
        let clock = Clock {
            utc: now,
            mono: 1000,
            date: now.date_naive(),
        };
        let s = crate::legacy::reconcile(&Default::default(), &Default::default()).unwrap();
        let mut c = json!({"Version":1,"Category":"exercise","Text":"saved","MessageUtc":"2026-10-23T12:00:00Z","CheckpointUtc":"2026-10-23T12:01:00Z","Pending":true,"Manual":true,"Followup":false,"Cycle":null});
        let state = convert(
            &s,
            &c,
            &Preferences::default(),
            clock,
            |_, _| unreachable!(),
        )
        .unwrap();
        let engine = crate::engine::Engine::new(Preferences::default(), state, clock).unwrap();
        assert_eq!(engine.state.pending[0].text, "saved");
        c["MessageUtc"] = json!("2026-10-23T10:00:00Z");
        let state = convert(
            &s,
            &c,
            &Preferences::default(),
            clock,
            |_, _| unreachable!(),
        )
        .unwrap();
        let restored =
            crate::engine::Engine::new(Preferences::default(), state.clone(), clock).unwrap();
        assert!(restored.state.pending[0].visible);
        assert_eq!(
            restored.state.pending[0].created,
            "2026-10-23T10:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
        let later = Clock {
            utc: now + Duration::minutes(6),
            ..clock
        };
        let restored = crate::engine::Engine::new(Preferences::default(), state, later).unwrap();
        assert!(!restored.state.pending[0].visible);
        c["Version"] = json!(2);
        assert!(
            convert(
                &s,
                &c,
                &Preferences::default(),
                clock,
                |_, _| unreachable!()
            )
            .is_err()
        );
    }
}
