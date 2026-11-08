//! Offline replay of recorded snapshots. No networking, device access, or output injection.
use arcane_bridge_core::continuous::{MotionFilter, Settings};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    env,
    fs::File,
    io::{BufRead, BufReader, Write},
    path::Path,
};

#[derive(Clone)]
enum Action {
    Sample(usize, [f64; 3]),
    Reset,
    Tick(String),
}
#[derive(Clone)]
struct Event {
    time: f64,
    action: Action,
}
#[derive(Default)]
struct Stats {
    n: u64,
    ready: u64,
    moving: u64,
    peak: f64,
    sum: f64,
    first_moving: Option<f64>,
    last_moving: Option<f64>,
    last_speed: f64,
    max_gap: f64,
    last_tick: Option<f64>,
}
fn datetime_seconds(s: &str) -> Option<f64> {
    // Recorder writes UTC ISO 8601. Date-to-days avoids locale/timezone dependencies.
    let date = s.get(..10)?;
    let year = date.get(..4)?.parse::<i64>().ok()?;
    let month = date.get(5..7)?.parse::<i64>().ok()?;
    let day = date.get(8..10)?.parse::<i64>().ok()?;
    let hh = s.get(11..13)?.parse::<i64>().ok()?;
    let mm = s.get(14..16)?.parse::<i64>().ok()?;
    let tail = s.get(17..)?;
    let end = tail
        .find(|c: char| c == 'Z' || c == '+' || c == '-')
        .unwrap_or(tail.len());
    let sec = tail.get(..end)?.parse::<f64>().ok()?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hh > 23
        || mm > 59
        || !(0.0..61.).contains(&sec)
    {
        return None;
    }
    let y = year - i64::from(month <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(((era * 146097 + doe) * 86400 + hh * 3600 + mm * 60) as f64 + sec)
}
fn timestamp(row: &Value) -> Option<f64> {
    row["steady_ns"]
        .as_u64()
        .map(|n| n as f64 / 1e9)
        .or_else(|| {
            row["time"]
                .as_str()
                .or_else(|| row["utc"].as_str())
                .and_then(datetime_seconds)
        })
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args_os().collect();
    if args.len() != 3 {
        return Err("Usage: replay-locomotion <recording.jsonl> <report.json>".into());
    }
    let input = Path::new(&args[1]);
    let output = Path::new(&args[2]);
    let file = BufReader::new(File::open(input)?);
    let mut events = Vec::new();
    let mut old_ids: [Option<String>; 2] = [None, None];
    let mut counts: [Option<u64>; 2] = [None, None];
    let mut skipped = 0u64;
    let mut changes = 0u64;
    let mut resets = 0u64;
    let mut records = 0u64;
    for line in file.lines() {
        let line = line?;
        let row: Value = serde_json::from_str(&line)?;
        let data = if row["kind"].as_str() == Some("vrcraft") {
            &row["data"]
        } else if row.get("trackers").is_some() {
            &row
        } else {
            continue;
        };
        let Some(now) = timestamp(&row) else {
            skipped += 1;
            continue;
        };
        if !now.is_finite() {
            skipped += 1;
            continue;
        }
        let phase = row["phase"].as_str().unwrap_or("unlabelled").to_owned();
        records += 1;
        let trackers = data["trackers"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for (i, name) in ["left", "right"].iter().enumerate() {
            let Some(id) = data["roles"][name]["id"].as_str().filter(|x| !x.is_empty()) else {
                continue;
            };
            if old_ids[i].as_deref() != Some(id) {
                if old_ids[i].is_some() {
                    events.push(Event {
                        time: now,
                        action: Action::Reset,
                    });
                    resets += 1;
                }
                old_ids[i] = Some(id.to_owned());
                counts[i] = None;
            }
            let Some(tr) = trackers.iter().find(|x| x["id"].as_str() == Some(id)) else {
                continue;
            };
            let Some(count) = tr["acceleration_count"].as_u64().filter(|c| *c > 0) else {
                continue;
            };
            if counts[i] == Some(count) {
                continue;
            }
            let Some(age) = tr["acceleration_age_ms"]
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.)
            else {
                skipped += 1;
                continue;
            };
            let Some(arr) = tr["acceleration"].as_array().filter(|a| a.len() == 3) else {
                skipped += 1;
                continue;
            };
            let Some(x) = arr[0].as_f64() else {
                skipped += 1;
                continue;
            };
            let Some(y) = arr[1].as_f64() else {
                skipped += 1;
                continue;
            };
            let Some(z) = arr[2].as_f64() else {
                skipped += 1;
                continue;
            };
            let time = now - age / 1000.;
            if !time.is_finite() || [x, y, z].iter().any(|n| !n.is_finite()) {
                skipped += 1;
                continue;
            }
            if counts[i].is_some_and(|old| count < old) {
                events.push(Event {
                    time,
                    action: Action::Reset,
                });
                resets += 1;
            }
            counts[i] = Some(count);
            changes += 1;
            events.push(Event {
                time,
                action: Action::Sample(i, [x, y, z]),
            });
        }
        events.push(Event {
            time: now,
            action: Action::Tick(phase),
        });
    }
    if records == 0 {
        return Err("No VRCraft snapshot records in input".into());
    }
    events.sort_by(|a, b| {
        a.time.total_cmp(&b.time).then_with(|| {
            fn order(a: &Action) -> u8 {
                match a {
                    Action::Reset => 0,
                    Action::Sample(..) => 1,
                    Action::Tick(..) => 2,
                }
            }
            order(&a.action).cmp(&order(&b.action))
        })
    });
    let start = events.first().ok_or("No replay events")?.time;
    let mut filter = MotionFilter::new(Settings::default())?;
    let mut phases: BTreeMap<String, Stats> = BTreeMap::new();
    let mut states: BTreeMap<String, u64> = BTreeMap::new();
    let mut errors = 0u64;
    for e in events {
        let t = e.time - start;
        match e.action {
            Action::Reset => filter.reset(),
            Action::Sample(foot, v) => {
                if filter.update_foot(foot, v, t).is_err() {
                    errors += 1;
                }
            }
            Action::Tick(phase) => {
                let z = filter.tick(t);
                *states.entry(z.status.into()).or_default() += 1;
                let s = phases.entry(phase).or_default();
                s.n += 1;
                s.ready += u64::from(z.status == "ready");
                s.moving += u64::from(z.speed > 0.);
                s.peak = s.peak.max(z.speed);
                s.sum += z.speed;
                s.last_speed = z.speed;
                if z.speed > 0. {
                    if s.first_moving.is_none() {
                        s.first_moving = Some(t);
                    }
                    s.last_moving = Some(t);
                }
                if let Some(last) = s.last_tick {
                    s.max_gap = s.max_gap.max(t - last);
                }
                s.last_tick = Some(t);
            }
        }
    }
    let mut map = serde_json::Map::new();
    for (phase, s) in phases {
        map.insert(phase,json!({"snapshots":s.n,"ready_snapshots":s.ready,"moving_snapshots":s.moving,"max_speed":s.peak,"mean_speed":s.sum/s.n as f64,"last_speed":s.last_speed,"first_moving_seconds":s.first_moving,"last_moving_seconds":s.last_moving,"max_observation_gap_seconds":s.max_gap}));
    }
    let report = json!({"input":input.display().to_string(),"settings":Settings::default().to_json(),"snapshot_count":records,"unique_observed_acceleration_samples":changes,"counter_or_role_resets":resets,"skipped_rows_or_samples":skipped,"filter_sample_errors":errors,"status_counts":states,"phases":map,"limitations":["Only the latest distinct acceleration counter visible in each captured snapshot is replayed; intermediate packets are absent.","Sample times are estimated from snapshot time minus reported acceleration age; phase labels apply to snapshot observations.","Capture gaps, counter resets, snapshot scheduling and different sampling rates can change filter behavior; replay is not a physical test or evidence of game output.","Preview speed is not armed or sent to any runtime."]});
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = File::create(output)?;
    writeln!(f, "{}", serde_json::to_string_pretty(&report)?)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
