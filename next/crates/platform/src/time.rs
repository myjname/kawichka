use chrono::{DateTime, Duration, Local, NaiveDate, TimeZone, Utc};
use kavychka_core::{
    engine::Clock,
    reminders::Event,
    schedule::{Schedule, ScheduleError},
};
pub fn monotonic_ms() -> u64 {
    #[cfg(windows)]
    {
        unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() }
    }
    #[cfg(not(windows))]
    {
        use std::sync::OnceLock;
        static START: OnceLock<std::time::Instant> = OnceLock::new();
        START
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_millis() as u64
    }
}

/// Qualification clock: excludes suspended/hibernated time. Domain deadlines
/// retain their existing monotonic clock; this adapter is for unattended tests.
pub fn awake_ms() -> Result<u64, String> {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn QueryUnbiasedInterruptTime(value: *mut u64) -> i32;
        }
        let mut value = 0;
        // SAFETY: stable Win32 API, valid writable u64; Windows 11 is the target.
        if unsafe { QueryUnbiasedInterruptTime(&mut value) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(value / 10_000)
    }
    #[cfg(not(windows))]
    {
        Err("Awake-time qualification clock unavailable on this platform".into())
    }
}
pub fn now() -> Clock {
    let local = Local::now();
    Clock {
        utc: local.with_timezone(&Utc),
        mono: monotonic_ms(),
        date: local.date_naive(),
    }
}
/// Ambiguous wall times fire once at the first instant; nonexistent wall times
/// resolve to the first valid minute after the DST gap, never to another date.
pub fn resolve(date: NaiveDate, minute: u16) -> Result<DateTime<Utc>, ScheduleError> {
    for minute in minute..1440 {
        let local = date
            .and_hms_opt(u32::from(minute / 60), u32::from(minute % 60), 0)
            .ok_or(ScheduleError::TimeZone)?;
        match Local.from_local_datetime(&local) {
            chrono::LocalResult::Single(at) => return Ok(at.with_timezone(&Utc)),
            chrono::LocalResult::Ambiguous(a, b) => return Ok(a.min(b).with_timezone(&Utc)),
            chrono::LocalResult::None => (),
        }
    }
    Err(ScheduleError::TimeZone)
}
pub fn day(schedule: &Schedule, clock: Clock) -> Result<Vec<Event>, ScheduleError> {
    schedule.day(clock.date, resolve)
}
pub fn tomorrow(clock: Clock) -> Result<DateTime<Utc>, ScheduleError> {
    resolve(clock.date.succ_opt().ok_or(ScheduleError::TimeZone)?, 0)
}
pub fn local_caption(at: DateTime<Utc>) -> String {
    at.with_timezone(&Local).format("%H:%M").to_string()
}
pub fn local_offset() -> i32 {
    Local::now().offset().local_minus_utc()
}
pub fn pause_minutes(clock: Clock, minutes: u32) -> DateTime<Utc> {
    clock.utc + Duration::minutes(minutes.min(1440).into())
}
pub fn now_in(zone: &str) -> Result<Clock, ScheduleError> {
    let mut c = now();
    c.date = local_in(c.utc, zone)?.date();
    Ok(c)
}
pub fn zones() -> Vec<(String, String)> {
    let mut out = vec![(String::new(), "Как в Windows / Follow Windows".into())];
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Time::*;
        for i in 0..1024 {
            let mut z = std::mem::zeroed();
            if EnumDynamicTimeZoneInformation(i, &mut z) != 0 {
                break;
            }
            let text = |s: &[u16]| {
                String::from_utf16_lossy(&s[..s.iter().position(|c| *c == 0).unwrap_or(s.len())])
            };
            out.push((text(&z.TimeZoneKeyName), text(&z.StandardName)));
        }
    }
    out
}
pub fn day_in(schedule: &Schedule, clock: Clock, zone: &str) -> Result<Vec<Event>, ScheduleError> {
    schedule.day(clock.date, |d, m| resolve_in(d, m, zone))
}
pub fn tomorrow_in(clock: Clock, zone: &str) -> Result<DateTime<Utc>, ScheduleError> {
    resolve_in(
        clock.date.succ_opt().ok_or(ScheduleError::TimeZone)?,
        0,
        zone,
    )
}
pub fn caption_in(at: DateTime<Utc>, zone: &str) -> Result<String, ScheduleError> {
    Ok(local_in(at, zone)?.format("%H:%M").to_string())
}
pub fn offset_in(at: DateTime<Utc>, zone: &str) -> Result<i32, ScheduleError> {
    Ok((local_in(at, zone)? - at.naive_utc()).num_seconds() as i32)
}
pub fn local_in(at: DateTime<Utc>, zone: &str) -> Result<chrono::NaiveDateTime, ScheduleError> {
    if zone.is_empty() {
        return Ok(at.with_timezone(&Local).naive_local());
    }
    #[cfg(windows)]
    {
        win::local(at, &win::zone(zone)?)
    }
    #[cfg(not(windows))]
    {
        Err(ScheduleError::TimeZone)
    }
}
pub fn resolve_in(
    date: NaiveDate,
    minute: u16,
    zone: &str,
) -> Result<DateTime<Utc>, ScheduleError> {
    if zone.is_empty() {
        return resolve(date, minute);
    }
    #[cfg(windows)]
    {
        win::resolve(date, minute, &win::zone(zone)?)
    }
    #[cfg(not(windows))]
    {
        Err(ScheduleError::TimeZone)
    }
}
#[cfg(windows)]
mod win {
    use super::*;
    use chrono::{Datelike, Timelike};
    use windows_sys::Win32::{Foundation::SYSTEMTIME, System::Time::*};
    fn text(s: &[u16]) -> String {
        String::from_utf16_lossy(&s[..s.iter().position(|c| *c == 0).unwrap_or(s.len())])
    }
    pub fn zone(id: &str) -> Result<DYNAMIC_TIME_ZONE_INFORMATION, ScheduleError> {
        static CACHE: std::sync::OnceLock<
            std::sync::Mutex<std::collections::HashMap<String, DYNAMIC_TIME_ZONE_INFORMATION>>,
        > = std::sync::OnceLock::new();
        let mut cache = CACHE
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| ScheduleError::TimeZone)?;
        if let Some(z) = cache.get(id) {
            return Ok(*z);
        }
        for i in 0..1024 {
            let mut z = unsafe { std::mem::zeroed() };
            let result = unsafe { EnumDynamicTimeZoneInformation(i, &mut z) };
            if result == 259 {
                break;
            }
            if result != 0 {
                return Err(ScheduleError::TimeZone);
            }
            if text(&z.TimeZoneKeyName) == id {
                cache.insert(id.into(), z);
                return Ok(z);
            }
        }
        Err(ScheduleError::TimeZone)
    }
    fn system(t: chrono::NaiveDateTime) -> SYSTEMTIME {
        SYSTEMTIME {
            wYear: t.year() as u16,
            wMonth: t.month() as u16,
            wDay: t.day() as u16,
            wDayOfWeek: 0,
            wHour: t.hour() as u16,
            wMinute: t.minute() as u16,
            wSecond: t.second() as u16,
            wMilliseconds: t.and_utc().timestamp_subsec_millis() as u16,
        }
    }
    pub fn local(
        at: DateTime<Utc>,
        zone: &DYNAMIC_TIME_ZONE_INFORMATION,
    ) -> Result<chrono::NaiveDateTime, ScheduleError> {
        let mut out = unsafe { std::mem::zeroed() };
        if unsafe { SystemTimeToTzSpecificLocalTimeEx(zone, &system(at.naive_utc()), &mut out) }
            == 0
        {
            return Err(ScheduleError::TimeZone);
        }
        NaiveDate::from_ymd_opt(out.wYear.into(), out.wMonth.into(), out.wDay.into())
            .and_then(|d| {
                d.and_hms_milli_opt(
                    out.wHour.into(),
                    out.wMinute.into(),
                    out.wSecond.into(),
                    out.wMilliseconds.into(),
                )
            })
            .ok_or(ScheduleError::TimeZone)
    }
    pub fn resolve(
        date: NaiveDate,
        minute: u16,
        zone: &DYNAMIC_TIME_ZONE_INFORMATION,
    ) -> Result<DateTime<Utc>, ScheduleError> {
        let mut annual = unsafe { std::mem::zeroed() };
        if unsafe { GetTimeZoneInformationForYear(date.year() as u16, zone, &mut annual) } == 0 {
            return Err(ScheduleError::TimeZone);
        }
        for m in minute..1440 {
            let wall = date
                .and_hms_opt(u32::from(m / 60), u32::from(m % 60), 0)
                .ok_or(ScheduleError::TimeZone)?;
            let mut candidates = Vec::new();
            for bias in [
                annual.Bias + annual.StandardBias,
                annual.Bias + annual.DaylightBias,
            ] {
                let utc = wall.and_utc() + Duration::minutes(bias.into());
                if local(utc, zone)? == wall {
                    candidates.push(utc)
                }
            }
            if let Some(at) = candidates.into_iter().min() {
                return Ok(at);
            }
        }
        Err(ScheduleError::TimeZone)
    }
}
#[cfg(all(test, windows))]
mod zone_tests {
    use super::*;
    #[test]
    fn named_zone_gap_overlap_and_date_are_explicit() {
        let z = "Central Europe Standard Time";
        assert_eq!(
            resolve_in("2026-03-29".parse().unwrap(), 150, z).unwrap(),
            "2026-03-29T01:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
        assert_eq!(
            resolve_in("2026-10-25".parse().unwrap(), 150, z).unwrap(),
            "2026-10-25T00:30:00Z".parse::<DateTime<Utc>>().unwrap()
        );
        assert_eq!(
            local_in("2026-10-23T23:00:00Z".parse().unwrap(), z)
                .unwrap()
                .date()
                .to_string(),
            "2026-10-24"
        );
        assert!(resolve_in("2026-10-23".parse().unwrap(), 0, "Unrecognized timezone").is_err());
    }
}
