//! Separate persistent mounting axes from the session's neutral rotation.
//! Follows SlimeVR's distinction between bone-axis alignment and reference offsets.
//! Adapted mathematical implementation; no SlimeVR source is copied.
type Q=[f64;4];
fn unit(q:Q)->Option<Q>{let n=q.iter().map(|v|v*v).sum::<f64>().sqrt();if n<1e-9||!n.is_finite(){None}else{Some(q.map(|v|v/n))}}
fn inv([w,x,y,z]:Q)->Q{[w,-x,-y,-z]}
fn mul([a,b,c,d]:Q,[w,x,y,z]:Q)->Q{[a*w-b*x-c*y-d*z,a*x+b*w+c*z-d*y,a*y-b*z+c*w+d*x,a*z+b*y-c*x+d*w]}
fn dot(a:[f64;3],b:[f64;3])->f64{a.iter().zip(b).map(|(x,y)|x*y).sum()}
fn normalized(v:[f64;3])->Option<[f64;3]>{let n=dot(v,v).sqrt();if n<1e-8||!n.is_finite(){None}else{Some(v.map(|x|x/n))}}
/// Returns [right lean, forward lean] in a calibrated body frame.
/// Mounting axes persist per tracker. Neutral is recreated after a reboot.
pub fn lean(q:Q,neutral:Q,forward:[f64;3],right:[f64;3])->Option<[f64;2]>{
 let f=normalized(forward)?;
 let overlap=dot(right,f);let r=normalized(std::array::from_fn(|i|right[i]-overlap*f[i]))?;
 let up=[f[1]*r[2]-f[2]*r[1],f[2]*r[0]-f[0]*r[2],f[0]*r[1]-f[1]*r[0]];
 let d=mul(inv(unit(neutral)?),unit(q)?);let v=[d[1],d[2],d[3]];
 // Conjugation into the orthonormal mounting frame: axes become body axes.
 let mut body=[d[0],dot(v,f),dot(v,r),dot(v,up)];
 // Remove heading twist separately; it must never redefine the lean axes.
 let n=body[0].hypot(body[3]);if n<1e-8{return None;}
 body=mul([body[0]/n,0.,0.,-body[3]/n],body);
 if body[0]<0.{body=body.map(|x|-x);}
 let length=(body[1]*body[1]+body[2]*body[2]+body[3]*body[3]).sqrt();
 if length<1e-9{return Some([0.,0.]);}
 let scale=2.*length.atan2(body[0])/length;
 Some([body[2]*scale,body[1]*scale])
}
/// Relative world-up twist, independent of sensor mounting and lean magnitude.
pub fn heading(q:Q,neutral:Q,world_up:[f64;3])->Option<f64> {
 let d=mul(unit(q)?,inv(unit(neutral)?));let up=normalized(world_up)?;
 let projection=dot([d[1],d[2],d[3]],up);
 if d[0].hypot(projection)<1e-8 {return None;}
 let angle=2.*projection.atan2(d[0]);Some(angle.sin().atan2(angle.cos()))
}
/// Heading-free rotation in the neutral sensor frame. World up comes from the
/// receiver convention, never from learned (possibly yaw-contaminated) leans.
pub fn level_tilt(q:Q, neutral:Q, world_up:[f64;3])->Option<[f64;3]> {
 let neutral=unit(neutral)?;
 let u=normalized(world_up)?;
 let local=mul(mul(inv(neutral),[0.,u[0],u[1],u[2]]),neutral);
 let up=[local[1],local[2],local[3]];
 let d=mul(inv(neutral),unit(q)?);
 let projection=dot([d[1],d[2],d[3]],up);
 let n=d[0].hypot(projection);if n<1e-8{return None;}
 let mut swing=mul([d[0]/n,-up[0]*projection/n,-up[1]*projection/n,-up[2]*projection/n],d);
 if swing[0]<0. {swing=swing.map(|v|-v);}
 let v=[swing[1],swing[2],swing[3]];let length=dot(v,v).sqrt();
 if length<1e-9{return Some([0.;3]);}
 let scale=2.*length.atan2(swing[0])/length;
 Some(v.map(|v|v*scale))
}
pub fn lean_with_up(q:Q,neutral:Q,forward:[f64;3],right:[f64;3],world_up:[f64;3])->Option<[f64;2]> {
 let n=unit(neutral)?;let u=normalized(world_up)?;
 let local=mul(mul(inv(n),[0.,u[0],u[1],u[2]]),n);let up=[local[1],local[2],local[3]];
 let f=normalized(std::array::from_fn(|i|forward[i]-dot(forward,up)*up[i]))?;
 let r=normalized(std::array::from_fn(|i|right[i]-dot(right,up)*up[i]-dot(right,f)*f[i]))?;
 let v=level_tilt(q,neutral,world_up)?;
 Some([dot(v,r),dot(v,f)])
}
#[cfg(test)] mod tests{
 use super::*;
 #[test] fn heading_does_not_flip_under_deep_lean_with_arbitrary_mount() {
  let mount=unit([-0.486,-0.385,0.470,0.628]).unwrap();
  for up in [[0.,1.,0.],[0.,0.,1.]] {
   let axis=if up[1]==1. {[0.,0.,1.]}else{[0.,1.,0.]};
   for degrees in -75..=75 {
    let h=(degrees as f64).to_radians()/2.;
    for a in [[1.,0.,0.],axis] {
     let lean=[h.cos(),a[0]*h.sin(),a[1]*h.sin(),a[2]*h.sin()];
     assert!(heading(mul(lean,mount),mount,up).unwrap().abs()<1e-9);
     for yaw in [-2.8,-1.,0.,1.,2.8] {let h:f64=yaw/2.;let turn=[h.cos(),up[0]*h.sin(),up[1]*h.sin(),up[2]*h.sin()];
      assert!((heading(mul(mul(turn,lean),mount),mount,up).unwrap()-yaw).abs()<1e-9);
     }
    }
   }
  }
 }

 #[test] fn contaminated_training_and_heading_drift_do_not_create_lean() {
  fn rotation(axis:[f64;3],angle:f64)->Q {let h=angle/2.;[h.cos(),axis[0]*h.sin(),axis[1]*h.sin(),axis[2]*h.sin()]}
  for up in [[0.,0.,1.],[0.,1.,0.]] {
   let body_right=if up[2]==1. {[0.,1.,0.]}else{[0.,0.,1.]};
   for mount in [[1.,0.,0.,0.],rotation([1.,0.,0.],0.7)] {
    let forward_pose=mul(mul(rotation(up,0.1),rotation([1.,0.,0.],0.25)),mount);
    let right_pose=mul(mul(rotation(up,-0.15),rotation(body_right,0.25)),mount);
    let f=normalized(level_tilt(forward_pose,mount,up).unwrap()).unwrap();
    let r=normalized(level_tilt(right_pose,mount,up).unwrap()).unwrap();
    // Also retain an old contaminated mapping: runtime removes its vertical component.
    let old=mul(inv(mount),forward_pose);let old_f=normalized([old[1],old[2],old[3]]).unwrap();
    for degrees in -179..=179 {
     let yaw=rotation(up,(degrees as f64).to_radians());
     for learned in [f,old_f] {
      let got=lean_with_up(mul(yaw,mount),mount,learned,r,up).unwrap();
      assert!(got[0].abs()<1e-8&&got[1].abs()<1e-8,"pure heading generated {got:?}");
     }
     for (axis,expected) in [([1.,0.,0.],[0.,0.2]),(body_right,[0.2,0.]),([-1.,0.,0.],[0.,-0.2]),(body_right.map(|v|-v),[-0.2,0.])] {
      let got=lean_with_up(mul(mul(yaw,rotation(axis,0.2)),mount),mount,f,r,up).unwrap();
      assert!((got[0]-expected[0]).abs()<1e-8&&(got[1]-expected[1]).abs()<1e-8,"{degrees}: {got:?}");
     }
    }
   }
  }
 }

 #[test] fn physical_face_turnover_with_nonidentity_mount_and_heading(){
  fn rot(q:Q,v:[f64;3])->[f64;3]{let a=mul(mul(q,[0.,v[0],v[1],v[2]]),inv(q));[a[1],a[2],a[3]]}
  let a=0.63_f64;let original=[a.cos(),a.sin(),0.,0.];
  for heading in [0_f64,0.8,2.7,-2.9]{
   let boot=[(heading/2.).cos(),0.,0.,(heading/2.).sin()];
   for turned in [false,true]{
    // Turn the physical board over around body up while keeping its top upright.
    let body_flip=if turned{[0.,0.,0.,1.]}else{[1.,0.,0.,0.]};
    let mount=mul(body_flip,original);
    let neutral=mul(boot,mount);
    let forward=rot(inv(mount),[1.,0.,0.]);let right=rot(inv(mount),[0.,1.,0.]);
    for (rotation,expected) in [([0.1_f64.cos(),0.1_f64.sin(),0.,0.],[0.,0.2]),([0.1_f64.cos(),-0.1_f64.sin(),0.,0.],[0.,-0.2]),([0.1_f64.cos(),0.,0.1_f64.sin(),0.],[0.2,0.]),([0.1_f64.cos(),0.,-0.1_f64.sin(),0.],[-0.2,0.])]{
     let actual=lean(mul(mul(boot,rotation),mount),neutral,forward,right).unwrap();
     assert!((actual[0]-expected[0]).abs()<1e-8 && (actual[1]-expected[1]).abs()<1e-8,"turned={turned} heading={heading}: {actual:?}");
    }
   }
  }
 }
 #[test] fn boot_heading_and_opposite_face_do_not_reverse_known_motion(){
  for yaw in [0.,0.8,2.4,-2.8]{
   let boot=[(yaw/2_f64).cos(),0.,0.,(yaw/2_f64).sin()];
   for face in [1.,-1.]{
    let f=[face,0.,0.];let r=[0.,face,0.];
    for (q,expected) in [([(0.1_f64).cos(),face*(0.1_f64).sin(),0.,0.],[0.,0.2]),([(0.1_f64).cos(),0.,face*(0.1_f64).sin(),0.],[0.2,0.]),([(0.1_f64).cos(),-face*(0.1_f64).sin(),0.,0.],[0.,-0.2])]{
     let got=lean(mul(boot,q),boot,f,r).unwrap();assert!((got[0]-expected[0]).abs()<1e-8 && (got[1]-expected[1]).abs()<1e-8);
    }
    for turn in [0.7,2.8,-2.9]{let yawq=[(turn/2_f64).cos(),0.,0.,(turn/2_f64).sin()];let got=lean(mul(boot,yawq),boot,f,r).unwrap();assert!(got[0].abs()<1e-8&&got[1].abs()<1e-8);}
   }
  }
 }
}
