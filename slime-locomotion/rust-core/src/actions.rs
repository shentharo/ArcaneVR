//! Independent, preview-only body actions; no runtime output or input injection.
//! Angles are radians, height is metres, acceleration is device-frame m/s².
use serde_json::{json, Value};
use std::collections::VecDeque;
const FRESH: f64 = 0.25;
#[derive(Clone, Debug)]
pub struct Direction {
    pub enabled: bool,
    pub deadzone: f64,
    pub forward_deadzone: Option<f64>,
    pub backward_deadzone: Option<f64>,
    pub left_deadzone: Option<f64>,
    pub right_deadzone: Option<f64>,
    pub max_angle: f64,
    pub horizontal_max_angle: Option<f64>,
    pub forward_max_angle: Option<f64>,
    pub forward_offset: f64,
    pub smoothing_ms: f64,
    pub exponent: f64,
    pub pitch_sign: f64,
    pub roll_sign: f64,
    pub pitch_axis: String,
    pub roll_axis: String,
    pub decouple: bool,
    pub decouple_strength: f64,
    pub forward_only: bool,
    pub strafe_enabled: bool,
    pub sprint_full_forward: bool,
    pub sprint_angle: Option<f64>,
    pub backward_deadzone_multiplier: f64,
    pub max_output: f64,
    pub learned_forward: Option<[f64;3]>,
    pub learned_right: Option<[f64;3]>,
}
#[derive(Clone, Debug)]
pub struct Jump {
    pub enabled: bool,
    pub source: String,
    pub accel_axis: String,
    pub height_window_seconds: f64,
    pub accel_window_seconds: f64,
    pub angle_compensation_percent: f64,
    pub compensation_axis: String,
    pub suppress_threshold: f64,
    pub threshold: f64,
    pub release: f64,
    pub pulse_seconds: f64,
    pub cooldown_seconds: f64,
}
#[derive(Clone, Debug)]
pub struct Crouch {
    pub enabled: bool,
    pub enter: f64,
    pub exit: f64,
    pub mode: String,
    pub height_correction: f64,
    pub use_reset_offset: bool,
    pub reference_height: f64,
    pub pulse_seconds: f64,
    pub slowdown: f64,
}
#[derive(Clone, Debug)]
pub struct Turn {
    pub enabled: bool,
    pub horizontal: bool,
    pub vertical: bool,
    pub deadzone: f64,
    pub max_angle: f64,
    pub exponent: f64,
    pub yaw_sign: f64,
    pub pitch_sign: f64,
}
#[derive(Clone, Debug, Default)]
pub struct Settings {
    pub direction: Direction,
    pub jump: Jump,
    pub crouch: Crouch,
    pub turn: Turn,
}
impl Default for Direction {
    fn default() -> Self {
        Self {
            enabled: false,
            deadzone: 0.10,
            forward_deadzone: None, backward_deadzone: None, left_deadzone: None, right_deadzone: None,
            max_angle: 0.65,
            horizontal_max_angle: None,
            forward_max_angle: None,
            forward_offset: 0.,
            smoothing_ms: 0.,
            exponent: 1.,
            pitch_sign: 1.,
            roll_sign: 1.,
            pitch_axis: "pitch".into(),
            roll_axis: "roll".into(),
            decouple: true,
            decouple_strength: 1.,
            forward_only: false,
            strafe_enabled: true,
            sprint_full_forward: false, sprint_angle:None,
            backward_deadzone_multiplier: 1.,
            max_output: 1.,
            learned_forward: None,
            learned_right: None,
        }
    }
}
impl Default for Jump {
    fn default() -> Self {
        Self {
            enabled: false,
            source: "height".into(),
            accel_axis: "magnitude".into(),
            height_window_seconds: 0.15,
            accel_window_seconds: 0.06,
            angle_compensation_percent: 0.,
            compensation_axis: "pitch".into(),
            suppress_threshold: 2.,
            threshold: 0.15,
            release: 0.06,
            pulse_seconds: 0.5,
            cooldown_seconds: 0.5,
        }
    }
}
impl Default for Crouch {
    fn default() -> Self {
        Self {
            enabled: false,
            enter: 1.10,
            exit: 1.40,
            mode: "hold".into(),
            height_correction: 0.,
            use_reset_offset: false,
            reference_height: 1.7,
            pulse_seconds: 0.15,
            slowdown: 0.5,
        }
    }
}
impl Default for Turn {
    fn default() -> Self {
        Self {
            enabled: false,
            horizontal: true,
            vertical: false,
            deadzone: 0.10,
            max_angle: 0.8,
            exponent: 1.,
            yaw_sign: 1.,
            pitch_sign: 1.,
        }
    }
}
fn bounded(x: f64, lo: f64, hi: f64) -> bool {
    x.is_finite() && (lo..=hi).contains(&x)
}
fn sign(x: f64) -> bool {
    x == 1. || x == -1.
}
impl Settings {
    pub fn validate(&self) -> Result<(), &'static str> {
        let d = &self.direction;
        let j = &self.jump;
        let c = &self.crouch;
        let t = &self.turn;
        if !bounded(d.deadzone, 0., 1.5)
            || !bounded(d.max_angle, 0.01, 3.14)
            || d.deadzone >= d.max_angle
            || d.horizontal_max_angle.is_some_and(|v| !bounded(v, 0.01, 3.14) || v <= d.deadzone)
            || d.forward_max_angle.is_some_and(|v| !bounded(v, 0.01, 3.14) || v <= d.deadzone * d.backward_deadzone_multiplier)
            || !bounded(d.forward_offset, -1.5, 1.5)
            || !bounded(d.smoothing_ms, 0., 2000.)
            || d.forward_deadzone.is_some_and(|v| !bounded(v,0.,1.5) || v>=d.forward_max_angle.unwrap_or(d.max_angle))
            || d.backward_deadzone.is_some_and(|v| !bounded(v,0.,1.5) || v>=d.forward_max_angle.unwrap_or(d.max_angle))
            || d.left_deadzone.is_some_and(|v| !bounded(v,0.,1.5) || v>=d.horizontal_max_angle.unwrap_or(d.max_angle))
            || d.right_deadzone.is_some_and(|v| !bounded(v,0.,1.5) || v>=d.horizontal_max_angle.unwrap_or(d.max_angle))
            || d.sprint_angle.is_some_and(|v| !bounded(v,0.01,3.14) || v<d.forward_max_angle.unwrap_or(d.max_angle).max(d.horizontal_max_angle.unwrap_or(d.max_angle)))
            || !bounded(d.exponent, 0.1, 5.)
            || !sign(d.pitch_sign)
            || !sign(d.roll_sign)
            || !["yaw", "pitch", "roll", "-yaw", "-pitch", "-roll"].contains(&d.pitch_axis.as_str())
            || !["yaw", "pitch", "roll", "-yaw", "-pitch", "-roll"].contains(&d.roll_axis.as_str())
            || !bounded(d.decouple_strength, 0., 1.)
            || !bounded(d.max_output, 0.05, 1.)
            || !bounded(d.backward_deadzone_multiplier,1.,3.) || d.deadzone*d.backward_deadzone_multiplier>=d.max_angle
            || d.learned_right.is_some_and(|v|v.iter().any(|x|!x.is_finite()) || (v.iter().map(|x|x*x).sum::<f64>()-1.).abs()>0.01)
            || d.learned_forward.is_some_and(|v|v.iter().any(|x|!x.is_finite()) || (v.iter().map(|x|x*x).sum::<f64>()-1.).abs()>0.01)
        {
            return Err("invalid direction settings");
        }
        if !["height", "acceleration"].contains(&j.source.as_str())
            || !["magnitude", "x", "-x", "y", "-y", "z", "-z"].contains(&j.accel_axis.as_str())
            || !bounded(j.height_window_seconds, 0.01, 5.)
            || !bounded(j.accel_window_seconds, 0.01, 5.)
            || !bounded(j.angle_compensation_percent, 0., 100.)
            || !["yaw", "pitch", "roll"].contains(&j.compensation_axis.as_str())
            || !bounded(j.suppress_threshold, 0.01, 100.)
            || !bounded(j.threshold, 0.01, 100.)
            || !bounded(j.release, 0., 100.)
            || j.release >= j.threshold
            || !bounded(j.pulse_seconds, 0.02, 2.)
            || !bounded(j.cooldown_seconds, 0., 5.)
        {
            return Err("invalid jump settings");
        }
        if !bounded(c.enter, 0.05, 4.)
            || !bounded(c.exit, 0.05, 4.)
            || c.enter >= c.exit
            || !bounded(c.height_correction, -2., 2.)
            || !bounded(c.reference_height, 0.05, 4.)
            || !["hold", "pulse"].contains(&c.mode.as_str())
            || !bounded(c.pulse_seconds, 0.02, 2.)
            || !bounded(c.slowdown, 0., 1.)
        {
            return Err("invalid crouch settings");
        }
        if !bounded(t.deadzone, 0., 1.5)
            || !bounded(t.max_angle, 0.01, 3.14)
            || t.deadzone >= t.max_angle
            || !bounded(t.exponent, 0.1, 5.)
            || !sign(t.yaw_sign)
            || !sign(t.pitch_sign)
        {
            return Err("invalid turn settings");
        }
        Ok(())
    }
    pub fn to_json(&self) -> Value {
        let d = &self.direction;
        let j = &self.jump;
        let c = &self.crouch;
        let t = &self.turn;
        json!({"direction":{"enabled":d.enabled,"deadzone":d.deadzone,"forward_deadzone":d.forward_deadzone,"backward_deadzone":d.backward_deadzone,"left_deadzone":d.left_deadzone,"right_deadzone":d.right_deadzone,"max_angle":d.max_angle,"horizontal_max_angle":d.horizontal_max_angle,"forward_max_angle":d.forward_max_angle,"forward_offset":d.forward_offset,"smoothing_ms":d.smoothing_ms,"exponent":d.exponent,"pitch_sign":d.pitch_sign,"roll_sign":d.roll_sign,"pitch_axis":d.pitch_axis,"roll_axis":d.roll_axis,"decouple":d.decouple,"decouple_strength":d.decouple_strength,"forward_only":d.forward_only,"strafe_enabled":d.strafe_enabled,"sprint_full_forward":d.sprint_full_forward,"sprint_angle":d.sprint_angle,"backward_deadzone_multiplier":d.backward_deadzone_multiplier,"max_output":d.max_output,"learned_forward":d.learned_forward,"learned_right":d.learned_right},"jump":{"enabled":j.enabled,"source":j.source,"accel_axis":j.accel_axis,"height_window_seconds":j.height_window_seconds,"accel_window_seconds":j.accel_window_seconds,"angle_compensation_percent":j.angle_compensation_percent,"compensation_axis":j.compensation_axis,"suppress_threshold":j.suppress_threshold,"threshold":j.threshold,"release":j.release,"pulse_seconds":j.pulse_seconds,"cooldown_seconds":j.cooldown_seconds},"crouch":{"enabled":c.enabled,"enter":c.enter,"exit":c.exit,"mode":c.mode,"height_correction":c.height_correction,"use_reset_offset":c.use_reset_offset,"reference_height":c.reference_height,"pulse_seconds":c.pulse_seconds,"slowdown":c.slowdown},"turn":{"enabled":t.enabled,"horizontal":t.horizontal,"vertical":t.vertical,"deadzone":t.deadzone,"max_angle":t.max_angle,"exponent":t.exponent,"yaw_sign":t.yaw_sign,"pitch_sign":t.pitch_sign}})
    }
    pub fn from_json(v: &Value) -> Result<Self, &'static str> {
        let obj = v.as_object().ok_or("settings must be object")?;
        let mut s = Self::default();
        for (section, val) in obj {
            let o = val.as_object().ok_or("section must be object")?;
            for (k, v) in o {
                macro_rules! number {
                    ($x:expr) => {{
                        $x = v.as_f64().ok_or("expected number")?
                    }};
                }
                macro_rules! boolean {
                    ($x:expr) => {{
                        $x = v.as_bool().ok_or("expected boolean")?
                    }};
                }
                macro_rules! text {
                    ($x:expr) => {{
                        $x = v.as_str().ok_or("expected text")?.to_owned()
                    }};
                }
                match (section.as_str(), k.as_str()) {
                    ("direction", "enabled") => boolean!(s.direction.enabled),
                    ("direction", "deadzone") => number!(s.direction.deadzone),
                    ("direction", "forward_deadzone" | "backward_deadzone" | "left_deadzone" | "right_deadzone") => {
                        let value=if v.is_null(){None}else{Some(v.as_f64().ok_or("expected number")?)};
                        match k.as_str(){"forward_deadzone"=>s.direction.forward_deadzone=value,"backward_deadzone"=>s.direction.backward_deadzone=value,"left_deadzone"=>s.direction.left_deadzone=value,_=>s.direction.right_deadzone=value}
                    }
                    ("direction", "max_angle") => number!(s.direction.max_angle),
                    ("direction", "horizontal_max_angle") => s.direction.horizontal_max_angle = if v.is_null() { None } else { Some(v.as_f64().ok_or("expected number")?) },
                    ("direction", "sprint_angle") => s.direction.sprint_angle = if v.is_null() {None}else{Some(v.as_f64().ok_or("expected number")?)},
                    ("direction", "forward_max_angle") => s.direction.forward_max_angle = if v.is_null() { None } else { Some(v.as_f64().ok_or("expected number")?) },
                    ("direction", "forward_offset") => number!(s.direction.forward_offset),
                    ("direction", "smoothing_ms") => number!(s.direction.smoothing_ms),
                    ("direction", "exponent") => number!(s.direction.exponent),
                    ("direction", "pitch_sign") => number!(s.direction.pitch_sign),
                    ("direction", "roll_sign") => number!(s.direction.roll_sign),
                    ("direction", "pitch_axis") => text!(s.direction.pitch_axis),
                    ("direction", "roll_axis") => text!(s.direction.roll_axis),
                    ("direction", "decouple") => boolean!(s.direction.decouple),
                    ("direction", "learned_forward" | "learned_right") => {
                        let axis=if v.is_null(){None}else{let a=v.as_array().filter(|a|a.len()==3).ok_or("expected three axis components")?;Some([a[0].as_f64().ok_or("invalid axis")?,a[1].as_f64().ok_or("invalid axis")?,a[2].as_f64().ok_or("invalid axis")?])};
                        if k=="learned_right" {s.direction.learned_right=axis;}else{s.direction.learned_forward=axis;}
                    }
                    ("direction", "backward_deadzone_multiplier") => number!(s.direction.backward_deadzone_multiplier),
                    ("direction", "forward_only") => boolean!(s.direction.forward_only),
                    ("direction", "strafe_enabled") => boolean!(s.direction.strafe_enabled),
                    ("direction", "sprint_full_forward") => boolean!(s.direction.sprint_full_forward),
                    ("direction", "max_output") => number!(s.direction.max_output),
                    ("direction", "decouple_strength") => number!(s.direction.decouple_strength),
                    ("jump", "enabled") => boolean!(s.jump.enabled),
                    ("jump", "source") => text!(s.jump.source),
                    ("jump", "accel_axis") => text!(s.jump.accel_axis),
                    ("jump", "height_window_seconds") => number!(s.jump.height_window_seconds),
                    ("jump", "accel_window_seconds") => number!(s.jump.accel_window_seconds),
                    ("jump", "angle_compensation_percent") => {
                        number!(s.jump.angle_compensation_percent)
                    }
                    ("jump", "compensation_axis") => text!(s.jump.compensation_axis),
                    ("jump", "suppress_threshold") => number!(s.jump.suppress_threshold),
                    ("jump", "threshold") => number!(s.jump.threshold),
                    ("jump", "release") => number!(s.jump.release),
                    ("jump", "pulse_seconds") => number!(s.jump.pulse_seconds),
                    ("jump", "cooldown_seconds") => number!(s.jump.cooldown_seconds),
                    ("crouch", "enabled") => boolean!(s.crouch.enabled),
                    ("crouch", "enter") => number!(s.crouch.enter),
                    ("crouch", "exit") => number!(s.crouch.exit),
                    ("crouch", "mode") => text!(s.crouch.mode),
                    ("crouch", "height_correction") => number!(s.crouch.height_correction),
                    ("crouch", "use_reset_offset") => boolean!(s.crouch.use_reset_offset),
                    ("crouch", "reference_height") => number!(s.crouch.reference_height),
                    ("crouch", "pulse_seconds") => number!(s.crouch.pulse_seconds),
                    ("crouch", "slowdown") => number!(s.crouch.slowdown),
                    ("turn", "enabled") => boolean!(s.turn.enabled),
                    ("turn", "horizontal") => boolean!(s.turn.horizontal),
                    ("turn", "vertical") => boolean!(s.turn.vertical),
                    ("turn", "deadzone") => number!(s.turn.deadzone),
                    ("turn", "max_angle") => number!(s.turn.max_angle),
                    ("turn", "exponent") => number!(s.turn.exponent),
                    ("turn", "yaw_sign") => number!(s.turn.yaw_sign),
                    ("turn", "pitch_sign") => number!(s.turn.pitch_sign),
                    _ => return Err("unknown action setting"),
                }
            }
        }
        s.validate()?;
        Ok(s)
    }
}
#[derive(Clone, Copy, Debug)]
pub struct TimedQuaternion {
    pub value: [f64; 4],
    pub seen: f64,
}
#[derive(Clone, Copy, Debug)]
pub struct TimedVector {
    pub value: [f64; 3],
    pub seen: f64,
}
#[derive(Clone, Copy, Debug)]
pub struct TimedHmd {
    pub height: f64,
    pub yaw: f64,
    pub pitch: f64,
    pub roll: f64,
    pub seen: f64,
}
#[derive(Clone, Copy, Debug)]
pub struct Input {
    pub t: f64,
    pub chest: Option<TimedQuaternion>,
    pub chest_accel: Option<TimedVector>,
    pub hmd: Option<TimedHmd>,
}
#[derive(Clone, Copy, Debug)]
pub struct Telemetry {
    pub travel_xy: [f64; 2],
    pub training_xy: [f64; 2],
    pub raw_direction_angles: Option<[f64; 2]>,
    pub chest_yaw: Option<f64>,
    pub sprint_pressed: bool,
    pub jump_pressed: bool,
    pub crouch_pressed: bool,
    pub crouching: bool,
    pub speed_multiplier: f64,
    pub turn_xy: [f64; 2],
    pub calibrated: bool,
    pub direction_status: &'static str,
    pub jump_status: &'static str,
    pub crouch_status: &'static str,
    pub turn_status: &'static str,
}
impl Default for Telemetry {
    fn default() -> Self {
        Self {
            travel_xy: [0.; 2],
            training_xy: [0.; 2],
            raw_direction_angles: None,
            chest_yaw: None,
            sprint_pressed: false,
            jump_pressed: false,
            crouch_pressed: false,
            crouching: false,
            speed_multiplier: 1.,
            turn_xy: [0.; 2],
            calibrated: false,
            direction_status: "disabled",
            jump_status: "disabled",
            crouch_status: "disabled",
            turn_status: "disabled",
        }
    }
}
fn fresh(t: f64, seen: f64) -> bool {
    t.is_finite() && seen.is_finite() && t >= seen && t - seen < FRESH
}
fn hmd(i: &Input) -> Option<TimedHmd> {
    let h = i.hmd?;
    if !fresh(i.t, h.seen)
        || !bounded(h.height, 0.05, 4.)
        || ![h.yaw, h.pitch, h.roll].iter().all(|x| x.is_finite())
    {
        return None;
    }
    Some(h)
}
fn angles(i: &Input) -> Option<[f64; 3]> {
    let s = i.chest?;
    if !fresh(i.t, s.seen) || !s.value.iter().all(|v| v.is_finite()) {
        return None;
    }
    let n = s.value.iter().map(|v| v * v).sum::<f64>().sqrt();
    if !(0.5..=1.5).contains(&n) {
        return None;
    }
    let [w, x, y, z] = s.value.map(|v| v / n);
    Some([
        (2. * (w * y + x * z)).atan2(1. - 2. * (x * x + y * y)),
        (2. * (w * x - y * z)).clamp(-1., 1.).asin(),
        (2. * (w * z + x * y)).atan2(1. - 2. * (x * x + z * z)),
    ])
}
fn wrap(a: f64) -> f64 {
    (a + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}
fn selected_delta(a: [f64; 3], center: [f64; 3], name: &str) -> f64 {
    let (negative, name) = name.strip_prefix('-').map_or((false, name), |n| (true, n));
    let i = match name {
        "yaw" => 0,
        "pitch" => 1,
        _ => 2,
    };
    wrap(a[i] - center[i]) * if negative { -1. } else { 1. }
}
fn axis(v: f64, dead: f64, max: f64, exp: f64) -> f64 {
    let n = ((v.abs() - dead) / (max - dead)).clamp(0., 1.).powf(exp);
    n * v.signum()
}
// Remove the directional ellipse radially, preserving direction and reaching full scale.
fn ellipse_axes(x:f64,y:f64,dx:f64,dy:f64,max_x:f64,max_y:f64,exp:f64)->(f64,f64) {
 let nx=x/max_x; let ny=y/max_y; let magnitude=nx.hypot(ny);
 if magnitude<1e-12 {return (0.,0.);}
 let ux=nx/magnitude; let uy=ny/magnitude;
 let term=|u:f64,d:f64,m:f64| if u.abs()<1e-12 {0.} else if d<=0. {f64::INFINITY} else {u.abs()*m/d};
 let sum=term(ux,dx,max_x).hypot(term(uy,dy,max_y));
 let boundary=if sum.is_infinite(){0.}else if sum>0. {1./sum}else{0.};
 let amount=((magnitude-boundary)/(1.-boundary).max(1e-9)).clamp(0.,1.).powf(exp);
 (ux*amount,uy*amount)
}
pub struct ActionProcessor {
    settings: Settings,
    chest_center: Option<[f64; 3]>,
    world_up: Option<[f64;3]>,
    chest_quaternion: Option<[f64;4]>,
    head_center: Option<TimedHmd>,
    walking_sway:bool,
    walking_forward_gain:Option<f64>,
    calibrated: bool,
    height_history: VecDeque<(f64, f64, f64)>,
    accel_history: VecDeque<(f64, f64)>,
    last_accel_seen: f64,
    suppress_until: f64,
    last_t: Option<f64>,
    jump_ready: bool,
    jump_until: f64,
    last_jump: f64,
    crouching: bool,
    crouch_initialized: bool,
    crouch_until: f64,
    sprint_active: bool,
    smoothed_travel: [f64; 2],
}
impl ActionProcessor {
    pub fn new(settings: Settings) -> Result<Self, &'static str> {
        settings.validate()?;
        Ok(Self {
            settings,
            chest_center: None,
            world_up: None,
            chest_quaternion: None,
            head_center: None,walking_sway:false,walking_forward_gain:None,
            calibrated: false,
            height_history: VecDeque::new(),
            accel_history: VecDeque::new(),
            last_accel_seen: -1e9,
            suppress_until: -1e9,
            last_t: None,
            jump_ready: true,
            jump_until: -1e9,
            last_jump: -1e9,
            crouching: false,
            crouch_initialized: false,
            crouch_until: -1e9,
            sprint_active: false,
            smoothed_travel: [0.; 2],
        })
    }
    pub fn set_world_up(&mut self, up:[f64;3]) { self.world_up=Some(up); }
    pub fn settings(&self) -> &Settings {
        &self.settings
    }
    pub fn set_settings(&mut self, settings: Settings) -> Result<(), &'static str> {
        settings.validate()?;
        self.settings = settings;
        self.reset();
        Ok(())
    }
    pub fn update_settings_preserving_center(&mut self, settings: Settings) -> Result<(), &'static str> {
        settings.validate()?;
        self.settings = settings;
        self.reset_transient();
        Ok(())
    }
    pub fn reset(&mut self) {
        self.chest_center = None;
        self.chest_quaternion=None;
        self.head_center = None;
        self.calibrated = false;
        self.last_t = None;
        self.release();
    }
    pub fn reset_transient(&mut self) {
        self.last_t = None;
        self.release();
    }
    fn release(&mut self) {
        self.height_history.clear();
        self.accel_history.clear();
        self.last_accel_seen = -1e9;
        self.suppress_until = -1e9;
        self.jump_ready = false;
        self.jump_until = -1e9;
        self.last_jump = -1e9;
        self.crouching = false;
        self.crouch_initialized = false;
        self.crouch_until = -1e9;
        self.sprint_active = false;
        self.smoothed_travel = [0.; 2];
    }
    pub fn calibrate_chest_preview(&mut self, input: &Input) -> Result<(), &'static str> {
        let settings = self.settings.clone();
        self.settings.direction.decouple = false;
        self.settings.turn.enabled = false;
        self.settings.crouch.enabled = false;
        self.settings.jump.angle_compensation_percent = 0.;
        let result = self.calibrate(input);
        self.settings = settings;
        result
    }
    pub fn set_walking_forward_gain(&mut self,gain:Option<f64>){self.walking_forward_gain=gain.map(|v|v.clamp(0.,1.));}
    pub fn set_walking_sway(&mut self,enabled:bool){self.walking_sway=enabled;}
    pub fn has_head_center(&self) -> bool { self.head_center.is_some() }
    pub fn has_fresh_head(input: &Input) -> bool { hmd(input).is_some() }
    pub fn process_chest_preview(&mut self, input: &Input) -> Telemetry {
        let decouple = self.settings.direction.decouple;
        self.settings.direction.decouple = false;
        let result = self.process(input);
        self.settings.direction.decouple = decouple;
        result
    }
    pub fn calibrate(&mut self, i: &Input) -> Result<(), &'static str> {
        self.reset();
        if !i.t.is_finite() {
            return Err("invalid time");
        }
        let chest = angles(i);
        let head = hmd(i);
        if self.settings.direction.enabled && chest.is_none() {
            return Err("fresh chest required");
        }
        if ((self.settings.direction.enabled && self.settings.direction.decouple)
            || self.settings.turn.enabled
            || (self.settings.jump.enabled && self.settings.jump.source == "height" && self.settings.jump.angle_compensation_percent > 0.)
            || (self.settings.crouch.enabled && self.settings.crouch.use_reset_offset))
            && head.is_none()
        {
            return Err("fresh HMD required");
        }
        self.chest_center = chest;
        self.chest_quaternion=i.chest.map(|s|s.value);
        self.head_center = head;
        self.calibrated = true;
        self.last_t = Some(i.t);
        self.release();
        Ok(())
    }
    fn relative_tilt(&self,i:&Input)->Option<[f64;3]>{
        if self.world_up.is_some() {return self.raw_relative_tilt(i);}
        self.relative_tilt_with_yaw(i, true)
    }
    fn raw_relative_tilt(&self,i:&Input)->Option<[f64;3]>{
        if let Some(up)=self.world_up {
            let q=i.chest.filter(|s|fresh(i.t,s.seen))?.value;
            return crate::mounting::level_tilt(q,self.chest_quaternion?,up);
        }
        self.relative_tilt_with_yaw(i, false)
    }
    fn relative_tilt_with_yaw(&self,i:&Input,remove_yaw:bool)->Option<[f64;3]>{
        let q=i.chest.filter(|s|fresh(i.t,s.seen))?.value;let n=self.chest_quaternion?;
        let norm=|q:[f64;4]|q.iter().map(|x|x*x).sum::<f64>().sqrt();let a=norm(q);let b=norm(n);if !a.is_finite()||!b.is_finite()||a<0.0001||b<0.0001{return None;}
        let [w,x,y,z]=q.map(|v|v/a);let [nw,nx,ny,nz]=n.map(|v|v/b);
        let mut d=[nw*w+nx*x+ny*y+nz*z,nw*x-nx*w-ny*z+nz*y,nw*y+nx*z-ny*w-nz*x,nw*z-nx*y+ny*x-nz*w];
        // Remove upright yaw twist before measuring lean, using the learned mounting axes.
        if let (true,Some(f),Some(r))=(remove_yaw,self.settings.direction.learned_forward,self.settings.direction.learned_right){
            let mut up=[f[1]*r[2]-f[2]*r[1],f[2]*r[0]-f[0]*r[2],f[0]*r[1]-f[1]*r[0]];
            let length=up.iter().map(|v|v*v).sum::<f64>().sqrt();if length>0.1{
                up=up.map(|v|v/length);let projection=d[1]*up[0]+d[2]*up[1]+d[3]*up[2];
                let norm=d[0].hypot(projection);if norm<1e-8{return None;}
                let t=d[0]/norm;let [tx,ty,tz]=up.map(|v|-v*projection/norm);let [w,x,y,z]=d;
                d=[t*w-tx*x-ty*y-tz*z,t*x+tx*w+ty*z-tz*y,t*y-tx*z+ty*w+tz*x,t*z+tx*y-ty*x+tz*w];
            }
        }
        if d[0]<0.{d=d.map(|v|-v);}
        let length=(d[1]*d[1]+d[2]*d[2]+d[3]*d[3]).sqrt();if length<1e-8{return Some([0.;3]);}
        let scale=2.*length.atan2(d[0])/length;Some([d[1]*scale,d[2]*scale,d[3]*scale])
    }
    pub fn learn_forward(&mut self,i:&Input)->Result<(),&'static str>{
        if !self.calibrated{return Err("Reset Center upright first");}
        let v=self.raw_relative_tilt(i).ok_or("Fresh chest data required")?;let angle=v.iter().map(|x|x*x).sum::<f64>().sqrt();
        if !(5f64.to_radians()..=35f64.to_radians()).contains(&angle){return Err("Lean forward 5 to 35 degrees, then press Learn forward lean");}
        self.settings.direction.learned_forward=Some(v.map(|x|x/angle*self.settings.direction.pitch_sign));self.settings.direction.learned_right=None;Ok(())
    }
    pub fn learn_right(&mut self,i:&Input)->Result<(),&'static str>{
        if !self.calibrated{return Err("Reset Center upright first");}
        let forward=self.settings.direction.learned_forward.ok_or("Learn forward lean first")?;
        let mut v=self.raw_relative_tilt(i).ok_or("Fresh chest data required")?;
        let total=v.iter().map(|x|x*x).sum::<f64>().sqrt();
        if !(5f64.to_radians()..=35f64.to_radians()).contains(&total){return Err("Lean right 5 to 35 degrees, then press Learn right lean");}
        let overlap=v.iter().zip(forward).map(|(a,b)|a*b).sum::<f64>();
        for j in 0..3{v[j]-=overlap*forward[j];}
        let angle=v.iter().map(|x|x*x).sum::<f64>().sqrt();
        if angle<5f64.to_radians()||angle<total*0.7{return Err("Lean sideways to your right, keeping forward lean small");}
        self.settings.direction.learned_right=Some(v.map(|x|x/angle*self.settings.direction.roll_sign));Ok(())
    }
    pub fn process(&mut self, i: &Input) -> Telemetry {
        let mut out = Telemetry::default();
        if !i.t.is_finite() || self.last_t.is_some_and(|t| i.t < t) {
            self.reset();
            return out;
        }
        let dt = self.last_t.map_or(0., |t| i.t - t);
        self.last_t = Some(i.t);
        if !self.calibrated {
            if self.settings.direction.enabled {
                out.direction_status = "not calibrated";
            }
            if self.settings.jump.enabled {
                out.jump_status = "not calibrated";
            }
            if self.settings.crouch.enabled {
                out.crouch_status = "not calibrated";
            }
            if self.settings.turn.enabled {
                out.turn_status = "not calibrated";
            }
            return out;
        }
        let chest_center = self.chest_center;
        let head_center = self.head_center;
        out.calibrated = true;
        let head = hmd(i);
        let chest = angles(i);
        let d = &self.settings.direction;
        if d.enabled {
            let horizontal_max = d.horizontal_max_angle.unwrap_or(d.max_angle);
            let forward_max = d.forward_max_angle.unwrap_or(d.max_angle);
            out.direction_status = "missing chest";
            if let (Some(a), Some(chest_center)) = (chest, chest_center) {
                out.direction_status = "ready";
                let chest_heading = match (i.chest,self.chest_quaternion,self.world_up) {
                    (Some(q),Some(neutral),Some(up)) => crate::mounting::heading(q.value,neutral,up),
                    _ => Some(wrap(a[0]-chest_center[0])),
                };
                out.chest_yaw=chest_heading;
                let mut raw_x=selected_delta(a,chest_center,&d.roll_axis)*d.roll_sign;
                let mut raw_y=selected_delta(a,chest_center,&d.pitch_axis)*d.pitch_sign;
                if let Some(vector)=d.learned_forward {
                    if let Some(tilt)=self.relative_tilt(i){raw_y=tilt.iter().zip(vector).map(|(a,b)|a*b).sum::<f64>()*d.pitch_sign;}
                    else {raw_x=0.;raw_y=0.;out.direction_status="missing chest";}
                } else if let Some(tilt)=self.relative_tilt(i) {
                    let value=|name:&str| {let (sign,n)=name.strip_prefix('-').map_or((1.,name),|n|(-1.,n));match n {"pitch"=>Some(sign*tilt[0]),"roll"=>Some(sign*tilt[2]),_=>None}};
                    if let Some(v)=value(&d.pitch_axis){raw_y=v*d.pitch_sign;}
                    if let Some(v)=value(&d.roll_axis){raw_x=v*d.roll_sign;}
                }
                if let (Some(vector),Some(tilt))=(d.learned_right,self.relative_tilt(i)) {raw_x=tilt.iter().zip(vector).map(|(a,b)|a*b).sum::<f64>()*d.roll_sign;}
                if let (Some(f),Some(r),Some(q),Some(neutral))=(d.learned_forward,d.learned_right,i.chest,self.chest_quaternion) {
                    let forward=f.map(|v|v*d.pitch_sign);let right=r.map(|v|v*d.roll_sign);
                    let lean=if let Some(up)=self.world_up {crate::mounting::lean_with_up(q.value,neutral,forward,right,up)} else {crate::mounting::lean(q.value,neutral,forward,right)};
                    if let Some([x,y])=lean{raw_x=x;raw_y=y;}
                    else {raw_x=0.;raw_y=0.;out.direction_status="invalid mounting reference";}
                }
                out.raw_direction_angles=Some([raw_x,raw_y]);
                let adjusted_y=raw_y-d.forward_offset;
                let dx=if raw_x<0. {d.left_deadzone.unwrap_or(d.deadzone)} else {d.right_deadzone.unwrap_or(d.deadzone)};
                let dy=if adjusted_y<0. {d.backward_deadzone.unwrap_or(d.deadzone*d.backward_deadzone_multiplier)} else {d.forward_deadzone.unwrap_or(d.deadzone)};
                let (mut x,mut y)=ellipse_axes(raw_x,adjusted_y,dx,dy,horizontal_max,forward_max,d.exponent);
                let mut training_x=x;
                if d.learned_forward.is_some() && d.learned_right.is_none() {training_x=0.;}
                let training_length=training_x.hypot(y);
                let training_scale=training_length.max(1.);
                out.training_xy=[training_x/training_scale,y/training_scale];
                if self.walking_sway && adjusted_y.abs()>0.05 { x=0.; }
                if let Some(gain)=self.walking_forward_gain {y*=gain;}
                if d.forward_only { y=y.max(0.); }
                if d.decouple {
                    if let (Some(h), Some(head_center), Some(chest_heading)) = (head, head_center, chest_heading) {
                        let delta = wrap(wrap(h.yaw - head_center.yaw) - chest_heading);
                        let theta = if d.decouple_strength == 0. {
                            0.
                        } else {
                            (d.decouple_strength * delta.sin()).atan2(delta.cos())
                        };
                        let (sin, cos) = theta.sin_cos();
                        (x, y) = (x * cos + y * sin, y * cos - x * sin);
                    } else {
                        x = 0.;
                        y = 0.;
                        out.direction_status = "missing HMD";
                    }
                }
                if !d.strafe_enabled || (d.learned_forward.is_some() && d.learned_right.is_none()) { x = 0.; }
                let len = (x * x + y * y).sqrt();
                if len > 1. {
                    x /= len;
                    y /= len;
                }
                out.travel_xy = [x*d.max_output, y*d.max_output];
            }
        }
        if d.enabled && out.direction_status == "ready" {
            let alpha = if d.smoothing_ms <= 0. { 1. } else { 1. - (-dt / (d.smoothing_ms / 1000.)).exp() };
            for axis in 0..2 { self.smoothed_travel[axis] += alpha * (out.travel_xy[axis] - self.smoothed_travel[axis]); }
            out.travel_xy = self.smoothed_travel;
        } else { self.smoothed_travel = [0.; 2]; }
        if d.enabled && d.sprint_full_forward && out.direction_status == "ready" {
            let full=d.forward_max_angle.unwrap_or(d.max_angle).max(d.horizontal_max_angle.unwrap_or(d.max_angle));
            let threshold=d.sprint_angle.unwrap_or((full*1.15).min(3.14)).max(full);
            let magnitude=out.raw_direction_angles.map_or(0.,|[x,y]|x.hypot(y-d.forward_offset));
            self.sprint_active=magnitude>threshold;
        } else { self.sprint_active=false; }
        out.sprint_pressed=self.sprint_active;
        let j = &self.settings.jump;
        if j.enabled {
            out.jump_status = if j.source == "height" {
                "missing HMD"
            } else {
                "missing chest acceleration"
            };
            let sample = if j.source == "height" {
                head.map(|h| {
                    let angle = match j.compensation_axis.as_str() {
                        "yaw" => h.yaw,
                        "roll" => h.roll,
                        _ => h.pitch,
                    };
                    if self.height_history.back().is_none_or(|v| h.seen > v.0) {
                        self.height_history.push_back((h.seen, h.height, angle));
                    }
                    while self
                        .height_history
                        .front()
                        .is_some_and(|v| i.t - v.0 > j.height_window_seconds)
                    {
                        self.height_history.pop_front();
                    }
                    while self.height_history.len() > 1024 {
                        self.height_history.pop_front();
                    }
                    let low = self
                        .height_history
                        .iter()
                        .map(|v| v.1)
                        .fold(h.height, f64::min);
                    let origin = self.height_history.front().map_or(angle, |v| v.2);
                    let (min_angle, max_angle) = self
                        .height_history
                        .iter()
                        .map(|v| wrap(v.2 - origin))
                        .fold((0f64, 0f64), |(lo, hi), v| (lo.min(v), hi.max(v)));
                    let range = max_angle - min_angle;
                    (h.height
                        - low
                        - (j.angle_compensation_percent / 100.)
                            * (range / (std::f64::consts::PI / 2.))
                            * 5.)
                        .max(0.)
                })
            } else {
                i.chest_accel
                    .filter(|v| {
                        fresh(i.t, v.seen)
                            && v.value.iter().all(|x| x.is_finite() && x.abs() <= 200.)
                    })
                    .map(|v| {
                        let raw = match j.accel_axis.as_str() {
                            "x" => v.value[0],
                            "-x" => -v.value[0],
                            "y" => v.value[1],
                            "-y" => -v.value[1],
                            "z" => v.value[2],
                            "-z" => -v.value[2],
                            _ => v.value.iter().map(|x| x * x).sum::<f64>().sqrt(),
                        };
                        if raw <= j.release
                            && self
                                .accel_history
                                .iter()
                                .all(|sample| sample.1 <= j.release)
                        {
                            self.jump_ready = true;
                        }
                        if v.seen > self.last_accel_seen {
                            self.last_accel_seen = v.seen;
                            self.accel_history.push_back((v.seen, raw));
                            if raw < -j.suppress_threshold {
                                self.suppress_until = v.seen + 0.5;
                                self.jump_until = -1e9;
                            }
                        }
                        while self
                            .accel_history
                            .front()
                            .is_some_and(|x| i.t - x.0 > j.accel_window_seconds)
                        {
                            self.accel_history.pop_front();
                        }
                        while self.accel_history.len() > 1024 {
                            self.accel_history.pop_front();
                        }
                        self.accel_history.iter().map(|x| x.1).sum::<f64>()
                            / self.accel_history.len().max(1) as f64
                    })
            };
            if let Some(v) = sample {
                out.jump_status = "ready";
                if j.source == "height" && v <= j.release {
                    self.jump_ready = true;
                }
                if self.jump_ready
                    && v > j.threshold
                    && i.t >= self.suppress_until
                    && i.t - self.last_jump >= j.cooldown_seconds
                {
                    self.jump_ready = false;
                    self.last_jump = i.t;
                    self.jump_until = i.t + j.pulse_seconds;
                }
                out.jump_pressed = i.t < self.jump_until && i.t >= self.suppress_until;
            } else {
                self.jump_ready = false;
                self.jump_until = -1e9;
                self.height_history.clear();
                self.accel_history.clear();
            }
        } else {
            self.jump_ready = false;
            self.jump_until = -1e9;
            self.height_history.clear();
            self.accel_history.clear();
        }
        let c = &self.settings.crouch;
        if c.enabled {
            out.crouch_status = "missing HMD";
            if let Some(h) = head.filter(|_| !c.use_reset_offset || head_center.is_some()) {
                out.crouch_status = "ready";
                let adjusted = h.height
                    + c.height_correction
                    + if c.use_reset_offset {
                        c.reference_height - head_center.unwrap().height
                    } else {
                        0.
                    };
                if !self.crouch_initialized {
                    self.crouching = adjusted < c.enter;
                    self.crouch_initialized = true;
                } else if !self.crouching && adjusted < c.enter {
                    self.crouching = true;
                    self.crouch_until = i.t + c.pulse_seconds;
                } else if self.crouching && adjusted > c.exit {
                    self.crouching = false;
                    self.crouch_until = i.t + c.pulse_seconds;
                }
                out.crouching = self.crouching;
                out.crouch_pressed = if c.mode == "hold" {
                    self.crouching
                } else {
                    i.t < self.crouch_until
                };
                if self.crouching {
                    out.speed_multiplier = c.slowdown;
                }
            } else {
                self.crouching = false;
                self.crouch_initialized = false;
                self.crouch_until = -1e9;
            }
        } else {
            self.crouching = false;
            self.crouch_until = -1e9;
        }
        let t = &self.settings.turn;
        if t.enabled {
            out.turn_status = "missing HMD";
            if let (Some(h), Some(head_center)) = (head, head_center) {
                out.turn_status = "ready";
                if t.horizontal {
                    out.turn_xy[0] = axis(
                        wrap(h.yaw - head_center.yaw) * t.yaw_sign,
                        t.deadzone,
                        t.max_angle,
                        t.exponent,
                    );
                }
                if t.vertical {
                    out.turn_xy[1] = axis(
                        wrap(h.pitch - head_center.pitch) * t.pitch_sign,
                        t.deadzone,
                        t.max_angle,
                        t.exponent,
                    );
                }
            }
        }
        out
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn input(t: f64) -> Input {
        Input {
            t,
            chest: Some(TimedQuaternion {
                value: [1., 0., 0., 0.],
                seen: t,
            }),
            chest_accel: Some(TimedVector {
                value: [0.; 3],
                seen: t,
            }),
            hmd: Some(TimedHmd {
                height: 1.7,
                yaw: 0.,
                pitch: 0.,
                roll: 0.,
                seen: t,
            }),
        }
    }
    #[test] fn walking_gate_preserves_crosshair_and_strafe() {
        let mut s=Settings::default();s.direction.enabled=true;s.direction.decouple=false;s.direction.smoothing_ms=0.;s.direction.learned_forward=Some([1.,0.,0.]);s.direction.learned_right=Some([0.,1.,0.]);
        let mut p=ActionProcessor::new(s).unwrap();p.set_world_up([0.,0.,1.]);p.calibrate(&input(0.)).unwrap();p.set_walking_forward_gain(Some(0.));
        let mut i=input(0.1);i.chest.as_mut().unwrap().value=[0.2f64.cos(),0.2f64.sin(),0.,0.];let out=p.process(&i);assert!(out.raw_direction_angles.unwrap()[1]>0.3);assert_eq!(out.travel_xy,[0.,0.]);
        i=input(0.2);i.chest.as_mut().unwrap().value=[0.2f64.cos(),0.,0.2f64.sin(),0.];assert!(p.process(&i).travel_xy[0]>0.);
        p.set_walking_forward_gain(Some(1.));i=input(0.3);i.chest.as_mut().unwrap().value=[0.2f64.cos(),0.2f64.sin(),0.,0.];assert!(p.process(&i).travel_xy[1]>0.);
    }
    #[test]
    fn mounted_chest_deep_lean_stays_on_axis_with_live_headset_decoupling() {
        fn mul([a,b,c,d]:[f64;4],[w,x,y,z]:[f64;4])->[f64;4]{[a*w-b*x-c*y-d*z,a*x+b*w+c*z-d*y,a*y-b*z+c*w+d*x,a*z+b*y-c*x+d*w]}
        let mount=crate::gait::unit([-0.486,-0.385,0.470,0.628]).unwrap();
        let mut settings=Settings::default();settings.direction.enabled=true;settings.direction.decouple=true;settings.direction.smoothing_ms=0.;
        let mut p=ActionProcessor::new(settings).unwrap();p.set_world_up([0.,0.,1.]);
        let mut neutral=input(0.);neutral.chest.as_mut().unwrap().value=mount;p.calibrate(&neutral).unwrap();
        let mut i=input(0.1);let h=0.15f64;
        i.chest.as_mut().unwrap().value=mul([h.cos(),h.sin(),0.,0.],mount);p.learn_forward(&i).unwrap();
        i.chest.as_mut().unwrap().value=mul([h.cos(),0.,h.sin(),0.],mount);p.learn_right(&i).unwrap();
        for axis in [0,1] {for sign in [-1.,1.] {let mut previous=0.;for degrees in 0..=75 {
            let mut i=input(1.+axis as f64*200.+(sign+1.)*50.+degrees as f64/100.);
            // Reset timing between sweeps while retaining the same physical centre and learned axes.
            p.last_t=None;
            let h=(degrees as f64).to_radians()/2.;let mut q=[h.cos(),0.,0.,0.];q[axis+1]=h.sin()*sign;i.chest.as_mut().unwrap().value=mul(q,mount);
            let out=p.process(&i);let main=if axis==0 {1}else{0};let value=out.travel_xy[main]*sign;
            assert!(value+1e-8>=previous,"lean reversed at {degrees}: {:?}",out.travel_xy);previous=value;
            assert!(out.travel_xy[1-main].abs()<1e-8,"cross-axis drift at {degrees}: {:?}",out.travel_xy);
            assert!(out.chest_yaw.unwrap().abs()<1e-8);
        }}}
    }
    #[test]
    fn configured_world_up_keeps_turns_neutral_after_contaminated_learning() {
        let mut settings=Settings::default();settings.direction.enabled=true;settings.direction.smoothing_ms=0.;
        let mut p=ActionProcessor::new(settings).unwrap();p.set_world_up([0.,0.,1.]);p.calibrate(&input(0.)).unwrap();
        let mut i=input(0.1);let (s,c)=0.125f64.sin_cos();let (h,w)=0.05f64.sin_cos();
        i.chest.as_mut().unwrap().value=[w*c,w*s,h*s,h*c];p.learn_forward(&i).unwrap();
        i.chest.as_mut().unwrap().value=[w*c,-h*s,w*s,h*c];p.learn_right(&i).unwrap();
        for degree in -179..=179 {
            i.t+=0.01;i.chest.as_mut().unwrap().seen=i.t;
            let a=(degree as f64).to_radians()/2.;i.chest.as_mut().unwrap().value=[a.cos(),0.,0.,a.sin()];
            let out=p.process(&i);assert!(out.travel_xy.iter().all(|v|v.abs()<1e-9));
        }
        i.t+=1.;assert_eq!(p.process(&i).travel_xy,[0.,0.]);
    }
    #[test]
    fn defaults_are_disabled_and_roundtrip() {
        let s = Settings::from_json(&Settings::default().to_json()).unwrap();
        let mut p = ActionProcessor::new(s).unwrap();
        p.calibrate(&input(1.)).unwrap();
        let out = p.process(&input(1.1));
        assert_eq!(out.travel_xy, [0.; 2]);
        assert!(!out.jump_pressed && !out.crouch_pressed);
    }
    #[test]
    fn ellipse_boundary_handles_diagonal_and_zero_axes() {
        assert_eq!(ellipse_axes(0.04,0.04,0.1,0.1,1.,1.,1.),(0.,0.));
        let outside=ellipse_axes(0.08,0.08,0.1,0.1,1.,1.,1.);
        assert!(outside.0>0. && outside.1>0.);
        assert_eq!(ellipse_axes(0.,0.1,0.,0.2,1.,1.,1.),(0.,0.));
        assert!(ellipse_axes(0.01,0.,0.,0.2,1.,1.,1.).0>0.);
    }
    #[test]
    fn asymmetric_deadzones_and_raw_angles_are_independent() {
        let mut s=Settings::default();s.direction.enabled=true;s.direction.deadzone=0.;s.direction.forward_deadzone=Some(0.1);s.direction.backward_deadzone=Some(0.3);s.direction.left_deadzone=Some(0.25);s.direction.right_deadzone=Some(0.05);s.direction.learned_forward=Some([1.,0.,0.]);s.direction.learned_right=Some([0.,0.,1.]);
        let mut p=ActionProcessor::new(Settings::from_json(&s.to_json()).unwrap()).unwrap();p.calibrate(&input(0.)).unwrap();
        for (q,expected) in [([0.1f64.cos(),0.1f64.sin(),0.,0.],1),([0.1f64.cos(),-0.1f64.sin(),0.,0.],0),([0.1f64.cos(),0.,0.,0.1f64.sin()],1),([0.1f64.cos(),0.,0.,-0.1f64.sin()],0)] {let mut i=input(0.1);i.chest.as_mut().unwrap().value=q;let o=p.process(&i);assert_eq!((o.travel_xy[0].abs()+o.travel_xy[1].abs()>0.) as i32,expected);assert!(o.raw_direction_angles.is_some());assert!(o.chest_yaw.is_some());}
        s.direction.left_deadzone=Some(0.7);assert!(s.validate().is_err());
    }
    #[test]
    fn independent_sensitivity_offset_and_legacy_roundtrip() {
        let old = Settings::from_json(&json!({"direction":{"max_angle":0.8}})).unwrap();
        assert_eq!(old.direction.forward_max_angle, None);
        assert_eq!(old.direction.horizontal_max_angle, None);
        let mut s=old; s.direction.enabled=true; s.direction.deadzone=0.;
        s.direction.forward_max_angle=Some(0.4); s.direction.horizontal_max_angle=Some(0.8);
        s.direction.forward_offset=0.1; s.direction.learned_forward=Some([1.,0.,0.]);
        s.direction.learned_right=Some([0.,0.,1.]);
        let s=Settings::from_json(&s.to_json()).unwrap();
        let mut p=ActionProcessor::new(s).unwrap(); p.calibrate(&input(0.)).unwrap();
        let mut i=input(0.1); i.chest.as_mut().unwrap().value=[0.15f64.cos(),0.15f64.sin(),0.,0.];
        assert!((p.process(&i).travel_xy[1]-0.5).abs()<1e-8);
        i.chest.as_mut().unwrap().value=[0.1f64.cos(),0.,0.,0.1f64.sin()];
        let o=p.process(&i); assert!((o.travel_xy[0]-0.25).abs()<1e-8);
        assert!((o.travel_xy[1]+0.25).abs()<1e-8);
    }
    #[test]
    fn smoothing_releases_immediately_on_stale_and_reset() {
        let mut s=Settings::default(); s.direction.enabled=true; s.direction.smoothing_ms=80.;
        let mut p=ActionProcessor::new(s).unwrap(); p.calibrate(&input(0.)).unwrap();
        let mut i=input(0.08); i.chest.as_mut().unwrap().value=[0.2f64.cos(),0.2f64.sin(),0.,0.];
        let first=p.process(&i).travel_xy[1]; assert!(first>0. && first<1.);
        i.t=0.5; assert_eq!(p.process(&i).travel_xy,[0.;2]);
        i.t=0.51; i.chest.as_mut().unwrap().seen=i.t;
        assert!(p.process(&i).travel_xy[1] < first);
        p.reset_transient(); i.t=0.52; i.chest.as_mut().unwrap().seen=i.t;
        assert_eq!(p.process(&i).travel_xy,[0.;2]);
    }
    #[test]
    fn crouch_hysteresis_and_stale_release() {
        let mut s = Settings::default();
        s.crouch.enabled = true;
        let mut p = ActionProcessor::new(s).unwrap();
        p.calibrate(&input(0.)).unwrap();
        let mut i = input(1.);
        assert!(!p.process(&i).crouch_pressed);
        i.t = 1.01;
        i.hmd.as_mut().unwrap().seen = 1.01;
        i.hmd.as_mut().unwrap().height = 1.0;
        assert!(p.process(&i).crouch_pressed);
        i.t = 1.1;
        i.hmd.as_mut().unwrap().seen = 1.1;
        i.hmd.as_mut().unwrap().height = 1.25;
        assert!(p.process(&i).crouching);
        i.t = 1.4;
        assert!(!p.process(&i).crouch_pressed);
    }
    #[test]
    fn jump_pulse_expires() {
        let mut s = Settings::default();
        s.jump.enabled = true;
        let mut p = ActionProcessor::new(s).unwrap();
        p.calibrate(&input(0.)).unwrap();
        let mut i = input(1.);
        assert!(!p.process(&i).jump_pressed);
        i.t = 1.1;
        i.hmd.as_mut().unwrap().seen = 1.1;
        i.hmd.as_mut().unwrap().height = 1.9;
        assert!(p.process(&i).jump_pressed);
        i.t = 1.7;
        i.hmd.as_mut().unwrap().seen = 1.7;
        assert!(!p.process(&i).jump_pressed);
    }
    #[test]
    fn tilt_forward_only_limits_speed_and_stops_upright_backward_and_stale(){
        let mut s=Settings::default();s.direction.enabled=true;s.direction.forward_only=true;s.direction.max_output=0.35;
        for (angle,forward) in [(0.,false),(0.05,false),(0.4,true),(-0.4,false)] {
            let mut p=ActionProcessor::new(s.clone()).unwrap();p.calibrate(&input(0.)).unwrap();
            let mut i=input(1.);let a: f64=angle;i.chest.as_mut().unwrap().value=[(a/2.).cos(),(a/2.).sin(),0.,0.];
            let r=p.process(&i);assert_eq!(r.travel_xy[0],0.);assert!(r.travel_xy[1]>=0.&&r.travel_xy[1]<=0.35);
            assert_eq!(r.travel_xy[1]>0.,forward);
            i.t=2.;assert_eq!(p.process(&i).travel_xy,[0.;2]);
        }
    }
    #[test]
    fn learned_forward_handles_nonidentity_mount_and_neutral(){
        let mut s=Settings::default();s.direction.enabled=true;s.direction.forward_only=true;
        let mut p=ActionProcessor::new(s).unwrap();let mut i=input(0.);i.chest.as_mut().unwrap().value=[0.7071067811865476,0.,0.,0.7071067811865476];p.calibrate(&i).unwrap();
        i.t=0.1;i.chest.as_mut().unwrap().seen=0.1;let c=(0.15f64).cos()*0.7071067811865476;let a=(0.15f64).sin()*0.7071067811865476;
        i.chest.as_mut().unwrap().value=[c,-a,a,c];p.learn_forward(&i).unwrap();assert!(p.process(&i).travel_xy[1]>0.);
        i.chest.as_mut().unwrap().value=[0.7071067811865476,0.,0.,0.7071067811865476];assert_eq!(p.process(&i).travel_xy,[0.;2]);assert!(p.learn_forward(&i).is_err());
    }
    #[test]
    fn forward_sign_reverses_learned_motion_and_survives_center_reset(){
        let mut s=Settings::default();s.direction.enabled=true;s.direction.learned_forward=Some([1.,0.,0.]);s.direction.pitch_sign=-1.;
        let mut p=ActionProcessor::new(s).unwrap();p.calibrate(&input(0.)).unwrap();
        let mut i=input(0.1);i.chest.as_mut().unwrap().value=[0.15f64.cos(),0.15f64.sin(),0.,0.];
        assert!(p.process(&i).travel_xy[1]<0.);
        i.chest.as_mut().unwrap().value=[0.15f64.cos(),-0.15f64.sin(),0.,0.];
        assert!(p.process(&i).travel_xy[1]>0.);
        p.calibrate(&input(0.2)).unwrap();i.t=0.3;i.chest.as_mut().unwrap().seen=0.3;
        assert!(p.process(&i).travel_xy[1]>0.);
        p.learn_forward(&i).unwrap();assert!(p.process(&i).travel_xy[1]>0.);
    }
    #[test]
    fn lateral_sign_applies_to_learned_axis_and_retraining_cancels_old_inversion() {
        let mut s=Settings::default();s.direction.decouple=false;s.direction.enabled=true;
        s.direction.learned_forward=Some([1.,0.,0.]);
        s.direction.learned_right=Some([0.,-1.,0.]);
        s.direction.roll_sign=-1.;s.direction.pitch_sign=-1.;
        let mut p=ActionProcessor::new(s).unwrap();p.calibrate(&input(0.)).unwrap();
        let mut i=input(0.1);
        i.chest.as_mut().unwrap().value=[0.15f64.cos(),0.,0.15f64.sin(),0.];
        assert!(p.process(&i).travel_xy[0]>0.1);
        i.chest.as_mut().unwrap().value=[0.15f64.cos(),0.,-0.15f64.sin(),0.];
        assert!(p.process(&i).travel_xy[0]< -0.1);
        i.chest.as_mut().unwrap().value=[0.15f64.cos(),-0.15f64.sin(),0.,0.];
        assert!(p.process(&i).travel_xy[1]>0.1, "lateral inversion cannot change forward");
        p.calibrate(&input(0.2)).unwrap();i.t=0.3;i.chest.as_mut().unwrap().seen=0.3;
        i.chest.as_mut().unwrap().value=[0.15f64.cos(),0.,-0.15f64.sin(),0.];
        p.learn_right(&i).unwrap();
        assert!(p.process(&i).travel_xy[0]>0.1, "recorded right lean must mean right even with an old inversion");
        p.calibrate(&input(0.4)).unwrap();i.t=0.5;i.chest.as_mut().unwrap().seen=0.5;
        assert!(p.process(&i).travel_xy[0]>0.1);
    }
    #[test]
    fn retraining_ignores_old_mount_and_forward_only_has_no_fake_strafe() {
        fn mul(a:[f64;4],b:[f64;4])->[f64;4] {
            let [w,x,y,z]=a;let [v,i,j,k]=b;
            [w*v-x*i-y*j-z*k,w*i+x*v+y*k-z*j,w*j-x*k+y*v+z*i,w*k+x*j-y*i+z*v]
        }
        let mut fresh=Settings::default();fresh.direction.decouple=false;fresh.direction.enabled=true;
        let mut old=fresh.clone();old.direction.learned_forward=Some([0.,1.,0.]);old.direction.learned_right=Some([0.,0.,1.]);
        let mut a=ActionProcessor::new(fresh).unwrap();let mut b=ActionProcessor::new(old).unwrap();
        let mount=[0.4f64.cos(),0.,0.4f64.sin(),0.];let mut i=input(0.);
        i.chest.as_mut().unwrap().value=mount;a.calibrate(&i).unwrap();b.calibrate(&i).unwrap();
        let c=0.15f64.cos();let s=0.15f64.sin();
        let forward=mul(mount,[c,s*0.8,0.,s*0.6]);let right=mul(mount,[c,-s*0.6,0.,s*0.8]);
        i.t=0.1;i.chest.as_mut().unwrap().seen=0.1;i.chest.as_mut().unwrap().value=forward;
        a.learn_forward(&i).unwrap();b.learn_forward(&i).unwrap();
        for (x,y) in a.settings.direction.learned_forward.unwrap().iter().zip(b.settings.direction.learned_forward.unwrap()) {assert!((x-y).abs()<1e-9);}
        let v=b.process(&i).travel_xy;assert!(v[1]>0.);assert_eq!(v[0],0.);
        i.chest.as_mut().unwrap().value=right;a.learn_right(&i).unwrap();b.learn_right(&i).unwrap();
        for (x,y) in a.settings.direction.learned_right.unwrap().iter().zip(b.settings.direction.learned_right.unwrap()) {assert!((x-y).abs()<1e-9);}
        for (pose,positive) in [(forward,true),(mul(mount,[c,-s*0.8,0.,-s*0.6]),false)] {
            i.chest.as_mut().unwrap().value=pose.map(|v|-v);let v=b.process(&i).travel_xy;
            assert!(v[0].abs()<1e-9);assert!(if positive {v[1]>0.} else {v[1]<0.});
        }
        i.t=0.2;i.chest.as_mut().unwrap().seen=0.2;i.chest.as_mut().unwrap().value=mount.map(|v|-v);b.calibrate(&i).unwrap();
        i.t=0.3;i.chest.as_mut().unwrap().seen=0.3;i.chest.as_mut().unwrap().value=forward;
        let v=b.process(&i).travel_xy;assert!(v[0].abs()<1e-9 && v[1]>0.);
    }
    #[test]
    fn learned_strafe_left_right_and_diagonal_cap(){
        let mut s=Settings::default();s.direction.enabled=true;s.direction.forward_only=true;s.direction.max_output=0.35;
        let mut p=ActionProcessor::new(s).unwrap();p.calibrate(&input(0.)).unwrap();
        let mut i=input(0.1);i.chest.as_mut().unwrap().value=[(0.15f64).cos(),(0.15f64).sin(),0.,0.];p.learn_forward(&i).unwrap();
        assert!(p.learn_right(&i).is_err());
        i.chest.as_mut().unwrap().value=[(0.15f64).cos(),0.,0.,(0.15f64).sin()];p.learn_right(&i).unwrap();
        assert!(p.process(&i).travel_xy[0]>0.);
        i.chest.as_mut().unwrap().value=[(0.15f64).cos(),0.,0.,-(0.15f64).sin()];assert!(p.process(&i).travel_xy[0]<0.);
        i.chest.as_mut().unwrap().value=[1.,0.,0.,0.];assert_eq!(p.process(&i).travel_xy,[0.;2]);
        i.chest.as_mut().unwrap().value=[(0.6f64).cos(),(0.6f64).sin()/2f64.sqrt(),0.,(0.6f64).sin()/2f64.sqrt()];
        let v=p.process(&i).travel_xy;assert!(v[0]>0.&&v[1]>0.);assert!((v[0]*v[0]+v[1]*v[1]).sqrt()<=0.3500001);
    }
    #[test]
    fn strafe_disabled_suppresses_learned_right_without_suppressing_forward() {
        let mut s=Settings::default();s.direction.enabled=true;s.direction.strafe_enabled=false;
        s.direction.learned_forward=Some([1.,0.,0.]);s.direction.learned_right=Some([0.,0.,1.]);
        let mut p=ActionProcessor::new(s).unwrap();p.calibrate(&input(0.)).unwrap();
        let mut i=input(0.1);i.chest.as_mut().unwrap().value=[(0.3f64).cos(),(0.3f64).sin()/2f64.sqrt(),0.,(0.3f64).sin()/2f64.sqrt()];
        let v=p.process(&i).travel_xy;assert_eq!(v[0],0.);assert!(v[1]>0.);
        assert!(Settings::from_json(&json!({"direction":{"strafe_enabled":false}})).unwrap().direction.strafe_enabled==false);
        assert!(Settings::from_json(&json!({"direction":{}})).unwrap().direction.strafe_enabled);
    }
    #[test]
    fn training_preview_ignores_movement_restrictions_and_speed_cap() {
        let mut s=Settings::default();s.direction.enabled=true;s.direction.forward_only=true;
        s.direction.strafe_enabled=false;s.direction.max_output=0.05;
        s.direction.learned_forward=Some([1.,0.,0.]);s.direction.learned_right=Some([0.,0.,1.]);
        let mut p=ActionProcessor::new(s).unwrap();p.calibrate(&input(0.)).unwrap();
        let mut i=input(0.1);let c=0.15f64.cos();let v=0.15f64.sin();
        i.chest.as_mut().unwrap().value=[c,-v,0.,0.];let out=p.process(&i);
        assert!(out.training_xy[1]< -0.03);assert_eq!(out.travel_xy,[0.,0.]);
        i.chest.as_mut().unwrap().value=[c,0.,0.,v];let out=p.process(&i);
        assert!(out.training_xy[0]>0.03);assert_eq!(out.travel_xy,[0.,0.]);
        i.chest.as_mut().unwrap().value=[c,v,0.,0.];let out=p.process(&i);
        assert!(out.training_xy[1]>0.03);assert!(out.travel_xy[1]>0. && out.travel_xy[1]<=0.05);
        p.settings.direction.learned_right=None;i.chest.as_mut().unwrap().value=[c,0.,0.,v];
        assert_eq!(p.process(&i).training_xy[0],0.);
    }
    #[test]
    fn sprint_ring_threshold_and_stale_release() {
        let mut s=Settings::default();s.direction.enabled=true;s.direction.sprint_full_forward=true;
        s.direction.max_angle=0.4;s.direction.sprint_angle=Some(0.6);
        s.direction.learned_forward=Some([1.,0.,0.]);s.direction.learned_right=Some([0.,0.,1.]);
        let mut p=ActionProcessor::new(s).unwrap();p.calibrate(&input(0.)).unwrap();let mut i=input(0.1);
        for (angle,expected) in [(0.39,false),(0.59,false),(0.61,true),(0.59,false),(-0.61,true)] {
            i.chest.as_mut().unwrap().value=[(angle/2.0_f64).cos(),(angle/2.0_f64).sin(),0.,0.];
            assert_eq!(p.process(&i).sprint_pressed,expected);
        }
        i.t=1.;assert!(!p.process(&i).sprint_pressed);
        let saved=p.settings().to_json();assert_eq!(Settings::from_json(&saved).unwrap().direction.sprint_angle,Some(0.6));
        let mut invalid=saved;invalid["direction"]["sprint_angle"]=json!(0.3);assert!(Settings::from_json(&invalid).is_err());
    }
    #[test]
    fn speed_starts_at_deadzone_edge_and_reaches_full_ring() {
        for (angle,expected) in [(0.0,0.0),(0.1,0.0),(0.25,0.5),(0.4,1.0),(0.6,1.0)] {
            let output=ellipse_axes(0.,angle,0.1,0.1,0.4,0.4,1.);
            assert!((output.1-expected).abs()<1e-9);
        }
    }
    #[test]
    fn full_rotation_preserves_lean_and_backward_deadzone(){
        let mut s=Settings::default();s.direction.decouple=false;s.direction.enabled=true;s.direction.backward_deadzone_multiplier=1.1;
        s.direction.learned_forward=Some([1.,0.,0.]);s.direction.learned_right=Some([0.,0.,1.]);
        let mut p=ActionProcessor::new(s).unwrap();p.calibrate(&input(0.)).unwrap();
        let mut reference:Option<f64>=None;
        for degrees in [0.,45.,90.,135.,180.,225.,270.,315.,360.] {
            let yaw=(degrees as f64).to_radians()/2.;let lean=0.15f64;
            let mut i=input(0.1);i.chest.as_mut().unwrap().value=[yaw.cos()*lean.cos(),yaw.cos()*lean.sin(),yaw.sin()*lean.cos(),-yaw.sin()*lean.sin()];
            let v=p.process(&i).travel_xy;assert!(v[1]>0.);assert!(v[0].abs()<1e-8);
            if let Some(old)=reference{assert!((v[1]-old).abs()<1e-8);}else{reference=Some(v[1]);}
            i.chest.as_mut().unwrap().value=[yaw.cos(),0.,yaw.sin(),0.];assert_eq!(p.process(&i).travel_xy,[0.;2]);
        }
        let mut i=input(0.1);let a=0.105f64/2.;i.chest.as_mut().unwrap().value=[a.cos(),-a.sin(),0.,0.];assert_eq!(p.process(&i).travel_xy,[0.;2]);
        i.chest.as_mut().unwrap().value=[a.cos(),a.sin(),0.,0.];assert!(p.process(&i).travel_xy[1]>0.);
        let a=0.15f64;i.chest.as_mut().unwrap().value=[a.cos(),-a.sin(),0.,0.];assert!(p.process(&i).travel_xy[1]<0.);
    }
    #[test]
    fn direction_and_turn_polarity() {
        let mut s = Settings::default();
        s.direction.enabled = true;
        s.turn.enabled = true;
        let mut p = ActionProcessor::new(s).unwrap();
        p.calibrate(&input(0.)).unwrap();
        let mut i = input(1.);
        let a = 0.4f64;
        i.chest.as_mut().unwrap().value = [(a / 2.).cos(), (a / 2.).sin(), 0., 0.];
        i.hmd.as_mut().unwrap().yaw = 0.4;
        let r = p.process(&i);
        assert!(r.travel_xy[1] > 0. && r.turn_xy[0] > 0.);
        i.t = 1.3;
        let r = p.process(&i);
        assert_eq!(r.travel_xy, [0.; 2]);
        assert_eq!(r.turn_xy, [0.; 2]);
    }
    #[test]
    fn invalid_settings_and_time_release() {
        assert!(Settings::from_json(&json!({"jump":{"threshold":0.01,"release":0.5}})).is_err());
        let mut p = ActionProcessor::new(Settings::default()).unwrap();
        p.calibrate(&input(1.)).unwrap();
        assert!(!p.process(&input(0.)).calibrated);
    }
    #[test]
    fn crouch_pulse_on_entry_and_exit() {
        let mut s = Settings::default();
        s.crouch.enabled = true;
        s.crouch.mode = "pulse".into();
        let mut p = ActionProcessor::new(s).unwrap();
        p.calibrate(&input(0.)).unwrap();
        let mut i = input(1.);
        assert!(!p.process(&i).crouch_pressed);
        i.t = 1.01;
        i.hmd.as_mut().unwrap().seen = 1.01;
        i.hmd.as_mut().unwrap().height = 1.0;
        assert!(p.process(&i).crouch_pressed);
        i.t = 1.2;
        i.hmd.as_mut().unwrap().seen = 1.2;
        assert!(!p.process(&i).crouch_pressed);
        i.t = 1.3;
        i.hmd.as_mut().unwrap().seen = 1.3;
        i.hmd.as_mut().unwrap().height = 1.7;
        assert!(p.process(&i).crouch_pressed);
    }
    #[test]
    fn jump_selects_signed_acceleration_axis() {
        let mut s = Settings::default();
        s.jump.enabled = true;
        s.jump.source = "acceleration".into();
        s.jump.accel_axis = "-y".into();
        s.jump.threshold = 2.;
        s.jump.release = 1.;
        let mut p = ActionProcessor::new(s).unwrap();
        p.calibrate(&input(0.)).unwrap();
        let mut i = input(1.);
        i.chest_accel.as_mut().unwrap().value = [9., 2., 0.];
        assert!(!p.process(&i).jump_pressed);
        i.t = 1.1;
        i.chest_accel.as_mut().unwrap().seen = 1.1;
        i.chest_accel.as_mut().unwrap().value = [0., -3., 0.];
        assert!(p.process(&i).jump_pressed);
    }
    #[test]
    fn calibration_uses_only_required_sources() {
        let empty = Input {
            t: 1.,
            chest: None,
            chest_accel: None,
            hmd: None,
        };
        let mut p = ActionProcessor::new(Settings::default()).unwrap();
        assert!(p.calibrate(&empty).is_ok());
        assert!(p.process(&empty).calibrated);
        let mut s = Settings::default(); s.direction.decouple=false;
        s.direction.enabled = true;
        let mut p = ActionProcessor::new(s).unwrap();
        let mut i = input(1.);
        i.hmd = None;
        assert!(p.calibrate(&i).is_ok());
        i.t = 1.1;
        i.chest.as_mut().unwrap().seen = 1.1;
        assert_eq!(p.process(&i).direction_status, "ready");
        let mut s = Settings::default(); s.direction.decouple=false;
        s.jump.enabled = true;
        s.jump.source = "acceleration".into();
        let mut p = ActionProcessor::new(s).unwrap();
        i.chest = None;
        i.chest_accel = None;
        assert!(p.calibrate(&i).is_ok());
    }
    #[test]
    fn center_does_not_require_acceleration_but_jump_requires_fresh_neutral_and_peak() {
        let mut s=Settings::default();s.direction.decouple=false;s.direction.enabled=true;s.jump.enabled=true;
        s.jump.source="acceleration".into();s.jump.threshold=2.;s.jump.release=1.;
        let mut p=ActionProcessor::new(s).unwrap();let mut i=input(1.);
        i.chest_accel=None;assert!(p.calibrate(&i).is_ok());
        let out=p.process(&i);assert_eq!(out.jump_status,"missing chest acceleration");assert!(!out.jump_pressed);
        i.chest_accel=Some(TimedVector{value:[5.,0.,0.],seen:0.});
        let out=p.process(&i);assert_eq!(out.jump_status,"missing chest acceleration");assert!(!out.jump_pressed);
        // A fresh peak alone cannot arm a jump after stale input.
        i.t=1.1;i.chest.as_mut().unwrap().seen=i.t;i.chest_accel.as_mut().unwrap().seen=i.t;
        assert!(!p.process(&i).jump_pressed);
        i.t=1.2;i.chest.as_mut().unwrap().seen=i.t;i.chest_accel.as_mut().unwrap().seen=i.t;i.chest_accel.as_mut().unwrap().value=[0.;3];
        assert!(!p.process(&i).jump_pressed);
        i.t=1.3;i.chest.as_mut().unwrap().seen=i.t;i.chest_accel.as_mut().unwrap().seen=i.t;assert!(!p.process(&i).jump_pressed);
        i.t=1.4;i.chest.as_mut().unwrap().seen=i.t;i.chest_accel.as_mut().unwrap().seen=i.t;i.chest_accel.as_mut().unwrap().value=[5.,0.,0.];
        assert!(p.process(&i).jump_pressed);
        i.t=1.8;i.chest.as_mut().unwrap().seen=i.t;
        let out=p.process(&i);assert_eq!(out.jump_status,"missing chest acceleration");assert!(!out.jump_pressed);
        assert!(p.calibrate(&i).is_ok());
    }
    #[test]
    fn center_allows_missing_hmd_for_uncompensated_height_jump_but_jump_needs_fresh_poses() {
        let mut s=Settings::default();s.direction.decouple=false;s.direction.enabled=true;s.jump.enabled=true;
        s.jump.source="height".into();s.jump.threshold=0.1;s.jump.release=0.04;
        let mut p=ActionProcessor::new(s.clone()).unwrap();let mut i=input(1.);i.hmd=None;
        assert!(p.calibrate(&i).is_ok());let out=p.process(&i);
        assert_eq!(out.jump_status,"missing HMD");assert!(!out.jump_pressed);
        i.hmd=Some(TimedHmd{height:1.8,yaw:0.,pitch:0.,roll:0.,seen:0.});
        let out=p.process(&i);assert_eq!(out.jump_status,"missing HMD");assert!(!out.jump_pressed);
        i.t=1.1;i.chest.as_mut().unwrap().seen=i.t;i.hmd.as_mut().unwrap().seen=i.t;i.hmd.as_mut().unwrap().height=1.6;
        assert!(!p.process(&i).jump_pressed);
        i.t=1.2;i.chest.as_mut().unwrap().seen=i.t;i.hmd.as_mut().unwrap().seen=i.t;i.hmd.as_mut().unwrap().height=1.8;
        assert!(p.process(&i).jump_pressed);
        i.t=1.5;i.chest.as_mut().unwrap().seen=i.t;let out=p.process(&i);
        assert_eq!(out.jump_status,"missing HMD");assert!(!out.jump_pressed);
        s.jump.angle_compensation_percent=1.;let mut p=ActionProcessor::new(s).unwrap();i.hmd=None;
        assert_eq!(p.calibrate(&i),Err("fresh HMD required"));
    }
    #[test]
    fn negative_acceleration_suppresses_jump_and_stale_releases() {
        let mut s = Settings::default();
        s.jump.enabled = true;
        s.jump.source = "acceleration".into();
        s.jump.accel_axis = "y".into();
        s.jump.threshold = 2.;
        s.jump.release = 1.;
        s.jump.suppress_threshold = 2.;
        let mut p = ActionProcessor::new(s).unwrap();
        p.calibrate(&input(0.)).unwrap();
        let mut i = input(1.);
        i.chest_accel.as_mut().unwrap().value = [0., -3., 0.];
        assert!(!p.process(&i).jump_pressed);
        i.t = 1.1;
        i.chest_accel.as_mut().unwrap().seen = 1.1;
        i.chest_accel.as_mut().unwrap().value = [0., 5., 0.];
        assert!(!p.process(&i).jump_pressed);
        i.t = 1.4;
        assert!(!p.process(&i).jump_pressed);
        assert_eq!(p.process(&i).jump_status, "missing chest acceleration");
    }
    #[test]
    fn crouch_absolute_hysteresis_and_height_correction() {
        let mut s = Settings::default();
        s.crouch.enabled = true;
        s.crouch.height_correction = 0.1;
        let mut p = ActionProcessor::new(s).unwrap();
        p.calibrate(&input(0.)).unwrap();
        let mut i = input(1.);
        i.hmd.as_mut().unwrap().height = 0.99;
        assert!(p.process(&i).crouching);
        i.t = 1.1;
        i.hmd.as_mut().unwrap().seen = 1.1;
        i.hmd.as_mut().unwrap().height = 1.25;
        assert!(p.process(&i).crouching);
        i.t = 1.2;
        i.hmd.as_mut().unwrap().seen = 1.2;
        i.hmd.as_mut().unwrap().height = 1.31;
        assert!(!p.process(&i).crouching);
    }
    #[test]
    fn zero_decouple_strength_never_reverses_direction() {
        let mut s = Settings::default();
        s.direction.enabled = true;
        s.direction.decouple = true;
        s.direction.decouple_strength = 0.;
        let mut p = ActionProcessor::new(s).unwrap();
        p.calibrate(&input(0.)).unwrap();
        let mut i = input(1.);
        let a = 0.4f64;
        i.chest.as_mut().unwrap().value = [(a / 2.).cos(), (a / 2.).sin(), 0., 0.];
        i.hmd.as_mut().unwrap().yaw = 2.5;
        let r = p.process(&i);
        assert!(r.travel_xy[1] > 0.);
        assert!(r.travel_xy[0].abs() < 1e-12);
    }
    #[test]
    fn transient_reset_requires_neutral_and_drops_old_pulses() {
        let mut s = Settings::default();
        s.jump.enabled = true;
        s.jump.source = "acceleration".into();
        s.jump.accel_axis = "y".into();
        s.jump.threshold = 2.;
        s.jump.release = 1.;
        let mut p = ActionProcessor::new(s).unwrap();
        let mut i = input(0.);
        i.chest_accel = Some(TimedVector {
            value: [0., 0., 0.],
            seen: 0.,
        });
        p.calibrate(&i).unwrap();
        p.process(&i);
        i.t = 0.1;
        i.chest_accel = Some(TimedVector {
            value: [0., 10., 0.],
            seen: 0.1,
        });
        assert!(p.process(&i).jump_pressed);
        p.reset_transient();
        i.t = 0.11;
        assert!(!p.process(&i).jump_pressed);
        i.t = 0.12;
        i.chest_accel = Some(TimedVector {
            value: [0., 10., 0.],
            seen: 0.12,
        });
        assert!(!p.process(&i).jump_pressed);
        p.reset_transient();
        i.t = 0.13;
        i.chest_accel = Some(TimedVector {
            value: [0., 0., 0.],
            seen: 0.13,
        });
        p.process(&i);
        i.t = 0.3;
        i.chest_accel = Some(TimedVector {
            value: [0., 10., 0.],
            seen: 0.3,
        });
        assert!(p.process(&i).jump_pressed);
    }
    #[test]
    fn crouch_pulse_initializes_current_posture_without_event() {
        let mut s = Settings::default();
        s.crouch.enabled = true;
        s.crouch.mode = "pulse".into();
        let mut p = ActionProcessor::new(s).unwrap();
        let mut i = input(0.);
        p.calibrate(&i).unwrap();
        i.hmd.as_mut().unwrap().height = 1.;
        let out = p.process(&i);
        assert!(out.crouching);
        assert!(!out.crouch_pressed);
        p.reset_transient();
        assert!(!p.process(&i).crouch_pressed);
    }
}
