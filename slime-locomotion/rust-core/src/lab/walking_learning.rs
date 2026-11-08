use super::*;
use std::collections::VecDeque;
#[derive(Default)] pub(super) struct Signal { samples:VecDeque<(f64,f64)> }
impl Signal {
 pub fn add(&mut self,t:f64,x:f64) {if self.samples.back().is_some_and(|p|t<=p.0){return;}self.samples.push_back((t,x));while self.samples.front().is_some_and(|p|t-p.0>1.5){self.samples.pop_front();}}
 pub fn measure(&self)->(f64,usize) {
  let Some(last)=self.samples.back() else{return (0.,0)};
  let recent_motion=self.samples.iter().zip(self.samples.iter().skip(1)).any(|(a,b)|last.0-b.0<0.25 && (b.1-a.1).abs()>0.001);
  if !recent_motion{return (0.,0);}

  let mut distance=0.;let mut reversals=0;let mut sign=0.;let mut anchor=self.samples.front().map_or(0.,|p|p.1);
  for pair in self.samples.iter().zip(self.samples.iter().skip(1)) {distance+=(pair.1.1-pair.0.1).abs();let delta=pair.1.1-anchor;if delta.abs()>0.008 {let next=delta.signum();if sign!=0. && next!=sign {reversals+=1;}sign=next;anchor=pair.1.1;}}
  let dt=self.samples.back().zip(self.samples.front()).map_or(0.,|(a,b)|a.0-b.0);
  (if dt>0.2 {distance/dt}else{0.},reversals)
 }
}
pub(super) struct Session {pub identity:String, started:f64, leaned:Option<f64>, capture:Option<f64>, signal:Signal,rates:Vec<f64>}
impl Session {
 pub fn new(identity:String,t:f64)->Self{Self{identity,started:t,leaned:None,capture:None,signal:Signal::default(),rates:vec![]}}
 pub fn state(&self,t:f64)->Value{json!({"active":true,"text":if self.capture.is_some(){"Okay, walk forward in place. Keep leaning forward and let your chest sway naturally left and right."}else{"Lean forward gently and hold it."},"remaining":self.capture.map(|start|(10.-(t-start)).max(0.).ceil() as u32)})}
 pub fn update(&mut self,t:f64,seen:f64,angles:[f64;2],forward:f64)->Result<Option<Value>,String>{
  if t<seen || t-seen>=0.25 || !angles.iter().all(|x|x.is_finite()){return Err("Chest signal lost. Nothing saved.".into());}
  if t-self.started>40. {return Err("Learning timed out. Nothing saved.".into());}
  if angles[1]<forward {if self.capture.is_some(){return Err("Forward lean was lost. Try again and keep a gentle forward lean.".into());}self.leaned=None;return Ok(None);}
  let leaned=*self.leaned.get_or_insert(t);if t-leaned<0.75{return Ok(None);}
  let start=*self.capture.get_or_insert(t);self.signal.add(seen,angles[0]);let (rate,turns)=self.signal.measure();
  if t-start>1.5 && turns>=2 && rate>0.02 {self.rates.push(rate);}
  if t-start<10. {return Ok(None);}
  if self.rates.len()<60 {return Err("Not enough repeated left/right walking sway. Try again; keep walking for the full countdown.".into());}
  self.rates.sort_by(f64::total_cmp);let rate=self.rates[self.rates.len()/2];
  Ok(Some(json!({"identity":self.identity,"rate":rate,"method":"lateral-wobble-v1"})))
 }
}
#[cfg(test)] mod tests {use super::*;
 #[test] fn saved_walking_model_survives_receiver_restart(){
  let path=std::env::temp_dir().join(format!("walking-persist-{}-{}.json",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
  let mut lab=Lab::new(path.clone()).unwrap();let model=json!({"identity":"02:00:00:00:00:01:0","rate":0.5,"method":"lateral-wobble-v1"});lab.walking_model=model.clone();lab.save(&lab.mapping,&lab.engine.cfg,&lab.output_mode).unwrap();
  lab.command(&json!({"command":"walking_full_speed","effort":0.5}),0.).unwrap();
  assert!(lab.command(&json!({"command":"walking_full_speed","effort":0.0}),0.).is_err());
  let model=lab.walking_model.clone();assert_eq!(model["full_speed_effort"],0.5);drop(lab);
  let restored=Lab::new(path.clone()).unwrap();assert_eq!(restored.walking_model,model);assert!(!restored.engine.armed);std::fs::remove_file(path).unwrap();
 }
 #[test] fn learns_only_after_forward_lean_and_repeated_sway(){let mut s=Session::new("sensor".into(),0.);for n in 0..100 {assert!(s.update(n as f64*0.01,n as f64*0.01,[0.,0.],0.1).unwrap().is_none());}let mut model=None;for n in 100..1300 {let t=n as f64*0.01;match s.update(t,t,[0.05*(t*8.).sin(),0.2],0.1){Ok(Some(v))=>{model=Some(v);break;},Ok(None)=>{},Err(e)=>panic!("{e}")}}assert!(model.unwrap()["rate"].as_f64().unwrap()>0.02);}
 #[test] fn stillness_and_stale_do_not_learn(){let mut s=Session::new("sensor".into(),0.);let mut failed=false;for n in 0..1200 {let t=n as f64*0.01;if s.update(t,t,[0.,0.2],0.1).is_err(){failed=true;break;}}assert!(failed);assert!(Session::new("sensor".into(),0.).update(1.,0.,[0.,0.2],0.1).is_err());}
}
