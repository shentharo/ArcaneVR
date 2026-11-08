//! Standalone USB tracker setup. The ArcaneVR receiver remains a separate process.
#![windows_subsystem = "windows"]
use arcane_bridge_core::{usb_setup::{self, Action, SetupState, UsbSetup}, wifi_profile::{self, Profile}, wifi_scan::WifiScan};
use eframe::egui::{self, Color32, RichText};
use std::time::Duration;
#[path = "../desktop/qa_capture.rs"] mod qa_capture;

struct App {
    usb: Option<UsbSetup>, port: String, ssid: String, password: String,
    remember: bool, auto_setup: bool, saved_ssid: Option<String>, notice: String,
    qa_state: Option<String>,
    scan: WifiScan,
}
impl App {
    fn new() -> Self {
        let qa = qa_capture::path().is_some();
        let args = std::env::args().collect::<Vec<_>>();
        let qa_state = if qa { args.windows(2).find(|x| x[0] == "--qa-setup-state").map(|x| x[1].clone()) } else { None };
        let (profile, notice) = if qa { (None, String::new()) } else {
            match wifi_profile::load() { Ok(profile) => (profile, String::new()), Err(e) => (None, format!("Could not load saved Wi-Fi: {e}")) }
        };
        let saved_ssid = profile.as_ref().map(|p| p.ssid.clone()).or_else(|| qa_state.as_ref().map(|_| "Example Wi-Fi".into()));
        let ssid = saved_ssid.clone().unwrap_or_default();
        let password = profile.as_ref().map(|p| p.password.clone()).unwrap_or_default();
        let auto_setup = profile.as_ref().map(|p| p.auto_setup).unwrap_or(true);
        let mut scan = WifiScan::default();
        if qa { scan.networks = vec!["Example Wi-Fi".into()]; } else { scan.start(); }
        Self { usb: if qa { None } else { Some(UsbSetup::new()) }, port: args.windows(2).find(|x| x[0] == "--usb-port").map(|x| x[1].clone()).unwrap_or_default(), ssid, password,
            remember: true, auto_setup, saved_ssid, notice, qa_state, scan }
    }
    fn send(&self, action: Action) { if let Some(usb) = &self.usb { let _ = usb.tx.send(action); } }
    fn save(&mut self) {
        if self.usb.is_none() { self.notice = "Screenshot preview: changes are disabled.".into(); return; }
        if let Err(e) = usb_setup::wifi_command(&self.ssid, &self.password) { self.notice = e; return; }
        if self.remember {
            let profile = Profile { ssid: self.ssid.clone(), password: self.password.clone(), auto_setup: self.auto_setup };
            match wifi_profile::save(&profile) {
                Ok(()) => { self.saved_ssid = Some(self.ssid.clone()); self.notice = "Wi-Fi saved securely for this Windows account.".into(); self.send(Action::ReloadProfile); }
                Err(e) => self.notice = format!("Could not save Wi-Fi: {e}"),
            }
        } else {
            match wifi_profile::forget() {
                Ok(()) => { self.saved_ssid = None; self.send(Action::ReloadProfile); self.notice = "Saved Wi-Fi removed. Connect a tracker in advanced settings to apply these details once.".into(); }
                Err(e) => self.notice = format!("Could not remove saved Wi-Fi: {e}"),
            }
        }
    }
    fn forget(&mut self) {
        if self.usb.is_none() { self.notice = "Screenshot preview: changes are disabled.".into(); return; }
        match wifi_profile::forget() {
            Ok(()) => { self.ssid.clear(); self.password.clear(); self.saved_ssid = None; self.notice = "Saved Wi-Fi forgotten.".into(); self.send(Action::ReloadProfile); }
            Err(e) => self.notice = format!("Could not forget Wi-Fi: {e}"),
        }
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        qa_capture::update(ctx);
        self.scan.poll();
        if let Some(usb) = &mut self.usb { usb.poll(); }
        let mut state = self.usb.as_ref().map(|u| u.state.clone()).unwrap_or_else(SetupState::default);
        if let Some(qa) = &self.qa_state {
            state.auto_enabled = true;
            if qa == "detected" || qa == "ready" { state.identified = true; state.connected = "Example USB".into(); }
            if qa == "ready" { state.address = Some("192.0.2.10".parse().unwrap()); }
        }
        if self.port.is_empty() && state.ports.len() == 1 { self.port = state.ports[0].0.clone(); }
        let mut networks = self.scan.networks.clone();
        for network in &state.networks { if !networks.contains(network) { networks.push(network.clone()); } }
        ctx.request_repaint_after(Duration::from_millis(100));
        egui::CentralPanel::default().show(ctx, |ui| egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(8.0);
            ui.heading(RichText::new("Tracker Wi-Fi Setup").size(28.0));
            ui.label("1. Save Wi-Fi once    2. Plug each tracker in    3. Ready");
            ui.label("Once configured, trackers reconnect after charging when powered on within Wi-Fi range.");
            ui.add_space(12.0);
            ui.group(|ui| {
                ui.set_min_width(ui.available_width() - 8.0);
                ui.heading("Your 2.4 GHz Wi-Fi");
                ui.add_space(7.0);
                ui.horizontal(|ui| { ui.label("Network name"); ui.add(egui::TextEdit::singleline(&mut self.ssid).desired_width(330.0));
                    if ui.add_enabled(self.usb.is_some() && !self.scan.busy, egui::Button::new("Scan Wi-Fi")).clicked() {
                        self.scan.start(); if state.identified { self.send(Action::Scan); }
                    }
                });
                egui::ComboBox::from_id_salt("ssid").width(330.0).selected_text("Choose network").show_ui(ui, |ui| for name in &networks { if ui.selectable_label(false, name).clicked() { self.ssid = name.clone(); } });
                if !self.scan.status.is_empty() { ui.small(&self.scan.status); }
                else if networks.is_empty() { ui.small("No networks found yet. You can enter the network name manually."); }
                ui.horizontal(|ui| { ui.label("Password        "); ui.add(egui::TextEdit::singleline(&mut self.password).password(true).desired_width(330.0)); });
                ui.checkbox(&mut self.remember, "Remember Wi-Fi securely on this Windows account");
                ui.add_enabled_ui(self.remember, |ui| { ui.checkbox(&mut self.auto_setup, "Automatically set up trackers when plugged in"); });
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    let label = if self.remember && self.auto_setup { "Save Wi-Fi & enable automatic setup" } else if self.remember { "Save Wi-Fi" } else { "Remove saved Wi-Fi" };
                    if ui.button(label).clicked() { self.save(); }
                    if ui.add_enabled(self.saved_ssid.is_some(), egui::Button::new("Forget network")).clicked() { self.forget(); }
                });
                if let Some(ssid) = &self.saved_ssid { ui.label(format!("Saved network: {ssid}")); }
                else { ui.label("No Wi-Fi saved yet."); }
            });
            ui.add_space(12.0);
            ui.heading("Tracker status");
            if let Some(ip) = state.address { ui.colored_label(Color32::LIGHT_GREEN, format!("Ready — tracker reports IP address {ip}")); }
            else if state.identified { ui.colored_label(Color32::LIGHT_BLUE, format!("Tracker identified on {}. Waiting for an IP address.", state.connected)); }
            else { ui.label("Plug in a powered tracker using a USB data cable."); }
            if let Some(id) = &state.device_id { ui.label(format!("Tracker: {id}")); }
            if !state.auto_status.is_empty() { ui.label(&state.auto_status); }
            if !state.message.is_empty() { ui.label(&state.message); }
            if state.accepted && state.address.is_none() { ui.label("Wi-Fi details accepted; waiting for the tracker to report an IP address."); }
            if !self.notice.is_empty() { ui.colored_label(Color32::YELLOW, &self.notice); }
            if self.usb.is_none() { ui.small("Screenshot preview: USB and saved profile access are disabled."); }
            ui.add_space(8.0);
            egui::CollapsingHeader::new("Advanced USB settings").show(ui, |ui| {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("port").width(220.0).selected_text(if self.port.is_empty() { "Select USB port" } else { &self.port })
                        .show_ui(ui, |ui| for (port, label) in &state.ports { ui.selectable_value(&mut self.port, port.clone(), format!("{port} — {label}")); });
                    if ui.button("Refresh").clicked() { self.send(Action::Refresh); }
                    if state.connected.is_empty() {
                        if ui.add_enabled(!self.port.is_empty() && self.usb.is_some(), egui::Button::new("Connect")).clicked() { self.send(Action::Connect(self.port.clone())); }
                    } else if ui.button("Disconnect").clicked() { self.send(Action::Disconnect); }
                });
                if ui.add_enabled(state.identified && !state.busy && !self.ssid.is_empty(), egui::Button::new("Apply shown Wi-Fi to connected tracker")).clicked() {
                    self.send(Action::Wifi { ssid: self.ssid.clone(), password: self.password.clone() });
                }
            });
        }));
    }
}
fn main() -> eframe::Result {
    let options = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_inner_size([760.0, 660.0]).with_min_inner_size([650.0, 560.0]), ..Default::default() };
    eframe::run_native("ArcaneVR Tracker Wi-Fi Setup", options, Box::new(|_| Ok(Box::new(App::new()))))
}
