//! Guided setup for the receiver-owned calibration draft.
use eframe::egui;
use serde_json::{json, Value};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Default)]
pub struct SensorCalibration {
    pub open: bool,
    session: String,
    pending: Option<(String, u64, Instant)>,
    holding: bool,
    countdown: Option<Instant>,
    moving: bool,
    anchor: Option<[f64; 4]>,
    stable_since: Option<Instant>,
    last_stamp: Option<f64>,
    last_fresh: Option<Instant>,
    error: String,
    started: bool,
    still_anchor: Option<[f64;4]>,
    still_since: Option<Instant>,
    cue_at: Option<Instant>,
    confirmed: Option<(String, Instant)>,
}

fn quaternion(state: &Value) -> Option<[f64; 4]> {
    let values = state["roles"]["torso"]["quaternion"].as_array()?;
    if values.len() != 4 { return None; }
    let mut q = [0.; 4];
    for i in 0..4 { q[i] = values[i].as_f64()?; }
    let norm = q.iter().map(|v| v * v).sum::<f64>().sqrt();
    if !norm.is_finite() || norm < 0.5 { return None; }
    Some(q.map(|v| v / norm))
}


const CYAN: egui::Color32 = egui::Color32::from_rgb(97, 217, 226);
const MUTED: egui::Color32 = egui::Color32::from_rgb(155, 176, 193);

fn target(stage:&str) -> [f32;2] {
    match stage {"left"=>[-1.,0.],"right"=>[1.,0.],"forward"=>[0.,1.],"backward"=>[0.,-1.],_=>[0.,0.]}
}
fn step_number(stage:&str)->usize {
    match stage {"forward"=>1,"backward"=>2,"left"=>3,"right"=>4,"finish"|"done"=>5,_=>0}
}
fn live_motion(draft:&Value, fresh:bool) -> Option<[f32;2]> {
    if !fresh { return None; }
    let a=draft["visual_lean"].as_array()?;
    let x=a.first()?.as_f64()?;
    let y=a.get(1)?.as_f64()?;
    if !x.is_finite() || !y.is_finite() {return None;}
    let scale=15_f64.to_radians();
    Some([(x/scale).clamp(-1.3,1.3) as f32,(y/scale).clamp(-1.3,1.3) as f32])
}

fn pose_matches(stage:&str, holding:bool, moving:bool, actual:Option<[f32;2]>,
                still:bool, tilt:Option<f64>, fresh:bool) -> bool {
    if !fresh {return false;}
    if stage=="center" {return still;}
    if stage=="finish" || (holding && !moving) {
        return still && tilt.is_some_and(|v|v.is_finite() && v<=5.);
    }
    if !holding || !moving {return false;}
    let Some([x,y])=actual else {return false;};
    let [tx,ty]=target(stage);
    if tx==0. && ty==0. {return false;}
    let along=x*tx+y*ty;
    let across=(x*ty-y*tx).abs();
    along>=0.55 && along<=1.25 && across<=0.3
}

fn overlay_matches(actual:Option<[f32;2]>, guide:[f32;2]) -> bool {
    actual.is_some_and(|pose| (pose[0]-guide[0]).abs()<=0.28 && (pose[1]-guide[1]).abs()<=0.28)
}


impl SensorCalibration {
    pub fn start(&mut self) {
        *self = Self::default();
        self.open = true;
        self.session = format!("{}-{}", std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos());
    }

    pub fn start_with_state(&mut self, state: &Value) {
        self.start();
        let draft = &state["sensor_calibration"];
        if let (Some(id), Some(stage)) = (draft["session"].as_str(), draft["stage"].as_str()) {
            if !["done", "canceled"].contains(&stage) {
                self.session = id.into();
                self.started = true;
            }
        }
    }

    fn send(&mut self, state: &Value, command: &str, stage: &str) -> Value {
        self.pending = Some((stage.into(), state["steamvr"]["generation"].as_u64().unwrap_or(0), Instant::now()));
        self.error.clear();
        self.holding = false;
        self.countdown = None;
        self.moving = false;
        self.cue_at = None;
        self.anchor = None;
        self.stable_since = None;
        json!({"command":command, "session":self.session, "stage":stage})
    }

    fn stable(&mut self, q: [f64; 4], now: Instant) -> bool {
        let moved = self.anchor.is_none_or(|anchor| {
            let dot = anchor.iter().zip(q).map(|(a,b)| a*b).sum::<f64>().abs().clamp(0.,1.);
            2. * dot.acos() > 5_f64.to_radians()
        });
        if moved {
            self.anchor = Some(q);
            self.stable_since = Some(now);
        }
        self.stable_since.is_some_and(|since| now.duration_since(since) >= Duration::from_millis(600))
    }

    fn advance_capture(&mut self, stage:&str, tilt:Option<f64>, q:[f64;4], now:Instant) -> bool {
        let neutral = matches!(stage, "center" | "finish");
        let limit = if stage == "finish" { 5. } else if self.countdown.is_some() { 8. } else { 6. };
        let upright = stage == "center" || tilt.is_some_and(|a| a.is_finite() && a <= limit);
        if !self.moving {
            if !upright {
                self.countdown = None;
                self.anchor = None;
                self.stable_since = None;
                return false;
            }
            if self.countdown.is_none() && !self.stable(q, now) {
                return false;
            }
            let start = *self.countdown.get_or_insert(now);
            if now.duration_since(start) >= Duration::from_secs(3) {
                self.moving = true;
                self.anchor = None;
                self.stable_since = None;
            }
            // The cue is rendered before any movement sample may be captured.
            return false;
        }
        let moved = tilt.is_some_and(|a| a.is_finite() && (8. ..=35.).contains(&a));
        if (neutral && !upright) || (!neutral && !moved) {
            self.anchor = None;
            self.stable_since = None;
            return false;
        }
        self.stable(q, now)
    }

    fn observe_still(&mut self, q:[f64;4], now:Instant) {
        let moved=self.still_anchor.is_none_or(|anchor| {
            let dot=anchor.iter().zip(q).map(|(a,b)|a*b).sum::<f64>().abs().clamp(0.,1.);
            2.*dot.acos()>5_f64.to_radians()
        });
        if moved {self.still_anchor=Some(q);self.still_since=Some(now);}
    }

    pub fn show(&mut self, ui: &mut egui::Ui, state: &Value, receiver_error: &str, online: bool) -> (Option<Value>,Option<egui::Rect>) {
        if !self.open { return (None,None); }
        let ctx=ui.ctx().clone();
        let now = Instant::now();
        let stamp = state["statistics"]["elapsed_seconds"].as_f64();
        let advanced = online && stamp.is_some_and(|s| s.is_finite() && self.last_stamp.is_none_or(|old| s > old));
        if advanced { self.last_stamp = stamp; }
        let draft = &state["sensor_calibration"];
        let own = draft["session"].as_str() == Some(&self.session);
        let stage = if own { draft["stage"].as_str().unwrap_or("center") } else { "center" };
        let fresh = online && state["roles"]["torso"]["fresh"] == true && quaternion(state).is_some()
            && (!self.started || (own && draft["fresh"] == true));
        let mut command = None;

        if let Some((expected, generation, sent)) = &self.pending {
            let acknowledged = own && match expected.as_str() {
                "begin" => stage == "center",
                "cancel" => stage == "canceled",
                "center" => stage == "forward",
                "forward" => stage == "backward",
                "backward" => stage == "left",
                "left" => stage == "right",
                "right" => stage == "finish",
                "finish" => stage == "done" && draft["saved_file"].is_string(),
                _ => false,
            };
            if acknowledged {
                if expected == "begin" { self.started = true; }
                if expected == "cancel" || expected == "finish" { self.open = false; }
                let completed_direction=matches!(expected.as_str(),"forward"|"backward"|"left"|"right");
                if completed_direction {
                    self.confirmed=Some((expected.clone(),now+Duration::from_millis(900)));
                }
                self.pending = None;
                self.holding = !completed_direction && !["done", "canceled"].contains(&stage);
            } else if advanced && state["steamvr"]["generation"].as_u64() != Some(*generation) && !receiver_error.is_empty() {
                self.error = receiver_error.into();
                self.pending = None;
            } else if sent.elapsed() > Duration::from_secs(6) {
                self.error = "No confirmation received. Retry this step.".into();
                self.pending = None;
            }
        }
        if !self.open {ctx.request_repaint();return (None,None);}
        if self.started && advanced && !own && self.pending.is_none() {
            self.error = "The receiver restarted or the session changed. Close and reopen calibration.".into();
        }
        if self.confirmed.as_ref().is_some_and(|(_,until)|now>=*until) {
            self.confirmed=None;
            self.holding=true;
        }
        let stream_ok=fresh && self.last_fresh.is_none_or(|last| now.duration_since(last)<=Duration::from_millis(400));
        if !stream_ok {self.still_anchor=None;self.still_since=None;}
        if advanced && fresh {
            if let Some(q)=quaternion(state) {self.observe_still(q,now);}
            self.last_fresh=Some(now);
        }
        if self.holding {
            let timely = stream_ok;
            if !timely {
                self.anchor = None;
                self.stable_since = None;
                self.countdown = None;
                self.moving = false;
            }
            if advanced && fresh {
                self.last_fresh = Some(now);
                if let Some(q) = quaternion(state) {
                    let tilt = draft["tilt_degrees"].as_f64();
                    let was_moving=self.moving;
                    if self.advance_capture(stage, tilt, q, now) {
                        command = Some(self.send(state, "sensor_calibration_capture", stage));
                    } else if !was_moving && self.moving {
                        self.cue_at=Some(now);
                    }
                }
            }
        }
        // This fixture is available only for application-owned QA screenshots.
        let qa_stage=if crate::qa_capture::path().is_some() {
            std::env::var("ARCANEVR_CALIBRATION_QA_STAGE").ok()
        } else {None};
        let confirmation_stage=self.confirmed.as_ref().map(|(stage,_)|stage.clone());
        let display_stage=qa_stage.as_deref().filter(|s| ["center","forward","backward","left","right","finish"].contains(s))
            .or(confirmation_stage.as_deref()).unwrap_or(stage);
        let qa=qa_stage.is_some();
        let target=target(display_stage);
        let progress=if qa {
            std::env::var("ARCANEVR_CALIBRATION_QA_PHASE").ok().and_then(|s|s.parse::<f32>().ok()).unwrap_or(1.).clamp(0.,1.)
        } else if confirmation_stage.is_some() {
            1.
        } else if self.moving {
            self.cue_at.map_or(0.,|at|(now.duration_since(at).as_secs_f32()/0.7).clamp(0.,1.))
        } else {0.};
        let guide=[target[0]*progress*0.8,target[1]*progress*0.8];
        let actual=if qa {
            let sign=if std::env::var_os("ARCANEVR_CALIBRATION_QA_WRONG").is_some() {-1.} else {1.};
            Some([target[0]*0.8*sign,target[1]*0.8*sign])
        } else {live_motion(draft,fresh && own)};
        let still=if qa {true} else {
            self.still_since.is_some_and(|since| now.duration_since(since)>=Duration::from_millis(600))
        };
        let tilt=if qa {Some(0.)} else {draft["tilt_degrees"].as_f64()};
        let matched=pose_matches(display_stage,self.holding || qa || confirmation_stage.is_some(),
            self.moving || qa || confirmation_stage.is_some(),
            actual,still,tilt,if qa {true} else {fresh})
            && (matches!(display_stage,"center"|"finish") || (!self.moving && confirmation_stage.is_none() && !qa)
                || overlay_matches(actual,guide));
        let green=egui::Color32::from_rgb(89,225,152);
        let mut viewer_rect=None;
        let available=egui::vec2(ui.available_width(),ctx.available_rect().height());
        ui.set_min_size(egui::vec2(available.x.max(0.),available.y.max(0.)));
        ui.add_space(20.);
        ui.horizontal(|ui| {
            ui.add_space(24.);
            ui.label(egui::RichText::new("ArcaneVR").size(32.).strong().color(egui::Color32::from_rgb(255,155,57)));
            ui.add_space(20.);
            ui.label(egui::RichText::new("SENSOR CALIBRATION").size(13.).strong().color(CYAN));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {
                ui.add_space(24.);
                ui.label(egui::RichText::new(format!("{} / 6",step_number(display_stage)+1)).size(15.).color(MUTED));
            });
        });
        ui.add_space(18.);
        let pane_height=(available.y-112.).clamp(360.,760.);
        ui.columns(2,|columns| {
            egui::Frame::new().fill(egui::Color32::from_rgb(19,30,44))
                .stroke(egui::Stroke::new(1.,egui::Color32::from_rgb(43,70,90)))
                .corner_radius(14).inner_margin(22).show(&mut columns[0],|ui| {
                    ui.set_min_height(pane_height-44.);
                    ui.label(egui::RichText::new("YOUR POSITION").size(12.).strong().color(CYAN));
                    ui.add_space(14.);
                    let title=match display_stage {
                        "forward"=>"Lean forward", "backward"=>"Lean backward",
                        "left"=>"Lean left", "right"=>"Lean right",
                        "finish"=>"Return to center", _=>"Stand comfortably upright",
                    };
                    ui.label(egui::RichText::new(title).size(28.).strong().color(egui::Color32::WHITE));
                    let instruction=if !fresh && !qa {"Waiting for your sensor".to_owned()}
                        else if self.pending.is_some() {"Saving your movement…".to_owned()}
                        else if confirmation_stage.is_some() {"Movement captured".to_owned()}
                        else if self.holding && !self.moving && !qa {
                            self.countdown.map_or("Return to center".to_owned(),|start| {
                                let n=3_u64.saturating_sub(now.duration_since(start).as_secs()).max(1);
                                format!("Get ready · {n}")
                            })
                        } else if self.moving || qa {
                            if matches!(display_stage,"center"|"finish") {"Hold comfortably".to_owned()}
                            else if actual.is_none() {"Lean now · learning your movement".to_owned()}
                            else {"Lean now · hold briefly".to_owned()}
                        } else if !self.started {"Follow the instructions to begin".to_owned()}
                        else {"Ready when you are".to_owned()};
                    ui.add_space(8.);
                    ui.label(egui::RichText::new(instruction).size(18.).color(if matched {green} else {MUTED}));
                    ui.add_space(28.);
                    if let Some([x,y])=actual {
                        let side=x*15.; let depth=y*15.;
                        ui.label(egui::RichText::new("SIDE").size(12.).strong().color(MUTED));
                        ui.label(egui::RichText::new(if side.abs()<1. {"Centered".to_owned()} else {format!("{}  {:.0}°",if side<0. {"LEFT"} else {"RIGHT"},side.abs())}).size(23.).color(egui::Color32::WHITE));
                        ui.add_space(14.);
                        ui.label(egui::RichText::new("FORWARD / BACK").size(12.).strong().color(MUTED));
                        ui.label(egui::RichText::new(if depth.abs()<1. {"Centered".to_owned()} else {format!("{}  {:.0}°",if depth>0. {"FORWARD"} else {"BACK"},depth.abs())}).size(23.).color(egui::Color32::WHITE));
                    } else {
                        ui.label(egui::RichText::new(if matched {"Centered and still"} else {"Position will appear as you move"}).size(18.).color(if matched {green} else {MUTED}));
                    }
                    if matched {ui.add_space(12.);ui.label(egui::RichText::new("MATCHED").size(16.).strong().color(green));}
                    if !self.error.is_empty() {ui.add_space(12.);ui.colored_label(egui::Color32::LIGHT_RED,&self.error);}
                    ui.add_space(28.);
                    ui.horizontal(|ui| {for i in 0..6 {
                        let color=if i<=step_number(display_stage) {CYAN} else {egui::Color32::from_rgb(53,69,83)};
                        let (r,_)=ui.allocate_exact_size(egui::vec2(24.,4.),egui::Sense::hover());
                        ui.painter().rect_filled(r,2.,color);
                    }});
                    ui.add_space(20.);
                    ui.horizontal_wrapped(|ui| {
                        if self.pending.is_some() {ui.add_enabled(false,egui::Button::new("Please wait"));}
                        else if confirmation_stage.is_some() {ui.add_enabled(false,egui::Button::new("Captured"));}
                        else if self.holding {
                            if ui.add_sized([120.,40.],egui::Button::new("Pause")).clicked() {
                                self.holding=false;self.countdown=None;self.moving=false;self.cue_at=None;
                                self.anchor=None;self.stable_since=None;
                            }
                        } else if !self.started {
                            if ui.add_enabled(fresh,egui::Button::new(egui::RichText::new("Start calibration").strong()).min_size(egui::vec2(150.,40.)).fill(CYAN)).clicked() {
                                command=Some(self.send(state,"sensor_calibration_begin","begin"));
                            }
                        } else if ui.add_enabled(fresh && own,egui::Button::new(egui::RichText::new("Continue").strong()).min_size(egui::vec2(120.,40.)).fill(CYAN)).clicked() {
                            self.error.clear();self.holding=true;self.countdown=None;self.moving=false;self.cue_at=None;
                            self.anchor=None;self.stable_since=None;self.last_fresh=None;
                        }
                        if ui.add_enabled(self.pending.is_none(),egui::Button::new("Cancel").min_size(egui::vec2(90.,40.))).clicked() {
                            if self.started || own {command=Some(self.send(state,"sensor_calibration_cancel","cancel"));}
                            else {self.open=false;}
                        }
                    });
                });
            egui::Frame::new().fill(egui::Color32::from_rgb(14,24,34))
                .stroke(egui::Stroke::new(1.,if matched {green} else {egui::Color32::from_rgb(43,70,90)}))
                .corner_radius(14).inner_margin(5).show(&mut columns[1],|ui| {
                    let size=egui::vec2(ui.available_width().max(1.),(pane_height-10.).max(1.));
                    let (rect,_)=ui.allocate_exact_size(size,egui::Sense::hover());
                    viewer_rect=Some(rect.intersect(ui.clip_rect()));
                });
        });
        if self.open { ctx.request_repaint_after(Duration::from_millis(50)); }
        (command,viewer_rect)
    }

    pub fn notice(&self) -> &str { &self.error }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibration_buttons_start_pause_resume_and_cancel() {
        fn frame(ctx:&egui::Context, wizard:&mut SensorCalibration, state:&Value, events:Vec<egui::Event>) -> (Option<Value>,egui::FullOutput) {
            let mut command=None;
            let input=egui::RawInput {
                screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(1400.,940.))),
                events,..Default::default()
            };
            let output=ctx.run(input,|ctx|{egui::CentralPanel::default().show(ctx,|ui|{command=wizard.show(ui,state,"",true).0;});});
            (command,output)
        }
        fn click(ctx:&egui::Context, wizard:&mut SensorCalibration, state:&Value, label:&str)->Option<Value> {
            let (_,output)=frame(ctx,wizard,state,vec![]);
            let pos=output.shapes.iter().find_map(|shape| {
                if let egui::Shape::Text(text)=&shape.shape {
                    if text.galley.text()==label {return Some(text.pos+text.galley.size()/2.);}
                }
                None
            }).unwrap_or_else(||panic!("button not rendered: {label}"));
            let mut result=None;
            for pressed in [true,false] {
                let event=egui::Event::PointerButton {pos,button:egui::PointerButton::Primary,pressed,modifiers:Default::default()};
                result=frame(ctx,wizard,state,vec![egui::Event::PointerMoved(pos),event]).0;
            }
            result
        }
        let ctx=egui::Context::default();
        let mut wizard=SensorCalibration::default();
        wizard.start();
        let mut state=json!({
            "roles":{"torso":{"fresh":true,"quaternion":[1.,0.,0.,0.]}},
            "sensor_calibration":null,"steamvr":{"generation":1},"statistics":{"elapsed_seconds":1.}
        });
        frame(&ctx,&mut wizard,&state,vec![]);
        let begin=click(&ctx,&mut wizard,&state,"Start calibration").unwrap();
        assert_eq!(begin["command"],"sensor_calibration_begin");
        state["sensor_calibration"]=json!({"session":wizard.session,"stage":"center","fresh":true});
        state["steamvr"]["generation"]=json!(2);
        state["statistics"]["elapsed_seconds"]=json!(2.);
        frame(&ctx,&mut wizard,&state,vec![]);
        assert!(wizard.holding);
        assert!(click(&ctx,&mut wizard,&state,"Pause").is_none());
        assert!(!wizard.holding);
        assert!(click(&ctx,&mut wizard,&state,"Continue").is_none());
        assert!(wizard.holding);
        let cancel=click(&ctx,&mut wizard,&state,"Cancel").unwrap();
        assert_eq!(cancel["command"],"sensor_calibration_cancel");
        state["sensor_calibration"]["stage"]=json!("canceled");
        state["steamvr"]["generation"]=json!(3);
        frame(&ctx,&mut wizard,&state,vec![]);
        assert!(!wizard.open);
    }

    #[test]
    fn match_feedback_requires_real_fresh_alignment() {
        assert!(pose_matches("center",true,false,None,true,None,true));
        assert!(!pose_matches("center",true,false,None,false,None,true));
        assert!(!pose_matches("center",true,false,None,true,None,false));
        assert!(pose_matches("left",true,false,None,true,Some(3.),true));
        assert!(!pose_matches("left",true,false,None,true,Some(7.),true));
        assert!(!pose_matches("forward",true,true,None,true,Some(14.),true));
        assert!(pose_matches("left",true,true,Some([-0.8,0.]),false,Some(12.),true));
        assert!(!pose_matches("left",true,true,Some([0.8,0.]),false,Some(12.),true));
        assert!(!pose_matches("left",true,true,Some([-0.8,0.5]),false,Some(12.),true));
        assert!(!pose_matches("left",true,true,Some([-0.8,0.]),false,Some(12.),false));
        assert!(overlay_matches(Some([-0.8,0.]),[-0.8,0.]));
        assert!(!overlay_matches(Some([0.8,0.]),[-0.8,0.]));
        assert!(!overlay_matches(None,[-0.8,0.]));
    }

    #[test]
    fn center_feedback_waits_until_tracker_is_still() {
        let mut wizard=SensorCalibration::default();
        let now=Instant::now();
        let zero=[1.,0.,0.,0.];
        wizard.observe_still(zero,now);
        assert!(wizard.still_since.is_some_and(|since| (now+Duration::from_millis(700)).duration_since(since)>=Duration::from_millis(600)));
        let tilted=[0.9950041653,0.0998334166,0.,0.];
        wizard.observe_still(tilted,now+Duration::from_millis(700));
        assert_eq!(wizard.still_since,Some(now+Duration::from_millis(700)));
    }

    #[test]
    fn recorded_direction_is_briefly_confirmed_before_next_step() {
        let mut wizard=SensorCalibration::default();
        wizard.start();
        wizard.started=true;
        wizard.pending=Some(("forward".into(),1,Instant::now()));
        let state=json!({
            "roles":{"torso":{"fresh":true,"quaternion":[1.,0.,0.,0.]}},
            "sensor_calibration":{"session":wizard.session,"stage":"backward","fresh":true,"tilt_degrees":12.,"visual_lean":[0.,0.21]},
            "steamvr":{"generation":2},"statistics":{"elapsed_seconds":1.}
        });
        let ctx=egui::Context::default();
        let _=ctx.run(egui::RawInput::default(),|ctx| {egui::CentralPanel::default().show(ctx,|ui|{wizard.show(ui,&state,"",true);});});
        assert_eq!(wizard.confirmed.as_ref().map(|(stage,_)|stage.as_str()),Some("forward"));
        assert!(!wizard.holding);
        wizard.confirmed=Some(("forward".into(),Instant::now()-Duration::from_millis(1)));
        let _=ctx.run(egui::RawInput::default(),|ctx| {egui::CentralPanel::default().show(ctx,|ui|{wizard.show(ui,&state,"",true);});});
        assert!(wizard.confirmed.is_none());
        assert!(wizard.holding);
    }
    #[test]
    fn live_silhouette_uses_signed_fresh_movement() {
        let value=json!({"visual_lean":[-0.2617993878,0.1308996939]});
        let motion=live_motion(&value,true).unwrap();
        assert!((motion[0]+1.).abs()<0.001 && (motion[1]-0.5).abs()<0.001);
        assert!(live_motion(&value,false).is_none());
        assert!(live_motion(&json!({"visual_lean":null}),true).is_none());
        assert_eq!(target("left"),[-1.,0.]);
        assert_eq!(target("right"),[1.,0.]);
    }
    #[test]
    fn cue_precedes_capture_and_only_post_cue_lean_is_learned() {
        let mut wizard=SensorCalibration::default();
        let now=Instant::now();
        let zero=[1.,0.,0.,0.];
        let lean=[0.9950041653,0.,0.0998334166,0.];
        assert!(!wizard.advance_capture("left",Some(0.),zero,now));
        assert!(!wizard.advance_capture("left",Some(0.),zero,now+Duration::from_secs(2)));
        assert!(!wizard.advance_capture("left",Some(0.),zero,now+Duration::from_secs(4)));
        assert!(!wizard.moving,"countdown has not completed");
        assert!(!wizard.advance_capture("left",Some(0.),zero,now+Duration::from_secs(5)));
        assert!(wizard.moving,"Lean left now");
        assert!(!wizard.advance_capture("left",Some(0.),zero,now+Duration::from_secs(8)));
        assert!(!wizard.advance_capture("left",Some(11.5),lean,now+Duration::from_secs(9)));
        assert!(wizard.advance_capture("left",Some(11.5),lean,now+Duration::from_secs(11)));
    }

    #[test]
    fn early_lean_restarts_countdown() {
        let mut wizard=SensorCalibration::default();
        let now=Instant::now();
        let zero=[1.,0.,0.,0.];
        wizard.advance_capture("left",Some(0.),zero,now);
        wizard.advance_capture("left",Some(0.),zero,now+Duration::from_secs(2));
        assert!(wizard.countdown.is_some());
        assert!(!wizard.advance_capture("left",Some(10.),zero,now+Duration::from_secs(3)));
        assert!(wizard.countdown.is_none());
        assert!(!wizard.moving);
    }

    #[test]
    fn movement_restarts_capture_and_quaternion_sign_is_equivalent() {
        let mut wizard = SensorCalibration::default();
        let now = Instant::now();
        assert!(!wizard.stable([1.,0.,0.,0.], now));
        assert!(wizard.stable([-1.,0.,0.,0.], now + Duration::from_secs(2)));
        let q = [0.9950041653,0.0998334166,0.,0.];
        assert!(!wizard.stable(q, now + Duration::from_secs(3)));
        assert!(wizard.stable(q, now + Duration::from_secs(5)));
    }

    fn rotation(degrees: f64) -> [f64; 4] {
        let half = degrees.to_radians() / 2.;
        [half.cos(), half.sin(), 0., 0.]
    }

    #[test]
    fn normal_sway_does_not_restart_readiness_or_countdown() {
        let mut wizard = SensorCalibration::default();
        let now = Instant::now();
        assert!(!wizard.advance_capture("left", Some(0.), rotation(0.), now));
        assert!(!wizard.advance_capture("left", Some(3.), rotation(3.), now + Duration::from_millis(300)));
        assert!(!wizard.advance_capture("left", Some(3.), rotation(-3.), now + Duration::from_millis(600)));
        let started = wizard.countdown.expect("sway should allow readiness");
        assert!(!wizard.advance_capture("left", Some(7.), rotation(7.), now + Duration::from_secs(2)));
        assert_eq!(wizard.countdown, Some(started));
        assert!(!wizard.advance_capture("left", Some(3.), rotation(-3.), now + Duration::from_millis(3600)));
        assert!(wizard.moving);
        assert!(!wizard.advance_capture("left", Some(13.), rotation(13.), now + Duration::from_millis(3700)));
        assert!(!wizard.advance_capture("left", Some(17.), rotation(17.), now + Duration::from_millis(4000)));
        assert!(wizard.advance_capture("left", Some(15.), rotation(15.), now + Duration::from_millis(4300)));
    }

    #[test]
    fn finish_rejects_tilt_above_five_degrees() {
        let mut wizard = SensorCalibration::default();
        let now = Instant::now();
        wizard.advance_capture("finish", Some(5.1), rotation(5.1), now);
        wizard.advance_capture("finish", Some(5.1), rotation(5.1), now + Duration::from_secs(1));
        assert!(wizard.countdown.is_none());
        wizard.advance_capture("finish", Some(0.), rotation(0.), now + Duration::from_secs(2));
        wizard.advance_capture("finish", Some(0.), rotation(0.), now + Duration::from_millis(2600));
        assert!(wizard.countdown.is_some());
        wizard.advance_capture("finish", Some(5.1), rotation(5.1), now + Duration::from_secs(3));
        assert!(wizard.countdown.is_none());
        wizard.moving = true;
        assert!(!wizard.advance_capture("finish", Some(5.1), rotation(5.1), now + Duration::from_secs(4)));
        assert!(!wizard.advance_capture("finish", Some(5.1), rotation(5.1), now + Duration::from_secs(5)));
    }

    #[test]
    fn reopening_adopts_only_an_active_draft() {
        let mut wizard = SensorCalibration::default();
        wizard.start_with_state(&json!({"sensor_calibration":{"session":"draft","stage":"right"}}));
        assert!(wizard.started);
        assert_eq!(wizard.session,"draft");
        wizard.start_with_state(&json!({"sensor_calibration":{"session":"old","stage":"done"}}));
        assert!(!wizard.started);
        assert_ne!(wizard.session,"old");
    }

    #[test]
    fn completion_requires_saved_file_and_cancel_confirmation() {
        let mut wizard = SensorCalibration::default();
        wizard.start();
        let mut state = json!({"sensor_calibration":{"session":wizard.session,"stage":"done"},
            "steamvr":{"generation":2},"statistics":{"elapsed_seconds":1.}});
        wizard.send(&state,"sensor_calibration_capture","finish");
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {egui::CentralPanel::default().show(ctx,|ui|{wizard.show(ui,&state,"",true);});});
        assert!(wizard.pending.is_some());
        assert!(wizard.open,"a missing saved file must not complete calibration");
        state["sensor_calibration"]["saved_file"] = json!("calibrations/new.json");
        let _ = ctx.run(egui::RawInput::default(), |ctx| {egui::CentralPanel::default().show(ctx,|ui|{wizard.show(ui,&state,"",true);});});
        assert!(wizard.pending.is_none());
        assert!(!wizard.open,"confirmed saved calibration returns home automatically");
        wizard.send(&state,"sensor_calibration_cancel","cancel");
        state["sensor_calibration"]["stage"] = json!("canceled");
        let _ = ctx.run(egui::RawInput::default(), |ctx| {egui::CentralPanel::default().show(ctx,|ui|{wizard.show(ui,&state,"",true);});});
        assert!(!wizard.open);
    }
}
