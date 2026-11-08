//! Saved network first USB setup. Closing the window leaves automatic setup running.
use arcane_bridge_core::{
    usb_setup::{Action, UsbSetup},
    wifi_profile::{self, Profile},
    wifi_scan::WifiScan,
};
use eframe::egui::{self, Color32, RichText};
use serde_json::Value;

fn tracker_live(state: &arcane_bridge_core::usb_setup::SetupState, backend: &Value, online: bool) -> bool {
    online && backend["trackers"].as_array().is_some_and(|list| list.iter().any(|t| {
        let identity_matches = state.device_id.as_deref().zip(t["mac"].as_str()).is_some_and(|(a,b)| a.eq_ignore_ascii_case(b));
        let address_matches = state.address.is_some_and(|ip| t["id"].as_str().is_some_and(|id| id.starts_with(&format!("{ip}:"))));
        (identity_matches || (state.device_id.is_none() && address_matches) || (t["mac"].as_str().is_none() && address_matches))
            && t["age_ms"].as_f64().is_some_and(|age| (0.0..1000.0).contains(&age))
    }))
}
fn tracker_status(state: &arcane_bridge_core::usb_setup::SetupState, live: bool) -> (&'static str, &'static str) {
    if live { ("Connected · receiving movement", "This tracker is ready. You can unplug USB and connect the next tracker.") }
    else if state.busy { ("Setting up tracker", "Keep the USB cable connected while setup finishes.") }
    else if state.address.is_some() { ("Waiting for sensor data", "A Wi-Fi address was reported. Keep the tracker powered on while ArcaneVR checks the connection.") }
    else if state.accepted { ("Joining Wi-Fi", "The tracker accepted the settings. Waiting for its network address.") }
    else if state.identified { ("Tracker detected", "Save your 2.4 GHz network below to enable automatic setup. Check USB details if setup has stopped.") }
    else if !state.connected.is_empty() { ("Identifying tracker", "Keep the tracker connected with a USB data cable.") }
    else { ("Connect a tracker", "Plug in a powered SlimeVR tracker using a USB data cable.") }
}

pub struct UsbWizard {
    ssid: String,
    password: String,
    auto_setup: bool,
    notice: String,
    loaded: bool,
    port: String,
    scan: WifiScan,
}
impl Default for UsbWizard {
    fn default() -> Self {
        Self {
            ssid: String::new(),
            password: String::new(),
            auto_setup: true,
            notice: String::new(),
            loaded: false,
            port: String::new(),
            scan: WifiScan::default(),
        }
    }
}
impl UsbWizard {
    fn clear_password(&mut self) {
        let mut bytes = std::mem::take(&mut self.password).into_bytes();
        bytes.fill(0);
        std::hint::black_box(&bytes);
    }

    pub fn closed(&mut self) {
        self.clear_password();
        self.ssid.clear();
        self.notice.clear();
        self.loaded = false;
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        open: &mut bool,
        usb: &UsbSetup,
        backend: &Value,
        online: bool,
        qa: bool,
    ) {
        if !self.loaded {
            self.loaded = true;
            if qa { self.scan.networks = vec!["Example Wi-Fi".into()]; }
            else { self.scan.start(); }
            if !qa {
                match wifi_profile::load() {
                    Ok(Some(profile)) => {
                        self.ssid = profile.ssid;
                        self.password = profile.password;
                        self.auto_setup = profile.auto_setup;
                        self.notice = "Saved network loaded. Plug in a tracker to continue.".into();
                    }
                    Ok(None) => {}
                    Err(error) => self.notice = error,
                }
            }
        }
        self.scan.poll();
        if self.scan.busy { ctx.request_repaint_after(std::time::Duration::from_millis(100)); }
        let state = &usb.state;
        let mut networks = self.scan.networks.clone();
        for network in &state.networks { if !networks.contains(network) { networks.push(network.clone()); } }
        let mut done = false;
        egui::Window::new("Add sensors")
            .open(open).default_height(650.).default_width(520.).max_width((ctx.content_rect().width()-32.).clamp(240., 580.)).max_height((ctx.content_rect().height()-64.).max(180.)).vscroll(true)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .resizable(false).collapsible(false)
            .show(ctx, |ui| {
                ui.heading("Connect your trackers");
                ui.label("Save a network once, then connect each tracker by USB.");
                ui.add_space(8.);
                let live = tracker_live(state, backend, online);
                let (title, detail) = tracker_status(state, live);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.colored_label(if live {Color32::LIGHT_GREEN} else {Color32::LIGHT_BLUE}, RichText::new(title).strong());
                    ui.label(detail);
                    if let Some(id)=&state.device_id {ui.small(format!("Tracker {id}"));}
                });
                ui.add_space(8.);
                ui.strong("1  ·  Your Wi-Fi network");
                ui.add_space(8.);
                ui.label("2.4 GHz network name (SSID)");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.ssid).desired_width((ui.available_width()-110.).max(100.)).hint_text("Your Wi-Fi network"));
                    if ui.add_enabled(!qa && !self.scan.busy, egui::Button::new("Scan Wi-Fi")).clicked() {
                        self.scan.start();
                        if state.identified { let _ = usb.tx.send(Action::Scan); }
                    }
                });
                egui::ComboBox::from_id_salt("wizard-wifi-network").selected_text(if self.ssid.is_empty() { "Choose a nearby network" } else { &self.ssid }).width(ui.available_width().min(400.)).show_ui(ui, |ui| {
                    for name in &networks { ui.selectable_value(&mut self.ssid, name.clone(), name); }
                });
                if !self.scan.status.is_empty() { ui.small(&self.scan.status); }
                else if networks.is_empty() { ui.small("No networks found yet. You can enter the network name manually."); }
                ui.label("Wi-Fi password");
                ui.add(egui::TextEdit::singleline(&mut self.password).password(true).desired_width(f32::INFINITY).hint_text("Network password"));
                ui.checkbox(&mut self.auto_setup, "Automatically set up trackers when connected by USB");
                let label = if self.auto_setup { "Save Wi-Fi & enable auto setup" } else { "Save Wi-Fi" };
                if ui.add_enabled(!qa && !state.busy && arcane_bridge_core::usb_setup::wifi_command(&self.ssid, &self.password).is_ok(), egui::Button::new(RichText::new(label).strong()).min_size(egui::vec2(ui.available_width().min(320.), 36.))).clicked() {
                    let profile = Profile { ssid: self.ssid.clone(), password: self.password.clone(), auto_setup: self.auto_setup };
                    match wifi_profile::save(&profile) {
                        Ok(()) => {
                            let _ = usb.tx.send(Action::ReloadProfile);
                            self.notice = if self.auto_setup { "Wi-Fi saved. Plug in a powered tracker with a USB data cable." } else { "Wi-Fi saved. Automatic USB setup is paused." }.into();
                        }
                        Err(error) => self.notice = error,
                    }
                }
                ui.small("Saved securely for this Windows user. The password stays masked.");
                if !self.notice.is_empty() { ui.colored_label(Color32::LIGHT_BLUE, &self.notice); }
                ui.add_space(6.);
                ui.separator();
                ui.strong("2  ·  Trackers on your network");
                let trackers = backend["trackers"].as_array();
                let live_count = if online { trackers.map(|list| list.iter().filter(|t| t["age_ms"].as_f64().is_some_and(|age| (0.0..1000.0).contains(&age))).count()).unwrap_or(0) } else { 0 };
                ui.label(format!("{live_count} sensors receiving live data"));
                if !online { ui.colored_label(Color32::YELLOW, "Receiver unavailable. Live connection cannot be verified."); }
                if !state.completed.is_empty() {
                    egui::CollapsingHeader::new("Recently configured trackers").show(ui, |ui| {
                        for (id, ip) in state.completed.iter().rev().take(8) {
                            let prefix = format!("{ip}:");
                            let fresh = online && trackers.is_some_and(|list| list.iter().any(|tracker| {
                                (tracker["mac"].as_str() == Some(id.as_str()) || tracker["id"].as_str().is_some_and(|key| key.starts_with(&prefix)))
                                    && tracker["age_ms"].as_f64().is_some_and(|age| age < 1000.)
                            }));
                            ui.label(format!("{id} · {ip} · {}", if fresh { "live" } else { "waiting for packets" }));
                        }
                    });
                }
                if !online { ui.small("Receiver is not connected yet."); }
                ui.add_space(6.);

                egui::CollapsingHeader::new("Advanced · USB ports").show(ui, |ui| {
                    ui.add_enabled_ui(!qa, |ui| {
                        egui::ComboBox::from_id_salt("wizard-usb-port").selected_text(if self.port.is_empty() { "Select USB port" } else { &self.port }).width(ui.available_width().min(400.)).show_ui(ui, |ui| {
                            for (name, label) in &state.ports { ui.selectable_value(&mut self.port, name.clone(), format!("{name} — {label}")); }
                        });
                        ui.horizontal(|ui| {
                            if ui.button("Refresh ports").clicked() { let _ = usb.tx.send(Action::Refresh); }
                            if ui.add_enabled(!self.port.is_empty() && !state.busy, egui::Button::new("Connect selected")).clicked() { let _ = usb.tx.send(Action::Connect(self.port.clone())); }
                        });
                    });
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.add_enabled(!qa && !state.busy, egui::Button::new("Forget saved Wi-Fi")).clicked() {
                        match wifi_profile::forget() {
                            Ok(()) => {
                                self.clear_password(); self.ssid.clear(); self.auto_setup = true;
                                self.notice = "Saved Wi-Fi removed. Trackers keep their own network settings.".into();
                                let _ = usb.tx.send(Action::ReloadProfile);
                            }
                            Err(error) => self.notice = error,
                        }
                    }
                    if ui.button("Done").clicked() { done = true; }
                });
            });
        if done {
            *open = false;
        }
        if !*open {
            self.closed();
        }
    }
}
impl Drop for UsbWizard {
    fn drop(&mut self) {
        self.clear_password();
    }
}

#[cfg(test)]
mod tests {
 use super::*;
 use arcane_bridge_core::usb_setup::SetupState;
 use serde_json::json;
 #[test] fn live_tracker_overrides_old_timeout_and_rejects_stale_or_other_identity() {
  let mut state=SetupState::default(); state.address=Some("192.0.2.12".parse().unwrap());state.device_id=Some("AA:BB".into());state.auto_status="Timed out".into();
  let live=json!({"trackers":[{"id":"192.0.2.12:0","mac":"AA:BB","age_ms":10.}]});
  assert!(tracker_live(&state,&live,true));assert!(tracker_status(&state,true).0.starts_with("Connected"));
  assert!(!tracker_live(&state,&live,false));
  assert!(!tracker_live(&state,&json!({"trackers":[{"id":"192.0.2.12:0","mac":"CC:DD","age_ms":10.}]}),true));
  assert!(!tracker_live(&state,&json!({"trackers":[{"id":"192.0.2.12:0","mac":"AA:BB","age_ms":1500.}]}),true));
 }
}
