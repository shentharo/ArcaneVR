//! Step-rhythm locomotion: speed comes from the rhythm of the user's own foot steps.
//!
//! Each foot reports how fast it is tilting (from its orientation, so turning on the
//! spot does not count) and how hard it is accelerating. A step is the rising edge of
//! that activity. Thresholds adapt to the user's own typical step, so light walkers
//! and heavy runners both register. Speed follows step rate, starts on the first step,
//! slows as steps slow down, and stops shortly after the expected next step fails to
//! arrive. Constants are untuned against live walking; replay real recordings to tune.
use std::collections::VecDeque;

/// Foot tilt rate (rad/s) that counts as one unit of activity. Matches the old step threshold.
const TILT_REFERENCE: f64 = 2.0;
/// Foot dynamic acceleration that counts as one unit of activity. Matches the old step threshold.
const ACCEL_REFERENCE: f64 = 1.2;
/// Readings older than this are ignored when combining tilt and acceleration.
const READING_AGE: f64 = 0.12;
/// Activity needed to start a step, before the user's own steps are learned.
const BASE_ON: f64 = 1.0;
/// Learned threshold is this fraction of the user's typical step peak, within these limits.
const LEARNED_ON_FRACTION: f64 = 0.35;
const MAX_ON: f64 = 2.5;
/// A foot cannot step twice within this time.
const FOOT_REFRACTORY: f64 = 0.28;
/// Two steps closer than this (either foot) are one step.
const MIN_INTERVAL: f64 = 0.15;
/// A gap longer than this between steps starts a new walk.
const MAX_INTERVAL: f64 = 1.6;
/// Step rates (steps per second, both feet) for the slowest and full speed.
const MIN_RATE: f64 = 0.6;
const FULL_RATE: f64 = 3.0;
/// Speed after the first step, and the lowest speed while walking.
const START_SPEED: f64 = 0.22;
const MIN_WALKING_SPEED: f64 = 0.2;
/// Speed smoothing time constants (seconds).
const ATTACK: f64 = 0.08;
const FIRST_STEP_ATTACK: f64 = 0.25;
const RELEASE: f64 = 0.1;

#[derive(Clone, Debug, Default)]
struct Foot {
    tilt: f64,
    tilt_at: Option<f64>,
    accel: f64,
    accel_at: Option<f64>,
    active: bool,
    active_since: f64,
    peak: f64,
    typical: Option<f64>,
    last_step: Option<f64>,
    steps: u64,
}
impl Foot {
    fn activity(&self, t: f64) -> f64 {
        let fresh = |at: Option<f64>| at.is_some_and(|a| t >= a && t - a <= READING_AGE);
        let tilt = if fresh(self.tilt_at) { self.tilt / TILT_REFERENCE } else { 0. };
        let accel = if fresh(self.accel_at) { self.accel / ACCEL_REFERENCE } else { 0. };
        tilt.max(accel)
    }
    fn on_threshold(&self) -> f64 {
        self.typical.map_or(BASE_ON, |p| (p * LEARNED_ON_FRACTION).clamp(BASE_ON, MAX_ON))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CadenceState {
    /// Smoothed output speed, 0..1.
    pub speed: f64,
    /// Speed the rhythm currently asks for, before smoothing.
    pub target: f64,
    /// Steps per second (both feet), 0 when not walking.
    pub rate: f64,
    pub steps: [u64; 2],
}

#[derive(Clone, Debug, Default)]
pub struct Cadence {
    feet: [Foot; 2],
    intervals: VecDeque<f64>,
    last_step: Option<(f64, usize)>,
    chain_steps: usize,
    speed: f64,
    last_tick: Option<f64>,
}
impl Cadence {
    /// Forget the walk in progress and the output speed. Learned step sizes are kept.
    pub fn stop(&mut self) {
        self.intervals.clear();
        self.last_step = None;
        self.chain_steps = 0;
        self.speed = 0.;
        for f in &mut self.feet {
            f.active = false;
            f.peak = 0.;
        }
    }
    /// Forget everything, including learned step sizes.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn tilt(&mut self, foot: usize, rate: f64, t: f64) {
        if foot < 2 && rate.is_finite() && t.is_finite() {
            self.feet[foot].tilt = rate.max(0.);
            self.feet[foot].tilt_at = Some(t);
            self.observe(foot, t);
        }
    }
    pub fn acceleration(&mut self, foot: usize, dynamic: f64, t: f64) {
        if foot < 2 && dynamic.is_finite() && t.is_finite() {
            self.feet[foot].accel = dynamic.max(0.);
            self.feet[foot].accel_at = Some(t);
            self.observe(foot, t);
        }
    }
    fn observe(&mut self, side: usize, t: f64) {
        let f = &mut self.feet[side];
        let activity = f.activity(t);
        if f.active {
            f.peak = f.peak.max(activity);
            let release = (f.peak * 0.5).max(0.45).min(f.on_threshold() * 0.9);
            let long_plateau = t - f.active_since > 0.35 && activity < f.peak * 0.6;
            if activity < release || long_plateau {
                f.active = false;
                // Learn this user's typical step size, ignoring tiny and wild peaks.
                let peak = f.peak.min(12.);
                f.typical = Some(f.typical.map_or(peak, |p| p + (peak - p) * 0.2));
                f.peak = 0.;
            }
            return;
        }
        if activity < f.on_threshold() || f.last_step.is_some_and(|l| t - l < FOOT_REFRACTORY) {
            return;
        }
        f.active = true;
        f.active_since = t;
        f.peak = activity;
        f.last_step = Some(t);
        f.steps += 1;
        self.step(t, side);
    }
    fn step(&mut self, t: f64, side: usize) {
        let Some((last, previous)) = self.last_step else {
            self.last_step = Some((t, side));
            self.chain_steps = 1;
            return;
        };
        let dt = t - last;
        if dt < MIN_INTERVAL {
            return;
        }
        // The same foot twice usually means the other foot's step was missed.
        let interval = if previous == side { dt / 2. } else { dt };
        self.last_step = Some((t, side));
        if dt > MAX_INTERVAL || interval < MIN_INTERVAL {
            self.intervals.clear();
            self.chain_steps = 1;
            return;
        }
        self.chain_steps += 1;
        self.intervals.push_back(interval);
        while self.intervals.len() > 4 {
            self.intervals.pop_front();
        }
    }
    fn typical_interval(&self) -> Option<f64> {
        if self.intervals.is_empty() {
            return None;
        }
        let mut v: Vec<f64> = self.intervals.iter().copied().collect();
        v.sort_by(f64::total_cmp);
        Some(if v.len() % 2 == 1 { v[v.len() / 2] } else { (v[v.len() / 2 - 1] + v[v.len() / 2]) / 2. })
    }
    pub fn tick(&mut self, t: f64) -> CadenceState {
        if !t.is_finite() || self.last_tick.is_some_and(|l| t < l) {
            self.stop();
            self.last_tick = t.is_finite().then_some(t);
            return self.state(0., 0.);
        }
        let dt = self.last_tick.map_or(0., |l| (t - l).clamp(0., 0.1));
        self.last_tick = Some(t);
        let (mut target, mut rate) = (0., 0.);
        if let Some((last, _)) = self.last_step {
            let since = t - last;
            match self.typical_interval() {
                None if since <= 0.8 => target = START_SPEED,
                Some(interval) if since <= (interval * 1.4).clamp(0.55, 1.1) => {
                    // Slowing down shows as a longer wait for the next step.
                    rate = 1. / interval.max(since);
                    target = ((rate - MIN_RATE) / (FULL_RATE - MIN_RATE)).clamp(MIN_WALKING_SPEED, 1.);
                }
                // The expected step is late: stop moving, but a step that still
                // arrives within MAX_INTERVAL continues the same walk.
                _ if since > MAX_INTERVAL => self.stop(),
                _ => {}
            }
        }
        let tau = if target > self.speed {
            if self.chain_steps <= 1 { FIRST_STEP_ATTACK } else { ATTACK }
        } else {
            RELEASE
        };
        self.speed += (target - self.speed) * (1. - (-dt / tau).exp());
        if target == 0. && self.speed < 0.04 {
            self.speed = 0.;
        }
        self.state(target, rate)
    }
    fn state(&self, target: f64, rate: f64) -> CadenceState {
        CadenceState { speed: self.speed, target, rate, steps: [self.feet[0].steps, self.feet[1].steps] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const DT: f64 = 0.01;

    /// Simulated feet: each step is a 0.25 s tilt bump plus an acceleration spike.
    fn walk(c: &mut Cadence, from: f64, to: f64, rate: f64, strength: f64, out: &mut Vec<(f64, CadenceState)>) {
        let mut t = from;
        let period = 2. / rate; // per foot
        while t < to {
            for side in 0..2 {
                let phase = ((t - from) - side as f64 * period / 2.).rem_euclid(period);
                let swing = if phase < 0.25 { (phase / 0.25 * std::f64::consts::PI).sin() } else { 0. };
                c.tilt(side, swing * 4. * strength, t);
                let impact = if (0.22..0.27).contains(&phase) { 3. * strength } else { 0.05 };
                c.acceleration(side, impact, t);
            }
            out.push((t, c.tick(t)));
            t += DT;
        }
    }
    fn still(c: &mut Cadence, from: f64, to: f64, noise: f64, out: &mut Vec<(f64, CadenceState)>) {
        let mut t = from;
        while t < to {
            for side in 0..2 {
                let n = noise * ((t * 37. + side as f64).sin() * 0.5 + 0.5);
                c.tilt(side, n * TILT_REFERENCE, t);
                c.acceleration(side, n * ACCEL_REFERENCE, t);
            }
            out.push((t, c.tick(t)));
            t += DT;
        }
    }
    fn mean_speed(out: &[(f64, CadenceState)], from: f64, to: f64) -> (f64, f64) {
        let v: Vec<f64> = out.iter().filter(|(t, _)| *t >= from && *t < to).map(|(_, s)| s.speed).collect();
        let m = v.iter().sum::<f64>() / v.len() as f64;
        let sd = (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / v.len() as f64).sqrt();
        (m, sd)
    }

    #[test]
    fn standing_still_with_small_movements_never_moves() {
        let (mut c, mut out) = (Cadence::default(), vec![]);
        still(&mut c, 0., 30., 0.8, &mut out);
        assert!(out.iter().all(|(_, s)| s.speed == 0.), "moved while standing");
    }
    #[test]
    fn starts_within_one_step_and_stops_soon_after_the_last() {
        let (mut c, mut out) = (Cadence::default(), vec![]);
        still(&mut c, 0., 1., 0., &mut out);
        walk(&mut c, 1., 6., 1.8, 1., &mut out);
        still(&mut c, 6., 9., 0., &mut out);
        let started = out.iter().find(|(_, s)| s.speed > 0.1).unwrap().0;
        assert!(started - 1. < 0.4, "start took {:.2}s", started - 1.);
        let last_step = 6. - (6. - 1.) % (1. / 1.8);
        let stopped = out.iter().rev().find(|(_, s)| s.speed > 0.).unwrap().0;
        assert!(stopped - last_step < 1.1, "stop took {:.2}s after last step", stopped - last_step);
    }
    #[test]
    fn faster_steps_always_mean_faster_movement() {
        let mut previous = 0.;
        for rate in [1.0, 1.4, 1.8, 2.2, 2.6, 3.2] {
            let (mut c, mut out) = (Cadence::default(), vec![]);
            walk(&mut c, 0., 8., rate, 1., &mut out);
            let (m, _) = mean_speed(&out, 3., 8.);
            assert!(m > previous, "rate {rate} gave {m:.3}, not above {previous:.3}");
            previous = m;
        }
        assert!(previous > 0.9, "running did not reach full speed: {previous:.3}");
    }
    #[test]
    fn steady_walking_gives_steady_speed() {
        for rate in [1.2, 1.8, 2.6] {
            let (mut c, mut out) = (Cadence::default(), vec![]);
            walk(&mut c, 0., 10., rate, 1., &mut out);
            let (m, sd) = mean_speed(&out, 3., 10.);
            assert!(sd / m < 0.1, "rate {rate}: speed varies {:.0}%", sd / m * 100.);
        }
    }
    #[test]
    fn slow_walking_moves_and_is_not_dropped() {
        let (mut c, mut out) = (Cadence::default(), vec![]);
        walk(&mut c, 0., 10., 1.0, 1., &mut out);
        let moving = out.iter().filter(|(t, s)| *t > 1. && s.speed > 0.1).count() as f64;
        let total = out.iter().filter(|(t, _)| *t > 1.).count() as f64;
        assert!(moving / total > 0.95, "slow walking stalled {:.0}% of the time", (1. - moving / total) * 100.);
    }
    #[test]
    fn light_steps_on_a_walker_are_learned_and_counted() {
        let (mut c, mut out) = (Cadence::default(), vec![]);
        walk(&mut c, 0., 10., 1.8, 0.55, &mut out);
        let (m, _) = mean_speed(&out, 3., 10.);
        assert!(m > 0.35, "light steps gave {m:.3}");
    }
    #[test]
    fn heavy_runner_does_not_lose_steps_after_learning() {
        let (mut c, mut out) = (Cadence::default(), vec![]);
        walk(&mut c, 0., 6., 3.2, 3., &mut out);
        let steps: u64 = out.last().unwrap().1.steps.iter().sum();
        assert!(steps as f64 >= 6. * 3.2 * 0.9, "only {steps} steps counted");
    }
    #[test]
    fn missed_steps_do_not_halve_the_speed() {
        let (mut c, mut out) = (Cadence::default(), vec![]);
        let mut t = 0.;
        let mut n = 0;
        while t < 8. {
            let side = n % 2;
            // Every third right step is missed.
            let missed = side == 1 && (n / 2) % 3 == 0;
            for k in 0..2 {
                let on = k == side && !missed;
                let s = if on { 4. } else { 0. };
                for j in 0..((1. / 2.2) / DT) as usize {
                    let tt = t + j as f64 * DT;
                    c.tilt(k, if j < 25 { s * (j as f64 / 25. * std::f64::consts::PI).sin() } else { 0. }, tt);
                    if k == 1 { out.push((tt, c.tick(tt))); }
                }
            }
            n += 1;
            t += 1. / 2.2;
        }
        let (m, _) = mean_speed(&out, 3., 8.);
        assert!(m > 0.45, "missed steps dropped speed to {m:.3}");
    }
    #[test]
    fn one_accidental_step_barely_moves() {
        let (mut c, mut out) = (Cadence::default(), vec![]);
        walk(&mut c, 0., 0.3, 1.0, 1., &mut out);
        still(&mut c, 0.3, 3., 0., &mut out);
        let distance: f64 = out.iter().map(|(_, s)| s.speed * DT).sum();
        assert!(distance < 0.15, "one step travelled {distance:.3} speed-seconds");
        assert_eq!(out.last().unwrap().1.speed, 0.);
    }
    #[test]
    fn bad_input_is_safe() {
        let mut c = Cadence::default();
        c.tilt(0, f64::NAN, 0.);
        c.acceleration(5, 3., 0.);
        assert_eq!(c.tick(f64::NAN).speed, 0.);
        assert_eq!(c.tick(1.).speed, 0.);
        assert_eq!(c.tick(0.5).speed, 0.);
    }
}
