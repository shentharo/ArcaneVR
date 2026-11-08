//! Test Room leg-pose estimates from the two independent foot orientations.
//! These are orientation-based visual estimates, not measured foot positions.
use bevy::prelude::{Quat,Vec3};
use serde_json::Value;
use std::{collections::HashMap,path::Path};

#[derive(Clone,Copy)]
struct Reference { axis:Vec3, range:f32 }
#[derive(Default)]
struct Foot { neutral:Option<Quat>, height:f32, mac:String, active:bool, settled:f32, energy:f32, peak:f32, peak_at:f32, last_peak:Option<f32>, period:Option<f32> }
#[derive(Default)]
pub struct FootRig { feet:[Foot;2], profiles:HashMap<String,Reference>, quiet:f32, had_motion:bool, clock:f32, last_motion:[Option<f32>;2], initial_settled:bool, initial_quiet:f32, running:bool, last_walk_signal:Option<f32>, last_step:Option<(usize,f32)>, step_interval:Option<f32> }
fn quaternion(v:&Value)->Option<Quat> {
 let a=v.as_array()?;if a.len()!=4{return None;}
 let q=Quat::from_xyzw(a[1].as_f64()? as f32,a[2].as_f64()? as f32,a[3].as_f64()? as f32,a[0].as_f64()? as f32);
 (q.is_finite() && q.length_squared()>0.5).then(||q.normalize())
}
fn rotation_vector(from:Quat,to:Quat)->Vec3 {
 let mut d=(from.inverse()*to).normalize();if d.w<0. {d=-d;}
 let v=Vec3::new(d.x,d.y,d.z);let len=v.length();if len<1e-7 {Vec3::ZERO}else{v/len*(2.*len.atan2(d.w))}
}
impl FootRig {
 pub fn profile_count(&self)->usize {self.profiles.len()}
 pub fn running(&self)->bool {self.running}
 pub fn load(dir:&Path)->Self {
  let mut rig=Self::default();
  let Ok(entries)=std::fs::read_dir(dir) else{return rig;};
  let mut paths:Vec<_>=entries.flatten().map(|e|e.path()).filter(|p|p.extension().is_some_and(|x|x=="json")).collect();
  paths.sort_by_key(|p|std::fs::metadata(p).and_then(|m|m.modified()).ok());paths.reverse();
  for path in paths.into_iter().take(80) {
   let Ok(meta)=std::fs::metadata(&path) else{continue;};if meta.len()>32*1024*1024 {continue;}
   let Ok(bytes)=std::fs::read(path) else{continue;};let Ok(v)=serde_json::from_slice::<Value>(&bytes) else{continue;};
   if v["version"]!=2 {continue;}let Some(sensors)=v["sensors"].as_array() else{continue;};let Some(samples)=v["samples"].as_array() else{continue;};
   for sensor in sensors {
    let Some(mac)=sensor["mac"].as_str().map(str::to_uppercase) else{continue;};if rig.profiles.contains_key(&mac){continue;}
    let Some(id)=sensor["sensor"].as_str() else{continue;};let side=match sensor["side"].as_str(){Some("left")=>0,Some("right")=>1,_=>continue};
    let mut vectors=Vec::new();
    for step in [side,side+2] {
     let values:Vec<Quat>=samples.iter().filter(|s|s["step"].as_u64()==Some(step) && s["sensor"]==id && s["kind"]=="rotation" && (s["stage"]=="lift" || s["stage"]=="lower")).filter_map(|s|quaternion(&s["value"])).collect();
     if let Some(base)=values.first() {if let Some(peak)=values.iter().map(|q|rotation_vector(*base,*q)).max_by(|a,b|a.length_squared().total_cmp(&b.length_squared())) {if peak.length()>0.1 {vectors.push(peak);}}}
    }
    if !vectors.is_empty() {let sum=vectors.iter().copied().fold(Vec3::ZERO,|a,b|a+b);if sum.length()>0.1 {let range=(vectors.iter().map(|v|v.length()).sum::<f32>()/vectors.len() as f32*0.16).clamp(0.07,0.2);rig.profiles.insert(mac,Reference{axis:sum.normalize(),range});}}
   }
  }
  rig
 }
 pub fn update(&mut self,v:&Value,dt:f32)->([f32;2],bool) {
  let dt=dt.clamp(0.,0.1);self.clock+=dt;let heading=v["heading"].as_f64().filter(|x|x.is_finite()).unwrap_or(0.) as f32;
  let up=if v["axis"]=="y" {Vec3::Y}else{Vec3::Z};let unyaw=Quat::from_axis_angle(up,-heading);
  let mut valid=true;let mut quiet=true;let mut targets=[0.;2];let mut events=Vec::new();
  for (i,role) in ["left","right"].iter().enumerate() {
   let r=&v["roles"][*role];let mac=r["mac"].as_str().unwrap_or("").to_uppercase();
   let q=if r["fresh"]==true {quaternion(&r["quaternion"])}else{None};
   let f=&mut self.feet[i];
   if f.mac!=mac {*f=Foot::default();f.mac=mac.clone();}
   if let Some(q)=q {
    let q=(unyaw*q).normalize();let base=*f.neutral.get_or_insert(q);let mut delta=rotation_vector(base,q);
    // A chest turn with planted feet changes their relative yaw, not their lift.
    // Remove world-up twist in the neutral sensor frame before measuring tilt.
    let local_up=(base.inverse()*up).normalize_or_zero();delta-=local_up*delta.dot(local_up);
    let profile=self.profiles.get(&mac);let magnitude=profile.map_or(delta.length(),|p|{let axis=(p.axis-local_up*p.axis.dot(local_up)).normalize_or_zero();delta.dot(axis).abs()});
    let acceleration=r["acceleration_magnitude"].as_f64();
    let fresh_accel=r["acceleration_age_ms"].as_f64().is_some_and(|n|n<250.);
    let amplitude=if fresh_accel {acceleration.unwrap_or(0.).clamp(0.,100.) as f32}else{0.};
    f.energy+=(amplitude-f.energy)*(1.-(-dt/0.08).exp());
    if f.energy>f.peak {f.peak=f.energy;f.peak_at=self.clock;}
    if f.peak>2. && f.energy<f.peak*0.5 {
     if f.last_peak.is_none_or(|t|f.peak_at-t>=0.32) {
      if let Some(last)=f.last_peak {let period=f.peak_at-last;if (0.32..=1.5).contains(&period) {f.period=Some(f.period.map_or(period,|old|old*0.7+period*0.3));}else{f.period=None;}}
      f.last_peak=Some(f.peak_at);
      events.push((i,f.peak_at));
     }
     f.peak=f.energy;f.peak_at=self.clock;
    }
    if (fresh_accel && acceleration.is_some_and(|n|n>1.2)) || magnitude>0.12 {f.active=true;f.settled=0.;}
    if f.active && magnitude<0.045 && (!fresh_accel || acceleration.is_some_and(|n|n<0.7)) {f.settled+=dt;if f.settled>0.3 {f.active=false;f.settled=0.;f.neutral=Some(q);}} else if magnitude>=0.045 {f.settled=0.;}
    if !f.active {f.neutral=Some(base.slerp(q,(dt/1.5).clamp(0.,1.)));}
    let range=profile.map_or(0.18,|p|p.range);targets[i]=if f.active {((magnitude-0.025)/range).clamp(0.,1.)}else{0.};
    if fresh_accel && acceleration.is_some_and(|n|n>1.2) {self.last_motion[i]=Some(self.clock);}
    quiet &= acceleration.is_some_and(|n|n<0.45);
   } else {valid=false;quiet=false;f.neutral=None;f.height=0.;f.active=false;f.settled=0.;f.energy=0.;f.peak=0.;f.last_peak=None;f.period=None;}
  }
  if !self.initial_settled && quiet && valid {
   self.initial_quiet+=dt;
   if self.initial_quiet>=0.5 {
    for (i,role) in ["left","right"].iter().enumerate() {if let Some(q)=quaternion(&v["roles"][*role]["quaternion"]) {let f=&mut self.feet[i];f.neutral=Some((unyaw*q).normalize());f.height=0.;f.active=false;f.settled=0.;}}
    self.initial_settled=true;self.initial_quiet=0.;self.had_motion=false;self.last_motion=[None,None];
   }
  } else if !self.initial_settled {self.initial_quiet=0.;}
  if self.last_motion.iter().all(|t|t.is_some_and(|t|self.clock-t<0.6)) {self.had_motion=true;self.quiet=0.;}
  else if quiet && self.had_motion {self.quiet+=dt;if self.quiet>1.0 {for f in &mut self.feet {f.neutral=None;f.height=0.;f.active=false;f.settled=0.;}self.quiet=0.;self.had_motion=false;self.last_step=None;self.step_interval=None;}}
  else {self.quiet=0.;}
  if !valid || self.last_step.is_some_and(|(_,at)|self.clock-at>1.5) {self.last_step=None;self.step_interval=None;}
  events.sort_by(|a,b|a.1.total_cmp(&b.1));
  for (side,at) in events {
   if let Some((previous,then))=self.last_step {
    let elapsed=at-then;
    if side!=previous && (0.16..=0.9).contains(&elapsed) {self.step_interval=Some(self.step_interval.map_or(elapsed,|old|old*0.7+elapsed*0.3));self.last_step=Some((side,at));}
   } else {self.last_step=Some((side,at));}
  }
  if v["locomotion"]["preview"]["speed"].as_f64().is_some_and(|speed|speed>0.03) {self.last_walk_signal=Some(self.clock);}
  self.running=valid && self.last_walk_signal.is_some_and(|t|self.clock-t<0.35) && self.last_step.is_some() && self.step_interval.is_some();
  if self.running {
   let (side,at)=self.last_step.unwrap();
   // Continue smoothly across a briefly missed peak; the next real foot event
   // re-anchors the phase. Stale movement stops this path above.
   let phase=((self.clock-at)/self.step_interval.unwrap()).clamp(0.,4.);
   targets[side]=0.5-0.5*(std::f32::consts::PI*phase).cos();targets[1-side]=1.-targets[side];
  }
  for (f,target) in self.feet.iter_mut().zip(targets) {f.height+=(target-f.height)*(1.-(-dt/0.075).exp());}
  ([self.feet[0].height,self.feet[1].height],valid)
 }
}
#[cfg(test)] mod tests {
 use super::*;use serde_json::json;
 fn sample(left:f32,right:f32)->Value {let q=|a:f32|json!([(a/2.).cos(),(a/2.).sin(),0.,0.]);json!({"axis":"z","heading":0.,"locomotion":{"preview":{"speed":0.}},"roles":{"left":{"fresh":true,"mac":"L","quaternion":q(left),"acceleration_magnitude":1.},"right":{"fresh":true,"mac":"R","quaternion":q(right),"acceleration_magnitude":1.}}})}
 #[test] fn feet_move_independently_and_return_to_neutral(){let mut r=FootRig::default();r.update(&sample(0.,0.),0.1);let (p,_)=r.update(&sample(0.2,0.),0.1);assert!(p[0]>0.5 && p[1]<0.01);let (p,_)=r.update(&sample(0.,0.2),0.1);assert!(p[1]>0.5 && p[0]<0.25);for _ in 0..8{r.update(&sample(0.,0.),0.1);}let (p,_)=r.update(&sample(0.,0.),0.1);assert!(p[0]<0.01 && p[1]<0.01);}
 #[test] fn stale_foot_is_not_animated(){let mut r=FootRig::default();r.update(&sample(0.,0.),0.1);let mut v=sample(0.2,0.);v["roles"]["left"]["fresh"]=json!(false);let (p,valid)=r.update(&v,0.1);assert!(!valid);assert_eq!(p[0],0.);}
 #[test] fn turning_the_body_does_not_lift_the_feet(){let mut r=FootRig::default();r.update(&sample(0.,0.),0.1);let yaw=Quat::from_rotation_z(1.2);let q=json!([yaw.w,yaw.x,yaw.y,yaw.z]);let mut v=sample(0.,0.);v["heading"]=json!(1.2);v["roles"]["left"]["quaternion"]=q.clone();v["roles"]["right"]["quaternion"]=q;let (p,_)=r.update(&v,0.1);assert!(p[0]<0.01 && p[1]<0.01);}
 #[test] fn turning_chest_while_feet_remain_planted_does_not_lift_them(){let mut r=FootRig::default();r.update(&sample(0.,0.),0.1);let mut v=sample(0.,0.);v["heading"]=json!(1.2);let (p,_)=r.update(&v,0.1);assert!(p[0]<0.01 && p[1]<0.01);}
 #[test] fn holding_one_lift_does_not_recenter_it(){let mut r=FootRig::default();let mut neutral=sample(0.,0.);neutral["roles"]["left"]["acceleration_magnitude"]=json!(0.1);neutral["roles"]["right"]["acceleration_magnitude"]=json!(0.1);for _ in 0..6{r.update(&neutral,0.1);}let mut v=sample(0.2,0.);v["roles"]["left"]["acceleration_magnitude"]=json!(0.1);v["roles"]["right"]["acceleration_magnitude"]=json!(0.1);for _ in 0..30{r.update(&v,0.1);}let (p,_)=r.update(&v,0.1);assert!(p[0]>0.5 && p[1]<0.01);}
 #[test] fn loads_lift_reference_for_matching_sensor_only(){
  let dir=std::env::temp_dir().join(format!("arcane-foot-profile-{}",std::process::id()));std::fs::create_dir_all(&dir).unwrap();
  let q=json!([(0.2f64).cos(),(0.2f64).sin(),0.,0.]);let mut samples=Vec::new();
  for step in [0,2] {samples.push(json!({"step":step,"sensor":"id","kind":"rotation","stage":"lift","value":[1.,0.,0.,0.]}));samples.push(json!({"step":step,"sensor":"id","kind":"rotation","stage":"lower","value":q}));}
  let v=json!({"version":2,"sensors":[{"side":"left","sensor":"id","mac":"L"}],"samples":samples});std::fs::write(dir.join("feet.json"),v.to_string()).unwrap();
  let r=FootRig::load(&dir);assert_eq!(r.profile_count(),1);assert!(r.profiles.contains_key("L"));assert!(!r.profiles.contains_key("R"));std::fs::remove_file(dir.join("feet.json")).unwrap();std::fs::remove_dir(dir).unwrap();
 }
 #[test] fn replay_live_recording_if_provided(){
  let (Ok(path),Ok(dir))=(std::env::var("ARCANEVR_QA_FOOT_RECORD"),std::env::var("ARCANEVR_QA_FOOT_DIR")) else{return;};
  let text=std::fs::read_to_string(path).unwrap();let mut rig=FootRig::load(Path::new(&dir));assert_eq!(rig.profile_count(),2);
  let mut origin=None;let mut previous=None;let mut maxima=[[0f32;2];3];let mut running_min=[1f32;2];let mut running_samples=0;let mut both_high=0;
  for line in text.lines(){let row:Value=serde_json::from_str(line).unwrap();let t=row["t"].as_f64().unwrap();let start=*origin.get_or_insert(t);let dt=previous.map_or(0.03,|p:f64|(t-p).clamp(0.,0.1));previous=Some(t);let state=json!({"heading":0.,"axis":"z","roles":row["roles"],"locomotion":{"preview":{"speed":row["speed"]}}});let (poses,_)=rig.update(&state,dt as f32);let sec=t-start;let group=if (3.0..7.0).contains(&sec){Some(0)}else if (10.0..11.5).contains(&sec){Some(1)}else if (11.5..18.0).contains(&sec){Some(2)}else{None};if let Some(g)=group{for (m,p) in maxima[g].iter_mut().zip(poses){*m=m.max(p);}}if (13.0..18.0).contains(&sec) && rig.running(){running_samples+=1;if poses[0]>0.8 && poses[1]>0.8 {both_high+=1;}for (m,p) in running_min.iter_mut().zip(poses){*m=m.min(p);}}}
  println!("recorded left-only/right-transition/fast peaks: {maxima:?}, running minima: {running_min:?}, running frames: {running_samples}, both high: {both_high}");assert!(maxima[0][0]>0.1 && maxima[0][1]<0.1 && maxima[1][1]>0.1 && maxima[1][0]<0.1 && maxima[2][0]>0.5 && maxima[2][1]>0.5 && running_samples>30 && running_min.iter().all(|n|*n<0.35) && both_high*10<running_samples);
 }
}
