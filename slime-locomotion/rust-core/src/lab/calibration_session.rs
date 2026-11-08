//! A draft owns fresh measurements. Only a completed draft can replace a mounting.
use super::*;
use crate::actions::{ActionProcessor, Settings, Telemetry};

pub(super) struct Session {
    id: String,
    identity: String,
    stage: &'static str,
    pub(super) processor: ActionProcessor,
    pub preview: Telemetry,
    samples: Vec<Value>,
}

impl Session {
    fn process(&mut self, input:&crate::actions::Input) {
        self.preview=self.processor.process_chest_preview(input);
        if self.processor.settings().direction.learned_right.is_none() {
            if let Some(angles)=self.preview.raw_direction_angles.as_mut() { angles[0]=0.; }
            self.preview.travel_xy[0]=0.;
        }
        if self.processor.settings().direction.learned_forward.is_none() {
            self.preview.raw_direction_angles=None;
            self.preview.travel_xy=[0.;2];
        }
    }
    fn tilt(&self, q:Q, up:[f64;3]) -> Option<[f64;3]> {
        let center:Q = serde_json::from_value(self.samples.first()?["quaternion"].clone()).ok()?;
        crate::mounting::level_tilt(q,center,up)
    }

    // Before side learning, infer the side axis from the new forward movement
    // and world up. Do not reuse an earlier tracker mounting.
    fn visual_lean(&self, q:Q, up:[f64;3]) -> Option<[f64;2]> {
        let tilt=self.tilt(q,up)?;
        let forward=self.processor.settings().direction.learned_forward?;
        if self.processor.settings().direction.learned_right.is_some() {
            return self.preview.raw_direction_angles;
        }
        let center:Q=serde_json::from_value(self.samples.first()?["quaternion"].clone()).ok()?;
        let length=center.iter().map(|v|v*v).sum::<f64>().sqrt();
        if !length.is_finite() || length<1e-9 {return None;}
        let [w,x,y,z]=center.map(|v|v/length);
        let inverse=[-x,-y,-z];
        let cross=|a:[f64;3],b:[f64;3]| [a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0]];
        let first=cross(inverse,up);
        let inner=std::array::from_fn(|i|first[i]+w*up[i]);
        let turn=cross(inverse,inner);
        let local_up=std::array::from_fn(|i|up[i]+2.*turn[i]);
        let right=cross(forward,local_up);
        let n=right.iter().map(|v|v*v).sum::<f64>().sqrt();
        if n<1e-8 {return None;}
        let x=tilt.iter().zip(right).map(|(a,b)|a*b/n).sum();
        let y=tilt.iter().zip(forward).map(|(a,b)|a*b).sum();
        Some([x,y])
    }

    fn learn_named_lean(&mut self, stage:&str, q:Q, up:[f64;3]) -> Result<(),String> {
        let tilt=self.tilt(q,up).ok_or("Capture upright center first")?;
        let length=tilt.iter().map(|v|v*v).sum::<f64>().sqrt();
        if !(5_f64.to_radians()..=35_f64.to_radians()).contains(&length) {
            return Err(format!("Lean {stage} 5 to 35 degrees after the cue"));
        }
        let mut direction=tilt.map(|v|v/length);
        let mut settings=self.processor.settings().clone();
        let d=&mut settings.direction;
        match stage {
            "forward" => { d.learned_forward=Some(direction); d.learned_right=None; }
            "backward" => {
                direction=direction.map(|v|-v);
                let first=d.learned_forward.ok_or("Capture forward first")?;
                d.learned_forward=Some(combine_opposites(first,direction)?);
            }
            "left" | "right" => {
                let forward=d.learned_forward.ok_or("Capture forward first")?;
                let overlap=direction.iter().zip(forward).map(|(a,b)|a*b).sum::<f64>();
                direction=std::array::from_fn(|i|direction[i]-overlap*forward[i]);
                let side=direction.iter().map(|v|v*v).sum::<f64>().sqrt();
                if side<0.7 { return Err(format!("Lean sideways {stage}, keeping forward lean small")); }
                direction=direction.map(|v|v/side);
                d.learned_right=Some(if stage=="left" {
                    direction.map(|v|-v)
                } else {
                    combine_opposites(d.learned_right.ok_or("Capture left first")?,direction)?
                });
            }
            _=>return Err("Unknown lean".into()),
        }
        self.processor.update_settings_preserving_center(settings).map_err(str::to_owned)
    }
}

fn combine_opposites(first:[f64;3], second:[f64;3]) -> Result<[f64;3],String> {
    if first.iter().zip(second).map(|(a,b)|a*b).sum::<f64>()<0.5 {
        return Err("The two movements were not opposite. Return upright and follow the next cue.".into());
    }
    let sum:[f64;3]=std::array::from_fn(|i|first[i]+second[i]);
    let length=sum.iter().map(|v|v*v).sum::<f64>().sqrt();
    Ok(sum.map(|v|v/length))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Lab, PathBuf) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!("arcane-session-{}-{}-{}", NEXT.fetch_add(1,std::sync::atomic::Ordering::Relaxed),
            std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir(&dir).unwrap();
        let mut lab = Lab::new(dir.join("config.json")).unwrap();
        lab.auto_add = false;
        lab.movement_engine = "body_tilt".into();
        lab.engine.cfg.require_steps = false;
        lab.mapping[2] = "127.0.0.1:0".into();
        lab.peer_macs.insert("127.0.0.1".into(), "02:00:00:00:00:01".into());
        let mut old = lab.actions.settings().clone();
        old.direction.enabled = true;
        old.direction.pitch_sign = -1.;
        old.direction.roll_sign = -1.;
        old.direction.learned_forward = Some([0.,1.,0.]);
        old.direction.learned_right = Some([1.,0.,0.]);
        lab.actions = ActionProcessor::new(old).unwrap();
        lab.learned_mounting_identity = Some("02:00:00:00:00:01:0".into());
        lab.remember_device_mounting("02:00:00:00:00:01:0");
        lab.save(&lab.mapping,&lab.engine.cfg,&lab.output_mode).unwrap();
        (lab,dir)
    }

    fn pose(lab:&mut Lab, q:Q, t:f64) {
        lab.records.insert(lab.mapping[2].clone(),Record{q,seen:t,angular:0.,has_interval:true});
        lab.engine.update(2,q,t).unwrap();
    }

    fn begin(lab:&mut Lab, id:&str, t:f64) {
        pose(lab,crate::gait::IDENTITY,t);
        lab.command(&json!({"command":"sensor_calibration_begin","session":id}),t).unwrap();
    }

    fn directions(lab:&mut Lab, id:&str, t:f64) {
        directions_mounted(lab,id,t,false);
    }

    fn directions_mounted(lab:&mut Lab, id:&str, t:f64, flipped:bool) {
        let c=0.15_f64.cos(); let s=0.15_f64.sin();
        for (i,(stage,q)) in [
            ("center",[1.,0.,0.,0.]),("forward",[c,-s,0.,0.]),("backward",[c,s,0.,0.]),
            ("left",[c,0.,-s,0.]),("right",[c,0.,s,0.])
        ].into_iter().enumerate() {
            let time=t+i as f64;
            let q = if flipped {[-q[3],q[2],-q[1],q[0]]} else {q};
            pose(lab,q,time);
            lab.command(&json!({"command":"sensor_calibration_capture","session":id,"stage":stage}),time).unwrap();
        }
    }

    #[test]
    fn draft_is_fresh_cancel_and_restart_preserve_original() {
        let (mut lab,dir)=fixture();
        let original=fs::read(&lab.config).unwrap();
        begin(&mut lab,"cancel",1.);
        let d=&lab.sensor_calibration.as_ref().unwrap().processor.settings().direction;
        assert_eq!((d.learned_forward,d.learned_right),(None,None));
        assert_eq!((d.pitch_sign,d.roll_sign),(1.,1.));
        assert!(lab.command(&json!({"command":"arm"}),1.).is_err());
        assert!(lab.command(&json!({"command":"sensor_calibration_capture","session":"cancel","stage":"finish"}),1.).is_err());
        directions(&mut lab,"cancel",2.);
        assert_eq!(fs::read(&lab.config).unwrap(),original);
        lab.command(&json!({"command":"sensor_calibration_cancel","session":"cancel"}),7.).unwrap();
        assert_eq!(lab.actions.settings().direction.pitch_sign,-1.);
        begin(&mut lab,"partial",8.);
        directions(&mut lab,"partial",9.);
        let restored=Lab::new(lab.config.clone()).unwrap();
        assert!(restored.sensor_calibration.is_none());
        assert_eq!(restored.actions.settings().direction.pitch_sign,-1.);
        assert_eq!(fs::read(&lab.config).unwrap(),original);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_activation_keeps_old_settings_and_retry_commits_one_archive() {
        let (mut lab,dir)=fixture();
        let original=fs::read(&lab.config).unwrap();
        begin(&mut lab,"save",1.);
        directions(&mut lab,"save",2.);
        pose(&mut lab,crate::gait::IDENTITY,7.);
        let obstruction=lab.config.with_extension("tmp");
        fs::create_dir(&obstruction).unwrap();
        let finish=json!({"command":"sensor_calibration_capture","session":"save","stage":"finish"});
        assert!(lab.command(&finish,7.).is_err());
        assert_eq!(lab.actions.settings().direction.pitch_sign,-1.);
        assert_eq!(fs::read(&lab.config).unwrap(),original);
        assert!(lab.sensor_calibration.is_some());
        fs::remove_dir(&obstruction).unwrap();
        pose(&mut lab,crate::gait::IDENTITY,8.);
        lab.command(&finish,8.).unwrap();
        lab.command(&finish,8.).unwrap();
        assert!(!lab.engine.armed);
        assert!(lab.sensor_calibration.is_none());
        assert_eq!(fs::read_dir(dir.join("calibrations")).unwrap().count(),1);
        let restored=Lab::new(lab.config.clone()).unwrap();
        let d=&restored.actions.settings().direction;
        assert_eq!((d.pitch_sign,d.roll_sign),(1.,1.));
        assert!((d.learned_forward.unwrap()[0]+1.).abs()<1e-6);
        assert!((d.learned_right.unwrap()[1]-1.).abs()<1e-6);
        assert_eq!(restored.mapping,lab.mapping);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn changed_sensor_and_stale_packets_cannot_finish_draft() {
        let (mut lab,dir)=fixture();
        begin(&mut lab,"identity",1.);
        directions(&mut lab,"identity",2.);
        let finish=json!({"command":"sensor_calibration_capture","session":"identity","stage":"finish"});
        assert!(lab.command(&finish,20.).is_err());
        pose(&mut lab,crate::gait::IDENTITY,21.);
        lab.peer_macs.insert("127.0.0.1".into(),"02:00:00:00:00:02".into());
        assert!(lab.command(&finish,21.).is_err());
        assert!(!dir.join("calibrations").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn recalibration_after_flipping_sensor_replaces_axes_and_preserves_archive() {
        let (mut lab,dir)=fixture();
        begin(&mut lab,"first",1.);
        directions(&mut lab,"first",2.);
        pose(&mut lab,crate::gait::IDENTITY,7.);
        lab.command(&json!({"command":"sensor_calibration_capture","session":"first","stage":"finish"}),7.).unwrap();
        let first=fs::read(dir.join("calibrations/calibration-first.json")).unwrap();
        let old_forward=lab.actions.settings().direction.learned_forward.unwrap();
        begin(&mut lab,"flipped",8.);
        directions_mounted(&mut lab,"flipped",9.,true);
        pose(&mut lab,[0.,0.,0.,1.],14.);
        lab.command(&json!({"command":"sensor_calibration_capture","session":"flipped","stage":"finish"}),14.).unwrap();
        let new_forward=lab.actions.settings().direction.learned_forward.unwrap();
        assert!(old_forward.iter().zip(new_forward).map(|(a,b)|a*b).sum::<f64>() < -0.99);
        assert_eq!(first,fs::read(dir.join("calibrations/calibration-first.json")).unwrap());
        assert_eq!(fs::read_dir(dir.join("calibrations")).unwrap().count(),2);
        let c=0.15_f64.cos(); let s=0.15_f64.sin();
        pose(&mut lab,[0.,0.,s,c],15.);
        lab.tick(15.,0.);
        assert!(lab.action_preview.travel_xy[1]>0.1);
        let restored=Lab::new(lab.config.clone()).unwrap();
        assert_eq!(restored.actions.settings().direction.learned_forward,Some(new_forward));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn named_left_movement_defines_left_even_when_sensor_axis_is_positive() {
        let (mut lab,dir)=fixture();
        begin(&mut lab,"left-cue",1.);
        let c=0.15_f64.cos(); let s=0.15_f64.sin();
        for (index,(stage,q)) in [
            ("center",[1.,0.,0.,0.]),("forward",[c,-s,0.,0.]),
            ("backward",[c,s,0.,0.]),("left",[c,0.,s,0.])
        ].into_iter().enumerate() {
            let t=2.+index as f64;
            pose(&mut lab,q,t);
            lab.command(&json!({"command":"sensor_calibration_capture","session":"left-cue","stage":stage}),t).unwrap();
        }
        assert!(lab.sensor_calibration.as_ref().unwrap().preview.raw_direction_angles.unwrap()[0] < -0.2);
        pose(&mut lab,[c,0.,-s,0.],6.);
        lab.command(&json!({"command":"sensor_calibration_capture","session":"left-cue","stage":"right"}),6.).unwrap();
        assert!(lab.sensor_calibration.as_ref().unwrap().preview.raw_direction_angles.unwrap()[0]>0.2);
        pose(&mut lab,[c,0.,s,0.],7.);
        lab.tick(7.,0.);
        assert!(lab.action_preview.travel_xy[0] < -0.1);
        assert_eq!(lab.sensor_calibration.as_ref().unwrap().stage,"finish");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn visual_lean_shows_live_side_after_forward_for_multiple_mounts() {
        for flipped in [false,true] {
            let (mut lab,dir)=fixture();
            begin(&mut lab,"visual",1.);
            let c=0.15_f64.cos(); let s=0.15_f64.sin();
            let mount=|q:Q| if flipped {[-q[3],q[2],-q[1],q[0]]} else {q};
            pose(&mut lab,mount([1.,0.,0.,0.]),2.);
            lab.command(&json!({"command":"sensor_calibration_capture","session":"visual","stage":"center"}),2.).unwrap();
            assert!(lab.sensor_calibration.as_ref().unwrap().visual_lean(mount([c,0.,-s,0.]),[0.,0.,1.]).is_none());
            pose(&mut lab,mount([c,-s,0.,0.]),3.);
            lab.command(&json!({"command":"sensor_calibration_capture","session":"visual","stage":"forward"}),3.).unwrap();
            let draft=lab.sensor_calibration.as_ref().unwrap();
            let left=draft.visual_lean(mount([c,0.,-s,0.]),[0.,0.,1.]).unwrap();
            let right=draft.visual_lean(mount([c,0.,s,0.]),[0.,0.,1.]).unwrap();
            assert!(left[0] < -0.2 && right[0] > 0.2,"{:?}",(left,right));
            assert!(left[1].abs()<0.001 && right[1].abs()<0.001,"{:?}",(left,right));
            fs::remove_dir_all(dir).unwrap();
        }
    }
}

impl Lab {
    pub(super) fn update_sensor_calibration(&mut self, t: f64) {
        if self.sensor_calibration.is_none() { return; }
        self.engine.disarm();
        let identity = self.current_mounting_identity(t);
        let input = self.action_input(t);
        let session = self.sensor_calibration.as_mut().unwrap();
        if identity.as_deref() == Some(&session.identity) {
            session.process(&input);
        } else { session.preview=Telemetry::default(); }
    }

    pub(super) fn sensor_calibration_state(&self, t: f64) -> Value {
        match &self.sensor_calibration {
            Some(s) => json!({
                "session":s.id, "identity":s.identity, "stage":s.stage,
                "fresh":self.current_mounting_identity(t).as_deref()==Some(&s.identity),
                "angles":s.preview.raw_direction_angles,
                "visual_lean":if self.current_mounting_identity(t).as_deref()==Some(&s.identity) {
                    self.action_input(t).chest.and_then(|q|s.visual_lean(q.value,if self.engine.cfg.y_up {[0.,1.,0.]}else{[0.,0.,1.]}))
                } else {None}
                ,"tilt_degrees":self.action_input(t).chest.and_then(|q|s.tilt(q.value,if self.engine.cfg.y_up {[0.,1.,0.]}else{[0.,0.,1.]}))
                    .map(|v|v.iter().map(|x|x*x).sum::<f64>().sqrt().to_degrees())
                ,"measurements":s.samples
            }),
            None => self.calibration_result.clone(),
        }
    }

    pub(super) fn sensor_calibration_command(&mut self, v: &Value, t: f64) -> Result<(), String> {
        self.engine.disarm();
        self.walking = false;
        let id = v["session"].as_str().filter(|s| !s.is_empty() && s.len() <= 80
            && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
            .ok_or("Invalid calibration session")?;
        let command = v["command"].as_str().unwrap_or("");
        if command == "sensor_calibration_begin" {
            if let Some(s) = &self.sensor_calibration {
                return if s.id == id { Ok(()) } else { Err("Cancel the current calibration first".into()) };
            }
            if self.calibration_result["session"] == id {
                return Err("Start a new calibration session".into());
            }
            let identity = self.current_mounting_identity(t).ok_or("Fresh assigned chest sensor required")?;
            let mut settings = Settings::default();
            settings.direction.enabled = true;
            settings.direction.deadzone = 0.;
            let mut processor = ActionProcessor::new(settings).map_err(str::to_owned)?;
            processor.set_world_up(if self.engine.cfg.y_up {[0.,1.,0.]} else {[0.,0.,1.]});
            self.sensor_calibration = Some(Session {
                id:id.into(), identity, stage:"center", processor,
                preview:Telemetry::default(), samples:Vec::new(),
            });
            self.calibration_result = Value::Null;
            return Ok(());
        }
        if command == "sensor_calibration_capture"
            && self.calibration_result["session"] == id && self.calibration_result["stage"] == "done" {
            return Ok(());
        }
        if command == "sensor_calibration_cancel" && self.sensor_calibration.is_none() {
            self.calibration_result=json!({"session":id,"stage":"canceled"});
            return Ok(());
        }
        let session = self.sensor_calibration.as_ref().ok_or("Start sensor calibration first")?;
        if session.id != id { return Err("Calibration session changed. Reopen setup.".into()); }
        if command == "sensor_calibration_cancel" {
            self.sensor_calibration = None;
            self.calibration_result = json!({"session":id,"stage":"canceled"});
            self.action_preview = Telemetry::default();
            return Ok(());
        }
        if command != "sensor_calibration_capture" { return Err("Unknown calibration command".into()); }
        let identity = self.current_mounting_identity(t).ok_or("Fresh chest orientation required")?;
        if identity != session.identity { return Err("Chest sensor changed. Cancel and restart calibration.".into()); }
        let stage = v["stage"].as_str().ok_or("Missing calibration stage")?;
        // Repeated acknowledgements are harmless, but cannot skip a stage.
        if session.samples.last().is_some_and(|sample| sample["stage"] == stage) { return Ok(()); }
        if stage != session.stage { return Err(format!("Complete {} first", session.stage)); }
        let input = self.action_input(t);
        let up=if self.engine.cfg.y_up {[0.,1.,0.]}else{[0.,0.,1.]};
        let session = self.sensor_calibration.as_mut().unwrap();
        match stage {
            "center" => session.processor.calibrate_chest_preview(&input).map_err(str::to_owned)?,
            "forward" | "backward" | "left" | "right" =>
                session.learn_named_lean(stage,input.chest.ok_or("Fresh chest orientation required")?.value,up)?,
            "finish" => {
                let preview = session.processor.process_chest_preview(&input);
                let [x,y] = preview.raw_direction_angles.ok_or("Fresh learned direction required")?;
                let valid = x.hypot(y) <= 5_f64.to_radians();
                if !valid { return Err(if stage=="finish" {"Return upright before saving"} else {"Hold the indicated lean and try again"}.into()); }
            }
            _ => return Err("Unknown calibration stage".into()),
        }
        if stage == "finish" { return self.commit_sensor_calibration(t); }
        session.samples.push(json!({"stage":stage,"quaternion":input.chest.map(|q|q.value),"receiver_time":t}));
        session.stage = match stage {
            "center"=>"forward", "forward"=>"backward", "backward"=>"left",
            "left"=>"right", "right"=>"finish", _=>unreachable!(),
        };
        session.process(&input);
        Ok(())
    }

    fn commit_sensor_calibration(&mut self, t: f64) -> Result<(), String> {
        let session = self.sensor_calibration.as_ref().ok_or("No calibration draft")?;
        let mut settings = self.actions.settings().clone();
        let learned = &session.processor.settings().direction;
        settings.direction.learned_forward = learned.learned_forward;
        settings.direction.learned_right = learned.learned_right;
        settings.direction.pitch_sign = 1.;
        settings.direction.roll_sign = 1.;
        settings.direction.pitch_axis = learned.pitch_axis.clone();
        settings.direction.roll_axis = learned.roll_axis.clone();
        settings.direction.forward_offset = 0.;
        settings.validate().map_err(str::to_owned)?;
        let directory = self.config.parent().unwrap_or(std::path::Path::new(".")).join("calibrations");
        fs::create_dir_all(&directory).map_err(|e|e.to_string())?;
        let path = directory.join(format!("calibration-{}.json", session.id));
        let record = json!({
            "schema":1, "session":session.id, "tracker_identity":session.identity,
            "method":"fresh-mounting-session-v1",
            "saved_unix_ms":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis(),
            "world_up":if self.engine.cfg.y_up {[0.,1.,0.]}else{[0.,0.,1.]},
            "measurements":session.samples,
            "final_center":self.action_input(t).chest.map(|q|q.value),
            "direction":settings.to_json()["direction"]
        });
        // Write a complete immutable record before changing the active configuration.
        // A failed activation leaves the old configuration and an inspectable archive.
        let mut neutral = self.action_input(t);
        if !path.exists() {
            let bytes = serde_json::to_vec_pretty(&record).map_err(|e|e.to_string())?;
            let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e|e.to_string())?;
            if let Err(e) = file.write_all(&bytes).and_then(|_|file.sync_all()) {
                drop(file);
                let _ = fs::remove_file(&path);
                return Err(e.to_string());
            }
        } else {
            let saved:Value = serde_json::from_slice(&fs::read(&path).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            if saved["session"] != record["session"] || saved["tracker_identity"] != record["tracker_identity"]
                || saved["measurements"] != record["measurements"] || saved["direction"] != record["direction"] {
                return Err("Calibration archive already exists with different measurements".into());
            }
            let q:Q = serde_json::from_value(saved["final_center"].clone()).map_err(|e|e.to_string())?;
            if let Some(chest) = neutral.chest.as_mut() { chest.value = q; }
        }
        let mut completed_processor = ActionProcessor::new(session.processor.settings().clone()).map_err(str::to_owned)?;
        completed_processor.set_world_up(if self.engine.cfg.y_up {[0.,1.,0.]} else {[0.,0.,1.]});
        completed_processor.calibrate_chest_preview(&neutral).map_err(str::to_owned)?;
        completed_processor.update_settings_preserving_center(settings.clone()).map_err(str::to_owned)?;
        let identity = session.identity.clone();
        let id = session.id.clone();
        let previous_settings = self.actions.settings().clone();
        let previous_identity = self.learned_mounting_identity.clone();
        let previous_record = self.device_mountings.get(&identity).cloned();
        self.actions.update_settings_preserving_center(settings.clone()).map_err(str::to_owned)?;
        self.learned_mounting_identity = Some(identity.clone());
        self.remember_device_mounting(&identity);
        let active = self.device_mountings.get_mut(&identity).unwrap();
        active["face_turned_over"] = json!(false);
        active["session"] = json!(id);
        active["calibration_file"] = json!(path);
        if let Err(e) = self.save(&self.mapping, &self.engine.cfg, &self.output_mode) {
            self.actions.update_settings_preserving_center(previous_settings).map_err(str::to_owned)?;
            self.learned_mounting_identity = previous_identity;
            match previous_record {
                Some(record) => {self.device_mountings.insert(identity,record);}
                None => {self.device_mountings.remove(&identity);}
            }
            return Err(format!("Calibration file written but activation failed: {e}. Retry Save."));
        }
        let input = self.action_input(t);
        self.sensor_calibration = None;
        self.actions = completed_processor;
        let _ = self.engine.calibrate(t);
        self.engine.disarm();
        self.action_preview = self.actions.process_chest_preview(&input);
        self.calibration_result = json!({"session":id,"identity":identity,"stage":"done","saved_file":path});
        self.stop_reason = "Sensor calibration saved. Output stopped.".into();
        Ok(())
    }
}

