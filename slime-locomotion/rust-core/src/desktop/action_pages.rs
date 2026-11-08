//! Native configuration pages; settings remain drafts until explicitly applied.
use super::pages::{self, Page, MUTED, ORANGE};
use eframe::egui::{self, RichText};
use serde_json::{json, Value};

#[derive(Default)]
pub struct ActionPages {
    draft: Value,
    dirty: bool,
    profile_name: String,
    profile_description: String,
    import_json: String,
    local_error: String,
}
fn slider(
    ui: &mut egui::Ui,
    group: &mut Value,
    key: &str,
    label: &str,
    range: std::ops::RangeInclusive<f64>,
    factor: f64,
) -> bool {
    let mut n = group[key].as_f64().unwrap_or(0.) * factor;
    let changed = ui
        .add(egui::Slider::new(&mut n, range).text(label))
        .changed();
    if changed {
        group[key] = json!(n / factor);
    }
    changed
}
fn check(ui: &mut egui::Ui, group: &mut Value, key: &str, label: &str) -> bool {
    let mut b = group[key].as_bool().unwrap_or(false);
    let changed = ui.checkbox(&mut b, label).changed();
    if changed {
        group[key] = json!(b);
    }
    changed
}
fn choice(
    ui: &mut egui::Ui,
    group: &mut Value,
    key: &str,
    label: &str,
    values: &[(&str, &str)],
) -> bool {
    let mut selected = group[key].as_str().unwrap_or_default().to_string();
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        egui::ComboBox::from_id_salt(key)
            .selected_text(
                values
                    .iter()
                    .find(|x| x.0 == selected)
                    .map(|x| x.1)
                    .unwrap_or(&selected),
            )
            .show_ui(ui, |ui| {
                for (value, label) in values {
                    changed |= ui
                        .selectable_value(&mut selected, (*value).into(), *label)
                        .changed();
                }
            });
    });
    if changed {
        group[key] = json!(selected);
    }
    changed
}
impl ActionPages {
    pub fn movement_deadzone(&mut self, ui: &mut egui::Ui, state: &Value, online: bool, busy: bool, details: bool) -> Option<Value> {
        let saved = &state["actions"]["settings"];
        if !self.dirty && saved.is_object() { self.draft = saved.clone(); }
        if !self.draft.is_object() { ui.label("Waiting for direction settings"); return None; }
        self.dirty |= super::chest_tuning::target(ui, &mut self.draft["direction"], &saved["direction"], &state["actions"]["preview"], state["roles"]["torso"]["fresh"] == true, details);
        if details {
            ui.label(if self.dirty { "Unsaved deadzone" } else { "Drag orange points to adjust deadzone" });
        }
        let mut command = None;
        ui.horizontal(|ui| {
            if ui.add_enabled(online && !busy && self.dirty, egui::Button::new("Apply & stop").fill(ORANGE)).clicked() {
                command = Some(json!({"command":"actions_settings","settings":self.draft}));
                self.dirty = false;
            }
            if self.dirty && ui.button("Discard").clicked() { self.draft = saved.clone(); self.dirty = false; }
        });
        command
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        state: &Value,
        page: Page,
        online: bool,
        busy: bool,
    ) -> Option<Value> {
        let saved = &state["actions"]["settings"];
        if !self.dirty && saved.is_object() {
            self.draft = saved.clone();
        }
        if !self.draft.is_object() {
            pages::card(ui, page.name(), |ui| {
                ui.label("Waiting for action settings from the receiver.");
            });
            return None;
        }
        let group = match page {
            Page::Direction => "direction",
            Page::Jump => "jump",
            Page::Crouch => "crouch",
            Page::Turn => "turn",
            _ => return None,
        };
        let mut command = None;
        if matches!(page,Page::Crouch|Page::Turn) && state["hmd_pose"]["fresh"]!=true {
            ui.colored_label(ORANGE,if page==Page::Crouch {"No headset height: connect the Quest to SteamVR to preview crouching. A chest IMU cannot measure standing height."} else {"No headset orientation: connect the Quest to SteamVR to preview head-controlled turning. Physical chest heading still turns the character."});
        }
        let column_count=if page == Page::Direction || ui.available_width()<850. {1}else{2};
        ui.columns(column_count, |cols| {
            pages::card(&mut cols[0], &format!("{} Setup", page.name()), |ui| {
                ui.set_enabled(!matches!(page, Page::Crouch | Page::Turn) || state["hmd_pose"]["fresh"] == true);
                ui.add_space(14.);
                if ui
                    .add_enabled(
                        online && !busy && self.dirty,
                        egui::Button::new(RichText::new("APPLY & STOP").strong()).fill(ORANGE),
                    )
                    .clicked()
                {
                    command = Some(json!({"command":"actions_settings","settings":self.draft}));
                    self.dirty = false;
                }
                if ui.button("Discard draft").clicked() {
                    self.dirty = false;
                    self.draft = saved.clone();
                }
                ui.small(if self.dirty {
                    "Unsaved draft. Apply to preview. Sensitivity and threshold changes keep your center; mounting changes require Reset Center."
                } else {
                    "Showing receiver settings. Errors appear in the status bar."
                });
                ui.separator();
                let v = &mut self.draft[group];
                if page==Page::Direction && state["movement_engine"]!="chest_heading_steps" {
                    ui.label("Direction is active for lean movement.");
                } else { self.dirty |= check(ui, v, "enabled", "Enabled"); }
                match page {
                    Page::Direction => {
                        ui.label("Source: assigned chest orientation");
                        self.dirty |= super::chest_tuning::reference_controls(ui, v, state["hmd_pose"]["fresh"] == true);
                        self.dirty |= slider(ui, v, "exponent", "Response exponent", 0.1..=5., 1.);
                        ui.separator();
                        self.dirty |= check(ui, v, "forward_only", "Forward only");
                        self.dirty |= check(ui, v, "strafe_enabled", "Enable strafing");
                        self.dirty |= check(ui, v, "sprint_full_forward", "Sprint at full forward lean");
                        self.dirty |= slider(ui, v, "max_output", "Maximum stick output", 0.05..=1.0, 1.);
                        if v["decouple"] == true {
                            ui.add_enabled_ui(state["hmd_pose"]["fresh"] == true, |ui| {
                                self.dirty |= slider(ui, v, "decouple_strength", "Decoupling strength", 0.0..=1., 1.);
                            });
                        }
                    }
                    Page::Jump => {
                        if ui.add_enabled(online && !busy && !self.dirty, egui::Button::new("LEARN JUMP").fill(ORANGE)).clicked() { command=Some(json!({"command":"jump_learn_start"})); }
                        ui.small("Uses the saved source. Apply or discard changes before learning.");
                        self.dirty |= choice(
                            ui,
                            v,
                            "source",
                            "Source",
                            &[
                                ("height", "Headset height"),
                                ("acceleration", "Chest acceleration"),
                            ],
                        );
                        self.dirty |= choice(
                            ui,
                            v,
                            "accel_axis",
                            "Acceleration axis",
                            &[
                                ("magnitude", "Magnitude"),
                                ("x", "X"),
                                ("-x", "−X"),
                                ("y", "Y"),
                                ("-y", "−Y"),
                                ("z", "Z"),
                                ("-z", "−Z"),
                            ],
                        );
                        ui.small(if v["source"] == "height" {
                            "Threshold and release: metres above recent height baseline."
                        } else {
                            "Threshold and release: device-frame m/s²."
                        });
                        let height = v["source"] == "height";
                        self.dirty |= slider(
                            ui,
                            v,
                            "threshold",
                            if height {
                                "Trigger height cm"
                            } else {
                                "Trigger acceleration"
                            },
                            if height { 1.0..=100. } else { 0.01..=100. },
                            if height { 100. } else { 1. },
                        );
                        self.dirty |= slider(
                            ui,
                            v,
                            "release",
                            if height {
                                "Release height cm"
                            } else {
                                "Release acceleration"
                            },
                            if height { 0.0..=100. } else { 0.0..=100. },
                            if height { 100. } else { 1. },
                        );
                        self.dirty |= slider(
                            ui,
                            v,
                            "height_window_seconds",
                            "Height window ms",
                            10.0..=2000.,
                            1000.,
                        );
                        self.dirty |= slider(
                            ui,
                            v,
                            "accel_window_seconds",
                            "Acceleration window ms",
                            10.0..=2000.,
                            1000.,
                        );
                        self.dirty |= slider(
                            ui,
                            v,
                            "angle_compensation_percent",
                            "Angle compensation %",
                            0.0..=100.,
                            1.,
                        );
                        self.dirty |= choice(
                            ui,
                            v,
                            "compensation_axis",
                            "Compensation axis",
                            &[("yaw", "Yaw"), ("pitch", "Pitch"), ("roll", "Roll")],
                        );
                        self.dirty |= slider(
                            ui,
                            v,
                            "suppress_threshold",
                            "Suppression threshold",
                            0.01..=20.,
                            1.,
                        );
                        self.dirty |= slider(
                            ui,
                            v,
                            "pulse_seconds",
                            "Button pulse ms",
                            20.0..=2000.,
                            1000.,
                        );
                        self.dirty |=
                            slider(ui, v, "cooldown_seconds", "Cooldown ms", 0.0..=5000., 1000.);
                    }
                    Page::Crouch => {
                        if ui.add_enabled(online && !busy && !self.dirty && state["hmd_pose"]["fresh"]==true, egui::Button::new("LEARN CROUCH").fill(ORANGE)).clicked() {command=Some(json!({"command":"crouch_learn_start"}));}
                        ui.small("Stand upright, then crouch and hold twice. Apply or discard drafts first. Learning uses headset height.");
                        ui.label("Source: tracked headset height");
                        self.dirty |= slider(ui, v, "enter", "Crouch below cm", 5.0..=300., 100.);
                        self.dirty |= slider(ui, v, "exit", "Stand above cm", 5.0..=300., 100.);
                        self.dirty |= slider(
                            ui,
                            v,
                            "height_correction",
                            "Height correction cm",
                            -200.0..=200.,
                            100.,
                        );
                        self.dirty |= check(ui, v, "use_reset_offset", "Use reset height offset");
                        self.dirty |= slider(
                            ui,
                            v,
                            "reference_height",
                            "Reference standing height cm",
                            5.0..=300.,
                            100.,
                        );
                        self.dirty |= choice(
                            ui,
                            v,
                            "mode",
                            "Button mode",
                            &[
                                ("hold", "Hold while crouching"),
                                ("pulse", "Pulse on crouch"),
                            ],
                        );
                        self.dirty |= slider(
                            ui,
                            v,
                            "pulse_seconds",
                            "Button pulse ms",
                            20.0..=2000.,
                            1000.,
                        );
                        self.dirty |=
                            slider(ui, v, "slowdown", "Crouched speed multiplier", 0.0..=1., 1.);
                    }
                    Page::Turn => {
                        ui.label("Source: headset orientation (yaw / pitch)");
                        self.dirty |= check(ui, v, "horizontal", "Horizontal turn");
                        self.dirty |= check(ui, v, "vertical", "Vertical turn");
                        self.dirty |= slider(
                            ui,
                            v,
                            "deadzone",
                            "Deadzone °",
                            0.0..=85.,
                            180. / std::f64::consts::PI,
                        );
                        self.dirty |= slider(
                            ui,
                            v,
                            "max_angle",
                            "Maximum angle °",
                            1.0..=180.,
                            180. / std::f64::consts::PI,
                        );
                        self.dirty |= slider(ui, v, "exponent", "Response exponent", 0.1..=5., 1.);
                        for (key, label) in [
                            ("yaw_sign", "Reverse horizontal"),
                            ("pitch_sign", "Reverse vertical"),
                        ] {
                            let mut reverse = v[key].as_f64().unwrap_or(1.) < 0.;
                            if ui.checkbox(&mut reverse, label).changed() {
                                v[key] = json!(if reverse { -1. } else { 1. });
                                self.dirty = true;
                            }
                        }
                    }
                    _ => {}
                }

            });
            if page != Page::Direction { pages::card(&mut cols[column_count-1], &format!("{} Preview", page.name()), |ui| {
                let preview = &state["actions"]["preview"];
                ui.colored_label(
                    ORANGE,
                    preview[format!("{group}_status")]
                        .as_str()
                        .unwrap_or("Waiting for preview"),
                );
                if matches!(page, Page::Direction | Page::Turn) {
                    let key = if page == Page::Turn {
                        "turn_xy"
                    } else {
                        "travel_xy"
                    };
                    let mut visual = state.clone();
                    visual["direction"]["preview_xy"] = preview[key].clone();
                    pages::xy(ui, &visual, ui.available_width().min(360.));
                } else {
                    let pressed = preview[if page == Page::Jump {
                        "jump_pressed"
                    } else {
                        "crouching"
                    }]
                    .as_bool()
                    .unwrap_or(false);
                    ui.add_space(30.);
                    ui.label(
                        RichText::new(if pressed { "ACTIVE" } else { "RELEASED" })
                            .size(38.)
                            .color(if pressed { ORANGE } else { MUTED }),
                    );
                    ui.add_space(30.);
                    if page == Page::Crouch {
                        ui.label(format!(
                            "Speed multiplier {:.2}×",
                            pages::number(&preview["speed_multiplier"])
                        ));
                    }
                }
                ui.separator();
                ui.label("Preview runs without arming.");
                if state["steamvr"]["protocol"].as_u64().unwrap_or(1) < 2 {
                    ui.colored_label(
                        ORANGE,
                        "Driver update required for jump, crouch and turn output (protocol 2).",
                    );
                } else {
                    ui.small("Protocol 2 action output is available when START is enabled.");
                }
                if ui
                    .add_enabled(online && !busy, egui::Button::new("RESET CENTER · instant"))
                    .clicked()
                {
                    command = Some(json!({"command":"calibrate"}));
                }
            }); }
        });
        command
    }
    pub fn profiles(&mut self, ui: &mut egui::Ui, state: &Value, online: bool) -> Option<Value> {
        let mut command = None;
        let column_count=if ui.available_width()<850. {1}else{2};
        ui.columns(column_count,|cols|{
            pages::card(&mut cols[0],"Save Profile",|ui|{ui.label("Name");ui.text_edit_singleline(&mut self.profile_name);ui.label("Description");ui.text_edit_multiline(&mut self.profile_description);if ui.add_enabled(online&&!self.profile_name.trim().is_empty(),egui::Button::new("Save current settings").fill(ORANGE)).clicked(){command=Some(json!({"command":"profile_save","name":self.profile_name.trim(),"description":self.profile_description.trim()}));}ui.small("Profiles store configuration. Hardware assignments remain local.");});
            pages::card(&mut cols[column_count-1],"Saved Profiles",|ui|{if let Some(profiles)=state["profiles"].as_array(){for profile in profiles {let name=profile["name"].as_str().unwrap_or("Unnamed");ui.label(RichText::new(name).strong());ui.label(profile["description"].as_str().unwrap_or_default());ui.horizontal(|ui|{if ui.add_enabled(online,egui::Button::new("Load & stop")).clicked(){command=Some(json!({"command":"profile_load","name":name}));}if ui.button("Copy JSON").clicked(){ui.ctx().copy_text(serde_json::to_string_pretty(profile).unwrap_or_default());}if ui.add_enabled(online,egui::Button::new("Delete")).clicked(){command=Some(json!({"command":"profile_delete","name":name}));}});ui.separator();}}else{ui.label("No saved profiles received.");}});
        });
        ui.add_space(12.);
        pages::card(ui, "Import ArcaneVR Profile JSON", |ui| {
            ui.small("Import preserves local SlimeVR assignments and mounting.");
            ui.add(
                egui::TextEdit::multiline(&mut self.import_json)
                    .desired_rows(5)
                    .desired_width(ui.available_width()),
            );
            if ui
                .add_enabled(
                    online && !self.import_json.trim().is_empty(),
                    egui::Button::new("Import ArcaneVR profile"),
                )
                .clicked()
            {
                match serde_json::from_str::<Value>(&self.import_json) {
                    Ok(profile) if profile.is_object() => {
                        command = Some(json!({"command":"profile_import","profile":profile}));
                        self.local_error.clear();
                    }
                    _ => self.local_error = "Paste a valid profile JSON object.".into(),
                }
            }
            ui.colored_label(ORANGE, &self.local_error);
        });
        command
    }
}

