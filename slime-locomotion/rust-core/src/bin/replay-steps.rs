//! Offline replay of a recorded state capture through the real step-rhythm engine.
//! No networking, device access, or game output. Private recordings stay on disk.
//!
//! Usage: replay-steps <recording.jsonl> [--rate <hz>] [--series <out.csv>] [--filter-off]
//!
//! Captures are polled snapshots (roughly 20-30 Hz) while live trackers stream much
//! faster. `--rate` resamples orientation by spherical interpolation so the engine
//! sees a stream at a realistic packet rate. Acceleration is held between samples.
use arcane_bridge_core::gait::{Detection, Engine, Q};
use serde_json::Value;
use std::{env, fs::File, io::{BufRead, BufReader, Write}};

#[derive(Clone)]
struct Row { t: f64, q: [Q; 3], a: [[f64; 3]; 2] }

fn quaternion(v: &Value) -> Option<Q> {
    let a = v.as_array()?;
    if a.len() != 4 { return None; }
    let mut q = [0.; 4];
    for (i, x) in a.iter().enumerate() { q[i] = x.as_f64()?; }
    Some(q)
}
fn vector(v: &Value) -> [f64; 3] {
    let mut out = [0.; 3];
    if let Some(a) = v.as_array() { for (i, x) in a.iter().take(3).enumerate() { out[i] = x.as_f64().unwrap_or(0.); } }
    out
}
fn slerp(a: Q, mut b: Q, f: f64) -> Q {
    let mut dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    if dot < 0. { b = b.map(|v| -v); dot = -dot; }
    if dot > 0.9995 {
        let q: Q = std::array::from_fn(|i| a[i] + (b[i] - a[i]) * f);
        let n = q.iter().map(|v| v * v).sum::<f64>().sqrt();
        return q.map(|v| v / n);
    }
    let theta = dot.clamp(-1., 1.).acos();
    let (sa, sb) = (((1. - f) * theta).sin() / theta.sin(), (f * theta).sin() / theta.sin());
    std::array::from_fn(|i| a[i] * sa + b[i] * sb)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 { return Err("Usage: replay-steps <recording.jsonl> [--rate <hz>] [--series <out.csv>] [--filter-off]".into()); }
    let mut rate = 0.;
    let mut series = None;
    let mut filter = true;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--rate" => { rate = args.get(i + 1).ok_or("missing rate")?.parse()?; i += 1; }
            "--series" => { series = Some(File::create(args.get(i + 1).ok_or("missing series path")?)?); i += 1; }
            "--filter-off" => filter = false,
            other => return Err(format!("unknown argument {other}").into()),
        }
        i += 1;
    }
    let mut rows = vec![];
    for line in BufReader::new(File::open(&args[1])?).lines() {
        let Ok(v) = serde_json::from_str::<Value>(&line?) else { continue; };
        let Some(t) = v["t"].as_f64().or_else(|| v["elapsed"].as_f64()) else { continue; };
        let roles = &v["roles"];
        let (Some(l), Some(r), Some(c)) = (quaternion(&roles["left"]["quaternion"]),
            quaternion(&roles["right"]["quaternion"]), quaternion(&roles["torso"]["quaternion"])) else { continue; };
        if rows.last().is_some_and(|p: &Row| t <= p.t) { continue; }
        rows.push(Row { t, q: [l, r, c], a: [vector(&roles["left"]["acceleration"]), vector(&roles["right"]["acceleration"])] });
    }
    if rows.len() < 3 { return Err("recording has no usable rows".into()); }
    let frames: Vec<Row> = if rate > 0. {
        let (mut out, mut k, mut t) = (vec![], 0, rows[0].t);
        while t <= rows[rows.len() - 1].t {
            while k + 2 < rows.len() && rows[k + 1].t < t { k += 1; }
            let (a, b) = (&rows[k], &rows[k + 1]);
            let f = ((t - a.t) / (b.t - a.t)).clamp(0., 1.);
            out.push(Row { t, q: std::array::from_fn(|n| slerp(a.q[n], b.q[n], f)), a: if f < 0.5 { a.a } else { b.a } });
            t += 1. / rate;
        }
        out
    } else { rows.clone() };
    let mut cfg = Detection::default();
    cfg.cadence = true;
    cfg.foot_motion.enabled = filter;
    let mut e = Engine::new(cfg);
    // What a single chest sensor alone would conclude about turning.
    let mut chest = arcane_bridge_core::foot_motion::ChestTurn::default();
    let (mut chest_eased, mut chest_off) = (0., 0.);
    let start = frames[0].t;
    let (mut armed, mut distance, mut moving, mut peak, mut last) = (false, 0., 0., 0_f64, start);
    if let Some(f) = series.as_mut() { writeln!(f, "t,speed,heading_degrees,steps_left,steps_right,status,turn,left_twist,left_lift,right_twist,right_lift,chest_change")?; }
    for row in &frames {
        let t = row.t - start + 1.;
        for role in 0..3 { e.update(role, row.q[role], t)?; }
        for role in 0..2 { e.update_acceleration(role, row.a[role], t)?; }
        if !armed && e.calibrate(t).is_ok() && e.arm(t).is_ok() { armed = true; }
        e.tick(t);
        if !e.armed && armed { armed = e.arm(t).is_ok(); }
        let dt = (row.t - last).clamp(0., 0.25);
        chest.orientation(row.q[2], t, false);
        let factor = chest.factor(t);
        if factor < 1. { chest_eased += dt; }
        if factor == 0. { chest_off += dt; }
        last = row.t;
        distance += e.speed * dt;
        if e.speed > 0.02 { moving += dt; }
        peak = peak.max(e.speed);
        if let Some(f) = series.as_mut() {
            let snapshot = e.foot_motion_snapshot(t);
            let number = |v: &Value| v.as_f64().unwrap_or(0.);
            writeln!(f, "{:.3},{:.3},{:.1},{},{},{},{},{:.1},{:.1},{:.1},{:.1},{:.1}", row.t - start, e.speed, e.heading.to_degrees(),
                e.cadence_state.steps[0], e.cadence_state.steps[1], e.preview.status,
                snapshot["turn"]["reason"].as_str().unwrap_or(""),
                number(&snapshot["feet"][0]["step_twist_degrees"]), number(&snapshot["feet"][0]["step_lift_degrees"]),
                number(&snapshot["feet"][1]["step_twist_degrees"]), number(&snapshot["feet"][1]["step_lift_degrees"]),
                number(&snapshot["turn"]["chest_change_degrees"]))?;
        }
    }
    let duration = frames[frames.len() - 1].t - start;
    println!("{}", serde_json::json!({"recording": args[1], "frames": frames.len(), "duration_seconds": duration,
        "resampled_hz": rate, "filter": filter, "steps": e.cadence_state.steps,
        "travel_speed_seconds": distance, "moving_seconds": moving, "moving_percent": 100. * moving / duration.max(1e-9),
        "peak_speed": peak,
        "chest_only": {"steps_eased_percent": 100. * chest_eased / duration.max(1e-9), "steps_off_percent": 100. * chest_off / duration.max(1e-9)}, "foot_motion": e.foot_motion_snapshot(frames[frames.len() - 1].t - start + 1.)["summary"]}));
    Ok(())
}
