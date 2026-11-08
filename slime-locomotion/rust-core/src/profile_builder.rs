//! Strict profile-builder translation; preserves all unselected calibration settings.
use serde_json::{json, Value};

pub fn build(current: &Value, choices: &Value) -> Result<Value,String> {
    let obj=choices.as_object().ok_or("choices must be an object")?;
    const KEYS:[&str;12]=["lean","steps","backward","strafe","jump","crouch","sprint","thighs","head_turn","max_output","sprint_full_forward","step_input"];
    if obj.keys().any(|k|!KEYS.contains(&k.as_str())) {return Err("choices contain unsupported fields".into());}
    let flag=|k:&str| obj.get(k).and_then(Value::as_bool).ok_or_else(||format!("{k} must be boolean"));
    let lean=flag("lean")?;let steps=flag("steps")?;let backward=flag("backward")?;let strafe=flag("strafe")?;let jump=flag("jump")?;let crouch=flag("crouch")?;let sprint=flag("sprint")?;let thighs=flag("thighs")?;let head_turn=flag("head_turn")?;
    let source=match obj.get("step_input") {Some(v)=>v.as_str().filter(|s|["off","chest","feet"].contains(s)).ok_or("Invalid step input")?,None=>if steps {"feet"}else{"off"}};
    if steps != (source!="off") {return Err("Walking choice and source disagree".into());}
    if source=="chest" && (!lean || thighs) {return Err("Chest wobble requires lean and one chest sensor".into());}
    if !lean&&!steps{return Err("Choose lean, steps, or both".into());}
    if (backward||strafe)&&!lean{return Err("Backward and strafe require lean".into());}
    if (thighs||sprint)&&!steps{return Err("Thigh sensors and sprint require steps".into());}
    let max=obj.get("max_output").and_then(Value::as_f64).filter(|x|x.is_finite()&&(0.05..=1.).contains(x)).ok_or("max_output must be between 0.05 and 1.0")?;
    let mut out=current.clone();if !out.is_object(){return Err("current profile is invalid".into());}
    out["sensor_layout"]=json!({"count":if !steps || source=="chest" {1}else if thighs {5}else{3}});
    out["step_input"]=json!(source);
    if source=="chest" {out["locomotion"]["algorithm"]=json!("continuous");}
    else if steps && source=="feet" {out["locomotion"]["algorithm"]=json!("cadence");}
    out["movement_engine"]=json!(if lean&&steps {"body_tilt_steps"} else if lean {"body_tilt"}else{"chest_heading_steps"});
    out["actions"]["direction"]["enabled"]=json!(lean);out["actions"]["direction"]["forward_only"]=json!(!backward);out["actions"]["direction"]["strafe_enabled"]=json!(strafe);out["actions"]["direction"]["max_output"]=json!(max);out["actions"]["direction"]["decouple"]=json!(false);
    if let Some(value)=obj.get("sprint_full_forward") {out["actions"]["direction"]["sprint_full_forward"]=json!(value.as_bool().ok_or("sprint_full_forward must be boolean")?);}
    out["actions"]["jump"]["enabled"]=json!(jump);if !matches!(out["actions"]["jump"]["source"].as_str(),Some("height"|"acceleration")){out["actions"]["jump"]["source"]=json!("height");}out["actions"]["crouch"]["enabled"]=json!(crouch);out["actions"]["crouch"]["use_reset_offset"]=json!(true);out["actions"]["crouch"]["mode"]=json!("hold");out["actions"]["turn"]["enabled"]=json!(head_turn);out["actions"]["turn"]["horizontal"]=json!(true);out["actions"]["turn"]["vertical"]=json!(false);
    out["locomotion"]["settings"]["sprint_enabled"]=json!(sprint);if sprint && source!="feet" {out["locomotion"]["algorithm"]=json!("continuous");}out["gestures"]["enabled"]=json!(false);out["output_mode"]=json!("steamvr");
    Ok(out)
}
#[cfg(test)] mod calibration_preservation {
 use super::*;
 #[test] fn calibrated_chest_jump_survives_profile_changes() {
  let current=json!({"actions":{"direction":{},"jump":{"enabled":true,"source":"acceleration","threshold":3.2,"release":1.1},"crouch":{},"turn":{}},"locomotion":{"settings":{}},"gestures":{}});
  let choices=json!({"lean":true,"steps":false,"backward":true,"strafe":true,"jump":true,"crouch":false,"sprint":false,"thighs":false,"head_turn":false,"max_output":0.35});
  let result=build(&current,&choices).unwrap();
  assert_eq!(result["actions"]["jump"]["source"],"acceleration");
  assert_eq!(result["actions"]["jump"]["threshold"],3.2);
  assert_eq!(result["actions"]["jump"]["release"],1.1);
 }
}
#[cfg(test)] mod tests {use super::*;
fn c()->Value{json!({"lean":true,"steps":false,"backward":false,"strafe":false,"jump":false,"crouch":false,"sprint":false,"thighs":false,"head_turn":false,"max_output":0.5})}
fn current()->Value{json!({"actions":{"direction":{"learned_forward":[1,0,0],"learned_right":[0,0,1],"deadzone":0.2},"jump":{},"crouch":{},"turn":{}},"locomotion":{"algorithm":"alternating","settings":{}},"gestures":{}})}
#[test] fn derives_modes_and_preserves_axes(){let mut x=c();let a=build(&current(),&x).unwrap();assert_eq!(a["sensor_layout"]["count"],1);assert_eq!(a["actions"]["direction"]["learned_right"],json!([0,0,1]));assert_eq!(a["actions"]["direction"]["deadzone"],0.2);x["steps"]=json!(true);x["thighs"]=json!(true);x["sprint"]=json!(true);let b=build(&current(),&x).unwrap();assert_eq!(b["movement_engine"],"body_tilt_steps");assert_eq!(b["sensor_layout"]["count"],5);assert_eq!(b["locomotion"]["algorithm"],"cadence");assert_eq!(b["locomotion"]["settings"]["sprint_enabled"],true);x["lean"]=json!(false);x["thighs"]=json!(false);let b=build(&current(),&x).unwrap();assert_eq!(b["movement_engine"],"chest_heading_steps");assert_eq!(b["sensor_layout"]["count"],3);}
#[test] fn rejects_incompatible_and_nontyped(){let mut x=c();x["lean"]=json!(false);assert!(build(&current(),&x).is_err());x["steps"]=json!(true);x["strafe"]=json!(true);assert!(build(&current(),&x).is_err());x=c();x["sprint"]=json!(true);assert!(build(&current(),&x).is_err());x=c();x["jump"]=json!(1);assert!(build(&current(),&x).is_err());x=c();x["max_output"]=json!(0.01);assert!(build(&current(),&x).is_err());}}

#[cfg(test)] mod walking_sources {
 use super::*;
 #[test] fn chest_and_feet_choose_distinct_sensor_contracts() {
  let current=json!({"actions":{"direction":{},"jump":{},"crouch":{},"turn":{}},"locomotion":{"settings":{}},"gestures":{}});
  let mut choices=json!({"lean":true,"steps":true,"step_input":"chest","backward":true,"strafe":true,"jump":false,"crouch":false,"sprint":false,"thighs":false,"head_turn":false,"max_output":0.5});
  let chest=build(&current,&choices).unwrap();assert_eq!(chest["step_input"],"chest");assert_eq!(chest["sensor_layout"]["count"],1);assert_eq!(chest["movement_engine"],"body_tilt_steps");assert_eq!(chest["locomotion"]["algorithm"],"continuous");
  choices["step_input"]=json!("feet");let feet=build(&current,&choices).unwrap();assert_eq!(feet["sensor_layout"]["count"],3);assert_eq!(feet["step_input"],"feet");
  choices["steps"]=json!(false);assert!(build(&current,&choices).is_err());
 }
}
