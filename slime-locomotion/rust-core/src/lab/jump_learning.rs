use super::*;

pub(super) struct Session {
    pub source: String,
    pub started: f64,
    last_seen: f64,
    baseline: [Vec<f64>; 2],
    jumps: [Vec<f64>; 2],
    smooth: std::collections::VecDeque<(f64, f64)>,
}
impl Session {
    pub fn new(source: String, t: f64) -> Self { Self { source, started:t, last_seen:-1., baseline:Default::default(), jumps:Default::default(), smooth:Default::default() } }
    pub fn state(&self, t:f64) -> Value {
        let e=(t-self.started).max(0.);
        let (text, remaining)=if e<3. {("Hey Mofo  Get ready to Jump!",3.-e)} else if e<4.5 {("Jump!",0.)} else if e<7.5 {("Stand still",7.5-e)} else if e<10.5 {("Okay one more jump",10.5-e)} else {("Jump!",0.)};
        json!({"active":true,"text":text,"countdown":remaining.ceil() as u32,"source":self.source})
    }
    pub fn update(&mut self, input:crate::actions::Input) -> Result<Option<Value>,String> {
        let t=input.t; let elapsed=t-self.started;
        let (seen,raw)=if self.source=="height" {
            let h=input.hmd.ok_or("Headset signal lost. No jump settings were saved.")?;
            (h.seen,h.height)
        } else {
            let a=input.chest_accel.ok_or("Chest acceleration lost. No jump settings were saved.")?;
            (a.seen,a.value.iter().map(|x|x*x).sum::<f64>().sqrt())
        };
        if !seen.is_finite() || !raw.is_finite() || t<seen || t-seen>0.25 {return Err("Sensor signal is stale. No jump settings were saved.".into());}
        if seen>self.last_seen {
            self.last_seen=seen;
            self.smooth.push_back((seen,raw));
            while self.smooth.front().is_some_and(|x|seen-x.0>0.06) {self.smooth.pop_front();}
            let value=if self.source=="acceleration" {self.smooth.iter().map(|x|x.1).sum::<f64>()/self.smooth.len() as f64} else {raw};
            if (0.5..3.).contains(&elapsed) {self.baseline[0].push(value);} else if (3.0..4.5).contains(&elapsed) {self.jumps[0].push(value);} else if (5.0..7.5).contains(&elapsed) {self.baseline[1].push(value);} else if (10.5..12.0).contains(&elapsed) {self.jumps[1].push(value);}
        }
        if elapsed<12. {return Ok(None);}
        let mut settings=Vec::new();
        for i in 0..2 {
            if self.baseline[i].len()<4 || self.jumps[i].len()<4 {return Err("Not enough fresh samples for both jumps. No jump settings were saved.".into());}
            let baseline=self.baseline[i].iter().sum::<f64>()/self.baseline[i].len() as f64;
            let peak=self.jumps[i].iter().copied().fold(f64::NEG_INFINITY,f64::max);
            settings.push(crate::calibration::jump_settings(&self.source,baseline,peak,self.jumps[i].len())?);
        }
        let chosen=if settings[0]["threshold"].as_f64()<settings[1]["threshold"].as_f64() {0}else{1};
        Ok(Some(settings.remove(chosen)))
    }
}
#[cfg(test)] mod tests {
 use super::*;
 fn input(t:f64, value:f64)->crate::actions::Input {crate::actions::Input{t,chest:None,chest_accel:Some(crate::actions::TimedVector{value:[value,0.,0.],seen:t}),hmd:None}}
 #[test] fn two_jumps_are_required_and_weakest_sets_threshold() {
  let mut s=Session::new("acceleration".into(),0.);
  let mut result=None;
  for i in 0..=1200 {let t=i as f64/100.;let v=if (3.1..3.5).contains(&t){6.} else if (10.6..11.).contains(&t){4.}else{0.2};result=s.update(input(t,v)).unwrap();}
  let r=result.unwrap();assert_eq!(r["enabled"],true);assert!((r["threshold"].as_f64().unwrap()-2.29).abs()<0.02);
 }
 #[test] fn missing_second_jump_and_stale_input_do_not_save() {
  let mut s=Session::new("acceleration".into(),0.);let mut failed=false;
  for i in 0..=1200 {let t=i as f64/100.;let v=if (3.1..3.5).contains(&t){6.}else{0.2};if s.update(input(t,v)).is_err(){failed=true;break;}}
  assert!(failed);
  let mut s=Session::new("acceleration".into(),0.);let mut v=input(1.,0.);v.chest_accel.as_mut().unwrap().seen=0.;assert!(s.update(v).is_err());
 }
}
