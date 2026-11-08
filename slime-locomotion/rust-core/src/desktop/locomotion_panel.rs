//! Standing-test UI and a local settings draft. Commands are returned to the app.
use super::pages;
use arcane_bridge_core::continuous::Settings;
use eframe::egui::{self, Color32, RichText, Stroke};
use serde_json::{json, Value};

const MINT: Color32 = Color32::from_rgb(117, 239, 196);
const CYAN: Color32 = Color32::from_rgb(86, 199, 245);
const VIOLET: Color32 = Color32::from_rgb(181, 153, 255);
const AMBER: Color32 = Color32::from_rgb(241, 192, 118);
const MUTED: Color32 = Color32::from_rgb(142, 164, 190);

pub struct LocomotionPanel {
    settings: Settings,
    algorithm: String,
    initialized: bool,
    dirty: bool,
    pending: Option<Value>,
    history: Vec<[f32; 5]>,
    last_sample: f64,
    history_source: String,
    raw_history: Vec<[f32; 3]>,
}
impl Default for LocomotionPanel {
    fn default() -> Self {
        Self {
            settings: Settings::default(),
            algorithm: "continuous".into(),
            initialized: false,
            dirty: false,
            pending: None,
            history: vec![],
            last_sample: -1.,
            history_source: String::new(),
            raw_history: vec![],
        }
    }
}
fn number(v: &Value) -> f32 {
    v.as_f64().filter(|n| n.is_finite()).unwrap_or(0.) as f32
}
fn diagnostic_sample(v:&Value)->Option<[f32;5]> {let keys=["signal","fast","slow","adaptive","inertia"];let mut a=[0.;5];for (i,key) in keys.iter().enumerate(){a[i]=v[*key].as_f64().filter(|v|v.is_finite())? as f32;}Some(a)}
fn marker(v:&Value,fresh:bool)->Option<(f32,f32)> {if !fresh{return None;}Some((v["raw_speed"].as_f64().filter(|v|v.is_finite())?.clamp(0.,1.) as f32,v["speed"].as_f64().filter(|v|v.is_finite())?.clamp(0.,1.) as f32))}
fn chest_raw(s:&Value,online:bool)->Option<[f32;3]> {if !online{return None;}let id=s["roles"]["torso"]["id"].as_str()?;if id.is_empty(){return None;}let t=s["trackers"].as_array()?.iter().find(|t|t["id"].as_str()==Some(id))?;if t["acceleration_age_ms"].as_f64().filter(|v|*v>=0. && *v<500.).is_none(){return None;}let a=t["acceleration"].as_array()?;if a.len()!=3{return None;}Some([a[0].as_f64().filter(|v|v.is_finite())? as f32,a[1].as_f64().filter(|v|v.is_finite())? as f32,a[2].as_f64().filter(|v|v.is_finite())? as f32])}
fn curve_value(s:&Settings,x:f32)->f32 {if s.curve_mode!="custom" {return x.powf(s.exponent as f32);}let x=x as f64;for pair in s.curve_points.windows(2){if x<=pair[1][0] {let t=(x-pair[0][0])/(pair[1][0]-pair[0][0]);return (pair[0][1]+t*(pair[1][1]-pair[0][1])) as f32;}}1.}
fn set_graph_threshold(s:&mut Settings,index:usize,value:f64) {
    if !value.is_finite(){return;}
    match index {
        0=>{let limit=if s.sprint_enabled {s.max_signal.min(s.sprint_threshold)} else {s.max_signal};s.threshold=value.clamp(0.,100.).min((limit-0.01).max(0.));},
        1=>{let upper=if s.sprint_enabled {s.sprint_threshold.min(500.)} else {500.};s.max_signal=value.max(s.threshold+0.01).max(0.1).min(upper);},
        2=>s.sprint_threshold=value.clamp(0.01,1000.).max(s.max_signal).max(s.threshold+0.01),
        _=>{}
    }
}
fn milliseconds(ui: &mut egui::Ui, value: &mut f64, min: f64, label: &str) -> bool {
    let mut ms = *value * 1000.;
    let changed = ui
        .add(
            egui::Slider::new(&mut ms, (min * 1000.)..=10_000.)
                .logarithmic(true)
                .suffix(" ms")
                .text(label),
        )
        .changed();
    if changed {
        *value = ms / 1000.;
    }
    changed
}
impl LocomotionPanel {
    fn reload(&mut self, motion: &Value) {
        if let Ok(settings) = Settings::from_json(&motion["settings"]) {
            self.settings = settings;
            self.algorithm = motion["algorithm"].as_str().unwrap_or("continuous").into();
            self.initialized = true;
            self.dirty = false;
            self.pending = None;
        }
    }
    fn preset(&mut self, preset: &str) {
        self.settings = Settings::default();
        match preset {
            "Gentle" => {
                self.settings.intensity = 1.;
                self.settings.fast_window = 0.08;
                self.settings.slow_window = 1.4;
                self.settings.adaptive_per_unit = 0.025;
                self.settings.inertia_window = 0.35;
                self.settings.max_signal = 22.;
                self.settings.exponent = 1.3;
            }
            "Responsive" => {
                self.settings.fast_window = 0.03;
                self.settings.slow_window = 1.;
                self.settings.adaptive_per_unit = 0.02;
                self.settings.inertia_window = 0.12;
            }
            _ => {}
        }
        self.algorithm = "continuous".into();
        self.dirty = true;
        self.pending = None;
    }
    pub fn show_steps(
        &mut self,
        ui: &mut egui::Ui,
        state: &Value,
        online: bool,
        busy: bool,
    ) -> Option<Value> {
        let motion = &state["locomotion"];
        if !self.initialized && motion.is_object() {
            self.reload(motion);
        }
        if let Some(pending) = &self.pending {
            if pending["algorithm"] == motion["algorithm"]
                && pending["settings"] == motion["settings"]
            {
                self.reload(motion);
            }
        } else if self.initialized && !self.dirty {
            self.reload(motion);
        }
        let source=state["step_input"].as_str().unwrap_or("feet");
        let preview = if source=="feet" {&motion["preview"]} else {&motion["diagnostic"]};
        let identity=format!("{}:{}:{}:{}",source,state["roles"]["torso"]["id"].as_str().unwrap_or(""),state["roles"]["left"]["id"].as_str().unwrap_or(""),state["roles"]["right"]["id"].as_str().unwrap_or(""));
        let fresh=online && preview["status"]=="ready";
        let raw=chest_raw(state,online);
        self.reset_history_if_needed(&identity,fresh);
        if raw.is_none(){self.raw_history.clear();}
        let now = ui.input(|i| i.time);
        if now-self.last_sample>=0.07 {
            self.last_sample=now;
            if let Some(sample)=fresh.then(||diagnostic_sample(preview)).flatten() {self.history.push(sample);if self.history.len()>160{self.history.remove(0);}}
            if let Some(sample)=raw {self.raw_history.push(sample);if self.raw_history.len()>160{self.raw_history.remove(0);}} else {self.raw_history.clear();}
        }
        let mut command = None;
        pages::warning(ui, state);
        if source == "chest" {
            ui.label("Chest acceleration drives speed. Lean selects direction.");
        } else if source == "off" {
            ui.colored_label(AMBER, "Step input is off. These settings can be adjusted, but do not affect lean-only movement. Enable chest steps in Movement to use them.");
        }

        ui.add_space(8.);
        let column_count=if ui.available_width()<850. {1}else{2};
        ui.columns(column_count,|columns|{
            let ui=&mut columns[0];
            pages::card(ui,"Locomotion Setup",|ui|{
                ui.horizontal_wrapped(|ui|{ui.selectable_value(&mut self.algorithm,"continuous".into(),"Continuous motion").changed().then(||self.dirty=true);if source == "feet" { self.dirty |= ui.selectable_value(&mut self.algorithm,"alternating".into(),"Alternating steps").changed(); }});
                ui.add_space(6.);ui.horizontal(|ui|{for name in ["Gentle","Balanced","Responsive"]{if ui.button(name).clicked(){self.preset(name);}}});
                ui.add_space(10.);
                ui.label(RichText::new("Step Input Source").strong());ui.label(if source!="feet" {"Assigned chest SlimeVR sensor"} else {"Assigned left and right SlimeVR sensors"});
                ui.small("Change the sensor assignments in Quick Setup. For one tracker, enable chest steps in Movement.");ui.add_space(10.);
                ui.add_enabled_ui(self.algorithm=="continuous",|ui|{
                    ui.style_mut().spacing.slider_width=(ui.available_width()-170.).clamp(90.,240.);
                    self.dirty|=ui.add(egui::Slider::new(&mut self.settings.intensity,0.01..=20.).logarithmic(true).text("Intensity Multiplier")).on_hover_text("Scales the raw acceleration signal before step detection.").changed();
                    self.dirty|=ui.add(egui::Slider::new(&mut self.settings.threshold,0.0..=100.).text("Deadzone / threshold")).on_hover_text("Minimum signal used to detect stepping.").changed();
                    self.dirty|=milliseconds(ui,&mut self.settings.inertia_window,0.01,"Inertia");
                    self.dirty|=ui.add(egui::Slider::new(&mut self.settings.max_signal,0.1..=500.).logarithmic(true).text("Maximum walking speed")).on_hover_text("Step signal that maps to maximum walking speed.").changed();
                    self.dirty|=ui.checkbox(&mut self.settings.fast_stop,"Instant stop below fast threshold").changed();
                    ui.separator();ui.label(RichText::new("Advanced smoothing & response").color(pages::ORANGE));
                    ui.small("Fast and slow windows smooth the signal; adaptive rate changes the window with intensity.");
                    self.dirty|=milliseconds(ui,&mut self.settings.fast_window,0.01,"Fast window");if self.settings.slow_window<self.settings.fast_window{self.settings.slow_window=self.settings.fast_window;}
                    self.dirty|=milliseconds(ui,&mut self.settings.slow_window,self.settings.fast_window,"Slow window");
                    let mut adaptive=self.settings.adaptive_per_unit*1000.;if ui.add(egui::Slider::new(&mut adaptive,0.0..=2000.).suffix(" ms/unit").text("Adaptive rate")).changed(){self.settings.adaptive_per_unit=adaptive/1000.;self.dirty=true;}
                    self.dirty|=ui.add(egui::Slider::new(&mut self.settings.exponent,0.1..=5.).text("Curve exponent")).changed();
                    self.dirty|=ui.checkbox(&mut self.settings.sprint_enabled,"Enable sprint threshold").changed();
                    if self.settings.sprint_enabled {let min=self.settings.max_signal.max(self.settings.threshold+0.01);self.settings.sprint_threshold=self.settings.sprint_threshold.max(min);self.dirty|=ui.add(egui::Slider::new(&mut self.settings.sprint_threshold,min..=1000.).logarithmic(true).text("Sprint signal threshold")).changed();}
                    self.dirty|=ui.add(egui::Slider::new(&mut self.settings.quiet_percent,0.0..=100.).suffix("%").text("Quiet gate")).changed();
                    ui.separator();ui.label(RichText::new("Axis filtering").color(pages::ORANGE));
                    ui.small("Checked axes contribute to movement detection. At least one axis per source must remain enabled.");
                    let sources:&[&str]=if source!="feet" {&["Chest (Raw)"]} else {&["Left Leg (Raw)","Right Leg (Raw)"]};for(foot,name)in sources.iter().enumerate(){ui.horizontal(|ui|{ui.label(*name);for(axis,label)in ["X","Y","Z"].iter().enumerate(){let last=self.settings.axes[foot][axis]&&self.settings.axes[foot].iter().filter(|on|**on).count()==1;self.dirty|=ui.add_enabled(!last,egui::Checkbox::new(&mut self.settings.axes[foot][axis],*label)).changed();}});}
                });
                if self.dirty{self.pending=None;}ui.add_space(12.);
                if ui.add_enabled(online&&!busy&&(self.dirty||self.pending.is_some()),egui::Button::new(RichText::new("APPLY & STOP").strong().color(Color32::BLACK)).fill(pages::ORANGE).min_size(egui::vec2(ui.available_width(),38.))).clicked(){let value=json!({"command":"locomotion_settings","algorithm":self.algorithm,"settings":self.settings.to_json()});self.pending=Some(value.clone());self.dirty=false;command=Some(value);}
                ui.small(if self.pending.is_some(){"Saving…"}else if self.dirty{"Unsaved draft · chart uses saved settings"}else{"Saved · tuning keeps center and stops game output"});
            });
            let ui=&mut columns[column_count-1];
            pages::card(ui,"Live Acceleration",|ui|{
                if let Some(raw)=raw {ui.label(format!("Chest acceleration X {:.2}  Y {:.2}  Z {:.2} m/s²",raw[0],raw[1],raw[2]));self.raw_chart(ui);} else {ui.label("Chest acceleration unavailable or stale");}
                ui.add_space(8.);
                if fresh {ui.horizontal_wrapped(|ui|{ui.label(RichText::new(format!("Signal {:.2}",number(&preview["signal"]))).strong().color(pages::MINT));ui.label(preview["status"].as_str().unwrap_or(""));});} else {ui.colored_label(MUTED,preview["status"].as_str().unwrap_or("Waiting for detector data"));}
                ui.add_space(6.);self.chart(ui,fresh,&motion["settings"],160.);
                if fresh {ui.horizontal_wrapped(|ui|{for(name,key,color)in [("Fast","fast",CYAN),("Slow","slow",VIOLET),("Adaptive","adaptive",MINT),("Inertia","inertia",AMBER)]{ui.colored_label(color,format!("{name} {:.2}",number(&preview[key])));}});ui.label(format!("Threshold {:.2} · Max walk {:.2} · Sprint {:.2}",number(&motion["settings"]["threshold"]),number(&motion["settings"]["max_signal"]),number(&motion["settings"]["sprint_threshold"])));let speed=number(&preview["speed"]).clamp(0.,1.);ui.label(RichText::new(format!("{} speed {:.0}%",if source=="off"{"Diagnostic"}else{"Preview"},speed*100.)).size(20.).color(pages::ORANGE));ui.add(egui::ProgressBar::new(speed).fill(pages::ORANGE).desired_width(ui.available_width()));}
                ui.small(if source=="off" {"Diagnostic only; lean movement does not use this speed."} else {"Detector preview runs while movement is stopped."});
            });
            ui.add_space(14.);pages::card(ui,"Speed Curves",|ui|{
                ui.label("Speed curve · movement intensity → stick deflection");
                ui.small("Exponent below 1 starts faster; 1 is linear; above 1 starts slower. Custom points can be dragged.");
                ui.horizontal(|ui|{self.dirty|=ui.selectable_value(&mut self.settings.curve_mode,"exponent".into(),"Exponent").changed();self.dirty|=ui.selectable_value(&mut self.settings.curve_mode,"custom".into(),"Custom points").changed();});
                let(width,height)=(ui.available_width().clamp(160.,700.),170.);let(rect,response)=ui.allocate_exact_size(egui::vec2(width,height),egui::Sense::click_and_drag());let p=ui.painter_at(rect);p.rect_filled(rect,7.,pages::BG);
                for f in [0.25,0.5,0.75]{p.line_segment([egui::pos2(rect.left(),rect.bottom()-height*f),egui::pos2(rect.right(),rect.bottom()-height*f)],Stroke::new(1.,Color32::from_rgb(48,59,73)));}
                if self.settings.curve_mode=="custom" {
                    if response.dragged(){if let Some(pos)=response.interact_pointer_pos(){let target=[((pos.x-rect.left())/width).clamp(0.,1.) as f64,((rect.bottom()-pos.y)/height).clamp(0.,1.) as f64];let points=&mut self.settings.curve_points;let n=points.len();if n>2 {let index=(1..n-1).min_by(|a,b|((points[*a][0]-target[0]).powi(2)+(points[*a][1]-target[1]).powi(2)).total_cmp(&((points[*b][0]-target[0]).powi(2)+(points[*b][1]-target[1]).powi(2)))).unwrap();points[index]=[target[0].clamp(points[index-1][0]+(points[index+1][0]-points[index-1][0])*0.001,points[index+1][0]-(points[index+1][0]-points[index-1][0])*0.001),target[1].clamp(points[index-1][1],points[index+1][1])];self.dirty=true;}}}
                    let points=self.settings.curve_points.iter().map(|v|egui::pos2(rect.left()+v[0] as f32*width,rect.bottom()-v[1] as f32*height)).collect::<Vec<_>>();p.add(egui::Shape::line(points.clone(),Stroke::new(2.5,pages::ORANGE)));for point in points{p.circle_filled(point,5.,pages::CYAN);}
                    ui.small("Drag interior points. Endpoints stay fixed and speed stays monotonic.");
                    ui.horizontal(|ui|{if ui.add_enabled(self.settings.curve_points.len()<32,egui::Button::new("Add point")).clicked(){let p=&mut self.settings.curve_points;let i=(0..p.len()-1).max_by(|a,b|(p[*a+1][0]-p[*a][0]).total_cmp(&(p[*b+1][0]-p[*b][0]))).unwrap();p.insert(i+1,[(p[i][0]+p[i+1][0])*0.5,(p[i][1]+p[i+1][1])*0.5]);self.dirty=true;}if ui.button("Linear reset").clicked(){self.settings.curve_points=vec![[0.,0.],[0.25,0.25],[0.5,0.5],[0.75,0.75],[1.,1.]];self.dirty=true;}});
                }else{let points=(0..=100).map(|i|{let x=i as f32/100.;egui::pos2(rect.left()+x*width,rect.bottom()-x.powf(self.settings.exponent as f32)*height)}).collect();p.add(egui::Shape::line(points,Stroke::new(2.5,pages::ORANGE)));}
                if let Ok(saved)=Settings::from_json(&motion["settings"]) {let points=(0..=100).map(|i|{let x=i as f32/100.;egui::pos2(rect.left()+x*width,rect.bottom()-curve_value(&saved,x)*height)}).collect();p.add(egui::Shape::line(points,Stroke::new(1.5,CYAN)));}
                if let Some((x,y))=marker(preview,fresh){p.circle_filled(egui::pos2(rect.left()+x*width,rect.bottom()-y*height),6.,MINT);ui.label(format!("Live normalized input {:.2} → output {:.2}",x,y));}else{ui.label("Live curve marker unavailable");}
                ui.label(if !fresh {"Sprint preview unavailable"} else if preview["sprint"]==true {"Sprint preview: active"}else{"Sprint preview: inactive"});
                ui.small("Orange: draft · Cyan: saved · Mint: actual receiver preview. Apply to use draft settings.");
                ui.add_space(8.);if ui.add_enabled(online&&!busy,egui::Button::new("RESET TEST")).clicked(){self.history.clear();self.raw_history.clear();command=Some(json!({"command":"reset_test"}));}
            });
        });
        command
    }
    /// Test-page step thresholds share the same draft and apply contract as Steps.
    pub fn show_test_graph(&mut self, ui:&mut egui::Ui, state:&Value, online:bool, busy:bool)->Option<Value> {
        let motion=&state["locomotion"];
        if !self.initialized && motion.is_object() {self.reload(motion);}
        if let Some(pending)=&self.pending {
            if pending["algorithm"]==motion["algorithm"] && pending["settings"]==motion["settings"] {self.reload(motion);}
        } else if self.initialized && !self.dirty {self.reload(motion);}
        if !self.initialized {ui.label("Waiting for step settings from the receiver.");return None;}
        let source=state["step_input"].as_str().unwrap_or("off");
        let preview=if source=="feet" {&motion["preview"]} else {&motion["diagnostic"]};
        let fresh=online && preview["status"]=="ready";
        let identity=format!("{}:{}:{}:{}",source,state["roles"]["torso"]["id"].as_str().unwrap_or(""),state["roles"]["left"]["id"].as_str().unwrap_or(""),state["roles"]["right"]["id"].as_str().unwrap_or(""));
        self.reset_history_if_needed(&identity,fresh);
        let now=ui.input(|i|i.time);
        if now-self.last_sample>=0.07 {self.last_sample=now;if let Some(sample)=fresh.then(||diagnostic_sample(preview)).flatten(){self.history.push(sample);if self.history.len()>160{self.history.remove(0);}}}
        let mut command=None;
        pages::card(ui,"Live Acceleration",|ui| {
            if source=="off" {ui.colored_label(AMBER,"Chest diagnostic only: step input is off. Enable chest steps in Setup to use these settings.");}
            else if self.algorithm!="continuous" {ui.colored_label(AMBER,"Continuous thresholds do not control the alternating-step algorithm. Select Continuous motion in Steps to use them.");}
            else {ui.small("Drag the handles to adjust step signal thresholds. Apply & stop saves the draft.");}
            let height=260.;let width=ui.available_width().max(1.);
            let (rect,_)=ui.allocate_exact_size(egui::vec2(width,height),egui::Sense::hover());
            #[cfg(test)] ui.ctx().data_mut(|data|data.insert_temp(egui::Id::new("test_graph_rect"),rect));
            let painter=ui.painter_at(rect);painter.rect_filled(rect,6.,pages::BG);
            let peak=self.history.iter().flatten().copied().fold(0.,f32::max);
            let ceiling=(self.settings.sprint_threshold.max(self.settings.max_signal).max(self.settings.threshold) as f32*1.25).max(peak*1.05).max(1.);
            for f in [0.25,0.5,0.75] {let y=rect.bottom()-f*height;painter.line_segment([egui::pos2(rect.left(),y),egui::pos2(rect.right(),y)],Stroke::new(1.,Color32::from_rgb(45,55,68)));}
            for (i,color) in [MUTED.gamma_multiply(0.6), CYAN, VIOLET, MINT, AMBER].into_iter().enumerate() {
                let points=self.history.iter().enumerate().map(|(n,v)|egui::pos2(rect.right()-(self.history.len()-1-n) as f32*rect.width()/159.,rect.bottom()-(v[i]/ceiling).clamp(0.,1.)*height)).collect::<Vec<_>>();
                if points.len()>1 {painter.add(egui::Shape::line(points,Stroke::new(1.5,color)));}
            }
            for (index,label,color) in [(0,"Deadzone",CYAN),(1,"Max walking",Color32::from_rgb(245,75,75)),(2,"Sprint",pages::ORANGE)] {
                let value=match index {0=>self.settings.threshold,1=>self.settings.max_signal,_=>self.settings.sprint_threshold};
                let y=rect.bottom()-(value as f32/ceiling).clamp(0.,1.)*height;
                painter.line_segment([egui::pos2(rect.left(),y),egui::pos2(rect.right(),y)],Stroke::new(1.5,color));
                let point=egui::pos2(rect.left()+12.,y.clamp(rect.top()+10.,rect.bottom()-10.));
                let hit=egui::Rect::from_center_size(point,egui::vec2(24.,24.));
                let response=ui.interact(hit,ui.id().with(("step_threshold",index)),egui::Sense::drag());
                painter.circle_filled(point,8.,color);painter.circle_stroke(point,10.,Stroke::new(1.,Color32::WHITE));
                painter.text(egui::pos2(point.x+16.,y-3.),egui::Align2::LEFT_BOTTOM,if index==2&&!self.settings.sprint_enabled {"Sprint (off)"} else {label},egui::FontId::proportional(13.),color);
                painter.text(egui::pos2(rect.right()-4.,y-3.),egui::Align2::RIGHT_BOTTOM,format!("{value:.1}"),egui::FontId::proportional(13.),color);
                if response.dragged() {if let Some(pos)=response.interact_pointer_pos(){let value=((rect.bottom()-pos.y)/height).clamp(0.,1.) as f64*ceiling as f64;set_graph_threshold(&mut self.settings,index,value);self.dirty=true;self.pending=None;}}
            }
            if fresh {ui.label(format!("Signal {:.2} · {} speed {:.0}% · Sprint {}",number(&preview["signal"]),if source=="off" {"diagnostic"}else{"preview"},number(&preview["speed"])*100.,if preview["sprint"]==true {"active"} else {"inactive"}));} else {ui.label("Waiting for fresh acceleration data");}
            ui.small("Trace: signal grey · fast cyan · slow violet · adaptive mint · inertia amber. Lines show draft values.");
            if ui.checkbox(&mut self.settings.sprint_enabled,"Enable sprint threshold").changed() {if self.settings.sprint_enabled {self.settings.sprint_threshold=self.settings.sprint_threshold.max(self.settings.max_signal).max(self.settings.threshold+0.01);}self.dirty=true;self.pending=None;}
            ui.horizontal(|ui| {
                let apply=ui.add_enabled(online&&!busy&&self.dirty,egui::Button::new("APPLY & STOP").fill(pages::ORANGE));
                #[cfg(test)] ui.ctx().data_mut(|data|data.insert_temp(egui::Id::new("test_graph_apply"),apply.rect));
                if apply.clicked(){let v=json!({"command":"locomotion_settings","algorithm":self.algorithm,"settings":self.settings.to_json()});self.pending=Some(v.clone());self.dirty=false;command=Some(v);}
                if ui.add_enabled(self.dirty,egui::Button::new("Discard")).clicked(){self.reload(motion);}
            });
            ui.small(if self.pending.is_some(){"Saving…"}else if self.dirty{"Unsaved draft"}else{"Showing saved step settings"});
        });
        command
    }
    fn reset_history_if_needed(&mut self,identity:&str,fresh:bool) {if self.history_source!=identity {self.history.clear();self.raw_history.clear();self.history_source=identity.to_owned();} if !fresh {self.history.clear();}}
    fn raw_chart(&self, ui:&mut egui::Ui) {
        let (rect,_)=ui.allocate_exact_size(egui::vec2(ui.available_width(),70.),egui::Sense::hover());let p=ui.painter_at(rect);p.rect_filled(rect,5.,pages::BG);
        let scale=self.raw_history.iter().flatten().map(|v|v.abs()).fold(1_f32,f32::max);
        for (axis,color) in [CYAN,VIOLET,MINT].into_iter().enumerate(){let points=self.raw_history.iter().enumerate().map(|(i,v)|egui::pos2(rect.right()-(self.raw_history.len()-1-i) as f32*rect.width()/159.,rect.center().y-v[axis]/scale*rect.height()*0.45)).collect::<Vec<_>>();if points.len()>1{p.add(egui::Shape::line(points,Stroke::new(1.4,color)));}}
        ui.small("Raw chest acceleration history · X cyan / Y violet / Z mint");
    }
    fn chart(&self, ui: &mut egui::Ui, available: bool, saved_settings: &Value, height: f32) {
        let width = ui.available_width().clamp(180., 1600.);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 8., Color32::from_rgb(8, 19, 34));
        let peak = self.history.iter().flatten().copied().fold(0., f32::max);
        let ceiling = (number(&saved_settings["sprint_threshold"]).max(number(&saved_settings["max_signal"])) * 1.25)
            .max(peak * 1.05)
            .max(1.);
        for fraction in [0.25, 0.5, 0.75] {
            let y = rect.bottom() - fraction * rect.height();
            painter.line_segment(
                [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                Stroke::new(1., Color32::from_rgb(28, 44, 64)),
            );
        }
        let threshold_y = rect.bottom()
            - (number(&saved_settings["threshold"]) / ceiling).clamp(0., 1.) * rect.height();
        painter.line_segment(
            [
                egui::pos2(rect.left(), threshold_y),
                egui::pos2(rect.right(), threshold_y),
            ],
            Stroke::new(1., AMBER.gamma_multiply(0.65)),
        );
        for (key,color) in [("max_signal",CYAN),("sprint_threshold",VIOLET)] {let y=rect.bottom()-(number(&saved_settings[key])/ceiling).clamp(0.,1.)*rect.height();painter.line_segment([egui::pos2(rect.left(),y),egui::pos2(rect.right(),y)],Stroke::new(1.,color.gamma_multiply(0.6)));}
        for (channel, color) in [MUTED.gamma_multiply(0.55), CYAN, VIOLET, MINT, AMBER]
            .into_iter()
            .enumerate()
        {
            let points: Vec<_> = self
                .history
                .iter()
                .enumerate()
                .map(|(index, sample)| {
                    egui::pos2(
                        rect.right()
                            - (self.history.len() - 1 - index) as f32 * rect.width() / 159.,
                        rect.bottom() - (sample[channel] / ceiling).clamp(0., 1.) * rect.height(),
                    )
                })
                .collect();
            if points.len() > 1 {
                painter.add(egui::Shape::line(
                    points,
                    Stroke::new(if channel == 3 { 2. } else { 1.2 }, color),
                ));
            }
        }
        painter.text(
            rect.left_top() + egui::vec2(8., 6.),
            egui::Align2::LEFT_TOP,
            if available {
                "ACCELERATION SIGNAL · rolling preview"
            } else {
                "Waiting for detector data"
            },
            egui::FontId::proportional(10.),
            MUTED,
        );
    }
}

#[cfg(test)] mod tests {
 use super::*;
 #[test] fn real_graph_handles_drag_and_apply() {
  let ctx=egui::Context::default();let mut panel=LocomotionPanel::default();let settings=Settings::default();
  let state=json!({"step_input":"chest","locomotion":{"algorithm":"continuous","settings":settings.to_json(),"diagnostic":{"status":"waiting"}}});
  let frame=|panel:&mut LocomotionPanel,events:Vec<egui::Event>,time:f64| {let input=egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(900.,900.))),events,time:Some(time),..Default::default()};let mut result=None;let _=ctx.run(input,|ctx|{egui::CentralPanel::default().show(ctx,|ui|{result=panel.show_test_graph(ui,&state,true,false);});});result};
  frame(&mut panel,vec![],0.);
  for index in 0..3 {
   let rect=ctx.data(|d|d.get_temp::<egui::Rect>(egui::Id::new("test_graph_rect"))).unwrap();
   let ceiling=(panel.settings.sprint_threshold.max(panel.settings.max_signal).max(panel.settings.threshold) as f32*1.25).max(1.);
   let value=match index {0=>panel.settings.threshold,1=>panel.settings.max_signal,_=>panel.settings.sprint_threshold};
   let start=egui::pos2(rect.left()+12.,(rect.bottom()-value as f32/ceiling*rect.height()).clamp(rect.top()+10.,rect.bottom()-10.));
   let end=start+egui::vec2(0.,-15.);
   let button=|pos,pressed|egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed,modifiers:Default::default()};
   frame(&mut panel,vec![egui::Event::PointerMoved(start),button(start,true)],1.+index as f64);
   frame(&mut panel,vec![egui::Event::PointerMoved(end)],1.1+index as f64);
   frame(&mut panel,vec![button(end,false)],1.2+index as f64);
   let new=match index {0=>panel.settings.threshold,1=>panel.settings.max_signal,_=>panel.settings.sprint_threshold};
   assert!(new>value,"handle {index}: {value} -> {new}");
  }
  assert!(panel.dirty);assert!(panel.settings.validate().is_ok());
  let at=ctx.data(|d|d.get_temp::<egui::Rect>(egui::Id::new("test_graph_apply"))).unwrap().center();
  let btn=|pressed|egui::Event::PointerButton{pos:at,button:egui::PointerButton::Primary,pressed,modifiers:Default::default()};
  frame(&mut panel,vec![egui::Event::PointerMoved(at),btn(true)],5.);
  let command=frame(&mut panel,vec![btn(false)],5.1).unwrap();
  assert_eq!(command["command"],"locomotion_settings");assert_eq!(command["settings"],panel.settings.to_json());
 }
 #[test] fn graph_thresholds_keep_valid_order() {
  let mut s=Settings::default();s.sprint_enabled=true;
  set_graph_threshold(&mut s,0,999.);assert!(s.threshold<s.max_signal);assert!(s.validate().is_ok());
  set_graph_threshold(&mut s,1,999.);assert!(s.max_signal<=s.sprint_threshold);assert!(s.validate().is_ok());
  set_graph_threshold(&mut s,2,0.);assert!(s.sprint_threshold>=s.max_signal);assert!(s.validate().is_ok());
  let previous=s.threshold;set_graph_threshold(&mut s,0,f64::NAN);assert_eq!(s.threshold,previous);
 }
 #[test] fn missing_stale_and_real_zero_are_distinct() {
  let state=json!({"roles":{"torso":{"id":"chest"}},"trackers":[{"id":"chest","acceleration":[0.,1.,-2.],"acceleration_age_ms":20.}]});
  assert_eq!(chest_raw(&state,true),Some([0.,1.,-2.]));assert_eq!(chest_raw(&state,false),None);
  let mut stale=state.clone();stale["trackers"][0]["acceleration_age_ms"]=json!(600.);assert_eq!(chest_raw(&stale,true),None);
  assert_eq!(marker(&json!({"raw_speed":0.3,"speed":0.2}),true),Some((0.3,0.2)));
  assert_eq!(marker(&json!({"raw_speed":0.3}),true),None);assert_eq!(marker(&json!({"raw_speed":0.3,"speed":0.2}),false),None);
 }
 #[test] fn source_change_and_stale_clear_history() {
  let mut p=LocomotionPanel::default();p.reset_history_if_needed("chest:a",true);p.history.push([1.;5]);p.raw_history.push([1.;3]);
  p.reset_history_if_needed("chest:a",true);assert_eq!(p.history.len(),1);
  p.reset_history_if_needed("chest:b",true);assert!(p.history.is_empty()&&p.raw_history.is_empty());
  p.history.push([1.;5]);p.reset_history_if_needed("chest:b",false);assert!(p.history.is_empty());
 }
}
