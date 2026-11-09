//! Saved head-gesture gate and receiver-owned progress; no local settings draft.
use eframe::egui;
use serde_json::{json, Value};

const LABEL: &str = "Recenter with four head sweeps";

#[derive(Default)]
pub struct HeadRecenterPanel;

impl HeadRecenterPanel {
    pub fn show(&mut self, ui: &mut egui::Ui, state: &Value, online: bool, busy: bool) -> Option<Value> {
        control(ui, state, online, busy).0
    }
}

fn control(ui: &mut egui::Ui, state: &Value, online: bool, busy: bool) -> (Option<Value>, egui::Response) {
    let gesture = &state["head_recenter"];
    let saved = gesture["settings"]["enabled"].as_bool();
    let mut enabled = saved.unwrap_or(false);
    let response = ui.add_enabled(online && !busy && saved.is_some(), egui::Checkbox::new(&mut enabled, LABEL));
    let command = response.changed().then(|| json!({"command":"head_recenter_settings", "settings":{"enabled":enabled}}));
    ui.small("Plant both feet; keep chest still. Look forward, then left-right four times (about 90° each side), then forward and hold still.");
    ui.small("Works on every page. The new center resumes movement only if movement was already on.");
    if saved.is_none() {
        ui.small("Waiting for saved head-recenter settings.");
    } else if !online {
        ui.small("Receiver offline. Head-recenter status unavailable.");
    } else if saved == Some(true) {
        let completed = gesture["completed_cycles"].as_u64().unwrap_or(0);
        let required = gesture["required_cycles"].as_u64().unwrap_or(4).max(1);
        ui.small(format!("Head sweeps {}/{required}", completed.min(required)));
        let progress = gesture["progress"].as_f64().unwrap_or(completed as f64 / required as f64);
        ui.add(egui::ProgressBar::new(progress.clamp(0., 1.) as f32).desired_width(220.));
        let phase = gesture["phase"].as_str().unwrap_or("waiting_still");
        let label = if gesture["pending_resume"] == true {
            "Center updated; waiting for fresh driver confirmation before resuming."
        } else { match phase {
            "centering" => "Centering — keep looking forward and hold still.",
            "resume" | "resuming" => "Resuming movement that was already on.",
            "waiting_still" | "waiting_forward" | "waiting_for_forward" => "Look forward and hold still to begin.",
            "return_forward" => "Return forward and hold still to recenter.",
            "cooldown" | "complete" | "completed" => "Head-sweep gesture completed.",
            "disabled" => "Head-sweep recentering is off.",
            "sweeping" | "collecting" => "Sweep left-right; keep both feet planted and chest still.",
            _ => "Follow the head-sweep instructions above.",
        }};
        ui.small(label);
        if let Some(result) = gesture["resume_result"].as_str().filter(|result| !result.is_empty()) {
            ui.small(result);
        }
        if let Some(reason) = gesture["reason"].as_str().filter(|reason| !reason.is_empty()) {
            ui.small(reason);
        }
    }
    (command, response)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(enabled: bool) -> Value {
        json!({"head_recenter":{"settings":{"enabled":enabled},"phase":"waiting_still",
            "completed_cycles":0,"required_cycles":4,"progress":0.0}})
    }

    fn frame(ctx: &egui::Context, state: &Value, online: bool, busy: bool, events: Vec<egui::Event>) -> (Option<Value>, egui::Response, Vec<String>) {
        let mut result = None;
        let output = ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 400.))),
            events, ..Default::default()
        }, |ctx| { egui::CentralPanel::default().show(ctx, |ui| { result = Some(control(ui, state, online, busy)); }); });
        fn texts(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => out.push(text.galley.job.text.clone()),
                egui::Shape::Vec(shapes) => for shape in shapes { texts(shape, out); }, _ => (),
            }
        }
        let mut text = Vec::new();
        for shape in output.shapes { texts(&shape.shape, &mut text); }
        let (command, response) = result.unwrap();
        (command, response, text)
    }

    fn click(ctx: &egui::Context, state: &Value, online: bool, busy: bool) -> Option<Value> {
        let pos = frame(ctx, state, online, busy, vec![]).1.rect.center();
        let mut command = None;
        for pressed in [true, false] {
            command = frame(ctx, state, online, busy, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default()
            }]).0;
        }
        command
    }

    #[test]
    fn real_checkbox_click_only_changes_saved_enabled_setting() {
        let ctx = egui::Context::default();
        let mut saved = state(false);
        assert_eq!(click(&ctx, &saved, true, false), Some(json!({"command":"head_recenter_settings","settings":{"enabled":true}})));
        // Until acknowledged, clicks continue to use the receiver's saved OFF.
        assert_eq!(click(&ctx, &saved, true, false).unwrap()["settings"]["enabled"], true);
        saved["head_recenter"]["settings"]["enabled"] = json!(true);
        assert_eq!(click(&ctx, &saved, true, false).unwrap()["settings"]["enabled"], false);
    }

    #[test]
    fn offline_busy_and_missing_settings_cannot_emit_commands() {
        for (saved, online, busy) in [(state(false), false, false), (state(true), true, true), (Value::Null, true, false)] {
            let ctx = egui::Context::default();
            assert!(!frame(&ctx, &saved, online, busy, vec![]).1.enabled());
            assert!(click(&ctx, &saved, online, busy).is_none());
        }
    }

    #[test]
    fn rendered_progress_tracks_receiver_snapshot() {
        let ctx = egui::Context::default();
        let mut saved = state(true);
        saved["head_recenter"]["completed_cycles"] = json!(2);
        let text = frame(&ctx, &saved, true, false, vec![]).2;
        assert!(text.iter().any(|text| text == "Head sweeps 2/4"));
        saved["head_recenter"]["phase"] = json!("centering");
        let text = frame(&ctx, &saved, true, false, vec![]).2;
        assert!(text.iter().any(|text| text.starts_with("Centering")));
        saved["head_recenter"]["phase"] = json!("resuming");
        assert!(frame(&ctx, &saved, true, false, vec![]).2.iter().any(|text| text.starts_with("Resuming")));
        saved["head_recenter"]["phase"] = json!("return_forward");
        assert!(frame(&ctx, &saved, true, false, vec![]).2.iter().any(|text| text.starts_with("Return forward")));
        saved["head_recenter"]["phase"] = json!("cooldown");
        let text = frame(&ctx, &saved, true, false, vec![]).2;
        assert!(text.iter().any(|text| text == "Head-sweep gesture completed."));
        assert!(!text.iter().any(|text| text == "Center updated."));
        saved["head_recenter"]["pending_resume"] = json!(true);
        saved["head_recenter"]["resume_result"] = json!("Recentered; waiting for fresh driver confirmation");
        let text = frame(&ctx, &saved, true, false, vec![]).2;
        assert!(text.iter().any(|text| text.contains("before resuming")));
        assert!(text.iter().any(|text| text == "Recentered; waiting for fresh driver confirmation"));
    }
}
