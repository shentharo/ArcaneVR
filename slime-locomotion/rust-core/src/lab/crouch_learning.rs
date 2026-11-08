use super::*;

pub(super) struct Session {
    started:f64,
    last_seen:f64,
    standing:[Vec<f64>;2],
    crouches:[Vec<f64>;2],
    correction:f64,
}
impl Session {
    pub fn new(t:f64, correction:f64)->Self {Self{started:t,last_seen:-1.,standing:Default::default(),crouches:Default::default(),correction}}
    pub fn state(&self,t:f64)->Value {
        let e=(t-self.started).max(0.);
        let (text,left)=if e<3. {("Hey Mofo  Get ready to Crouch! Stand upright.",3.-e)} else if e<5. {("Crouch and hold!",0.)} else if e<8. {("Stand still",8.-e)} else if e<11. {("Okay one more crouch",11.-e)} else {("Crouch and hold!",0.)};
        json!({"active":true,"text":text,"countdown":left.ceil() as u32})
    }
    pub fn update(&mut self,input:crate::actions::Input)->Result<Option<Value>,String> {
        let t=input.t;let e=t-self.started;
        let h=input.hmd.ok_or("Headset signal lost. No crouch settings were saved.")?;
        if !h.seen.is_finite() || t<h.seen || t-h.seen>0.25 || !h.height.is_finite() || !(0.05..=4.).contains(&h.height) {return Err("Headset height is stale or invalid. No crouch settings were saved.".into());}
        if h.seen>self.last_seen {
            self.last_seen=h.seen;
            if (0.5..3.).contains(&e) {self.standing[0].push(h.height);}
            else if (4.2..5.).contains(&e) {self.crouches[0].push(h.height);}
            else if (6.0..8.).contains(&e) {self.standing[1].push(h.height);}
            else if (12.2..13.).contains(&e) {self.crouches[1].push(h.height);}
        }
        if e<13. {return Ok(None);}
        let mean=|v:&Vec<f64>| v.iter().sum::<f64>()/v.len() as f64;
        let mut standing=[0.;2];let mut crouched=[0.;2];
        for i in 0..2 {
            if self.standing[i].len()<4 || self.crouches[i].len()<4 {return Err("Not enough fresh headset samples for both crouches. No settings saved.".into());}
            standing[i]=mean(&self.standing[i]);crouched[i]=mean(&self.crouches[i]);
            if standing[i]-crouched[i]<0.12 {return Err("Could not detect both crouches clearly. No crouch settings were saved.".into());}
        }
        let stand=standing[0].min(standing[1]);
        let crouch=crouched[0].max(crouched[1]);
        let difference=stand-crouch;
        if difference<0.12 {return Err("Standing and crouching heights overlap. No crouch settings were saved.".into());}
        let enter=crouch+difference*0.4+self.correction;
        let exit=crouch+difference*0.7+self.correction;
        if !(0.05..=4.).contains(&enter) || !(0.05..=4.).contains(&exit) {return Err("Height correction puts the learned thresholds out of range. No settings saved.".into());}
        Ok(Some(json!({"enabled":true,"enter":enter,"exit":exit,"use_reset_offset":false,"reference_height":(standing[0]+standing[1])/2.})))
    }
}
#[cfg(test)] mod tests {
 use super::*;
 fn input(t:f64,h:f64)->crate::actions::Input {crate::actions::Input{t,chest:None,chest_accel:None,hmd:Some(crate::actions::TimedHmd{height:h,yaw:0.,pitch:0.,roll:0.,seen:t})}}
 #[test] fn learns_both_crouches_and_applies_correction() {let mut s=Session::new(0.,0.1);let mut r=None;for i in 0..=1300 {let t=i as f64/100.;let h=if (3.3..5.).contains(&t) {1.0}else if (11.3..13.).contains(&t){1.1}else{1.7};r=s.update(input(t,h)).unwrap();}let r=r.unwrap();assert!((r["enter"].as_f64().unwrap()-1.44).abs()<1e-6);assert!((r["exit"].as_f64().unwrap()-1.62).abs()<1e-6);assert_eq!(r["use_reset_offset"],false);}
 #[test] fn rejects_failed_crouch_and_stale_headset(){let mut s=Session::new(0.,0.);let mut failed=false;for i in 0..=1300 {let t=i as f64/100.;let h=if (3.3..5.).contains(&t){1.0}else{1.7};if s.update(input(t,h)).is_err(){failed=true;break;}}assert!(failed);let mut s=Session::new(0.,0.);let mut i=input(1.,1.7);i.hmd.as_mut().unwrap().seen=0.;assert!(s.update(i).is_err());}
}
