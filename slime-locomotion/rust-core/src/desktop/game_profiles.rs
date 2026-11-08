//! Pre-launch game and movement profile selection.
use super::pages;
use eframe::egui::{self, RichText};
use serde_json::{json, Value};

#[derive(Default)]
pub struct GameProfiles {
    game: String,
    profile: String,
    initialized: bool,
}

pub enum Action {
    Apply(Value),
    ResetCenter,
    SensorCalibration,
}

impl GameProfiles {
    pub fn show(&mut self, ui: &mut egui::Ui, state: &Value, online: bool, busy: bool) -> Option<Action> {
        let saved = &state["game_profiles"];
        if !self.initialized && online {
            self.game = match saved["selected"].as_str() {
                Some("custom") => "custom",
                _ => "arizona_sunshine_legacy",
            }.into();
            self.profile = saved["associations"][&self.game]["profile_name"].as_str().unwrap_or("").into();
            self.initialized = true;
        }
        if self.game.is_empty() { self.game = "arizona_sunshine_legacy".into(); }
        let mut action = None;
        pages::card(ui, "Before you launch", |ui| {
            let previous_game = self.game.clone();
            egui::ComboBox::from_label("Game")
                .selected_text(if self.game == "custom" { "Other game" } else { "Arizona Sunshine VR Legacy (SteamVR)" })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.game, "arizona_sunshine_legacy".into(), "Arizona Sunshine VR Legacy (SteamVR)");
                    ui.selectable_value(&mut self.game, "custom".into(), "Other game");
                });
            if self.game != previous_game {
                self.profile = saved["associations"][&self.game]["profile_name"].as_str().unwrap_or("").into();
            }
            if self.game == "custom" {
                ui.small("Other games require manual SteamVR bindings. Game input compatibility has not been verified.");
            } else {
                ui.small("ArcaneVR supplies a SteamVR sliding movement and sprint binding for Steam app 342180. Game input has not been verified; standalone Quest is not supported by this binding.");
            }
            { let selected = saved["selected"].as_str().unwrap_or("");
                let label = match selected { "custom" => "Other game", "arizona_sunshine_legacy" => "Arizona Sunshine VR Legacy", "" => "None selected", _ => "Unknown" };
                ui.label(RichText::new(format!("Applied game: {label}")).strong());
            }
            ui.add_space(6.);
            let profiles = state["profiles"].as_array();
            if !self.profile.is_empty() && !profiles.is_some_and(|ps| ps.iter().any(|p| p["name"].as_str() == Some(self.profile.as_str()))) {
                self.profile.clear();
            }
            egui::ComboBox::from_label("Movement settings")
                .selected_text(if self.profile.is_empty() { "Current settings" } else { &self.profile })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.profile, String::new(), "Current settings");
                    if let Some(profiles) = profiles {
                        for profile in profiles {
                            if let Some(name) = profile["name"].as_str() {
                                ui.selectable_value(&mut self.profile, name.to_owned(), name);
                            }
                        }
                    }
                });
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(online && !busy, egui::Button::new("Apply and save game settings")).clicked() {
                    let mut command = json!({"command":"game_profile_apply", "game_id":self.game});
                    if !self.profile.is_empty() { command["profile_name"] = json!(self.profile); }
                    action = Some(Action::Apply(command));
                }
                if ui.add_enabled(online && !busy && saved["associations"][&self.game].is_object(), egui::Button::new("Use saved game settings")).clicked() {
                    action = Some(Action::Apply(json!({"command":"game_profile_restore", "game_id":self.game})));
                }
                if ui.add_enabled(online && !busy, egui::Button::new("Reset center · instant")).clicked() { action = Some(Action::ResetCenter); }
                if ui.add_enabled(online && !busy && (state["movement_engine"] == "body_tilt" || state["movement_engine"] == "body_tilt_steps"), egui::Button::new("Calibrate sensor movements")).clicked() { action = Some(Action::SensorCalibration); }
            });
            ui.small("Apply saves a snapshot for this game and stops movement. Reset center before starting; applying never starts game output.");
        });
        ui.add_space(10.);
        action
    }
}


