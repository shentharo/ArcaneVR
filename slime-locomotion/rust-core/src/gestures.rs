//! Original threshold-based gesture mappings. Preview only; caller gates output.
use crate::actions::{TimedHmd, TimedQuaternion, TimedVector};
use serde_json::{json, Value};
#[derive(Clone, Debug)]
pub struct Mapping {
    pub source: String,
    pub channel: String,
    pub op: String,
    pub trigger: f64,
    pub release: f64,
    pub dwell_seconds: f64,
    pub mode: String,
    pub pulse_seconds: f64,
    pub action: String,
}
#[derive(Clone, Debug, Default)]
pub struct Settings {
    pub enabled: bool,
    pub mappings: Vec<Mapping>,
}
impl Settings {
    pub fn from_json(v: &Value) -> Result<Self, &'static str> {
        let o = v.as_object().ok_or("gesture settings must be object")?;
        if o.keys().any(|k| k != "enabled" && k != "mappings") {
            return Err("unknown gesture setting");
        }
        let enabled = match o.get("enabled") {
            None => false,
            Some(v) => v.as_bool().ok_or("invalid enabled")?,
        };
        let a = match o.get("mappings") {
            None => &[][..],
            Some(v) => v.as_array().ok_or("invalid mappings")?.as_slice(),
        };
        if a.len() > 32 {
            return Err("too many mappings");
        }
        let mut mappings = Vec::new();
        for x in a {
            let obj = x.as_object().ok_or("invalid mapping")?;
            const F: &[&str] = &[
                "source",
                "channel",
                "op",
                "trigger",
                "release",
                "dwell_seconds",
                "mode",
                "pulse_seconds",
                "action",
            ];
            if obj.keys().any(|k| !F.contains(&k.as_str())) {
                return Err("unknown mapping field");
            }
            let text = |k: &str| {
                x[k].as_str()
                    .map(str::to_owned)
                    .ok_or("missing mapping text")
            };
            let number = |k: &str| x[k].as_f64().ok_or("missing mapping number");
            mappings.push(Mapping {
                source: text("source")?,
                channel: text("channel")?,
                op: text("op")?,
                trigger: number("trigger")?,
                release: number("release")?,
                dwell_seconds: number("dwell_seconds")?,
                mode: text("mode")?,
                pulse_seconds: number("pulse_seconds")?,
                action: text("action")?,
            });
        }
        let s = Self { enabled, mappings };
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.mappings.len() > 32 {
            return Err("too many mappings");
        }
        for m in &self.mappings {
            if !["chest", "left", "right", "hmd"].contains(&m.source.as_str())
                || ![
                    "yaw",
                    "pitch",
                    "roll",
                    "height",
                    "accel_x",
                    "accel_y",
                    "accel_z",
                    "accel_magnitude",
                ]
                .contains(&m.channel.as_str())
                || (m.source == "hmd" && m.channel.starts_with("accel"))
                || (m.source != "hmd" && m.channel == "height")
                || !["above", "below"].contains(&m.op.as_str())
                || !["hold", "tap"].contains(&m.mode.as_str())
                || !["jump", "crouch", "sprint"].contains(&m.action.as_str())
                || !m.trigger.is_finite()
                || !m.release.is_finite()
                || m.trigger.abs() > 200.
                || m.release.abs() > 200.
                || (m.op == "above" && m.release >= m.trigger)
                || (m.op == "below" && m.release <= m.trigger)
                || !m.dwell_seconds.is_finite()
                || !(0.0..=5.).contains(&m.dwell_seconds)
                || !m.pulse_seconds.is_finite()
                || !(0.02..=2.).contains(&m.pulse_seconds)
            {
                return Err("invalid gesture mapping");
            }
        }
        Ok(())
    }
    pub fn to_json(&self) -> Value {
        json!({"enabled":self.enabled,"mappings":self.mappings.iter().map(|m|json!({"source":m.source,"channel":m.channel,"op":m.op,"trigger":m.trigger,"release":m.release,"dwell_seconds":m.dwell_seconds,"mode":m.mode,"pulse_seconds":m.pulse_seconds,"action":m.action})).collect::<Vec<_>>()})
    }
}
#[derive(Clone, Copy, Default, Debug)]
pub struct Source {
    pub rotation: Option<TimedQuaternion>,
    pub acceleration: Option<TimedVector>,
}
#[derive(Clone, Copy, Debug)]
pub struct Input {
    pub t: f64,
    pub chest: Source,
    pub left: Source,
    pub right: Source,
    pub hmd: Option<TimedHmd>,
}
#[derive(Clone, Debug, Default)]
pub struct Telemetry {
    pub jump: bool,
    pub crouch: bool,
    pub sprint: bool,
    pub statuses: Vec<&'static str>,
}
#[derive(Clone, Default)]
struct State {
    neutral_seen: bool,
    active: bool,
    dwell_since: Option<f64>,
    pulse_until: Option<f64>,
}
pub struct GestureProcessor {
    settings: Settings,
    states: Vec<State>,
    last_time: Option<f64>,
}
fn fresh(t: f64, seen: f64) -> bool {
    t.is_finite() && seen.is_finite() && t >= seen && t - seen < 0.25
}
fn read(m: &Mapping, i: &Input) -> Option<f64> {
    let source = match m.source.as_str() {
        "chest" => i.chest,
        "left" => i.left,
        "right" => i.right,
        _ => Source::default(),
    };
    if m.source == "hmd" {
        let h = i.hmd?;
        if !fresh(i.t, h.seen) {
            return None;
        }
        let x = match m.channel.as_str() {
            "yaw" => h.yaw,
            "pitch" => h.pitch,
            "roll" => h.roll,
            "height" => h.height,
            _ => return None,
        };
        return (x.is_finite() && x.abs() <= 200.).then_some(x);
    }
    if m.channel.starts_with("accel") {
        let a = source.acceleration?;
        if !fresh(i.t, a.seen) || a.value.iter().any(|x| !x.is_finite() || x.abs() > 200.) {
            return None;
        }
        return Some(match m.channel.as_str() {
            "accel_x" => a.value[0],
            "accel_y" => a.value[1],
            "accel_z" => a.value[2],
            _ => a.value.iter().map(|x| x * x).sum::<f64>().sqrt(),
        });
    }
    let q = source.rotation?;
    if !fresh(i.t, q.seen) || q.value.iter().any(|x| !x.is_finite()) {
        return None;
    }
    let n = q.value.iter().map(|x| x * x).sum::<f64>().sqrt();
    if !(0.5..=1.5).contains(&n) {
        return None;
    }
    let [w, x, y, z] = q.value.map(|v| v / n);
    Some(match m.channel.as_str() {
        "yaw" => (2. * (w * y + x * z)).atan2(1. - 2. * (x * x + y * y)),
        "pitch" => (2. * (w * x - y * z)).clamp(-1., 1.).asin(),
        "roll" => (2. * (w * z + x * y)).atan2(1. - 2. * (x * x + z * z)),
        _ => return None,
    })
}
impl GestureProcessor {
    pub fn new(settings: Settings) -> Result<Self, &'static str> {
        settings.validate()?;
        let states = vec![State::default(); settings.mappings.len()];
        Ok(Self {
            settings,
            states,
            last_time: None,
        })
    }
    pub fn settings(&self) -> &Settings {
        &self.settings
    }
    pub fn reset(&mut self) {
        self.states = vec![State::default(); self.settings.mappings.len()];
        self.last_time = None;
    }
    pub fn set_settings(&mut self, settings: Settings) -> Result<(), &'static str> {
        settings.validate()?;
        self.settings = settings;
        self.reset();
        Ok(())
    }
    pub fn process(&mut self, i: &Input) -> Telemetry {
        let mut out = Telemetry::default();
        if !i.t.is_finite() || self.last_time.is_some_and(|t| i.t < t) {
            self.reset();
            return out;
        }
        self.last_time = Some(i.t);
        if !self.settings.enabled {
            self.states.fill(State::default());
            return out;
        }
        for (m, state) in self.settings.mappings.iter().zip(&mut self.states) {
            let Some(value) = read(m, i) else {
                *state = State::default();
                out.statuses.push("source stale");
                continue;
            };
            let neutral = if m.op == "above" {
                value <= m.release
            } else {
                value >= m.release
            };
            let triggered = if m.op == "above" {
                value >= m.trigger
            } else {
                value <= m.trigger
            };
            if neutral {
                state.neutral_seen = true;
                state.active = false;
                state.dwell_since = None;
            }
            if !state.neutral_seen {
                out.statuses.push("waiting for neutral");
                continue;
            }
            if triggered && !state.active {
                let since = *state.dwell_since.get_or_insert(i.t);
                if i.t - since >= m.dwell_seconds {
                    state.active = true;
                    state.dwell_since = None;
                    if m.mode == "tap" {
                        state.pulse_until = Some(i.t + m.pulse_seconds);
                    }
                }
            } else if !triggered && !state.active {
                state.dwell_since = None;
            }
            let on = if m.mode == "hold" {
                state.active
            } else {
                state.pulse_until.is_some_and(|until| i.t < until)
            };
            if on {
                match m.action.as_str() {
                    "jump" => out.jump = true,
                    "crouch" => out.crouch = true,
                    "sprint" => out.sprint = true,
                    _ => {}
                }
            }
            out.statuses.push(if on {
                "active"
            } else if state.active {
                "latched"
            } else {
                "ready"
            });
        }
        out
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn mapping() -> Value {
        json!({"enabled":true,"mappings":[{"source":"left","channel":"accel_x","op":"above","trigger":2.,"release":1.,"dwell_seconds":0.1,"mode":"hold","pulse_seconds":0.15,"action":"jump"}]})
    }
    fn input(t: f64, x: f64) -> Input {
        Input {
            t,
            left: Source {
                rotation: None,
                acceleration: Some(TimedVector {
                    value: [x, 0., 0.],
                    seen: t,
                }),
            },
            right: Source::default(),
            chest: Source::default(),
            hmd: None,
        }
    }
    #[test]
    fn neutral_dwell_hysteresis_and_stale() {
        let mut p = GestureProcessor::new(Settings::from_json(&mapping()).unwrap()).unwrap();
        assert!(!p.process(&input(0., 3.)).jump);
        assert!(!p.process(&input(0.1, 0.)).jump);
        assert!(!p.process(&input(0.2, 3.)).jump);
        assert!(p.process(&input(0.31, 3.)).jump);
        assert!(p.process(&input(0.32, 1.5)).jump);
        let mut stale = input(0.33, 3.);
        stale.t = 0.6;
        assert!(!p.process(&stale).jump);
        assert!(!p.process(&input(0.61, 3.)).jump);
    }
    #[test]
    fn tap_expires_and_does_not_repeat_without_neutral() {
        let mut v = mapping();
        v["mappings"][0]["mode"] = json!("tap");
        v["mappings"][0]["dwell_seconds"] = json!(0.);
        let mut p = GestureProcessor::new(Settings::from_json(&v).unwrap()).unwrap();
        p.process(&input(0., 0.));
        assert!(p.process(&input(0.1, 3.)).jump);
        assert!(!p.process(&input(0.3, 3.)).jump);
        assert!(!p.process(&input(0.4, 3.)).jump);
    }
    #[test]
    fn rejects_invalid_settings() {
        let mut v = mapping();
        v["mappings"][0]["release"] = json!(3.);
        assert!(Settings::from_json(&v).is_err());
        assert!(!Settings::default().enabled);
    }
}
