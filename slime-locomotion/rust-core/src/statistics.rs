//! Local, bounded daily movement estimates. Dates are stored as UTC calendar days.
use serde_json::{json, Value};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub const MAX_DAYS: usize = 366;
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DayStats {
    pub day: String,
    pub moving_seconds: f64,
    pub estimated_distance_m: f64,
    pub peak_speed: f64,
}

#[derive(Clone, Debug, Default)]
pub struct History {
    pub days: Vec<DayStats>,
    pub storage_error: String,
    last_attempt: f64,
}

impl History {
    pub fn load(path: &Path) -> Self {
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                return Self {
                    storage_error: format!("Statistics file could not be read: {e}"),
                    ..Default::default()
                }
            }
        };
        let mut raw = String::new();
        if let Err(e) = file.take(1_048_577).read_to_string(&mut raw) {
            return Self {
                storage_error: format!("Statistics file could not be read: {e}"),
                ..Default::default()
            };
        }
        if raw.len() > 1_048_576 {
            return Self {
                storage_error: "Statistics file exceeds the 1 MiB size limit".into(),
                ..Default::default()
            };
        }
        match serde_json::from_str::<Value>(&raw) {
            Ok(v) => Self {
                days: parse_days(&v),
                ..Default::default()
            },
            Err(e) => Self {
                storage_error: format!("Statistics file could not be read: {e}"),
                ..Default::default()
            },
        }
    }

    pub fn add(
        &mut self,
        epoch_seconds: f64,
        moving_seconds: f64,
        estimated_distance_m: f64,
        peak_speed: f64,
    ) {
        let day = utc_day(epoch_seconds);
        let slot = match self.days.iter().position(|d| d.day == day) {
            Some(i) => i,
            None => {
                self.days.push(DayStats {
                    day,
                    ..Default::default()
                });
                self.days.len() - 1
            }
        };
        let row = &mut self.days[slot];
        row.moving_seconds += finite_nonnegative(moving_seconds);
        row.estimated_distance_m += finite_nonnegative(estimated_distance_m);
        row.peak_speed = row.peak_speed.max(finite_nonnegative(peak_speed)).min(1.);
        self.days.sort_by(|a, b| a.day.cmp(&b.day));
        if self.days.len() > MAX_DAYS {
            self.days.drain(..self.days.len() - MAX_DAYS);
        }
    }

    /// Persist at most once per ten seconds. A failed save is retried on a later call.
    pub fn persist_if_due(&mut self, path: &Path, now: f64) {
        if !now.is_finite() || now < self.last_attempt || now - self.last_attempt < 10. {
            return;
        }
        self.last_attempt = now;
        match save_atomic(path, &self.days) {
            Ok(()) => {
                self.storage_error.clear();
            }
            Err(e) => self.storage_error = format!("Statistics could not be saved: {e}"),
        }
    }

    pub fn persist_now(&mut self, path: &Path) {
        match save_atomic(path, &self.days) {
            Ok(()) => self.storage_error.clear(),
            Err(e) => self.storage_error = format!("Statistics could not be saved: {e}"),
        }
    }

    pub fn to_json(&self, today: &str) -> Value {
        json!({"days":self.days.iter().map(|d| json!({"day":d.day,"moving_seconds":d.moving_seconds,"estimated_distance_m":d.estimated_distance_m,"peak_speed":d.peak_speed})).collect::<Vec<_>>(),"today_day":today,"today":self.days.iter().find(|d|d.day==today).map(|d|json!({"moving_seconds":d.moving_seconds,"estimated_distance_m":d.estimated_distance_m,"peak_speed":d.peak_speed})).unwrap_or(json!({"moving_seconds":0.,"estimated_distance_m":0.,"peak_speed":0.})),"storage_error":self.storage_error,"date_basis":"UTC"})
    }
}

fn finite_nonnegative(v: f64) -> f64 {
    if v.is_finite() && v > 0. {
        v
    } else {
        0.
    }
}

fn parse_days(v: &Value) -> Vec<DayStats> {
    let mut result = Vec::new();
    if let Some(rows) = v.get("days").and_then(Value::as_array) {
        for row in rows {
            let Some(day) = row
                .get("day")
                .and_then(Value::as_str)
                .filter(|s| valid_day(s))
            else {
                continue;
            };
            let parsed = DayStats {
                day: day.to_owned(),
                moving_seconds: finite_nonnegative(row["moving_seconds"].as_f64().unwrap_or(0.)),
                estimated_distance_m: finite_nonnegative(
                    row["estimated_distance_m"].as_f64().unwrap_or(0.),
                ),
                peak_speed: finite_nonnegative(row["peak_speed"].as_f64().unwrap_or(0.)).min(1.),
            };
            if let Some(existing) = result
                .iter_mut()
                .find(|d: &&mut DayStats| d.day == parsed.day)
            {
                existing.moving_seconds += parsed.moving_seconds;
                existing.estimated_distance_m += parsed.estimated_distance_m;
                existing.peak_speed = existing.peak_speed.max(parsed.peak_speed);
            } else {
                result.push(parsed);
            }
        }
    }
    result.sort_by(|a, b| a.day.cmp(&b.day));
    if result.len() > MAX_DAYS {
        result.drain(..result.len() - MAX_DAYS);
    }
    result
}

fn valid_day(s: &str) -> bool {
    if !s.is_ascii() || s.len() != 10 || s.as_bytes()[4] != b'-' || s.as_bytes()[7] != b'-' {
        return false;
    }
    let (Ok(y), Ok(m), Ok(d)) = (
        s[..4].parse::<i32>(),
        s[5..7].parse::<u32>(),
        s[8..].parse::<u32>(),
    ) else {
        return false;
    };
    m >= 1 && m <= 12 && d >= 1 && d <= month_days(y, m)
}
fn month_days(y: i32, m: u32) -> u32 {
    match m {
        2 if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Convert epoch seconds to an ISO UTC date without a timezone dependency.
pub fn utc_day(epoch_seconds: f64) -> String {
    let secs = if epoch_seconds.is_finite() {
        epoch_seconds.floor().clamp(0., 253_402_300_799.) as i64
    } else {
        0
    };
    let z = secs.div_euclid(86400) + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    y += if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn utc_day_index(day: &str) -> Option<i64> {
    if !valid_day(day) {
        return None;
    }
    let y: i64 = day[..4].parse().ok()?;
    let m: i64 = day[5..7].parse().ok()?;
    let d: i64 = day[8..10].parse().ok()?;
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = m + if m > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe - 719468)
}

fn save_atomic(path: &Path, days: &[DayStats]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp: PathBuf = path.with_extension("json.tmp");
    let data = json!({"version":1,"days":days.iter().map(|d|json!({"day":d.day,"moving_seconds":d.moving_seconds,"estimated_distance_m":d.estimated_distance_m,"peak_speed":d.peak_speed})).collect::<Vec<_>>()});
    fs::write(
        &tmp,
        serde_json::to_vec_pretty(&data).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utc_rollover_and_epoch() {
        assert_eq!(utc_day(0.), "1970-01-01");
        assert_eq!(utc_day(86_399.), "1970-01-01");
        assert_eq!(utc_day(86_400.), "1970-01-02");
    }
    #[test]
    fn invalid_and_bounded_history() {
        let mut h = History::default();
        for n in 0..400 {
            h.add(n as f64 * 86400., 1., 2., 0.5);
        }
        assert_eq!(h.days.len(), MAX_DAYS);
        let v = json!({"days":[{"day":"nonsense"},{"day":"2024-02-30"},{"day":"2024-02-29","moving_seconds":-3.}]});
        assert_eq!(parse_days(&v).len(), 1);
        assert_eq!(parse_days(&v)[0].moving_seconds, 0.);
    }
    #[test]
    fn persistence_roundtrip() {
        let path = std::env::temp_dir().join(format!("vrcraft-stats-{}.json", std::process::id()));
        let mut h = History::default();
        h.add(0., 12., 3., 0.4);
        save_atomic(&path, &h.days).unwrap();
        let restored = History::load(&path);
        assert_eq!(restored.days, h.days);
        let _ = fs::remove_file(path);
    }
    #[test]
    fn existing_file_can_be_replaced() {
        let path =
            std::env::temp_dir().join(format!("vrcraft-stats-replace-{}.json", std::process::id()));
        let mut h = History::default();
        h.add(0., 1., 1., 0.1);
        save_atomic(&path, &h.days).unwrap();
        h.add(86400., 2., 2., 0.2);
        save_atomic(&path, &h.days).unwrap();
        assert_eq!(History::load(&path).days, h.days);
        let _ = fs::remove_file(path);
    }
    #[test]
    fn non_ascii_date_is_rejected() {
        assert!(!valid_day("2024-日-01"));
        assert_eq!(utc_day_index("1970-01-02"), Some(1));
    }
}
