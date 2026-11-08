//! Three-IMU locomotion with continuous acceleration and a legacy step mode.
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
fn gravity(q: Q, y_up: bool) -> [f64; 3] {
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
    ready: bool,
}
impl Default for Sensor {
    fn default() -> Self {
        Self {
            q: IDENTITY,
            seen: -1e9,
            angular: 0.,
            ready: true,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Detection {
    pub require_steps: bool,
    pub chest_steps: bool,
    pub continuous: bool,
    pub motion: Settings,
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
            motion: Settings::default(),
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
        {
            Err("invalid detection settings")
        } else {
            Ok(())
        }
    }
}
pub struct Engine {
    pub motion: MotionFilter,
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
            motion, diagnostic_motion, diagnostic:Default::default(), diagnostic_baseline:None,
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
        let dt = t - s.seen;
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
        if role == 2 {
            return Ok(());
        }
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
            self.heading = 0.;
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
        if self.cfg.continuous {
            if self.armed && !self.input_ready(t) {
                self.disarm();
            }
            self.preview = self.motion.tick(t);
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
