use eframe::egui::{self,RichText,Color32};
use serde_json::{Value,json};
#[derive(Clone)]struct Draft{lean:bool,steps:bool,feet_steps:bool,backward:bool,strafe:bool,jump:bool,crouch:bool,sprint:bool,sprint_full_forward:bool,thighs:bool,head_turn:bool,max_output:f64}
impl Draft{
 fn from_state(s:&Value)->Self{let a=&s["actions"]["settings"];Self{lean:s["movement_engine"]!="chest_heading_steps",steps:s["step_input"]=="chest",feet_steps:s["step_input"]=="feet" || (s["step_input"].is_null() && s["movement_engine"]!="body_tilt"),backward:a["direction"]["forward_only"]==false,strafe:a["direction"]["strafe_enabled"]!=false,jump:a["jump"]["enabled"]==true,crouch:a["crouch"]["enabled"]==true,sprint:s["locomotion"]["settings"]["sprint_enabled"]==true,sprint_full_forward:a["direction"]["sprint_full_forward"]==true,thighs:s["sensor_layout"]["count"]==5,head_turn:a["turn"]["enabled"]==true,max_output:a["direction"]["max_output"].as_f64().unwrap_or(0.35)}}
 fn set_lean(&mut self,enabled:bool){self.lean=enabled;if !enabled {self.steps=false;self.backward=false;self.strafe=false;}}
 fn count(&self)->u64{if self.feet_steps{if self.thighs{5}else{3}}else{1}}
 fn choices(&self)->Value{json!({"lean":self.lean,"steps":self.steps||self.feet_steps,"step_input":if self.steps {"chest"}else if self.feet_steps {"feet"}else{"off"},"backward":self.backward,"strafe":self.strafe,"jump":self.jump,"crouch":self.crouch,"sprint":self.sprint,"sprint_full_forward":self.sprint_full_forward,"thighs":self.thighs,"head_turn":self.head_turn,"max_output":self.max_output})}
}
pub struct ProfileBuilder{draft:Option<Draft>,name:String,dirty:bool,last_active:String}
impl Default for ProfileBuilder{fn default()->Self{Self{draft:None,name:"My movement profile".into(),dirty:false,last_active:String::new()}}}
impl ProfileBuilder{
 pub fn show(&mut self,ui:&mut egui::Ui,s:&Value,online:bool)->Option<Value>{
  if !s["movement_engine"].is_string(){ui.label("Connecting to your saved profile...");return None;}
  let signature=serde_json::json!([s["movement_engine"],s["step_input"],s["actions"]["settings"],s["locomotion"]["settings"],s["sensor_layout"]["count"]]).to_string();
  if self.draft.is_none()||(!self.dirty&&signature!=self.last_active){self.draft=Some(Draft::from_state(s));self.last_active=signature;}
  let mut command=None;
  ui.horizontal(|ui|{ui.heading("How do you want to play?");if ui.small_button("Reload active settings").clicked(){self.draft=Some(Draft::from_state(s));self.dirty=false;}});
  let d=self.draft.as_mut().unwrap();
   egui::Frame::new().fill(Color32::from_rgb(17,32,46)).corner_radius(10.).inner_margin(12.).show(ui,|ui|{
    ui.columns(3,|cols|{
     cols[0].label(RichText::new("MOVEMENT").color(Color32::from_rgb(87,219,218)).strong());
     if cols[0].checkbox(&mut d.lean,"Lean to move").on_hover_text("Enable lean steering and full-forward lean sprint. Turning it off also turns off chest-wobble walking, which uses lean for direction.").changed(){self.dirty=true;d.set_lean(d.lean);}
     if cols[0].checkbox(&mut d.steps,"Walk in place").on_hover_text("Forward movement and sprint require learned chest wobble. When leaning to strafe is enabled, sideways lean strafes without walking.").changed(){self.dirty=true;if d.steps {d.feet_steps=false;d.thighs=false;d.lean=true;}}
     cols[0].small("1 sensor · learn wobble first; lean to steer");
     if cols[0].checkbox(&mut d.feet_steps,"Walk in place with feet").on_hover_text("Requires chest + left and right foot sensors. Replaces chest wobble.").changed(){self.dirty=true;if d.feet_steps {d.steps=false;}}
     cols[0].small("3 sensors · chest + both feet");

     if !d.lean{d.backward=false;d.strafe=false;}
     cols[0].add_enabled_ui(d.lean,|ui|{self.dirty|=ui.checkbox(&mut d.backward,"Allow backward movement").changed();self.dirty|=ui.checkbox(&mut d.strafe,"Enable leaning to strafe").changed();});
     if !d.lean && d.feet_steps {cols[0].small("Enable lateral lean independently on Movement > Test.");}
     cols[1].label(RichText::new("ACTIONS").color(Color32::from_rgb(87,219,218)).strong());
     cols[1].add_enabled_ui(d.lean||d.feet_steps,|ui|{self.dirty|=ui.checkbox(&mut d.sprint_full_forward,if d.feet_steps {"Lean to sprint"} else {"Sprint past sprint ring"}).changed();});
     self.dirty|=cols[1].checkbox(&mut d.jump,"Jump (uses sensor calibration)").changed();
     self.dirty|=cols[1].add_enabled(s["hmd_pose"]["fresh"]==true,egui::Checkbox::new(&mut d.crouch,"Crouch with headset height")).on_hover_text("Connect the headset to configure crouch.").changed();
     if !d.steps && !d.feet_steps{d.sprint=false;} if !d.feet_steps {d.thighs=false;}
     cols[1].add_enabled_ui(d.steps||d.feet_steps,|ui|{self.dirty|=ui.checkbox(&mut d.sprint,"Sprint while walking").changed();self.dirty|=ui.add_enabled(d.feet_steps,egui::Checkbox::new(&mut d.thighs,"Add thigh trackers (5 sensors)")).on_hover_text("SlimeVR placement. Thigh signals are monitored; this is not full-body IK.").changed();});
     cols[2].label(RichText::new("TURNING & CONTROLLERS").color(Color32::from_rgb(87,219,218)).strong());
     cols[2].label("Keep Quest controllers for hands,");cols[2].label("buttons and thumbstick turning.");
     self.dirty|=cols[2].add_enabled(s["hmd_pose"]["fresh"]==true,egui::Checkbox::new(&mut d.head_turn,"Head-turn steering")).on_hover_text("Optional: headset yaw drives virtual turning. Leave off for normal controller turning.").changed();
     cols[2].small("Physical 360° rotation stays available.");
    });
    ui.separator();ui.horizontal_wrapped(|ui|{ui.label("Movement limit");self.dirty|=ui.add(egui::Slider::new(&mut d.max_output,0.05..=1.).custom_formatter(|n,_|format!("{:.0}%",n*100.))).changed();ui.label("Profile name");ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(175.));
     if ui.add_enabled(online&&(d.lean||d.steps||d.feet_steps)&&!self.name.trim().is_empty(),egui::Button::new("Save & use profile").fill(Color32::from_rgb(36,135,143))).clicked(){command=Some(json!({"command":"apply_movement_profile","name":self.name.trim(),"choices":d.choices()}));self.dirty=false;}
    });
   });
  ui.horizontal_wrapped(|ui|{
   ui.label(RichText::new(format!("{} sensor{} required",d.count(),if d.count()==1{""}else{"s"})).strong().color(Color32::from_rgb(87,219,218)));
   ui.label(match d.count(){1=>"Chest",3=>"Chest + both ankles / feet",_=>"Chest + both thighs + both ankles"});
   if self.dirty{ui.colored_label(Color32::from_rgb(232,176,104),"Unsaved choices - Save & use to activate");}
  });
  ui.small("Quest 3 + SteamVR: keep your Touch controllers and bind ArcaneVR movement per game. Standalone Quest input injection is not available.");
  command
 }
}

#[cfg(test)] mod tests {
 use super::*;
 #[test] fn lean_can_be_changed_with_chest_walking_without_erasing_sprint_preference(){
  let s=json!({"movement_engine":"body_tilt_steps","step_input":"chest","sensor_layout":{"count":1},"actions":{"settings":{"direction":{"sprint_full_forward":true,"enabled":true}}}});
  let mut d=Draft::from_state(&s);assert!(d.steps&&d.lean&&d.sprint_full_forward);
  d.set_lean(false);assert!(!d.lean&&!d.steps);assert!(d.sprint_full_forward);
  d.set_lean(true);assert!(d.lean&&d.sprint_full_forward);assert_eq!(d.choices()["step_input"],"off");
 }
}
