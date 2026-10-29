//! Local calendar rules. A platform resolver owns time-zone/DST conversion.
use crate::reminders::{Category, Event};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schedule {
    /// .NET-compatible numbering: Sunday=0, Monday=1, ... Saturday=6.
    pub work_days: Vec<u32>,
    pub work_start: u16,
    pub work_end: u16,
    pub break_every: u16,
    pub meals_enabled: bool,
    pub breakfast: u16,
    pub lunch: u16,
    pub dinner: u16,
    pub exercise_enabled: bool,
    pub exercise_after_minutes: u16,
}
impl Default for Schedule {
    fn default() -> Self {
        Self {
            work_days: vec![1, 2, 3, 4, 5],
            work_start: 600,
            work_end: 1140,
            break_every: 60,
            meals_enabled: true,
            breakfast: 600,
            lunch: 870,
            dinner: 1110,
            exercise_enabled: true,
            exercise_after_minutes: 120,
        }
    }
}
#[derive(Debug, thiserror::Error)]
pub enum ScheduleError {
    #[error("invalid schedule configuration")]
    Invalid,
    #[error("time-zone conversion failed")]
    TimeZone,
}
impl Schedule {
    pub fn validate(&self) -> Result<(), ScheduleError> {
        let mut days = self.work_days.clone();
        days.sort();
        days.dedup();
        if days.len() != self.work_days.len()
            || days.iter().any(|day| *day > 6)
            || self.work_start > self.work_end
            || self.work_end >= 1440
            || !(5..=720).contains(&self.break_every)
            || [self.breakfast, self.lunch, self.dinner]
                .iter()
                .any(|time| *time >= 1440)
            || !(1..=1440).contains(&self.exercise_after_minutes)
        {
            return Err(ScheduleError::Invalid);
        }
        Ok(())
    }
    pub fn day(
        &self,
        date: NaiveDate,
        mut resolve: impl FnMut(NaiveDate, u16) -> Result<DateTime<Utc>, ScheduleError>,
    ) -> Result<Vec<Event>, ScheduleError> {
        self.validate()?;
        if !self
            .work_days
            .contains(&date.weekday().num_days_from_sunday())
        {
            return Ok(Vec::new());
        }
        let mut events = Vec::new();
        for minute in (self.work_start..=self.work_end).step_by(self.break_every.into()) {
            events.push(make(date, minute, Category::Break, &mut resolve)?);
        }
        if self.meals_enabled {
            for (minute, category) in [
                (self.breakfast, Category::Breakfast),
                (self.lunch, Category::Lunch),
                (self.dinner, Category::Dinner),
            ] {
                events.push(make(date, minute, category, &mut resolve)?);
            }
        }
        events.sort_by(|a, b| a.due.cmp(&b.due).then_with(|| a.id.cmp(&b.id)));
        Ok(events)
    }
    /// The stretch timer is elapsed time after confirmation, on every weekday.
    pub fn mark_breakfast(&self, now: DateTime<Utc>) -> Result<Option<Event>, ScheduleError> {
        self.validate()?;
        if !self.exercise_enabled {
            return Ok(None);
        }
        let due = now + Duration::minutes(self.exercise_after_minutes.into());
        // Preserve legacy C# round-trip ID precision (100 ns).
        let id = format!(
            "exercise:{}.{:07}Z",
            due.format("%Y-%m-%dT%H:%M:%S"),
            due.timestamp_subsec_nanos() / 100
        );
        Ok(Some(Event {
            id,
            category: Category::Exercise,
            due,
            followup: false,
        }))
    }
}
fn make(
    date: NaiveDate,
    minute: u16,
    category: Category,
    resolve: &mut impl FnMut(NaiveDate, u16) -> Result<DateTime<Utc>, ScheduleError>,
) -> Result<Event, ScheduleError> {
    let name = match category {
        Category::Breakfast => "breakfast",
        Category::Lunch => "lunch",
        Category::Dinner => "dinner",
        Category::Break => "break",
        Category::Exercise => "exercise",
    };
    Ok(Event {
        id: format!("{date}:{minute}:{name}"),
        category,
        due: resolve(date, minute)?,
        followup: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn utc(date: NaiveDate, minute: u16) -> Result<DateTime<Utc>, ScheduleError> {
        Ok(date
            .and_hms_opt((minute / 60).into(), (minute % 60).into(), 0)
            .unwrap()
            .and_utc())
    }
    #[test]
    fn friday_events_keep_meal_break_ids_separate_for_later_merging() {
        let events = Schedule::default()
            .day(NaiveDate::from_ymd_opt(2026, 10, 23).unwrap(), utc)
            .unwrap();
        assert_eq!(events.len(), 13);
        assert!(events.iter().any(|e| e.id == "2026-10-23:600:breakfast"));
        assert!(events.iter().any(|e| e.id == "2026-10-23:600:break"));
    }
    #[test]
    fn weekend_has_no_work_events_but_breakfast_still_arms_stretch() {
        let config = Schedule::default();
        let date = NaiveDate::from_ymd_opt(2026, 10, 24).unwrap();
        assert!(config.day(date, utc).unwrap().is_empty());
        let now = utc(date, 600).unwrap();
        let exercise = config.mark_breakfast(now).unwrap().unwrap();
        assert_eq!(exercise.due, now + Duration::minutes(120));
        assert_eq!(exercise.id, "exercise:2026-10-24T12:00:00.0000000Z");
    }
    #[test]
    fn invalid_or_failed_zone_conversion_is_not_silently_changed() {
        let mut config = Schedule {
            break_every: 0,
            ..Schedule::default()
        };
        assert!(
            config
                .day(NaiveDate::from_ymd_opt(2026, 10, 23).unwrap(), utc)
                .is_err()
        );
        config.break_every = 60;
        assert!(
            config
                .day(NaiveDate::from_ymd_opt(2026, 10, 23).unwrap(), |_, _| Err(
                    ScheduleError::TimeZone
                ))
                .is_err()
        );
    }
}
