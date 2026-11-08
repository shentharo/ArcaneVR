//! Independent continuous acceleration preview. This is not a reconstruction of
//! proprietary code. The application must gate its output on explicit arming.
use serde_json::{json, Value};
use std::collections::VecDeque;

const STALE: f64 = 0.25;
const FROZEN: f64 = 0.5;
const CAPACITY: usize = 4096;
const MAX_WINDOW: f64 = 10.0;

#[derive(Clone, Debug)]
pub struct Settings {
    pub intensity: f64,
    pub axes: [[bool; 3]; 2],
    pub threshold: f64,
    pub fast_window: f64,
    pub slow_window: f64,
    pub adaptive_per_unit: f64,
    pub inertia_window: f64,
    pub max_signal: f64,
    pub exponent: f64,
    pub curve_mode: String,
    pub curve_points: Vec<[f64; 2]>,
    pub sprint_enabled: bool,
    pub sprint_threshold: f64,
    pub fast_stop: bool,
    pub quiet_percent: f64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            intensity: 1.36,
            axes: [[true; 3]; 2],
            threshold: 4.206,
            fast_window: 0.060,
            slow_window: 2.010,
            adaptive_per_unit: 0.0382,
            inertia_window: 0.535,
            max_signal: 20.85,
            exponent: 1.0,
            curve_mode: "exponent".into(),
            curve_points: vec![[0., 0.], [0.25, 0.25], [0.5, 0.5], [0.75, 0.75], [1., 1.]],
            sprint_enabled: false,
            sprint_threshold: 38.4,
            fast_stop: true,
            quiet_percent: 0.0,
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.intensity.is_finite()
            || !(0.01..=20.).contains(&self.intensity)
            || !self.threshold.is_finite()
            || !(0.0..=100.).contains(&self.threshold)
            || !self.fast_window.is_finite()
            || !(0.01..=MAX_WINDOW).contains(&self.fast_window)
            || !self.slow_window.is_finite()
            || !(self.fast_window..=MAX_WINDOW).contains(&self.slow_window)
            || !self.adaptive_per_unit.is_finite()
            || !(0.0..=2.).contains(&self.adaptive_per_unit)
            || !self.inertia_window.is_finite()
            || !(0.01..=MAX_WINDOW).contains(&self.inertia_window)
            || !self.max_signal.is_finite()
            || !(0.1..=500.).contains(&self.max_signal)
            || !self.exponent.is_finite()
            || !(0.1..=5.).contains(&self.exponent)
            || !["exponent", "custom"].contains(&self.curve_mode.as_str())
            || !(2..=32).contains(&self.curve_points.len())
            || self.curve_points.first() != Some(&[0., 0.])
            || self.curve_points.last() != Some(&[1., 1.])
            || self
                .curve_points
                .iter()
                .any(|p| p.iter().any(|x| !x.is_finite() || !(0.0..=1.).contains(x)))
            || self
                .curve_points
                .windows(2)
                .any(|p| p[0][0] >= p[1][0] || p[0][1] > p[1][1])
            || !self.sprint_threshold.is_finite()
            || !(0.01..=1000.).contains(&self.sprint_threshold)
            || (self.sprint_enabled
                && (self.sprint_threshold <= self.threshold
                    || self.sprint_threshold < self.max_signal))
            || !self.quiet_percent.is_finite()
            || !(0.0..=100.).contains(&self.quiet_percent)
            || self.axes.iter().any(|a| !a.iter().any(|x| *x))
        {
            return Err("invalid continuous settings");
        }
        Ok(())
    }
    /// Missing fields retain defaults. Unknown names are rejected to catch typos.
    pub fn from_json(v: &Value) -> Result<Self, &'static str> {
        let obj = v.as_object().ok_or("settings must be an object")?;
        let mut s = Self::default();
        for (k, v) in obj {
            match k.as_str() {
                "intensity" => s.intensity = v.as_f64().ok_or("invalid intensity")?,
                "threshold" => s.threshold = v.as_f64().ok_or("invalid threshold")?,
                "fast_window" => s.fast_window = v.as_f64().ok_or("invalid fast window")?,
                "slow_window" => s.slow_window = v.as_f64().ok_or("invalid slow window")?,
                "adaptive_per_unit" => {
                    s.adaptive_per_unit = v.as_f64().ok_or("invalid adaptive window")?
                }
                "inertia_window" => {
                    s.inertia_window = v.as_f64().ok_or("invalid inertia window")?
                }
                "max_signal" => s.max_signal = v.as_f64().ok_or("invalid max signal")?,
                "exponent" => s.exponent = v.as_f64().ok_or("invalid exponent")?,
                "curve_mode" => s.curve_mode = v.as_str().ok_or("invalid curve mode")?.to_owned(),
                "curve_points" => {
                    let points = v.as_array().ok_or("invalid curve points")?;
                    if points.len() > 32 {
                        return Err("too many curve points");
                    }
                    let mut parsed = Vec::with_capacity(points.len());
                    for point in points {
                        let pair = point
                            .as_array()
                            .filter(|p| p.len() == 2)
                            .ok_or("invalid curve point")?;
                        parsed.push([
                            pair[0].as_f64().ok_or("invalid curve x")?,
                            pair[1].as_f64().ok_or("invalid curve y")?,
                        ]);
                    }
                    s.curve_points = parsed;
                }
                "sprint_enabled" => {
                    s.sprint_enabled = v.as_bool().ok_or("invalid sprint enabled")?
                }
                "sprint_threshold" => {
                    s.sprint_threshold = v.as_f64().ok_or("invalid sprint threshold")?
                }
                "fast_stop" => s.fast_stop = v.as_bool().ok_or("invalid fast stop")?,
                "quiet_percent" => s.quiet_percent = v.as_f64().ok_or("invalid quiet percent")?,
                "axes" => {
                    let a = v
                        .as_array()
                        .filter(|a| a.len() == 2)
                        .ok_or("invalid axes")?;
                    for i in 0..2 {
                        let b = a[i]
                            .as_array()
                            .filter(|a| a.len() == 3)
                            .ok_or("invalid axes")?;
                        for j in 0..3 {
                            s.axes[i][j] = b[j].as_bool().ok_or("invalid axes")?;
                        }
                    }
                }
                _ => return Err("unknown continuous setting"),
            }
        }
        s.validate()?;
        Ok(s)
    }
    pub fn to_json(&self) -> Value {
        json!({"intensity":self.intensity,"axes":self.axes,
        "threshold":self.threshold,"fast_window":self.fast_window,"slow_window":self.slow_window,
        "adaptive_per_unit":self.adaptive_per_unit,"inertia_window":self.inertia_window,
        "max_signal":self.max_signal,"exponent":self.exponent,"curve_mode":self.curve_mode,"curve_points":self.curve_points,"sprint_enabled":self.sprint_enabled,"sprint_threshold":self.sprint_threshold,"fast_stop":self.fast_stop,
        "quiet_percent":self.quiet_percent})
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Telemetry {
    pub speed: f64,
    pub raw_speed: f64,
    pub sprint: bool,
    pub signal: f64,
    pub fast: f64,
    pub slow: f64,
    pub adaptive: f64,
    pub inertia: f64,
    pub status: &'static str,
}
impl Default for Telemetry {
    fn default() -> Self {
        Self::stopped("waiting for both feet")
    }
}
impl Telemetry {
    fn stopped(status: &'static str) -> Self {
        Self {
            speed: 0.,
            raw_speed: 0.,
            sprint: false,
            signal: 0.,
            fast: 0.,
            slow: 0.,
            adaptive: 0.,
            inertia: 0.,
            status,
        }
    }
}
#[derive(Clone, Copy)]
struct Foot {
    v: [f64; 3],
    seen: f64,
    changed: f64,
    changed_reference: [f64; 3],
}
#[derive(Default)]
struct History {
    samples: VecDeque<(f64, f64)>,
}
impl History {
    fn push(&mut self, t: f64, v: f64) {
        if self.samples.back().is_some_and(|x| x.0 == t) {
            self.samples.pop_back();
        }
        self.samples.push_back((t, v));
        while self.samples.len() > CAPACITY {
            self.samples.pop_front();
        }
        while self.samples.len() > 1 && self.samples.get(1).is_some_and(|x| x.0 < t - MAX_WINDOW) {
            self.samples.pop_front();
        }
    }
    // Piecewise constant, time-weighted mean; time before the first sample is zero.
    fn average(&self, t: f64, window: f64) -> f64 {
        let start = t - window;
        let mut previous = None;
        let mut sum = 0.;
        for &(sample_t, value) in &self.samples {
            if sample_t > t {
                break;
            }
            if let Some((old_t, old_v)) = previous {
                let a = f64::max(old_t, start);
                let b = f64::min(sample_t, t);
                if b > a {
                    sum += (b - a) * old_v;
                }
            }
            previous = Some((sample_t, value));
        }
        if let Some((old_t, old_v)) = previous {
            let a = f64::max(old_t, start);
            if t > a {
                sum += (t - a) * old_v;
            }
        }
        sum / window
    }
    fn range_mean(&self, t: f64, window: f64) -> (f64, f64) {
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for &(ts, v) in &self.samples {
            if ts >= t - window && ts <= t {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        if !lo.is_finite() {
            return (0., 0.);
        }
        (hi - lo, self.average(t, window))
    }
    fn clear(&mut self) {
        self.samples.clear();
    }
}
/// Interpolate a validated monotonic curve over [0,1].
fn curve(points: &[[f64; 2]], x: f64) -> f64 {
    let x = x.clamp(0., 1.);
    for pair in points.windows(2) {
        if x <= pair[1][0] {
            let t = (x - pair[0][0]) / (pair[1][0] - pair[0][0]);
            return pair[0][1] + t * (pair[1][1] - pair[0][1]);
        }
    }
    1.
}
pub struct MotionFilter {
    settings: Settings,
    feet: [Option<Foot>; 2],
    single_input: bool,
    samples: History,
    inertia: History,
    quiet_observations: History,
    last_time: Option<f64>,
    quiet_since: Option<f64>,
}
impl MotionFilter {
    pub fn new(settings: Settings) -> Result<Self, &'static str> {
        settings.validate()?;
        Ok(Self {
            settings,
            feet: [None, None],
            single_input: false,
            samples: History::default(),
            inertia: History::default(),
            quiet_observations: History::default(),
            last_time: None,
            quiet_since: None,
        })
    }
    pub fn set_single_input(&mut self, single: bool) { self.reset(); self.single_input=single; }
    /// Reports whether both real samples meet liveness and frozen-signal gates.
    pub fn inputs_fresh(&self, t: f64) -> bool {
        if !t.is_finite() || self.last_time.is_some_and(|last| t < last) {
            return false;
        }
        self.feet.iter().take(if self.single_input {1} else {2}).enumerate().all(|(i, item)| {
            let Some(foot) = item else {
                return false;
            };
            if t < foot.seen || t - foot.seen >= STALE {
                return false;
            }
            let magnitude: f64 = foot
                .v
                .iter()
                .zip(self.settings.axes[i])
                .filter_map(|(x, on)| on.then_some(x.abs()))
                .sum();
            magnitude <= 0.45 || t - foot.changed < FROZEN
        })
    }
    pub fn settings(&self) -> &Settings {
        &self.settings
    }
    pub fn set_settings(&mut self, settings: Settings) -> Result<(), &'static str> {
        settings.validate()?;
        self.settings = settings;
        self.reset();
        Ok(())
    }
    /// Call for actual, newly received samples only. `t` is monotonic seconds.
    pub fn update_foot(&mut self, foot: usize, v: [f64; 3], t: f64) -> Result<(), &'static str> {
        if foot >= 2
            || !t.is_finite()
            || v.iter().any(|x| !x.is_finite() || x.abs() > 200.)
            || self.last_time.is_some_and(|last| t < last)
        {
            self.reset();
            return Err("invalid sample or time");
        }
        let (changed, changed_reference) = match self.feet[foot] {
            None => (t, v),
            Some(old)
                if v.iter()
                    .zip(old.changed_reference)
                    .any(|(x, y)| (x - y).abs() > 0.05) =>
            {
                (t, v)
            }
            Some(old) => (old.changed, old.changed_reference),
        };
        self.feet[foot] = Some(Foot {
            v,
            seen: t,
            changed,
            changed_reference,
        });
        self.last_time = Some(t);
        Ok(())
    }
    /// Clears preview smoothing and quiet-gate state while keeping the last real feet.
    pub fn clear_history(&mut self) {
        self.samples.clear();
        self.inertia.clear();
        self.quiet_observations.clear();
        self.quiet_since = None;
    }
    /// Clears all histories and requires both feet to provide new measurements.
    pub fn reset(&mut self) {
        self.feet = [None, None];
        self.clear_history();
        self.last_time = None;
    }
    pub fn tick(&mut self, t: f64) -> Telemetry {
        if !t.is_finite() || self.last_time.is_some_and(|x| t < x) {
            self.reset();
            return Telemetry::stopped("invalid time");
        }
        self.last_time = Some(t);
        let count=if self.single_input {1} else {2};
        let mut signal=0.;
        for (i,item) in self.feet.iter().take(count).enumerate() {
            let Some(foot)=item else {return Telemetry::stopped(if self.single_input {"waiting for chest acceleration"}else{"waiting for both feet"});};
            if t < foot.seen || t-foot.seen >= STALE {self.reset();return Telemetry::stopped(if self.single_input {"stale chest acceleration"} else {"stale foot"});}
            let magnitude:f64=foot.v.iter().zip(self.settings.axes[i]).filter_map(|(x,on)|on.then_some(x.abs())).sum();
            if magnitude>0.45 && t-foot.changed>=FROZEN {self.clear_history();return Telemetry::stopped(if self.single_input {"frozen chest acceleration"} else {"frozen foot"});}
            signal+=magnitude;
        }
        let signal=signal*self.settings.intensity;
        self.quiet_observations.push(t, signal);
        if self.settings.quiet_percent > 0. {
            let (range, mean) = self.quiet_observations.range_mean(t, 0.3);
            if mean > 0.01 && range < mean * self.settings.quiet_percent / 100. {
                let since = *self.quiet_since.get_or_insert(t);
                if t - since >= 0.25 {
                    self.samples.clear();
                    self.inertia.clear();
                    return Telemetry::stopped("quiet gate");
                }
            } else {
                self.quiet_since = None;
            }
        }
        self.samples.push(t, signal);
        let fast = self.samples.average(t, self.settings.fast_window);
        let slow = self.samples.average(t, self.settings.slow_window);
        let adaptive_window = (self.settings.fast_window + fast * self.settings.adaptive_per_unit)
            .clamp(self.settings.fast_window, self.settings.slow_window);
        let adaptive = self.samples.average(t, adaptive_window);
        self.inertia.push(t, adaptive);
        let inertia = self.inertia.average(t, self.settings.inertia_window);
        let raw_speed=(inertia / self.settings.max_signal).clamp(0.,1.);
        let speed = if self.settings.fast_stop && fast < self.settings.threshold {
            0.
        } else {
            let normalized = raw_speed;
            if self.settings.curve_mode == "custom" {
                curve(&self.settings.curve_points, normalized)
            } else {
                normalized.powf(self.settings.exponent)
            }
        };
        Telemetry {
            speed,
            raw_speed,
            sprint: self.settings.sprint_enabled && fast >= self.settings.sprint_threshold,
            signal,
            fast,
            slow,
            adaptive,
            inertia,
            status: "ready",
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn step(f: &mut MotionFilter, t: f64, l: [f64; 3], r: [f64; 3]) -> Telemetry {
        f.update_foot(0, l, t).unwrap();
        f.update_foot(1, r, t).unwrap();
        f.tick(t)
    }
    #[test]
    fn walking_smooth_preview_and_release() {
        let mut f = MotionFilter::new(Settings::default()).unwrap();
        let mut peak: f64 = 0.;
        for i in 0..100 {
            let t = i as f64 * 0.02;
            let wave = (t * 6.).sin() * 7.;
            let z = step(&mut f, t, [wave, 0., 0.], [-wave, 0., 0.]);
            peak = peak.max(z.speed);
        }
        assert!(peak > 0.1 && peak <= 1.);
        let mut end = Telemetry::stopped("");
        for i in 100..140 {
            end = step(&mut f, i as f64 * 0.02, [0.; 3], [0.; 3]);
        }
        assert_eq!(end.speed, 0.);
    }
    #[test]
    fn quiet_and_frozen() {
        let mut f = MotionFilter::new(Settings::default()).unwrap();
        for i in 0..100 {
            let n = if i % 2 == 0 { 0.02 } else { -0.02 };
            assert_eq!(
                step(&mut f, i as f64 * 0.02, [n, 0., 0.], [0.; 3]).speed,
                0.
            );
        }
        f.reset();
        let mut z = Telemetry::stopped("");
        for i in 0..26 {
            z = step(&mut f, i as f64 * 0.02, [5., 0., 0.], [0.; 3]);
        }
        assert_eq!(z.status, "frozen foot");
        assert_eq!(z.speed, 0.);
    }
    #[test]
    fn unequal_rates_and_dropout() {
        let mut f = MotionFilter::new(Settings::default()).unwrap();
        let mut z = Telemetry::stopped("");
        for i in 0..30 {
            let t = i as f64 * 0.02;
            f.update_foot(0, [(t * 8.).sin() * 5., 0., 0.], t).unwrap();
            if i % 5 == 0 {
                f.update_foot(1, [(t * 8.).cos() * 5., 0., 0.], t).unwrap();
            }
            z = f.tick(t);
            assert_eq!(z.status, "ready");
        }
        assert!(z.speed > 0.);
        let t = 0.85;
        f.update_foot(0, [2., 0., 0.], t).unwrap();
        z = f.tick(t);
        assert_eq!(z.status, "stale foot");
        assert_eq!(z.speed, 0.);
    }
    #[test]
    fn axes_json_invalid_time_and_nonfinite() {
        let mut cfg = Settings::default();
        cfg.axes[0][0] = false;
        let round = Settings::from_json(&cfg.to_json()).unwrap();
        let mut f = MotionFilter::new(round).unwrap();
        let z = step(&mut f, 1., [10., 0., 0.], [0., 0., 0.]);
        assert_eq!(z.signal, 0.);
        assert!(f.update_foot(1, [f64::NAN, 0., 0.], 1.1).is_err());
        assert_eq!(f.tick(1.1).speed, 0.);
        assert!(Settings::from_json(&json!({"max_signal":0.})).is_err());
        assert!(Settings::from_json(&json!({"slow_window":0.01})).is_err());
        step(&mut f, 2., [0.; 3], [0.; 3]);
        assert_eq!(f.tick(1.).status, "invalid time");
    }
    #[test]
    fn first_sample_is_not_full_window_and_arm_clear_keeps_inputs() {
        let mut f = MotionFilter::new(Settings::default()).unwrap();
        f.update_foot(0, [15., 0., 0.], 1.).unwrap();
        assert_eq!(f.tick(1.).speed, 0.);
        f.update_foot(1, [0., 0., 0.], 1.01).unwrap();
        assert_eq!(f.tick(1.01).speed, 0.);
        f.clear_history();
        assert!(f.inputs_fresh(1.01));
        assert_eq!(f.tick(1.01).speed, 0.);
    }
    #[test]
    fn frozen_one_foot_cannot_reactivate_on_identical_packets() {
        let mut f = MotionFilter::new(Settings::default()).unwrap();
        for i in 0..80 {
            let t = i as f64 * 0.02;
            let wave = (t * 9.).sin() * 7.;
            let z = step(&mut f, t, [5., 0., 0.], [wave, 0., 0.]);
            if t >= 0.5 {
                assert_eq!(z.status, "frozen foot");
                assert_eq!(z.speed, 0.);
            }
        }
        let t = 1.6;
        let z = step(&mut f, t, [6., 0., 0.], [1., 0., 0.]);
        assert_eq!(z.status, "ready");
    }
    #[test]
    fn quiet_gate_recovers_on_variation() {
        let mut settings = Settings::default();
        settings.quiet_percent = 30.;
        let mut f = MotionFilter::new(settings).unwrap();
        let mut latched = false;
        for i in 0..22 {
            let t = i as f64 * 0.02;
            let z = step(&mut f, t, [2., 0., 0.], [0., 0., 0.]);
            latched |= z.status == "quiet gate";
        }
        assert!(latched);
        let mut last = Telemetry::default();
        for i in 22..60 {
            let t = i as f64 * 0.02;
            let wave = (t * 10.).sin() * 9.;
            last = step(&mut f, t, [wave, 0., 0.], [-wave, 0., 0.]);
        }
        assert_eq!(last.status, "ready");
        assert!(last.speed > 0.);
    }
    #[test]
    fn infinity_backwards_and_all_axes_disabled_fail_closed() {
        let mut s = Settings::default();
        s.axes[1] = [false; 3];
        assert!(s.validate().is_err());
        let mut f = MotionFilter::new(Settings::default()).unwrap();
        assert!(f.update_foot(0, [f64::INFINITY, 0., 0.], 1.).is_err());
        assert!(f.update_foot(0, [0.; 3], f64::NAN).is_err());
        f.update_foot(0, [0.; 3], 2.).unwrap();
        assert!(f.update_foot(1, [0.; 3], 1.).is_err());
        assert!(!f.inputs_fresh(2.));
        assert_eq!(f.tick(f64::INFINITY).speed, 0.);
    }
    #[test]
    fn slow_accumulating_change_does_not_look_frozen() {
        let mut f = MotionFilter::new(Settings::default()).unwrap();
        for i in 0..55 {
            let t = i as f64 * 0.02;
            let value = 8. + t.sin();
            let z = step(&mut f, t, [value, 0., 0.], [0., 0., 0.]);
            assert_ne!(z.status, "frozen foot", "t={t}");
            assert!(f.inputs_fresh(t));
        }
    }
    #[test]
    fn custom_curve_roundtrip_and_validation() {
        let mut s = Settings::default();
        s.curve_mode = "custom".into();
        s.curve_points = vec![[0., 0.], [0.5, 0.2], [1., 1.]];
        let restored = Settings::from_json(&s.to_json()).unwrap();
        assert_eq!(restored.curve_points, s.curve_points);
        assert!((curve(&restored.curve_points, 0.25) - 0.1).abs() < 1e-12);
        assert!((curve(&restored.curve_points, 0.75) - 0.6).abs() < 1e-12);
        s.curve_points = vec![[0., 0.], [0.5, 0.8], [0.7, 0.2], [1., 1.]];
        assert!(s.validate().is_err());
        s.curve_points = vec![[0., 0.], [0.5, 0.2], [0.5, 0.3], [1., 1.]];
        assert!(s.validate().is_err());
        s.curve_points = vec![[0., 0.], [0.9, 1.]];
        assert!(s.validate().is_err());
        assert_eq!(
            Settings::from_json(&json!({})).unwrap().curve_mode,
            "exponent"
        );
    }
    #[test]
    fn exponent_and_sprint_preview_are_gated() {
        let mut s = Settings::default();
        s.sprint_enabled = true;
        let mut f = MotionFilter::new(s).unwrap();
        let z = step(&mut f, 0., [30., 0., 0.], [0.; 3]);
        assert!(!z.sprint && z.speed == 0.);
        let mut saw = false;
        for i in 1..15 {
            let t = i as f64 * 0.02;
            let x = if i % 2 == 0 { 35. } else { 40. };
            let z = step(&mut f, t, [x, 0., 0.], [0.; 3]);
            saw |= z.sprint;
        }
        assert!(saw);
        let z = f.tick(2.);
        assert!(!z.sprint && z.speed == 0.);
        let mut s = Settings::default();
        s.sprint_enabled = true;
        s.sprint_threshold = 2.;
        assert!(s.validate().is_err());
    }
    #[test]
    fn exponent_mode_changes_normalized_preview() {
        let linear = Settings::default();
        let mut squared = linear.clone();
        squared.exponent = 2.;
        let mut a = MotionFilter::new(linear).unwrap();
        let mut b = MotionFilter::new(squared).unwrap();
        let mut av = 0.;
        let mut bv = 0.;
        let mut saw = false;
        for i in 0..80 {
            let t = i as f64 * 0.02;
            let wave = (t * 8.).sin() * 4.;
            av = step(&mut a, t, [wave, 0., 0.], [-wave, 0., 0.]).speed;
            bv = step(&mut b, t, [wave, 0., 0.], [-wave, 0., 0.]).speed;
            if av > 0. {
                saw = true;
                assert!((bv - av * av).abs() < 1e-10);
            }
        }
        assert!(saw && av >= 0. && bv >= 0.);
    }
}
