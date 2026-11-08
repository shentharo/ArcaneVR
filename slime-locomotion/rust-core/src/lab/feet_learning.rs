//! Synchronized foot measurements, saved for later detector development.
use super::*;

/// Fewer detected alternating steps than this and the walk is redone. Deliberately low:
/// the counter is untuned against live walking, so it only rejects clearly-not-walking.
const MIN_WALK_STEPS: usize = 4;
/// Recorded as `steps_verified` when at least this many alternating steps were seen.
const VERIFIED_WALK_STEPS: usize = 8;

pub(super) struct Session {
    pub stage: &'static str,
    pub sensor: String,
    pub ids: [String;2],
    /// Chest sensor id, empty when no chest sensor is assigned. It must stay on, but is not recorded.
    pub chest: String,
    pub step: usize,
    pub started: f64,
    pub last_motion: f64,
    pub baseline: Option<Q>,
    pub manual_lift: bool,
    pub samples: Vec<Value>,
    pub message: String,
}
impl Session {
    pub fn new(ids:[String;2],chest:String) -> Self { Self {stage:"waiting",sensor:String::new(),ids,chest,step:0,started:0.,last_motion:0.,baseline:None,manual_lift:false,samples:Vec::new(),message:String::new()} }
    pub fn state(&self,t:f64)->Value {json!({"active":true,"stage":self.stage,"sensor":self.sensor,"step":self.step,"elapsed":if matches!(self.stage,"ready"|"walk") {(t-self.started).clamp(0.,10.)}else{0.},"samples":self.samples.len(),"message":self.message})}
    fn angle(a:Q,b:Q)->f64 {2.*a.iter().zip(b).map(|(x,y)|x*y).sum::<f64>().abs().clamp(0.,1.).acos()}
    pub fn observe(&mut self,id:&str,t:f64,kind:&str,value:Value) {
        if !self.ids.iter().any(|x|x==id) || self.stage=="waiting" {return;}
        if self.samples.len()<40000 {self.samples.push(json!({"time":t,"stage":self.stage,"step":self.step,"sensor":id,"kind":kind,"value":value}));}
        if id!=self.sensor || kind!="rotation" {return;}
        let Ok(q)=serde_json::from_value::<Q>(value) else{return;};
        if self.stage=="lift" && self.baseline.is_some_and(|b|Self::angle(b,q)>8_f64.to_radians()) {
            self.last_motion=t;self.stage="lower";
        } else if self.stage=="lower" && !self.manual_lift && t-self.last_motion>0.4 && self.baseline.is_some_and(|b|Self::angle(b,q)<5_f64.to_radians()) {
            self.advance(t);
        }
    }
    pub fn advance(&mut self,t:f64) {
        self.step+=1;self.sensor.clear();self.baseline=None;self.manual_lift=false;self.started=t;
        self.stage=if self.step>=4 {"ready"}else{"prepare"};
    }
}
impl Lab {
    pub(super) fn update_feet_learning(&mut self,t:f64) {
        let Some(mut s)=self.feet_learning.take() else{return;};
        let fresh=|id:&String| self.identity_matches(id) && self.records.get(id).is_some_and(|r|r.seen>=0.) && id.rsplit_once(':').is_some_and(|(ip,_)|self.peers.iter().any(|(peer,seen)|peer.ip().to_string()==ip && t>=*seen && t-*seen<2.));
        if !s.ids.iter().all(&fresh) || (!s.chest.is_empty() && !fresh(&s.chest)) {
            if s.stage!="waiting" {s.step=0;s.samples.clear();s.sensor.clear();s.baseline=None;}
            s.stage="waiting";s.message=if s.chest.is_empty() {"Waiting for both foot sensors. Turn them on and keep them connected."} else {"Waiting for your chest and both foot sensors. Turn them all on and keep them connected."}.into();
        } else if s.stage=="waiting" {s.stage="prepare";s.message.clear();}
        else if s.stage=="prepare" {
            s.sensor=s.ids[s.step%2].clone();s.baseline=self.records.get(&s.sensor).map(|r|r.q);s.manual_lift=false;s.stage="lift";
        } else if s.stage=="ready" && t-s.started>=3. {s.started=t;s.stage="walk";s.message.clear();}
        else if s.stage=="walk" && t-s.started>=10. {
            let mut valid=true;
            for id in &s.ids {
                let rotations:Vec<Q>=s.samples.iter().filter(|v|v["stage"]=="walk" && v["sensor"]==*id && v["kind"]=="rotation").filter_map(|v|serde_json::from_value(v["value"].clone()).ok()).collect();
                let accel:Vec<[f64;3]>=s.samples.iter().filter(|v|v["stage"]=="walk" && v["sensor"]==*id && v["kind"]=="acceleration").filter_map(|v|serde_json::from_value(v["value"].clone()).ok()).collect();
                let moved=rotations.first().is_some_and(|first|rotations.iter().any(|q|Session::angle(*first,*q)>5_f64.to_radians())) || accel.first().is_some_and(|first|accel.iter().any(|a|a.iter().zip(first).map(|(x,y)|(x-y)*(x-y)).sum::<f64>().sqrt()>1.));
                valid &= rotations.len()>=10 && accel.len()>=10 && moved;
            }
            let magnitudes=|id:&String|->Vec<(f64,f64)> {s.samples.iter().filter(|v|v["stage"]=="walk" && v["sensor"]==*id && v["kind"]=="acceleration").filter_map(|v|{let a:[f64;3]=serde_json::from_value(v["value"].clone()).ok()?;Some((v["time"].as_f64()?,a.iter().map(|x|x*x).sum::<f64>().sqrt()))}).collect()};
            let (left,right)=(magnitudes(&s.ids[0]),magnitudes(&s.ids[1]));
            let steps=step_count::count_alternating_steps([&left,&right]);
            if !valid {s.message="Not enough movement or sensor data from both feet. Please try again.".into();s.step=0;s.samples.clear();s.stage="prepare";}
            else if steps.total<MIN_WALK_STEPS {
                // Keep the four lifts and redo only the walk.
                s.message=format!("I only saw {} steps from your feet. Walk in place steadily, lifting each foot, for the full 10 seconds.",steps.total);
                s.samples.retain(|v|v["stage"]!="walk");s.stage="ready";s.started=t;
            } else {
                let sensors:Vec<Value>=s.ids.iter().enumerate().map(|(i,id)|json!({"side":if i==0{"left"}else{"right"},"sensor":id,"mac":id.rsplit_once(':').and_then(|(ip,_)|self.peer_macs.get(ip))})).collect();
                let dir=self.config.parent().unwrap_or(std::path::Path::new(".")).join("feet-calibrations");let path=dir.join(format!("feet-{}.json",std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis()));
                let record=json!({"version":2,"purpose":"synchronized raw feet learning; not applied to locomotion","sensors":sensors,"duration_seconds":10.,"instructed_steps":10,"steps_detected":steps.total,"steps_per_foot":steps.per_foot,"steps_verified":steps.total>=VERIFIED_WALK_STEPS,"walk_started_at":s.started,"samples":s.samples});
                let save=fs::create_dir_all(&dir).and_then(|_|fs::write(&path,serde_json::to_vec_pretty(&record).unwrap()));
                self.feet_learning_result=match save {Ok(())=>json!({"active":false,"stage":"done","saved_file":path.to_string_lossy(),"message":"Feet recordings saved for later."}),Err(e)=>json!({"active":false,"stage":"error","message":format!("Could not save feet recordings: {e}")})};return;
            }
        }
        self.feet_learning=Some(s);
    }
}
#[cfg(test)] mod tests {
 use super::*;
 #[test] fn alternating_lifts_require_the_prompted_sensor_and_record_both() {
  let mut s=Session::new(["left".into(),"right".into()],"chest".into());s.stage="lift";s.sensor="left".into();s.baseline=Some([1.,0.,0.,0.]);let q=[0.15_f64.cos(),0.15_f64.sin(),0.,0.];
  s.observe("right",1.,"rotation",json!(q));assert_eq!(s.stage,"lift");s.observe("left",1.,"rotation",json!(q));assert_eq!(s.stage,"lower");
  s.observe("left",1.6,"rotation",json!([1.,0.,0.,0.]));assert_eq!(s.stage,"prepare");assert_eq!(s.step,1);assert_eq!(s.samples.len(),3);
  s.stage="lower";s.sensor="right".into();s.baseline=Some([1.,0.,0.,0.]);s.manual_lift=true;s.last_motion=2.;s.observe("right",3.,"rotation",json!([1.,0.,0.,0.]));assert_eq!(s.stage,"lower");
 }
}
