use super::pages::{self, Page};
use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
use serde_json::Value;

const PANEL: Color32 = Color32::from_rgb(19, 30, 44);
const BORDER: Color32 = Color32::from_rgb(43, 60, 77);
const TEXT: Color32 = Color32::from_rgb(235, 241, 247);
const SUB: Color32 = Color32::from_rgb(161, 179, 194);

pub enum Action { Navigate(Page), Command(Value), Edit, Done, ResetCenter, SensorCalibration, SetMovement(bool), ForceSteamHome, BoundarySetup }

fn sensor_status(state: &Value, role: &str, online: bool) -> &'static str {
    if !online { return "Receiver offline"; }
    let sensor = &state["roles"][role];
    if sensor["id"].as_str().unwrap_or("").is_empty() { "Not assigned" }
    else if sensor["fresh"] == true { "Connected" }
    else { "Offline" }
}

fn chest_calibrated(state:&Value, online:bool) -> bool {
    online && sensor_status(state,"torso",online)=="Connected"
        && state["calibrated"]==true
        && state["mounting_calibration"]["valid"]==true
        && state["mounting_calibration"]["mapping"]["learned_forward"].is_array()
        && state["mounting_calibration"]["mapping"]["learned_right"].is_array()
}

fn surface(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new().fill(PANEL).stroke(Stroke::new(1., BORDER))
        .corner_radius(14).inner_margin(20).show(ui, |ui| {
            ui.set_min_width(ui.available_width()); contents(ui);
        });
}

fn section(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.).strong().color(pages::CYAN));
    ui.add_space(8.);
}

fn action_button(ui: &mut egui::Ui, text: &str, primary: bool) -> bool {
    ui.add(egui::Button::new(RichText::new(text).size(14.).strong().color(if primary { Color32::from_rgb(20, 22, 25) } else { TEXT }))
        .fill(if primary {pages::ORANGE} else {Color32::from_rgb(34, 49, 65)})
        .stroke(Stroke::NONE).corner_radius(8).min_size(Vec2::new(118., 36.))).clicked()
}

fn dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(12., 12.), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4., color);
}

fn readiness(state: &Value, online: bool, sensors_connected: bool) -> &'static str {
    if !online { return "Connect the receiver"; }
    if state["mode"] == "demo" { return "Select live sensors"; }
    if !sensors_connected { return "Add and connect your sensors"; }
    if state["output_mode"] != "steamvr" { return "Select SteamVR in Settings"; }
    if state["steamvr"]["driver_connected"] != true { return "Open SteamVR and connect the ArcaneVR driver"; }
    if state["steamvr"]["hmd_ready"] != true { return "Connect your Quest headset"; }
    if state["calibrated"] != true || state["steamvr"]["driver_calibrated"] != true { return "Stand upright and press Recenter"; }
    if (state["movement_engine"] == "body_tilt" || state["movement_engine"] == "body_tilt_steps") &&
        (state["actions"]["preview"]["calibrated"] != true || state["actions"]["preview"]["direction_status"] != "ready") { return "Calibrate sensor movements"; }
    if state["locomotion"]["input_ready"] != true { return "Check sensor movement data"; }
    "Ready"
}

pub fn show(ui: &mut egui::Ui, state: &Value, online: bool, _busy: bool, editing: bool, can_enable: bool) -> Option<Action> {
    let mut action = None;
    let count = state["sensor_layout"]["count"].as_u64().unwrap_or(1);
    let selected = state["game_profiles"]["selected"].as_str().unwrap_or("");
    if editing || (crate::qa_capture::path().is_some() && std::env::var_os("ARCANEVR_QA_SENSOR_LIST").is_some()) {
        ui.heading("Edit sensors");
        if action_button(ui, "Done", false) { return Some(Action::Done); }
        ui.add_space(12.);
        ui.heading("Saved tracker calibrations");
        ui.label("Mounting and direction settings are restored for the same tracker when it reconnects. Reset center while upright each session.");
        match state["device_calibrations"].as_object() {
            Some(records) if !records.is_empty() => {
                let mut entries: Vec<_> = records.iter().collect();
                entries.sort_by_key(|(identity, _)| *identity);
                for (identity, record) in entries {
                    let current = state["mounting_calibration"]["tracker_identity"].as_str() == Some(identity.as_str());
                    surface(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(identity).strong().color(TEXT));
                            if current { ui.colored_label(pages::MINT, "Current chest tracker"); }
                        });
                        let trained = record["learned_forward"].is_array() && record["learned_right"].is_array();
                        ui.label(if trained { "Forward and sideways mounting saved" } else { "Mounting directions incomplete — sensor calibration needed" });
                        ui.small(format!("Forward flip: {}  ·  Sideways flip: {}", if record["pitch_sign"].as_f64().unwrap_or(1.) < 0. {"on"} else {"off"}, if record["roll_sign"].as_f64().unwrap_or(1.) < 0. {"on"} else {"off"}));
                    });
                    ui.add_space(6.);
                }
            }
            _ => { ui.label("No saved tracker calibrations yet."); }
        }
        ui.add_space(12.);
        return None;
    }
    let roles: &[(&str,f32,f32)] = if count==1 {&[("torso",0.,0.265)]} else if count==5 {&[("torso",0.,0.265),("left",0.22,0.855),("right",-0.22,0.855),("left_thigh",0.18,0.58),("right_thigh",-0.18,0.58)]} else {&[("torso",0.,0.265),("left",0.22,0.855),("right",-0.22,0.855)]};
    let connected = |role: &str| if role.ends_with("thigh") {
        let id=state["sensor_layout"][role].as_str().unwrap_or("");
        online && !id.is_empty() && state["trackers"].as_array().is_some_and(|trackers|trackers.iter().any(|t|t["id"].as_str()==Some(id)&&(t["fresh"]==true||t["sample_mode"]=="held")))
    } else {sensor_status(state,role,online)=="Connected"};
    let all=roles.iter().all(|(role,_,_)|connected(role));
    let status=readiness(state,online,all);
    let is_ready=status=="Ready";
    // Positions are in the designer's 1024 x 765 reference, cropped to its window.
    let scale=ui.available_width()/964.;
    let (canvas,_)=ui.allocate_exact_size(egui::vec2(964.,705.)*scale,egui::Sense::hover());
    let pos=|x:f32,y:f32| canvas.min+egui::vec2(x-30.,y-30.)*scale;
    let box_at=|x:f32,y:f32,w:f32,h:f32| egui::Rect::from_min_size(pos(x,y),egui::vec2(w,h)*scale);
    let uv=|x:f32,y:f32,w:f32,h:f32| egui::Rect::from_min_max(egui::pos2(x/1024.,y/765.),egui::pos2((x+w)/1024.,(y+h)/765.));
    let key=egui::Id::new("dashboard skin");
    let skin=ui.ctx().data_mut(|d|d.get_temp::<egui::TextureHandle>(key)).unwrap_or_else(||{
        let texture=ui.ctx().load_texture("dashboard skin",egui::ColorImage::from_rgba_unmultiplied([1451,1084],include_bytes!("../../assets/home-skin.rgba")),egui::TextureOptions::LINEAR);
        ui.ctx().data_mut(|d|d.insert_temp(key,texture.clone()));texture
    });
    let paint=ui.painter().clone();
    paint.image(skin.id(),canvas,uv(30.,30.,964.,705.),Color32::WHITE);
    let hit=|name:&str,rect:egui::Rect,enabled:bool|{
        let response=ui.interact(rect,ui.id().with(name),if enabled {egui::Sense::click()} else {egui::Sense::hover()});
        if enabled && (response.hovered() || response.has_focus()) {
            paint.rect_stroke(rect,7.,Stroke::new(1.,Color32::from_white_alpha(140)),egui::StrokeKind::Inside);
        }
        response
    };
    let text_at=|x,y,label:&str,size,color| {paint.text(pos(x,y),egui::Align2::LEFT_CENTER,label,egui::FontId::proportional(size*scale),color);};
    // Extend the blank strip above a changing label to preserve the field gradient.
    let field=|y,label:&str| {
        paint.image(skin.id(),box_at(154.,y-15.,286.,30.),uv(154.,y-20.,286.,2.),Color32::WHITE);
        text_at(160.,y,label,19.,TEXT);
    };
    field(253.,match count {1=>"1 sensor · Chest",5=>"5 sensors · Chest, thighs & feet",_=>"3 sensors · Chest & feet"});
    field(380.,match selected {"arizona_sunshine_legacy"=>"Arizona Sunshine VR Legacy","custom"=>"Other game",_=>"Choose your game"});
    let mode=hit("Mode",box_at(147.,227.,333.,52.),true);
    egui::Popup::menu(&mode).show(|ui| {
        for (n,label) in [(1,"1 sensor · Chest"),(3,"3 sensors · Chest & feet"),(5,"5 sensors · Chest, thighs & feet")] {
            if ui.selectable_label(count==n,label).clicked(){action=Some(Action::Command(serde_json::json!({"command":"sensor_layout","count":n})));ui.close();}
        }
    });
    let game=hit("Game",box_at(147.,353.,333.,52.),true);
    egui::Popup::menu(&game).show(|ui| {
        for (id,label) in [("arizona_sunshine_legacy","Arizona Sunshine VR Legacy"),("custom","Other game")] {
            if ui.selectable_label(selected==id,label).clicked(){
                action=Some(if state["game_profiles"]["associations"][id]["settings"].is_object(){Action::Command(serde_json::json!({"command":"game_profile_restore","game_id":id}))}else{Action::Navigate(Page::GameProfiles)});ui.close();
            }
        }
    });
    let time=ui.input(|i|i.time) as f32;
    let pulse=(time*std::f32::consts::TAU/3.).cos()*0.5+0.5;
    let badge=box_at(261.,77.,160.,40.);
    if is_ready {
        paint.rect_filled(badge,20.,Color32::from_rgb(18,70,49));paint.rect_stroke(badge,20.,Stroke::new(2.,pages::MINT),egui::StrokeKind::Inside);
        paint.circle_filled(pos(285.,97.),6.*scale,pages::MINT);text_at(305.,97.,"READY",20.,pages::MINT);
    } else {paint.rect_filled(badge,20.,Color32::from_black_alpha((48.*pulse) as u8));}
    let banner=box_at(60.,642.,442.,65.);
    if status!="Add and connect your sensors" {
        let col=if is_ready {pages::MINT}else{pages::ORANGE};
        paint.rect_filled(banner,12.,if is_ready {Color32::from_rgb(18,49,37)} else {Color32::from_rgb(51,39,26)});
        paint.rect_stroke(banner,12.,Stroke::new(2.,col),egui::StrokeKind::Inside);
        text_at(78.,675.,status,if status.len()>37 {16.}else{19.},col);
    }
    if !is_ready {paint.rect_filled(banner,12.,Color32::from_black_alpha((35.*pulse) as u8));}
    let calibrate=online && all && !_busy;
    if hit("Recenter",box_at(434.,76.,111.,42.),calibrate && state["calibration_ready"]==true).clicked(){action=Some(Action::ResetCenter);}
    let tilt=matches!(state["movement_engine"].as_str(),Some("body_tilt"|"body_tilt_steps"));
    if hit("Calibrate",box_at(555.,76.,109.,42.),calibrate && tilt).clicked(){action=Some(Action::SensorCalibration);}
    let armed=state["armed"]==true;
    if armed {paint.circle_filled(pos(159.,451.),7.*scale,pages::MINT);}
    if hit("Game movement",box_at(145.,434.,215.,36.),armed||can_enable).clicked(){action=Some(Action::SetMovement(!armed));}
    let boundary=box_at(365.,431.,125.,36.);
    paint.rect_filled(boundary,8.,Color32::from_rgb(22,44,57));paint.rect_stroke(boundary,8.,Stroke::new(1.,Color32::from_rgb(56,143,164)),egui::StrokeKind::Inside);
    paint.text(boundary.center(),egui::Align2::CENTER_CENTER,"Boundary setup",egui::FontId::proportional(14.*scale),pages::MINT);
    if hit("Boundary setup",boundary,true).on_hover_text("Open SteamVR Room Setup. Quest boundaries may need to be adjusted in the headset.").clicked(){action=Some(Action::BoundarySetup);}
    let force=box_at(145.,471.,345.,31.);
    paint.rect_filled(force,8.,Color32::from_rgb(15,37,51));paint.rect_stroke(force,8.,Stroke::new(1.,Color32::from_rgb(56,143,164)),egui::StrokeKind::Inside);
    paint.text(force.center(),egui::Align2::CENTER_CENTER,"↩  Return to SteamVR Home",egui::FontId::proportional(15.*scale),pages::MINT);
    if hit("Force Steam Home",force,true).on_hover_text("Exit the current VR game and return to SteamVR Home; unsaved progress may be lost.").clicked(){action=Some(Action::ForceSteamHome);}
    if let Some(message)=ui.ctx().data(|d|d.get_temp::<String>(egui::Id::new("force_home_result"))) {paint.text(pos(60.,714.),egui::Align2::LEFT_BOTTOM,message,egui::FontId::proportional(12.*scale),pages::MINT);}
    // Replace the baked-in action buttons with a live connection strip.
    paint.rect_filled(box_at(66.,510.,426.,98.),10.,Color32::from_rgb(13,29,40));
    let devices=[("Sensors",Some(online && all)),("SteamVR",Some(online && state["steamvr"]["driver_connected"]==true)),("Quest 3",Some(online && state["hmd_pose"]["fresh"]==true)),("L",if online {state["controllers"]["left"].as_bool()} else {None}),("R",if online {state["controllers"]["right"].as_bool()} else {None})];
    for (index,(label,connected)) in devices.into_iter().enumerate() {
        let x=80.+index as f32*81.;
        let rect=box_at(x,522.,73.,72.);
        let color=match connected {Some(true)=>pages::MINT,Some(false)=>SUB,None=>Color32::from_rgb(132,149,170)};
        paint.rect_filled(rect,8.,Color32::from_rgb(22,40,53));
        let center=pos(x+36.5,544.);
        let stroke=Stroke::new(1.8*scale,color);
        match index {
            0=>{paint.circle_stroke(center,10.*scale,stroke);paint.circle_filled(center,3.*scale,color);}
            1=>{paint.circle_stroke(center+egui::vec2(6.,-4.)*scale,7.*scale,stroke);paint.circle_stroke(center+egui::vec2(-9.,6.)*scale,4.*scale,stroke);paint.line_segment([center+egui::vec2(-6.,5.)*scale,center+egui::vec2(3.,-1.)*scale],stroke);}
            2=>{
                let r=egui::Rect::from_center_size(center,egui::vec2(37.,22.)*scale);
                paint.rect_filled(r.expand(2.*scale),8.,Color32::from_rgb(35,55,67));
                paint.rect_filled(r,7.,Color32::from_rgb(224,233,238));
                for dx in [-12.,0.,12.] {let pill=egui::Rect::from_center_size(center+egui::vec2(dx,0.)*scale,egui::vec2(5.,14.)*scale);paint.rect_filled(pill,3.,Color32::from_rgb(28,40,49));paint.circle_filled(pill.center()-egui::vec2(0.,3.)*scale,1.4*scale,color);}
                paint.line_segment([r.left_center()-egui::vec2(5.,0.)*scale,r.left_center()],Stroke::new(4.*scale,color));
                paint.line_segment([r.right_center(),r.right_center()+egui::vec2(5.,0.)*scale],Stroke::new(4.*scale,color));
            }
            _=>{
                let grip=egui::Rect::from_center_size(center+egui::vec2(0.,7.)*scale,egui::vec2(13.,24.)*scale);
                paint.rect_filled(grip,6.,Color32::from_rgb(210,222,230));
                paint.circle_filled(center-egui::vec2(0.,5.)*scale,11.*scale,Color32::from_rgb(231,239,243));
                paint.circle_filled(center-egui::vec2(0.,5.)*scale,8.5*scale,Color32::from_rgb(43,58,70));
                let sign=if index==3 {-1.}else{1.};
                paint.circle_filled(center+egui::vec2(-sign*3.,-7.)*scale,3.*scale,color);
                for dy in [-6.,-1.] {paint.circle_filled(center+egui::vec2(sign*4.,dy)*scale,1.7*scale,Color32::WHITE);}
                paint.line_segment([center+egui::vec2(-3.,12.)*scale,center+egui::vec2(3.,12.)*scale],Stroke::new(1.*scale,Color32::from_rgb(125,151,165)));
            }
        }
        paint.text(pos(x+36.5,569.),egui::Align2::CENTER_CENTER,label,egui::FontId::proportional(12.*scale),TEXT);
        paint.circle_filled(pos(x+36.5,585.),2.5*scale,color);
        let response=hit(label,rect,index==0);
        response.clone().on_hover_text(match connected {Some(true)=>"Connected",Some(false)=>"Not connected",None=>"Controller status unavailable: the receiver does not report individual controllers"});
        if index==0 && response.clicked(){action=Some(Action::Navigate(Page::QuickSetup));}
    }
    let settings=box_at(675.,76.,42.,42.);
    paint.rect_filled(settings,8.,Color32::from_rgb(24,43,57));
    let center=settings.center();let color=pages::CYAN;
    paint.circle_stroke(center,9.*scale,Stroke::new(2.*scale,color));
    paint.circle_stroke(center,3.5*scale,Stroke::new(2.*scale,color));
    for i in 0..8 {let angle=i as f32*std::f32::consts::TAU/8.;let v=egui::vec2(angle.cos(),angle.sin());paint.line_segment([center+v*10.*scale,center+v*14.*scale],Stroke::new(3.*scale,color));}
    if hit("Settings",settings,true).on_hover_text("Settings").clicked(){action=Some(Action::Navigate(Page::Settings));}
    if hit("Minimize",box_at(906.,47.,31.,31.),true).clicked(){ui.ctx().send_viewport_cmd(egui::ViewportCommand::Minimized(true));}
    if hit("Close",box_at(947.,47.,31.,31.),true).clicked(){ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);}
    if ui.interact(box_at(33.,32.,865.,39.),ui.id().with("drag home"),egui::Sense::drag()).drag_started(){ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);}
    for (role,_,_) in roles {
        let (x,y)=match *role {"torso"=>(735.,248.),"right_thigh"=>(688.,456.),"left_thigh"=>(790.,456.),"right"=>(677.,633.),_=>(811.,633.)};
        let center=pos(x,y);
        let live=connected(role);
        let calibrated=*role=="torso" && chest_calibrated(state,online);
        let color=if calibrated {Color32::from_rgb(69,255,146)}
            else if live && *role!="torso" {pages::MINT}
            else {Color32::from_rgb(62,178,244)};
        let radius=if calibrated {19.+3.*(3.*time).sin()} else if live {17.+3.*(3.*time).sin()} else {17.};
        if calibrated {
            for (extra,alpha) in [(14.,18),(9.,30),(5.,50)] {
                paint.circle_filled(center,(radius+extra)*scale,Color32::from_rgba_unmultiplied(color.r(),color.g(),color.b(),alpha));
            }
        } else {
            paint.circle_filled(center,(radius+4.)*scale,Color32::from_rgba_unmultiplied(color.r(),color.g(),color.b(),35));
        }
        paint.circle_stroke(center,radius*scale,Stroke::new(if calibrated {3.*scale} else {2.2*scale},color));
        paint.circle_filled(center,if calibrated {6.*scale} else {4.2*scale},color);
    }
    action
}

#[cfg(test)] mod tests {
    use super::*;
    #[test]
    fn skin_targets_dispatch_real_dashboard_actions() {
        let ctx=egui::Context::default();
        let state=serde_json::json!({"sensor_layout":{"count":1},"roles":{"torso":{"id":"chest","fresh":true}},"movement_engine":"body_tilt","calibration_ready":true});
        let mut frame=|events|{
            let mut result=None;
            let input=egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(964.,705.))),events,..Default::default()};
            let _=ctx.run(input,|ctx|{egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx,|ui|{result=show(ui,&state,true,false,false,true);});});
            result
        };
        frame(vec![]);
        let mut click=|x,y|{
            let pos=egui::pos2(x,y);
            for pressed in [true,false] {
                let event=egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed,modifiers:Default::default()};
                let result=frame(vec![egui::Event::PointerMoved(pos),event]);
                if !pressed {return result;}
            }
            None
        };
        assert!(matches!(click(87.,530.),Some(Action::Navigate(Page::QuickSetup))));
        assert!(matches!(click(665.,67.),Some(Action::Navigate(Page::Settings))));
        assert!(click(350.,530.).is_none());
        assert!(matches!(click(450.,67.),Some(Action::ResetCenter)));
        assert!(matches!(click(570.,67.),Some(Action::SensorCalibration)));
        assert!(matches!(click(130.,422.),Some(Action::SetMovement(true))));
        assert!(matches!(click(400.,420.),Some(Action::BoundarySetup)));
        assert!(matches!(click(270.,455.),Some(Action::ForceSteamHome)));
    }
    #[test] fn readiness_tracks_each_dependency_in_order() {
        let mut s=serde_json::json!({"mode":"live","output_mode":"steamvr","steamvr":{"driver_connected":true,"hmd_ready":true,"driver_calibrated":true},"calibrated":true,"movement_engine":"body_tilt","actions":{"preview":{"calibrated":true,"direction_status":"ready"}},"locomotion":{"input_ready":true}});
        assert_eq!(readiness(&s,false,false),"Connect the receiver");
        assert_eq!(readiness(&s,true,false),"Add and connect your sensors");
        s["steamvr"]["driver_connected"]=false.into(); assert!(readiness(&s,true,true).contains("SteamVR"));
        s["steamvr"]["driver_connected"]=true.into(); s["steamvr"]["hmd_ready"]=false.into(); assert!(readiness(&s,true,true).contains("headset"));
        s["steamvr"]["hmd_ready"]=true.into(); s["calibrated"]=false.into(); assert!(readiness(&s,true,true).contains("Recenter"));
        s["calibrated"]=true.into(); s["actions"]["preview"]["direction_status"]="missing".into(); assert!(readiness(&s,true,true).contains("Calibrate"));
        s["actions"]["preview"]["direction_status"]="ready".into(); assert_eq!(readiness(&s,true,true),"Ready");
        s["locomotion"]["input_ready"]=false.into(); assert!(readiness(&s,true,true).contains("data"));
    }
    #[test] fn sensor_labels_follow_assignment_and_freshness() {
        let mut state=serde_json::json!({"roles":{"torso":{"id":"x","fresh":true}}});
        assert_eq!(sensor_status(&state,"torso",true),"Connected");
        state["roles"]["torso"]["fresh"]=false.into(); assert_eq!(sensor_status(&state,"torso",true),"Offline");
        assert_eq!(sensor_status(&state,"torso",false),"Receiver offline");
        assert_eq!(sensor_status(&state,"left",true),"Not assigned");
    }
    #[test] fn chest_glow_requires_a_fresh_complete_calibration() {
        let mut s=serde_json::json!({
            "roles":{"torso":{"id":"chest","fresh":true}},
            "calibrated":true,
            "mounting_calibration":{"valid":true,"mapping":{"learned_forward":[1,0,0],"learned_right":[0,1,0]}}
        });
        assert!(chest_calibrated(&s,true));
        assert!(!chest_calibrated(&s,false));
        s["calibrated"]=false.into();assert!(!chest_calibrated(&s,true));
        s["calibrated"]=true.into();s["mounting_calibration"]["valid"]=false.into();assert!(!chest_calibrated(&s,true));
        s["mounting_calibration"]["valid"]=true.into();s["mounting_calibration"]["mapping"]["learned_right"]=serde_json::Value::Null;
        assert!(!chest_calibrated(&s,true));
        s["mounting_calibration"]["mapping"]["learned_right"]=serde_json::json!([0,1,0]);s["roles"]["torso"]["fresh"]=false.into();
        assert!(!chest_calibrated(&s,true));
    }
}

