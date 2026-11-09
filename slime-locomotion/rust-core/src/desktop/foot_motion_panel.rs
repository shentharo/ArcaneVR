//! A narrow live settings draft; preserves calibration and the output arming state.
use arcane_bridge_core::foot_motion::Settings;
use eframe::egui;
use serde_json::{json, Value};

#[derive(Default)]
pub struct FootMotionPanel {
    draft: Option<Settings>,
    saved: Value,
    pending: Option<(Value, f64)>,
    notice: String,
}
impl FootMotionPanel {
    pub fn show(&mut self, ui: &mut egui::Ui, state: &Value, online: bool, busy: bool, error: &str) -> Option<Value> {
        if state["step_input"] != "feet" || state["locomotion"]["algorithm"] != "cadence" {
            return None;
        }
        let current = &state["foot_motion"]["settings"];
        let Ok(settings) = Settings::from_json(current) else {
            ui.small("Small steps & turning requires an updated receiver.");
            return None;
        };
        let now = ui.input(|i| i.time);
        if let Some((pending, sent)) = &self.pending {
            if online && pending == current {
                self.pending = None;
                self.notice = "Applied live. Center preserved; step rhythm cleared.".into();
                self.saved = current.clone();
                self.draft = Some(settings.clone());
            } else if now - sent > 3. {
                self.pending = None;
                self.notice = if error.is_empty() {"Apply not confirmed. Check receiver connection and retry.".into()}
                    else {format!("Not applied: {error}")};
            }
        }
        let dirty = self.draft.as_ref().is_some_and(|draft| draft.to_json() != self.saved);
        // Refresh untouched drafts only. A later snapshot must not erase local edits.
        if self.draft.is_none() || (!dirty && self.pending.is_none()) {
            self.draft = Some(settings);
            self.saved = current.clone();
        }
        let mut command = None;
        egui::CollapsingHeader::new("Small steps & turning").id_salt("foot_motion_live").default_open(std::env::args().any(|arg|arg=="--qa-foot-motion-open")).show(ui, |ui| {
            ui.small("Applies live without recentering or changing whether game movement is enabled.");
            ui.small("With the filter enabled, body turning pauses translation. Let your heading settle, then take new steps.");
            let draft = self.draft.as_mut().unwrap();
            ui.add_enabled_ui(online && !busy && self.pending.is_none(), |ui| {
                let mut edited = ui.checkbox(&mut draft.enabled, "Filter foot pivots and tiny adjustments").changed();
                ui.add_enabled_ui(draft.enabled, |ui| {
                    edited |= ui.add(egui::Slider::new(&mut draft.min_swing_degrees, 0.5..=8.).suffix("°").text("Minimum foot swing")).changed();
                    ui.small("Lower values allow smaller steps. Raise it to ignore tiny foot adjustments.");
                    edited |= ui.add(egui::Slider::new(&mut draft.pivot_ratio, 1.0..=5.0).text("Foot turning tolerance")).changed();
                    ui.small("Lower values reject more foot twisting. Body turning still pauses translation.");
                });
                if edited { self.notice.clear(); }
            });
            let dirty = draft.to_json() != *current;
            let mut discard = false;
            ui.horizontal(|ui| {
                if ui.add_enabled(online && !busy && self.pending.is_none() && dirty, egui::Button::new("Apply live")).clicked() {
                    let settings = draft.to_json();
                    command = Some(json!({"command":"foot_motion_settings","settings":settings}));
                    self.pending = Some((settings, now));
                    self.notice = "Waiting for receiver confirmation…".into();
                }
                if ui.add_enabled(self.pending.is_none() && dirty, egui::Button::new("Discard draft")).clicked() {
                    discard = true;
                }
                if dirty && self.pending.is_none() { ui.small("Draft — not applied"); }
            });
            if discard {
                self.draft = Settings::from_json(current).ok();
                self.saved = current.clone();
                self.notice.clear();
            }
            if !online { ui.small("Receiver offline. Live tuning unavailable."); }
            if !self.notice.is_empty() { ui.small(&self.notice); }
            if online {
                if state["foot_motion"]["pivot_suppressed"] == true {
                    ui.small("Turning detected: translation stopped. Wait for your heading to settle.");
                }
                if let Some(feet) = state["foot_motion"]["feet"].as_array() {
                    let labels: Vec<_> = feet.iter().enumerate().filter_map(|(i, foot)| {
                        foot["reason"].as_str().map(|reason| format!("{}: {}", if i == 0 {"Left"} else {"Right"}, reason.replace('_', " ")))
                    }).collect();
                    if !labels.is_empty() { ui.small(labels.join(" · ")); }
                }
            }
        });
        command
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> Value {
        json!({"step_input":"feet","locomotion":{"algorithm":"cadence"},"foot_motion":{"settings":Settings::default().to_json(),"pivot_suppressed":true,"feet":[{"reason":"pivot"},{"reason":"tiny_motion"}]}})
    }
    fn frame(panel: &mut FootMotionPanel, ctx: &egui::Context, state: &Value, online: bool, events: Vec<egui::Event>) -> (Option<Value>, Vec<(String, egui::Rect)>) {
        let mut command = None;
        let output = ctx.run(egui::RawInput {screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(850.,650.))), events, ..Default::default()}, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| { command = panel.show(ui,state,online,false,""); });
        });
        fn texts(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
            match shape {
                egui::Shape::Text(t) => out.push((t.galley.job.text.clone(), egui::Rect::from_min_size(t.pos,t.galley.size()))),
                egui::Shape::Vec(shapes) => for shape in shapes { texts(shape,out); }, _ => (),
            }
        }
        let mut text = Vec::new();
        for shape in output.shapes { texts(&shape.shape,&mut text); }
        (command,text)
    }
    fn click(panel:&mut FootMotionPanel,ctx:&egui::Context,state:&Value,online:bool,label:&str)->Option<Value> {
        let (_,text)=frame(panel,ctx,state,online,vec![]);
        let pos=text.iter().find(|(text,_)|text==label).unwrap_or_else(||panic!("Missing {label}: {text:?}")).1.center();
        let mut result=None;
        for pressed in [true,false] {
            result=frame(panel,ctx,state,online,vec![egui::Event::PointerMoved(pos),egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed,modifiers:Default::default()}]).0;
        }
        result
    }
    fn open(panel:&mut FootMotionPanel,ctx:&egui::Context,state:&Value) {
        frame(panel,ctx,state,true,vec![]);
        click(panel,ctx,state,true,"Small steps & turning");
        // Let egui finish opening before clicking body controls.
        ctx.style_mut(|style|style.animation_time=0.);
        frame(panel,ctx,state,true,vec![]);
    }
    #[test]
    fn real_click_draft_apply_and_receiver_acknowledgment() {
        let ctx=egui::Context::default(); let mut panel=FootMotionPanel::default(); let mut state=state();
        open(&mut panel,&ctx,&state);
        assert!(click(&mut panel,&ctx,&state,true,"Filter foot pivots and tiny adjustments").is_none());
        assert!(!panel.draft.as_ref().unwrap().enabled);
        assert!(panel.pending.is_none());
        assert_eq!(state["foot_motion"]["settings"]["enabled"],true);
        let command=click(&mut panel,&ctx,&state,true,"Apply live").unwrap();
        let mut expected=Settings::default().to_json(); expected["enabled"]=json!(false);
        assert_eq!(command,json!({"command":"foot_motion_settings","settings":expected}));
        assert!(panel.pending.is_some());
        state["foot_motion"]["settings"]=expected;
        let (_,text)=frame(&mut panel,&ctx,&state,true,vec![]);
        assert!(panel.pending.is_none());
        assert!(text.iter().any(|(t,_)|t.contains("Applied live")));
        assert!(text.iter().any(|(t,_)|t.contains("translation stopped")));
    }
    #[test]
    fn offline_apply_and_non_cadence_cannot_emit_commands() {
        let ctx=egui::Context::default(); let mut panel=FootMotionPanel::default(); let mut state=state();
        open(&mut panel,&ctx,&state);
        click(&mut panel,&ctx,&state,true,"Filter foot pivots and tiny adjustments");
        assert!(click(&mut panel,&ctx,&state,false,"Apply live").is_none());
        assert!(panel.pending.is_none());
        state["locomotion"]["algorithm"]=json!("continuous");
        let (command,text)=frame(&mut panel,&ctx,&state,true,vec![]);
        assert!(command.is_none()); assert!(text.is_empty());
    }
    #[test]
    fn newer_saved_fields_are_not_sent_by_live_gate_command() {
        let ctx=egui::Context::default(); let mut panel=FootMotionPanel::default(); let mut state=state();
        open(&mut panel,&ctx,&state);
        click(&mut panel,&ctx,&state,true,"Filter foot pivots and tiny adjustments");
        state["actions"]=json!({"settings":{"direction":{"learned_forward":[1.,0.,0.]}}});
        state["locomotion"]["settings"]=json!({"intensity":9.});
        let command=click(&mut panel,&ctx,&state,true,"Apply live").unwrap();
        assert_eq!(command.as_object().unwrap().len(),2);
        assert!(command.get("actions").is_none()); assert!(command.get("locomotion").is_none());
    }
}
