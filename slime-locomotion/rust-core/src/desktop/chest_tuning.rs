use super::pages::{self, ORANGE, MUTED};
use eframe::egui::{self, Color32, RichText, Stroke};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct ChestTuning { draft: Value, dirty: bool, pending: Option<Value>, history: Vec<f32>, last_sample: Option<Instant> }
pub enum Action { Command(Value), ResetCenter, SensorCalibration, TestRoom }
const CYAN: Color32 = Color32::from_rgb(64, 220, 229);
fn number(v: &Value) -> f64 { v.as_f64().unwrap_or(0.) }
fn slider(ui: &mut egui::Ui, d: &mut Value, key: &str, label: &str, range: std::ops::RangeInclusive<f64>, factor: f64) -> bool {
 let mut n=number(&d[key])*factor; let changed=ui.horizontal(|ui|{ui.add_sized([155.,22.],egui::Label::new(label));ui.spacing_mut().slider_width=(ui.available_width()-68.).max(60.);ui.add(egui::Slider::new(&mut n,range)).changed()}).inner; if changed { d[key]=json!(n/factor); } changed
}
fn clamp_sprint(d:&mut Value) {let full=d["horizontal_max_angle"].as_f64().unwrap_or(number(&d["max_angle"])).max(d["forward_max_angle"].as_f64().unwrap_or(number(&d["max_angle"])));if let Some(s)=d["sprint_angle"].as_f64(){d["sprint_angle"]=json!(s.max(full));}}
fn baseline(d:&mut Value) {d["deadzone"]=json!(0.0);for key in ["forward_deadzone","backward_deadzone","left_deadzone","right_deadzone"] {d[key]=json!(25_f64.to_radians()*0.05);}d["max_angle"]=json!(25_f64.to_radians());d["horizontal_max_angle"]=Value::Null;d["forward_max_angle"]=Value::Null;d["exponent"]=json!(1.0);d["max_output"]=json!(1.0);clamp_sprint(d);}
fn reference_slider(ui:&mut egui::Ui,d:&mut Value,key:&str,label:&str,range:std::ops::RangeInclusive<f64>,factor:f64,unit:&str,fallback:f64)->bool {
 let mut value=d[key].as_f64().unwrap_or(fallback)*factor;
 let mut changed=false;
 ui.add_space(6.);
 ui.horizontal(|ui|{ui.label(label);ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
  egui::Frame::new().fill(Color32::from_rgb(17,25,36)).stroke(Stroke::new(1.,Color32::from_rgb(48,65,80))).corner_radius(8.).inner_margin(6.).show(ui,|ui|{changed|=ui.add(egui::DragValue::new(&mut value).range(range.clone()).speed(0.1).suffix(unit).max_decimals(1)).changed();});
 });});
 let (rect,response)=ui.allocate_exact_size(egui::vec2(ui.available_width(),32.),egui::Sense::click_and_drag());
 let track=rect.shrink2(egui::vec2(15.,0.));let lo=*range.start();let hi=*range.end();
 if response.dragged()||response.clicked(){if let Some(p)=response.interact_pointer_pos(){value=lo+((p.x-track.left())/track.width()).clamp(0.,1.) as f64*(hi-lo);changed=true;}}
 let p=ui.painter_at(rect);let y=rect.center().y;let c=egui::pos2(track.left()+track.width()*((value-lo)/(hi-lo)).clamp(0.,1.) as f32,y);
 p.line_segment([egui::pos2(track.left(),y),egui::pos2(track.right(),y)],Stroke::new(7.,Color32::from_rgb(48,65,80)));
 p.line_segment([egui::pos2(track.left(),y),c],Stroke::new(7.,ORANGE));p.circle_filled(c,14.,Color32::from_rgb(20,30,42));p.circle_stroke(c,14.,Stroke::new(2.,ORANGE));
 for dx in [-4.,0.,4.] {p.line_segment([c+egui::vec2(dx,-4.),c+egui::vec2(dx,4.)],Stroke::new(1.,ORANGE));}
 if changed {d[key]=json!(value/factor);clamp_sprint(d);}changed
}
pub(super) fn reference_controls(ui:&mut egui::Ui,d:&mut Value,hmd_ready:bool)->bool {
 let mut changed=false;let fallback=number(&d["max_angle"]);let deg=180./std::f64::consts::PI;
 let dead=number(&d["deadzone"])*deg;
 let backward=number(&d["backward_deadzone_multiplier"]).max(1.);
 changed|=reference_slider(ui,d,"horizontal_max_angle","Horizontal Sensitivity (L/R)",(dead+0.2).max(1.)..=179.,deg,"°",fallback);
 changed|=reference_slider(ui,d,"forward_max_angle","Forward Sensitivity (F/B)",(dead*backward+0.2).max(1.)..=179.,deg,"°",fallback);
 changed|=reference_slider(ui,d,"forward_offset","Forward/Back Offset",-30.0..=30.,deg,"°",0.);
 if ui.button("Reset Offset").on_hover_text("Set the manual forward/back offset to zero; learned mounting is preserved.").clicked(){d["forward_offset"]=json!(0.);changed=true;}
 ui.separator();ui.colored_label(ORANGE,"Smoothing");
 changed|=reference_slider(ui,d,"smoothing_ms","Smoothing",0.0..=500.,1.," ms",0.);
 ui.small("Higher = more inertia (less jitter, more lag). 0 ms = instant.");
 ui.separator();ui.colored_label(ORANGE,"Decouple Head Direction");
 ui.add_enabled_ui(hmd_ready,|ui|{if check(ui,d,"decouple","Decouple Head Direction"){if d["decouple"]==true {d["decouple_strength"]=json!(1.);}changed=true;}});
 if !hmd_ready {ui.colored_label(ORANGE,"Connect the headset to use head decoupling.");}
 ui.small("Move relative to your chest while looking around. Requires a connected headset.");
 changed
}
pub(super) fn tilt_range(ui:&mut egui::Ui,d:&mut Value)->bool {
 let mut dead=number(&d["deadzone"]).to_degrees() as f32;
 let mut max=number(&d["max_angle"]).to_degrees() as f32;
 ui.label("Deadzone / Max Tilt");
 ui.small(format!("Deadzone: {dead:.1}°  |  Max tilt: {max:.1}°"));
 let (rect,mut response)=ui.allocate_exact_size(egui::vec2(ui.available_width(),40.),egui::Sense::click_and_drag());
 let track=rect.shrink2(egui::vec2(16.,0.));
 let pos=|angle:f32|track.left()+track.width()*angle/179.;
 let handle=ui.make_persistent_id("tilt_range_handle");
 if response.drag_started() || response.clicked() {
  if let Some(p)=response.interact_pointer_pos(){ui.data_mut(|data|data.insert_temp(handle,(p.x-pos(dead)).abs()<=(p.x-pos(max)).abs()));}
 }
 let mut changed=false;
 if response.dragged() || response.clicked() {
  if let Some(p)=response.interact_pointer_pos(){
   let angle=((p.x-track.left())/track.width()*179.).clamp(0.,179.);
   let lower=ui.data(|data|data.get_temp::<bool>(handle)).unwrap_or(true);
   let factor=number(&d["backward_deadzone_multiplier"]).max(1.) as f32;
   if lower {
    let horizontal=d["horizontal_max_angle"].as_f64().unwrap_or(number(&d["max_angle"])).to_degrees() as f32;
    let forward=d["forward_max_angle"].as_f64().unwrap_or(number(&d["max_angle"])).to_degrees() as f32;
    dead=angle.min(85.).min((horizontal-0.2).min((forward-0.2)/factor).min((max-0.2)/factor).max(0.));
   } else {
    max=angle.max((dead*factor+0.2).max(1.)).min(179.);
    // The overall maximum resets both sensitivities; each can then be tuned separately.
    d["horizontal_max_angle"]=Value::Null;d["forward_max_angle"]=Value::Null;
   }
   d["deadzone"]=json!((dead as f64).to_radians());d["max_angle"]=json!((max as f64).to_radians());
   clamp_sprint(d);changed=true;response.mark_changed();
  }
 }
 let painter=ui.painter_at(rect);let y=rect.center().y;
 painter.line_segment([egui::pos2(track.left(),y),egui::pos2(track.right(),y)],Stroke::new(8.,Color32::from_rgb(48,65,80)));
 painter.line_segment([egui::pos2(pos(dead),y),egui::pos2(pos(max),y)],Stroke::new(8.,ORANGE));
 for angle in [dead,max] {let c=egui::pos2(pos(angle),y);painter.circle_filled(c,14.,Color32::from_rgb(20,30,42));painter.circle_stroke(c,14.,Stroke::new(2.,ORANGE));for dx in [-4.,0.,4.] {painter.line_segment([c+egui::vec2(dx,-4.),c+egui::vec2(dx,4.)],Stroke::new(1.,ORANGE));}}
 response.on_hover_text("Left: deadzone. Right: set both sensitivity angles together; use the separate controls below to tune each axis.");
 changed
}
// Returns directional input-angle thresholds, applying the receiver's legacy fallback.
fn deadzones(d:&Value)->[f64;4] {let common=number(&d["deadzone"]);[d["forward_deadzone"].as_f64().unwrap_or(common),d["backward_deadzone"].as_f64().unwrap_or(common*number(&d["backward_deadzone_multiplier"]).max(1.)),d["left_deadzone"].as_f64().unwrap_or(common),d["right_deadzone"].as_f64().unwrap_or(common)]}
fn set_deadzone(d:&mut Value, values:&mut [f64;4], index:usize, value:f64) {
 let keys=["forward_deadzone","backward_deadzone","left_deadzone","right_deadzone"];
 values[index]=value; d[keys[index]]=json!(value);
 if index>=2 {let other=if index==2 {3}else{2};values[other]=value;d[keys[other]]=json!(value);}
}
fn adjusted_angles(preview:&Value,saved:&Value,fresh:bool)->Option<[f64;2]> {if !fresh{return None;}let a=preview["raw_direction_angles"].as_array()?;if a.len()!=2{return None;}let x=a[0].as_f64().filter(|v|v.is_finite())?;let y=a[1].as_f64().filter(|v|v.is_finite())?;Some([x,y-number(&saved["forward_offset"])])}
fn fit_rings(d:&mut Value,view:f64)->bool {
 let base=number(&d["max_angle"]);
 let full=d["horizontal_max_angle"].as_f64().unwrap_or(base).max(d["forward_max_angle"].as_f64().unwrap_or(base));
 let sprint=d["sprint_angle"].as_f64().unwrap_or((full*1.15).min(3.14)).max(full);
 let factor=(view*0.9/sprint).min(1.);
 if factor>=1. {return false;}
 for key in ["max_angle","horizontal_max_angle","forward_max_angle","deadzone","forward_deadzone","backward_deadzone","left_deadzone","right_deadzone"] {if let Some(v)=d[key].as_f64(){d[key]=json!(v*factor);}}
 d["sprint_angle"]=json!(sprint*factor);true
}
#[derive(Debug, PartialEq)]
struct OutputMeter { x: f32, y: f32, sprint: bool, armed: bool }
fn output_meter(state:&Value, online:bool, fresh:bool)->Option<OutputMeter> {
 if !online || !fresh { return None; }
 let output=&state["game_output_preview"];
 let xy=output["travel_xy"].as_array()?;
 let axis=|i:usize| xy.get(i)?.as_f64().filter(|v|v.is_finite()).map(|v|v.clamp(-1.,1.) as f32);
 Some(OutputMeter {x:axis(0)?, y:axis(1)?, sprint:output["buttons"].as_u64()? & 4 != 0, armed:output["armed"].as_bool()?})
}
pub(super) fn target(ui:&mut egui::Ui,d:&mut Value,_saved:&Value,preview:&Value,state:&Value,online:bool,fresh:bool,details:bool)->bool {
 let mut changed=check(ui,d,"sprint_full_forward",if state["step_input"]=="feet" {"Lean to sprint"}else{"Sprint"});
 let mut full_x=d["horizontal_max_angle"].as_f64().unwrap_or(number(&d["max_angle"]));
 let mut full_y=d["forward_max_angle"].as_f64().unwrap_or(number(&d["max_angle"]));
 let mut full=full_x.max(full_y);let mut sprint=d["sprint_angle"].as_f64().unwrap_or((full*1.15).min(3.14)).max(full);
 let side=ui.available_width().min(640.).max(160.);let (area,response)=ui.allocate_exact_size(egui::vec2(ui.available_width(),side),egui::Sense::click_and_drag());let p=ui.painter().clone();let c=area.center();let mut radius=(side*0.39).min((side*0.5-40.).max(20.));
 // The outer boundary keeps all editable thresholds visible.
 let zoom_key=ui.make_persistent_id("lean_ring_zoom");
 let start_key=ui.make_persistent_id("lean_ring_zoom_start");
 let mut top=c-egui::vec2(0.,radius);
 let zoom_response=ui.interact(egui::Rect::from_center_size(top-egui::vec2(0.,8.),egui::vec2(100.,40.)),ui.make_persistent_id("lean_ring_zoom_grip"),egui::Sense::drag());
 let mut view=ui.data(|data|data.get_temp::<f64>(zoom_key)).unwrap_or((sprint*1.35).max(0.8));
 if zoom_response.drag_started(){ui.data_mut(|data|data.insert_temp(start_key,view));}
 if zoom_response.dragged() || zoom_response.drag_stopped(){
  let start=ui.data(|data|data.get_temp::<f64>(start_key)).unwrap_or(view);
  if let Some(pointer)=zoom_response.interact_pointer_pos(){
   let fraction=((c.y-pointer.y)/radius).max(0.03) as f64;
   view=(start*fraction).clamp(0.1,3.3);
   changed|=fit_rings(d,view);
   full_x=d["horizontal_max_angle"].as_f64().unwrap_or(number(&d["max_angle"]));full_y=d["forward_max_angle"].as_f64().unwrap_or(number(&d["max_angle"]));full=full_x.max(full_y);sprint=d["sprint_angle"].as_f64().unwrap_or(full);
   // Keep the angular scale fixed during the gesture: the entire outer ring
   // follows the pointer, including outside its original bounds. Fit on release.
   if zoom_response.dragged(){radius=(radius*(view/start) as f32).min(side*0.42);top=c-egui::vec2(0.,radius);}
  }
 }
 if !zoom_response.dragged() && !zoom_response.drag_stopped(){view=view.max(sprint/0.9);}
 ui.data_mut(|data|data.insert_temp(zoom_key,view));
 zoom_response.on_hover_text("Pull the outer ring down or up to choose the displayed range. Shrinking pulls all inner rings inside the boundary. Apply to save changed thresholds.");
 let offsets=[egui::vec2(0.,-1.),egui::vec2(0.,1.),egui::vec2(-1.,0.),egui::vec2(1.,0.)];
 let mut values=deadzones(d);let handle=ui.make_persistent_id("lean_ring_handle");
 let mut points:Vec<_>=(0..4).map(|i|c+offsets[i]*(radius*(values[i]/view) as f32)).collect();
 points.push(c+egui::vec2(0.707,-0.707)*(radius*(full/view) as f32));
 points.push(c+egui::vec2(0.707,0.707)*(radius*(sprint/view) as f32));
 if response.drag_started(){ui.data_mut(|data|data.remove::<usize>(handle));if let Some(pos)=response.interact_pointer_pos(){let end=if d["sprint_full_forward"]==true{6}else{5};let nearest=(0..end).min_by(|a,b|points[*a].distance(pos).total_cmp(&points[*b].distance(pos))).unwrap();if points[nearest].distance(pos)<26.{ui.data_mut(|data|data.insert_temp(handle,nearest));}}}
 if response.dragged(){if let (Some(pos),Some(index))=(response.interact_pointer_pos(),ui.data(|data|data.get_temp::<usize>(handle))){
  if index<4 {let limit=if index<2{full_y}else{full_x};let value=((pos-c).dot(offsets[index]).max(0.) as f64/radius as f64*view).clamp(0.,(limit-0.003).max(0.));set_deadzone(d,&mut values,index,value);}
  else if index==4 {let minimum=values.iter().copied().fold(number(&d["deadzone"])*number(&d["backward_deadzone_multiplier"]).max(0.01),f64::max)+0.003;full=((pos-c).length() as f64/radius as f64*view).clamp(minimum,(view*0.9).min(3.14).max(minimum));d["max_angle"]=json!(full);d["forward_max_angle"]=json!(full);d["horizontal_max_angle"]=json!(full);sprint=sprint.max(full);d["sprint_angle"]=json!(sprint);}
  else {sprint=((pos-c).length() as f64/radius as f64*view).clamp(full,(view*0.9).min(3.14).max(full));d["sprint_angle"]=json!(sprint);}
  changed=true;
 }}
 if response.drag_stopped(){ui.data_mut(|data|{data.remove::<usize>(handle);});}
 p.circle_filled(c,radius,Color32::from_rgb(11,25,37));for f in [0.25,0.5,0.75,1.]{p.circle_stroke(c,radius*f,Stroke::new(1.,Color32::from_rgb(44,62,76)));}
 p.circle_filled(top,7.,CYAN);p.text(top+egui::vec2(0.,-31.),egui::Align2::CENTER_CENTER,format!("Range {:.1}° ↕",view.to_degrees()),egui::FontId::proportional(12.),CYAN);
 p.line_segment([c-egui::vec2(radius,0.),c+egui::vec2(radius,0.)],Stroke::new(1.,MUTED));p.line_segment([c-egui::vec2(0.,radius),c+egui::vec2(0.,radius)],Stroke::new(1.,MUTED));
 let ellipse=|rx:f64,ry:f64,color:Color32|{let points=(0..=128).map(|n|{let a=n as f32*std::f32::consts::TAU/128.;c+egui::vec2(a.cos()*rx as f32,a.sin()*ry as f32)*radius/view as f32}).collect();p.add(egui::Shape::line(points,Stroke::new(2.,color)));};
 let dead=(0..=128).map(|n|{let a=n as f32*std::f32::consts::TAU/128.;let x=a.cos();let y=a.sin();c+egui::vec2(x*values[if x<0.{2}else{3}] as f32,y*values[if y<0.{0}else{1}] as f32)*radius/view as f32}).collect();p.add(egui::Shape::line(dead,Stroke::new(2.,ORANGE)));
 for i in 0..4 {p.circle_filled(c+offsets[i]*(radius*(values[i]/view) as f32),6.,ORANGE);}
 let green=Color32::from_rgb(93,226,165);let purple=Color32::from_rgb(197,151,255);
 ellipse(d["horizontal_max_angle"].as_f64().unwrap_or(full),d["forward_max_angle"].as_f64().unwrap_or(full),green);
 let fp=c+egui::vec2(0.707,-0.707)*(radius*(full/view) as f32);p.circle_filled(fp,7.,green);p.text(fp+egui::vec2(10.,-9.),egui::Align2::LEFT_BOTTOM,"Full speed",egui::FontId::proportional(12.),green);
 if d["sprint_full_forward"]==true {ellipse(sprint,sprint,purple);let sp=c+egui::vec2(0.707,0.707)*(radius*(sprint/view) as f32);p.circle_filled(sp,7.,purple);p.text(sp+egui::vec2(10.,9.),egui::Align2::LEFT_TOP,"Sprint",egui::FontId::proportional(12.),purple);}
 for (offset,label) in [(egui::vec2(48.,-radius-13.),"FORWARD"),(egui::vec2(35.,radius+13.),"BACK"),(egui::vec2(-radius-15.,20.),"L"),(egui::vec2(radius+15.,20.),"R")] {p.text(c+offset,egui::Align2::CENTER_CENTER,label,egui::FontId::proportional(11.),MUTED);}
 if let Some([x,y])=adjusted_angles(preview,_saved,fresh){let vector=egui::vec2((x/view) as f32,-(y/view) as f32);let point=c+vector/vector.length().max(1.)*radius;p.line_segment([c,point],Stroke::new(2.,CYAN));p.circle_filled(point,6.,CYAN);}
 if fresh {if let Some(yaw)=preview["chest_yaw"].as_f64().filter(|v|v.is_finite()) {
  let radial=egui::vec2(yaw.sin() as f32,-yaw.cos() as f32);let tangent=egui::vec2(-radial.y,radial.x);let tip=c+radial*(radius+10.);let base=c+radial*(radius+22.);
  p.add(egui::Shape::convex_polygon(vec![tip,base+tangent*6.,base-tangent*6.],CYAN,Stroke::NONE));
 }}
 let yc=egui::pos2(area.right()-28.,area.top()+26.);p.circle_stroke(yc,18.,Stroke::new(1.,MUTED));
 if fresh {if let Some(yaw)=preview["chest_yaw"].as_f64(){p.line_segment([yc,yc+egui::vec2(yaw.sin() as f32,-yaw.cos() as f32)*15.],Stroke::new(2.,CYAN));}}
 p.text(yc+egui::vec2(0.,23.),egui::Align2::CENTER_TOP,"YAW",egui::FontId::proportional(10.),MUTED);
 // The bars show final receiver stick output; the rings above are lean angles.
 let meter=output_meter(state,online,fresh);
 let bar_color=Color32::from_rgb(64,220,229);
 let horizontal=egui::Rect::from_center_size(egui::pos2(c.x,c.y+radius+28.),egui::vec2(radius*1.6,10.));
 let vertical=egui::Rect::from_center_size(egui::pos2(c.x+radius+28.,c.y),egui::vec2(10.,radius*1.6));
 for r in [horizontal,vertical] {p.rect_filled(r,3.,Color32::from_rgb(48,65,80));}
 if let Some(m)=&meter {
  let h=egui::pos2(horizontal.center().x+m.x*horizontal.width()*0.5,horizontal.center().y);
  let v=egui::pos2(vertical.center().x,vertical.center().y-m.y*vertical.height()*0.5);
  p.line_segment([horizontal.center(),h],Stroke::new(7.,bar_color));p.circle_filled(h,5.,bar_color);
  p.line_segment([vertical.center(),v],Stroke::new(7.,bar_color));p.circle_filled(v,5.,bar_color);
 }
 p.line_segment([horizontal.center_top(),horizontal.center_bottom()],Stroke::new(1.,Color32::WHITE));
 p.line_segment([vertical.left_center(),vertical.right_center()],Stroke::new(1.,Color32::WHITE));
 // Adjacent thin guides have the ring's angular scale, distinct from stick output.
 let hy=horizontal.top()-6.;let vx=vertical.right()+6.;
 p.line_segment([egui::pos2(horizontal.left(),hy),egui::pos2(horizontal.right(),hy)],Stroke::new(1.,MUTED));
 p.line_segment([egui::pos2(vx,vertical.top()),egui::pos2(vx,vertical.bottom())],Stroke::new(1.,MUTED));
 let hx=|angle:f64| horizontal.center().x+(angle/view).clamp(-1.,1.) as f32*horizontal.width()*0.5;
 let vy=|angle:f64| vertical.center().y-(angle/view).clamp(-1.,1.) as f32*vertical.height()*0.5;
 for angle in [-values[2],values[3]] {let x=hx(angle);p.line_segment([egui::pos2(x,hy-3.),egui::pos2(x,hy+3.)],Stroke::new(2.,ORANGE));}
 for angle in [-values[1],values[0]] {let y=vy(angle);p.line_segment([egui::pos2(vx-3.,y),egui::pos2(vx+3.,y)],Stroke::new(2.,ORANGE));}
 for angle in [-full_y,full_y] {let y=vy(angle);p.line_segment([egui::pos2(vx-3.,y),egui::pos2(vx+3.,y)],Stroke::new(2.,green));}
 if d["sprint_full_forward"]==true {let y=vy(sprint);p.line_segment([egui::pos2(vx-3.,y),egui::pos2(vx+3.,y)],Stroke::new(2.,purple));}

 p.text(horizontal.center_bottom()+egui::vec2(0.,3.),egui::Align2::CENTER_TOP,"L -100%    0    +100% R",egui::FontId::proportional(10.),MUTED);
 p.text(vertical.center_top()+egui::vec2(0.,-2.),egui::Align2::CENTER_BOTTOM,"F 100%",egui::FontId::proportional(10.),MUTED);
 p.text(vertical.center_bottom()+egui::vec2(0.,2.),egui::Align2::CENTER_TOP,"B 100%",egui::FontId::proportional(10.),MUTED);
 match meter {
  Some(m)=>{ui.label(format!("{} · stick L/R {:+.0}% · F/B {:+.0}% · Sprint button {}",if m.armed {"Movement preview · game output armed"} else {"Movement preview · game output stopped"},m.x*100.,m.y*100.,if m.sprint {"ON"}else{"OFF"}));}
  None=>{ui.label(if !online {"Receiver offline · stick output unavailable"} else if !fresh {"Chest sensor offline / stale · stick output unavailable"} else {"Receiver output unavailable"});}
 }
 ui.small(format!("Lean input deadzone (draft): F {:.1}° / B {:.1}° / L {:.1}° / R {:.1}° · ring range {:.1}°",values[0].to_degrees(),values[1].to_degrees(),values[2].to_degrees(),values[3].to_degrees(),view.to_degrees()));
 ui.small("Bars: signed stick output; 100% is full stick. Thin guides: lean angles on the ring scale (orange deadzone, green full, purple sprint). Sprint is a separate button; game speed depends on the game.");
 if details {ui.horizontal_wrapped(|ui|{for (i,label) in ["F","B","L","R"].into_iter().enumerate(){ui.label(label);let mut angle=values[i].to_degrees();let limit=if i<2{full_y}else{full_x};if ui.add(egui::DragValue::new(&mut angle).range(0.0..=(limit-0.003).max(0.).to_degrees()).speed(0.1).suffix("°")).changed(){set_deadzone(d,&mut values,i,angle.to_radians());changed=true;}}});}
 ui.small("Orange: deadzone (0 speed) · Green: full speed · Purple: sprint. Drag the coloured handles. Shrink FORWARD to bring all rings inward; Apply to save.");
 if details {ui.label(format!("Full speed {:.1}° · Sprint {:.1}°",full.to_degrees(),sprint.to_degrees()));ui.small("Speed rises from zero at the deadzone edge to full speed at the green ring. Sprint cannot be inside full speed. Apply to save.");}
 changed
}

fn check(ui:&mut egui::Ui,d:&mut Value,key:&str,label:&str)->bool { let mut b=d[key]==true; let c=ui.checkbox(&mut b,label).changed(); if c {d[key]=json!(b);} c }
impl ChestTuning {
 pub fn show(&mut self,ui:&mut egui::Ui,s:&Value,online:bool,busy:bool,error:&str)->Option<Action> {
  let saved=&s["actions"]["settings"];
  if self.pending.as_ref().is_some_and(|p|p==saved) { self.pending=None; self.dirty=false; }
  if !self.dirty && self.pending.is_none() && saved.is_object(){self.draft=saved.clone();}
  let fresh=online && s["roles"]["torso"]["fresh"]==true;
  let preview=&s["actions"]["preview"]; let xy=s["game_output_preview"]["travel_xy"].as_array();
  let x=xy.and_then(|a|a.first()).map(number).unwrap_or(0.) as f32; let y=xy.and_then(|a|a.get(1)).map(number).unwrap_or(0.) as f32;
  if self.last_sample.is_none_or(|t|t.elapsed()>=Duration::from_millis(100)) {self.last_sample=Some(Instant::now()); self.history.push(if fresh {y} else {0.}); if self.history.len()>100 {self.history.remove(0);} }
  let mut action=None;
  ui.horizontal(|ui|{ui.heading("Chest tracker movement");ui.colored_label(if fresh {CYAN}else{MUTED},if fresh {"LIVE SENSOR"}else{"SENSOR OFFLINE / STALE"});});
  ui.label(RichText::new("Tune lean-to-move with one chest tracker. Preview is independent of game output.").color(MUTED)); ui.add_space(12.);
  if s["mounting_calibration"]["valid"]==false {ui.colored_label(ORANGE,"This chest tracker needs Sensor Calibration. Reset Center alone does not learn forward/backward.");}
  let width=ui.available_width();
  ui.horizontal_top(|ui|{
   ui.allocate_ui_with_layout(egui::vec2(width*0.32, 0.),egui::Layout::top_down(egui::Align::Min),|ui|{pages::card(ui,"Movement settings",|ui|{
    ui.spacing_mut().item_spacing.y=2.; ui.spacing_mut().interact_size.y=22.;
    let id=s["roles"]["torso"]["id"].as_str().unwrap_or("");
    ui.label("Chest tracker source"); egui::ComboBox::from_id_salt("chest_source").selected_text(if id.is_empty(){"Select tracker"}else{id}).show_ui(ui,|ui|{
      if ui.selectable_label(id.is_empty(),"Unassigned").clicked(){action=Some(Action::Command(json!({"command":"assign_sensor_role","role":"torso","id":""})));}
      if let Some(ts)=s["trackers"].as_array(){for t in ts {if let Some(candidate)=t["id"].as_str(){if ui.selectable_label(candidate==id,candidate).clicked(){action=Some(Action::Command(json!({"command":"assign_sensor_role","role":"torso","id":candidate})));}}}}
    });
    ui.small(if fresh {"Fresh tracker data"}else{"Waiting for fresh tracker data"});
    ui.collapsing("Sensor data",|ui|{if let Some(t)=s["trackers"].as_array().and_then(|ts|ts.iter().find(|t|t["id"].as_str()==Some(id))) {for (key,label) in [("acceleration","Acceleration m/s²"),("q","Quaternion")] {if let Some(a)=t[key].as_array(){ui.label(format!("{}: {}",label,a.iter().map(|v|format!("{:.3}",number(v))).collect::<Vec<_>>().join(", ")));}else{ui.label(format!("{label}: unavailable"));}}}else{ui.label("No tracker data available.");}});ui.separator();
    if self.draft.is_object(){let mut scroll=egui::ScrollArea::vertical().id_salt("chest_tuning_controls").max_height(230.).min_scrolled_height(230.);if super::qa_capture::path().is_some() && super::qa_capture::page().as_deref()==Some("chest-advanced"){scroll=scroll.vertical_scroll_offset(330.);}scroll.show(ui,|ui|{let d=&mut self.draft["direction"];
     self.dirty|=tilt_range(ui,d);
     self.dirty|=reference_controls(ui,d,s["hmd_pose"]["fresh"]==true);
     self.dirty|=slider(ui,d,"exponent","Response exponent",0.1..=5.,1.);
     self.dirty|=slider(ui,d,"max_output","Maximum output",0.05..=1.,1.);
     let backward_limit=if number(&d["deadzone"])>0. {(number(&d["max_angle"]).min(d["forward_max_angle"].as_f64().unwrap_or(number(&d["max_angle"])))-0.001)/number(&d["deadzone"])}else{3.};
     self.dirty|=slider(ui,d,"backward_deadzone_multiplier","Backward deadzone factor",1.0..=backward_limit.clamp(1.,3.),1.);
     let mut inverted=number(&d["pitch_sign"])<0.; if ui.checkbox(&mut inverted,"Invert forward / backward").changed(){d["pitch_sign"]=json!(if inverted {-1.}else{1.});self.dirty=true;}
     let mut inverted=number(&d["roll_sign"])<0.; if ui.checkbox(&mut inverted,"Invert left / right").changed(){d["roll_sign"]=json!(if inverted {-1.}else{1.});self.dirty=true;}
     self.dirty|=check(ui,d,"strafe_enabled","Enable leaning to strafe");self.dirty|=check(ui,d,"sprint_full_forward","Sprint past sprint ring");
     if ui.button("Linear 25° baseline").clicked(){baseline(d);self.dirty=true;}
     ui.small("Draft: 5% deadzone, linear response, 25° maximum. Apply to save; mounting stays unchanged.");
     });
     let validation=arcane_bridge_core::actions::Settings::from_json(&self.draft).err();
     if let Some(err)=validation {ui.colored_label(ORANGE,format!("Invalid settings: {err}. Deadzone × backward factor must be below maximum angle."));}
     if ui.add_enabled(online&&!busy&&self.dirty&&self.pending.is_none()&&validation.is_none(),egui::Button::new(RichText::new("APPLY & STOP").strong().color(Color32::BLACK)).fill(ORANGE)).clicked(){self.pending=Some(self.draft.clone());action=Some(Action::Command(json!({"command":"actions_settings","settings":self.draft})));}
     if ui.button("Discard draft").clicked(){self.draft=saved.clone();self.dirty=false;self.pending=None;}
     ui.small(if self.pending.is_some(){"Saving · waiting for receiver acknowledgement"}else if self.dirty{"Unsaved draft"}else{"Settings saved"});
     if !error.is_empty() && self.pending.is_some(){ui.colored_label(ORANGE,format!("Receiver: {error} · discard or retry after resolving")); if ui.button("Retry save").clicked(){action=Some(Action::Command(json!({"command":"actions_settings","settings":self.draft})));}}
    }else{ui.label("Waiting for receiver settings.");}
   });});
   ui.allocate_ui_with_layout(egui::vec2(width*0.49, 0.),egui::Layout::top_down(egui::Align::Min),|ui|{pages::card(ui,"Live lean preview",|ui|{
    ui.label(format!("Calibration: {}",preview["direction_status"].as_str().unwrap_or("waiting")));
    ui.horizontal_wrapped(|ui|{if ui.add_enabled(fresh&&!busy,egui::Button::new("Reset center · instant")).clicked(){action=Some(Action::ResetCenter);}if ui.add_enabled(fresh&&!busy,egui::Button::new("Calibrate sensor")).clicked(){action=Some(Action::SensorCalibration);}if ui.button("Open test room").clicked(){action=Some(Action::TestRoom);}});
    if self.draft.is_object(){self.dirty|=target(ui,&mut self.draft["direction"],&saved["direction"],preview,s,online,fresh,true);}
    if fresh {if let Some(yaw)=preview["chest_yaw"].as_f64().filter(|v|v.is_finite()){ui.label(format!("Chest rotation around vertical axis: {:.1}°",yaw.to_degrees()));}else{ui.label("Chest rotation unavailable");}}else{ui.label("Chest rotation unavailable");}
    ui.small(if fresh {format!("Receiver movement preview  X {x:.2}   Y {y:.2}")}else{"Preview unavailable until chest data is fresh".to_owned()});
    ui.label("Forward / backward output history · −1 to +1 stick"); let (r,_)=ui.allocate_exact_size(egui::vec2(ui.available_width(),65.),egui::Sense::hover()); let p=ui.painter_at(r);p.rect_filled(r,5.,Color32::from_rgb(12,25,37));for (i,w) in self.history.windows(2).enumerate(){let at=|j:usize,v:f32|egui::pos2(r.left()+r.width()*j as f32/99.,r.center().y-v.clamp(-1.,1.)*r.height()*0.45);p.line_segment([at(i,w[0]),at(i+1,w[1])],Stroke::new(1.5,CYAN));}
    ui.label(format!("Response curve · 0° to {:.1}° · 0–100% stick",saved["direction"]["forward_max_angle"].as_f64().unwrap_or(number(&saved["direction"]["max_angle"])).to_degrees()));let (r,_)=ui.allocate_exact_size(egui::vec2(ui.available_width(),85.),egui::Sense::hover());let p=ui.painter_at(r);p.rect_filled(r,5.,Color32::from_rgb(12,25,37));let d=&saved["direction"];let dead=deadzones(d)[0];let max=d["forward_max_angle"].as_f64().unwrap_or(number(&d["max_angle"])).max(dead+0.001);let exponent=number(&d["exponent"]).max(0.1);let out=number(&d["max_output"]).clamp(0.,1.);for i in 0..99 {let point=|j:usize|{let t=j as f64/99.;let v=((t*max-number(&d["forward_offset"])-dead)/(max-dead)).clamp(0.,1.).powf(exponent)*out;egui::pos2(r.left()+t as f32*r.width(),r.bottom()-v as f32*r.height())};p.line_segment([point(i),point(i+1)],Stroke::new(2.,ORANGE));}
   });});
   ui.allocate_ui_with_layout(egui::vec2((width*0.19-16.).max(100.), 0.),egui::Layout::top_down(egui::Align::Min),|ui|{pages::card(ui,"Speed",|ui|{
    let cap=1.0_f32;let magnitude=(x*x+y*y).sqrt();ui.label(format!("{:.0}% output",if fresh {magnitude*100.}else{0.}));ui.small("Receiver stick magnitude");let (r,_)=ui.allocate_exact_size(egui::vec2(ui.available_width(),160.),egui::Sense::hover());let p=ui.painter_at(r);p.rect_filled(r,5.,Color32::from_rgb(12,25,37));let track=egui::Rect::from_center_size(r.center(),egui::vec2(32.,r.height()-24.));p.rect_filled(track,6.,Color32::from_rgb(42,56,70));if fresh {let bar=egui::Rect::from_min_max(egui::pos2(track.left(),track.bottom()-track.height()*(magnitude/cap).clamp(0.,1.)),track.right_bottom());p.rect_filled(bar,6.,CYAN);}ui.small(if fresh {"Movement preview"}else{"Waiting for sensor"});
    ui.separator();ui.label("Input-angle guides · draft");let dir=&self.draft["direction"];let dz=deadzones(dir);let maxx=dir["horizontal_max_angle"].as_f64().unwrap_or(number(&dir["max_angle"])).max(0.01);let maxy=dir["forward_max_angle"].as_f64().unwrap_or(number(&dir["max_angle"])).max(0.01);let angles=adjusted_angles(preview,&saved["direction"],fresh).map(|[x,y]|[y,x]);for (i,name) in ["Forward","Back","Left","Right"].into_iter().enumerate(){let limit=if i<2{maxy}else{maxx};ui.label(format!("{name}  {:.1}° / {:.1}°",dz[i].to_degrees(),limit.to_degrees()));let (r,_)=ui.allocate_exact_size(egui::vec2(ui.available_width(),8.),egui::Sense::hover());let p=ui.painter_at(r);p.line_segment([r.left_center(),r.right_center()],Stroke::new(3.,Color32::from_rgb(55,75,90)));let threshold=r.left()+r.width()*(dz[i]/limit).clamp(0.,1.) as f32;p.line_segment([egui::pos2(threshold,r.top()),egui::pos2(threshold,r.bottom())],Stroke::new(2.,ORANGE));if let Some(angles)=angles {let v=match i{0=>angles[0].max(0.),1=>(-angles[0]).max(0.),2=>(-angles[1]).max(0.),_=>angles[1].max(0.)};let x=r.left()+r.width()*(v/limit).clamp(0.,1.) as f32;p.circle_filled(egui::pos2(x,r.center().y),3.,CYAN);}}ui.small("Orange: draft deadzone · cyan: saved-offset-adjusted live angle. Meter above: stick output.");
   });});
  }); action
 }
}


#[cfg(test)] mod tests {
 use super::*;
 #[test] fn output_meter_uses_final_receiver_values_and_requires_fresh_data() {
  let state=json!({"actions":{"preview":{"travel_xy":[0.9,0.9]}},"game_output_preview":{"travel_xy":[-0.25,0.75],"buttons":4,"armed":false}});
  assert_eq!(output_meter(&state,true,true),Some(OutputMeter{x:-0.25,y:0.75,sprint:true,armed:false}));
  assert_eq!(output_meter(&state,false,true),None);assert_eq!(output_meter(&state,true,false),None);
  assert_eq!(output_meter(&json!({"game_output_preview":{"travel_xy":[0,0]}}),true,true),None);
 }
 #[test] fn outer_forward_drag_fits_thresholds_inside_boundary() {
  let ctx=egui::Context::default();let mut d=arcane_bridge_core::actions::Settings::default().to_json()["direction"].clone();let original=d.clone();
  let mut frame=|events:Vec<egui::Event>| {
   let mut zoom=0.;let output=ctx.run(egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(600.,800.))),events,..Default::default()},|ctx|{egui::CentralPanel::default().show(ctx,|ui|{target(ui,&mut d,&original,&json!({}),&json!({}),false,false,false);zoom=ui.data(|data|data.get_temp::<f64>(ui.make_persistent_id("lean_ring_zoom"))).unwrap();});});
   let top=output.shapes.iter().find_map(|s|if let egui::Shape::Circle(c)=&s.shape {if c.radius==7. && c.fill==CYAN {Some(c.center)}else{None}}else{None}).unwrap();(top,zoom)
  };
  let (top,before)=frame(vec![]);
  frame(vec![egui::Event::PointerMoved(top),egui::Event::PointerButton{pos:top,button:egui::PointerButton::Primary,pressed:true,modifiers:Default::default()}]);
  let (dragged,after)=frame(vec![egui::Event::PointerMoved(top+egui::vec2(0.,45.))]);
  assert!((dragged.y-top.y-45.).abs()<0.1,"outer ring must follow pointer");
  assert!(after<before,"drag must reduce range: {before} -> {after}");
  let (released,final_zoom)=frame(vec![egui::Event::PointerButton{pos:top+egui::vec2(0.,45.),button:egui::PointerButton::Primary,pressed:false,modifiers:Default::default()}]);
  assert!((released.y-top.y).abs()<0.1);assert!((final_zoom-after).abs()<1e-6);
  assert!(number(&d["sprint_angle"])<=final_zoom*0.900001);
  assert!(number(&d["max_angle"])<number(&original["max_angle"]));
  assert_eq!(d["learned_forward"],original["learned_forward"]);
 }
 #[test] fn shrinking_boundary_keeps_all_rings_inside_and_valid() {
 let mut settings=arcane_bridge_core::actions::Settings::default().to_json();
 assert!(fit_rings(&mut settings["direction"],0.2));
 let d=&settings["direction"];assert!(number(&d["sprint_angle"])<=0.180001);assert!(number(&d["max_angle"])<=number(&d["sprint_angle"]));
 assert!(arcane_bridge_core::actions::Settings::from_json(&settings).is_ok());
 }
 #[test] fn sides_are_linked_front_and_back_are_independent() {let mut d=json!({});let mut values=[0.1,0.2,0.3,0.4];set_deadzone(&mut d,&mut values,2,0.5);assert_eq!(values,[0.1,0.2,0.5,0.5]);assert_eq!(d["left_deadzone"],d["right_deadzone"]);set_deadzone(&mut d,&mut values,3,0.6);assert_eq!(values,[0.1,0.2,0.6,0.6]);set_deadzone(&mut d,&mut values,0,0.7);set_deadzone(&mut d,&mut values,1,0.8);assert_eq!(values,[0.7,0.8,0.6,0.6]);}
 #[test] fn live_angles_use_saved_forward_offset() {let p=json!({"raw_direction_angles":[0.2,0.4]});let d=json!({"forward_offset":0.1});let a=adjusted_angles(&p,&d,true).unwrap();assert!((a[0]-0.2).abs()<1e-9 && (a[1]-0.3).abs()<1e-9);assert!(adjusted_angles(&p,&d,false).is_none());assert!(adjusted_angles(&json!({"raw_direction_angles":null}),&d,true).is_none());}
 #[test] fn directional_deadzone_fallback_and_overrides() {let mut d=json!({"deadzone":0.1,"backward_deadzone_multiplier":2.0});assert_eq!(deadzones(&d),[0.1,0.2,0.1,0.1]);d["left_deadzone"]=json!(0.03);assert_eq!(deadzones(&d),[0.1,0.2,0.03,0.1]);}
 #[test] fn baseline_preserves_mount_and_requires_valid_deadzone() {
  let mut settings=arcane_bridge_core::actions::Settings::default().to_json();
  settings["direction"]["learned_forward"]=json!([1.,0.,0.]);
  settings["direction"]["pitch_axis"]=json!("-pitch");
  baseline(&mut settings["direction"]);
  assert_eq!(settings["direction"]["learned_forward"],json!([1.,0.,0.]));
  assert_eq!(settings["direction"]["pitch_axis"],"-pitch");
  assert!(arcane_bridge_core::actions::Settings::from_json(&settings).is_ok());
  settings["direction"]["deadzone"]=json!(0.3);settings["direction"]["backward_deadzone_multiplier"]=json!(2.0);
  assert!(arcane_bridge_core::actions::Settings::from_json(&settings).is_err());
 }
}
