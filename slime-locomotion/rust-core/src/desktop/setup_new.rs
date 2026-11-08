//! Simplified sensor setup using the user's supplied character reference.
use eframe::egui::{self,Color32,RichText,TextureHandle,TextureOptions,Rect,Stroke};
use serde_json::{Value,json};
const CYAN:Color32=Color32::from_rgb(66,222,232);const MUTED:Color32=Color32::from_rgb(149,166,184);
fn roles(count:u64)->Vec<(&'static str,&'static str)>{match count{1=>vec![("torso","Chest")],5=>vec![("torso","Chest"),("left_thigh","Left thigh"),("right_thigh","Right thigh"),("left","Left ankle"),("right","Right ankle")],_=>vec![("torso","Chest"),("left","Left ankle / foot"),("right","Right ankle / foot")]}}
fn assigned<'a>(s:&'a Value,role:&str)->&'a str{if role.ends_with("thigh"){s["sensor_layout"][role].as_str().unwrap_or("")}else{s["roles"][role]["id"].as_str().unwrap_or("")}}
pub fn show(ui:&mut egui::Ui,s:&Value,texture:&mut Option<TextureHandle>,online:bool)->Option<Value>{
 let mut command=None;let count=s["sensor_layout"]["count"].as_u64().unwrap_or(1);
 ui.add_space(10.);
 let trackers=s["trackers"].as_array().cloned().unwrap_or_default();
 ui.horizontal_top(|ui|{
  let left_width=(ui.available_width()*0.30).clamp(230.,340.);
  ui.allocate_ui_with_layout(egui::vec2(left_width,340.),egui::Layout::top_down(egui::Align::Min),|ui|{
   ui.horizontal(|ui|{ui.strong(format!("Your sensors  ({})",trackers.len()));if ui.small_button("USB Wi-Fi setup").clicked(){command=Some(json!({"command":"open_usb"}));}});
   egui::ScrollArea::vertical().id_salt("new_sensor_list").max_height(310.).show(ui,|ui|{
    if trackers.is_empty(){ui.colored_label(MUTED,"No tracker packets yet. Power on a tracker or connect USB.");}
    for sensor in &trackers{
     let id=sensor["id"].as_str().unwrap_or("Unknown");let fresh=sensor["fresh"]==true;let peer=sensor["peer_active"]==true;
     egui::Frame::new().fill(Color32::from_rgb(20,34,49)).corner_radius(9.).inner_margin(12.).show(ui,|ui|{
      ui.set_min_width(left_width-37.);ui.colored_label(if fresh{CYAN}else{Color32::from_rgb(238,174,98)},RichText::new(id).strong());
      let label=if fresh{"Live motion"}else if peer{"Connected / waiting for pose"}else{"Offline / stale"};ui.small(format!("{}  |  {:.0} ms",label,sensor["age_ms"].as_f64().unwrap_or(0.)));
      ui.small(format!("Rotation speed  {:.2} rad/s",sensor["angular_speed"].as_f64().unwrap_or(0.)));
      if let Some(a)=sensor["acceleration"].as_array(){ui.small(format!("Accel  {:.2}, {:.2}, {:.2} m/s²",a[0].as_f64().unwrap_or(0.),a[1].as_f64().unwrap_or(0.),a[2].as_f64().unwrap_or(0.)));}else{ui.small("Acceleration unavailable");}
      if let Some(q)=sensor["q"].as_array(){ui.small(format!("Quaternion  {:.2}  {:.2}  {:.2}  {:.2}",q[0].as_f64().unwrap_or(0.),q[1].as_f64().unwrap_or(0.),q[2].as_f64().unwrap_or(0.),q[3].as_f64().unwrap_or(0.)));}
      let used=roles(count).into_iter().find(|(r,_)|assigned(s,r)==id).map(|(_,name)|name).unwrap_or("Not assigned");ui.colored_label(MUTED,used);
     });ui.add_space(8.);
    }
   });
  });
  ui.add_space(12.);
  let size=egui::vec2(ui.available_width().max(350.),340.);let(board,_)=ui.allocate_exact_size(size,egui::Sense::hover());let p=ui.painter_at(board);
  p.rect_filled(board,12.,Color32::from_rgb(8,22,34));
  if texture.is_none(){*texture=Some(ui.ctx().load_texture("arcane-clean-avatar",egui::ColorImage::from_rgba_unmultiplied([1024,1536],include_bytes!("../../assets/sensor-avatar-clean.rgba")),TextureOptions::LINEAR));}
  let image=Rect::from_center_size(board.center(),egui::vec2(216.,324.));p.image(texture.as_ref().unwrap().id(),image,Rect::from_min_max(egui::pos2(0.,0.),egui::pos2(1.,1.)),Color32::WHITE);
  for(role,name)in roles(count){
   let (fraction,side)=match role{"torso"=>(0.32,1.),"left_thigh"=>(0.60,1.),"right_thigh"=>(0.60,-1.),"left"=>(0.90,1.),_=>(0.90,-1.)};
   let point=egui::pos2(image.center().x+if role=="torso"{0.}else{side*32.},image.top()+image.height()*fraction);
   let width=((board.width()-image.width())/2.-15.).clamp(128.,215.);
   let x=if side>0.{board.right()-width-10.}else{board.left()+10.};let y=board.top()+match role{"torso"=>35.,"left_thigh"|"right_thigh"=>137.,_=>239.};
   let callout=Rect::from_min_size(egui::pos2(x,y),egui::vec2(width,94.));
   let id=assigned(s,role);let data=trackers.iter().find(|t|t["id"].as_str()==Some(id));let fresh=data.is_some_and(|d|d["fresh"]==true||d["sample_mode"]=="held");
   let color=if fresh{CYAN}else{Color32::from_rgb(224,156,114)};
   p.line_segment([point,egui::pos2(if side>0.{callout.left()}else{callout.right()},point.y)],Stroke::new(1.5,color));p.circle_filled(point,5.,color);
   p.rect_filled(callout,8.,Color32::from_rgb(19,37,52));p.rect_stroke(callout,8.,Stroke::new(1.,color),egui::StrokeKind::Inside);
   ui.add_enabled_ui(online,|ui|{ui.scope_builder(egui::UiBuilder::new().max_rect(callout.shrink(7.)).layout(egui::Layout::top_down(egui::Align::Min)),|ui|{
    ui.set_width(width-14.);ui.colored_label(color,RichText::new(name).strong());
    egui::ComboBox::from_id_salt(("sensor_role",role)).width(width-20.).selected_text(if id.is_empty(){"Select sensor"}else{id}).show_ui(ui,|ui|{
     if ui.selectable_label(id.is_empty(),"Unassigned").clicked(){command=Some(json!({"command":"assign_sensor_role","role":role,"id":""}));}
     for t in &trackers{let candidate=t["id"].as_str().unwrap_or("");if ui.selectable_label(candidate==id,candidate).clicked(){command=Some(json!({"command":"assign_sensor_role","role":role,"id":candidate}));}}
    });
    ui.small(if id.is_empty(){"Choose a tracker"}else if fresh{"Live"}else{"Waiting for tracker"});
   });});
  }
 });
 ui.add_space(10.);ui.horizontal(|ui|{
  let ready=s["calibration_ready"]==true&&online;
  if ui.add_enabled(ready,egui::Button::new("Reset Center - stand upright").fill(Color32::from_rgb(39,136,145))).clicked(){command=Some(json!({"command":"calibrate"}));}
  if s["calibrated"]==true&&ready{ui.colored_label(CYAN,"Calibrated - ready for local test");}else{ui.colored_label(MUTED,if ready{"Stand upright, then Reset Center"}else{s["calibration_error"].as_str().unwrap_or("Assign the required trackers")});}
 });
 command
}
#[cfg(test)]mod tests{use super::*;#[test]fn modes_only_expose_required_slots(){assert_eq!(roles(1),vec![("torso","Chest")]);assert_eq!(roles(3).len(),3);assert_eq!(roles(5).len(),5);assert!(roles(5).iter().any(|(r,_)|*r=="left_thigh"));}}
