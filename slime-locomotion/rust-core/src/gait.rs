//! Three-IMU locomotion with continuous acceleration and a legacy step mode.
//!
//! Step rhythm (`cadence`): each foot's activity starts a step, `foot_motion`
//! decides whether that step is walking or the feet following a turn, and only
//! walking steps drive `Cadence`. The chest supplies heading and an early hint
//! that a turn is under way; chest movement alone never pauses translation.
use crate::cadence::Cadence;
use crate::continuous::{MotionFilter, Settings, Telemetry};
pub type Q = [f64; 4]; // internal WXYZ; Slime wire packets are XYZW
pub const IDENTITY: Q = [1., 0., 0., 0.];
pub fn unit(q: Q) -> Result<Q, &'static str> {
    let n = q.iter().map(|v| v * v).sum::<f64>().sqrt();
    if !n.is_finite() || !(0.5..=1.5).contains(&n) {
        return Err("invalid quaternion");
    }
    Ok(q.map(|v| v / n))
}
#[cfg(test)]
fn mul(a: Q, b: Q) -> Q {
    let [w, x, y, z] = a;
    let [v, i, j, k] = b;
    [
        w * v - x * i - y * j - z * k,
        w * i + x * v + y * k - z * j,
        w * j - x * k + y * v + z * i,
        w * k + x * j - y * i + z * v,
    ]
}
pub(crate) fn gravity(q: Q, y_up: bool) -> [f64; 3] {
    let [w, x, y, z] = q;
    if y_up {
        [
            2. * (x * y + w * z),
            1. - 2. * (x * x + z * z),
            2. * (y * z - w * x),
        ]
    } else {
        [
            2. * (x * z - w * y),
            2. * (y * z + w * x),
            1. - 2. * (x * x + y * y),
        ]
    }
}
#[derive(Clone, Debug)]
pub struct Sensor {
    pub q: Q,
    pub seen: f64,
    pub angular: f64,
    rotation_seen: f64,
    ready: bool,
}
impl Default for Sensor {
    fn default() -> Self {
        Self {
            q: IDENTITY,
            seen: -1e9,
            angular: 0.,
            rotation_seen: -1e9,
            ready: true,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Detection {
    pub require_steps: bool,
    pub chest_steps: bool,
    pub continuous: bool,
    /// Step-rhythm speed from both feet (three-sensor mode). Excludes `continuous`.
    pub cadence: bool,
    pub motion: Settings,
    pub foot_motion: crate::foot_motion::Settings,
    pub accel_has_gravity: bool,
    pub threshold: f64,
    pub release: f64,
    pub stale: f64,
    pub stop_after: f64,
    pub y_up: bool,
    pub sign: f64,
}
impl Default for Detection {
    fn default() -> Self {
        Self {
            require_steps: true,
            chest_steps: false,
            continuous: false,
            cadence: false,
            motion: Settings::default(),
            foot_motion: crate::foot_motion::Settings::default(),
            accel_has_gravity: false,
            threshold: 2.,
            release: 0.7,
            stale: 0.25,
            stop_after: 0.55,
            y_up: false,
            sign: 1.,
        }
    }
}
impl Detection {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.foot_motion.validate()?;
        self.motion
            .validate()
            .map_err(|_| "Invalid locomotion settings")?;
        if !self.threshold.is_finite()
            || !(0.1..=20.).contains(&self.threshold)
            || !self.release.is_finite()
            || self.release < 0.
            || self.release >= self.threshold
            || !self.stale.is_finite()
            || !(0.05..=0.5).contains(&self.stale)
            || !self.stop_after.is_finite()
            || !(0.16..=1.5).contains(&self.stop_after)
            || ![-1., 1.].contains(&self.sign)
            || (self.cadence && (self.continuous || self.chest_steps))
        {
            Err("invalid detection settings")
        } else {
            Ok(())
        }
    }
}
pub struct Engine {
    pub motion: MotionFilter,
    pub cadence: Cadence,
    pub cadence_state: crate::cadence::CadenceState,
    foot_intent: crate::foot_motion::Guard,
    pivot_suppressed: bool,
    /// Smoothed slow-down for steps that twist as well as lift (walking a curve).
    curve_scale: f64,
    chest_turn: crate::foot_motion::ChestTurn,
    pub diagnostic_motion: MotionFilter,
    pub diagnostic: Telemetry,
    diagnostic_baseline: Option<[f64;3]>,
    pub preview: Telemetry,
    pub sensors: [Sensor; 3],
    pub cfg: Detection,
    pub calibrated: bool,
    pub armed: bool,
    pub speed: f64,
    pub heading: f64,
    pub steps: Vec<(f64, usize)>,
    pub preview_steps: [u64; 2],
    accel_seen: [f64; 2],
    accel_ready: [bool; 2],
    accel_baseline: [Option<[f64; 3]>; 2],
    pub accel_mode: [&'static str; 2],
    pub accel_dynamic: [f64; 2],
    last_pulse: [f64; 2],
    origin: Q,
    last_tick: f64,
}
impl Engine {
    pub fn new(cfg: Detection) -> Self {
        let mut motion=MotionFilter::new(cfg.motion.clone()).expect("validated motion settings");
        motion.set_single_input(cfg.chest_steps);
        let mut diagnostic_motion=MotionFilter::new(cfg.motion.clone()).expect("validated motion settings");diagnostic_motion.set_single_input(true);
        Self {
            motion, cadence: Cadence::default(), cadence_state: Default::default(),
            foot_intent: Default::default(), pivot_suppressed: false, curve_scale: 1., chest_turn: Default::default(), diagnostic_motion, diagnostic:Default::default(), diagnostic_baseline:None,
            preview: Default::default(),
            sensors: Default::default(),
            cfg,
            calibrated: false,
            armed: false,
            speed: 0.,
            heading: 0.,
            steps: vec![],
            preview_steps: [0; 2],
            accel_seen: [-1e9; 2],
            accel_ready: [true; 2],
            accel_baseline: [None; 2],
            accel_mode: ["uninitialized"; 2],
            accel_dynamic: [0.; 2],
            last_pulse: [-1e9; 2],
            origin: IDENTITY,
            last_tick: 0.,
        }
    }
    /// Live adjustment retains calibration and arming; new steps must establish intent.
    pub fn update_foot_motion_settings(&mut self, settings: crate::foot_motion::Settings) -> Result<(), &'static str> {
        settings.validate()?;
        self.cfg.foot_motion = settings;
        self.foot_intent.clear();
        self.pivot_suppressed = false;
        self.stop_cadence(self.last_tick);
        Ok(())
    }
    /// Single-sensor step modes: 1 normally, falling to 0 while the chest shows
    /// turning on the spot. Always 1 when the feet have their own sensors.
    pub fn chest_turn_factor(&self, t: f64) -> f64 {
        if self.cfg.chest_steps { self.chest_turn.factor(t) } else { 1. }
    }
    pub fn chest_turn_snapshot(&self, t: f64) -> serde_json::Value {
        let mut v = self.chest_turn.snapshot(t);
        v["applies"] = serde_json::json!(self.cfg.chest_steps);
        v["factor"] = serde_json::json!(self.chest_turn_factor(t));
        v
    }
    pub fn foot_motion_snapshot(&self, t: f64) -> serde_json::Value {
        self.foot_intent.snapshot(t, &self.cfg.foot_motion, self.pivot_suppressed)
    }
    fn stop_cadence(&mut self, t: f64) {
        // Cadence::stop retains raw activity. Clear both channels first, on every
        // blocked event/tick, so release cannot replay a pre-turn acceleration.
        for side in 0..2 { self.cadence.sample(side, 0., None, 0., None, t); }
        self.cadence.stop();
        self.cadence_state.speed = 0.;
        self.cadence_state.target = 0.;
        self.cadence_state.rate = 0.;
        self.speed = 0.;
        self.preview.speed = 0.;
        self.preview.raw_speed = 0.;
    }
    /// Decide undecided steps and notice turning. Steps are judged oldest first so
    /// the rhythm always receives them in the order the feet made them.
    fn resolve_steps(&mut self, t: f64) -> bool {
        if !self.cfg.cadence || self.cfg.chest_steps { self.pivot_suppressed = false; return false; }
        use crate::foot_motion::Verdict;
        let settings = self.cfg.foot_motion.clone();
        let pending = [self.cadence.pending(0), self.cadence.pending(1)];
        let order = if pending[1].unwrap_or(f64::INFINITY) < pending[0].unwrap_or(f64::INFINITY) { [1, 0] } else { [0, 1] };
        for side in order {
            if pending[side].is_none() { continue; }
            let verdict = self.foot_intent.verdict(side, t, &settings);
            match verdict {
                Verdict::Walk => { self.cadence.commit(side); }
                Verdict::Turn => { self.cadence.reject(side); }
                Verdict::Wait => break,
            }
            self.foot_intent.count(verdict);
        }
        self.pivot_suppressed = self.foot_intent.watch(t, &settings);
        if self.pivot_suppressed {
            // Keep watching the feet: each step during the turn is still anchored
            // and judged, so marching out of a turn is recognised as walking.
            self.cadence.quiet();
            self.cadence_state.speed = 0.;
            self.cadence_state.target = 0.;
            self.cadence_state.rate = 0.;
            self.speed = 0.;
            self.preview.speed = 0.;
            self.preview.raw_speed = 0.;
        }
        self.pivot_suppressed
    }
    fn feed_cadence(&mut self, role: usize, t: f64) {
        if !self.cfg.cadence { return; }
        let settings = &self.cfg.foot_motion;
        self.cadence.set_deferred(settings.enabled && !self.cfg.chest_steps);
        self.resolve_steps(t);
        let settings = &self.cfg.foot_motion;
        let evidence = self.foot_intent.evidence(role, t, settings);
        // Replace both channels atomically, including on rejected acceleration events.
        let allowed = !settings.enabled || (evidence.qualified && evidence.ready);
        let (tilt, tilt_at, accel, accel_at) = self.foot_intent.sample(role, t, allowed);
        let before = self.cadence.pending(role);
        self.cadence.sample(role, tilt, tilt_at, accel, accel_at, t);
        if let (None, Some(began)) = (before, self.cadence.pending(role)) {
            self.foot_intent.lift(role, began);
            self.resolve_steps(t);
        }
    }
    pub fn fresh(&self, t: f64) -> bool {
        t.is_finite()
            && if self.cfg.require_steps {
                self.sensors
                    .iter()
                    .all(|s| t >= s.seen && t - s.seen < self.cfg.stale)
            } else {
                let s = &self.sensors[2];
                t >= s.seen && t - s.seen < self.cfg.stale
            }
    }
    pub fn reconfigure_chest_preserving_center(&mut self,cfg:Detection)->Result<(), &'static str>{
        cfg.validate()?;if self.cfg.require_steps || cfg.require_steps || self.cfg.y_up!=cfg.y_up || self.cfg.sign!=cfg.sign{return Err("Reference changed");}
        let mut next=Self::new(cfg);next.origin=self.origin;next.sensors=self.sensors.clone();next.calibrated=self.calibrated;next.heading=self.heading;*self=next;Ok(())
    }
    pub fn input_ready(&self, t: f64) -> bool {
        self.fresh(t)
            && (!self.cfg.continuous
                || !self.cfg.require_steps
                || (self.motion.inputs_fresh(t)
                    && self
                        .accel_seen
                        .iter().take(if self.cfg.chest_steps {1} else {2})
                        .all(|seen| t >= *seen && t - *seen < self.cfg.stale.min(0.25))))
    }
    pub fn calibrate(&mut self, t: f64) -> Result<(), &'static str> {
        if !self.fresh(t) {
            return Err("Need fresh samples from both ankles and torso");
        }
        self.origin = self.sensors[2].q;
        self.accel_baseline = [None; 2];
        self.accel_seen = [-1e9; 2];
        self.calibrated = true;
        self.disarm();
        Ok(())
    }
    pub fn arm(&mut self, t: f64) -> Result<(), &'static str> {
        if !self.calibrated || !self.input_ready(t) {
            return Err("Calibrate, then wait for fresh chest and both foot samples");
        }
        self.motion.clear_history();
        self.stop_cadence(t);
        self.foot_intent.clear();
        self.pivot_suppressed = false;
        self.preview = Default::default();
        self.speed = 0.;
        self.steps.clear();
        self.armed = true;
        Ok(())
    }
    /// Change detector response without changing the measured neutral pose.
    pub fn update_motion_settings(&mut self, cfg: Detection) -> Result<(), &'static str> {
        cfg.validate()?;
        if cfg.require_steps != self.cfg.require_steps || cfg.chest_steps != self.cfg.chest_steps {
            return Err("Changing sensor source requires recalibration");
        }
        self.disarm();
        self.motion.set_settings(cfg.motion.clone())?;
        self.diagnostic_motion.set_settings(cfg.motion.clone())?;self.diagnostic=Default::default();
        self.cfg = cfg;
        self.accel_seen = [-1e9; 2];
        self.accel_ready = [true; 2];
        self.last_pulse = [-1e9; 2];
        Ok(())
    }
    pub fn disarm(&mut self) {
        self.armed = false;
        self.speed = 0.;
        self.steps.clear();
        self.motion.reset();
        self.stop_cadence(self.last_tick);
        self.foot_intent.clear();
        self.pivot_suppressed = false;
        self.preview = Default::default();
    }
    pub fn update(&mut self, role: usize, q: Q, t: f64) -> Result<(), &'static str> {
        let q = unit(q)?;
        if role > 2 || !t.is_finite() {
            return Err("invalid sample");
        }
        if self.armed && !self.fresh(t) {
            self.disarm();
        }
        let s = &mut self.sensors[role];
        let dt = t - s.rotation_seen;
        if dt <= 0. {
            return Ok(());
        }
        if dt < self.cfg.stale {
            let a = gravity(s.q, self.cfg.y_up);
            let b = gravity(q, self.cfg.y_up);
            s.angular = a
                .iter()
                .zip(b)
                .map(|(v, w)| v * w)
                .sum::<f64>()
                .clamp(-1., 1.)
                .acos()
                / dt;
        } else {
            s.angular = 0.;
            s.ready = true;
        }
        s.q = q;
        s.seen = t;
        s.rotation_seen = t;
        if role == 2 {
            self.chest_turn.orientation(q, t, self.cfg.y_up);
            if self.cfg.cadence && !self.cfg.chest_steps {
                self.foot_intent.body_orientation(q, t, self.cfg.y_up);
                self.resolve_steps(t);
            }
            return Ok(());
        }
        let angular = s.angular;
        if self.cfg.cadence {
            self.foot_intent.orientation(role, q, angular, t, self.cfg.y_up);
            self.feed_cadence(role, t);
        }
        let s = &mut self.sensors[role];
        if s.angular < self.cfg.release {
            s.ready = true;
        }
        if s.ready && s.angular >= self.cfg.threshold && t - self.accel_seen[role] >= self.cfg.stale
        {
            s.ready = false;
            self.pulse(role, t);
        }
        Ok(())
    }
    fn pulse(&mut self, role: usize, t: f64) {
        if t - self.last_pulse[role] < 0.18 {
            return;
        }
        self.last_pulse[role] = t;
        self.preview_steps[role] = self.preview_steps[role].saturating_add(1);
        if !self.calibrated || !self.armed || !self.fresh(t) {
            return;
        }
        if let Some(&(last, previous)) = self.steps.last() {
            let dt = t - last;
            if previous == role || dt < 0.18 {
                return;
            }
            if dt > self.cfg.stop_after {
                self.steps.clear();
            }
        }
        self.steps.push((t, role));
        if self.steps.len() > 8 {
            self.steps.remove(0);
        }
    }
    pub fn update_chest_diagnostic(&mut self, a:[f64;3], t:f64) -> Result<(), &'static str> {
        if !t.is_finite() || !a.iter().all(|v|v.is_finite() && v.abs()<=200.) {self.diagnostic_motion.reset();self.diagnostic=Default::default();return Err("invalid chest acceleration");}
        let baseline=if self.cfg.accel_has_gravity {*self.diagnostic_baseline.get_or_insert(a)}else{[0.;3]};
        self.diagnostic_motion.update_foot(0,std::array::from_fn(|i|a[i]-baseline[i]),t)
    }
    pub fn update_acceleration(
        &mut self,
        role: usize,
        a: [f64; 3],
        t: f64,
    ) -> Result<(), &'static str> {
        let role = if role == 2 && self.cfg.chest_steps {0} else if role < 2 && !self.cfg.chest_steps {role} else {return Err("acceleration source does not match mode");};
        if !t.is_finite() || !a.iter().all(|v| v.is_finite() && v.abs() <= 200.) {
            self.disarm();
            return Err("invalid acceleration");
        }
        if t <= self.accel_seen[role] {
            return Ok(());
        }
        self.accel_seen[role] = t;
        let baseline = if self.cfg.accel_has_gravity {
            *self.accel_baseline[role].get_or_insert(a)
        } else {
            [0.; 3]
        };
        self.accel_mode[role] = if self.cfg.accel_has_gravity {
            "gravity-baseline"
        } else {
            "linear"
        };
        let dynamic = a
            .iter()
            .zip(baseline)
            .map(|(x, b)| (x - b) * (x - b))
            .sum::<f64>()
            .sqrt();
        self.accel_dynamic[role] = dynamic;
        if self.cfg.cadence && !self.cfg.chest_steps {
            self.foot_intent.acceleration(role, dynamic, t);
            self.feed_cadence(role, t);
        }
        self.motion
            .update_foot(role, std::array::from_fn(|i| a[i] - baseline[i]), t)?;
        if dynamic <= 0.45 {
            self.accel_ready[role] = true;
        }
        if !self.cfg.chest_steps && dynamic >= 1.2 && self.accel_ready[role] {
            self.accel_ready[role] = false;
            self.pulse(role, t);
        }
        Ok(())
    }
    pub fn tick(&mut self, t: f64) {
        self.diagnostic=self.diagnostic_motion.tick(t);
        if !t.is_finite() || t < self.last_tick {
            self.disarm();
            return;
        }
        let dt = (t - self.last_tick).clamp(0., 0.1);
        self.last_tick = t;
        if !self.fresh(t) {
            self.disarm();
            let chest=&self.sensors[2];
            self.heading = if self.calibrated && t>=chest.seen && t-chest.seen<self.cfg.stale {
                crate::mounting::heading(chest.q,self.origin,if self.cfg.y_up {[0.,1.,0.]} else {[0.,0.,1.]})
                    .map(|h|h*self.cfg.sign).unwrap_or(self.heading)
            } else {0.};
            return;
        }
        let active = self.armed && self.calibrated;
        let mut target = 0.;
        if active && self.steps.len() >= 2 && t - self.steps.last().unwrap().0 < self.cfg.stop_after
        {
            let interval =
                (self.steps.last().unwrap().0 - self.steps[0].0) / (self.steps.len() - 1) as f64;
            target = ((1. / interval.max(0.16) - 0.8) / 3.2).clamp(0.15, 0.85);
        }
        if self.cfg.cadence {
            let blocked = self.resolve_steps(t);
            self.preview = self.motion.tick(t);
            self.cadence_state = self.cadence.tick(t);
            if self.cadence_state.speed == 0. && self.cadence_state.target == 0. {
                self.foot_intent.reset_curve();
                self.curve_scale = 1.;
            } else {
                let scale = self.foot_intent.curve_scale(t, &self.cfg.foot_motion);
                self.curve_scale += (scale - self.curve_scale) * (1. - (-dt / 0.12).exp());
            }
            self.cadence_state.speed *= self.curve_scale;
            self.preview.speed = self.cadence_state.speed;
            self.preview.raw_speed = self.cadence_state.target;
            self.preview.status = if blocked { "turning: translation paused" }
                else if self.cadence_state.rate > 0. { "walking" } else { "waiting for steps" };
            self.speed = if active { self.cadence_state.speed } else { 0. };
        } else if self.cfg.continuous {
            if self.armed && !self.input_ready(t) {
                self.disarm();
            }
            self.preview = self.motion.tick(t);
            let turn = self.chest_turn_factor(t);
            if turn < 1. {
                self.preview.speed *= turn;
                self.preview.sprint &= turn > 0.;
                if self.preview.status == "ready" { self.preview.status = "turning: steps eased"; }
            }
            self.speed = if self.armed && self.calibrated {
                self.preview.speed
            } else {
                0.
            };
        } else {
            self.preview = self.motion.tick(t);
            self.speed += (target - self.speed) * (1. - (-dt / 0.1).exp());
            if target == 0. {
                self.speed = 0.;
            }
        }
        self.heading = crate::mounting::heading(self.sensors[2].q,self.origin,
            if self.cfg.y_up {[0.,1.,0.]} else {[0.,0.,1.]})
            .unwrap_or(self.heading / self.cfg.sign) * self.cfg.sign;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    /// Three-sensor step rhythm: feet tilt forward 0.5 rad per step, chest yaw is `yaw`.
    fn rhythm_frame(e: &mut Engine, t: f64, rate: f64, yaw: f64, feet_yaw: f64) {
        let period = 2. / rate;
        for side in 0..2 {
            let phase = (t - side as f64 * period / 2.).rem_euclid(period);
            let tilt = if rate > 0. && phase < 0.3 { 0.5 * (phase / 0.3 * std::f64::consts::PI).sin() } else { 0. };
            // Tilt about X, then yaw about Z (z-up world).
            let (c, s, cy, sy) = ((tilt / 2.).cos(), (tilt / 2.).sin(), (feet_yaw / 2.).cos(), (feet_yaw / 2.).sin());
            e.update(side, mul([cy, 0., 0., sy], [c, s, 0., 0.]), t).unwrap();
            e.update_acceleration(side, [if rate > 0. && (0.27..0.31).contains(&phase) { 3. } else { 0. }, 0., 0.], t).unwrap();
        }
        e.update(2, [(yaw / 2.).cos(), 0., 0., (yaw / 2.).sin()], t).unwrap();
        e.tick(t);
    }
    fn rhythm_engine() -> Engine {
        let mut cfg = Detection::default();
        cfg.cadence = true;
        let mut e = Engine::new(cfg);
        rhythm_frame(&mut e, 0., 0., 0., 0.);
        e.calibrate(0.).unwrap();
        rhythm_frame(&mut e, 0.01, 0., 0., 0.);
        e.arm(0.01).unwrap();
        e
    }
    #[test]
    fn step_rhythm_walks_with_feet_and_turns_with_chest() {
        let mut e = rhythm_engine();
        let mut t = 0.02;
        let mut speeds = vec![];
        while t < 5. {
            rhythm_frame(&mut e, t, 1.8, 0.6, 0.);
            if t > 2. { speeds.push(e.speed); }
            t += 0.01;
        }
        let mean = speeds.iter().sum::<f64>() / speeds.len() as f64;
        assert!(mean > 0.3 && speeds.iter().all(|s| *s > 0.2), "walking speed {mean:.3}");
        assert!((e.heading.abs() - 0.6).abs() < 0.05, "heading {:.3} did not follow chest", e.heading);
    }
    #[test]
    fn step_rhythm_ignores_turning_on_the_spot_and_leaning() {
        let mut e = rhythm_engine();
        let mut t = 0.02;
        while t < 6. {
            // Feet and chest rotate a full turn without stepping.
            let yaw = (t / 6.) * std::f64::consts::TAU;
            rhythm_frame(&mut e, t, 0., yaw, yaw);
            assert_eq!(e.speed, 0., "moved while turning at {t:.2}");
            t += 0.01;
        }
    }
    #[test]
    fn step_rhythm_stops_on_stale_feet() {
        let mut e = rhythm_engine();
        let mut t = 0.02;
        while t < 3. { rhythm_frame(&mut e, t, 2.2, 0., 0.); t += 0.01; }
        assert!(e.speed > 0.3);
        e.update(2, IDENTITY, 3.3).unwrap();
        e.tick(3.3);
        assert!(!e.armed && e.speed == 0., "stale feet did not stop movement");
    }
    fn continuous_frame(e: &mut Engine, t: f64, a: [f64; 2]) {
        for i in 0..3 {
            e.update(i, IDENTITY, t).unwrap();
        }
        for i in 0..2 {
            e.update_acceleration(i, [a[i], 0., 0.], t).unwrap();
        }
        e.tick(t);
    }
    #[test]
    fn single_chest_sensor_steps_stop_while_turning_on_the_spot_and_return_after() {
        let mut cfg = Detection::default();
        cfg.continuous = true;
        cfg.chest_steps = true;
        cfg.require_steps = false;
        let mut e = Engine::new(cfg);
        // The chest bounces the same whether the wearer walks or turns.
        let mut frame = |e: &mut Engine, t: f64, yaw_degrees: f64| {
            let h = yaw_degrees.to_radians() / 2.;
            e.update(2, [h.cos(), 0., 0., h.sin()], t).unwrap();
            e.update_acceleration(2, [8. + 3. * (t * 12.).sin(), 0., 0.], t).unwrap();
            e.tick(t);
        };
        frame(&mut e, 0., 0.);
        e.calibrate(0.).unwrap();
        frame(&mut e, 0.01, 0.);
        e.arm(0.01).unwrap();
        // Walking on the spot with stride sway.
        for n in 2..400 { let t = n as f64 * 0.01; frame(&mut e, t, 4. * (t * 7.).sin()); }
        assert!(e.speed > 0.2 && e.chest_turn_factor(3.99) == 1., "walking speed {}", e.speed);
        // Turning on the spot at 100 degrees a second.
        for n in 400..700 {
            let t = n as f64 * 0.01;
            frame(&mut e, t, (t - 4.) * 100.);
            if t > 4.4 { assert_eq!(e.speed, 0., "turning moved at {t}: {}", e.chest_turn_snapshot(t)); }
        }
        assert_eq!(e.preview.status, "turning: steps eased");
        assert!(e.armed && e.calibrated, "turning must not disarm");
        // Facing the new way, walking again.
        for n in 700..1000 { let t = n as f64 * 0.01; frame(&mut e, t, 300. + 4. * (t * 7.).sin()); }
        assert!(e.speed > 0.2, "walking did not return: {}", e.speed);
        // With foot sensors the chest alone never scales speed.
        let mut feet = Engine::new(Detection::default());
        feet.update(2, IDENTITY, 0.).unwrap();
        feet.update(2, [0.5f64.cos(), 0., 0., 0.5f64.sin()], 0.1).unwrap();
        assert_eq!(feet.chest_turn_factor(0.1), 1.);
    }
    #[test]
    fn preview_does_not_move_and_arm_clears_inertia() {
        let mut cfg = Detection::default();
        cfg.continuous = true;
        cfg.motion.fast_stop = false;
        let mut e = Engine::new(cfg);
        continuous_frame(&mut e, 0., [0., 0.]);
        e.calibrate(0.).unwrap();
        for n in 1..150 {
            let t = n as f64 * 0.02;
            continuous_frame(&mut e, t, [8. + t.sin(), 9. + (t * 2.).sin()]);
        }
        assert!(
            e.preview.speed > 0.2,
            "preview {} status {}",
            e.preview.speed,
            e.preview.status
        );
        assert_eq!(e.speed, 0.);
        e.arm(2.98).unwrap();
        e.tick(2.98);
        assert_eq!(e.speed, 0., "Pre-arm history cannot enter output");
        continuous_frame(&mut e, 3., [0., 0.]);
        e.disarm();
        assert_eq!(e.preview.speed, 0.);
    }
    #[test]
    fn stale_acceleration_disarms_despite_fresh_poses() {
        let mut cfg = Detection::default();
        cfg.continuous = true;
        let mut e = Engine::new(cfg);
        continuous_frame(&mut e, 0., [0., 0.]);
        e.calibrate(0.).unwrap();
        continuous_frame(&mut e, 0.02, [0., 0.]);
        e.arm(0.02).unwrap();
        for n in 2..20 {
            let t = n as f64 * 0.02;
            for i in 0..3 {
                e.update(i, IDENTITY, t).unwrap();
            }
            e.update_acceleration(0, [10. + t, 0., 0.], t).unwrap();
            e.tick(t);
        }
        assert!(e.fresh(0.38));
        assert!(!e.armed && e.speed == 0.);
        continuous_frame(&mut e, 0.4, [12., 12.]);
        assert!(!e.armed && e.speed == 0.);
    }
    #[test]
    fn frozen_foot_disarms_even_while_other_foot_moves() {
        let mut cfg = Detection::default();
        cfg.continuous = true;
        let mut e = Engine::new(cfg);
        continuous_frame(&mut e, 0., [0., 0.]);
        e.calibrate(0.).unwrap();
        continuous_frame(&mut e, 0.02, [0., 0.]);
        e.arm(0.02).unwrap();
        for n in 2..100 {
            let t = n as f64 * 0.02;
            continuous_frame(&mut e, t, [10., 4. + (t * 20.).sin() * 3.]);
        }
        assert!(!e.armed && e.speed == 0.);
    }
    #[test]
    fn recalibration_reestablishes_gravity_baseline() {
        let mut cfg = Detection::default();
        cfg.continuous = true;
        cfg.accel_has_gravity = true;
        let mut e = Engine::new(cfg);
        continuous_frame(&mut e, 0., [9.81, 9.81]);
        e.calibrate(0.).unwrap();
        for i in 0..2 {
            e.update_acceleration(i, [0., 9.81, 0.], 0.02).unwrap();
        }
        assert_eq!(e.accel_dynamic, [0., 0.]);
        assert_eq!(e.preview.speed, 0.);
    }
    #[test]
    fn acceleration_steps_preview_and_stale_stop() {
        let mut e = prepared();
        for s in &mut e.sensors {
            s.seen = 1.1;
        }
        e.update_acceleration(0, [0., 0., 0.], 1.10).unwrap();
        e.update_acceleration(1, [0., 0., 0.], 1.10).unwrap();
        e.update_acceleration(0, [1.5, 0., 0.], 1.12).unwrap();
        for s in &mut e.sensors {
            s.seen = 1.3;
        }
        e.update_acceleration(1, [1.5, 0., 0.], 1.32).unwrap();
        e.tick(1.32);
        assert_eq!(e.preview_steps, [1, 1]);
        assert!(e.speed > 0.);
        e.tick(1.60);
        assert!(!e.armed && e.speed == 0.);
        e.update_acceleration(0, [0., 0., 0.], 1.7).unwrap();
        e.update_acceleration(0, [1.5, 0., 0.], 1.9).unwrap();
        assert_eq!(e.preview_steps[0], 2);
        assert_eq!(e.speed, 0.);
    }
    #[test]
    fn linear_mode_detects_large_first_sample() {
        let mut e = Engine::new(Detection::default());
        e.update_acceleration(0, [10., 0., 0.], 1.).unwrap();
        assert_eq!(e.preview_steps[0], 1);
        assert_eq!(e.accel_mode[0], "linear");
    }
    #[test]
    fn gravity_baseline_noise_and_duplicate_pulses() {
        let mut e = prepared();
        e.cfg.accel_has_gravity = true;
        for s in &mut e.sensors {
            s.seen = 1.1;
        }
        e.update_acceleration(0, [0., 0., 9.81], 1.1).unwrap();
        e.update_acceleration(0, [0.1, 0., 9.85], 1.12).unwrap();
        assert_eq!(e.preview_steps[0], 0);
        e.update_acceleration(0, [1.5, 0., 9.81], 1.14).unwrap();
        assert_eq!(e.preview_steps[0], 1);
        e.update(0, [(0.2f64).cos(), (0.2f64).sin(), 0., 0.], 1.15)
            .unwrap();
        assert_eq!(e.preview_steps[0], 1);
    }
    fn prepared() -> Engine {
        let mut e = Engine::new(Detection::default());
        for i in 0..3 {
            e.update(i, IDENTITY, 1.).unwrap();
        }
        e.calibrate(1.).unwrap();
        e.arm(1.).unwrap();
        e
    }
    fn step(e: &mut Engine, role: usize, t: f64) {
        for s in &mut e.sensors {
            s.seen = t - 0.03;
        }
        e.update(role, IDENTITY, t - 0.02).unwrap();
        e.update(role, [(0.08f64).cos(), (0.08f64).sin(), 0., 0.], t)
            .unwrap();
        for s in &mut e.sensors {
            s.seen = t;
        }
        e.tick(t);
    }
    #[test]
    fn alternating_and_stop() {
        let mut e = prepared();
        step(&mut e, 0, 1.1);
        step(&mut e, 0, 1.4);
        assert_eq!(e.speed, 0.);
        step(&mut e, 1, 1.6);
        assert!(e.speed > 0.);
        for s in &mut e.sensors {
            s.seen = 2.2;
        }
        e.tick(2.2);
        assert_eq!(e.speed, 0.);
    }
    #[test]
    fn reconnect_never_rearms() {
        let mut e = prepared();
        for i in 0..3 {
            e.update(i, IDENTITY, 2.).unwrap();
        }
        assert!(!e.armed);
        e.tick(2.);
        assert_eq!(e.speed, 0.);
    }
    #[test]
    fn yaw_turns_are_not_steps() {
        let mut e = prepared();
        for n in 1..30 {
            let a = n as f64 * 0.05;
            let t = 1. + n as f64 * 0.01;
            for i in 0..3 {
                e.update(i, [a.cos(), 0., 0., a.sin()], t).unwrap();
            }
            e.tick(t);
            assert_eq!(e.speed, 0.);
        }
    }
    #[test]
    fn invalid_and_sign_flip() {
        assert!(unit([f64::NAN, 0., 0., 0.]).is_err());
        let mut e = prepared();
        e.update(0, [-1., 0., 0., 0.], 1.05).unwrap();
        assert_eq!(e.sensors[0].angular, 0.);
    }
    #[test]
    fn stale_disarms_and_y_up_heading() {
        let mut e = prepared();
        e.tick(1.26);
        assert!(!e.armed);
        e.cfg.y_up = true;
        for i in 0..3 {
            e.update(i, IDENTITY, 2.).unwrap();
        }
        e.calibrate(2.).unwrap();
        let a = std::f64::consts::FRAC_PI_4;
        e.update(2, [a.cos(), 0., a.sin(), 0.], 2.1).unwrap();
        e.tick(2.1);
        assert!((e.heading - std::f64::consts::FRAC_PI_2).abs() < 1e-6);
    }

    #[test]
    fn stale_feet_stop_speed_but_keep_fresh_chest_heading() {
        let mut e=prepared();
        let angle=0.4_f64;
        e.update(2,[angle.cos(),0.,0.,angle.sin()],1.3).unwrap();
        e.tick(1.3);
        assert!(!e.armed);assert_eq!(e.speed,0.);
        assert!((e.heading-0.8).abs()<1e-6);
    }

    #[test]
    fn heading_tracks_full_turn_with_tilt_and_mounted_sensor() {
        use std::f64::consts::{PI, TAU};
        for y_up in [false, true] {
            let mut cfg = Detection::default();
            cfg.y_up = y_up;
            cfg.require_steps = false;
            let mut e = Engine::new(cfg);
            // Non-identity sensor mounting at the calibrated forward pose.
            let mount = mul([0.4f64.cos(), 0.4f64.sin(), 0., 0.],
                            [0.3f64.cos(), 0., 0.3f64.sin(), 0.]);
            e.update(2, mount, 1.).unwrap();
            e.calibrate(1.).unwrap();
            let tilt = [0.2f64.cos(), 0.2f64.sin(), 0., 0.];
            for n in 0..=72 {
                let yaw = TAU * n as f64 / 72.;
                let half = yaw / 2.;
                let turn = if y_up { [half.cos(), 0., half.sin(), 0.] }
                           else { [half.cos(), 0., 0., half.sin()] };
                let t = 1.01 + n as f64 * 0.01;
                e.update(2, mul(mul(turn, tilt), mount), t).unwrap();
                e.tick(t);
                let error = (e.heading - yaw + PI).rem_euclid(TAU) - PI;
                assert!(error.abs() < 1e-6, "y_up={y_up} yaw={yaw} heading={} error={error}", e.heading);
            }
        }
    }
}

#[cfg(test)] mod rotation_timing_regression {
 use super::*;
 #[test] fn held_liveness_timestamp_cannot_amplify_real_rotation() {
  let mut e=Engine::new(Detection::default());e.update(0,IDENTITY,1.).unwrap();
  e.sensors[0].seen=1.019;
  e.update(0,[(0.01_f64).cos(),0.01_f64.sin(),0.,0.],1.02).unwrap();
  assert!((e.sensors[0].angular-1.).abs()<0.001,"{}",e.sensors[0].angular);
 }
}

#[cfg(test)]
mod foot_intent_tests {
    use super::*;
    fn rotation(axis: [f64; 3], angle: f64) -> Q {
        let h = angle / 2.; [h.cos(), axis[0] * h.sin(), axis[1] * h.sin(), axis[2] * h.sin()]
    }
    fn frame(e: &mut Engine, t: f64, tilts: [f64; 2], yaw: f64,
             accelerations: [f64; 2], mount: Q, accel_first: bool, flip_sign: bool) {
        let up = if e.cfg.y_up { [0., 1., 0.] } else { [0., 0., 1.] };
        let turn = rotation(up, yaw);
        e.update(2, turn, t).unwrap();
        for side in 0..2 {
            let mut q = mul(mul(turn, rotation([1., 0., 0.], tilts[side])), mount);
            if flip_sign { q = q.map(|x| -x); }
            if accel_first { e.update_acceleration(side, [accelerations[side], 0., 0.], t).unwrap(); }
            e.update(side, q, t).unwrap();
            if !accel_first { e.update_acceleration(side, [accelerations[side], 0., 0.], t).unwrap(); }
        }
        e.tick(t);
    }
    fn engine(y_up: bool, mount: Q) -> Engine {
        let mut cfg = Detection::default(); cfg.cadence = true; cfg.y_up = y_up;
        let mut e = Engine::new(cfg);
        frame(&mut e, 0., [0.; 2], 0., [0.; 2], mount, false, false);
        e.calibrate(0.).unwrap();
        frame(&mut e, 0.01, [0.; 2], 0., [0.; 2], mount, false, false);
        e.arm(0.01).unwrap(); e
    }
    fn stride(e: &mut Engine, t: f64, rate: f64, degrees: f64, yaw: f64, mount: Q, first: bool) {
        let period = 2. / rate;
        let mut tilts = [0.; 2]; let mut accel = [0.; 2];
        for side in 0..2 {
            let phase = (t - side as f64 * period / 2.).rem_euclid(period);
            if phase < 0.3 { tilts[side] = degrees.to_radians() * (phase / 0.3 * std::f64::consts::PI).sin(); }
            accel[side] = if (0.12..0.20).contains(&phase) { if degrees < 3. { 1.3 } else { 3. } } else { 0. };
        }
        frame(e, t, tilts, yaw, accel, mount, first, (t * 100.).round() as usize % 2 == 0);
    }
    #[test]
    fn realistic_yaw_dominant_pivot_with_roll_and_acceleration_never_walks() {
        let mount = unit([0.6, 0.3, -0.4, 0.5]).unwrap();
        for y_up in [false, true] {
            let mut e = engine(y_up, mount);
            for n in 2..601 {
                let t = n as f64 * 0.01;
                let roll = (t * 20.).sin() * 4_f64.to_radians();
                frame(&mut e, t, [roll, -roll], t * 4., [4., 3.], mount, n % 2 == 0, n % 3 == 0);
                assert_eq!(e.speed, 0., "pivot moved at {t}: {}", e.foot_motion_snapshot(t));
            }
            assert_eq!(e.cadence_state.steps, [0, 0]);
        }
    }
    #[test]
    fn tiny_fast_jitter_with_acceleration_does_not_count() {
        let mut e = engine(false, IDENTITY);
        for n in 20..2000 {
            let t = n as f64 * 0.001;
            let tilt = 0.15_f64.to_radians() * (t * 1200.).sin();
            frame(&mut e, t, [tilt, -tilt], 0., [3.; 2], IDENTITY, false, n % 2 == 0);
            assert_eq!(e.speed, 0.);
        }
        assert_eq!(e.cadence_state.steps, [0, 0]);
    }
    #[test]
    fn gentle_slow_alternating_swings_with_small_impacts_keep_walking() {
        let mut e = engine(false, IDENTITY);
        for n in 2..1000 {
            let t = n as f64 * 0.01;
            stride(&mut e, t, 1., 2.5, 0., IDENTITY, false);
        }
        assert!(e.cadence_state.steps.iter().sum::<u64>() >= 8, "{:?}", e.cadence_state);
        assert!(e.speed > 0.1, "gentle steps stalled: {:?}", e.cadence_state);
    }
    #[test]
    fn acceleration_first_and_orientation_first_produce_same_rhythm() {
        let mut a = engine(false, IDENTITY); let mut b = engine(false, IDENTITY);
        for n in 2..600 {
            let t = n as f64 * 0.01;
            stride(&mut a, t, 2.2, 25., 0., IDENTITY, true);
            stride(&mut b, t, 2.2, 25., 0., IDENTITY, false);
            assert_eq!(a.cadence_state.steps, b.cadence_state.steps);
            assert!((a.speed - b.speed).abs() < 1e-10);
        }
        assert!(a.speed > 0.4);
    }
    #[test]
    fn deliberate_turn_pauses_even_running_but_straight_running_stays_fast() {
        let mut straight = engine(false, IDENTITY); let mut turning = engine(false, IDENTITY);
        let (mut a, mut b, mut count) = (0., 0., 0.);
        for n in 2..800 {
            let t = n as f64 * 0.01;
            stride(&mut straight, t, 3.2, 30., 0., IDENTITY, false);
            stride(&mut turning, t, 3.2, 30., t * 2., IDENTITY, false);
            if t > 3. {
                // Intentionally conservative: torso turning pauses translation,
                // even when a genuine stride is present at the same time.
                assert_eq!(turning.speed, 0., "turn escaped at {t}: {}", turning.foot_motion_snapshot(t));
                a += straight.speed; b += turning.speed; count += 1.;
            }
        }
        assert!(a / count > 0.85, "straight running speed {}", a / count);
        assert_eq!(b, 0.);
    }
    #[test]
    fn gentle_curved_walking_below_body_turn_threshold_keeps_walking() {
        let mut e = engine(false, IDENTITY);
        for n in 2..800 {
            let t = n as f64 * 0.01;
            stride(&mut e, t, 1., 2.5, t * 10_f64.to_radians(), IDENTITY, false);
            assert!(!e.pivot_suppressed, "gentle curve suppressed at {t}");
        }
        assert!(e.speed > 0.1);
        assert!(e.cadence_state.steps.iter().sum::<u64>() >= 6);
    }
    /// Chest and each foot with its own heading, as on a real body.
    fn body(e: &mut Engine, t: f64, chest_yaw: f64, feet_yaw: [f64; 2], tilts: [f64; 2], accelerations: [f64; 2]) {
        let up = if e.cfg.y_up { [0., 1., 0.] } else { [0., 0., 1.] };
        e.update(2, rotation(up, chest_yaw), t).unwrap();
        for side in 0..2 {
            e.update(side, mul(rotation(up, feet_yaw[side]), rotation([1., 0., 0.], tilts[side])), t).unwrap();
            e.update_acceleration(side, [accelerations[side], 0., 0.], t).unwrap();
        }
        e.tick(t);
    }
    /// Walking on the spot as recorded: each foot lifts about 25 degrees and wobbles
    /// a few degrees in heading, while the chest sways with the stride.
    fn sway_walk(e: &mut Engine, t: f64, rate: f64, heading: f64, sway: f64) {
        curve_walk(e, t, rate, heading, 0., sway);
    }
    /// `turn_rate` (radians per second) turns the body while marching. A planted
    /// foot keeps the heading it landed with and catches up during its next step.
    fn curve_walk(e: &mut Engine, t: f64, rate: f64, heading: f64, turn_rate: f64, sway: f64) {
        let period = 2. / rate;
        let heading = heading + turn_rate * t;
        let (mut yaw, mut tilts, mut accel) = ([heading; 2], [0.; 2], [0.05; 2]);
        for side in 0..2 {
            let phase = (t - side as f64 * period / 2.).rem_euclid(period);
            // Heading when this foot last lifted, then easing to the body's heading.
            let lifted = heading - turn_rate * (phase + period);
            let f = (phase / 0.3).min(1.);
            yaw[side] = lifted + (heading - lifted) * f * f * (3. - 2. * f) - turn_rate * (phase - 0.3).max(0.);
            if phase < 0.3 {
                let swing = (phase / 0.3 * std::f64::consts::PI).sin();
                tilts[side] = 25_f64.to_radians() * swing;
                yaw[side] += 5_f64.to_radians() * swing * if side == 0 { 1. } else { -1. };
            }
            if (0.12..0.20).contains(&phase) { accel[side] = 4.; }
        }
        let chest = heading + sway.to_radians() * (t * std::f64::consts::TAU / period).sin();
        body(e, t, chest, yaw, tilts, accel);
    }
    /// Turning on the spot as recorded: the chest rotates steadily and the feet
    /// follow in alternating steps, each lifting a little and twisting to catch up.
    fn stepping_turn(e: &mut Engine, t: f64, start: f64, heading: f64, degrees_per_second: f64) -> f64 {
        let rate = degrees_per_second.to_radians();
        let elapsed = t - start;
        let (mut yaw, mut tilts, mut accel) = ([heading; 2], [0.; 2], [0.05; 2]);
        for side in 0..2 {
            // Each foot steps once a second for 0.6 s, half a second apart.
            let local = elapsed - side as f64 * 0.5;
            let (count, phase) = ((local / 1.).floor().max(0.), local.rem_euclid(1.));
            let mut steps = if local < 0. { 0. } else { count };
            if local >= 0. && phase < 0.6 {
                let f = phase / 0.6;
                steps += f * f * (3. - 2. * f);
                tilts[side] = 15_f64.to_radians() * (f * std::f64::consts::PI).sin();
                accel[side] = 4.;
            } else if local >= 0. { steps += 1.; }
            yaw[side] += steps * rate;
        }
        let chest = heading + elapsed * rate;
        body(e, t, chest, yaw, tilts, accel);
        chest
    }
    #[test]
    fn chest_sway_and_foot_wobble_while_walking_never_pause_translation() {
        // 1.1.20 treated any 18 degree-per-second chest movement as a turn. Normal
        // stride sway is faster than that, so walking stalled on real trackers.
        for (rate, sway) in [(1.2, 4.), (2.2, 6.), (3.2, 8.)] {
            let (mut steady, mut swaying) = (engine(false, IDENTITY), engine(false, IDENTITY));
            let (mut a, mut b, mut count) = (0., 0., 0.);
            for n in 2..900 {
                let t = n as f64 * 0.01;
                sway_walk(&mut steady, t, rate, 0., 0.);
                sway_walk(&mut swaying, t, rate, 0., sway);
                assert!(!swaying.pivot_suppressed, "rate {rate}: sway paused walking at {t}: {}", swaying.foot_motion_snapshot(t));
                if t > 3. {
                    assert!(swaying.speed > 0.15, "rate {rate}: walking stalled at {t}");
                    a += steady.speed; b += swaying.speed; count += 1.;
                }
            }
            assert!((a - b).abs() / count < 0.05, "rate {rate}: sway changed speed {} -> {} steps {:?} {:?}", a / count, b / count, steady.cadence_state.steps, swaying.cadence_state.steps);
            let total: u64 = swaying.cadence_state.steps.iter().sum();
            assert!(total as f64 >= 8.9 * rate * 0.9, "rate {rate}: only {total} steps counted");
        }
    }
    #[test]
    fn stepping_turn_on_the_spot_never_translates() {
        for y_up in [false, true] { for direction in [-1., 1.] { for speed in [60., 100., 170.] {
            let mut e = engine(y_up, IDENTITY);
            let mut travelled = 0.;
            for n in 2..800 {
                let t = n as f64 * 0.01;
                stepping_turn(&mut e, t, 0.02, 0., direction * speed);
                travelled += e.speed * 0.01;
                assert!(e.speed < 0.1, "y_up={y_up} {speed} deg/s: turn moved at {t}: {}", e.foot_motion_snapshot(t));
            }
            assert!(travelled < 0.01, "y_up={y_up} direction={direction} {speed} deg/s travelled {travelled}");
            assert_eq!(e.cadence_state.steps, [0, 0], "turning steps were counted as walking");
        } } }
    }
    #[test]
    fn walking_stops_for_a_stepping_turn_and_resumes_after_it() {
        let mut e = engine(false, IDENTITY);
        let mut t = 0.02;
        while t < 4. { sway_walk(&mut e, t, 2.2, 0., 5.); t += 0.01; }
        assert!(e.speed > 0.3, "walk before turn {}", e.speed);
        let mut heading = 0.;
        let (mut travelled, mut stopped_at) = (0., None);
        while t < 6. {
            heading = stepping_turn(&mut e, t, 4., 0., 100.);
            if e.speed == 0. { stopped_at.get_or_insert(t); }
            if t > 4.6 { travelled += e.speed * 0.01; }
            t += 0.01;
        }
        assert!(stopped_at.is_some_and(|at| at < 4.6), "turn did not stop walking promptly: {stopped_at:?}");
        assert!(travelled < 0.01, "moved {travelled} while turning");
        // Feet finish level with the chest, then walking starts again.
        let resume = t;
        let mut moving_at = None;
        while t < resume + 4. {
            sway_walk(&mut e, t, 2.2, heading, 5.);
            if e.speed > 0.3 { moving_at.get_or_insert(t); }
            t += 0.01;
        }
        assert!(moving_at.is_some_and(|at| at - resume < 1.6), "walking resumed late: {moving_at:?} after {resume}");
        assert!(e.armed && e.calibrated);
    }
    #[test]
    fn walking_round_a_curve_keeps_moving() {
        // The body turns 25 degrees a second while marching; each foot lands a
        // little further round, but lifts far more than it twists.
        let mut e = engine(false, IDENTITY);
        let mut slow = 0;
        for n in 2..1000 {
            let t = n as f64 * 0.01;
            curve_walk(&mut e, t, 2.2, 0., 25_f64.to_radians(), 4.);
            if t > 3. && e.speed < 0.15 { slow += 1; }
        }
        assert!(slow < 30, "curve walking stalled for {slow} frames");
        assert!((e.heading.to_degrees().rem_euclid(360.) - (9.99_f64 * 25.).rem_euclid(360.)).abs() < 8., "heading {}", e.heading.to_degrees());
    }
    #[test]
    fn tighter_curves_walk_slower_until_the_turn_takes_over() {
        // Straight, gentle, moderate, tight, and effectively turning on the spot.
        let mut previous = f64::INFINITY;
        let mut speeds = vec![];
        for rate in [0., 12., 25., 40., 70.] {
            let mut e = engine(false, IDENTITY);
            let (mut sum, mut count) = (0., 0.);
            for n in 2..1200 {
                let t = n as f64 * 0.01;
                curve_walk(&mut e, t, 2.2, 0., f64::to_radians(rate), 4.);
                if t > 4. { sum += e.speed; count += 1.; }
            }
            let mean = sum / count;
            assert!(mean <= previous + 0.02, "{rate} deg/s was faster ({mean:.2}) than a gentler curve ({previous:.2})");
            previous = mean;
            speeds.push(mean);
        }
        assert!(speeds[0] > 0.5 && speeds[1] > 0.9 * speeds[0], "gentle curve lost speed: {speeds:?}");
        assert!(speeds[2] > 0.3, "moderate curve stalled: {speeds:?}");
        assert!(speeds[4] < 0.05, "spinning still walked: {speeds:?}");
    }
    #[test]
    fn chest_twist_without_foot_twist_keeps_walking() {
        // Looking over a shoulder turns the chest; the feet keep marching straight.
        let mut e = engine(false, IDENTITY);
        for n in 2..400 { sway_walk(&mut e, n as f64 * 0.01, 2.2, 0., 4.); }
        assert!(e.speed > 0.3);
        for n in 400..900 {
            let t = n as f64 * 0.01;
            let twist = 40_f64.to_radians() * ((t - 4.) * 2.).min(1.);
            let period = 2. / 2.2;
            let (mut tilts, mut accel) = ([0.; 2], [0.05; 2]);
            for side in 0..2 {
                let phase = (t - side as f64 * period / 2.).rem_euclid(period);
                if phase < 0.3 { tilts[side] = 25_f64.to_radians() * (phase / 0.3 * std::f64::consts::PI).sin(); }
                if (0.12..0.20).contains(&phase) { accel[side] = 4.; }
            }
            body(&mut e, t, twist, [0.; 2], tilts, accel);
            assert!(e.speed > 0.15 && !e.pivot_suppressed, "chest twist stalled walking at {t}: {}", e.foot_motion_snapshot(t));
        }
    }
    #[test]
    fn full_turn_both_directions_arbitrary_chest_mount_and_preview_never_translate() {
        let mount = unit([0.6, 0.3, -0.4, 0.5]).unwrap();
        for y_up in [false, true] { for direction in [-1., 1.] { for armed in [false, true] {
            let mut e = engine(y_up, mount);
            if !armed { e.disarm(); }
            let up = if y_up { [0., 1., 0.] } else { [0., 0., 1.] };
            for n in 2..702 {
                let t = n as f64 * 0.01;
                let yaw = direction * t * std::f64::consts::TAU / 6.;
                let mut chest = mul(rotation(up, yaw), mount);
                if n % 2 == 0 { chest = chest.map(|v| -v); }
                e.update(2, chest, t).unwrap();
                // Reproduce roll + impacts during the turn, with a qualifying opposite foot.
                for side in 0..2 {
                    let roll = ((t * 18.) + side as f64).sin() * 8_f64.to_radians();
                    e.update(side, mul(mul(rotation(up, yaw), rotation([1., 0., 0.], roll)), mount), t).unwrap();
                    e.update_acceleration(side, [3., 0., 0.], t).unwrap();
                }
                e.tick(t);
                assert_eq!((e.speed, e.preview.speed), (0., 0.), "direction={direction} y_up={y_up} armed={armed} t={t}: {}", e.foot_motion_snapshot(t));
            }
            assert_eq!(e.armed, armed);
        } } }
    }
    #[test]
    fn either_foot_pivot_holds_across_opposite_qualified_support() {
        let mut e = engine(false, IDENTITY);
        for n in 2..400 {
            let t = n as f64 * 0.01;
            e.update(2, IDENTITY, t).unwrap();
            let pivot_side = (n / 20) % 2;
            for side in 0..2 {
                let q = if side == pivot_side { rotation([0., 0., 1.], t * 4.) }
                    else { rotation([1., 0., 0.], (t * 30.).sin() * 0.3) };
                e.update(side, q, t).unwrap();
                e.update_acceleration(side, [4., 0., 0.], t).unwrap();
            }
            e.tick(t);
            if t > 0.25 {
                assert!(e.pivot_suppressed, "alternating support escaped at {t}: {}", e.foot_motion_snapshot(t));
                assert_eq!((e.speed, e.preview.speed), (0., 0.));
            }
        }
    }
    #[test]
    fn walking_to_pivot_stops_without_waiting_for_cadence_timeout() {
        let mut e = engine(false, IDENTITY);
        for n in 2..400 { let t = n as f64 * 0.01; stride(&mut e, t, 2.2, 25., 0., IDENTITY, false); }
        assert!(e.speed > 0.3);
        for n in 400..440 {
            let t = n as f64 * 0.01;
            frame(&mut e, t, [0.; 2], (t - 4.) * 4., [4.; 2], IDENTITY, false, false);
        }
        assert!(e.pivot_suppressed);
        assert_eq!(e.speed, 0.);
        assert!(e.armed && e.calibrated);
    }
    #[test]
    fn settings_update_clears_activity_preserves_calibration_and_rejects_invalid_values() {
        let mut e = engine(false, IDENTITY);
        for n in 2..300 { let t = n as f64 * 0.01; stride(&mut e, t, 2.2, 25., 0.4, IDENTITY, false); }
        let before = (e.heading, e.cadence_state.steps, e.steps.clone());
        let mut settings = e.cfg.foot_motion.clone(); settings.min_swing_degrees = 3.;
        e.update_foot_motion_settings(settings).unwrap();
        assert!(e.armed && e.calibrated);
        assert_eq!((e.heading, e.cadence_state.steps, e.steps.clone()), before);
        assert_eq!((e.speed, e.preview.speed, e.cadence_state.speed), (0., 0., 0.));
        assert!(!e.pivot_suppressed);
        let mut invalid = e.cfg.foot_motion.clone(); invalid.pivot_ratio = f64::NAN;
        assert!(e.update_foot_motion_settings(invalid).is_err());
    }
}
