//! Foot intent evidence used only by step-rhythm locomotion.
//! Thresholds are provisional until checked against physical walking recordings.
use crate::gait::{unit, Q};
use serde_json::{json, Value};
use std::collections::VecDeque;

const WINDOW: f64 = 0.15;
const READING_AGE: f64 = 0.12;
pub(crate) const PIVOT_STOP_CONFIRMATION: f64 = 0.12;

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    pub min_swing_degrees: f64,
    pub pivot_ratio: f64,
}
impl Default for Settings {
    fn default() -> Self {
        Self { enabled: true, min_swing_degrees: 2., pivot_ratio: 2. }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.min_swing_degrees.is_finite() || !(0.5..=8.).contains(&self.min_swing_degrees)
            || !self.pivot_ratio.is_finite() || !(1. ..=5.).contains(&self.pivot_ratio) {
            return Err("invalid foot motion settings");
        }
        Ok(())
    }
    /// Partial objects retain defaults; unknown fields and wrong types are errors.
    pub fn from_json(v: &Value) -> Result<Self, &'static str> {
        let mut s = Self::default();
        for (key, value) in v.as_object().ok_or("foot motion settings must be an object")? {
            match key.as_str() {
                "enabled" => s.enabled = value.as_bool().ok_or("invalid foot motion enabled")?,
                "min_swing_degrees" => s.min_swing_degrees = value.as_f64().ok_or("invalid minimum swing")?,
                "pivot_ratio" => s.pivot_ratio = value.as_f64().ok_or("invalid pivot ratio")?,
                _ => return Err("unknown foot motion setting"),
            }
        }
        s.validate()?;
        Ok(s)
    }
    pub fn to_json(&self) -> Value {
        json!({"enabled":self.enabled,"min_swing_degrees":self.min_swing_degrees,"pivot_ratio":self.pivot_ratio})
    }
}
#[derive(Clone, Debug)]
struct Pose { t: f64, gravity: [f64; 3], yaw: f64 }
#[derive(Clone, Debug, Default)]
struct Foot {
    poses: VecDeque<Pose>,
    previous: Option<(Q, f64)>,
    yaw: f64,
    tilt: Option<(f64, f64)>,
    accel: Option<(f64, f64)>,
    candidate_since: Option<f64>,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Evidence {
    pub swing_degrees: f64,
    pub yaw_degrees: f64,
    pub qualified: bool,
    pub pivot: bool,
    pub fresh: bool,
    pub ready: bool,
}
#[derive(Clone, Debug, Default)]
pub struct Guard { feet: [Foot; 2] }
impl Guard {
    pub fn clear(&mut self) { *self = Self::default(); }
    pub fn orientation(&mut self, side: usize, q: Q, tilt: f64, t: f64, y_up: bool) {
        if side >= 2 || !t.is_finite() || !tilt.is_finite() { return; }
        let Ok(q) = unit(q) else { return; };
        let f = &mut self.feet[side];
        if let Some((previous, at)) = f.previous {
            if t <= at { return; }
            if t - at > WINDOW {
                f.poses.clear();
                f.yaw = 0.;
                f.candidate_since = None;
            } else {
                let up = if y_up { [0., 1., 0.] } else { [0., 0., 1.] };
                if let Some(delta) = crate::mounting::heading(q, previous, up) { f.yaw += delta; }
            }
        }
        f.previous = Some((q, t));
        f.tilt = Some((tilt.max(0.), t));
        f.poses.push_back(Pose { t, gravity: crate::gait::gravity(q, y_up), yaw: f.yaw });
        while f.poses.front().is_some_and(|p| t - p.t > WINDOW) { f.poses.pop_front(); }
        // Bound memory for malformed/unreasonably high-rate streams.
        while f.poses.len() > 512 { f.poses.pop_front(); }
        Self::update_candidate(f, t);
    }
    pub fn acceleration(&mut self, side: usize, accel: f64, t: f64) {
        if side < 2 && accel.is_finite() && t.is_finite()
            && self.feet[side].accel.is_none_or(|(_, at)| t > at) {
            self.feet[side].accel = Some((accel.max(0.), t));
            Self::update_candidate(&mut self.feet[side], t);
        }
    }
    pub fn evidence(&self, side: usize, t: f64, settings: &Settings) -> Evidence {
        let f = &self.feet[side];
        let mut e = Evidence::default();
        e.fresh = t.is_finite() && f.previous.is_some_and(|(_, at)| t >= at && t - at <= READING_AGE);
        if !e.fresh { return e; }
        (e.swing_degrees, e.yaw_degrees) = Self::spans(f, t);
        e.pivot = e.yaw_degrees > 3.
            && e.yaw_degrees > settings.pivot_ratio * e.swing_degrees.max(settings.min_swing_degrees);
        e.qualified = e.swing_degrees >= settings.min_swing_degrees && !e.pivot;
        e.ready = f.candidate_since.is_some_and(|at| t >= at && t - at >= 0.06);
        e
    }
    fn spans(f: &Foot, t: f64) -> (f64, f64) {
        let poses: Vec<_> = f.poses.iter().filter(|p| t >= p.t && t - p.t <= WINDOW).collect();
        let (mut swing, mut yaw) = (0_f64, 0_f64);
        for (i, a) in poses.iter().enumerate() {
            for b in poses.iter().skip(i + 1) {
                let dot = a.gravity.iter().zip(b.gravity).map(|(x,y)| x*y).sum::<f64>();
                swing = swing.max(dot.clamp(-1., 1.).acos().to_degrees());
                yaw = yaw.max((a.yaw - b.yaw).abs().to_degrees());
            }
        }
        (swing, yaw)
    }
    fn update_candidate(f: &mut Foot, t: f64) {
        let (swing, yaw) = Self::spans(f, t);
        let activity = |sample: Option<(f64, f64)>, reference: f64| sample
            .filter(|(_, at)| t >= *at && t - *at <= READING_AGE)
            .map_or(0., |(value, _)| value / reference);
        let moving = swing >= 0.5 || yaw >= 1.
            || activity(f.tilt, 2.).max(activity(f.accel, 1.2)) >= 1.;
        if moving { f.candidate_since.get_or_insert(t); } else { f.candidate_since = None; }
    }
    /// Raw timestamps are retained: replaying an acceleration on a pose event never refreshes it.
    pub fn sample(&self, side: usize, t: f64, allowed: bool) -> (f64, Option<f64>, f64, Option<f64>) {
        let f = &self.feet[side];
        let raw = |sample: Option<(f64, f64)>| {
            let Some((v, at)) = sample else { return (0., None); };
            (if allowed && t >= at && t - at <= READING_AGE { v } else { 0. }, Some(at))
        };
        let (tilt, tilt_at) = raw(f.tilt);
        let (accel, accel_at) = raw(f.accel);
        (tilt, tilt_at, accel, accel_at)
    }
    pub fn snapshot(&self, t: f64, settings: &Settings, suppressed: bool) -> Value {
        let feet: Vec<_> = (0..2).map(|side| {
            let e = self.evidence(side, t, settings);
            let reason = if !settings.enabled { "disabled" } else if !e.fresh { "waiting for fresh orientation" }
                else if e.pivot { "yaw-dominant pivot" } else if e.qualified && !e.ready { "checking swing" } else if e.qualified { "swing qualified" } else { "below minimum swing" };
            json!({"swing_degrees":e.swing_degrees,"yaw_degrees":e.yaw_degrees,"qualified":e.qualified,
                "pivot":e.pivot,"fresh":e.fresh,"ready":e.ready,"reason":reason})
        }).collect();
        json!({"settings":settings.to_json(),"feet":feet,"pivot_suppressed":suppressed,"window_seconds":WINDOW,"evidence_delay_seconds":0.06,"pivot_stop_confirmation_seconds":PIVOT_STOP_CONFIRMATION})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tilt(degrees: f64) -> Q { let h = degrees.to_radians() / 2.; [h.cos(), h.sin(), 0., 0.] }
    #[test]
    fn pending_acceleration_can_qualify_but_never_becomes_fresh_again() {
        let mut g = Guard::default(); let s = Settings::default();
        g.orientation(0, tilt(0.), 0., 0., false);
        g.acceleration(0, 3., 0.01);
        g.orientation(0, tilt(3.), 0.5, 0.08, false);
        let e = g.evidence(0, 0.08, &s);
        assert!(e.qualified && e.ready);
        assert_eq!(g.sample(0, 0.08, true).2, 3.);
        assert_eq!(g.sample(0, 0.14, true).2, 0.);
        g.acceleration(0, 0., 0.15);
        assert_eq!(g.sample(0, 0.16, true).2, 0.);
    }
    #[test]
    fn stale_or_nonpositive_time_cannot_supply_swing_evidence() {
        let mut g = Guard::default(); let s = Settings::default();
        g.orientation(0, tilt(0.), 0., 1., false);
        g.orientation(0, tilt(20.), 10., 1., false);
        g.orientation(0, tilt(20.), 10., 0.9, false);
        assert!(!g.evidence(0, 1.01, &s).qualified);
        g.orientation(0, tilt(20.), 0., 1.3, false);
        assert!(!g.evidence(0, 1.3, &s).qualified);
        assert!(!g.evidence(0, 1.5, &s).fresh);
    }
    #[test]
    fn settings_json_roundtrip_and_strict_validation() {
        let settings = Settings::default();
        assert_eq!(Settings::from_json(&settings.to_json()).unwrap(), settings);
        for v in [json!({"enabled":1}), json!({"min_swing_degrees":0.1}), json!({"pivot_ratio":6.}), json!({"unknown":true})] {
            assert!(Settings::from_json(&v).is_err());
        }
    }
}
