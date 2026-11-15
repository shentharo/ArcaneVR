//! Foot intent evidence for step-rhythm locomotion: is this foot movement a walking
//! step, or the feet repositioning because the body is turning?
//!
//! Physical recordings of walking on the spot and of turning on the spot separate
//! cleanly on one measurement: how far the foot twists about the vertical between
//! lifting and landing. A walking step lands pointing where it started (a few
//! degrees of sensor wobble). A turning step lifts, twists tens of degrees to follow
//! the chest, and lands. The chest tells us early that a turn may be under way, so a
//! step taken while the chest is rotating is only counted once the foot has shown it
//! is not twisting. Ordinary chest sway while walking never blocks movement.
use crate::gait::{unit, Q};
use serde_json::{json, Value};
use std::collections::VecDeque;

const WINDOW: f64 = 0.15;
const READING_AGE: f64 = 0.12;
/// Evidence must persist this long before a step can count at all.
const EVIDENCE_DELAY: f64 = 0.06;
/// Chest heading change over `CHEST_WINDOW` that makes step checks careful.
const CHEST_WINDOW: f64 = 0.4;
const CHEST_TURN_DEGREES: f64 = 8.;
/// A faster chest movement is noticed sooner: this many degrees within `CHEST_QUICK_WINDOW`.
const CHEST_QUICK_WINDOW: f64 = 0.1;
const CHEST_QUICK_DEGREES: f64 = 3.;
/// A foot twisting this far since it lifted is repositioning for a turn, however
/// high it lifted. Recorded turning steps twist 50 to 150 degrees.
const TURN_TWIST_DEGREES: f64 = 45.;
/// Between this and `TURN_TWIST_DEGREES` a twist only counts as a turn when it
/// dominates the lift (`pivot_ratio`), so walking round a curve keeps moving.
const TWIST_MIN_DEGREES: f64 = 12.;
/// With a steady chest, a step whose foot has twisted less than this counts at once.
/// Recorded walking steps wobble up to about 9 degrees; turning steps pass 50.
const QUICK_TWIST_DEGREES: f64 = 10.;
/// The foot's heading is "settled" when it changed less than this over `WINDOW`.
const SETTLED_TWIST_DEGREES: f64 = 8.;
/// While the chest is turning, wait this long before trusting a quiet foot.
const CAREFUL_DELAY: f64 = 0.2;
/// A lift is followed for this long; an undecided step then counts as walking.
const STEP_TIMEOUT: f64 = 0.6;
/// After a turning step, walking steps are ignored for this long. Turning steps
/// alternate feet, and the supporting foot rolls enough to look like a small step.
pub(crate) const TURN_HOLD: f64 = 0.5;

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
/// The foot pose just before a step began, and how far it has moved since.
#[derive(Clone, Debug)]
struct Lift { t: f64, yaw: f64, gravity: [f64; 3], swing: f64 }
#[derive(Clone, Debug, Default)]
struct Foot {
    poses: VecDeque<Pose>,
    previous: Option<(Q, f64)>,
    yaw: f64,
    tilt: Option<(f64, f64)>,
    accel: Option<(f64, f64)>,
    candidate_since: Option<f64>,
    lift: Option<Lift>,
    /// How much of a walking step the last finished step was (see `curve_scale`).
    walked: Option<f64>,
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
/// Movement since a step began: foot twist in signed degrees, lift tilt in
/// degrees, and seconds since the step began.
#[derive(Clone, Copy, Debug)]
struct Step { twist: f64, swing: f64, age: f64 }
/// What a lifted foot has turned out to be doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict { Wait, Walk, Turn }
#[derive(Clone, Debug, Default)]
pub struct Guard {
    feet: [Foot; 2],
    body_previous: Option<(Q, f64)>,
    body_poses: VecDeque<(f64, f64)>,
    body_yaw: f64,
    turn_until: Option<f64>,
    walk_steps: u64,
    turn_steps: u64,
}
/// 0 below `from`, 1 above `to`, smooth between.
fn smoothstep(x: f64, from: f64, to: f64) -> f64 {
    let f = ((x - from) / (to - from)).clamp(0., 1.);
    f * f * (3. - 2. * f)
}
/// Share of a step that is walking rather than turning, from its twist so far.
fn walked(twist_degrees: f64) -> f64 {
    1. - smoothstep(twist_degrees.abs(), TWIST_MIN_DEGREES, TURN_TWIST_DEGREES)
}
fn angle_degrees(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>().clamp(-1., 1.).acos().to_degrees()
}
impl Guard {
    pub fn clear(&mut self) { *self = Self::default(); }
    /// Successive world-space twists cancel the arbitrary physical sensor mount.
    /// Accumulation also handles +/-pi wrap and either quaternion representation.
    pub fn body_orientation(&mut self, q: Q, t: f64, y_up: bool) {
        if !t.is_finite() { return; }
        let Ok(q) = unit(q) else { return; };
        if let Some((previous, at)) = self.body_previous {
            if t <= at { return; }
            if t - at > CHEST_WINDOW {
                self.body_poses.clear();
            } else {
                let up = if y_up { [0., 1., 0.] } else { [0., 0., 1.] };
                if let Some(delta) = crate::mounting::heading(q, previous, up) {
                    self.body_yaw += delta.to_degrees();
                }
            }
        }
        self.body_previous = Some((q, t));
        self.body_poses.push_back((t, self.body_yaw));
        while self.body_poses.front().is_some_and(|(at, _)| t - at > CHEST_WINDOW) { self.body_poses.pop_front(); }
        while self.body_poses.len() > 512 { self.body_poses.pop_front(); }
    }
    /// Chest heading change (degrees) over the last `CHEST_WINDOW`; 0 when stale.
    fn chest_change(&self, t: f64) -> f64 { self.chest_change_over(t, CHEST_WINDOW) }
    fn chest_change_over(&self, t: f64, window: f64) -> f64 {
        if !self.body_previous.is_some_and(|(_, at)| t >= at && t - at <= READING_AGE) { return 0.; }
        self.body_poses.iter().find(|(at, _)| t - at <= window).map_or(0., |(_, yaw)| self.body_yaw - yaw)
    }
    /// The chest is rotating enough that the next step may be a turning step.
    pub fn chest_turning(&self, t: f64) -> bool {
        self.chest_change(t).abs() >= CHEST_TURN_DEGREES
            || self.chest_change_over(t, CHEST_QUICK_WINDOW).abs() >= CHEST_QUICK_DEGREES
    }
    /// True for `TURN_HOLD` after the last sign of a turning step.
    pub fn turning(&self, t: f64) -> bool {
        t.is_finite() && self.turn_until.is_some_and(|until| t < until)
    }
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
                f.lift = None;
                f.walked = None;
            } else {
                let up = if y_up { [0., 1., 0.] } else { [0., 0., 1.] };
                if let Some(delta) = crate::mounting::heading(q, previous, up) { f.yaw += delta; }
            }
        }
        f.previous = Some((q, t));
        f.tilt = Some((tilt.max(0.), t));
        let gravity = crate::gait::gravity(q, y_up);
        if let Some(lift) = &mut f.lift {
            if t - lift.t > STEP_TIMEOUT {
                f.walked = Some(walked((f.yaw - lift.yaw).to_degrees()));
                f.lift = None;
            } else { lift.swing = lift.swing.max(angle_degrees(gravity, lift.gravity)); }
        }
        f.poses.push_back(Pose { t, gravity, yaw: f.yaw });
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
    /// A step began on this foot at `t`. Remember the pose it started from: the
    /// oldest pose still in the window, so a twist that began just before the
    /// step registered is included.
    pub fn lift(&mut self, side: usize, t: f64) {
        if side >= 2 || !t.is_finite() { return; }
        let f = &mut self.feet[side];
        if let Some(previous) = &f.lift { f.walked = Some(walked((f.yaw - previous.yaw).to_degrees())); }
        let Some(start) = f.poses.front() else { return; };
        let mut lift = Lift { t, yaw: start.yaw, gravity: start.gravity, swing: 0. };
        for p in &f.poses { lift.swing = lift.swing.max(angle_degrees(p.gravity, lift.gravity)); }
        f.lift = Some(lift);
    }
    /// Twist (signed degrees) and lift tilt (degrees) since this foot's step began.
    fn since_lift(&self, side: usize, t: f64) -> Option<Step> {
        let f = &self.feet[side];
        let lift = f.lift.as_ref()?;
        if !(t >= lift.t && t - lift.t <= STEP_TIMEOUT) { return None; }
        Some(Step { twist: (f.yaw - lift.yaw).to_degrees(), swing: lift.swing, age: t - lift.t })
    }
    /// A large twist is always a turning step. A moderate twist is one when it
    /// dominates the lift: the foot turned more than it stepped.
    fn is_turn_twist(step: &Step, settings: &Settings) -> bool {
        let twist = step.twist.abs();
        twist >= TURN_TWIST_DEGREES
            || (twist >= TWIST_MIN_DEGREES && twist > settings.pivot_ratio * step.swing.max(settings.min_swing_degrees))
    }
    /// Decide what the step that began with `lift` is. Also notes turning so that
    /// `turning` holds across the alternating feet of a turn.
    pub fn verdict(&mut self, side: usize, t: f64, settings: &Settings) -> Verdict {
        if side >= 2 || !t.is_finite() { return Verdict::Wait; }
        if !settings.enabled { return Verdict::Walk; }
        let Some(step) = self.since_lift(side, t) else { return Verdict::Walk; };
        let (twist, age) = (step.twist, step.age);
        if Self::is_turn_twist(&step, settings) {
            self.turn_until = Some(t + TURN_HOLD);
            return Verdict::Turn;
        }
        if self.turning(t) { return Verdict::Turn; }
        let f = &self.feet[side];
        let (_, recent) = Self::spans(f, t);
        let settled = recent < SETTLED_TWIST_DEGREES;
        let twist = twist.abs();
        if age >= EVIDENCE_DELAY && twist < QUICK_TWIST_DEGREES && !self.chest_turning(t) {
            return Verdict::Walk;
        }
        if age >= CAREFUL_DELAY && twist < TWIST_MIN_DEGREES && settled { return Verdict::Walk; }
        if f.candidate_since.is_none() && age >= EVIDENCE_DELAY { return Verdict::Walk; }
        Verdict::Wait
    }
    /// Watch both feet for a twist that shows a turning step, including after the
    /// step was already counted or when no step registered. Returns true while
    /// walking steps must be ignored.
    pub fn watch(&mut self, t: f64, settings: &Settings) -> bool {
        if !settings.enabled || !t.is_finite() { return false; }
        for side in 0..2 {
            let turning = match self.since_lift(side, t) {
                Some(step) => self.feet[side].candidate_since.is_some() && Self::is_turn_twist(&step, settings),
                // A foot swivelling on the floor never lifts, so no step anchors it.
                None => {
                    let e = self.evidence(side, t, settings);
                    e.pivot && e.yaw_degrees >= TWIST_MIN_DEGREES
                }
            };
            if turning { self.turn_until = Some(t + TURN_HOLD); }
        }
        self.turning(t)
    }
    /// Speed scale for walking round a curve: 1 while steps land where they
    /// lifted, falling smoothly to 0 as each step becomes more twist than walk.
    /// Uses each foot's last finished step and its step in progress, so a tight
    /// curve slows the walk instead of switching it off at a threshold.
    pub fn curve_scale(&self, t: f64, settings: &Settings) -> f64 {
        if !settings.enabled || !t.is_finite() { return 1.; }
        (0..2).map(|side| {
            let running = self.since_lift(side, t).map_or(1., |step| walked(step.twist));
            self.feet[side].walked.unwrap_or(1.).min(running)
        }).fold(1., f64::min)
    }
    /// Forget finished steps once walking has stopped, so a new walk starts at full scale.
    pub fn reset_curve(&mut self) { for f in &mut self.feet { f.walked = None; } }
    pub fn count(&mut self, verdict: Verdict) {
        match verdict {
            Verdict::Walk => self.walk_steps = self.walk_steps.saturating_add(1),
            Verdict::Turn => self.turn_steps = self.turn_steps.saturating_add(1),
            Verdict::Wait => {}
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
        e.ready = f.candidate_since.is_some_and(|at| t >= at && t - at >= EVIDENCE_DELAY);
        e
    }
    fn spans(f: &Foot, t: f64) -> (f64, f64) {
        let poses: Vec<_> = f.poses.iter().filter(|p| t >= p.t && t - p.t <= WINDOW).collect();
        let (mut swing, mut yaw) = (0_f64, 0_f64);
        for (i, a) in poses.iter().enumerate() {
            for b in poses.iter().skip(i + 1) {
                swing = swing.max(angle_degrees(a.gravity, b.gravity));
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
            let step = self.since_lift(side, t);
            let twisting = step.as_ref().is_some_and(|step| Self::is_turn_twist(step, settings));
            let reason = if !settings.enabled { "disabled" } else if !e.fresh { "waiting for fresh orientation" }
                else if twisting { "turning step" }
                else if e.pivot { "yaw-dominant pivot" } else if e.qualified && !e.ready { "checking swing" } else if e.qualified { "swing qualified" } else { "below minimum swing" };
            json!({"swing_degrees":e.swing_degrees,"yaw_degrees":e.yaw_degrees,"qualified":e.qualified,
                "pivot":e.pivot || twisting,"fresh":e.fresh,"ready":e.ready,"reason":reason,
                "step_twist_degrees":step.as_ref().map_or(0., |s| s.twist),"step_lift_degrees":step.as_ref().map_or(0., |s| s.swing)})
        }).collect();
        let hold = self.turn_until.map_or(0., |until| (until - t).max(0.));
        let body_fresh = self.body_previous.is_some_and(|(_, at)| t >= at && t - at <= READING_AGE);
        let chest = self.chest_change(t);
        let reason = if !settings.enabled { "disabled" }
            else if hold > 0. { "turning step: translation paused" }
            else if !body_fresh { "waiting for fresh torso orientation" }
            else if self.chest_turning(t) { "chest turning: checking each step" }
            else { "translation allowed" };
        let turn = json!({"reason":reason,"fresh":body_fresh,"hold_seconds":hold,"suppressed":suppressed,
            "chest_change_degrees":chest,"chest_window_seconds":CHEST_WINDOW,"chest_turn_degrees":CHEST_TURN_DEGREES,
            "turn_twist_degrees":TURN_TWIST_DEGREES,"twist_minimum_degrees":TWIST_MIN_DEGREES,
            "quiet_hold_seconds":TURN_HOLD,"walk_steps":self.walk_steps,"turn_steps":self.turn_steps,
            "curve_scale":self.curve_scale(t, settings)});
        // `body_turn` keeps its 1.1.20 name for existing readers of this state.
        let mut body_turn = turn.clone();
        body_turn["yaw_rate_degrees_per_second"] = json!(chest / CHEST_WINDOW);
        body_turn["pivot_hold_seconds"] = json!(hold);
        json!({"settings":settings.to_json(),"feet":feet,"pivot_suppressed":suppressed,"window_seconds":WINDOW,
            "evidence_delay_seconds":EVIDENCE_DELAY,"turn":turn,"body_turn":body_turn,
            "summary":{"walk_steps":self.walk_steps,"turn_steps":self.turn_steps}})
    }
}

/// Chest heading change over `CHEST_FADE_WINDOW` where single-sensor steps start
/// to fade, and where they stop. Recorded walking on the spot stays under 9
/// degrees; recorded turning on the spot is typically 20 to 50, up to 85.
const CHEST_FADE_WINDOW: f64 = 0.5;
const CHEST_FADE_FROM_DEGREES: f64 = 10.;
const CHEST_FADE_TO_DEGREES: f64 = 25.;
/// A turn made of steps slows between them; keep steps off this long after it.
const CHEST_FADE_HOLD: f64 = 0.4;

/// Turning seen from the chest alone, for modes where the chest is the only
/// sensor and its wobble or bounce stands in for footsteps. Turning on the spot
/// wobbles the chest as much as walking does, but walking sway swings back
/// within a stride while a turn keeps going the same way.
#[derive(Clone, Debug, Default)]
pub struct ChestTurn {
    previous: Option<(Q, f64)>,
    poses: VecDeque<(f64, f64)>,
    yaw: f64,
    hold_until: Option<f64>,
}
impl ChestTurn {
    pub fn clear(&mut self) { *self = Self::default(); }
    pub fn orientation(&mut self, q: Q, t: f64, y_up: bool) {
        if !t.is_finite() { return; }
        let Ok(q) = unit(q) else { return; };
        if let Some((previous, at)) = self.previous {
            if t <= at { return; }
            if t - at > CHEST_FADE_WINDOW { self.poses.clear(); self.hold_until = None; }
            else {
                let up = if y_up { [0., 1., 0.] } else { [0., 0., 1.] };
                if let Some(delta) = crate::mounting::heading(q, previous, up) { self.yaw += delta.to_degrees(); }
            }
        }
        self.previous = Some((q, t));
        self.poses.push_back((t, self.yaw));
        while self.poses.front().is_some_and(|(at, _)| t - at > CHEST_FADE_WINDOW) { self.poses.pop_front(); }
        while self.poses.len() > 512 { self.poses.pop_front(); }
        if self.change(t).abs() >= CHEST_FADE_TO_DEGREES { self.hold_until = Some(t + CHEST_FADE_HOLD); }
    }
    /// Net chest heading change (signed degrees) over the window; 0 when stale.
    pub fn change(&self, t: f64) -> f64 {
        if !self.previous.is_some_and(|(_, at)| t >= at && t - at <= READING_AGE) { return 0.; }
        self.poses.front().map_or(0., |(_, yaw)| self.yaw - yaw)
    }
    /// 1 when not turning, 0 while turning on the spot, smooth between.
    pub fn factor(&self, t: f64) -> f64 {
        if !t.is_finite() { return 1.; }
        if self.hold_until.is_some_and(|until| t < until) { return 0.; }
        1. - smoothstep(self.change(t).abs(), CHEST_FADE_FROM_DEGREES, CHEST_FADE_TO_DEGREES)
    }
    pub fn snapshot(&self, t: f64) -> Value {
        json!({"factor":self.factor(t),"change_degrees":self.change(t),"window_seconds":CHEST_FADE_WINDOW,
            "fade_from_degrees":CHEST_FADE_FROM_DEGREES,"fade_to_degrees":CHEST_FADE_TO_DEGREES,"hold_seconds":CHEST_FADE_HOLD})
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
    fn pose(yaw: f64, lift: f64) -> Q {
        let (y, l) = (yaw.to_radians() / 2., lift.to_radians() / 2.);
        // Heading about world Z applied after a lift about X.
        [y.cos() * l.cos(), y.cos() * l.sin(), y.sin() * l.sin(), y.sin() * l.cos()]
    }
    /// Feed one foot a step lasting `seconds` that lifts `lift` degrees and lands
    /// twisted by `twist` degrees, with the chest turning `chest` degrees meanwhile.
    /// Returns the first verdict with its delay, and when turning was noticed.
    fn step(g: &mut Guard, s: &Settings, seconds: f64, lift: f64, twist: f64, chest: f64) -> (Verdict, f64, Option<f64>) {
        for n in 0..20 { let t = n as f64 * 0.01; g.orientation(0, pose(0., 0.), 0., t, false); g.body_orientation(pose(0., 0.), t, false); }
        let start = 0.2;
        g.lift(0, start);
        let (mut first, mut noticed) = (None, None);
        for n in 0..100 {
            let t = start + n as f64 * 0.01;
            let f = ((t - start) / seconds).min(1.);
            let ease = f * f * (3. - 2. * f);
            g.orientation(0, pose(twist * ease, lift * (f * std::f64::consts::PI).sin()), if f < 1. { 3. } else { 0. }, t, false);
            g.body_orientation(pose(chest * ease, 0.), t, false);
            if first.is_none() {
                let verdict = g.verdict(0, t, s);
                if verdict != Verdict::Wait { first = Some((verdict, t - start)); }
            }
            if g.watch(t, s) { noticed.get_or_insert(t - start); }
        }
        let (verdict, after) = first.unwrap_or((Verdict::Wait, 1.));
        (verdict, after, noticed)
    }
    #[test]
    fn a_step_that_lands_where_it_lifted_is_walking_and_counts_quickly() {
        let s = Settings::default();
        for (lift, wobble) in [(8., 0.), (25., 5.), (40., -9.)] {
            let (verdict, after, turning) = step(&mut Guard::default(), &s, 0.3, lift, wobble, 0.);
            assert_eq!(verdict, Verdict::Walk, "lift {lift} wobble {wobble}");
            assert!(after <= 0.08, "walking step waited {after:.2}s");
            assert_eq!(turning, None, "lift {lift} wobble {wobble} was called a turn");
        }
    }
    #[test]
    fn a_step_that_twists_is_turning_whatever_the_chest_does() {
        let s = Settings::default();
        // Recorded turning steps: 10-34 degrees of lift, 50-155 degrees of twist.
        for (lift, twist, chest) in [(11., 85., 65.), (34., -155., -98.), (16., 55., 20.), (10., 50., 0.)] {
            let (_, _, turning) = step(&mut Guard::default(), &s, 0.6, lift, twist, chest);
            assert!(turning.is_some_and(|after| after < 0.45), "lift {lift} twist {twist}: noticed {turning:?}");
        }
        // The chest leading the feet, as recorded, means the step is never counted.
        let mut g = Guard::default();
        for n in 0..20 { let t = n as f64 * 0.01; g.orientation(0, pose(0., 0.), 0., t, false); g.body_orientation(pose(t * 100., 0.), t, false); }
        g.lift(0, 0.2);
        for n in 20..80 {
            let t = n as f64 * 0.01;
            let f = (t - 0.2) / 0.6;
            g.orientation(0, pose(100. * f * f * (3. - 2. * f), 15. * (f * std::f64::consts::PI).sin()), 3., t, false);
            g.body_orientation(pose(t * 100., 0.), t, false);
            assert_ne!(g.verdict(0, t, &s), Verdict::Walk, "turning step counted as walking at {t}");
        }
        assert!(g.turning(0.8));
    }
    #[test]
    fn chest_turning_makes_the_check_careful_but_does_not_block_a_straight_step() {
        let s = Settings::default();
        let (verdict, after, turning) = step(&mut Guard::default(), &s, 0.3, 25., 3., 40.);
        assert_eq!((verdict, turning), (Verdict::Walk, None));
        assert!((CAREFUL_DELAY - 0.011..=0.36).contains(&after), "careful check took {after:.2}s");
    }
    #[test]
    fn a_moderate_twist_is_walking_only_when_the_foot_really_lifted() {
        let s = Settings::default();
        assert_eq!(step(&mut Guard::default(), &s, 0.3, 25., 22., 10.).2, None);
        assert!(step(&mut Guard::default(), &s, 0.3, 5., 22., 10.).2.is_some());
        let mut off = s.clone(); off.enabled = false;
        let (verdict, _, turning) = step(&mut Guard::default(), &off, 0.6, 10., 120., 90.);
        assert_eq!((verdict, turning), (Verdict::Walk, None));
    }
    #[test]
    fn a_foot_swivelling_on_the_floor_without_a_step_is_turning() {
        let (mut g, s) = (Guard::default(), Settings::default());
        for n in 0..60 {
            let t = n as f64 * 0.01;
            g.orientation(1, pose(t * 150., 0.5), 0.1, t, false);
            let turning = g.watch(t, &s);
            if t > 0.15 { assert!(turning, "swivel not noticed at {t}"); }
        }
        for n in 60..200 { let t = n as f64 * 0.01; g.orientation(1, pose(90., 0.5), 0., t, false); g.watch(t, &s); }
        assert!(!g.watch(2., &s));
    }
    #[test]
    fn curve_scale_slows_twisting_steps_smoothly_and_recovers() {
        let s = Settings::default();
        let mut previous = 1.01;
        for twist in [0., 8., 12., 20., 30., 40., 45., 90.] {
            let mut g = Guard::default();
            step(&mut g, &s, 0.3, 30., twist, 0.);
            let scale = g.curve_scale(1.25, &s);
            assert!(scale <= previous && (0. ..=1.).contains(&scale), "twist {twist}: {scale}");
            if twist <= 12. { assert_eq!(scale, 1.); }
            if twist >= 45. { assert_eq!(scale, 0.); }
            previous = scale;
            g.reset_curve();
            assert_eq!(g.curve_scale(1.25, &s), 1.);
        }
        let mut off = s.clone(); off.enabled = false;
        let mut g = Guard::default();
        step(&mut g, &s, 0.3, 30., 40., 0.);
        assert_eq!(g.curve_scale(1.25, &off), 1.);
    }
    #[test]
    fn chest_turn_ignores_walking_sway_and_fades_out_a_real_turn() {
        let mut c = ChestTurn::default();
        // Sway of 8 degrees peak to peak at stride rate: fast, but it swings back.
        for n in 0..600 {
            let t = n as f64 * 0.01;
            c.orientation(pose(4. * (t * 7.).sin(), 0.), t, false);
            assert_eq!(c.factor(t), 1., "sway faded steps at {t}: {}", c.snapshot(t));
        }
        // A stepping turn: 100 degrees a second, pausing briefly between steps.
        let mut yaw = 0.;
        for n in 600..1000 {
            let t = n as f64 * 0.01;
            if (t * 1.25).fract() < 0.75 { yaw += 1.; }
            c.orientation(pose(yaw, 0.), t, false);
            if t > 6.6 { assert_eq!(c.factor(t), 0., "turn let steps through at {t}: {}", c.snapshot(t)); }
        }
        // Steps return soon after the chest settles, and stale data never blocks.
        for n in 1000..1100 { let t = n as f64 * 0.01; c.orientation(pose(yaw, 0.), t, false); }
        assert_eq!(c.factor(10.99), 1.);
        assert_eq!(c.factor(20.), 1.);
        // A gentle curve keeps full speed; a tighter one slows without stopping.
        for (rate, low, high) in [(15., 1., 1.), (35., 0.2, 0.8)] {
            let mut c = ChestTurn::default();
            for n in 0..300 { let t = n as f64 * 0.01; c.orientation(pose(t * rate, 0.), t, false); }
            let f = c.factor(2.99);
            assert!((low..=high).contains(&f), "{rate} deg/s gave {f}");
        }
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
