//! Deliberate, chest-relative head sweep gesture. This module never changes calibration.
//! Inputs use receiver monotonic seconds; callers must gate assigned tracker freshness.
use crate::gait::{unit, Q};
use serde_json::{json, Value};

const FRESH: f64 = 0.25;
const ARM_DWELL: f64 = 0.5;
const END_DWELL: f64 = 0.08;
const FORWARD_DWELL: f64 = 0.6;
const COOLDOWN: f64 = 10.;
const REQUIRED_EXTREMES: usize = 8;
const EPS: f64 = 1e-8;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    pub enabled: bool,
}
impl Settings {
    pub fn validate(&self) -> Result<(), &'static str> { Ok(()) }
    pub fn from_json(v: &Value) -> Result<Self, &'static str> {
        let mut s = Self::default();
        for (key, value) in v.as_object().ok_or("head recenter settings must be an object")? {
            match key.as_str() {
                "enabled" => s.enabled = value.as_bool().ok_or("invalid head recenter enabled")?,
                _ => return Err("unknown head recenter setting"),
            }
        }
        s.validate()?;
        Ok(s)
    }
    pub fn to_json(&self) -> Value { json!({"enabled":self.enabled}) }
}

#[derive(Clone, Copy, Debug)]
pub struct Input {
    pub t: f64,
    /// Raw HMD yaw in radians and the timestamp of that actual HMD reading.
    pub hmd: Option<(f64, f64)>,
    pub chest: Option<Q>,
    /// None means an unassigned foot. Assigned but stale/missing feet set ready=false.
    pub feet: [Option<Q>; 2],
    /// Fresh linear acceleration magnitude, in m/s², for each assigned foot.
    pub foot_accel: [Option<f64>; 2],
    pub y_up: bool,
    /// Protocol, ownership, driver and assigned-tracker freshness gates belong to caller.
    pub ready: bool,
}

#[derive(Clone, Copy, Debug)]
struct Baseline {
    chest: Q,
    feet: [Option<Q>; 2],
    y_up: bool,
    center: f64,
    since: f64,
}

#[derive(Clone, Debug)]
pub struct Detector {
    settings: Settings,
    baseline: Option<Baseline>,
    last_hmd: Option<(f64, f64)>,
    unrolled: f64,
    last_relative: Option<f64>,
    armed: bool,
    extremes: usize,
    last_side: i8,
    candidate: Option<(i8, f64, f64)>,
    started: Option<f64>,
    last_extreme: Option<f64>,
    forward_since: Option<f64>,
    forward_anchor: Option<f64>,
    last_completed: Option<f64>,
    reason: &'static str,
}

fn wrap(angle: f64) -> f64 { angle.sin().atan2(angle.cos()) }
fn angle(a: Q, b: Q) -> f64 {
    let dot = a.iter().zip(b).map(|(a, b)| a * b).sum::<f64>().abs().clamp(0., 1.);
    2. * dot.acos()
}

impl Detector {
    pub fn new(settings: Settings) -> Result<Self, &'static str> {
        settings.validate()?;
        Ok(Self { settings, baseline: None, last_hmd: None, unrolled: 0.,
            last_relative: None, armed: false, extremes: 0, last_side: 0,
            candidate: None, started: None, last_extreme: None, forward_since: None, forward_anchor: None,
            last_completed: None, reason: "waiting for stillness" })
    }
    pub fn settings(&self) -> &Settings { &self.settings }
    pub fn set_settings(&mut self, settings: Settings) -> Result<(), &'static str> {
        settings.validate()?;
        self.settings = settings;
        self.reset();
        Ok(())
    }
    /// Clear the current attempt without changing enabled state or erasing cooldown.
    pub fn reset(&mut self) {
        self.baseline = None;
        self.last_hmd = None;
        self.unrolled = 0.;
        self.last_relative = None;
        self.armed = false;
        self.extremes = 0;
        self.last_side = 0;
        self.candidate = None;
        self.started = None;
        self.last_extreme = None;
        self.forward_since = None;
        self.forward_anchor = None;
        self.reason = "waiting for stillness";
    }
    fn cancel(&mut self, reason: &'static str) -> bool {
        self.reset();
        self.reason = reason;
        false
    }
    pub fn snapshot(&self) -> Value {
        let phase = if !self.settings.enabled { "disabled" }
            else if self.reason == "cooldown" { "cooldown" }
            else if !self.armed { "waiting_still" }
            else if self.extremes == REQUIRED_EXTREMES { "return_forward" }
            else { "sweeping" };
        json!({"enabled":self.settings.enabled,"phase":phase,
            "completed_cycles":self.extremes / 2,"required_cycles":4,
            "progress":self.extremes as f64 / REQUIRED_EXTREMES as f64,
            "reason":self.reason,"last_completed":self.last_completed})
    }
    /// Return true exactly once after four alternating left/right pairs and a forward dwell.
    pub fn update(&mut self, i: &Input) -> bool {
        if !self.settings.enabled { return self.cancel("disabled"); }
        if !i.t.is_finite() || !i.ready { return self.cancel("input not ready"); }
        if self.last_completed.is_some_and(|t| i.t < t + COOLDOWN) {
            return self.cancel("cooldown");
        }
        let Some((yaw, seen)) = i.hmd else { return self.cancel("head tracking unavailable"); };
        if !yaw.is_finite() || !seen.is_finite() || seen > i.t + EPS || i.t - seen > FRESH + EPS {
            return self.cancel("head tracking stale");
        }
        let Some(chest) = i.chest.and_then(|q| unit(q).ok()) else { return self.cancel("chest unavailable"); };
        let mut feet = [None; 2];
        for side in 0..2 {
            if let Some(q) = i.feet[side] {
                let Ok(q) = unit(q) else { return self.cancel("invalid foot orientation"); };
                feet[side] = Some(q);
                if !i.foot_accel[side].is_some_and(|v| v.is_finite() && v >= 0. && v <= 0.8) {
                    return self.cancel("foot moving or acceleration unavailable");
                }
            }
        }
        if let Some(b) = self.baseline {
            if b.y_up != i.y_up || angle(chest, b.chest) > 5_f64.to_radians() {
                return self.cancel("chest moved");
            }
            for side in 0..2 {
                match (feet[side], b.feet[side]) {
                    (Some(a), Some(b)) if angle(a, b) <= 4_f64.to_radians() => {},
                    (None, None) => {},
                    _ => return self.cancel("foot moved or assignment changed"),
                }
            }
        }
        if self.started.is_some_and(|t| seen - t > 45.)
            || self.last_extreme.is_some_and(|t| seen - t > 5.) {
            return self.cancel("gesture timed out");
        }
        let mut dt = 0.;
        if let Some((previous_yaw, previous_seen)) = self.last_hmd {
            if seen < previous_seen { return self.cancel("head timestamp went backwards"); }
            if seen == previous_seen { return false; }
            dt = seen - previous_seen;
            let delta = wrap(yaw - previous_yaw);
            if dt > FRESH + EPS || delta.abs() > 45_f64.to_radians()
                || delta.abs() / dt > 540_f64.to_radians() {
                return self.cancel("head tracking discontinuity");
            }
            self.unrolled += delta;
        } else {
            self.unrolled = yaw;
        }
        self.last_hmd = Some((yaw, seen));
        if self.baseline.is_none() {
            self.baseline = Some(Baseline { chest, feet, y_up: i.y_up, center: self.unrolled, since: seen });
            self.last_relative = Some(0.);
            self.reason = "hold forward still";
            return false;
        }
        let b = self.baseline.unwrap();
        let up = if i.y_up { [0., 1., 0.] } else { [0., 0., 1.] };
        let Some(chest_heading) = crate::mounting::heading(chest, b.chest, up) else {
            return self.cancel("chest heading unavailable");
        };
        let relative = self.unrolled - b.center - chest_heading;
        let speed = self.last_relative.map_or(0., |last| (relative - last).abs() / dt.max(EPS));
        self.last_relative = Some(relative);
        if !self.armed {
            if relative.abs() > 3_f64.to_radians() || speed > 20_f64.to_radians() {
                return self.cancel("hold forward still");
            }
            if seen - b.since + EPS >= ARM_DWELL {
                self.armed = true;
                self.reason = "ready for four head sweep pairs";
            }
            return false;
        }
        if relative.abs() > 120_f64.to_radians() {
            return self.cancel("head sweep exceeded comfortable range");
        }
        if self.extremes == REQUIRED_EXTREMES {
            if relative.abs() <= 15_f64.to_radians() && speed <= 15_f64.to_radians() {
                if self.forward_anchor.is_some_and(|anchor| (relative - anchor).abs() > 3_f64.to_radians()) {
                    self.forward_since = None;
                    self.forward_anchor = None;
                }
                self.forward_anchor.get_or_insert(relative);
                let since = *self.forward_since.get_or_insert(seen);
                if seen - since + EPS >= FORWARD_DWELL {
                    self.last_completed = Some(i.t);
                    self.reset();
                    self.reason = "cooldown";
                    return true;
                }
                self.reason = "hold forward still to finish";
            } else {
                self.forward_since = None;
                self.forward_anchor = None;
                self.reason = "return forward to finish";
            }
            return false;
        }
        let side = if relative >= 70_f64.to_radians() { 1 }
            else if relative <= -70_f64.to_radians() { -1 } else { 0 };
        if side != 0 && side != self.last_side && speed <= 30_f64.to_radians() {
            let since = match self.candidate {
                Some((candidate, since, anchor)) if candidate == side
                    && (relative - anchor).abs() <= 3_f64.to_radians() => since,
                _ => { self.candidate = Some((side, seen, relative)); seen },
            };
            if seen - since + EPS >= END_DWELL {
                self.extremes += 1;
                self.last_side = side;
                self.candidate = None;
                self.started.get_or_insert(seen);
                self.last_extreme = Some(seen);
                self.reason = if self.extremes == REQUIRED_EXTREMES { "return forward to finish" }
                    else { "look to the opposite side" };
            }
        } else {
            self.candidate = None;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gait::IDENTITY;
    struct Run { d: Detector, i: Input, yaw: f64, events: usize }
    impl Run {
        fn new(center_degrees: f64) -> Self {
            let mut run = Self { d: Detector::new(Settings { enabled: true }).unwrap(),
                i: Input { t: 0., hmd: None, chest: Some(IDENTITY), feet: [Some(IDENTITY); 2],
                    foot_accel: [Some(0.); 2], y_up: true, ready: true },
                yaw: center_degrees.to_radians(), events: 0 };
            run.hold(0.6);
            assert!(run.d.armed);
            run
        }
        fn sample(&mut self, yaw: f64) {
            self.i.t += 0.02;
            self.yaw = yaw;
            self.i.hmd = Some((wrap(yaw), self.i.t));
            if self.d.update(&self.i) { self.events += 1; }
        }
        fn hold(&mut self, seconds: f64) {
            for _ in 0..(seconds / 0.02).round() as usize { self.sample(self.yaw); }
        }
        fn move_to(&mut self, degrees: f64) {
            let start = self.yaw;
            let target = degrees.to_radians();
            let steps = ((target - start).abs() / 4_f64.to_radians()).ceil() as usize;
            for step in 1..=steps { self.sample(start + (target - start) * step as f64 / steps as f64); }
            self.hold(0.12);
        }
        fn pairs(&mut self, n: usize, center: f64, first: f64) {
            for _ in 0..n { self.move_to(center + first * 90.); self.move_to(center - first * 90.); }
        }
        fn finish(&mut self, center: f64) { self.move_to(center); self.hold(0.7); }
    }
    fn yaw_q(degrees: f64) -> Q {
        let h = degrees.to_radians() / 2.; [h.cos(), 0., h.sin(), 0.]
    }
    #[test]
    fn strict_settings_default_disabled_and_roundtrip() {
        assert!(!Settings::default().enabled);
        let s = Settings { enabled: true };
        assert_eq!(Settings::from_json(&s.to_json()).unwrap(), s);
        assert_eq!(Settings::from_json(&json!({})).unwrap(), Settings::default());
        for invalid in [json!(null), json!([]), json!({"enabled":1}), json!({"extra":false})] {
            assert!(Settings::from_json(&invalid).is_err());
        }
    }
    #[test]
    fn exactly_four_pairs_then_forward_still_once_either_first_side() {
        for side in [-1., 1.] {
            let mut r = Run::new(0.);
            r.pairs(4, 0., side);
            assert_eq!(r.events, 0);
            assert_eq!(r.d.extremes, 8);
            r.hold(0.8);
            assert_eq!(r.events, 0, "finishing at an extreme must not fire");
            r.move_to(0.);
            r.hold(0.3);
            assert_eq!(r.events, 0, "forward dwell is required");
            r.hold(0.4);
            assert_eq!(r.events, 1);
            r.hold(0.8);
            assert_eq!(r.events, 1);
        }
    }
    #[test]
    fn three_pairs_and_ordinary_looking_do_not_complete() {
        let mut r = Run::new(0.);
        r.pairs(3, 0., 1.);
        r.finish(0.);
        assert_eq!(r.events, 0);
        assert_eq!(r.d.extremes, 6);
        r.hold(5.2);
        assert_eq!(r.d.extremes, 0);
        for _ in 0..5 { r.move_to(40.); r.move_to(-40.); }
        r.finish(0.);
        assert_eq!(r.events, 0);
    }
    #[test]
    fn wrap_boundary_does_not_interrupt_gesture() {
        for center in [-179., 179.] {
            let mut r = Run::new(center);
            r.pairs(4, center, 1.);
            r.finish(center);
            assert_eq!(r.events, 1);
        }
    }
    #[test]
    fn duplicate_hmd_frames_cannot_hold_endpoint_or_finish() {
        let mut r = Run::new(0.);
        r.move_to(90.);
        assert_eq!(r.d.extremes, 1);
        r.move_to(-65.);
        r.sample(-72_f64.to_radians());
        let old_seen = r.i.hmd.unwrap().1;
        for _ in 0..8 { r.i.t += 0.02; assert!(!r.d.update(&r.i)); }
        assert_eq!(r.d.extremes, 1);
        assert_eq!(r.d.last_hmd.unwrap().1, old_seen);
        r.i.t += 0.2;
        assert!(!r.d.update(&r.i));
        assert_eq!(r.d.extremes, 0);
        assert_eq!(r.events, 0);
    }
    #[test]
    fn repeated_same_side_counts_once_and_requires_opposite_extremes() {
        let mut r = Run::new(0.);
        for _ in 0..8 { r.move_to(90.); r.move_to(0.); }
        assert_eq!(r.events, 0);
        assert!(r.d.extremes <= 1);
        let mut r = Run::new(0.);
        r.move_to(90.); r.move_to(-90.); r.move_to(-80.); r.move_to(-90.);
        assert_eq!(r.d.extremes, 2);
    }
    #[test]
    fn moving_through_end_zone_without_a_hold_never_counts() {
        let mut r = Run::new(0.);
        for target in [90_f64, -90., 90., -90.] {
            let start = r.yaw;
            let target = target.to_radians();
            for step in 1..=45 { r.sample(start + (target - start) * step as f64 / 45.); }
        }
        assert_eq!(r.d.extremes, 0);
        assert_eq!(r.events, 0);
    }
    #[test]
    fn tracking_jump_gap_and_unavailable_input_cancel() {
        for failure in 0..4 {
            let mut r = Run::new(0.);
            r.pairs(2, 0., 1.);
            match failure {
                0 => r.sample(20_f64.to_radians()),
                1 => { r.i.t += 0.4; r.sample(r.yaw); },
                2 => { r.i.hmd = None; assert!(!r.d.update(&r.i)); },
                _ => { r.i.ready = false; assert!(!r.d.update(&r.i)); },
            }
            assert_eq!(r.d.extremes, 0);
            assert_eq!(r.events, 0);
        }
    }
    #[test]
    fn fixed_chest_baseline_rejects_gradual_body_rotation() {
        let mut r = Run::new(0.);
        r.pairs(1, 0., 1.);
        for step in 1..=12 { r.i.chest = Some(yaw_q(step as f64 * 0.5)); r.sample(r.yaw); }
        assert_eq!(r.d.extremes, 0);
        assert_eq!(r.events, 0);
    }
    #[test]
    fn foot_rotation_acceleration_missing_and_assignment_change_cancel() {
        for failure in 0..5 {
            let mut r = Run::new(0.);
            r.pairs(1, 0., 1.);
            match failure {
                0 => r.i.feet[0] = Some(yaw_q(5.)),
                1 => r.i.foot_accel[1] = Some(0.81),
                2 => r.i.foot_accel[0] = None,
                3 => r.i.ready = false,
                _ => r.i.feet[0] = None,
            }
            r.sample(r.yaw);
            assert_eq!(r.d.extremes, 0);
            assert_eq!(r.events, 0);
        }
    }
    #[test]
    fn unassigned_feet_are_optional_and_cooldown_requires_new_sequence() {
        let mut r = Run::new(0.);
        r.i.feet = [None; 2]; r.i.foot_accel = [None; 2]; r.d.reset(); r.hold(0.6);
        r.pairs(4, 0., 1.); r.finish(0.);
        assert_eq!(r.events, 1);
        r.pairs(4, 0., -1.); r.finish(0.);
        assert_eq!(r.events, 1);
        r.hold(11.);
        assert_eq!(r.events, 1);
        r.pairs(4, 0., -1.); r.finish(0.);
        assert_eq!(r.events, 2);
    }
    #[test]
    fn reset_preserves_settings_and_cooldown_toggle_clears_attempt() {
        let mut r = Run::new(0.);
        r.pairs(1, 0., 1.);
        r.d.reset(); assert!(r.d.settings().enabled); assert_eq!(r.d.extremes, 0);
        r.d.set_settings(Settings::default()).unwrap();
        r.pairs(4, 0., 1.); r.finish(0.);
        assert_eq!(r.events, 0);
        r.d.set_settings(Settings { enabled: true }).unwrap();
        r.hold(0.6); r.pairs(4, 0., 1.); r.finish(0.);
        assert_eq!(r.events, 1);
        let completed = r.d.last_completed;
        r.d.reset();
        assert_eq!(r.d.last_completed, completed);
        r.sample(0.); assert_eq!(r.d.snapshot()["phase"], "cooldown");
    }
}
