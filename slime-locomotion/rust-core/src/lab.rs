use crate::{
    encode_motion,
    gait::{Detection, Engine, Q},
    slime,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    io::{BufWriter, Write},
    net::SocketAddr,
    path::PathBuf,
};
pub const ROLES: [&str; 3] = ["left", "right", "torso"];
mod calibration_session;
mod jump_learning;
mod walking_learning;
mod crouch_learning;
#[derive(Clone)]
pub struct Record {
    pub q: Q,
    pub seen: f64,
    pub angular: f64,
    pub has_interval: bool,
}
#[derive(Clone)]
struct Acceleration {
    vector: [f64; 3],
    seen: f64,
    count: u64,
}
pub struct Lab {
    pub gestures: crate::gestures::GestureProcessor,
    pub gesture_preview: crate::gestures::Telemetry,
    pub actions: crate::actions::ActionProcessor,
    pub action_preview: crate::actions::Telemetry,
    pub movement_engine: String,
    pub sensor_count:u8,
    pub thighs:[String;2],
    pub hmd: Option<crate::actions::TimedHmd>,
    pub protocol_v2: bool,
    pub v2_sequence: Option<u32>,
    pub travel_xy: [f64; 2],
    pub turn_xy: [f64; 2],
    pub output_buttons: u32,
    pub profile_list: Vec<Value>,
    pub game_profiles: Value,
    pub moving_seconds: f64,
    pub distance_m: f64,
    pub peak_speed: f64,
    pub statistics_history: crate::statistics::History,
    pub stats_tick: f64,
    pub engine: Engine,
    pub mapping: [String; 3],
    pub records: HashMap<String, Record>,
    pub peers: HashMap<SocketAddr, f64>,
    tracker_identities: HashMap<String, String>,
    peer_macs: HashMap<String, String>,
    learned_mounting_identity: Option<String>,
    device_mountings: HashMap<String, Value>,
    sensor_calibration: Option<calibration_session::Session>,
    calibration_result: Value,
    walking_learning: Option<walking_learning::Session>,
    walking_result: Value,
    walking_model: Value,
    walking_signal: walking_learning::Signal,
    walking_speed:f64,
    walking_effort:f64,
    jump_learning: Option<jump_learning::Session>,
    jump_learning_result: Value,
    crouch_learning: Option<crouch_learning::Session>,
    crouch_learning_result: Value,
    auto_add: bool,
    sensor_status: HashMap<String, u8>,
    accelerations: HashMap<String, Acceleration>,
    counters: HashMap<SocketAddr, u64>,
    pub demo: bool,
    pub walking: bool,
    pub demo_yaw: f64,
    pub received: u64,
    pub rejected: u64,
    pub error: String,
    pub stop_reason: String,
    pub stop_detail: Value,
    pub seq: u64,
    pub packet: [u8; 261],
    pub config: PathBuf,
    recording: Option<BufWriter<fs::File>>,
    record_path: Option<PathBuf>,
    pub heading: f64,
    pub output_mode: String,
    pub generation: u32,
    pub steam_port: u16,
    pub steam_sequence: u32,
    pub steam_packets_sent: u64,
    pub steam_error: String,
    pub steam_speed: f32,
    pub steam_yaw: f32,
    pub neutral_pending: bool,
    pub ack_seen: f64,
    pub ack_flags: u32,
    pub ack_sequence: Option<u32>,
    pub was_driver_ready: bool,
    steam_retry_since: Option<f64>,
}
impl Lab {
    pub fn flush_statistics(&mut self) {
        self.statistics_history.persist_now(
            &self
                .config
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join("stats.json"),
        );
    }
    pub fn new(config: PathBuf) -> Result<Self, String> {
        let v: Value = if config.exists() {
            serde_json::from_str(&fs::read_to_string(&config).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
        } else {
            json!({})
        };
        let d = &v["detection"];
        let mut cfg = Detection::default();
        let movement_engine = v["movement_engine"]
            .as_str()
            .unwrap_or("chest_heading_steps")
            .to_string();
        if !["chest_heading_steps", "body_tilt_steps", "body_tilt"]
            .contains(&movement_engine.as_str())
        {
            return Err("Unknown movement engine".into());
        }
        cfg.chest_steps = v["step_input"] == "chest" && movement_engine == "body_tilt_steps";
        cfg.require_steps = movement_engine != "body_tilt" && !cfg.chest_steps;
        let mut actions = if let Some(a) = v.get("actions") {
            crate::actions::Settings::from_json(a).map_err(|e| e.to_string())?
        } else {
            let mut defaults=crate::actions::Settings::default();
            let angle=defaults.direction.max_angle*0.05;
            defaults.direction.forward_deadzone=Some(angle);defaults.direction.backward_deadzone=Some(angle);
            defaults.direction.left_deadzone=Some(angle);defaults.direction.right_deadzone=Some(angle);
            defaults
        };
        let mut device_mountings:HashMap<String,Value>=v["mounting_calibration"]["devices"].as_object().map(|o|o.iter().map(|(k,v)|(k.clone(),v.clone())).collect()).unwrap_or_default();
        if let Some(id)=v["mounting_calibration"]["learned_identity"].as_str() {device_mountings.entry(id.to_owned()).or_insert_with(|| actions.to_json()["direction"].clone());}
        if let Some(record)=v["mounting_calibration"]["learned_identity"].as_str().and_then(|id|device_mountings.get(id)) {
            let mut canonical=actions.to_json();
            for key in ["learned_forward","learned_right","pitch_axis","roll_axis","pitch_sign","roll_sign"] {
                if let Some(value)=record.get(key) {canonical["direction"][key]=value.clone();}
            }
            actions=crate::actions::Settings::from_json(&canonical).map_err(str::to_owned)?;
        }
        let gestures = if let Some(g) = v.get("gestures") {
            crate::gestures::Settings::from_json(g).map_err(str::to_owned)?
        } else {
            crate::gestures::Settings::default()
        };
        cfg.continuous = match v["locomotion"]["algorithm"].as_str() {
            None | Some("continuous") => true,
            Some("alternating") => false,
            _ => return Err("Unknown locomotion algorithm".into()),
        };
        if let Some(settings) = v["locomotion"].get("settings") {
            cfg.motion =
                crate::continuous::Settings::from_json(settings).map_err(|e| e.to_string())?;
        }
        for (key, target) in [
            ("threshold", &mut cfg.threshold),
            ("release", &mut cfg.release),
            ("stale", &mut cfg.stale),
            ("stop_after", &mut cfg.stop_after),
            ("heading_sign", &mut cfg.sign),
        ] {
            if let Some(n) = d.get(key) {
                *target = n.as_f64().ok_or("Invalid numeric setting")?;
            }
        }
        cfg.accel_has_gravity = match d.get("acceleration_mode") {
            None => false,
            Some(v) if v.as_str() == Some("linear") => false,
            Some(v) if v.as_str() == Some("gravity-baseline") => true,
            _ => return Err("Invalid acceleration mode".into()),
        };
        cfg.y_up = match d.get("heading_axis").and_then(Value::as_str) {
            None | Some("z") => false,
            Some("y") => true,
            _ => return Err("heading_axis must be y or z".into()),
        };
        cfg.validate()?;
        let output_mode = match v["output_mode"].as_str() {
            None | Some("meta") => "meta",
            Some("steamvr") => "steamvr",
            _ => return Err("Invalid output_mode".into()),
        }
        .to_string();
        let mapping = ROLES.map(|r| v["trackers"][r].as_str().unwrap_or("").to_string());
        Ok(Self {
            gestures: crate::gestures::GestureProcessor::new(gestures).map_err(str::to_owned)?,
            gesture_preview: Default::default(),
            actions: crate::actions::ActionProcessor::new(actions).map_err(|e| e.to_string())?,
            action_preview: Default::default(),
            sensor_count:if movement_engine=="body_tilt" || cfg.chest_steps {1}else{v["sensor_layout"]["count"].as_u64().filter(|n|*n==3||*n==5).unwrap_or(3) as u8},
            thighs:[v["sensor_layout"]["thighs"][0].as_str().unwrap_or("").into(),v["sensor_layout"]["thighs"][1].as_str().unwrap_or("").into()],
            movement_engine,
            hmd: None,
            protocol_v2: false,
            v2_sequence: None,
            travel_xy: [0.; 2],
            turn_xy: [0.; 2],
            output_buttons: 0,
            game_profiles: v.get("game_profiles").filter(|v| v.is_object()).cloned().unwrap_or_else(||json!({"selected":"", "associations":{}})),
            profile_list: crate::profiles::ProfileStore::new(
                config
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .to_path_buf(),
            )
            .list()
            .unwrap_or_default(),
            moving_seconds: 0.,
            distance_m: 0.,
            peak_speed: 0.,
            statistics_history: crate::statistics::History::load(
                &config
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .join("stats.json"),
            ),
            stats_tick: 0.,
            engine: Engine::new(cfg),
            mapping,
            records: HashMap::new(),
            peers: HashMap::new(),
            tracker_identities: v["tracker_identities"].as_object().map(|o| o.iter().filter_map(|(id, mac)| mac.as_str().map(|m|(id.clone(),m.to_string()))).collect()).unwrap_or_default(),
            peer_macs: HashMap::new(),
            learned_mounting_identity: v["mounting_calibration"]["learned_identity"].as_str().map(str::to_owned),
            device_mountings,
            sensor_calibration: None,
            calibration_result: Value::Null,
            walking_learning:None,walking_result:Value::Null,walking_model:v["walking_model"].clone(),walking_signal:Default::default(),walking_speed:0.,walking_effort:0.,
            jump_learning: None,
            jump_learning_result: Value::Null,
            crouch_learning: None,
            crouch_learning_result: Value::Null,
            auto_add: v["auto_add"].as_bool().unwrap_or(true),
            sensor_status: HashMap::new(),
            accelerations: HashMap::new(),
            counters: HashMap::new(),
            demo: false,
            walking: false,
            demo_yaw: 0.,
            received: 0,
            rejected: 0,
            error: String::new(),
            stop_reason: String::new(),
            stop_detail: Value::Null,
            seq: 0,
            packet: [0; 261],
            config,
            recording: None,
            record_path: None,
            heading: 0.,
            output_mode,
            generation: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u32,
            steam_port: 29741,
            steam_sequence: 0,
            steam_packets_sent: 0,
            steam_error: String::new(),
            steam_speed: 0.,
            steam_yaw: 0.,
            neutral_pending: true,
            ack_seen: -1e9,
            ack_flags: 0,
            ack_sequence: None,
            was_driver_ready: false,
            steam_retry_since: None,
        })
    }
    fn reset(&mut self) {
        self.engine = Engine::new(self.engine.cfg.clone());
        self.actions.reset();
        self.gestures.reset();
        self.gesture_preview = Default::default();
        self.action_preview = Default::default();
        self.heading = 0.;
        self.new_generation();
    }
    fn new_generation(&mut self) {
        self.gestures.reset();
        self.gesture_preview = Default::default();
        self.travel_xy = [0.; 2];
        self.turn_xy = [0.; 2];
        self.output_buttons = 0;
        self.generation = self.generation.wrapping_add(1);
        self.neutral_pending = true;
        self.ack_seen = -1e9;
        self.ack_flags = 0;
        self.ack_sequence = None;
        self.v2_sequence = None;
        self.was_driver_ready = false;
        self.steam_retry_since = None;
    }
    fn save(
        &self,
        mapping: &[String; 3],
        cfg: &Detection,
        output_mode: &str,
    ) -> Result<(), String> {
        self.save_options(
            mapping,
            cfg,
            output_mode,
            self.actions.settings(),
            self.gestures.settings(),
            &self.movement_engine,
        )
    }
    fn save_options(
        &self,
        mapping: &[String; 3],
        cfg: &Detection,
        output_mode: &str,
        actions: &crate::actions::Settings,
        gestures: &crate::gestures::Settings,
        movement_engine: &str,
    ) -> Result<(), String> {
        self.save_options_count(mapping,cfg,output_mode,actions,gestures,movement_engine,if movement_engine=="body_tilt" || cfg.chest_steps {1}else{self.sensor_count.max(3)})
    }
    fn save_options_count(&self,mapping:&[String;3],cfg:&Detection,output_mode:&str,actions:&crate::actions::Settings,gestures:&crate::gestures::Settings,movement_engine:&str,sensor_count:u8)->Result<(),String> {
        let v = json!({"walking_model":self.walking_model,"auto_add":self.auto_add,"tracker_identities":self.tracker_identities,"mounting_calibration":{"learned_identity":self.learned_mounting_identity,"devices":self.device_mountings},"sensor_layout":{"count":sensor_count,"thighs":self.thighs},"actions":actions.to_json(),"gestures":gestures.to_json(),"movement_engine":movement_engine,"step_input":if cfg.chest_steps {"chest"} else if movement_engine=="body_tilt" {"off"} else {"feet"},"locomotion":{"algorithm":if cfg.continuous {"continuous"} else {"alternating"},"settings":cfg.motion.to_json()},"output_mode":output_mode,"trackers":{"left":mapping[0],"right":mapping[1],"torso":mapping[2]},"detection":{"acceleration_mode":if cfg.accel_has_gravity {"gravity-baseline"} else {"linear"},"threshold":cfg.threshold,"release":cfg.release,"stale":cfg.stale,"stop_after":cfg.stop_after,"heading_axis":if cfg.y_up{"y"}else{"z"},"heading_sign":cfg.sign}});
        if let Some(parent) = self.config.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut v = v;
        v["game_profiles"] = self.game_profiles.clone();
        let tmp = self.config.with_extension("tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(&v).unwrap()).map_err(|e| e.to_string())?;
        fs::rename(tmp, &self.config).map_err(|e| e.to_string())?;
        Ok(())
    }
    fn layout_count(&self)->u8 {if self.movement_engine=="body_tilt" || self.engine.cfg.chest_steps {1}else{self.sensor_count.max(3)}}
    fn identity_matches(&self, id: &str) -> bool {
        self.tracker_identities.get(id).is_none_or(|expected| id.rsplit_once(':').and_then(|(ip,_)|self.peer_macs.get(ip)) == Some(expected))
    }
    /// Do not keep a pre-restart session alive when its saved identity is unverified.
    /// Silence lets the tracker reconnect and send its normal discovery handshake.
    pub fn needs_identity_handshake(&self, ip: std::net::IpAddr) -> bool {
        let ip = ip.to_string();
        !self.peer_macs.contains_key(&ip) && self.mapping.iter().chain(self.thighs.iter()).any(|id| {
            id.rsplit_once(':').is_some_and(|(host,_)| host == ip) && self.tracker_identities.contains_key(id)
        })
    }
    // Identity comes from the current hello, never just a saved IP assignment.
    // The sensor index distinguishes multiple IMUs on the same device.
    fn current_mounting_identity(&self, t: f64) -> Option<String> {
        let id = &self.mapping[2];
        let (ip, index) = id.rsplit_once(':')?;
        let index: u8 = index.parse().ok()?;
        let rotation = self.validated_rotation(2, t)?;
        if !t.is_finite() || t < rotation.seen || t - rotation.seen >= self.engine.cfg.stale { return None; }
        let mac = self.peer_macs.get(ip)?;
        Some(format!("{}:{}", mac.to_ascii_uppercase(), index))
    }
    fn remember_device_mounting(&mut self, identity:&str) {
        let d=self.actions.settings().to_json()["direction"].clone();
        let face=self.device_mountings.get(identity).and_then(|v|v["face_turned_over"].as_bool()).unwrap_or(false);
        self.device_mountings.insert(identity.to_owned(),json!({"face_turned_over":face,"method":"mounting-frame-v1","learned_forward":d["learned_forward"],"learned_right":d["learned_right"],"pitch_axis":d["pitch_axis"],"roll_axis":d["roll_axis"],"pitch_sign":d["pitch_sign"],"roll_sign":d["roll_sign"]}));
    }
    fn restore_device_mounting(&mut self, identity:&str) -> Result<bool,String> {
        let Some(saved)=self.device_mountings.get(identity) else {return Ok(false)};
        let mut settings=self.actions.settings().to_json();
        for key in ["learned_forward","learned_right","pitch_axis","roll_axis","pitch_sign","roll_sign"] {if !saved[key].is_null() || key=="learned_forward" || key=="learned_right" {if saved.get(key).is_some(){settings["direction"][key]=saved[key].clone();}}}
        let parsed=crate::actions::Settings::from_json(&settings).map_err(str::to_owned)?;
        self.actions=crate::actions::ActionProcessor::new(parsed).map_err(str::to_owned)?;
        self.learned_mounting_identity=Some(identity.to_owned());Ok(true)
    }
    fn mounting_valid(&self, t: f64) -> bool {
        self.demo || self.current_mounting_identity(t).is_some_and(|id| self.learned_mounting_identity.as_deref() == Some(id.as_str()))
    }
    fn gate_mounting_preview(&mut self, t: f64) {
        if let Some(session) = &self.sensor_calibration {
            self.action_preview = session.preview;
            return;
        }
        if self.movement_engine != "chest_heading_steps" && !self.mounting_valid(t) {
            self.action_preview.travel_xy = [0.; 2];
            self.action_preview.raw_direction_angles = None;self.action_preview.chest_yaw = None;
            self.action_preview.direction_status = "Sensor calibration required for this chest tracker";
        }
    }
    fn thighs_ready(&self,t:f64)->bool {
        self.layout_count()!=5 || self.thighs.iter().all(|id|self.identity_matches(id)&&self.records.get(id).is_some_and(|r|t>=r.seen&&(t-r.seen<self.engine.cfg.stale || (r.has_interval&&self.sensor_status.get(id)==Some(&1)&&self.peers.iter().any(|(addr,seen)|id.starts_with(&format!("{}:",addr.ip()))&&t>=*seen&&t-*seen<2.)))))
    }
    // A healthy connected device may omit unchanged rotation packets. Holding a
    // pose does not create motion events, and cannot detect a frozen IMU.
    fn refresh_held_poses(&mut self, t: f64) {
        if self.demo {
            return;
        }
        for i in 0..3 {
            let id = &self.mapping[i];
            if !self.identity_matches(id) { continue; }
            if self.sensor_status.get(id) != Some(&1) {
                continue;
            }
            let Some(record) = self.records.get(id) else {
                continue;
            };
            let ip = id.rsplit_once(':').map(|(ip, _)| ip).unwrap_or("");
            let active = self
                .peers
                .iter()
                .any(|(addr, seen)| addr.ip().to_string() == ip && t >= *seen && t - *seen <= 2.);
            if active
                && record.has_interval
                && t >= record.seen
                && t - record.seen >= self.engine.cfg.stale
            {
                let sensor = &mut self.engine.sensors[i];
                sensor.q = record.q;
                sensor.angular = 0.;
                sensor.seen = t;
            }
        }
    }
    fn readiness_detail(&self, t: f64, stage: &str) -> Value {
        let mut roles = serde_json::Map::new();
        for (i, role) in ROLES.iter().enumerate() {
            let id = &self.mapping[i];
            let sample = &self.engine.sensors[i];
            let record = self.records.get(id);
            let ip = id.rsplit_once(':').map(|(ip, _)| ip).unwrap_or("");
            let peer = self
                .peers
                .iter()
                .filter(|(addr, _)| addr.ip().to_string() == ip)
                .map(|(_, seen)| *seen)
                .fold(None::<f64>, |a, b| Some(a.map_or(b, |x| x.max(b))));
            roles.insert((*role).into(), json!({"id":id, "engine_age_ms":(t-sample.seen)*1000.,
                "engine_fresh":t>=sample.seen && t-sample.seen<self.engine.cfg.stale,
                "rotation_age_ms":record.filter(|r| r.seen>=0.).map(|r| (t-r.seen)*1000.),
                "has_interval":record.is_some_and(|r| r.has_interval),
                "peer_age_ms":peer.map(|seen| (t-seen)*1000.), "sensor_status":self.sensor_status.get(id)}));
        }
        json!({"stage":stage, "time":t, "stale_ms":self.engine.cfg.stale*1000., "roles":roles,
            "driver_ready":self.driver_ready(t), "ack_age_ms":(t-self.ack_seen)*1000., "ack_flags":self.ack_flags})
    }
    fn validated_rotation(&self, i: usize, t: f64) -> Option<crate::actions::TimedQuaternion> {
        let sensor = &self.engine.sensors[i];
        if !self.demo {
            let id = &self.mapping[i];
            if !self.identity_matches(id) { return None; }
            let record = self.records.get(id)?;
            if self
                .sensor_status
                .get(id)
                .is_some_and(|status| *status != 1)
            {
                return None;
            }
            if t - record.seen >= self.engine.cfg.stale {
                let ip = id.rsplit_once(':').map(|(ip, _)| ip).unwrap_or("");
                let active = self.peers.iter().any(|(addr, seen)| {
                    addr.ip().to_string() == ip && t >= *seen && t - *seen <= 2.
                });
                if self.sensor_status.get(id) != Some(&1) || !record.has_interval || !active {
                    return None;
                }
            }
        }
        Some(crate::actions::TimedQuaternion {
            value: sensor.q,
            seen: sensor.seen,
        })
    }
    fn action_input(&self, t: f64) -> crate::actions::Input {
        let chest = self.validated_rotation(2, t);
        let chest_accel =
            self.accelerations
                .get(&self.mapping[2])
                .map(|a| crate::actions::TimedVector {
                    value: a.vector,
                    seen: a.seen,
                });
        crate::actions::Input {
            t,
            chest,
            chest_accel,
            hmd: self.hmd,
        }
    }
    fn current_profile(&self) -> Value {
        json!({"locomotion":{"algorithm":if self.engine.cfg.continuous {"continuous"} else {"alternating"},"settings":self.engine.cfg.motion.to_json()},"actions":self.actions.settings().to_json(),"gestures":self.gestures.settings().to_json(),"movement_engine":self.movement_engine,"step_input":if self.engine.cfg.chest_steps {"chest"}else if self.movement_engine=="body_tilt" {"off"}else{"feet"},"detection":{"heading_axis":if self.engine.cfg.y_up {"y"} else {"z"},"threshold":self.engine.cfg.threshold,"acceleration_mode":if self.engine.cfg.accel_has_gravity {"gravity-baseline"} else {"linear"}},"output_mode":self.output_mode,"sensor_layout":{"count":self.layout_count()}})
    }
    fn gesture_input(&self, t: f64) -> crate::gestures::Input {
        let source = |i: usize| {
            let rotation = self.validated_rotation(i, t);
            let acceleration =
                self.accelerations
                    .get(&self.mapping[i])
                    .map(|a| crate::actions::TimedVector {
                        value: a.vector,
                        seen: a.seen,
                    });
            crate::gestures::Source {
                rotation,
                acceleration,
            }
        };
        crate::gestures::Input {
            t,
            chest: source(2),
            left: source(0),
            right: source(1),
            hmd: self.hmd,
        }
    }
    fn profile_store(&self) -> crate::profiles::ProfileStore {
        crate::profiles::ProfileStore::new(
            self.config
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .to_path_buf(),
        )
    }
    fn load_profile_settings(&mut self,v:&Value)->Result<(),String>{self.load_profile_settings_inner(v,false)}
    fn load_profile_settings_inner(&mut self, v: &Value, preserve_chest:bool) -> Result<(), String> {
        let mut cfg = self.engine.cfg.clone();
        if let Some(l) = v.get("locomotion") {
            cfg.continuous = match l["algorithm"].as_str() {
                Some("continuous") => true,
                Some("alternating") => false,
                _ => return Err("Invalid profile algorithm".into()),
            };
            cfg.motion =
                crate::continuous::Settings::from_json(&l["settings"]).map_err(str::to_owned)?;
        }
        let mode = v
            .get("movement_engine")
            .map(|x| x.as_str().ok_or("Invalid movement engine"))
            .transpose()?
            .unwrap_or(&self.movement_engine);
        if !["chest_heading_steps", "body_tilt_steps", "body_tilt"].contains(&mode) {
            return Err("Invalid movement engine".into());
        }
        let step_input=v.get("step_input").map(|x|x.as_str().filter(|s|["off","chest","feet"].contains(s)).ok_or("Invalid profile step input")).transpose()?;
        let count=if let Some(layout)=v.get("sensor_layout") {
            let o=layout.as_object().ok_or("Invalid sensor layout")?;
            if o.len()!=1 {return Err("Profile sensor layout must contain only count".into());}
            o.get("count").and_then(Value::as_u64).filter(|n|[1,3,5].contains(n)).ok_or("Invalid sensor layout count")? as u8
        } else if mode=="body_tilt" {1} else {self.sensor_count.max(3)};
        cfg.chest_steps = mode=="body_tilt_steps" && count==1 && step_input.map_or(self.engine.cfg.chest_steps,|s|s=="chest");
        if step_input.is_some_and(|s| (s=="chest") != cfg.chest_steps || (s=="off") != (mode=="body_tilt")) {return Err("Profile step input and movement engine do not match".into());}
        if cfg.chest_steps && !cfg.continuous {return Err("Chest step input requires continuous motion".into());}
        if (mode=="body_tilt" || cfg.chest_steps) != (count==1) {return Err("Sensor count and movement engine do not match".into());}
        cfg.require_steps = mode != "body_tilt" && !cfg.chest_steps;
        if let Some(d) = v.get("detection") {
            if let Some(x) = d.get("threshold") {
                cfg.threshold = x.as_f64().ok_or("Invalid threshold")?;
            }
            if let Some(x) = d.get("heading_axis") {
                cfg.y_up = match x.as_str() {
                    Some("y") => true,
                    Some("z") => false,
                    _ => return Err("Invalid axis".into()),
                };
            }
            if let Some(x) = d.get("acceleration_mode") {
                cfg.accel_has_gravity = match x.as_str() {
                    Some("linear") => false,
                    Some("gravity-baseline") => true,
                    _ => return Err("Invalid acceleration mode".into()),
                };
            }
        }
        cfg.validate().map_err(str::to_owned)?;
        let mut actions = if let Some(a) = v.get("actions") {
            crate::actions::Settings::from_json(a).map_err(str::to_owned)?
        } else {
            self.actions.settings().clone()
        };
        // Axis mapping belongs to the wearer and physical tracker. Every
        // profile entry point must preserve it, not only game_profile_apply.
        // An old profile can contain the opposite sign or another mount's axes.
        let mounting = &self.actions.settings().direction;
        actions.direction.learned_forward = mounting.learned_forward;
        actions.direction.learned_right = mounting.learned_right;
        actions.direction.pitch_axis = mounting.pitch_axis.clone();
        actions.direction.pitch_sign = mounting.pitch_sign;
        actions.direction.roll_axis = mounting.roll_axis.clone();
        actions.direction.roll_sign = mounting.roll_sign;
        if mode != "chest_heading_steps" {
            actions.direction.enabled = true;
        }
        let gestures = if let Some(g) = v.get("gestures") {
            crate::gestures::Settings::from_json(g).map_err(str::to_owned)?
        } else {
            self.gestures.settings().clone()
        };
        let output = v
            .get("output_mode")
            .and_then(Value::as_str)
            .unwrap_or(&self.output_mode);
        if !["meta", "steamvr"].contains(&output) {
            return Err("Invalid output mode".into());
        }
        self.save_options_count(&self.mapping, &cfg, output, &actions, &gestures, mode, count)?;
        self.gestures = crate::gestures::GestureProcessor::new(gestures).map_err(str::to_owned)?;
        let preserve=preserve_chest && !self.engine.cfg.require_steps && !cfg.require_steps && self.engine.cfg.y_up==cfg.y_up && self.engine.cfg.sign==cfg.sign;
        if preserve {self.engine.reconfigure_chest_preserving_center(cfg).map_err(str::to_owned)?;self.actions.update_settings_preserving_center(actions).map_err(str::to_owned)?;}
        else {self.actions=crate::actions::ActionProcessor::new(actions).map_err(str::to_owned)?;self.engine=Engine::new(cfg);}
        self.output_mode = output.to_owned();
        self.movement_engine = mode.to_owned();
        self.sensor_count = count;
        self.stop_reason = if preserve {"Movement choices updated; centre preserved"}else{"Profile loaded; recalibration required"}.into();
        self.walking = false;
        self.new_generation();
        Ok(())
    }
    pub fn command(&mut self, v: &Value, t: f64) -> Result<(), String> {
        self.actions.set_walking_forward_gain(if self.engine.cfg.chest_steps {Some(self.walking_speed)}else{None});
        self.actions.set_walking_sway(self.engine.cfg.chest_steps && self.walking_model.is_object() && self.walking_speed>0.);
        self.actions.set_world_up(if self.engine.cfg.y_up {[0.,1.,0.]} else {[0.,0.,1.]});
        self.refresh_held_poses(t);
        let command = v["command"].as_str().ok_or("Missing command")?;
        if self.crouch_learning.is_some() && !matches!(command, "crouch_learn_cancel" | "stop") { return Err("Finish or cancel crouch learning first".into()); }
        if self.walking_learning.is_some() && !matches!(command,"walking_learn_cancel"|"stop") {return Err("Finish or cancel walking learning first".into());}
        if self.jump_learning.is_some() && !matches!(command, "jump_learn_cancel" | "stop") { return Err("Finish or cancel jump learning first".into()); }
        if command.starts_with("sensor_calibration_") {
            self.new_generation();
            return self.sensor_calibration_command(v, t);
        }
        if self.sensor_calibration.is_some() && command != "stop" {
            return Err("Finish or cancel sensor calibration first".into());
        }
        match v
            .get("command")
            .and_then(Value::as_str)
            .ok_or("Command must be an object with a command")?
        {
            "crouch_learn_start" => {
                let h=self.action_input(t).hmd.ok_or("Connect the headset before learning crouch")?;
                if !h.height.is_finite() || !(0.05..=4.).contains(&h.height) || t<h.seen || t-h.seen>=0.25 {return Err("Connect the headset before learning crouch".into());}
                self.engine.disarm();self.walking=false;self.actions.reset_transient();self.new_generation();
                self.crouch_learning=Some(crouch_learning::Session::new(t,self.actions.settings().crouch.height_correction));self.crouch_learning_result=Value::Null;
            },
            "crouch_learn_cancel" => {self.crouch_learning=None;self.crouch_learning_result=json!({"active":false,"status":"cancelled","message":"Crouch learning cancelled. No settings saved."});},
            "crouch_learn_dismiss" => {self.crouch_learning_result=Value::Null;},
            "walking_learn_start"=>{
                if !self.engine.cfg.chest_steps || !self.mounting_valid(t) || !self.engine.calibrated {return Err("Enable one-sensor Walk in place, calibrate the chest sensor, then Reset Center first.".into());}
                let identity=self.current_mounting_identity(t).ok_or("Fresh chest tracker required")?;
                self.engine.disarm();self.walking_speed=0.;self.walking_signal=Default::default();self.walking_result=Value::Null;
                self.walking_learning=Some(walking_learning::Session::new(identity,t));
            },
            "walking_full_speed"=>{
                let effort=v["effort"].as_f64().filter(|v|v.is_finite() && (0.1..=2.).contains(v)).ok_or("Walking effort must be between 10% and 200%")?;
                if !self.walking_model.is_object(){return Err("Learn walking first".into());}
                let previous=self.walking_model.clone();self.walking_model["full_speed_effort"]=json!(effort);
                if let Err(e)=self.save(&self.mapping,&self.engine.cfg,&self.output_mode){self.walking_model=previous;return Err(e);}
                self.engine.disarm();
            },
            "walking_learn_cancel"=>{self.walking_learning=None;self.walking_signal=Default::default();self.walking_speed=0.;self.walking_result=json!({"active":false,"message":"Walking learning cancelled. Previous settings kept."});},
            "jump_learn_start" => {
                let source=self.actions.settings().jump.source.clone();
                let input=self.action_input(t);
                let fresh=if source=="height" {input.hmd.is_some_and(|h|t>=h.seen && t-h.seen<0.25)} else {input.chest_accel.is_some_and(|a|t>=a.seen && t-a.seen<0.25)};
                if !fresh {return Err(if source=="height" {"Connect the headset before learning jump"} else {"Connect the assigned chest sensor before learning jump"}.into());}
                self.engine.disarm(); self.walking=false; self.actions.reset_transient(); self.new_generation();
                self.jump_learning=Some(jump_learning::Session::new(source,t)); self.jump_learning_result=Value::Null;
            },
            "jump_learn_cancel" => {self.jump_learning=None;self.jump_learning_result=json!({"active":false,"status":"cancelled","message":"Jump learning cancelled. No settings saved."});},
            "jump_learn_dismiss" => {self.jump_learning_result=Value::Null;},
            "output_mode" => {
                let mode = v["mode"].as_str().ok_or("Missing output mode")?;
                if mode != "meta" && mode != "steamvr" {
                    return Err("Unknown output mode".into());
                }
                if mode != self.output_mode {
                    self.save(&self.mapping, &self.engine.cfg, mode)?;
                    self.output_mode = mode.to_string();
                    self.walking = false;
                    self.reset();
                }
            }
            "mode" => {
                let mode = v["mode"].as_str().ok_or("Missing mode")?;
                if mode != "live" && mode != "demo" {
                    return Err("Unknown mode".into());
                }
                if let Some(mut f) = self.recording.take() {
                    let _ = f.flush();
                }
                self.demo = mode == "demo";
                self.walking = false;
                self.reset();
            }
            "calibrate" => {
                self.gestures.reset();
                self.gesture_preview = Default::default();
                self.walking = false;
                self.engine.disarm();
                self.engine.calibrated = false;
                self.new_generation();
                let missing: Vec<_> = ["left foot", "right foot", "chest"].iter().enumerate()
                    .filter(|(i,_)| (*i==2 || self.engine.cfg.require_steps) && (t < self.engine.sensors[*i].seen || t-self.engine.sensors[*i].seen >= self.engine.cfg.stale))
                    .map(|(_,name)| *name).collect();
                if !missing.is_empty(){return Err(format!("Missing tracker data: {}. Check tracker power and Wi-Fi.",missing.join(", ")));}

                if !self.thighs_ready(t){return Err("5-sensor mode needs live left and right thigh data".into());}
                self.engine.calibrate(t)?;
                let input = self.action_input(t);
                let calibration = if !crate::actions::ActionProcessor::has_fresh_head(&input) {
                    self.actions.calibrate_chest_preview(&input)
                } else {
                    self.actions.calibrate(&input)
                };
                if let Err(e) = calibration {
                    self.engine.calibrated = false;
                    return Err(e.into());
                }
            }
            "arm" => {
                if !self.thighs_ready(t){return Err("Waiting for both thigh trackers".into());}
                let a = self.actions.settings();
                let advanced = self.movement_engine != "chest_heading_steps"
                    || a.direction.enabled
                    || a.jump.enabled
                    || a.crouch.enabled
                    || a.turn.enabled
                    || self.engine.cfg.motion.sprint_enabled
                    || self.gestures.settings().enabled;
                if !self.demo && advanced && (self.output_mode == "meta" || !self.protocol_v2) {
                    return Err("Selected actions require SteamVR protocol 2 output".into());
                }
                if self.output_mode == "steamvr" && !self.demo && !self.driver_ready(t) {
                    return Err("SteamVR driver and HMD are not ready".into());
                }
                let requires_head_center = (a.direction.enabled && a.direction.decouple)
                    || a.turn.enabled || (a.crouch.enabled && a.crouch.use_reset_offset)
                    || (a.jump.enabled && a.jump.source == "height" && a.jump.angle_compensation_percent > 0.);
                if requires_head_center && !self.actions.has_head_center() {
                    return Err("Connect the headset and recenter before game movement".into());
                }
                self.engine.arm(t)?;
                self.actions.reset_transient();
                self.action_preview = Default::default();
                self.gestures.reset();
                self.gesture_preview = Default::default();
                self.stop_reason.clear();
                self.stop_detail = Value::Null;
            }
            "reset_test" => {
                self.actions.reset_transient();
                self.action_preview = Default::default();
                self.engine.disarm();
                self.gestures.reset();
                self.gesture_preview = Default::default();
                self.engine.preview_steps = [0; 2];
                self.walking = false;
                self.stop_reason = "Test reset by user".into();
                self.stop_detail = self.readiness_detail(t, "reset_test");
            }
            "stop" => {
                if self.walking_learning.take().is_some(){self.walking_result=json!({"active":false,"message":"Walking learning stopped. Previous settings kept."});}
                self.walking_signal=Default::default();self.walking_speed=0.;
                self.actions.reset_transient();
                self.action_preview = Default::default();
                self.gestures.reset();
                self.gesture_preview = Default::default();
                self.stop_reason = "Stopped by user".into();
                self.stop_detail = self.readiness_detail(t, "stop");
                self.engine.disarm();
                self.walking = false;
            }
            "demo" => {
                if !self.demo {
                    return Err("Switch to Synthetic demo first".into());
                }
                let yaw = v["yaw"].as_f64().ok_or("Missing yaw")?;
                if !yaw.is_finite() || yaw.abs() > 180. {
                    return Err("Yaw must be -180 to 180".into());
                }
                self.demo_yaw = yaw.to_radians();
                self.walking = v["walk"].as_bool().ok_or("walk must be boolean")?;
            }
            "sensor_layout" => {
                let count=v["count"].as_u64().filter(|n|[1,3,5].contains(n)).ok_or("Choose 1, 3 or 5 sensors")? as u8;
                let old=self.sensor_count;self.sensor_count=count;
                let mode=if count==1{"body_tilt"}else{"chest_heading_steps"};
                if let Err(e)=self.command(&json!({"command":"movement_engine","engine":mode}),t){self.sensor_count=old;return Err(e);}
            }
            "assign_sensor_role" => {
                let role=v["role"].as_str().ok_or("Missing body role")?;let id=v["id"].as_str().ok_or("Missing sensor ID")?;
                if !id.is_empty()&&!self.records.contains_key(id){return Err("Sensor has not sent rotation data".into());}
                let mut mapping=self.mapping.clone();let mut thighs=self.thighs.clone();
                let index=match role {"left"=>0,"right"=>1,"torso"=>2,"left_thigh"=>3,"right_thigh"=>4,_=>return Err("Invalid body role".into())};
                if self.layout_count()==1&&index!=2{return Err("Chest is the only role in 1-sensor mode".into());}
                if index>=3&&self.layout_count()!=5{return Err("Thigh roles require 5-sensor mode".into());}
                // Moving a sensor between slots clears its previous assignment.
                if !id.is_empty(){for old in mapping.iter_mut().chain(thighs.iter_mut()){if old==id{old.clear();}}}
                if index<3{mapping[index]=id.into();}else{thighs[index-3]=id.into();}
                if let Some((ip,_))=id.rsplit_once(':') {if let Some(mac)=self.peer_macs.get(ip){self.tracker_identities.insert(id.into(),mac.clone());}}
                let previous=self.thighs.clone();self.thighs=thighs;
                if let Err(e)=self.save(&mapping,&self.engine.cfg,&self.output_mode){self.thighs=previous;return Err(e);}
                self.mapping=mapping;self.reset();
                if index==2 {let chest=self.mapping[2].clone();if let Some((ip,sensor))=chest.rsplit_once(':'){if let Some(mac)=self.peer_macs.get(ip){let identity=format!("{}:{}",mac.to_ascii_uppercase(),sensor);if self.restore_device_mounting(&identity)? {self.save(&self.mapping,&self.engine.cfg,&self.output_mode)?;}}}}
            }
            "bind_device_identity" => {
                let id=v["id"].as_str().ok_or("Missing sensor ID")?;
                if !self.mapping.iter().chain(self.thighs.iter()).any(|x|x==id) {return Err("Assign the sensor before binding its identity".into());}
                let mac=v["mac"].as_str().ok_or("Missing MAC")?.replace(':',"").replace('-',"");
                if mac.len()!=12 || !mac.bytes().all(|b|b.is_ascii_hexdigit()) {return Err("Invalid MAC".into());}
                let bytes=(0..6).map(|i|u8::from_str_radix(&mac[i*2..i*2+2],16).unwrap()).collect::<Vec<_>>();
                if bytes[0]&1!=0 || bytes.iter().all(|b|*b==0) {return Err("Invalid MAC".into());}
                let mac=bytes.iter().map(|b|format!("{b:02X}")).collect::<Vec<_>>().join(":");
                self.tracker_identities.insert(id.into(),mac);
                self.save(&self.mapping,&self.engine.cfg,&self.output_mode)?;
                self.reset();
            }
            "assign" => {
                if self.demo {
                    return Err("Switch to live trackers before assigning hardware".into());
                }
                let object = v["trackers"].as_object().ok_or("Choose three trackers")?;
                if object.len() != 3 {
                    return Err("Assign exactly three roles".into());
                }
                let mut m = std::array::from_fn(|_| String::new());
                for (i, r) in ROLES.iter().enumerate() {
                    let id = object
                        .get(*r)
                        .and_then(Value::as_str)
                        .ok_or("Missing tracker role")?;
                    if self.movement_engine!="body_tilt" || i==2 {
                        self.records.get(id).ok_or("Required tracker not discovered")?;
                    }
                    m[i] = id.to_string();
                }
                if (0..3).any(|i|(i+1..3).any(|j| !m[i].is_empty() && m[i]==m[j])) {
                    return Err("Choose three DIFFERENT trackers".into());
                }
                for id in &m {if let Some((ip,_))=id.rsplit_once(':') {if let Some(mac)=self.peer_macs.get(ip){self.tracker_identities.insert(id.clone(),mac.clone());}}}
                self.save(&m, &self.engine.cfg, &self.output_mode)?;
                self.mapping = m;
                self.reset();
                let chest=self.mapping[2].clone();
                if let Some((ip,sensor))=chest.rsplit_once(':'){if let Some(mac)=self.peer_macs.get(ip){let identity=format!("{}:{}",mac.to_ascii_uppercase(),sensor);if self.restore_device_mounting(&identity)? {self.save(&self.mapping,&self.engine.cfg,&self.output_mode)?;}}}
            }
            "remap_tracker" => {
                let id = v["id"].as_str().ok_or("Missing tracker ID")?;
                let (old_ip, sensor) = id.rsplit_once(':').ok_or("Invalid tracker ID")?;
                old_ip
                    .parse::<std::net::Ipv4Addr>()
                    .map_err(|_| "Invalid tracker ID")?;
                let sensor = sensor.parse::<u8>().map_err(|_| "Invalid tracker ID")?;
                let index = self
                    .mapping
                    .iter()
                    .position(|mapped| mapped == id)
                    .ok_or("Assign this sensor to body first")?;
                let ip = v["ip"]
                    .as_str()
                    .ok_or("Missing IPv4 address")?
                    .parse::<std::net::Ipv4Addr>()
                    .map_err(|_| "Invalid IPv4 address")?;
                if ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast() {
                    return Err("Invalid IPv4 address".into());
                }
                let new_id = format!("{ip}:{sensor}");
                if self
                    .mapping
                    .iter()
                    .enumerate()
                    .any(|(i, mapped)| i != index && *mapped == new_id)
                {
                    return Err("That address is already assigned to another role".into());
                }
                let mut mapping = self.mapping.clone();
                mapping[index] = new_id;
                self.save(&mapping, &self.engine.cfg, &self.output_mode)?;
                self.mapping = mapping;
                self.reset();
            }
            "save_settings" => {
                if let Some(settings) = v.get("settings") {
                    self.load_profile_settings(settings)?;
                } else {
                    self.save(&self.mapping, &self.engine.cfg, &self.output_mode)?;
                }
            }
            "gesture_settings" => {
                let gestures =
                    crate::gestures::Settings::from_json(&v["settings"]).map_err(str::to_owned)?;
                self.save_options(
                    &self.mapping,
                    &self.engine.cfg,
                    &self.output_mode,
                    self.actions.settings(),
                    &gestures,
                    &self.movement_engine,
                )?;
                self.gestures =
                    crate::gestures::GestureProcessor::new(gestures).map_err(str::to_owned)?;
                self.engine = Engine::new(self.engine.cfg.clone());
                self.actions.reset();
                self.walking = false;
                self.stop_reason = "Gesture settings changed; recalibration required".into();
                self.new_generation();
            }
            "step_input" => {
                let source=v["source"].as_str().ok_or("Missing step input source")?;
                let (mode,chest)=match source {"off"=>("body_tilt",false),"chest"=>("body_tilt_steps",true),"feet"=>("body_tilt_steps",false),_=>return Err("Choose off, chest or feet".into())};
                let mut cfg=self.engine.cfg.clone();cfg.chest_steps=chest;cfg.require_steps=source=="feet";if chest {cfg.continuous=true;}
                let mut actions=self.actions.settings().clone();actions.direction.enabled=true;
                let count=if source=="feet" {3} else {1};
                self.save_options_count(&self.mapping,&cfg,&self.output_mode,&actions,self.gestures.settings(),mode,count)?;
                let preserve=!self.engine.cfg.require_steps && !cfg.require_steps;
                self.movement_engine=mode.into();self.sensor_count=count;
                if preserve {self.engine.reconfigure_chest_preserving_center(cfg).map_err(str::to_owned)?;self.actions.update_settings_preserving_center(actions).map_err(str::to_owned)?;}else{self.engine=Engine::new(cfg);self.actions=crate::actions::ActionProcessor::new(actions).map_err(str::to_owned)?;}
                self.walking=false;self.stop_reason=if preserve {"Walking option changed; centre preserved"}else{"Step input changed; recalibration required"}.into();self.new_generation();
            }
            "movement_engine" => {
                let mode = v["engine"].as_str().ok_or("Missing movement engine")?;
                if !["chest_heading_steps", "body_tilt_steps", "body_tilt"].contains(&mode) {
                    return Err("Invalid movement engine".into());
                }
                let mut cfg = self.engine.cfg.clone();
                cfg.chest_steps = false;
                cfg.require_steps = mode != "body_tilt";
                let mut actions = self.actions.settings().clone();
                if mode != "chest_heading_steps" {
                    actions.direction.enabled = true;
                }
                self.save_options(
                    &self.mapping,
                    &cfg,
                    &self.output_mode,
                    &actions,
                    self.gestures.settings(),
                    mode,
                )?;
                self.movement_engine = mode.to_owned();
                self.actions =
                    crate::actions::ActionProcessor::new(actions).map_err(str::to_owned)?;
                self.engine = Engine::new(cfg);
                self.walking = false;
                self.stop_reason = "Movement engine changed; recalibration required".into();
                self.new_generation();
            }
            "device_mounting" => {
                let identity=self.current_mounting_identity(t).ok_or("A fresh assigned chest tracker is required")?;
                if v["identity"].as_str()!=Some(identity.as_str()) {return Err("Selected device is not the current chest tracker".into());}
                if !self.mounting_valid(t) {return Err("Sensor calibration required for this chest tracker".into());}
                let operation=v["operation"].as_str().ok_or("Missing device mapping operation")?;
                let mut settings=self.actions.settings().clone();
                match operation {
                    "turn_over"=>{settings.direction.pitch_sign *= -1.;settings.direction.roll_sign *= -1.;},
                    "flip_forward"=>settings.direction.pitch_sign *= -1.,
                    "flip_lateral"=>settings.direction.roll_sign *= -1.,
                    "reset"=>{let defaults=crate::actions::Direction::default();settings.direction.pitch_axis=defaults.pitch_axis;settings.direction.roll_axis=defaults.roll_axis;settings.direction.pitch_sign=1.;settings.direction.roll_sign=1.;settings.direction.learned_forward=None;settings.direction.learned_right=None;},
                    _=>return Err("Unknown device mapping operation".into()),
                }
                settings.validate().map_err(str::to_owned)?;
                self.engine.disarm();self.action_preview=Default::default();self.walking=false;
                if operation=="reset" {self.device_mountings.remove(&identity);self.learned_mounting_identity=None;self.actions=crate::actions::ActionProcessor::new(settings).map_err(str::to_owned)?;self.engine.calibrated=false;}
                else {self.actions.update_settings_preserving_center(settings).map_err(str::to_owned)?;self.remember_device_mounting(&identity);}
                if operation=="turn_over" {
                    let record=self.device_mountings.get_mut(&identity).unwrap();let face=record["face_turned_over"].as_bool().unwrap_or(false);record["face_turned_over"]=json!(!face);
                    self.actions.reset();self.engine.calibrated=false;
                }
                self.save(&self.mapping,&self.engine.cfg,&self.output_mode)?;self.new_generation();
                self.stop_reason=if operation=="turn_over" {"Mounting face changed; stand upright and Reset Center"} else if operation=="reset" {"Device calibration reset; recalibration required"}else{"Device mapping changed; movement stopped"}.into();
            }
            "learn_tilt_right" => {
                self.engine.disarm();self.new_generation();
                if !self.mounting_valid(t) { return Err("Sensor calibration required for this chest tracker".into()); }
                let input=self.action_input(t);
                self.actions.learn_right(&input).map_err(str::to_owned)?;
                if let Some(id)=self.learned_mounting_identity.clone(){self.remember_device_mounting(&id);}
                self.save(&self.mapping,&self.engine.cfg,&self.output_mode)?;
                self.stop_reason="Right lean learned; return upright. Movement stopped.".into();
            }
            "learn_tilt_forward" => {
                self.engine.disarm();self.new_generation();
                let identity = if self.demo { None } else { Some(self.current_mounting_identity(t).ok_or("Sensor calibration required for this chest tracker")?) };
                let input=self.action_input(t);self.actions.learn_forward(&input).map_err(str::to_owned)?;
                if let Some(identity) = identity { self.learned_mounting_identity = Some(identity.clone());self.remember_device_mounting(&identity);self.device_mountings.get_mut(&identity).unwrap()["face_turned_over"]=json!(false); }
                self.save(&self.mapping,&self.engine.cfg,&self.output_mode)?;
                self.stop_reason="Forward lean learned; return upright. Movement stopped.".into();
            }
            "actions_settings" => {
                let mut actions =
                    crate::actions::Settings::from_json(&v["settings"]).map_err(str::to_owned)?;
                if self.movement_engine != "chest_heading_steps" {
                    actions.direction.enabled = true;
                }
                // Tuning pages and wizard snapshots may predate a device flip.
                // Hardware mapping is changed only by device_mounting or learning,
                // never by applying an entire (possibly stale) motion draft.
                let current=&self.actions.settings().direction;
                actions.direction.learned_forward=current.learned_forward;
                actions.direction.learned_right=current.learned_right;
                actions.direction.pitch_axis=current.pitch_axis.clone();
                actions.direction.roll_axis=current.roll_axis.clone();
                actions.direction.pitch_sign=current.pitch_sign;
                actions.direction.roll_sign=current.roll_sign;
                // Restoring/canceling a wizard must not attach its old axes to
                // the tracker that was just learned during the canceled run.
                let old_binding = self.learned_mounting_identity.clone();
                let previous = &self.actions.settings().direction;
                let same_mounting = actions.direction.learned_forward == previous.learned_forward
                    && actions.direction.learned_right == previous.learned_right
                    && actions.direction.pitch_axis == previous.pitch_axis && actions.direction.roll_axis == previous.roll_axis
                    && actions.direction.pitch_sign == previous.pitch_sign && actions.direction.roll_sign == previous.roll_sign;
                if actions.direction.learned_forward != previous.learned_forward
                    || actions.direction.learned_right != previous.learned_right {
                    self.learned_mounting_identity = None;
                }
                let saved = self.save_options(
                    &self.mapping,
                    &self.engine.cfg,
                    &self.output_mode,
                    &actions,
                    self.gestures.settings(),
                    &self.movement_engine,
                );
                if let Err(error) = saved {
                    self.learned_mounting_identity = old_binding;
                    return Err(error);
                }
                if same_mounting {
                    self.actions.update_settings_preserving_center(actions).map_err(str::to_owned)?;
                    self.engine.disarm();
                    self.stop_reason = "Actions changed; movement stopped".into();
                } else {
                    self.actions = crate::actions::ActionProcessor::new(actions).map_err(str::to_owned)?;
                    self.engine = Engine::new(self.engine.cfg.clone());
                    self.stop_reason = "Actions changed; recalibration required".into();
                }
                if let Some(id)=self.learned_mounting_identity.clone() {self.remember_device_mounting(&id);self.save(&self.mapping,&self.engine.cfg,&self.output_mode)?;}
                self.action_preview=Default::default();self.gestures.reset();self.gesture_preview=Default::default();
                self.walking = false;
                self.new_generation();
            }
            "apply_movement_profile" => {
                let mut profile=crate::profile_builder::build(&self.current_profile(),v.get("choices").ok_or("Missing choices")?)?;
                // Profile builder explicitly selects chest wobble, feet, or off.
                crate::actions::Settings::from_json(&profile["actions"]).map_err(str::to_owned)?;
                crate::gestures::Settings::from_json(&profile["gestures"]).map_err(str::to_owned)?;
                crate::continuous::Settings::from_json(&profile["locomotion"]["settings"]).map_err(str::to_owned)?;
                if let Some(name)=v.get("name") {let name=name.as_str().ok_or("Profile name must be text")?;let store=self.profile_store();store.save(name,"Movement profile",profile.clone())?;self.profile_list=store.list()?;}
                self.load_profile_settings_inner(&profile,true)?;
            }
            "profile_save" => {
                let name = v["name"].as_str().ok_or("Missing name")?;
                let desc = v["description"].as_str().unwrap_or("");
                let store = self.profile_store();
                store.save(name, desc, self.current_profile())?;
                self.profile_list = store.list()?;
            }
            "profile_import" => {
                let value = v.get("profile").ok_or("Missing profile")?.clone();
                let profile_settings = value
                    .get("settings")
                    .and_then(Value::as_object)
                    .ok_or("Missing profile settings")?;
                if let Some(g) = profile_settings.get("gestures") {
                    crate::gestures::Settings::from_json(g).map_err(str::to_owned)?;
                }
                if let Some(a) = profile_settings.get("actions") {
                    crate::actions::Settings::from_json(a).map_err(str::to_owned)?;
                }
                if let Some(l) = profile_settings.get("locomotion") {
                    if ![Some("continuous"), Some("alternating")].contains(&l["algorithm"].as_str())
                    {
                        return Err("Invalid profile algorithm".into());
                    }
                    crate::continuous::Settings::from_json(&l["settings"])
                        .map_err(str::to_owned)?;
                }
                if let Some(mode) = profile_settings.get("movement_engine") {
                    if ![
                        Some("chest_heading_steps"),
                        Some("body_tilt_steps"),
                        Some("body_tilt"),
                    ]
                    .contains(&mode.as_str())
                    {
                        return Err("Invalid movement engine".into());
                    }
                }
                let store = self.profile_store();
                store.import(value)?;
                self.profile_list = store.list()?;
            }
            "profile_delete" => {
                let store = self.profile_store();
                store.delete(v["name"].as_str().ok_or("Missing name")?)?;
                self.profile_list = store.list()?;
            }
            "game_profile_apply" | "game_profile_restore" => {
                self.engine.disarm();
                self.walking = false;
                self.travel_xy = [0.;2]; self.turn_xy = [0.;2]; self.output_buttons = 0;
                let id = v["game_id"].as_str().ok_or("Missing game ID")?;
                if !["arizona_sunshine_legacy", "custom"].contains(&id) { return Err("Unknown game profile".into()); }
                let restore = v["command"] == "game_profile_restore";
                let name = if restore {self.game_profiles["associations"][id]["profile_name"].as_str()} else {v["profile_name"].as_str()}.map(str::to_owned);
                let partial = if restore {
                    self.game_profiles["associations"][id].get("settings").cloned().ok_or("No settings saved for this game")?
                } else if let Some(name) = &name { self.profile_store().load(name)? } else { self.current_profile() };
                let mut settings = self.current_profile();
                for (key, value) in partial.as_object().ok_or("Invalid game settings")? {
                    if key == "detection" {
                        for (field,value) in value.as_object().ok_or("Invalid detection settings")? {settings[key][field]=value.clone();}
                    } else {settings[key]=value.clone();}
                }
                if partial.get("sensor_layout").is_none() && partial.get("movement_engine").is_some() {
                    settings["sensor_layout"]=json!({"count":if settings["movement_engine"]=="body_tilt" {1}else{self.sensor_count.max(3)}});
                }
                if partial.get("step_input").is_none() && partial.get("movement_engine").is_some() {
                    settings["step_input"]=json!(if settings["movement_engine"]=="body_tilt" {"off"}else{"feet"});
                }
                // Mounting directions belong to this wearer and hardware, not a game.
                // Merge partial profiles before preserving these fields.
                if settings.get("actions").is_none() {settings["actions"] = self.actions.settings().to_json();}
                let current = self.actions.settings().to_json();
                for key in ["learned_forward", "learned_right", "pitch_axis", "pitch_sign", "roll_axis", "roll_sign"] {
                    settings["actions"]["direction"][key] = current["direction"][key].clone();
                }
                settings["output_mode"] = json!("steamvr");
                let previous = self.game_profiles.clone();
                self.game_profiles["selected"] = json!(id);
                if !self.game_profiles["associations"].is_object() {self.game_profiles["associations"] = json!({});}
                self.game_profiles["associations"][id] = json!({"profile_name":name,"settings":settings});
                if let Err(error) = self.load_profile_settings(&settings) {self.game_profiles = previous;return Err(error);}
                self.stop_reason = "Game settings applied; reset center, then Start Movement when ready".into();
            }
            "profile_load" => {
                let store = self.profile_store();
                let profile = store.load(v["name"].as_str().ok_or("Missing name")?)?;
                self.load_profile_settings(&profile)?;
            }
            "locomotion_settings" => {
                let mut cfg = self.engine.cfg.clone();
                cfg.continuous = match v["algorithm"].as_str() {
                    Some("continuous") => true,
                    Some("alternating") if !cfg.chest_steps => false,
                    _ => return Err("Choose continuous or alternating locomotion".into()),
                };
                cfg.motion = crate::continuous::Settings::from_json(&v["settings"])
                    .map_err(|e| e.to_string())?;
                cfg.validate()?;
                self.save(&self.mapping, &cfg, &self.output_mode)?;
                self.engine.update_motion_settings(cfg).map_err(str::to_owned)?;
                self.actions.reset_transient();self.action_preview=Default::default();self.gestures.reset();self.gesture_preview=Default::default();
                self.stop_reason = "Locomotion settings saved; movement stopped".into();
                self.walking = false;
                self.new_generation();
            }
            "settings" => {
                let mut cfg = self.engine.cfg.clone();
                cfg.y_up = match v["axis"].as_str() {
                    Some("y") => true,
                    Some("z") => false,
                    _ => return Err("Axis must be y or z".into()),
                };
                cfg.threshold = v["threshold"].as_f64().ok_or("Missing threshold")?;
                if let Some(mode) = v.get("acceleration_mode") {
                    cfg.accel_has_gravity = match mode.as_str() {
                        Some("linear") => false,
                        Some("gravity-baseline") => true,
                        _ => return Err("Invalid acceleration mode".into()),
                    };
                }
                cfg.validate()?;
                self.save(&self.mapping, &cfg, &self.output_mode)?;
                self.engine = Engine::new(cfg);
                self.stop_reason = "Settings changed; recalibration required".into();
                self.new_generation();
            }
            "record" => {
                if let Some(mut f) = self.recording.take() {
                    f.flush().map_err(|e| e.to_string())?;
                } else {
                    if self.demo {
                        return Err("Live recording only; demo is synthetic".into());
                    }
                    let dir = self
                        .config
                        .parent()
                        .unwrap_or(std::path::Path::new("."))
                        .join("recordings");
                    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                    let name = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos();
                    let path = dir.join(format!("trackers-{name}.jsonl"));
                    let file = fs::OpenOptions::new()
                        .create_new(true)
                        .write(true)
                        .open(&path)
                        .map_err(|e| e.to_string())?;
                    self.recording = Some(BufWriter::new(file));
                    self.record_path = Some(path);
                }
            }
            _ => return Err("Unknown command".into()),
        }
        Ok(())
    }
    pub fn ingest(
        &mut self,
        data: &[u8],
        addr: SocketAddr,
        t: f64,
    ) -> Result<Vec<Vec<u8>>, &'static str> {
        let (seq, packets) = slime::unpack(data)?;
        // Validate the entire packet before advancing counters or touching motion.
        for (k, b) in &packets {
            slime::rotation(*k, b)?;
            slime::acceleration(*k, b)?;
        }
        let hello = packets.iter().any(|(k, _)| *k == 3);
        // Older devices can reply to heartbeat requests with sequence zero.
        // That proves device activity only, never a new pose or motion sample.
        let zero_heartbeat =
            seq == 0 && !packets.is_empty() && packets.iter().all(|(k, _)| *k == 0);
        if !hello && !zero_heartbeat && self.counters.get(&addr).is_some_and(|last| seq <= *last) {
            return Ok(vec![]);
        }
        self.refresh_held_poses(t);
        if self.engine.armed && !self.engine.fresh(t) {
            self.stop_reason = "Tracker signal or device heartbeat expired".into();
            self.stop_detail = self.readiness_detail(t, "before_ingest");
            self.engine.disarm();
            self.actions.reset_transient();
            self.action_preview = Default::default();
            self.gestures.reset();
            self.gesture_preview = Default::default();
        }
        if hello {
            let prefix = format!("{}:", addr.ip());
            let ip = addr.ip().to_string();
            let mac = packets.iter().find_map(|(k,b)| (*k==3).then(||slime::discovery_mac(b)).flatten());
            let verified_identity=mac.as_ref().is_some_and(|mac|self.mapping.iter().chain(self.thighs.iter()).any(|id|self.tracker_identities.get(id)==Some(mac)));
            self.peer_macs.remove(&ip);
            if let Some(mac)=mac.clone() {
                self.peer_macs.insert(ip.clone(),mac.clone());
                let mut changed=false;
                let current:Vec<String>=self.mapping.iter().chain(self.thighs.iter()).cloned().collect();
                for (index,old) in current.iter().enumerate() {
                    if old.is_empty() {continue;}
                    let Some((old_ip,sensor))=old.rsplit_once(':') else {continue;};
                    if old_ip==ip {
                        if !self.tracker_identities.contains_key(old) {self.tracker_identities.insert(old.clone(),mac.clone());changed=true;}
                    } else if self.tracker_identities.get(old)==Some(&mac) {
                        let new_id=format!("{ip}:{sensor}");
                        if current.iter().any(|x|x==&new_id) || self.tracker_identities.get(&new_id).is_some_and(|m|m!=&mac) {continue;}
                        if index<3 {self.mapping[index]=new_id.clone();} else {self.thighs[index-3]=new_id.clone();}
                        self.tracker_identities.remove(old);self.tracker_identities.insert(new_id,mac.clone());changed=true;
                        // The MAC proves this is the same assigned device; retain its center.
                        self.engine.disarm();self.actions.reset_transient();self.gestures.reset();self.new_generation();
                        if index<3 {self.engine.sensors[index]=Default::default();}
                        self.stop_reason="Known tracker reconnected; center preserved, movement stopped".into();
                    }
                }
                let chest=self.mapping[2].clone();
                if let Some((chest_ip,index))=chest.rsplit_once(':') {if chest_ip==ip {let identity=format!("{}:{}",mac.to_ascii_uppercase(),index);if self.learned_mounting_identity.as_deref()!=Some(identity.as_str()) && self.restore_device_mounting(&identity).unwrap_or(false) {changed=true;}}}
                if changed {if let Err(e)=self.save(&self.mapping,&self.engine.cfg,&self.output_mode) {self.error=format!("Could not save tracker identity: {e}");self.reset();}}
            }
            let hello_detail = self.readiness_detail(t, "tracker_handshake");
            self.records.retain(|id, _| !id.starts_with(&prefix));
            self.accelerations.retain(|id, _| !id.starts_with(&prefix));
            self.sensor_status.retain(|id, _| !id.starts_with(&prefix));
            if self.mapping.iter().any(|id| id.starts_with(&prefix)) || (self.layout_count()==5 && self.thighs.iter().any(|id|id.starts_with(&prefix))) {
                let same_device=verified_identity && mac.as_ref().is_some_and(|mac|self.mapping.iter().chain(self.thighs.iter()).filter(|id|id.starts_with(&prefix)).all(|id|self.tracker_identities.get(id)==Some(mac)));
                self.stop_detail = hello_detail;
                if same_device {
                    self.engine.disarm();self.actions.reset_transient();self.gestures.reset();self.action_preview=Default::default();self.walking_signal=Default::default();self.walking_speed=0.;self.walking_effort=0.;
                    for i in 0..3 {if self.mapping[i].starts_with(&prefix){self.engine.sensors[i]=Default::default();}}
                    self.new_generation();self.stop_reason="Tracker reconnected; center preserved. Start movement when ready.".into();
                } else {self.stop_reason="Tracker identity changed or unavailable; recalibration required".into();self.reset();}
            }
        }
        if !self.peers.contains_key(&addr) && self.peers.len() >= 128 {
            return Err("peer limit");
        }
        if !zero_heartbeat {
            self.counters.insert(addr, seq);
        }
        self.peers.insert(addr, t);
        self.refresh_held_poses(t);
        self.received += 1;
        let mut replies = vec![];
        for (k, b) in packets {
            match k {
                3 => {
                    let mut p = vec![0; 64];
                    p[..14].copy_from_slice(b"\x03Hey OVR =D 5\0");
                    replies.push(p);
                }
                15 if b.len() >= 2 => {
                    let id = format!("{}:{}", addr.ip(), b[0]);
                    self.sensor_status.insert(id.clone(), b[1]);
                    if b[1] != 1 {
                        self.records.remove(&id);
                        self.accelerations.remove(&id);
                        if self.mapping.iter().any(|mapped| *mapped == id) || (self.layout_count()==5 && self.thighs.contains(&id)) {
                            self.stop_reason =
                                "Assigned sensor reported an error; recalibration required".into();
                            self.reset();
                        }
                    }
                    let mut p = 15u32.to_be_bytes().to_vec();
                    p.extend(&b[..2]);
                    replies.push(p);
                }
                10 if b.len() >= 4 => {
                    let mut p = 10u32.to_be_bytes().to_vec();
                    p.extend(0u64.to_be_bytes());
                    p.extend(&b[..4]);
                    replies.push(p);
                }
                22 => {
                    let mut p = 22u32.to_be_bytes().to_vec();
                    p.extend([0; 12]);
                    replies.push(p);
                }
                _ => {}
            }
            if let Some((sensor, vector)) = slime::acceleration(k, b)? {
                let id = format!("{}:{sensor}", addr.ip());
                if !self.records.contains_key(&id) && self.records.len() >= 256 {
                    return Err("tracker limit");
                }
                self.records.entry(id.clone()).or_insert(Record {
                    q: [1., 0., 0., 0.],
                    seen: -1e9,
                    angular: 0.,
                    has_interval: false,
                });
                let count = self
                    .accelerations
                    .get(&id)
                    .map_or(1, |a| a.count.saturating_add(1));
                if !self.demo
                    && self
                        .sensor_status
                        .get(&id)
                        .is_none_or(|status| *status == 1)
                {
                    if self.mapping[2]==id && self.identity_matches(&id) {self.engine.update_chest_diagnostic(vector,t)?;}
                    for i in 0..3 {
                        if (if self.engine.cfg.chest_steps {i==2} else {i<2}) && self.mapping[i] == id && self.identity_matches(&id) {
                            // Acceleration is a real foot sample; retain the last real pose,
                            // but never create one from acceleration alone.
                            if self.sensor_status.get(&id) == Some(&1)
                                && self.records[&id].seen >= 0.
                            {
                                self.engine.sensors[i].q = self.records[&id].q;
                                self.engine.sensors[i].seen = t;
                            }
                            self.engine.update_acceleration(i, vector, t)?;
                        }
                    }
                }
                self.accelerations.insert(
                    id,
                    Acceleration {
                        vector,
                        seen: t,
                        count,
                    },
                );
            }
            if let Some((sensor, q)) = slime::rotation(k, b)? {
                let id = format!("{}:{sensor}", addr.ip());
                if !self.records.contains_key(&id) && self.records.len() >= 256 {
                    return Err("tracker limit");
                }
                let had_interval = self.records.get(&id).is_some_and(|r| r.has_interval);
                let previous_seen = self
                    .records
                    .get(&id)
                    .and_then(|r| (r.seen >= 0.).then_some(r.seen));
                // Diagnostic speed is independent of role assignment.
                let angular = self
                    .records
                    .get(&id)
                    .filter(|r| r.seen >= 0.)
                    .map_or(0., |old| {
                        let dt = t - old.seen;
                        if dt <= 0. {
                            return 0.;
                        }
                        let dot: f64 = old.q.iter().zip(q).map(|(a, b)| a * b).sum();
                        2. * dot.abs().clamp(0., 1.).acos() / dt
                    });
                self.records.insert(
                    id.clone(),
                    Record {
                        q,
                        seen: t,
                        angular,
                        has_interval: had_interval || previous_seen.is_some_and(|seen| t > seen),
                    },
                );
                if self.auto_add && self.layout_count()==1 && self.mapping[2].is_empty() && !self.demo && self.sensor_status.get(&id).is_none_or(|status|*status==1) {
                    self.mapping[2]=id.clone();
                    if let Some(mac)=self.peer_macs.get(&addr.ip().to_string()) {self.tracker_identities.insert(id.clone(),mac.clone());}
                    self.reset();
                    if let Err(e)=self.save(&self.mapping,&self.engine.cfg,&self.output_mode) {self.error=format!("Could not save tracker assignment: {e}");}
                }
                if !self.demo
                    && self
                        .sensor_status
                        .get(&id)
                        .is_none_or(|status| *status == 1)
                {
                    for i in 0..3 {
                        if self.mapping[i] == id && self.identity_matches(&id) {
                            if previous_seen.is_some_and(|seen| t - seen >= self.engine.cfg.stale) {
                                // A held timestamp is not a real measurement interval.
                                // Reacquire without inventing a high-speed step.
                                let sensor = &mut self.engine.sensors[i];
                                sensor.q = q;
                                sensor.seen = t;
                                sensor.angular = 0.;
                            } else {
                                let was_armed = self.engine.armed;
                                let detail = if was_armed && !self.engine.fresh(t) {
                                    Some(self.readiness_detail(t, "rotation_update"))
                                } else {
                                    None
                                };
                                self.engine.update(i, q, t)?;
                                if was_armed && !self.engine.armed {
                                    self.actions.reset_transient();
                                    self.action_preview = Default::default();
                                    self.gestures.reset();
                                    self.gesture_preview = Default::default();
                                    self.stop_reason =
                                        "Tracker signal or device heartbeat expired".into();
                                    self.stop_detail = detail.unwrap_or(Value::Null);
                                }
                            }
                        }
                    }
                }
                if let Some(f) = self.recording.as_mut() {
                    if writeln!(f, "{}", json!({"t":t,"id":id,"q":q})).is_err() {
                        self.error = "Recording write failed".into();
                        self.recording = None;
                    }
                }
            }
        }
        Ok(replies)
    }
    pub fn tick(&mut self, t: f64, wall: f64) {
        self.update_sensor_calibration(t);
        if let Some(mut learning)=self.crouch_learning.take() {
            match learning.update(self.action_input(t)) {
                Ok(None)=>self.crouch_learning=Some(learning),
                Ok(Some(crouch))=>{
                    let mut settings=self.actions.settings().to_json();
                    if let Some(fields)=crouch.as_object() {for (key,value) in fields {settings["crouch"][key]=value.clone();}}
                    match self.command(&json!({"command":"actions_settings","settings":settings}),t) {
                        Ok(())=>self.crouch_learning_result=json!({"active":false,"status":"saved","message":"Crouch learned, applied and saved."}),
                        Err(e)=>self.crouch_learning_result=json!({"active":false,"status":"error","message":format!("Could not save crouch settings: {e}")}),
                    }
                },
                Err(e)=>self.crouch_learning_result=json!({"active":false,"status":"error","message":e}),
            }
        }
        if let Some(mut learning)=self.jump_learning.take() {
            match learning.update(self.action_input(t)) {
                Ok(None)=>self.jump_learning=Some(learning),
                Ok(Some(jump))=>{
                    let mut settings=self.actions.settings().to_json();
                    if let Some(fields)=jump.as_object() {for (key,value) in fields {settings["jump"][key]=value.clone();}}
                    match self.command(&json!({"command":"actions_settings","settings":settings}),t) {
                        Ok(())=>self.jump_learning_result=json!({"active":false,"status":"saved","message":"Jump learned and saved."}),
                        Err(e)=>self.jump_learning_result=json!({"active":false,"status":"error","message":format!("Could not save jump settings: {e}")}),
                    }
                },
                Err(e)=>self.jump_learning_result=json!({"active":false,"status":"error","message":e}),
            }
        }
        if self.demo {
            for (i, phase) in [(0, 0.), (1, std::f64::consts::PI)] {
                let a = if self.walking {
                    0.7 * (t * std::f64::consts::TAU * 1.2 + phase).sin().max(0.)
                } else {
                    0.
                };
                let _ = self
                    .engine
                    .update(i, [(a / 2.).cos(), (a / 2.).sin(), 0., 0.], t);
                let _ = self.engine.update_acceleration(i, [a * 12., 0., 0.], t);
            }
            let a = self.demo_yaw / 2.;
            let q = if self.engine.cfg.y_up {
                [a.cos(), 0., a.sin(), 0.]
            } else {
                [a.cos(), 0., 0., a.sin()]
            };
            let _ = self.engine.update(2, q, t);
        }
        self.refresh_held_poses(t);
        if self.output_mode == "steamvr" && self.was_driver_ready && !self.driver_ready(t) {
            self.steamvr_lost(t);
        }
        // The preview also needs a valid centre. Previously only armed output
        // stopped on loss, leaving Test Room calibrated across a tracker reboot.
        if self.engine.calibrated && self.validated_rotation(2,t).is_none() {
            self.stop_detail = self.readiness_detail(t,"tick");
            self.engine.disarm();self.actions.reset_transient();self.action_preview=Default::default();self.walking_signal=Default::default();self.walking_speed=0.;self.walking_effort=0.;
            self.stop_reason = "Chest signal lost; center preserved. Wait for tracker and start movement.".into();
        }
        let was_armed = self.engine.armed;
        let detail = if was_armed && !self.engine.fresh(t) {
            Some(self.readiness_detail(t, "tick"))
        } else {
            None
        };
        self.engine.tick(t);
        if was_armed && !self.engine.armed {
            self.actions.reset_transient();
            self.action_preview = Default::default();
            self.gestures.reset();
            self.gesture_preview = Default::default();
            self.stop_reason = "Tracker signal or device heartbeat expired".into();
            self.stop_detail = detail.unwrap_or(Value::Null);
        }
        if self.engine.armed&&!self.thighs_ready(t){self.engine.disarm();self.stop_reason="Thigh tracker signal expired".into();}
        let input = self.action_input(t);
        self.actions.set_walking_forward_gain(if self.engine.cfg.chest_steps {Some(self.walking_speed)}else{None});
        self.actions.set_walking_sway(self.engine.cfg.chest_steps && self.walking_model.is_object() && self.walking_speed>0.);
        self.actions.set_world_up(if self.engine.cfg.y_up {[0.,1.,0.]} else {[0.,0.,1.]});
        self.action_preview = if !self.engine.armed && !crate::actions::ActionProcessor::has_fresh_head(&input) {
            self.actions.process_chest_preview(&input)
        } else {
            self.actions.process(&input)
        };
        self.gate_mounting_preview(t);
        self.walking_speed=0.;self.walking_effort=0.;
        let identity=self.current_mounting_identity(t);
        let angles=self.action_preview.raw_direction_angles;
        let seen=self.engine.sensors[2].seen;
        let forward=self.actions.settings().direction.forward_deadzone.unwrap_or(self.actions.settings().direction.deadzone).max(0.05);
        if let Some(mut learning)=self.walking_learning.take() {
            self.engine.disarm();
            let result=if identity.as_deref()!=Some(learning.identity.as_str()) {Err("Tracker changed or disconnected. Nothing saved.".into())}else if let Some(angles)=angles {learning.update(t,seen,angles,forward)}else{Err("Reset Center and keep the chest sensor connected. Nothing saved.".into())};
            match result {Ok(None)=>self.walking_learning=Some(learning),Ok(Some(model))=>{let previous=self.walking_model.clone();self.walking_model=model;if let Err(e)=self.save(&self.mapping,&self.engine.cfg,&self.output_mode){self.walking_model=previous;self.walking_result=json!({"active":false,"message":e});}else{self.walking_result=json!({"active":false,"message":"Walking learned and saved. Lean forward and walk in place to test."});}},Err(e)=>self.walking_result=json!({"active":false,"message":e})}
        }
        if self.engine.cfg.chest_steps && self.walking_model["identity"].as_str()==identity.as_deref() && identity.is_some() && self.walking_learning.is_none() {
            if let Some(a)=angles.filter(|_|t>=seen && t-seen<0.25) {self.walking_signal.add(seen,a[0]);let(rate,turns)=self.walking_signal.measure();if turns>=2 {if let Some(learned)=self.walking_model["rate"].as_f64().filter(|r|*r>0.02){self.walking_effort=(rate/(learned*self.walking_model["full_speed_effort"].as_f64().unwrap_or(0.5).clamp(0.1,2.))).max(0.);self.walking_speed=self.walking_effort.min(1.);}}}else{self.walking_signal=Default::default();}
        }

        self.gesture_preview = Default::default();
        let is_tilt = self.movement_engine != "chest_heading_steps";
        let direction_ready = !is_tilt || self.action_preview.direction_status == "ready";
        let speed = if self.movement_engine == "body_tilt" || self.engine.cfg.chest_steps {
            1.
        } else {
            if self.engine.cfg.chest_steps && self.walking_model.is_object() {self.walking_speed}else{self.engine.speed}
        };
        self.travel_xy = if !direction_ready {
            [0.; 2]
        } else if is_tilt {
            self.action_preview
                .travel_xy
                .map(|x| x * speed * self.action_preview.speed_multiplier)
        } else {
            [0., speed * self.action_preview.speed_multiplier]
        };
        if self.engine.cfg.chest_steps && self.walking_model.is_object() {

            if self.walking_learning.is_some(){self.travel_xy=[0.;2];}
        }
        self.turn_xy = self.action_preview.turn_xy;
        self.output_buttons = ((self.action_preview.jump_pressed || self.gesture_preview.jump)
            as u32)
            | (((self.action_preview.crouch_pressed || self.gesture_preview.crouch) as u32) << 1)
            | (((self.engine.preview.sprint || self.gesture_preview.sprint || self.action_preview.sprint_pressed || (self.engine.cfg.chest_steps && self.actions.settings().direction.sprint_full_forward && self.walking_effort>=1.2 && self.walking_speed>0.)) as u32) << 2);
        if self.engine.cfg.chest_steps && self.walking_speed<=0. {self.output_buttons &= !4;}
        if self.engine.armed && !direction_ready {
            self.travel_xy = [0.; 2];
        }
        let dt = if t >= self.stats_tick {
            (t - self.stats_tick).min(0.1)
        } else {
            0.
        };
        self.stats_tick = t;
        if self.engine.armed && self.engine.calibrated && self.engine.input_ready(t) && !self.demo {
            let norm = (self.travel_xy[0] * self.travel_xy[0]
                + self.travel_xy[1] * self.travel_xy[1])
                .sqrt()
                .clamp(0., 1.);
            if norm > 0.001 {
                self.moving_seconds += dt;
                self.distance_m += norm * dt;
                self.peak_speed = self.peak_speed.max(norm);
                self.statistics_history.add(wall, dt, norm * dt, norm);
            }
        }
        self.statistics_history.persist_if_due(
            &self
                .config
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join("stats.json"),
            wall,
        );
        self.heading = self.engine.heading;
        self.packet = encode_motion(
            if direction_ready { self.engine.speed as f32 } else { 0. },
            self.heading as f32,
            self.engine.fresh(t),
            self.engine.armed,
            wall,
            self.seq,
        )
        .unwrap();
        self.seq = self.seq.wrapping_add(1);
        let old: Vec<_> = self
            .peers
            .iter()
            .filter(|(_, last)| t - **last > 10.)
            .map(|(a, _)| *a)
            .collect();
        for a in old {
            self.peers.remove(&a);
            self.counters.remove(&a);
        }
        self.records.retain(|id, r| {
            t - r.seen < 600.
                || self
                    .accelerations
                    .get(id)
                    .is_some_and(|a| t - a.seen < 600.)
        });
        self.accelerations.retain(|_, a| t - a.seen < 600.);
    }
    pub fn driver_ready(&self, t: f64) -> bool {
        t >= self.ack_seen && t - self.ack_seen <= 0.25 && self.ack_flags & 7 == 7
    }
    fn steamvr_lost(&mut self,t:f64) {
        let a=self.actions.settings();
        let head_dependent=(a.direction.enabled && a.direction.decouple) || a.turn.enabled || a.crouch.enabled || (a.jump.enabled && a.jump.source=="height") || (self.gestures.settings().enabled && self.gestures.settings().mappings.iter().any(|m|m.source=="hmd"));
        let preserve=self.movement_engine=="body_tilt" && !head_dependent && self.engine.calibrated && self.engine.fresh(t) && self.validated_rotation(2,t).is_some();
        self.stop_detail=self.readiness_detail(t,"steamvr_lost");
        self.engine.disarm();self.actions.reset_transient();self.action_preview=Default::default();self.gestures.reset();self.gesture_preview=Default::default();
        if preserve {self.stop_reason="SteamVR interrupted; movement stopped. When connected, press Start Movement.".into();}
        else {self.engine.calibrated=false;self.actions.reset();self.stop_reason="SteamVR driver or HMD lost; recalibration required".into();}
        self.was_driver_ready=false;
    }
    pub fn steam_ack(&mut self, data: &[u8], addr: SocketAddr, t: f64) {
        let v2 = data.len() == 32 && &data[..4] == b"AKV2";
        let v1 = data.len() == 16 && &data[..4] == b"ALAK";
        if addr != SocketAddr::from(([127, 0, 0, 1], self.steam_port))
            || !t.is_finite()
            || !(v1 || v2)
        {
            return;
        }
        let seq = u32::from_le_bytes(data[4..8].try_into().unwrap());
        let generation = u32::from_le_bytes(data[8..12].try_into().unwrap());
        let mut flags = u32::from_le_bytes(data[12..16].try_into().unwrap());
        let age = self.steam_sequence.wrapping_sub(seq);
        if generation != self.generation || age == 0 || age > 64 {
            return;
        }
        if v2 {
            if self
                .v2_sequence
                .is_some_and(|last| (seq.wrapping_sub(last) as i32) <= 0)
            {
                return;
            }
            if self
                .ack_sequence
                .is_some_and(|last| (seq.wrapping_sub(last) as i32) < 0)
            {
                return;
            }
            let f =
                |offset| f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as f64;
            let (height, yaw, pitch, roll) = (f(16), f(20), f(24), f(28));
            if [height, yaw, pitch, roll].iter().any(|v| !v.is_finite())
                || !(0.05..=4.).contains(&height)
            {
                flags &= !2;
                self.hmd = None;
            } else if flags & 2 != 0 {
                self.hmd = Some(crate::actions::TimedHmd {
                    height,
                    yaw,
                    pitch,
                    roll,
                    seen: t,
                });
            } else {
                self.hmd = None;
            }
            self.protocol_v2 = true;
            self.v2_sequence = Some(seq);
        } else if self
            .ack_sequence
            .is_some_and(|last| (seq.wrapping_sub(last) as i32) <= 0)
        {
            return;
        }
        if self.was_driver_ready && !self.driver_ready(t) { self.steamvr_lost(t); }
        self.ack_sequence = Some(seq);
        self.ack_seen = t;
        self.steam_retry_since = Some(t);
        self.ack_flags = flags;
        if self.driver_ready(t) {
            self.was_driver_ready = true;
        } else if self.was_driver_ready {
            self.steamvr_lost(t);
        }
    }
    pub fn steam_packet(&mut self, t: f64) -> Vec<u8> {
        if self.output_mode == "steamvr" && !self.demo && t.is_finite() {
            let since = *self.steam_retry_since.get_or_insert(t);
            if t >= since && t - since >= 1. {
                // An expired driver session rejects the same generation indefinitely.
                // Retry with a neutral new generation, never restoring armed output.
                if self.engine.armed { self.steamvr_lost(t); }
                self.new_generation();
                self.steam_retry_since = Some(t);
            }
        }
        if self.engine.armed&&!self.thighs_ready(t){self.engine.disarm();self.stop_reason="Thigh tracker signal expired".into();}
        let input = self.action_input(t);
        self.actions.set_walking_forward_gain(if self.engine.cfg.chest_steps {Some(self.walking_speed)}else{None});
        self.actions.set_walking_sway(self.engine.cfg.chest_steps && self.walking_model.is_object() && self.walking_speed>0.);
        self.actions.set_world_up(if self.engine.cfg.y_up {[0.,1.,0.]} else {[0.,0.,1.]});
        self.action_preview = if !self.engine.armed && !crate::actions::ActionProcessor::has_fresh_head(&input) {
            self.actions.process_chest_preview(&input)
        } else {
            self.actions.process(&input)
        };
        self.gate_mounting_preview(t);
        self.gesture_preview = Default::default();
        let is_tilt = self.movement_engine != "chest_heading_steps";
        let scale = if self.movement_engine == "body_tilt" || self.engine.cfg.chest_steps {
            1.
        } else {
            if self.engine.cfg.chest_steps && self.walking_model.is_object() {self.walking_speed}else{self.engine.speed}
        };
        self.travel_xy = if is_tilt && self.action_preview.direction_status != "ready" {
            [0.; 2]
        } else if is_tilt {
            self.action_preview
                .travel_xy
                .map(|x| x * scale * self.action_preview.speed_multiplier)
        } else {
            [0., scale * self.action_preview.speed_multiplier]
        };
        if self.engine.cfg.chest_steps && self.walking_model.is_object() {
            if let Some([x,y])=self.action_preview.raw_direction_angles.filter(|a|a[1].abs()>0.05) {
                // Remove the learned lateral gait component before head-relative rotation.
                // Reuse the forward direction's rotation rather than forcing head-local X to zero.
                let d=self.actions.settings().direction.clone();
                if d.decouple {if let (Some(head),Some(chest))=(self.hmd,self.action_preview.chest_yaw) {
                    let _=(head,chest,x,y); // Direction is handled by the processor's gait suppression below.
                }}
            }
            if self.walking_learning.is_some(){self.travel_xy=[0.;2];}
        }
        self.turn_xy = self.action_preview.turn_xy;
        self.output_buttons = ((self.action_preview.jump_pressed || self.gesture_preview.jump)
            as u32)
            | (((self.action_preview.crouch_pressed || self.gesture_preview.crouch) as u32) << 1)
            | (((self.engine.preview.sprint || self.gesture_preview.sprint || self.action_preview.sprint_pressed || (self.engine.cfg.chest_steps && self.actions.settings().direction.sprint_full_forward && self.walking_effort>=1.2 && self.walking_speed>0.)) as u32) << 2);
        if self.engine.cfg.chest_steps && self.walking_speed<=0. {self.output_buttons &= !4;}
        let active = self.output_mode == "steamvr"
            && !self.demo
            && self.engine.armed
            && self.engine.calibrated
            && self.engine.input_ready(t)
            && !self.neutral_pending
            && self.driver_ready(t);
        self.steam_speed = if active && (!is_tilt || self.mounting_valid(t)) {
            self.engine.speed.clamp(0., 1.) as f32
        } else {
            0.
        };
        self.steam_yaw = if active { self.heading as f32 } else { 0. };
        let seq = self.steam_sequence;
        self.steam_sequence = self.steam_sequence.wrapping_add(1);
        if !self.protocol_v2 {
            let mut p = vec![0; 20];
            p[..4].copy_from_slice(b"ALOC");
            p[4..8].copy_from_slice(&seq.to_le_bytes());
            p[8..12].copy_from_slice(&self.generation.to_le_bytes());
            p[12..16].copy_from_slice(&self.steam_speed.to_le_bytes());
            p[16..20].copy_from_slice(&self.steam_yaw.to_le_bytes());
            return p;
        }
        let mut p = vec![0; 40];
        p[..4].copy_from_slice(b"ALC2");
        p[4..8].copy_from_slice(&seq.to_le_bytes());
        p[8..12].copy_from_slice(&self.generation.to_le_bytes());
        if active {
            for (i, v) in self.travel_xy.iter().chain(self.turn_xy.iter()).enumerate() {
                p[12 + i * 4..16 + i * 4].copy_from_slice(&(*v as f32).to_le_bytes());
            }
            p[28..32].copy_from_slice(&self.output_buttons.to_le_bytes());
            let flags = if self.movement_engine == "chest_heading_steps" {
                0u32
            } else {
                1u32
            };
            p[32..36].copy_from_slice(&flags.to_le_bytes());
            p[36..40].copy_from_slice(&self.steam_yaw.to_le_bytes());
        }
        p
    }
    pub fn steam_sent(&mut self, result: std::io::Result<usize>) {
        match result {
            Ok(n) if n == if self.protocol_v2 { 40 } else { 20 } => {
                self.steam_packets_sent += 1;
                self.steam_error.clear();
                self.neutral_pending = false;
            }
            Ok(_) => {
                self.neutral_pending = true;
                self.steam_error = "Incomplete SteamVR datagram".into();
            }
            Err(e) => {
                self.neutral_pending = true;
                self.steam_error = e.to_string();
            }
        }
    }
    pub fn snapshot(&self, t: f64) -> Value {
        let hmd = self.hmd.filter(|h| t.is_finite() && h.seen.is_finite() && t>=h.seen && t-h.seen<=0.25 && h.height.is_finite() && (0.05..=4.).contains(&h.height));
        let mut roles = serde_json::Map::new();
        for (i, r) in ROLES.iter().enumerate() {
            let s = &self.engine.sensors[i];
            let accel = self.accelerations.get(&self.mapping[i]);
            roles.insert(r.to_string(),json!({"id":if self.demo{format!("synthetic:{r}")}else{self.mapping[i].clone()},"mac":self.tracker_identities.get(&self.mapping[i]),"fresh":t>=s.seen&&t-s.seen<self.engine.cfg.stale,"age_ms":((t-s.seen)*1000.).clamp(0.,999999.),"tilt_speed":s.angular,"quaternion":s.q,
                "acceleration":accel.map(|a| a.vector), "acceleration_magnitude":accel.map(|a| a.vector.iter().map(|x| x*x).sum::<f64>().sqrt()),
                "acceleration_age_ms":accel.map(|a| ((t-a.seen)*1000.).max(0.)), "preview_steps":if i<2 {self.engine.preview_steps[i]} else {0},
                "acceleration_mode":if i<2 {self.engine.accel_mode[i]} else {"not applicable"}, "accel_source":if i>=2 {"not applicable"} else if accel.is_some_and(|a| t>=a.seen && t-a.seen<self.engine.cfg.stale) {"acceleration"} else {"tilt"}, "acceleration_threshold":1.2, "acceleration_units":"m/s^2"}));
        }
        let mut trackers: Vec<_> = self
            .records
            .iter()
            .map(|(id, r)| {
                let ip = id.rsplit_once(':').map(|(ip, _)| ip).unwrap_or("");
                let peer_seen = self
                    .peers
                    .iter()
                    .filter(|(addr, _)| addr.ip().to_string() == ip)
                    .map(|(_, seen)| *seen)
                    .fold(None::<f64>, |a, b| Some(a.map_or(b, |v| v.max(b))));
                let age = (t - r.seen).max(0.);
                let acceleration = self.accelerations.get(id);
                json!({"id":id,"mac":self.peer_macs.get(ip),"age_ms":age*1000.,"q":r.q,
                    "rotation_available":r.seen >= 0., "has_interval":r.has_interval,
                    "acceleration":acceleration.map(|a| a.vector),
                    "acceleration_magnitude":acceleration.map(|a| a.vector.iter().map(|x| x*x).sum::<f64>().sqrt()),
                    "acceleration_age_ms":acceleration.map(|a| ((t-a.seen)*1000.).max(0.)),
                    "acceleration_count":acceleration.map_or(0, |a| a.count),
                    "fresh":t>=r.seen && age<self.engine.cfg.stale,
                    "angular_speed":if age<self.engine.cfg.stale {r.angular} else {0.},
                    "peer_age_ms":peer_seen.map(|seen| ((t-seen)*1000.).max(0.)),
                    "peer_active":peer_seen.is_some_and(|seen| t>=seen && t-seen<=2.),
                    "sensor_status":self.sensor_status.get(id),
                    "sample_mode":if age<self.engine.cfg.stale {"streaming"} else if self.sensor_status.get(id)==Some(&1) && r.has_interval && peer_seen.is_some_and(|seen| t>=seen && t-seen<=2.) {"held"} else {"stale"}})
            })
            .collect();
        trackers.sort_by_key(|v| v["id"].as_str().unwrap().to_string());
        let preview = &self.engine.preview;
        let locomotion = json!({"algorithm":if self.engine.cfg.continuous {"continuous"} else {"alternating"},
            "settings":self.engine.cfg.motion.to_json(), "input_ready":self.engine.input_ready(t)&&self.thighs_ready(t),
            "diagnostic":{"source":"chest","status":self.engine.diagnostic.status,"speed":self.engine.diagnostic.speed,"raw_speed":self.engine.diagnostic.raw_speed,"signal":self.engine.diagnostic.signal,"fast":self.engine.diagnostic.fast,"slow":self.engine.diagnostic.slow,"adaptive":self.engine.diagnostic.adaptive,"inertia":self.engine.diagnostic.inertia,"sprint":self.engine.diagnostic.sprint},
            "preview":{"raw_speed":preview.raw_speed,"speed":preview.speed,"sprint":preview.sprint,"signal":preview.signal,"fast":preview.fast,"slow":preview.slow,
                "adaptive":preview.adaptive,"inertia":preview.inertia,"status":preview.status}});
        let preview = self.action_preview;
        let actions = json!({"settings":self.actions.settings().to_json(),"preview":{"travel_xy":preview.travel_xy,"training_xy":preview.training_xy,"raw_direction_angles":preview.raw_direction_angles,"chest_yaw":preview.chest_yaw,"sprint_pressed":preview.sprint_pressed,"jump_pressed":preview.jump_pressed,"crouch_pressed":preview.crouch_pressed,"crouching":preview.crouching,"speed_multiplier":preview.speed_multiplier,"turn_xy":preview.turn_xy,"calibrated":preview.calibrated,"direction_status":preview.direction_status,"jump_status":preview.jump_status,"crouch_status":preview.crouch_status,"turn_status":preview.turn_status}});
        let mut result=json!({"sensor_layout":{"count":self.layout_count(),"left_thigh":self.thighs[0],"right_thigh":self.thighs[1],"thighs_ready":self.thighs_ready(t)},"sensor_peers":self.peers.iter().map(|(address,seen)|json!({"address":address.to_string(),"age_ms":(t-seen)*1000.})).collect::<Vec<_>>(),"gestures":{"settings":self.gestures.settings().to_json(),"preview":{"jump":self.gesture_preview.jump,"crouch":self.gesture_preview.crouch,"sprint":self.gesture_preview.sprint,"statuses":self.gesture_preview.statuses}},"movement_engine":self.movement_engine,"actions":actions,"profiles":self.profile_list,"statistics":{"elapsed_seconds":t.max(0.),"moving_seconds":self.moving_seconds,"estimated_distance_m":self.distance_m,"average_speed_mps":if self.moving_seconds>0. {self.distance_m/self.moving_seconds}else{0.},"peak_speed":self.peak_speed,"steps_left":self.engine.preview_steps[0],"steps_right":self.engine.preview_steps[1],"history":self.statistics_history.to_json(&crate::statistics::utc_day(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64()))},"locomotion":locomotion,"acceleration_input_mode":if self.engine.cfg.accel_has_gravity {"gravity-baseline"} else {"linear"},"preview_steps":{"left":self.engine.preview_steps[0],"right":self.engine.preview_steps[1]},"acceleration_threshold":1.2,"output_mode":self.output_mode,"output_status":if self.output_mode == "steamvr" {if self.driver_ready(t) {"SteamVR driver and HMD ready"} else {"Waiting for SteamVR driver and HMD"}} else {"Meta Nexus loopback mock; no headset injection"},"output_error":self.steam_error,"calibration_ready":self.engine.fresh(t)&&self.thighs_ready(t),"calibration_error":if !self.thighs_ready(t){"Waiting for both thigh sensors (5-sensor layout)"}else if self.engine.fresh(t) {""} else if self.mapping[2].rsplit_once(':').and_then(|(ip,_)|ip.parse().ok()).is_some_and(|ip|self.needs_identity_handshake(ip)) {"Reconnecting chest tracker: waiting for its identity handshake"} else if !self.engine.cfg.require_steps {"Waiting for assigned CHEST tracker. This mode does not require ankle trackers."} else {"Need fresh samples from both ankles and torso"},"steamvr":{"protocol":if self.protocol_v2 {2} else {1},"driver_connected":t >= self.ack_seen && t-self.ack_seen <= 0.25 && self.ack_flags & 1 != 0,"hmd_ready":t >= self.ack_seen && t-self.ack_seen <= 0.25 && self.ack_flags & 2 != 0,"driver_calibrated":t >= self.ack_seen && t-self.ack_seen <= 0.25 && self.ack_flags & 4 != 0,"sequence":self.steam_sequence.wrapping_sub(1),"generation":self.generation,"speed":self.steam_speed,"yaw":self.steam_yaw,"packets_sent":self.steam_packets_sent,"send_error":self.steam_error},"mode":if self.demo{"demo"}else{"live"},"trackers":trackers,"roles":roles,"armed":self.engine.armed,"calibrated":self.engine.calibrated,"speed":self.engine.speed,"heading":self.heading,"steps":self.engine.steps.len(),"packet_bytes":261,"packet_hex":self.packet.iter().map(|b|format!("{b:02x}")).collect::<String>(),"packet":{"sequence":self.seq.saturating_sub(1),"rotation_xyzw":[0.,(self.heading/2.).sin(),0.,(self.heading/2.).cos()],"velocity_xyz":[0.,0.,-self.engine.speed]},"received":self.received,"rejected":self.rejected,"error":self.error,"stop_reason":self.stop_reason,"stop_detail":self.stop_detail,"recording":if self.recording.is_some(){self.record_path.as_ref().map(|p|p.display().to_string())}else{None},"output":if self.output_mode == "steamvr" {"Experimental SteamVR loopback output"} else {"Meta Nexus loopback mock; no headset injection"},"backend":"Rust","axis":if self.engine.cfg.y_up{"y"}else{"z"},"threshold":self.engine.cfg.threshold});
        result["game_output_preview"]=json!({"travel_xy":self.travel_xy.map(|v|v as f32),"turn_xy":self.turn_xy.map(|v|v as f32),"buttons":self.output_buttons,"armed":self.engine.armed});
        result["step_input"]=json!(if self.engine.cfg.chest_steps {"chest"} else if self.movement_engine=="body_tilt" {"off"} else {"feet"});
        result["sensor_calibration"] = self.sensor_calibration_state(t);
        result["crouch_learning"] = self.crouch_learning.as_ref().map(|s|s.state(t)).unwrap_or_else(||self.crouch_learning_result.clone());
        result["walking_learning"]=self.walking_learning.as_ref().map(|s|s.state(t)).unwrap_or_else(||self.walking_result.clone());
        result["walking_learned"]=json!(self.walking_model["rate"].as_f64().is_some_and(|r|r.is_finite() && r>0.02) && self.current_mounting_identity(t).is_some_and(|id|self.walking_model["identity"].as_str()==Some(id.as_str())));
        result["walking_model"]=self.walking_model.clone();
        result["walking_speed"]=json!(self.walking_speed);
        result["walking_effort"]=json!(self.walking_effort);
        result["jump_learning"] = self.jump_learning.as_ref().map(|s|s.state(t)).unwrap_or_else(||self.jump_learning_result.clone());
        result["device_calibrations"]=json!(self.device_mountings);
        result["mounting_calibration"] = json!({"valid":self.mounting_valid(t),"tracker_identity":self.current_mounting_identity(t),"learned_identity":self.learned_mounting_identity,"mapping":self.learned_mounting_identity.as_ref().and_then(|id|self.device_mountings.get(id)),"message":if self.mounting_valid(t) {""} else {"Sensor calibration required for this chest tracker"}});
        result["game_profiles"] = self.game_profiles.clone();
        result["steamvr"]["ack_age_ms"] = if self.ack_seen>=0. && t>=self.ack_seen {json!((t-self.ack_seen)*1000.)}else{Value::Null};
        result["steamvr"]["neutral_handshake_pending"] = json!(self.neutral_pending);
        result["hmd_pose"]=json!({"fresh":hmd.is_some(),"height":hmd.map(|h|h.height),"sample_time":hmd.map(|h|h.seen),"sequence":if hmd.is_some(){self.v2_sequence}else{None}});
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restart_releases_unknown_saved_tracker_until_identity_arrives() {
        let mut l=Lab::new(PathBuf::from("nonexistent-handshake-recovery-fixture.json")).unwrap();
        let ip="192.0.2.12".parse().unwrap();
        l.mapping[2]="192.0.2.12:0".into();
        assert!(!l.needs_identity_handshake(ip), "legacy unbound trackers remain supported");
        l.tracker_identities.insert(l.mapping[2].clone(),"02:00:00:00:00:01".into());
        assert!(l.needs_identity_handshake(ip));
        l.peer_macs.insert("192.0.2.12".into(),"02:00:00:00:00:01".into());
        assert!(!l.needs_identity_handshake(ip));assert!(l.identity_matches(&l.mapping[2]));
        l.peer_macs.insert("192.0.2.12".into(),"02:00:00:80:BE:03".into());
        assert!(!l.identity_matches(&l.mapping[2]), "different hardware must not inherit the role");
        assert!(!l.engine.armed);
    }
    #[test]
    fn profiles_center_and_restart_preserve_corrected_physical_direction() {
        let dir = std::env::temp_dir().join(format!("direction-persistence-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let mut l = Lab::new(path.clone()).unwrap();
        l.movement_engine = "body_tilt".into();
        l.engine.cfg.require_steps = false;
        l.mapping[2] = "10.0.0.94:0".into();
        l.learned_mounting_identity = Some("02:00:00:80:BE:03:0".into());
        let mut settings = l.actions.settings().clone();
        settings.direction.enabled = true;
        settings.direction.pitch_sign = -1.;
        settings.direction.roll_sign = -1.;
        settings.direction.learned_forward = Some([1.,0.,0.]);
        settings.direction.learned_right = Some([0.,-1.,0.]);
        settings.direction.deadzone = 0.;
        settings.direction.smoothing_ms = 0.;
        l.actions = crate::actions::ActionProcessor::new(settings).unwrap();
        let mut opposite = l.current_profile();
        opposite["actions"]["direction"]["pitch_sign"] = json!(1.);
        opposite["actions"]["direction"]["roll_sign"] = json!(1.);
        opposite["actions"]["direction"]["learned_forward"] = json!([0.,0.,1.]);
        opposite["actions"]["direction"]["learned_right"] = json!([1.,0.,0.]);
        opposite["actions"]["direction"]["forward_max_angle"] = json!(0.5);
        l.profile_store().save("Old reversed profile", "regression fixture", opposite).unwrap();
        l.command(&json!({"command":"profile_load","name":"Old reversed profile"}), 1.).unwrap();
        assert_eq!(l.actions.settings().direction.forward_max_angle, Some(0.5));
        fn pose(l: &mut Lab, q: Q, t: f64) {
            l.peer_macs.insert("10.0.0.94".into(), "02:00:00:80:BE:03".into());
            l.records.insert(l.mapping[2].clone(), Record {q,seen:t,angular:0.,has_interval:true});
            l.engine.update(2,q,t).unwrap();
        }
        fn mul(a: Q, b: Q) -> Q {
            let [w,x,y,z]=a;let [v,i,j,k]=b;
            [w*v-x*i-y*j-z*k,w*i+x*v+y*k-z*j,w*j-x*k+y*v+z*i,w*k+x*j-y*i+z*v]
        }
        for round in 0..3 {
            let t = 2. + round as f64;
            if round > 0 {
                l = Lab::new(path.clone()).unwrap();
                // A physical tracker reboot starts a new stream/zero heading;
                // retain MAC-bound axes while invalidating the old center.
                l.ingest(&identity_packet([0x02,0x00,0x00,0x80,0xbe,0x03]), "10.0.0.94:6969".parse().unwrap(),t-0.01).unwrap();
                assert!(!l.engine.calibrated);
            }
            assert_eq!(l.actions.settings().direction.pitch_sign, -1.);
            assert_eq!(l.actions.settings().direction.roll_sign, -1.);
            assert_eq!(l.actions.settings().direction.learned_forward, Some([1.,0.,0.]));
            assert_eq!(l.actions.settings().direction.learned_right, Some([0.,-1.,0.]));
            let angle=round as f64*0.6;
            let center=[angle.cos(),0.,0.,angle.sin()];
            pose(&mut l, center, t);
            l.command(&json!({"command":"calibrate"}), t).unwrap();
            pose(&mut l, mul(center,[0.15f64.cos(),-0.15f64.sin(),0.,0.]), t+0.02);
            l.tick(t+0.02,0.);
            assert!(l.action_preview.travel_xy[1] > 0.1, "forward must stay forward after profile/center/restart");
            pose(&mut l, mul(center,[0.15f64.cos(),0.15f64.sin(),0.,0.]), t+0.04);
            l.tick(t+0.04,0.);
            assert!(l.action_preview.travel_xy[1] < -0.1, "backward must stay backward");
            pose(&mut l, mul(center,[0.15f64.cos(),0.,0.15f64.sin(),0.]), t+0.06);
            l.tick(t+0.06,0.);
            assert!(l.action_preview.travel_xy[0] > 0.1, "right mapping must remain unchanged");
            pose(&mut l, mul(center,[0.15f64.cos(),0.,-0.15f64.sin(),0.]), t+0.08);
            l.tick(t+0.08,0.);
            assert!(l.action_preview.travel_xy[0] < -0.1, "left must stay left");
            pose(&mut l,center,t+0.1);l.tick(t+0.1,0.);
            assert_eq!(l.action_preview.travel_xy,[0.;2]);
            l.tick(t+0.6,0.);
            assert_eq!(l.action_preview.travel_xy,[0.;2]);
            assert!(!l.engine.armed);
        }
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn device_mapping_flips_persist_and_reset_without_touching_assignment() {
        let dir=std::env::temp_dir().join(format!("device-flip-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));fs::create_dir_all(&dir).unwrap();let path=dir.join("config.json");
        let mut lab=Lab::new(path.clone()).unwrap();lab.movement_engine="body_tilt".into();lab.engine.cfg.require_steps=false;lab.mapping[2]="10.0.0.3:0".into();let identity="02:00:00:80:BE:03:0";
        lab.peer_macs.insert("10.0.0.3".into(),"02:00:00:80:BE:03".into());lab.records.insert(lab.mapping[2].clone(),Record{q:crate::gait::IDENTITY,seen:1.,angular:0.,has_interval:true});lab.engine.update(2,crate::gait::IDENTITY,1.).unwrap();
        lab.learned_mounting_identity=Some(identity.into());let mut s=lab.actions.settings().clone();s.direction.enabled=true;s.direction.learned_forward=Some([1.,0.,0.]);s.direction.learned_right=Some([0.,0.,1.]);lab.actions=crate::actions::ActionProcessor::new(s).unwrap();lab.remember_device_mounting(identity);
        lab.command(&json!({"command":"device_mounting","identity":identity,"operation":"flip_forward"}),1.).unwrap();lab.command(&json!({"command":"device_mounting","identity":identity,"operation":"flip_lateral"}),1.).unwrap();assert_eq!(lab.actions.settings().direction.pitch_sign,-1.);assert_eq!(lab.actions.settings().direction.roll_sign,-1.);assert!(!lab.engine.armed);
        let mut stale=lab.actions.settings().to_json();stale["direction"]["pitch_sign"]=json!(1.);stale["direction"]["roll_sign"]=json!(1.);
        stale["direction"]["forward_offset"]=json!(0.01);
        lab.command(&json!({"command":"actions_settings","settings":stale}),1.).unwrap();
        assert_eq!(lab.actions.settings().direction.pitch_sign,-1.,"stale draft cannot overwrite forward flip");
        assert_eq!(lab.actions.settings().direction.roll_sign,-1.,"stale draft cannot overwrite lateral flip");
        assert_eq!(lab.actions.settings().direction.forward_offset,0.01,"ordinary tuning still applies");
        lab.command(&json!({"command":"device_mounting","identity":identity,"operation":"turn_over"}),1.).unwrap();
        assert_eq!(lab.actions.settings().direction.pitch_sign,1.);assert_eq!(lab.actions.settings().direction.roll_sign,1.);assert!(!lab.engine.calibrated);
        assert_eq!(lab.device_mountings[identity]["face_turned_over"],true);
        drop(lab);let mut lab=Lab::new(path.clone()).unwrap();assert_eq!(lab.device_mountings[identity]["face_turned_over"],true);
        lab.peer_macs.insert("10.0.0.3".into(),"02:00:00:80:BE:03".into());lab.records.insert(lab.mapping[2].clone(),Record{q:crate::gait::IDENTITY,seen:1.,angular:0.,has_interval:true});lab.engine.update(2,crate::gait::IDENTITY,1.).unwrap();
        lab.command(&json!({"command":"device_mounting","identity":identity,"operation":"turn_over"}),1.).unwrap();assert_eq!(lab.device_mountings[identity]["face_turned_over"],false);
        let mut lab=Lab::new(path.clone()).unwrap();assert_eq!(lab.actions.settings().direction.pitch_sign,-1.);assert_eq!(lab.device_mountings[identity]["roll_sign"],-1.);lab.peer_macs.insert("10.0.0.3".into(),"02:00:00:80:BE:03".into());lab.records.insert(lab.mapping[2].clone(),Record{q:crate::gait::IDENTITY,seen:2.,angular:0.,has_interval:true});lab.engine.update(2,crate::gait::IDENTITY,2.).unwrap();
        let before=lab.actions.settings().direction.max_angle;lab.command(&json!({"command":"device_mounting","identity":identity,"operation":"reset"}),2.).unwrap();assert_eq!(lab.actions.settings().direction.max_angle,before);assert_eq!(lab.mapping[2],"10.0.0.3:0");assert!(lab.actions.settings().direction.learned_forward.is_none());assert!(lab.learned_mounting_identity.is_none());assert!(!lab.device_mountings.contains_key(identity));assert!(!lab.engine.calibrated);drop(lab);let mut restored=Lab::new(path).unwrap();restored.ingest(&identity_packet([0x02,0x00,0x00,0x80,0xbe,0x03]),"10.0.0.3:6969".parse().unwrap(),3.).unwrap();assert!(restored.learned_mounting_identity.is_none());assert!(restored.actions.settings().direction.learned_forward.is_none());assert!(!restored.device_mountings.contains_key(identity));fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn device_calibrations_follow_identity_through_switch_and_dhcp() {
        let dir=std::env::temp_dir().join(format!("device-switch-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));fs::create_dir_all(&dir).unwrap();let path=dir.join("config.json");let mut l=Lab::new(path.clone()).unwrap();l.movement_engine="body_tilt".into();l.engine.cfg.require_steps=false;l.auto_add=false;
        let a="10.1.0.1:0";let b="10.1.0.2:0";let ma="02:00:00:82:6C:63";let mb="02:00:00:80:BE:03";let ia=format!("{ma}:0");let ib=format!("{mb}:0");
        for (id,mac) in [(a,ma),(b,mb)] {let ip=id.rsplit_once(':').unwrap().0;l.peer_macs.insert(ip.into(),mac.into());l.records.insert(id.into(),Record{q:crate::gait::IDENTITY,seen:1.,angular:0.,has_interval:true});}
        let mut s=l.actions.settings().clone();s.direction.enabled=true;s.direction.learned_forward=Some([1.,0.,0.]);s.direction.learned_right=Some([0.,0.,1.]);l.actions=crate::actions::ActionProcessor::new(s.clone()).unwrap();l.learned_mounting_identity=Some(ia.clone());l.remember_device_mounting(&ia);
        s.direction.learned_forward=Some([0.,1.,0.]);s.direction.roll_sign=-1.;l.actions=crate::actions::ActionProcessor::new(s).unwrap();l.learned_mounting_identity=Some(ib.clone());l.remember_device_mounting(&ib);
        l.command(&json!({"command":"assign_sensor_role","role":"torso","id":a}),1.).unwrap();assert_eq!(l.actions.settings().direction.learned_forward,Some([1.,0.,0.]));assert!(!l.engine.calibrated);
        l.engine.update(2,crate::gait::IDENTITY,1.).unwrap();l.command(&json!({"command":"calibrate"}),1.).unwrap();assert!(l.engine.calibrated);
        assert!(l.command(&json!({"command":"device_mounting","identity":ib,"operation":"flip_forward"}),1.).is_err());assert!(l.engine.calibrated);
        l.command(&json!({"command":"device_mounting","identity":ia,"operation":"flip_forward"}),1.).unwrap();assert!(l.engine.calibrated);assert!(!l.engine.armed);assert_eq!(l.actions.settings().direction.pitch_sign,-1.);
        l.command(&json!({"command":"assign_sensor_role","role":"torso","id":b}),1.).unwrap();assert!(!l.engine.calibrated);assert_eq!(l.actions.settings().direction.learned_forward,Some([0.,1.,0.]));assert_eq!(l.actions.settings().direction.roll_sign,-1.);
        l.command(&json!({"command":"assign_sensor_role","role":"torso","id":a}),1.).unwrap();assert_eq!(l.actions.settings().direction.pitch_sign,-1.);assert!(!l.engine.calibrated);
        drop(l);let mut l=Lab::new(path).unwrap();l.ingest(&identity_packet([0x02,0x00,0x00,0x82,0x6c,0x63]),"10.1.0.7:6969".parse().unwrap(),2.).unwrap();assert_eq!(l.mapping[2],"10.1.0.7:0");assert!(!l.engine.calibrated);l.ingest(&identity_rotation(1),"10.1.0.7:6969".parse().unwrap(),2.1).unwrap();assert!(l.mounting_valid(2.1));assert_eq!(l.actions.settings().direction.pitch_sign,-1.);fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn mounting_binding_follows_mac_not_address_or_center() {
        let dir = std::env::temp_dir().join(format!("mounting-binding-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let mut l = Lab::new(path.clone()).unwrap();
        l.engine.cfg.y_up=true; l.movement_engine = "body_tilt".into(); l.engine.cfg.require_steps = false; l.auto_add = false;
        let mut settings = l.actions.settings().clone(); settings.direction.enabled = true;
        settings.direction.learned_forward = Some([1.,0.,0.]); settings.direction.learned_right = Some([0.,0.,1.]);
        l.actions = crate::actions::ActionProcessor::new(settings).unwrap();
        fn pose(l: &mut Lab, id: &str, mac: &str, q: Q, t: f64) {
            let ip = id.rsplit_once(':').unwrap().0;
            l.peer_macs.insert(ip.into(), mac.into());
            l.records.insert(id.into(), Record { q, seen:t, angular:0., has_interval:true });
            if l.mapping[2] == id { l.engine.update(2,q,t).unwrap(); }
        }
        let a="10.0.0.1:0"; let b="10.0.0.2:0";
        let ma="02:00:00:82:6C:63"; let mb="02:00:00:80:BE:03";
        pose(&mut l,a,ma,crate::gait::IDENTITY,1.);
        l.command(&json!({"command":"assign_sensor_role","role":"torso","id":a}),1.).unwrap();
        pose(&mut l,a,ma,crate::gait::IDENTITY,1.1); l.command(&json!({"command":"calibrate"}),1.1).unwrap();
        l.tick(1.1,0.); assert!(!l.mounting_valid(1.1));
        let forward=[(0.15f64).cos(),(0.15f64).sin(),0.,0.];
        pose(&mut l,a,ma,forward,1.2); l.command(&json!({"command":"learn_tilt_forward"}),1.2).unwrap();
        let right=[(0.15f64).cos(),0.,0.,(0.15f64).sin()];
        pose(&mut l,a,ma,right,1.3); l.command(&json!({"command":"learn_tilt_right"}),1.3).unwrap();
        pose(&mut l,b,mb,crate::gait::IDENTITY,2.);
        l.command(&json!({"command":"assign_sensor_role","role":"torso","id":b}),2.).unwrap();
        pose(&mut l,b,mb,crate::gait::IDENTITY,2.1); l.command(&json!({"command":"calibrate"}),2.1).unwrap();
        pose(&mut l,b,mb,forward,2.2); l.tick(2.2,0.);
        assert_eq!(l.action_preview.travel_xy,[0.;2]); assert_eq!(l.travel_xy,[0.;2]);
        assert!(!l.mounting_valid(2.2)); assert!(l.command(&json!({"command":"learn_tilt_right"}),2.2).is_err());
        l.command(&json!({"command":"learn_tilt_forward"}),2.2).unwrap(); assert!(l.mounting_valid(2.2));
        pose(&mut l,b,mb,right,2.3); l.command(&json!({"command":"learn_tilt_right"}),2.3).unwrap();
        let mut restored = Lab::new(path.clone()).unwrap();
        let new_addr: SocketAddr = "10.0.0.3:6969".parse().unwrap();
        restored.ingest(&identity_packet([0x02,0x00,0x00,0x80,0xbe,0x03]),new_addr,3.).unwrap();
        restored.ingest(&identity_rotation(1),new_addr,3.1).unwrap();
        assert_eq!(restored.mapping[2],"10.0.0.3:0"); assert!(restored.mounting_valid(3.1));
        assert!(!restored.engine.armed);
        let mut canceled = restored.actions.settings().to_json();
        canceled["direction"]["learned_forward"] = json!([0.,1.,0.]);
        restored.command(&json!({"command":"actions_settings","settings":canceled}),3.1).unwrap();
        assert!(restored.learned_mounting_identity.is_some(),"old wizard drafts cannot remove the current device binding");
        assert!(restored.mounting_valid(3.1));
        assert_ne!(restored.actions.settings().direction.learned_forward,Some([0.,1.,0.]));
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn chest_step_profile_survives_switch_and_restart() {
        let dir=std::env::temp_dir().join(format!("chest-step-profile-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));fs::create_dir_all(&dir).unwrap();let path=dir.join("config.json");
        let mut lab=Lab::new(path.clone()).unwrap();lab.command(&json!({"command":"step_input","source":"chest"}),0.).unwrap();lab.command(&json!({"command":"profile_save","name":"Chest","description":""}),0.).unwrap();
        lab.command(&json!({"command":"step_input","source":"off"}),0.).unwrap();lab.command(&json!({"command":"profile_save","name":"Off","description":""}),0.).unwrap();drop(lab);
        let mut lab=Lab::new(path).unwrap();lab.command(&json!({"command":"profile_load","name":"Chest"}),1.).unwrap();assert_eq!(lab.snapshot(1.)["step_input"],"chest");assert_eq!(lab.layout_count(),1);assert!(!lab.engine.armed);
        lab.command(&json!({"command":"profile_load","name":"Off"}),2.).unwrap();assert_eq!(lab.snapshot(2.)["step_input"],"off");assert!(!lab.engine.armed);fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn action_tuning_preserves_center_but_mount_changes_reset_it() {
        let path=std::env::temp_dir().join(format!("tuning-center-{}-{}.json",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let mut lab=Lab::new(path.clone()).unwrap();lab.movement_engine="body_tilt".into();lab.engine.cfg.require_steps=false;lab.mapping[2]="chest".into();
        lab.records.insert("chest".into(),Record{q:crate::gait::IDENTITY,seen:1.,angular:0.,has_interval:true});lab.engine.update(2,crate::gait::IDENTITY,1.).unwrap();
        let mut settings=lab.actions.settings().clone();settings.direction.enabled=true;lab.actions=crate::actions::ActionProcessor::new(settings).unwrap();lab.command(&json!({"command":"calibrate"}),1.).unwrap();
        let mut changed=lab.actions.settings().to_json();changed["direction"]["forward_max_angle"]=json!(0.5);changed["jump"]["threshold"]=json!(0.3);
        lab.command(&json!({"command":"actions_settings","settings":changed}),1.).unwrap();assert!(lab.engine.calibrated);assert!(!lab.engine.armed);let input=lab.action_input(1.01);assert!(lab.actions.process(&input).calibrated);
        let mut changed=lab.actions.settings().to_json();changed["direction"]["pitch_sign"]=json!(-1.);lab.command(&json!({"command":"actions_settings","settings":changed}),1.02).unwrap();assert!(lab.engine.calibrated);assert_eq!(lab.actions.settings().direction.pitch_sign,1.,"stale tuning cannot flip a device");
        let _=fs::remove_file(path);
    }
    #[test]
    fn chest_diagnostic_ingests_real_packets_without_enabling_step_output() {
        let mut lab=Lab::new(PathBuf::from("nonexistent-chest-diagnostic-fixture.json")).unwrap();lab.movement_engine="body_tilt".into();lab.engine.cfg.require_steps=false;lab.mapping[2]="127.0.0.2:0".into();
        let addr:SocketAddr="127.0.0.2:6969".parse().unwrap();
        for n in 1..20 {let t=1.+n as f64*0.01;let mut packet=4u32.to_be_bytes().to_vec();packet.extend((n as u64).to_be_bytes());for v in [8.+n as f32*0.1,0.,0.] {packet.extend(v.to_be_bytes());}lab.ingest(&packet,addr,t).unwrap();lab.tick(t,0.);}
        let snap=lab.snapshot(1.19);assert_eq!(snap["step_input"],"off");assert_eq!(snap["locomotion"]["diagnostic"]["status"],"ready");assert!(snap["locomotion"]["diagnostic"]["signal"].as_f64().unwrap()>0.);assert!(snap["locomotion"]["diagnostic"]["raw_speed"].as_f64().unwrap()>0.);assert_eq!(lab.engine.speed,0.);assert!(!lab.engine.armed);
        lab.tick(1.5,0.);let snap=lab.snapshot(1.5);assert_ne!(snap["locomotion"]["diagnostic"]["status"],"ready");assert_eq!(snap["locomotion"]["diagnostic"]["signal"],0.);
    }
    #[test]
    fn step_tuning_preserves_center_and_keeps_lean_ready_without_acceleration() {
        let path=std::env::temp_dir().join(format!("step-tuning-{}-{}.json",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let mut lab=Lab::new(path.clone()).unwrap();lab.command(&json!({"command":"step_input","source":"chest"}),0.).unwrap();
        lab.engine.update(2,crate::gait::IDENTITY,1.).unwrap();lab.engine.calibrate(1.).unwrap();lab.engine.update_acceleration(2,[8.,0.,0.],1.01).unwrap();lab.engine.arm(1.01).unwrap();
        let mut settings=lab.engine.cfg.motion.to_json();settings["threshold"]=json!(3.);
        lab.command(&json!({"command":"locomotion_settings","algorithm":"continuous","settings":settings}),1.02).unwrap();
        assert!(lab.engine.calibrated);assert!(!lab.engine.armed);assert_eq!(lab.engine.speed,0.);assert!(lab.engine.input_ready(1.02),"Fresh chest lean remains usable while walking filter restarts");
        lab.engine.update_acceleration(2,[9.,0.,0.],1.03).unwrap();assert!(lab.engine.input_ready(1.03));assert!(lab.engine.arm(1.03).is_ok());
        let _=fs::remove_file(path);
    }
    #[test]
    fn chest_steps_persist_and_use_one_real_sensor() {
        let dir=std::env::temp_dir().join(format!("chest-steps-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(&dir).unwrap();let path=dir.join("config.json");let mut lab=Lab::new(path.clone()).unwrap();
        lab.command(&json!({"command":"step_input","source":"chest"}),0.).unwrap();
        assert_eq!(lab.layout_count(),1);assert_eq!(lab.snapshot(0.)["step_input"],"chest");assert!(!lab.engine.armed);
        let mut lab=Lab::new(path).unwrap();assert!(lab.engine.cfg.chest_steps);assert_eq!(lab.layout_count(),1);
        lab.engine.update(2,crate::gait::IDENTITY,1.).unwrap();lab.engine.calibrate(1.).unwrap();
        for n in 1..10 {let t=1.+n as f64*0.01;lab.engine.update_acceleration(2,[8.+n as f64*0.1,0.,0.],t).unwrap();lab.engine.tick(t);}
        assert!(lab.engine.preview.speed>0.);assert_eq!(lab.engine.speed,0.);lab.engine.arm(1.09).unwrap();
        for n in 10..20 {let t=1.+n as f64*0.01;lab.engine.update_acceleration(2,[8.+n as f64*0.1,0.,0.],t).unwrap();lab.engine.tick(t);}assert!(lab.engine.speed>0.);
        lab.engine.update(2,crate::gait::IDENTITY,1.5).unwrap();lab.engine.tick(1.5);assert!(!lab.engine.armed);assert_eq!(lab.engine.speed,0.);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn unarmed_test_room_preserves_center_after_chest_loss() {
        let mut l=Lab::new(PathBuf::from("nonexistent-restart-preview-test.json")).unwrap();
        l.movement_engine="body_tilt".into();l.engine.cfg.require_steps=false;l.mapping[2]="10.0.0.191:0".into();
        let mut settings=l.actions.settings().clone();settings.direction.enabled=true;settings.direction.learned_forward=Some([1.,0.,0.]);
        l.actions=crate::actions::ActionProcessor::new(settings).unwrap();
        l.records.insert(l.mapping[2].clone(),Record{q:crate::gait::IDENTITY,seen:1.,angular:0.,has_interval:true});
        l.engine.update(2,crate::gait::IDENTITY,1.).unwrap();l.command(&json!({"command":"calibrate"}),1.).unwrap();
        l.tick(2.,0.);assert!(l.engine.calibrated);assert!(!l.engine.armed);
        let reboot_pose=[0.,0.,0.,1.];
        l.records.get_mut(&l.mapping[2]).unwrap().q=reboot_pose;l.records.get_mut(&l.mapping[2]).unwrap().seen=2.1;
        l.engine.update(2,reboot_pose,2.1).unwrap();l.tick(2.1,0.);
        assert!(l.action_preview.calibrated);assert!(!l.engine.armed);
        assert_eq!(l.actions.settings().direction.learned_forward,Some([1.,0.,0.]));
        l.command(&json!({"command":"calibrate"}),2.1).unwrap();l.tick(2.1,0.);
        assert!(l.action_preview.calibrated);assert_eq!(l.action_preview.travel_xy,[0.;2]);
    }
    #[test]
    fn game_profiles_persist_but_never_restore_old_mounting_or_arm() {
        let dir=std::env::temp_dir().join(format!("arcane-game-profile-{}",std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path=dir.join("config.json");
        let mut l=Lab::new(path.clone()).unwrap();
        l.mapping[2]="10.0.0.191:0".into();
        let mut settings=l.actions.settings().clone();
        settings.direction.learned_forward=Some([1.,0.,0.]);
        settings.direction.learned_right=Some([0.,0.,1.]);
        l.actions=crate::actions::ActionProcessor::new(settings.clone()).unwrap();
        l.engine.armed=true;
        l.command(&json!({"command":"game_profile_apply","game_id":"arizona_sunshine_legacy"}),1.).unwrap();
        assert!(!l.engine.armed && !l.engine.calibrated && l.neutral_pending);
        assert_eq!(l.mapping[2],"10.0.0.191:0");
        let mut restored=Lab::new(path.clone()).unwrap();
        assert_eq!(restored.game_profiles["selected"],"arizona_sunshine_legacy");
        settings.direction.learned_forward=Some([0.,1.,0.]);
        restored.actions=crate::actions::ActionProcessor::new(settings).unwrap();
        restored.command(&json!({"command":"game_profile_restore","game_id":"arizona_sunshine_legacy"}),2.).unwrap();
        assert_eq!(restored.actions.settings().direction.learned_forward,Some([0.,1.,0.]));
        assert!(!restored.engine.armed);
        let previous=restored.game_profiles.clone();
        assert!(restored.command(&json!({"command":"game_profile_apply","game_id":"custom","profile_name":"missing"}),3.).is_err());
        assert_eq!(restored.game_profiles,previous);
        restored.profile_store().save("Partial","",json!({"detection":{"threshold":3.}})).unwrap();
        restored.command(&json!({"command":"game_profile_apply","game_id":"custom","profile_name":"Partial"}),4.).unwrap();
        let original_mode=restored.movement_engine.clone();
        let original_cap=restored.actions.settings().direction.max_output;
        let mut changed=restored.actions.settings().clone();changed.direction.max_output=0.1;
        restored.actions=crate::actions::ActionProcessor::new(changed).unwrap();
        restored.command(&json!({"command":"game_profile_restore","game_id":"custom"}),5.).unwrap();
        assert_eq!(restored.actions.settings().direction.max_output,original_cap);
        assert_eq!(restored.movement_engine,original_mode);
        assert_eq!(restored.engine.cfg.threshold,3.);
        restored.engine.armed=true;
        assert!(restored.command(&json!({"command":"game_profile_apply","game_id":"invalid"}),6.).is_err());
        assert!(!restored.engine.armed);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn full_lean_sprint_is_only_sent_when_armed_and_fresh() {
        let mut l=Lab::new(PathBuf::from("nonexistent-lean-sprint-fixture.json")).unwrap();
        l.engine.cfg.y_up=true; l.output_mode="steamvr".into();l.protocol_v2=true;l.movement_engine="body_tilt".into();l.engine.cfg.require_steps=false;l.mapping[2]="10.0.0.1:0".into();
        let mut settings=crate::actions::Settings::default();settings.direction.enabled=true;settings.direction.decouple=false;settings.direction.sprint_full_forward=true;settings.direction.learned_forward=Some([1.,0.,0.]);settings.direction.learned_right=Some([0.,0.,1.]);
        l.actions=crate::actions::ActionProcessor::new(settings).unwrap();
        l.records.insert(l.mapping[2].clone(),Record{q:crate::gait::IDENTITY,seen:1.,angular:0.,has_interval:true});
        l.engine.update(2,crate::gait::IDENTITY,1.).unwrap();l.command(&json!({"command":"calibrate"}),1.).unwrap();
        let q=[0.45f64.cos(),0.45f64.sin(),0.,0.];l.records.get_mut(&l.mapping[2]).unwrap().q=q;l.records.get_mut(&l.mapping[2]).unwrap().seen=1.1;l.engine.update(2,q,1.1).unwrap();
        l.ack_seen=1.1;l.ack_flags=7;l.neutral_pending=false;
        let p=l.steam_packet(1.1);assert_eq!(u32::from_le_bytes(p[28..32].try_into().unwrap()),0);
        l.command(&json!({"command":"arm"}),1.1).unwrap();let p=l.steam_packet(1.1);assert_eq!(u32::from_le_bytes(p[28..32].try_into().unwrap())&4,4);
        l.command(&json!({"command":"stop"}),1.1).unwrap();let p=l.steam_packet(1.1);assert_eq!(u32::from_le_bytes(p[28..32].try_into().unwrap()),0);
        l.engine.armed=true;let p=l.steam_packet(1.5);assert_eq!(u32::from_le_bytes(p[28..32].try_into().unwrap()),0);
    }
    #[test]
    fn silent_driver_retries_neutral_generation_and_rejects_old_ack() {
        let mut l=Lab::new(PathBuf::from("nonexistent-retry-fixture.json")).unwrap();
        l.output_mode="steamvr".into();l.protocol_v2=true;
        let old=l.steam_packet(1.);let initial=l.generation;
        assert_eq!(l.generation,initial);l.steam_packet(1.9);assert_eq!(l.generation,initial);
        let retry=l.steam_packet(2.01);assert_eq!(l.generation,initial.wrapping_add(1));
        assert!(retry[12..40].iter().all(|b|*b==0));
        let mut ack=[0u8;32];ack[..4].copy_from_slice(b"AKV2");ack[4..12].copy_from_slice(&old[4..12]);ack[12..16].copy_from_slice(&7u32.to_le_bytes());ack[16..20].copy_from_slice(&1.7f32.to_le_bytes());
        l.steam_ack(&ack,([127,0,0,1],29741).into(),2.02);assert!(!l.driver_ready(2.02));
        ack[4..12].copy_from_slice(&retry[4..12]);ack[12..16].copy_from_slice(&1u32.to_le_bytes());
        l.steam_ack(&ack,([127,0,0,1],29741).into(),2.02);
        for t in [2.5,3.,3.5,4.] {let p=l.steam_packet(t);ack[4..12].copy_from_slice(&p[4..12]);l.steam_ack(&ack,([127,0,0,1],29741).into(),t);assert_eq!(l.generation,initial.wrapping_add(1));}
        assert!(!l.driver_ready(4.));
        let p=l.steam_packet(4.1);ack[4..12].copy_from_slice(&p[4..12]);ack[12..16].copy_from_slice(&7u32.to_le_bytes());l.steam_ack(&ack,([127,0,0,1],29741).into(),4.1);
        assert!(l.driver_ready(4.1));assert!(!l.engine.armed);
    }
    #[test]
    fn lost_ready_driver_disarms_and_retries_without_replaying_motion() {
        let mut l=Lab::new(PathBuf::from("nonexistent-retry-lost-fixture.json")).unwrap();
        l.output_mode="steamvr".into();l.protocol_v2=true;l.engine.calibrated=true;l.engine.armed=true;l.was_driver_ready=true;l.ack_seen=1.;l.ack_flags=7;l.steam_retry_since=Some(1.);
        let old=l.generation;l.tick(1.3,0.);assert!(!l.engine.armed&&!l.engine.calibrated);
        let p=l.steam_packet(2.01);assert_eq!(l.generation,old.wrapping_add(1));assert!(p[12..40].iter().all(|b|*b==0));assert!(!l.engine.armed&&!l.engine.calibrated);
    }
    #[test]
    fn chest_only_center_survives_short_driver_gap_without_auto_arm() {
        let mut l=Lab::new(PathBuf::from("nonexistent-chest-gap-fixture.json")).unwrap();
        l.output_mode="steamvr".into();l.protocol_v2=true;l.movement_engine="body_tilt".into();l.engine.cfg.require_steps=false;l.mapping[2]="10.0.0.1:0".into();
        let mut s=crate::actions::Settings::default();s.direction.enabled=true;s.direction.decouple=false;s.jump.enabled=true;s.jump.source="acceleration".into();l.actions=crate::actions::ActionProcessor::new(s).unwrap();
        l.records.insert(l.mapping[2].clone(),Record{q:crate::gait::IDENTITY,seen:1.,angular:0.,has_interval:true});l.engine.update(2,crate::gait::IDENTITY,1.).unwrap();l.command(&json!({"command":"calibrate"}),1.).unwrap();
        l.engine.update(2,crate::gait::IDENTITY,1.259).unwrap();l.records.get_mut(&l.mapping[2]).unwrap().seen=1.259;
        l.engine.armed=true;l.was_driver_ready=true;l.ack_seen=1.;l.ack_flags=7;l.neutral_pending=false;l.steam_retry_since=Some(1.);
        l.tick(1.266,0.);assert!(!l.engine.armed && l.engine.calibrated);assert!(l.stop_reason.contains("press Start Movement"));
        assert!(l.actions.process(&l.action_input(1.266)).calibrated);
        let generation=l.generation;let p=l.steam_packet(2.01);assert_eq!(l.generation,generation.wrapping_add(1));assert!(p[12..40].iter().all(|b|*b==0));assert!(l.engine.calibrated&&!l.engine.armed);
        l.ack_seen=2.01;l.ack_flags=7;l.neutral_pending=false;assert!(l.command(&json!({"command":"arm"}),2.01).is_err(),"stale chest cannot arm");
    }
    #[test]
    fn headset_dependent_center_is_invalidated_on_driver_gap() {
        let mut l=Lab::new(PathBuf::from("nonexistent-hmd-gap-fixture.json")).unwrap();
        l.output_mode="steamvr".into();l.movement_engine="body_tilt".into();l.engine.cfg.require_steps=false;l.mapping[2]="10.0.0.1:0".into();
        let mut s=crate::actions::Settings::default();s.direction.enabled=true;s.turn.enabled=true;l.actions=crate::actions::ActionProcessor::new(s).unwrap();
        l.records.insert(l.mapping[2].clone(),Record{q:crate::gait::IDENTITY,seen:1.,angular:0.,has_interval:true});l.engine.update(2,crate::gait::IDENTITY,1.).unwrap();l.hmd=Some(crate::actions::TimedHmd{height:1.7,yaw:0.,pitch:0.,roll:0.,seen:1.});l.command(&json!({"command":"calibrate"}),1.).unwrap();
        l.engine.update(2,crate::gait::IDENTITY,1.259).unwrap();l.records.get_mut(&l.mapping[2]).unwrap().seen=1.259;l.engine.armed=true;l.was_driver_ready=true;l.ack_seen=1.;l.ack_flags=7;
        l.tick(1.266,0.);assert!(!l.engine.armed&&!l.engine.calibrated);assert!(l.stop_reason.contains("recalibration required"));
    }
    #[test]
    fn snapshot_only_exposes_fresh_valid_headset_pose() {
        let mut l=Lab::new(PathBuf::from("nonexistent-hmd-fixture.json")).unwrap();
        l.hmd=Some(crate::actions::TimedHmd{height:1.7,yaw:0.,pitch:0.,roll:0.,seen:2.});l.v2_sequence=Some(42);
        let pose=l.snapshot(2.1)["hmd_pose"].clone();assert_eq!(pose["fresh"],true);assert_eq!(pose["height"],1.7);assert_eq!(pose["sequence"],42);assert_eq!(pose["sample_time"],2.);
        let pose=l.snapshot(2.3)["hmd_pose"].clone();assert_eq!(pose["fresh"],false);assert!(pose["height"].is_null()&&pose["sequence"].is_null());
        l.hmd.as_mut().unwrap().height=f64::NAN;assert_eq!(l.snapshot(2.1)["hmd_pose"]["fresh"],false);
    }
    fn identity_packet(mac:[u8;6]) -> Vec<u8> {
        let mut p=3u32.to_be_bytes().to_vec();p.extend(0u64.to_be_bytes());p.extend([0;28]);p.push(3);p.extend(b"0.7");p.extend(mac);p.push(0);p
    }
    fn identity_rotation(seq:u64) -> Vec<u8> {
        let mut p=1u32.to_be_bytes().to_vec();p.extend(seq.to_be_bytes());
        for f in [0f32,0.,0.,1.] {p.extend(f.to_be_bytes());}p
    }
    #[test]
    fn known_tracker_reconnect_preserves_center_and_stops_output() {
        let mut l=Lab::new(PathBuf::from("nonexistent-reconnect-center-fixture.json")).unwrap();
        l.auto_add=false;l.movement_engine="body_tilt".into();l.engine.cfg.require_steps=false;
        let addr:SocketAddr="10.1.0.20:6969".parse().unwrap();let mac=[0x20,1,2,3,4,5];
        l.mapping[2]="10.1.0.20:0".into();l.tracker_identities.insert(l.mapping[2].clone(),"20:01:02:03:04:05".into());
        l.engine.update(2,crate::gait::IDENTITY,1.).unwrap();l.engine.calibrate(1.).unwrap();l.engine.armed=true;
        l.ingest(&identity_packet(mac),addr,2.).unwrap();
        assert!(l.engine.calibrated);assert!(!l.engine.armed);assert!(!l.engine.fresh(2.));assert!(l.stop_reason.contains("center preserved"));
        l.ingest(&identity_packet([0x21,1,2,3,4,5]),addr,3.).unwrap();assert!(!l.engine.calibrated);assert!(!l.engine.armed);
    }
    #[test]
    fn fifth_sensor_reconnect_and_error_invalidate_calibration() {
        let path=std::env::temp_dir().join(format!("arcane-thigh-reset-{}.json",std::process::id()));
        let mut l=Lab::new(path.clone()).unwrap();l.sensor_count=5;l.auto_add=false;
        l.thighs=["10.1.0.20:0".into(),"10.1.0.21:0".into()];
        let addr:SocketAddr="10.1.0.20:6969".parse().unwrap();
        l.engine.calibrated=true;l.engine.armed=true;
        l.ingest(&identity_packet([0x20,1,2,3,4,5]),addr,1.).unwrap();
        assert!(!l.engine.calibrated&&!l.engine.armed);
        l.engine.calibrated=true;l.engine.armed=true;
        let mut error=15u32.to_be_bytes().to_vec();error.extend(1u64.to_be_bytes());error.extend([0,2]);
        l.ingest(&error,addr,2.).unwrap();
        assert!(!l.engine.calibrated&&!l.engine.armed);
        l.ingest(&identity_rotation(2),addr,2.1).unwrap();
        assert!(!l.engine.calibrated&&!l.engine.armed);
        let _=fs::remove_file(path);
    }
    #[test]
    fn identity_follows_dhcp_without_stealing_roles() {
        let path=std::env::temp_dir().join(format!("arcane-identity-{}-{}.json",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let mut l=Lab::new(path.clone()).unwrap();l.movement_engine="body_tilt".into();l.engine.cfg.require_steps=false;
        let a:SocketAddr="10.1.0.10:6969".parse().unwrap();let b:SocketAddr="10.1.0.11:6969".parse().unwrap();
        let mac=[0x02,0x00,0x00,0x82,0x6c,0x63];
        l.ingest(&identity_packet(mac),a,1.).unwrap();l.ingest(&identity_rotation(1),a,1.1).unwrap();
        assert_eq!(l.mapping[2],"10.1.0.10:0");assert_eq!(l.snapshot(1.1)["roles"]["torso"]["mac"],"02:00:00:82:6C:63");
        l.engine.calibrated=true;l.engine.armed=true;
        l.ingest(&identity_packet([0x20,1,2,3,4,5]),a,2.).unwrap();l.ingest(&identity_rotation(1),a,2.1).unwrap();
        assert!(!l.engine.armed && !l.engine.fresh(2.1));assert_eq!(l.mapping[2],"10.1.0.10:0");
        l.ingest(&identity_packet(mac),b,3.).unwrap();l.ingest(&identity_rotation(1),b,3.1).unwrap();
        assert_eq!(l.mapping[2],"10.1.0.11:0");assert!(!l.engine.calibrated && !l.engine.armed);
        let mut restored=Lab::new(path.clone()).unwrap();assert_eq!(restored.mapping[2],"10.1.0.11:0");assert_eq!(restored.tracker_identities["10.1.0.11:0"],"02:00:00:82:6C:63");
        let c:SocketAddr="10.1.0.12:6969".parse().unwrap();
        restored.ingest(&identity_packet(mac),c,4.).unwrap();restored.ingest(&identity_rotation(1),c,4.1).unwrap();
        assert_eq!(restored.mapping[2],"10.1.0.12:0");assert!(!restored.engine.armed && !restored.engine.calibrated);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn multi_sensor_handshake_never_autoassigns() {
        let path=std::env::temp_dir().join(format!("arcane-multi-{}-{}.json",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let mut l=Lab::new(path.clone()).unwrap();let a:SocketAddr="10.2.0.1:6969".parse().unwrap();
        l.ingest(&identity_packet([0x20,1,2,3,4,5]),a,1.).unwrap();l.ingest(&identity_rotation(1),a,1.1).unwrap();
        assert!(l.mapping.iter().all(String::is_empty));assert!(l.records.contains_key("10.2.0.1:0"));
    }
    #[test]
    fn known_thigh_and_foot_follow_mac_and_sensor_index() {
        let path=std::env::temp_dir().join(format!("arcane-thigh-{}-{}.json",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let mut l=Lab::new(path.clone()).unwrap();l.sensor_count=5;
        l.mapping[0]="10.3.0.1:0".into();l.thighs[0]="10.3.0.1:1".into();
        let a:SocketAddr="10.3.0.1:6969".parse().unwrap();let b:SocketAddr="10.3.0.2:6969".parse().unwrap();let mac=[0x20,1,2,3,4,5];
        l.ingest(&identity_packet(mac),a,1.).unwrap();l.ingest(&identity_packet(mac),b,2.).unwrap();
        assert_eq!(l.mapping[0],"10.3.0.2:0");assert_eq!(l.thighs[0],"10.3.0.2:1");
        let restored=Lab::new(path.clone()).unwrap();assert_eq!(restored.mapping[0],l.mapping[0]);assert_eq!(restored.thighs[0],l.thighs[0]);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn continuous_settings_persist_atomically_with_roles_and_reset_generation() {
        let path = std::env::temp_dir().join(format!(
            "vrcraft-motion-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut l = Lab::new(path.clone()).unwrap();
        l.mapping = [
            "sensor-left".into(),
            "sensor-right".into(),
            "sensor-chest".into(),
        ];
        assert!(l.engine.cfg.continuous);
        let generation = l.generation;
        l.command(&json!({"command":"locomotion_settings","algorithm":"continuous","settings":{"intensity":2.,"fast_stop":false}}),0.).unwrap();
        assert_ne!(l.generation, generation);
        assert!(!l.engine.armed && !l.engine.calibrated && l.neutral_pending);
        let saved = std::fs::read(&path).unwrap();
        let generation = l.generation;
        assert!(l.command(&json!({"command":"locomotion_settings","algorithm":"continuous","settings":{"slow_window":0.001}}),0.).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), saved);
        assert_eq!(l.generation, generation);
        let restored = Lab::new(path.clone()).unwrap();
        assert_eq!(restored.mapping, l.mapping);
        assert_eq!(restored.engine.cfg.motion.intensity, 2.);
        assert!(!restored.engine.cfg.motion.fast_stop);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn continuous_demo_preview_and_output_have_separate_gates() {
        let mut l = Lab::new(PathBuf::from("nonexistent-continuous-fixture.json")).unwrap();
        l.command(&json!({"command":"mode","mode":"demo"}), 0.)
            .unwrap();
        l.command(&json!({"command":"demo","walk":true,"yaw":45}), 0.)
            .unwrap();
        for n in 1..200 {
            let t = n as f64 * 0.02;
            l.tick(t, t);
        }
        assert!(l.engine.preview.speed > 0.);
        assert_eq!(l.engine.speed, 0.);
        l.command(&json!({"command":"calibrate"}), 3.98).unwrap();
        assert!(l.command(&json!({"command":"arm"}), 3.98).is_err());
        l.tick(4., 4.);
        l.command(&json!({"command":"arm"}), 4.).unwrap();
        l.command(&json!({"command":"demo","walk":true,"yaw":45}), 4.)
            .unwrap();
        for n in 201..280 {
            let t = n as f64 * 0.02;
            l.tick(t, t);
        }
        assert!(l.engine.speed > 0., "{}", l.snapshot(5.58));
        let packet = l.steam_packet(5.58);
        assert_eq!(
            f32::from_le_bytes(packet[12..16].try_into().unwrap()),
            0.,
            "Synthetic mode cannot inject SteamVR output"
        );
        l.command(&json!({"command":"stop"}), 5.58).unwrap();
        assert!(!l.engine.armed && l.engine.speed == 0.);
    }
    // Existing fixtures exercise the original rotation/alternation mode.
    fn legacy_lab(path: PathBuf) -> Result<Lab, String> {
        let mut lab = Lab::new(path)?;
        lab.engine.cfg.continuous = false;
        Ok(lab)
    }

    #[test]
    fn zero_heartbeat_refreshes_device_only_without_rewinding_motion_sequence() {
        let mut lab =
            legacy_lab(std::env::temp_dir().join("slime-heartbeat-ordering-test.json")).unwrap();
        let addr: SocketAddr = "127.0.0.1:42000".parse().unwrap();
        let mut pose = 17u32.to_be_bytes().to_vec();
        pose.extend(100u64.to_be_bytes());
        pose.extend([0, 1]);
        for f in [0f32, 0., 0., 1.] {
            pose.extend(f.to_be_bytes());
        }
        pose.push(3);
        lab.ingest(&pose, addr, 1.).unwrap();
        let mut heartbeat = 0u32.to_be_bytes().to_vec();
        heartbeat.extend(0u64.to_be_bytes());
        lab.ingest(&heartbeat, addr, 2.).unwrap();
        assert_eq!(lab.peers[&addr], 2.);
        assert_eq!(lab.counters[&addr], 100);
        assert_eq!(lab.records["127.0.0.1:0"].seen, 1.);
        pose[4..12].copy_from_slice(&99u64.to_be_bytes());
        lab.ingest(&pose, addr, 2.1).unwrap();
        assert_eq!(lab.records["127.0.0.1:0"].seen, 1.);
        assert_eq!(lab.counters[&addr], 100);
        let mut mixed = 100u32.to_be_bytes().to_vec();
        mixed.extend(0u64.to_be_bytes());
        mixed.extend(4u16.to_be_bytes());
        mixed.extend(0u32.to_be_bytes());
        mixed.extend(((pose.len() - 8) as u16).to_be_bytes());
        mixed.extend(17u32.to_be_bytes());
        mixed.extend(&pose[12..]);
        lab.ingest(&mixed, addr, 2.2).unwrap();
        assert_eq!(lab.records["127.0.0.1:0"].seen, 1.);
        assert!(!lab.engine.fresh(2.2));
        pose[4..12].copy_from_slice(&101u64.to_be_bytes());
        lab.ingest(&pose, addr, 2.3).unwrap();
        assert_eq!(lab.records["127.0.0.1:0"].seen, 2.3);
    }
    #[test]
    fn stale_discovered_trackers_can_be_assigned_but_not_calibrated() {
        let path =
            std::env::temp_dir().join(format!("slime-assignment-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut lab = legacy_lab(path.clone()).unwrap();
        for id in ["a:0", "b:0", "c:0"] {
            lab.records.insert(
                id.into(),
                Record {
                    q: [1., 0., 0., 0.],
                    seen: 1.,
                    angular: 0.,
                    has_interval: true,
                },
            );
        }
        lab.command(
            &json!({"command":"assign","trackers":{"left":"a:0","right":"b:0","torso":"c:0"}}),
            5.,
        )
        .unwrap();
        assert_eq!(legacy_lab(path.clone()).unwrap().mapping, lab.mapping);
        assert!(!lab.engine.fresh(5.));
        assert!(lab.command(&json!({"command":"calibrate"}), 5.).is_err());
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn acceleration_mode_settings_persist_and_default_linear() {
        let path =
            std::env::temp_dir().join(format!("slime-accel-mode-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut lab = legacy_lab(path.clone()).unwrap();
        assert_eq!(lab.snapshot(0.)["acceleration_input_mode"], "linear");
        lab.command(&json!({"command":"settings","axis":"z","threshold":2.,"acceleration_mode":"gravity-baseline"}), 1.).unwrap();
        assert_eq!(
            legacy_lab(path.clone()).unwrap().snapshot(0.)["acceleration_input_mode"],
            "gravity-baseline"
        );
        assert!(lab.command(&json!({"command":"settings","axis":"z","threshold":2.,"acceleration_mode":"invalid"}), 1.).is_err());
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn remap_persists_offline_address_and_disarms() {
        let path = std::env::temp_dir().join(format!("slime-remap-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut lab = legacy_lab(path.clone()).unwrap();
        lab.mapping = [
            "10.0.0.1:0".into(),
            "10.0.0.2:0".into(),
            "10.0.0.3:1".into(),
        ];
        lab.engine.armed = true;
        for ip in ["bad", "0.0.0.0", "224.0.0.1", "255.255.255.255", "10.0.0.2"] {
            assert!(lab
                .command(
                    &json!({"command":"remap_tracker","id":"10.0.0.1:0","ip":ip}),
                    1.
                )
                .is_err());
        }
        assert!(lab
            .command(
                &json!({"command":"remap_tracker","id":"10.0.0.99:0","ip":"10.0.0.7"}),
                1.
            )
            .is_err());
        lab.command(
            &json!({"command":"remap_tracker","id":"10.0.0.1:0","ip":"10.0.0.7"}),
            1.,
        )
        .unwrap();
        assert_eq!(lab.mapping[0], "10.0.0.7:0");
        assert_eq!(legacy_lab(path.clone()).unwrap().mapping, lab.mapping);
        assert!(!lab.engine.armed);
        assert!(!lab.engine.calibrated);
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn acceleration_before_rotation_and_reordered_packets() {
        let mut lab = legacy_lab(std::env::temp_dir().join("slime-accel-test.json")).unwrap();
        let addr: SocketAddr = "10.0.0.1:6969".parse().unwrap();
        let mut packet = 4u32.to_be_bytes().to_vec();
        packet.extend(2u64.to_be_bytes());
        for f in [1f32, 2., 2.] {
            packet.extend(f.to_be_bytes());
        }
        packet.push(0);
        lab.ingest(&packet, addr, 1.).unwrap();
        let snap = lab.snapshot(1.1);
        assert_eq!(snap["trackers"][0]["rotation_available"], false);
        assert_eq!(snap["trackers"][0]["acceleration_magnitude"], 3.);
        assert_eq!(snap["trackers"][0]["acceleration_count"], 1);
        assert!(!lab.engine.fresh(1.1));
        packet[4..12].copy_from_slice(&1u64.to_be_bytes());
        lab.ingest(&packet, addr, 1.2).unwrap();
        assert_eq!(lab.snapshot(1.2)["trackers"][0]["acceleration_count"], 1);
        let mut invalid = packet.clone();
        invalid[4..12].copy_from_slice(&3u64.to_be_bytes());
        invalid[12..16].copy_from_slice(&f32::NAN.to_be_bytes());
        assert!(lab.ingest(&invalid, addr, 1.3).is_err());
        assert_eq!(lab.snapshot(1.3)["trackers"][0]["acceleration_count"], 1);
    }
    #[test]
    fn hold_requires_two_real_rotation_samples() {
        let mut lab =
            legacy_lab(std::env::temp_dir().join("slime-single-sample-test.json")).unwrap();
        let addr: SocketAddr = "10.0.0.1:6969".parse().unwrap();
        lab.mapping[0] = "10.0.0.1:0".into();
        lab.sensor_status.insert("10.0.0.1:0".into(), 1);
        let mut packet = 17u32.to_be_bytes().to_vec();
        packet.extend(1u64.to_be_bytes());
        packet.extend([0, 1]);
        for f in [0f32, 0., 0., 1.] {
            packet.extend(f.to_be_bytes());
        }
        packet.push(3);
        lab.ingest(&packet, addr, 1.).unwrap();
        lab.tick(1.3, 0.);
        assert_eq!(lab.snapshot(1.3)["trackers"][0]["sample_mode"], "stale");
        assert_ne!(lab.engine.sensors[0].seen, 1.3);
        packet[4..12].copy_from_slice(&2u64.to_be_bytes());
        lab.ingest(&packet, addr, 1.4).unwrap();
        lab.tick(1.7, 0.);
        assert_eq!(lab.snapshot(1.7)["trackers"][0]["sample_mode"], "held");
        assert_eq!(lab.records["10.0.0.1:0"].seen, 1.4);
    }
    #[test]
    fn burst_same_timestamp_keeps_interval_and_held_pose() {
        let mut lab =
            legacy_lab(std::env::temp_dir().join("slime-burst-interval-test.json")).unwrap();
        let addr: SocketAddr = "10.0.0.1:6969".parse().unwrap();
        lab.mapping = [
            "10.0.0.1:0".into(),
            "10.0.0.1:1".into(),
            "10.0.0.1:2".into(),
        ];
        let mut seq = 0u64;
        for id in 0..3u8 {
            let mut info = 15u32.to_be_bytes().to_vec();
            seq += 1;
            info.extend(seq.to_be_bytes());
            info.extend([id, 1]);
            lab.ingest(&info, addr, 1.).unwrap();
            for t in [1.01, 1.02, 1.02] {
                let mut packet = 17u32.to_be_bytes().to_vec();
                seq += 1;
                packet.extend(seq.to_be_bytes());
                packet.extend([id, 1]);
                for f in [0f32, 0., 0., 1.] {
                    packet.extend(f.to_be_bytes());
                }
                packet.push(3);
                lab.ingest(&packet, addr, t).unwrap();
            }
        }
        assert!(lab.records.values().all(|r| r.has_interval));
        lab.tick(1.03, 0.);
        lab.command(&json!({"command":"calibrate"}), 1.03).unwrap();
        lab.command(&json!({"command":"arm"}), 1.03).unwrap();
        let mut heartbeat = 0u32.to_be_bytes().to_vec();
        seq += 1;
        heartbeat.extend(seq.to_be_bytes());
        lab.ingest(&heartbeat, addr, 1.5).unwrap();
        lab.tick(1.5, 0.);
        assert!(lab.engine.armed && lab.engine.fresh(1.5));
        assert!(lab.snapshot(1.5)["trackers"]
            .as_array()
            .unwrap()
            .iter()
            .all(|x| x["has_interval"] == true));
        lab.tick(4., 0.);
        assert!(!lab.engine.armed);
        assert_eq!(lab.stop_detail["stage"], "tick");
        assert_eq!(lab.stop_detail["roles"]["left"]["engine_fresh"], false);
    }
    #[test]
    fn reset_test_stops_output_and_preserves_calibration() {
        let mut lab = legacy_lab(std::env::temp_dir().join("slime-test-reset.json")).unwrap();
        lab.engine.calibrated = true;
        lab.engine.armed = true;
        lab.engine.speed = 0.5;
        lab.engine.preview_steps = [4, 7];
        lab.command(&json!({"command":"reset_test"}), 1.).unwrap();
        assert!(!lab.engine.armed && lab.engine.calibrated && lab.engine.speed == 0.);
        assert_eq!(lab.engine.preview_steps, [0, 0]);
    }
    #[test]
    fn sparse_poses_heartbeat_and_real_acceleration_events() {
        let mut lab = legacy_lab(std::env::temp_dir().join("slime-sparse-pose-test.json")).unwrap();
        let addr: SocketAddr = "10.0.0.1:6969".parse().unwrap();
        lab.mapping = [
            "10.0.0.1:0".into(),
            "10.0.0.1:1".into(),
            "10.0.0.1:2".into(),
        ];
        let mut seq = 0u64;
        fn send(lab: &mut Lab, addr: SocketAddr, seq: &mut u64, kind: u32, body: &[u8], t: f64) {
            *seq += 1;
            let mut p = kind.to_be_bytes().to_vec();
            p.extend(seq.to_be_bytes());
            p.extend(body);
            lab.ingest(&p, addr, t).unwrap();
        }
        for id in 0..3 {
            send(&mut lab, addr, &mut seq, 15, &[id, 1], 1.);
            let mut b = vec![id, 1];
            for f in [0f32, 0., 0., 1.] {
                b.extend(f.to_be_bytes());
            }
            b.push(3);
            send(&mut lab, addr, &mut seq, 17, &b, 1.01);
            send(&mut lab, addr, &mut seq, 17, &b, 1.02);
        }
        lab.tick(1.5, 0.);
        lab.command(&json!({"command":"calibrate"}), 1.5).unwrap();
        lab.command(&json!({"command":"arm"}), 1.5).unwrap();
        send(&mut lab, addr, &mut seq, 0, &[], 1.8);
        lab.tick(1.8, 0.);
        assert!(lab.engine.armed);
        assert_eq!(lab.engine.speed, 0.);
        assert_eq!(lab.snapshot(1.8)["trackers"][1]["sample_mode"], "held");
        let acceleration = |id, x: f32| {
            let mut b = vec![];
            for f in [x, 0., 0.] {
                b.extend(f.to_be_bytes());
            }
            b.push(id);
            b
        };
        send(&mut lab, addr, &mut seq, 4, &acceleration(0, 1.5), 1.81);
        send(&mut lab, addr, &mut seq, 4, &acceleration(1, 1.5), 2.01);
        lab.tick(2.01, 0.);
        assert!(lab.engine.armed && lab.engine.speed > 0.);
        let count = lab.engine.preview_steps;
        send(&mut lab, addr, &mut seq, 0, &[], 2.7);
        lab.tick(2.7, 0.);
        assert_eq!(lab.engine.preview_steps, count);
        assert_eq!(lab.engine.speed, 0.);
        lab.tick(5., 0.);
        assert!(!lab.engine.armed && lab.engine.speed == 0.);
        assert!(lab.stop_reason.contains("signal"));
    }
    #[test]
    fn healthy_stationary_hold_and_faults() {
        let path = std::env::temp_dir().join("slime-hold-test.json");
        let mut lab = legacy_lab(path).unwrap();
        for i in 0..3 {
            let ip = format!("10.0.0.{}", i + 1);
            let id = format!("{ip}:0");
            lab.mapping[i] = id.clone();
            lab.sensor_status.insert(id.clone(), 1);
            lab.records.insert(
                id,
                Record {
                    q: [1., 0., 0., 0.],
                    seen: 1.,
                    angular: 0.,
                    has_interval: true,
                },
            );
            lab.peers.insert(format!("{ip}:6969").parse().unwrap(), 1.5);
        }
        lab.tick(1.5, 0.);
        assert!(lab.engine.fresh(1.5));
        lab.engine.calibrate(1.5).unwrap();
        lab.engine.arm(1.5).unwrap();
        lab.tick(1.6, 0.);
        assert!(lab.engine.armed);
        assert!(lab.engine.steps.is_empty());
        assert_eq!(lab.snapshot(1.6)["trackers"][0]["sample_mode"], "held");
        lab.records.get_mut("10.0.0.1:0").unwrap().angular = lab.engine.cfg.release + 1.;
        lab.tick(1.9, 0.);
        assert!(lab.engine.armed);
        lab.sensor_status.insert("10.0.0.1:0".into(), 1);
        lab.engine.armed = true;
        lab.engine.calibrated = true;
        let addr: SocketAddr = "10.0.0.1:6969".parse().unwrap();
        let mut error = 15u32.to_be_bytes().to_vec();
        error.extend(1u64.to_be_bytes());
        error.extend([0, 2]);
        lab.ingest(&error, addr, 2.).unwrap();
        assert!(!lab.engine.armed && !lab.engine.calibrated);
        lab.sensor_status.insert("10.0.0.1:0".into(), 1);
        lab.engine.armed = true;
        lab.engine.calibrated = true;
        let mut hello = 3u32.to_be_bytes().to_vec();
        hello.extend(2u64.to_be_bytes());
        lab.ingest(&hello, addr, 2.1).unwrap();
        assert!(!lab.engine.armed && !lab.engine.calibrated);
        assert!(!lab.records.contains_key("10.0.0.1:0"));
    }
    #[test]
    fn legacy_probe_upgrades_same_sequence_and_v2_output_is_neutral_until_armed() {
        let mut l = Lab::new(std::env::temp_dir().join("vrcraft-v2-protocol-test.json")).unwrap();
        l.output_mode = "steamvr".into();
        l.engine.cfg.continuous = false;
        for i in 0..3 {
            l.engine.update(i, crate::gait::IDENTITY, 1.).unwrap();
        }
        let probe = l.steam_packet(1.);
        assert_eq!(&probe[..4], b"ALOC");
        l.steam_sent(Ok(20));
        let addr = ([127, 0, 0, 1], 29741).into();
        let mut a = vec![0u8; 16];
        a[..4].copy_from_slice(b"ALAK");
        a[4..12].copy_from_slice(&probe[4..12]);
        a[12..16].copy_from_slice(&7u32.to_le_bytes());
        l.steam_ack(&a, addr, 1.);
        let mut b = vec![0u8; 32];
        b[..4].copy_from_slice(b"AKV2");
        b[4..16].copy_from_slice(&a[4..16]);
        for (i, x) in [1.7f32, 0., 0., 0.].iter().enumerate() {
            b[16 + i * 4..20 + i * 4].copy_from_slice(&x.to_le_bytes());
        }
        l.steam_ack(&b, addr, 1.);
        assert!(l.protocol_v2 && l.hmd.is_some());
        let neutral = l.steam_packet(1.01);
        assert_eq!(&neutral[..4], b"ALC2");
        assert!(neutral[12..40].iter().all(|x| *x == 0));
        l.steam_sent(Ok(40));
        l.steam_ack(&b, addr, 1.01);
        assert_eq!(l.v2_sequence, Some(0));
        let mut invalid = b.clone();
        invalid[4..8].copy_from_slice(&1u32.to_le_bytes());
        invalid[16..20].copy_from_slice(&f32::NAN.to_le_bytes());
        l.steam_ack(&invalid, addr, 1.02);
        assert!(!l.driver_ready(1.02));
        assert!(l.hmd.is_none());
    }
    #[test]
    fn legacy_gesture_mapping_never_emits_buttons() {
        let path =
            std::env::temp_dir().join(format!("vrcraft-gesture-test-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut l = Lab::new(path.clone()).unwrap();
        l.command(&json!({"command":"gesture_settings","settings":{"enabled":true,"mappings":[{"source":"left","channel":"accel_x","op":"above","trigger":2.,"release":1.,"dwell_seconds":0.,"mode":"hold","pulse_seconds":0.15,"action":"jump"}]}}),0.).unwrap();
        l.mapping = ["a:0".into(), "b:0".into(), "c:0".into()];
        l.engine.cfg.require_steps = false;
        l.engine.sensors[2].seen = 1.;
        l.engine.calibrated = true;
        l.engine.armed = true;
        l.output_mode = "steamvr".into();
        l.protocol_v2 = true;
        l.neutral_pending = false;
        l.ack_seen = 1.;
        l.ack_flags = 7;
        l.accelerations.insert(
            "a:0".into(),
            Acceleration {
                vector: [3., 0., 0.],
                seen: 1.,
                count: 1,
            },
        );
        let p = l.steam_packet(1.);
        assert_eq!(u32::from_le_bytes(p[28..32].try_into().unwrap()), 0);
        l.accelerations.get_mut("a:0").unwrap().vector = [0., 0., 0.];
        l.accelerations.get_mut("a:0").unwrap().seen = 1.01;
        l.steam_packet(1.01);
        l.accelerations.get_mut("a:0").unwrap().vector = [3., 0., 0.];
        l.accelerations.get_mut("a:0").unwrap().seen = 1.02;
        let p = l.steam_packet(1.02);
        assert_eq!(u32::from_le_bytes(p[28..32].try_into().unwrap()) & 1, 0);
        l.engine.sensors[2].seen = 1.4;
        l.ack_seen = 1.4;
        let p = l.steam_packet(1.4);
        assert_eq!(u32::from_le_bytes(p[28..32].try_into().unwrap()) & 1, 0);
        l.command(&json!({"command":"stop"}), 1.4).unwrap();
        let p = l.steam_packet(1.4);
        assert!(p[12..40].iter().all(|x| *x == 0));
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn legacy_and_v2_forward_have_same_positive_sign() {
        let mut l = Lab::new(std::env::temp_dir().join("vrcraft-v2-sign-test.json")).unwrap();
        l.output_mode = "steamvr".into();
        l.engine.cfg.continuous = false;
        for i in 0..3 {
            l.engine.update(i, crate::gait::IDENTITY, 1.).unwrap();
        }
        l.engine.calibrated = true;
        l.engine.armed = true;
        l.engine.speed = 0.5;
        l.neutral_pending = false;
        l.ack_seen = 1.;
        l.ack_flags = 7;
        let legacy = l.steam_packet(1.01);
        let speed = f32::from_le_bytes(legacy[12..16].try_into().unwrap());
        l.protocol_v2 = true;
        let extended = l.steam_packet(1.01);
        let y = f32::from_le_bytes(extended[16..20].try_into().unwrap());
        assert!(speed > 0.);
        assert_eq!(y, speed);
    }
    #[test]
    fn v2_tilt_output_requires_arm_and_stale_hmd_releases_turn() {
        let path = std::env::temp_dir().join("vrcraft-v2-tilt-test.json");
        let mut l = Lab::new(path).unwrap();
        l.output_mode = "steamvr".into();
        l.protocol_v2 = true;
        l.movement_engine = "body_tilt".into();
        l.engine.cfg.require_steps = false;
        l.mapping[2] = "10.0.0.1:0".into();
        l.peer_macs.insert("10.0.0.1".into(), "02:00:00:80:BE:03".into());
        l.learned_mounting_identity = Some("02:00:00:80:BE:03:0".into());
        let mut settings = crate::actions::Settings::default();
        settings.direction.enabled = true;
        settings.turn.enabled = true;
        l.actions = crate::actions::ActionProcessor::new(settings).unwrap();
        l.records.insert(
            l.mapping[2].clone(),
            Record {
                q: [1., 0., 0., 0.],
                seen: 1.,
                angular: 0.,
                has_interval: true,
            },
        );
        l.engine.update(2, [1., 0., 0., 0.], 1.).unwrap();
        l.hmd = Some(crate::actions::TimedHmd {
            height: 1.7,
            yaw: 0.,
            pitch: 0.,
            roll: 0.,
            seen: 1.,
        });
        l.command(&json!({"command":"calibrate"}), 1.).unwrap();
        let p = l.steam_packet(1.01);
        assert!(p[12..40].iter().all(|v| *v == 0));
        l.steam_sent(Ok(40));
        let mut a = vec![0u8; 32];
        a[..4].copy_from_slice(b"AKV2");
        a[4..12].copy_from_slice(&p[4..12]);
        a[12..16].copy_from_slice(&7u32.to_le_bytes());
        for (i, x) in [1.7f32, 0., 0., 0.].iter().enumerate() {
            a[16 + i * 4..20 + i * 4].copy_from_slice(&x.to_le_bytes());
        }
        l.steam_ack(&a, ([127, 0, 0, 1], 29741).into(), 1.01);
        l.command(&json!({"command":"arm"}), 1.01).unwrap();
        let q = [(0.2f64).cos(), (0.2f64).sin(), 0., 0.];
        l.records.get_mut(&l.mapping[2]).unwrap().q = q;
        l.records.get_mut(&l.mapping[2]).unwrap().seen = 1.1;
        l.engine.sensors[2].seen = 1.1;
        l.engine.sensors[2].q = q;
        l.hmd = Some(crate::actions::TimedHmd {
            height: 1.7,
            yaw: 0.5,
            pitch: 0.,
            roll: 0.,
            seen: 1.1,
        });
        let p = l.steam_packet(1.1);
        assert_eq!(&p[..4], b"ALC2");
        assert!(f32::from_le_bytes(p[16..20].try_into().unwrap()) > 0.);
        assert!(f32::from_le_bytes(p[20..24].try_into().unwrap()) > 0.);
        assert_eq!(u32::from_le_bytes(p[32..36].try_into().unwrap()), 1);
        l.hmd.as_mut().unwrap().seen = 0.;
        let p = l.steam_packet(1.11);
        assert_eq!(f32::from_le_bytes(p[20..24].try_into().unwrap()), 0.);
        l.engine.disarm();
        let p = l.steam_packet(1.12);
        assert!(p[12..40].iter().all(|v| *v == 0));
    }
    #[test]
    fn tilt_accepts_chest_only_assignment_and_calibration_without_feet(){
        let dir=std::env::temp_dir().join(format!("arcane-tilt-chest-only-{}",std::process::id()));std::fs::create_dir_all(&dir).unwrap();
        let mut l=Lab::new(dir.join("config.json")).unwrap();
        l.command(&json!({"command":"movement_engine","engine":"body_tilt"}),0.).unwrap();
        l.records.insert("192.0.2.12:0".into(),Record{q:[1.,0.,0.,0.],seen:1.,angular:0.,has_interval:true});
        l.command(&json!({"command":"assign","trackers":{"left":"","right":"","torso":"192.0.2.12:0"}}),1.).unwrap();
        l.engine.update(2,[1.,0.,0.,0.],1.).unwrap();
        assert!(l.engine.fresh(1.));
        l.command(&json!({"command":"calibrate"}),1.).unwrap();assert!(l.engine.calibrated);assert!(!l.engine.armed);
        assert!(!l.engine.fresh(2.));
        assert!(l.command(&json!({"command":"calibrate"}),2.).is_err());
        l.command(&json!({"command":"movement_engine","engine":"chest_heading_steps"}),2.).unwrap();
        assert!(l.command(&json!({"command":"assign","trackers":{"left":"","right":"","torso":"192.0.2.12:0"}}),2.).is_err());
    }
    #[test]
    fn sensor_layouts_persist_roles_and_require_thighs(){
        // Windows reuses process IDs. A previous run's saved thigh assignments
        // must not become this test's supposedly empty starting configuration.
        let nonce=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir=std::env::temp_dir().join(format!("arcane-layout-test-{}-{nonce}",std::process::id()));std::fs::create_dir(&dir).unwrap();let path=dir.join("config.json");let mut l=Lab::new(path.clone()).unwrap();
        l.command(&json!({"command":"sensor_layout","count":1}),0.).unwrap();assert_eq!(l.layout_count(),1);assert!(!l.engine.cfg.require_steps);
        for id in ["a","b","c","d","e"]{l.records.insert(id.into(),Record{q:[1.,0.,0.,0.],seen:1.,angular:0.,has_interval:true});}
        l.command(&json!({"command":"assign_sensor_role","role":"torso","id":"a"}),1.).unwrap();
        l.command(&json!({"command":"sensor_layout","count":5}),1.).unwrap();assert!(!l.thighs_ready(1.));
        for (role,id) in [("left","b"),("right","c"),("left_thigh","d"),("right_thigh","e")]{l.command(&json!({"command":"assign_sensor_role","role":role,"id":id}),1.).unwrap();}
        assert!(l.thighs_ready(1.));assert!(!l.thighs_ready(4.));let restored=Lab::new(path).unwrap();assert_eq!(restored.layout_count(),5);assert_eq!(restored.thighs,["d","e"]);
        l.command(&json!({"command":"assign_sensor_role","role":"torso","id":"b"}),1.).unwrap();assert_eq!(l.mapping[0],"");assert_eq!(l.mapping[2],"b");assert!(!l.engine.armed);
        assert!(l.command(&json!({"command":"sensor_layout","count":2}),1.).is_err());
    }
    #[test]
    fn profiles_save_and_load_preserve_mapping_and_disarm() {
        let dir = std::env::temp_dir().join(format!("vrcraft-lab-profiles-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut l = Lab::new(dir.join("config.json")).unwrap();
        l.mapping = ["a".into(), "b".into(), "c".into()];
        l.command(
            &json!({"command":"profile_save","name":"Safe","description":"test"}),
            1.,
        )
        .unwrap();
        assert_eq!(l.profile_list.len(), 1);
        l.engine.armed = true;
        l.command(&json!({"command":"profile_load","name":"Safe"}), 1.)
            .unwrap();
        assert!(!l.engine.armed && !l.engine.calibrated);
        assert_eq!(l.mapping, ["a", "b", "c"]);
        l.command(&json!({"command":"profile_delete","name":"Safe"}), 1.)
            .unwrap();
        assert!(l.profile_list.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn builder_profile_persists_layout_and_preserves_hardware() {
        let dir=std::env::temp_dir().join(format!("arcane-builder-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&dir).unwrap();let path=dir.join("config.json");let mut l=Lab::new(path.clone()).unwrap();
        l.mapping=["left".into(),"right".into(),"chest".into()];l.thighs=["thigh1".into(),"thigh2".into()];l.engine.armed=true;
        let choices=json!({"lean":true,"steps":true,"backward":false,"strafe":true,"jump":false,"crouch":false,"sprint":false,"thighs":true,"head_turn":false,"max_output":0.6});
        l.command(&json!({"command":"apply_movement_profile","name":"Five","choices":choices}),0.).unwrap();
        assert_eq!(l.layout_count(),5);assert!(!l.engine.armed&&!l.engine.calibrated);assert_eq!(l.mapping,["left","right","chest"]);assert_eq!(l.thighs,["thigh1","thigh2"]);
        let p=l.profile_store().load("Five").unwrap();assert_eq!(p["sensor_layout"],json!({"count":5}));assert!(p.to_string().find("thigh1").is_none());
        let restored=Lab::new(path).unwrap();assert_eq!(restored.layout_count(),5);assert_eq!(restored.mapping,l.mapping);assert_eq!(restored.thighs,l.thighs);
        let mut bad=choices.clone();bad["steps"]=json!(false);assert!(l.command(&json!({"command":"apply_movement_profile","choices":bad}),0.).is_err());assert_eq!(l.layout_count(),5);
        let _=std::fs::remove_dir_all(dir);
    }
    fn ack(l: &mut Lab, t: f64, flags: u32) {
        let packet = l.steam_packet(t);
        l.steam_sent(Ok(20));
        let mut a = [0u8; 16];
        a[..4].copy_from_slice(b"ALAK");
        a[4..12].copy_from_slice(&packet[4..12]);
        a[12..].copy_from_slice(&flags.to_le_bytes());
        l.steam_ack(&a, ([127, 0, 0, 1], 29741).into(), t);
    }
    #[test]
    fn steamvr_requires_neutral_ack_and_fresh_live_input() {
        let mut l = legacy_lab(PathBuf::from("nonexistent-test-config.json")).unwrap();
        l.output_mode = "steamvr".into();
        for i in 0..3 {
            l.engine.update(i, crate::gait::IDENTITY, 1.).unwrap();
        }
        l.command(&json!({"command":"calibrate"}), 1.).unwrap();
        assert!(l.command(&json!({"command":"arm"}), 1.).is_err());
        ack(&mut l, 1., 7);
        l.command(&json!({"command":"arm"}), 1.).unwrap();
        l.engine.speed = 0.5;
        let p = l.steam_packet(1.1);
        assert_eq!(f32::from_le_bytes(p[12..16].try_into().unwrap()), 0.5);
        l.demo = true;
        assert_eq!(
            f32::from_le_bytes(l.steam_packet(1.1)[12..16].try_into().unwrap()),
            0.
        );
        l.demo = false;
        l.tick(1.26, 0.);
        assert!(!l.engine.armed);
        assert_eq!(
            f32::from_le_bytes(l.steam_packet(1.26)[12..16].try_into().unwrap()),
            0.
        );
    }
    #[test]
    fn steamvr_rejects_wrong_ack_and_disarms_on_hmd_loss() {
        let mut l = legacy_lab(PathBuf::from("nonexistent-test-config.json")).unwrap();
        l.output_mode = "steamvr".into();
        for i in 0..3 {
            l.engine.update(i, crate::gait::IDENTITY, 1.).unwrap();
        }
        l.command(&json!({"command":"calibrate"}), 1.).unwrap();
        let p = l.steam_packet(1.);
        let mut a = [0u8; 16];
        a[..4].copy_from_slice(b"ALAK");
        a[4..12].copy_from_slice(&p[4..12]);
        a[12..].copy_from_slice(&7u32.to_le_bytes());
        l.steam_ack(&a, ([127, 0, 0, 1], 1234).into(), 1.);
        assert!(!l.driver_ready(1.));
        ack(&mut l, 1., 7);
        l.command(&json!({"command":"arm"}), 1.).unwrap();
        ack(&mut l, 1.01, 1);
        assert!(!l.engine.armed);
        assert!(!l.engine.calibrated);
        ack(&mut l, 1.02, 7);
        assert!(l.command(&json!({"command":"arm"}), 1.02).is_err());
        let generation = l.generation;
        assert!(l.command(&json!({"command":"calibrate"}), 2.).is_err());
        assert!(!l.engine.calibrated);
        assert_ne!(l.generation, generation);
    }
    #[test]
    fn late_ack_invalidates_calibration_before_it_can_restore_readiness() {
        let mut l = legacy_lab(PathBuf::from("nonexistent-test-config.json")).unwrap();
        l.output_mode = "steamvr".into();
        for i in 0..3 {
            l.engine.update(i, crate::gait::IDENTITY, 1.).unwrap();
        }
        l.command(&json!({"command":"calibrate"}), 1.).unwrap();
        ack(&mut l, 1., 7);
        l.command(&json!({"command":"arm"}), 1.).unwrap();
        ack(&mut l, 1.26, 7);
        assert!(!l.engine.armed && !l.engine.calibrated);
        assert!(l.command(&json!({"command":"arm"}), 1.26).is_err());
    }
    #[test]
    fn demo_gait_and_stop() {
        let mut l = legacy_lab(PathBuf::from("nonexistent-test-config.json")).unwrap();
        l.command(&json!({"command":"mode","mode":"demo"}), 0.)
            .unwrap();
        l.tick(0., 0.);
        l.command(&json!({"command":"calibrate"}), 0.).unwrap();
        l.command(&json!({"command":"arm"}), 0.).unwrap();
        l.command(&json!({"command":"demo","walk":true,"yaw":90}), 0.)
            .unwrap();
        for n in 1..150 {
            l.tick(n as f64 * 0.02, 0.);
        }
        assert!(l.engine.speed > 0.);
        assert!((l.heading - std::f64::consts::FRAC_PI_2).abs() < 1e-6);
        l.command(&json!({"command":"stop"}), 3.).unwrap();
        l.tick(3., 0.);
        assert_eq!(l.engine.speed, 0.);
    }
    #[test]
    fn invalid_commands_and_stale() {
        let mut l = legacy_lab(PathBuf::from("nonexistent-test-config.json")).unwrap();
        for v in [
            json!(null),
            json!([]),
            json!({"command":"arm"}),
            json!({"command":"assign","trackers":[]}),
        ] {
            assert!(l.command(&v, 0.).is_err());
        }
    }
    #[test]
    fn held_pose_is_available_only_with_healthy_peer() {
        let mut l = Lab::new(std::env::temp_dir().join("held-action-pose-test.json")).unwrap();
        l.mapping[2] = "10.1.2.3:0".into();
        l.records.insert(
            l.mapping[2].clone(),
            Record {
                q: [1., 0., 0., 0.],
                seen: 0.,
                angular: 0.,
                has_interval: true,
            },
        );
        l.sensor_status.insert(l.mapping[2].clone(), 1);
        l.peers.insert(([10, 1, 2, 3], 6969).into(), 1.);
        l.refresh_held_poses(1.);
        assert!(l.action_input(1.).chest.is_some());
        assert_eq!(l.action_input(1.).chest.unwrap().seen, 1.);
        assert!(l.gesture_input(1.).chest.rotation.is_some());
        assert!(l.action_input(3.01).chest.is_none());
        l.sensor_status.insert(l.mapping[2].clone(), 2);
        assert!(l.action_input(1.01).chest.is_none());
    }
    #[test]
    fn unsupported_output_rejects_advanced_actions() {
        let mut l = Lab::new(std::env::temp_dir().join("advanced-output-arm-test.json")).unwrap();
        l.movement_engine = "body_tilt".into();
        assert!(l
            .command(&json!({"command":"arm"}), 1.)
            .unwrap_err()
            .contains("protocol 2"));
        l.output_mode = "steamvr".into();
        assert!(l
            .command(&json!({"command":"arm"}), 1.)
            .unwrap_err()
            .contains("protocol 2"));
    }
}
