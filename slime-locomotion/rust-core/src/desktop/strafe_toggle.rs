//! Test-page control for the receiver's saved lateral-lean gate.
use eframe::egui;
use serde_json::{json, Value};

pub const LABEL: &str = "Enable leaning to strafe";

pub fn show(ui: &mut egui::Ui, state: &Value, online: bool, busy: bool) -> Option<Value> {
    control(ui, state, online, busy).0
}

fn control(ui: &mut egui::Ui, state: &Value, online: bool, busy: bool) -> (Option<Value>, egui::Response) {
    let saved = &state["actions"]["settings"];
    let direction = &saved["direction"];
    // Each frame reads the receiver snapshot. An unacknowledged click must not
    // become a local draft or overwrite settings received since the last frame.
    let gate = mode_gate(state);
    let heading = state["movement_engine"] == "chest_heading_steps";
    let mut enabled = displayed_checked(state);
    let ready = online && !busy && (heading || direction["enabled"] == true)
        && gate.is_some_and(|gate| direction[gate].is_boolean());
    let response = ui.add_enabled(ready, egui::Checkbox::new(&mut enabled, LABEL));
    let command = if response.changed() {
        let mut settings = saved.clone();
        settings["direction"][gate.expect("a changed checkbox has a valid mode gate")] = json!(enabled);
        Some(json!({"command":"actions_settings", "settings":settings}))
    } else { None };
    if heading {
        ui.small("Feet control forward speed; leaning left/right adds strafing while stepping.");
        if !direction["heading_strafe_enabled"].is_boolean() {
            ui.small("Waiting for saved leaning-to-strafe settings.");
        }
    } else if direction["enabled"] == false {
        ui.small("Enable Direction before changing leaning to strafe.");
    } else if gate.is_none() || !direction["strafe_enabled"].is_boolean() {
        ui.small("Waiting for saved lean movement settings.");
    }
    (command, response)
}

fn mode_gate(state: &Value) -> Option<&'static str> {
    match state["movement_engine"].as_str() {
        Some("body_tilt" | "body_tilt_steps") => Some("strafe_enabled"),
        Some("chest_heading_steps") => Some("heading_strafe_enabled"),
        _ => None,
    }
}

fn displayed_checked(state: &Value) -> bool {
    let direction = &state["actions"]["settings"]["direction"];
    (state["movement_engine"] == "chest_heading_steps" || direction["enabled"] == true)
        && mode_gate(state).is_some_and(|gate| direction[gate] == true)
}

pub fn chest_walking_hint(state: &Value) -> &'static str {
    if state["movement_engine"] == "chest_heading_steps" && displayed_checked(state) {
        "Feet control forward speed; leaning left/right adds strafing while stepping."
    } else if displayed_checked(state) {
        "The crosshair always shows lean. Sideways lean strafes without walking."
    } else {
        "The crosshair always shows lean. Leaning to strafe is off."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(count: u64, mode: &str, enabled: bool) -> Value {
        json!({"sensor_layout":{"count":count},"movement_engine":mode,
            "actions":{"settings":{"direction":{"enabled":true,"strafe_enabled":enabled,
                "learned_right":[0.2,0.3,0.9],"deadzone":0.17},"jump":{"enabled":true},
                "crouch":{"source":"headset"},"turn":{"enabled":false}}},
            "roles":{"torso":{"id":"chest"}},"mounting_calibration":{"valid":true}})
    }

    fn frame(ctx: &egui::Context, state: &Value, online: bool, busy: bool, events: Vec<egui::Event>) -> (Option<Value>, egui::Response) {
        let mut result = None;
        let _ = ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600., 250.))),
            events, ..Default::default()
        }, |ctx| { egui::CentralPanel::default().show(ctx, |ui| { result = Some(control(ui, state, online, busy)); }); });
        result.unwrap()
    }

    fn click(ctx: &egui::Context, state: &Value, online: bool, busy: bool) -> Option<Value> {
        let (_, response) = frame(ctx, state, online, busy, vec![]);
        let pos = response.rect.center();
        let mut result = None;
        for pressed in [true, false] {
            result = frame(ctx, state, online, busy, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default()
            }]).0;
        }
        result
    }

    #[test]
    fn real_checkbox_click_changes_only_saved_gate_for_all_sensor_layouts() {
        for count in [1, 3, 5] {
            for mode in ["body_tilt", "body_tilt_steps"] {
                for enabled in [false, true] {
                    let ctx = egui::Context::default();
                    let saved = state(count, mode, enabled);
                    assert_eq!(displayed_checked(&saved), enabled);
                    let command = click(&ctx, &saved, true, false).unwrap();
                    let mut expected = saved["actions"]["settings"].clone();
                    expected["direction"]["strafe_enabled"] = json!(!enabled);
                    assert_eq!(command, json!({"command":"actions_settings","settings":expected}));
                    assert_eq!(saved["actions"]["settings"]["direction"]["strafe_enabled"], enabled);
                }
            }
        }
    }

    #[test]
    fn heading_checkbox_changes_only_heading_gate_independently_of_legacy_direction_settings() {
        for count in [3, 5] {
            for enabled in [false, true] {
                for legacy_strafe in [false, true] {
                    for legacy_direction in [false, true] {
                        let ctx = egui::Context::default();
                        let mut saved = state(count, "chest_heading_steps", legacy_strafe);
                        saved["actions"]["settings"]["direction"]["enabled"] = json!(legacy_direction);
                        saved["actions"]["settings"]["direction"]["heading_strafe_enabled"] = json!(enabled);
                        assert_eq!(displayed_checked(&saved), enabled);
                        assert!(frame(&ctx, &saved, true, false, vec![]).1.enabled());
                        let command = click(&ctx, &saved, true, false).unwrap();
                        let mut expected = saved["actions"]["settings"].clone();
                        expected["direction"]["heading_strafe_enabled"] = json!(!enabled);
                        assert_eq!(command, json!({"command":"actions_settings", "settings":expected}));
                        saved["actions"]["settings"] = command["settings"].clone();
                        assert_eq!(displayed_checked(&saved), !enabled);
                    }
                }
            }
        }
    }

    #[test]
    fn disabled_and_incomplete_states_never_send_a_settings_command() {
        let saved = state(3, "body_tilt_steps", true);
        let mut heading = saved.clone(); heading["movement_engine"] = json!("chest_heading_steps");
        let mut no_direction = saved.clone(); no_direction["actions"]["settings"]["direction"]["enabled"] = json!(false);
        let mut missing_gate = saved.clone(); missing_gate["actions"]["settings"]["direction"].as_object_mut().unwrap().remove("strafe_enabled");
        let mut heading_saved = heading.clone(); heading_saved["actions"]["settings"]["direction"]["heading_strafe_enabled"] = json!(true);
        let mut unknown = saved.clone(); unknown["movement_engine"] = json!("unrecognized");
        assert!(!displayed_checked(&heading));
        assert!(!displayed_checked(&no_direction));
        assert_eq!(heading["actions"]["settings"]["direction"]["strafe_enabled"], true);
        assert_eq!(no_direction["actions"]["settings"]["direction"]["strafe_enabled"], true);
        for (s, online, busy) in [(&saved, false, false), (&saved, true, true), (&heading, true, false),
            (&heading_saved, false, false), (&heading_saved, true, true), (&unknown, true, false),
            (&no_direction, true, false), (&missing_gate, true, false), (&Value::Null, true, false)] {
            let ctx = egui::Context::default();
            assert!(!frame(&ctx, s, online, busy, vec![]).1.enabled());
            assert!(click(&ctx, s, online, busy).is_none());
        }
    }

    #[test]
    fn each_click_uses_latest_saved_gate_and_settings_without_a_stale_draft() {
        let ctx = egui::Context::default();
        let mut saved = state(1, "body_tilt", false);
        let first = click(&ctx, &saved, true, false).unwrap();
        // Until acknowledged, another click still requests ON from saved OFF.
        assert_eq!(click(&ctx, &saved, true, false).unwrap(), first);
        saved["actions"]["settings"] = first["settings"].clone();
        saved["actions"]["settings"]["direction"]["deadzone"] = json!(0.28);
        let next = click(&ctx, &saved, true, false).unwrap();
        assert_eq!(next["settings"]["direction"]["strafe_enabled"], false);
        assert_eq!(next["settings"]["direction"]["deadzone"], 0.28);
        assert!(chest_walking_hint(&saved).contains("strafes without walking"));
        saved["actions"]["settings"] = next["settings"].clone();
        assert!(chest_walking_hint(&saved).contains("is off"));
    }
}
