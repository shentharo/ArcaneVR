#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
//! Native Rust desktop interface and receiver in one process.
#[allow(dead_code)]
#[path = "arcane-slime-lab.rs"]
mod receiver;

#[path = "../desktop/chest_tuning.rs"]
mod chest_tuning;
#[path = "../desktop/game_profiles.rs"]
mod game_profiles;
#[path = "../desktop/home_dashboard.rs"]
mod home_dashboard;
#[path = "../desktop/action_pages.rs"]
mod action_pages;
#[path = "../desktop/body_setup.rs"]
mod body_setup;
#[path = "../desktop/locomotion_panel.rs"]
mod locomotion_panel;
#[path = "../desktop/overlay.rs"]
mod overlay;
#[path = "../desktop/pages.rs"]
mod pages;
#[path = "../desktop/startup.rs"]
mod startup;
#[path = "../desktop/test_room.rs"]
mod test_room;
#[path="../desktop/setup_new.rs"]
mod setup_new;
#[path="../desktop/profile_builder.rs"]
mod profile_builder;
#[path="../desktop/bevy_room.rs"]
mod bevy_room;
#[path="../desktop/sensor_calibration.rs"]
mod sensor_calibration;
#[path="../desktop/qa_capture.rs"]
mod qa_capture;
#[path = "../desktop/usb_wizard.rs"]
mod usb_wizard;
#[path = "../desktop/updater.rs"]
mod updater;
use arcane_bridge_core::usb_setup::{Action as UsbAction, UsbSetup};
use eframe::egui::{self, Color32, RichText};
use serde_json::{json, Value};
#[cfg(windows)]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    io::{Read, Write},
    net::TcpStream,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

fn usb_qa() -> bool {
    qa_capture::path().is_some() || std::env::args().any(|arg| arg == "--qa-usb-setup")
}

#[derive(Default, Clone)]
struct Status {
    state: Value,
    error: String,
    countdown: Option<u64>,
    online: bool,
}

fn request(port: u16, command: Option<&Value>) -> Result<Value, String> {
    let mut socket =
        TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(250))
            .map_err(|e| e.to_string())?;
    socket
        .set_read_timeout(Some(Duration::from_millis(500)))
        .map_err(|e| e.to_string())?;
    socket
        .set_write_timeout(Some(Duration::from_millis(500)))
        .map_err(|e| e.to_string())?;
    let body = command.map(Value::to_string).unwrap_or_default();
    let (method, path) = if command.is_some() {
        ("POST", "/api/command")
    } else {
        ("GET", "/api/state")
    };
    write!(socket, "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).map_err(|e| e.to_string())?;
    let mut response = String::new();
    socket
        .read_to_string(&mut response)
        .map_err(|e| e.to_string())?;
    let (header, body) = response
        .split_once("\r\n\r\n")
        .ok_or("Invalid receiver response")?;
    let value: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    if !header.starts_with("HTTP/1.1 200")
        || (command.is_some()
            && value
                .get("error")
                .and_then(Value::as_str)
                .is_some_and(|e| !e.is_empty()))
    {
        return Err(value["error"]
            .as_str()
            .unwrap_or("Receiver rejected command")
            .into());
    }
    Ok(value)
}

struct ArcaneVR {
    updater: updater::Updater,
    new_setup:bool,
    profile_builder:profile_builder::ProfileBuilder,
    new_test:bool,
    http_port:u16,
    bevy_room:bevy_room::BevyRoom,
    avatar_texture:Option<egui::TextureHandle>,
    shared: Arc<Mutex<Status>>,
    tx: mpsc::Sender<Value>,
    halt: Arc<AtomicBool>,
    backend: Option<thread::JoinHandle<()>>,
    worker: Option<thread::JoinHandle<()>>,
    selected: [String; 3],
    initialized: bool,
    choice: bool,
    axis: String,
    threshold: f64,
    gravity_input: bool,
    walking: bool,
    yaw: f64,
    history: Vec<f32>,
    selected_sensor: Option<String>,
    locomotion_panel: locomotion_panel::LocomotionPanel,
    action_pages: action_pages::ActionPages,
    chest_tuning: chest_tuning::ChestTuning,
    old_chest_main: bool,
    game_profiles: game_profiles::GameProfiles,
    startup: startup::Startup,
    test_room: test_room::TestRoom,
    overlays: overlay::OverlayUi,
    page: pages::Page,
    header_profile: String,
    test_settings_tab: usize,
    show_hints: bool,
    show_movement_preview: bool,
    debug: bool,
    network_open: bool,
    network_ips: [String; 3],
    usb: UsbSetup,
    setup_open: bool,
    usb_wizard: usb_wizard::UsbWizard,
}
impl ArcaneVR {
    fn new() -> Self {
        let halt = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Mutex::new(Status::default()));
        let mut args: Vec<String> = std::env::args().collect();
        let port = args
            .windows(2)
            .find(|p| p[0] == "--http-port")
            .and_then(|p| p[1].parse().ok())
            .unwrap_or(8811);
        if !args.iter().any(|a| a == "--config") {
            // Preserve the existing portable configuration and recordings.
            let config = std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .join("tracker-group.json");
            args.extend(["--config".into(), config.display().to_string()]);
        }
        let startup_args = args.clone();
        let stop = halt.clone();
        let errors = shared.clone();
        let backend = thread::spawn(move || {
            if let Err(e) = receiver::run(args, stop.clone()) {
                errors.lock().unwrap().error = format!("Receiver could not start: {e}. Close another running copy of ArcaneVR, then reopen.");
                stop.store(true, Ordering::Relaxed);
            }
        });
        let (tx, rx) = mpsc::channel::<Value>();
        let startup = startup::Startup::new(&startup_args, tx.clone());
        let stop = halt.clone();
        let output = shared.clone();
        let worker = thread::spawn(move || {
            let mut deadline = None;
            while !stop.load(Ordering::Relaxed) {
                for cmd in rx.try_iter() {
                    deadline = None;
                    let result = if cmd["command"] == "countdown" {
                        request(port, Some(&json!({"command":"stop"}))).map(|v| {
                            deadline = Some(Instant::now() + Duration::from_secs(3));
                            v
                        })
                    } else {
                        request(port, Some(&cmd))
                    };
                    output.lock().unwrap().error = result.err().unwrap_or_default();
                }
                if deadline.is_some_and(|d| Instant::now() >= d) {
                    deadline = None;
                    output.lock().unwrap().error =
                        request(port, Some(&json!({"command":"calibrate"})))
                            .err()
                            .unwrap_or_default();
                }
                let result = request(port, None);
                let mut status = output.lock().unwrap();
                status.countdown =
                    deadline.map(|d| d.saturating_duration_since(Instant::now()).as_secs() + 1);
                match result {
                    Ok(state) => {
                        status.state = state;
                        status.online = true;
                    }
                    Err(_) => {
                        status.online = false;
                        deadline = None;
                        status.countdown = None;
                    }
                }
                drop(status);
                thread::sleep(Duration::from_millis(70));
            }
        });
        Self {
            updater: updater::Updater::default(),
            shared,
            tx,
            halt,
            backend: Some(backend),
            worker: Some(worker),
            selected: Default::default(),
            initialized: false,
            choice: false,
            axis: "y".into(),
            threshold: 1.8,
            gravity_input: false,
            walking: false,
            yaw: 0.,
            history: vec![],
            selected_sensor: None,
            locomotion_panel: locomotion_panel::LocomotionPanel::default(),
            action_pages: action_pages::ActionPages::default(),
            chest_tuning: chest_tuning::ChestTuning::default(),
            old_chest_main: false,
            game_profiles: game_profiles::GameProfiles::default(),
            startup,
            profile_builder:profile_builder::ProfileBuilder::default(),
            new_setup:qa_capture::page().as_deref()!=Some("setup-old"),
            new_test:qa_capture::page().as_deref()!=Some("test-old"),
            http_port:port,
            bevy_room:bevy_room::BevyRoom::default(),
            avatar_texture:None,
            test_room: test_room::TestRoom::default(),
            overlays: overlay::OverlayUi::new(port),
            page:if matches!(qa_capture::page().as_deref(),Some("main" | "chest" | "chest-advanced")){pages::Page::Main}else if qa_capture::page().as_deref()==Some("devices"){pages::Page::Devices}else if qa_capture::page().as_deref()==Some("profiles"){pages::Page::Profiles}else if qa_capture::page().as_deref()==Some("direction"){pages::Page::Direction}else if qa_capture::page().as_deref()==Some("jump"){pages::Page::Jump}else if qa_capture::page().as_deref()==Some("crouch"){pages::Page::Crouch}else if qa_capture::page().as_deref()==Some("steps"){pages::Page::Steps}else if qa_capture::page().as_deref()==Some("updates"){pages::Page::MovementSetup}else if matches!(qa_capture::page().as_deref(),Some("settings" | "settings-preview")){pages::Page::Settings}else if qa_capture::page().as_deref()==Some("setup"){pages::Page::QuickSetup}else if qa_capture::page().is_some_and(|p|p.starts_with("test")){pages::Page::TestRoom}else{pages::Page::Home},
            header_profile: String::new(),
            test_settings_tab: match qa_capture::page().as_deref(){Some("test-steps")=>1,Some("test-jump")=>2,Some("test-crouch")=>3,Some("test-turn")=>4,_=>0},
            show_hints: true,
            show_movement_preview: matches!(qa_capture::page().as_deref(), Some("settings-preview" | "test-preview" | "test-compact")),
            debug: false,
            network_open: false,
            network_ips: {
                let plan = std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|p| p.join("sensor-network-plan.json")))
                    .and_then(|p| std::fs::read_to_string(p).ok())
                    .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                    .unwrap_or_default();
                ["left", "right", "torso"].map(|r| {
                    plan["targets"][r]["ip"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string()
                })
            },
            usb: if usb_qa() { UsbSetup::disabled() } else { UsbSetup::new() },
            setup_open: std::env::args().any(|arg| arg == "--usb-setup" || arg == "--qa-usb-setup") || qa_capture::page().as_deref() == Some("usb"),
            usb_wizard: usb_wizard::UsbWizard::default(),
        }
    }
    fn send(&self, value: Value) {
        let _ = self.tx.send(value);
    }
    fn switch(&mut self, command: &str, mode: &str) {
        self.send(json!({"command":"stop"}));
        self.send(json!({"command":command,"mode":mode}));
        self.walking = false;
    }
}
impl Drop for ArcaneVR {
    fn drop(&mut self) {
        self.overlays.stop();
        self.halt.store(true, Ordering::Relaxed);
        if let Some(handle) = self.backend.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}
impl eframe::App for ArcaneVR {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        #[cfg(windows)]
        apply_rounded_window(ctx, frame);
        ctx.request_repaint_after(Duration::from_millis(80));
        qa_capture::update(ctx);
        let skinned_home = self.page == pages::Page::Home;
        if qa_capture::path().is_none(){self.overlays.poll();}
        self.bevy_room.poll();
        if std::env::var_os("ARCANEVR_EMBED_QA_NAVIGATE").is_some() && qa_capture::path().is_some() {
            match ctx.cumulative_frame_nr() {
                60=>ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1100.,900.))),
                80=>self.page=pages::Page::Main,
                110=>self.page=pages::Page::TestRoom,
                _=>{},
            }
        }
        self.bevy_room.set_page_visible(
            (self.page == pages::Page::TestRoom && self.new_test)
            || (self.page == pages::Page::Home && self.bevy_room.sensor_calibration_open())
        );
        if !self.startup.poll(ctx).is_empty() {
            self.walking = false;
            self.test_room.stop();
        }
        let status = self.shared.lock().unwrap().clone();
        let s = &status.state;
        let crouch_learning=&s["crouch_learning"];
        if crouch_learning.is_object() {
            egui::Window::new("Learn Crouch").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER,egui::Vec2::ZERO).show(ctx,|ui| {
                if crouch_learning["active"] == true {
                    ui.heading(crouch_learning["text"].as_str().unwrap_or("Preparing"));
                    if let Some(n)=crouch_learning["countdown"].as_u64().filter(|n|*n>0) {ui.heading(n.to_string());}
                    if ui.button("Cancel").clicked() {self.send(json!({"command":"crouch_learn_cancel"}));}
                } else {
                    ui.label(crouch_learning["message"].as_str().unwrap_or("Crouch learning ended."));
                    if ui.button("OK").clicked() {self.send(json!({"command":"crouch_learn_dismiss"}));}
                }
            });
        }
        let jump_learning=&s["jump_learning"];
        if jump_learning.is_object() {
            egui::Window::new("Learn Jump").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER,egui::Vec2::ZERO).show(ctx,|ui| {
                if jump_learning["active"] == true {
                    ui.heading(jump_learning["text"].as_str().unwrap_or("Preparing"));
                    if let Some(n)=jump_learning["countdown"].as_u64().filter(|n|*n>0) {ui.heading(n.to_string());}
                    if ui.button("Cancel").clicked() {self.send(json!({"command":"jump_learn_cancel"}));}
                } else {
                    ui.label(jump_learning["message"].as_str().unwrap_or("Jump learning ended."));
                    if ui.button("OK").clicked() {self.send(json!({"command":"jump_learn_dismiss"}));}
                }
            });
        }
        if qa_capture::path().is_some() { self.bevy_room.open_qa_calibration(); }
        if self.bevy_room.sensor_calibration_open() && self.page != pages::Page::Home {
            self.page=pages::Page::Home;
            ctx.request_repaint();
        }
        if !self.bevy_room.sensor_calibration_open() {self.bevy_room.stop_calibration_preview();}
        self.usb.poll();
        let demo = s["mode"] == "demo";
        let busy = status.countdown.is_some();
        let roles = ["left", "right", "torso"];
        if status.online && !self.initialized && s["roles"].is_object() {
            for (i, role) in roles.iter().enumerate() {
                self.selected[i] = s["roles"][role]["id"].as_str().unwrap_or_default().into();
            }
            self.axis = s["axis"].as_str().unwrap_or("y").into();
            self.threshold = s["threshold"].as_f64().unwrap_or(1.8);
            self.gravity_input = s["acceleration_input_mode"] == "gravity-baseline";
            self.initialized = true;
            self.choice = true;
        }
        let assigned = roles.iter().enumerate().all(|(i, r)| {
            (s["sensor_layout"]["count"]==1&&i!=2) || (!self.selected[i].is_empty() && s["roles"][r]["id"].as_str() == Some(&self.selected[i]))
        });
        let ready =
            status.online && (demo || assigned || (s["sensor_layout"]["count"]==1&&!self.selected[2].is_empty())) && s["calibration_ready"] == true && self.choice;
        let driver_ready = demo
            || s["output_mode"] != "steamvr"
            || (s["steamvr"]["driver_connected"] == true
                && s["steamvr"]["hmd_ready"] == true
                && s["steamvr"]["driver_calibrated"] == true);
        let actions_output_ready = s["output_mode"] != "steamvr"
            || s["steamvr"]["protocol"].as_u64().unwrap_or(1) >= 2
            || (!(["jump", "crouch", "turn"]
                .iter()
                .any(|key| s["actions"]["settings"][key]["enabled"] == true))
                && s["locomotion"]["settings"]["sprint_enabled"] != true
                && s["movement_engine"] != "body_tilt"
                && s["movement_engine"] != "body_tilt_steps");
        use pages::Page;
        if self.page == Page::Settings {
            self.page = Page::TestRoom;
            self.show_movement_preview = true;
        }
        let show_steps = pages::shows_step_settings(s);
        if status.online && !show_steps {
            if self.test_settings_tab == 1 { self.test_settings_tab = 0; }
        }
        let mut install_update = false;
        if self.page != pages::Page::Home { egui::TopBottomPanel::top("title")
            .frame(
                egui::Frame::new()
                    .fill(pages::BG)
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6., 3.);
                if demo {
                    ui.colored_label(pages::ORANGE, "SYNTHETIC DEMO — game output disabled");
                }
                if let Some(n) = status.countdown {
                    ui.colored_label(pages::ORANGE, format!("Face forward and stand still · {n}"));
                }
                ui.horizontal_wrapped(|ui| {
                    for page in [pages::Page::Home, pages::Page::TestRoom, pages::Page::Settings] {
                        if self.page==pages::Page::Home {continue;}
                        let selected = self.page == page || (page == pages::Page::Settings && !matches!(self.page,pages::Page::Home|pages::Page::TestRoom));
                        if ui
                            .add_enabled(
                                !self.bevy_room.sensor_calibration_open(),
                                egui::Button::new(RichText::new(if page == Page::TestRoom { "Movement" } else { page.name() }).color(if selected {
                                    Color32::BLACK
                                } else {
                                    Color32::from_rgb(205, 208, 214)
                                }))
                                .fill(if selected { pages::ORANGE } else { pages::CARD })
                                .stroke(egui::Stroke::NONE).corner_radius(8)
                                .min_size(egui::vec2(74., 28.)),
                            )
                            .clicked()
                        {
                            if page == Page::Settings {
                                self.page = Page::TestRoom;
                                self.show_movement_preview = true;
                            } else {
                                self.page = page;
                            }
                        }
                    }
                    if self.page == Page::TestRoom {
                        let needs_center = s["actions"]["preview"]["calibrated"] != true;
                        let pulse = (ui.input(|i| i.time) * 3.0).sin() > 0.0;
                        let mut recenter = egui::Button::new("Recenter");
                        if needs_center && status.online && s["calibration_ready"] == true {
                            recenter = recenter.fill(if pulse { pages::ORANGE } else { pages::CARD });
                        }
                        if ui.add_enabled(status.online && !busy && s["calibration_ready"] == true, recenter).clicked() {
                            self.walking = false;
                            self.send(json!({"command":"calibrate"}));
                        }
                        let tilt = s["movement_engine"] == "body_tilt" || s["movement_engine"] == "body_tilt_steps";
                        if ui.add_enabled(status.online && !busy && tilt, egui::Button::new("Calibration")).clicked() {
                            self.walking = false;
                            self.send(json!({"command":"stop"}));
                            self.bevy_room.start_sensor_calibration(s);
                        }
                    }
                    let crosshair = egui::Button::new("").min_size(egui::vec2(32., 28.))
                        .fill(if self.show_movement_preview { pages::ORANGE } else { pages::CARD });
                    let response = ui.add(crosshair).on_hover_text("Toggle movement crosshair");
                    let center = response.rect.center();
                    let stroke = egui::Stroke::new(1.5, if self.show_movement_preview { Color32::BLACK } else { Color32::LIGHT_GRAY });
                    ui.painter().circle_stroke(center, 7., stroke);
                    ui.painter().line_segment([center - egui::vec2(10., 0.), center + egui::vec2(10., 0.)], stroke);
                    ui.painter().line_segment([center - egui::vec2(0., 10.), center + egui::vec2(0., 10.)], stroke);
                    if response.clicked() { self.show_movement_preview = !self.show_movement_preview; }
                    let enabled = s["armed"] == true;
                    let can_enable = ready && !busy && !self.bevy_room.sensor_calibration_open() && s["calibrated"] == true && driver_ready && actions_output_ready && s["locomotion"]["input_ready"] == true;
                    let movement_button = egui::Button::new("Game movement")
                        .fill(if enabled { pages::ORANGE } else { pages::CARD });
                    if ui.add_enabled(enabled || can_enable, movement_button).clicked() {
                        if !enabled { self.send(json!({"command":"arm"})); } else { self.walking=false; self.test_room.stop(); self.send(json!({"command":"stop"})); }
                    }
                    if ui.selectable_label(matches!(self.page, Page::QuickSetup | Page::Devices), "Sensors").clicked() { self.page = Page::QuickSetup; }
                    install_update = self.updater.show(ctx, ui, status.online && !busy && s["armed"] != true);
                    let drag_width = (ui.available_width() - 110.).max(0.);
                    let (_, drag) = ui.allocate_exact_size(egui::vec2(drag_width, 30.), egui::Sense::click_and_drag());
                    if drag.drag_started() { ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag); }
                    if ui.add_sized([36., 28.], egui::Button::new("−")).clicked() { ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)); }
                    if ui.add_sized([36., 28.], egui::Button::new("×")).clicked() { ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
                });
                if self.page != Page::Home {
                    ui.add_space(2.);
                    ui.add_enabled_ui(!self.bevy_room.sensor_calibration_open(), |ui| {
                        ui.horizontal_wrapped(|ui| {
                            for (label, tab) in [
                                ("Test", Page::TestRoom), ("Setup", Page::MovementSetup), ("Direction", Page::Direction), ("Steps", Page::Steps),
                                ("Jump", Page::Jump), ("Crouch", Page::Crouch),
                                ("Turn", Page::Turn),

                                ("Profiles", Page::GameProfiles),
                            ] {
                                ui.selectable_value(&mut self.page, tab, label);
                            }
                        });
                        if matches!(self.page, Page::QuickSetup | Page::Devices) {
                            ui.horizontal(|ui| {
                                ui.selectable_value(&mut self.page, Page::QuickSetup, "Sensor setup");
                                ui.selectable_value(&mut self.page, Page::Devices, "Devices");
                            });
                        } else if matches!(self.page, Page::GameProfiles | Page::Profiles) {
                            ui.horizontal(|ui| {
                                ui.selectable_value(&mut self.page, Page::GameProfiles, "Game profile");
                                ui.selectable_value(&mut self.page, Page::Profiles, "Saved profiles");
                            });
                        }
                    });
                }
            });
        }
        if install_update {
            let stopped = request(self.http_port, Some(&json!({"command":"stop"})))
                .and_then(|_| request(self.http_port, None))
                .and_then(|v| if v["armed"] == false { Ok(()) } else { Err("Receiver has not confirmed stopped output".into()) });
            match stopped.and_then(|_| self.updater.launch_installer()) {
                Ok(()) => { self.walking=false; self.test_room.stop(); self.halt.store(true, Ordering::Relaxed); ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
                Err(e) => self.updater.error(format!("Update was not installed: {e}")),
            }
        }
        if self.page != pages::Page::TestRoom || self.new_test {
            self.test_room.stop();
        }
        if (self.show_movement_preview || self.page == Page::Direction) && self.page != Page::Home && !self.bevy_room.sensor_calibration_open() {
            let crosshair_width = if matches!(self.page, Page::TestRoom | Page::Direction) {
                ctx.content_rect().width() * 0.5
            } else {
                214.
            };
            let mut deadzone_command = None;
            egui::SidePanel::right("movement_crosshair")
                .exact_width(crosshair_width)
                .resizable(false)
                .frame(egui::Frame::new().fill(pages::BG).inner_margin(egui::Margin::same(20)))
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().id_salt("movement_deadzone_scroll").show(ui, |ui| {
                        if matches!(self.page, Page::TestRoom | Page::Direction) {
                            deadzone_command = self.action_pages.movement_deadzone(ui, s, status.online, busy, self.page == Page::TestRoom);
                            if self.page == Page::Direction {
                                ui.add_space(8.);
                                ui.label("Direction angles · yaw");
                                let chest = s["actions"]["preview"]["chest_yaw"].as_f64();
                                let heading = s["heading"].as_f64().filter(|_| s["calibrated"] == true && s["roles"]["torso"]["fresh"] == true);
                                let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.), egui::Sense::hover());
                                let painter = ui.painter_at(rect);
                                painter.rect_filled(rect, 4., pages::CARD);
                                painter.line_segment([egui::pos2(rect.center().x, rect.top()), egui::pos2(rect.center().x, rect.bottom())], egui::Stroke::new(1., pages::MUTED));
                                for (angle, color) in [(chest, pages::ORANGE), (heading, pages::MINT)] {
                                    if let Some(angle) = angle {
                                        let x = rect.center().x + (angle as f32 / std::f32::consts::PI).clamp(-1., 1.) * rect.width() * 0.5;
                                        painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], egui::Stroke::new(2., color));
                                    }
                                }
                                ui.horizontal_wrapped(|ui| {
                                    ui.colored_label(pages::ORANGE, format!("Chest {}", chest.map(|v| format!("{:.0}°", v.to_degrees())).unwrap_or_else(|| "unavailable".into())));
                                    ui.colored_label(pages::MINT, format!("Heading {}", heading.map(|v| format!("{:.0}°", v.to_degrees())).unwrap_or_else(|| "unavailable".into())));
                                });
                                ui.small("Head yaw is not available in this view.");
                                ui.add_space(10.);
                                ui.label(egui::RichText::new("Output monitor").strong().color(pages::ORANGE));
                                let travel = s["actions"]["preview"]["travel_xy"].as_array();
                                for (index, name) in [(0, "Stick X"), (1, "Stick Y")] {
                                    let value = travel.and_then(|v| v.get(index)).and_then(|v| v.as_f64()).unwrap_or(0.).clamp(-1., 1.) as f32;
                                    ui.horizontal(|ui| {
                                        ui.label(name);
                                        let (rect, _) = ui.allocate_exact_size(egui::vec2((ui.available_width() - 52.).max(50.), 18.), egui::Sense::hover());
                                        let painter = ui.painter_at(rect);
                                        painter.rect_filled(rect, 4., pages::CARD);
                                        let center = rect.center().x;
                                        let end = center + value * rect.width() * 0.5;
                                        let fill = egui::Rect::from_min_max(egui::pos2(center.min(end), rect.top()), egui::pos2(center.max(end), rect.bottom()));
                                        painter.rect_filled(fill, 3., pages::ORANGE);
                                        painter.line_segment([egui::pos2(center, rect.top()), egui::pos2(center, rect.bottom())], egui::Stroke::new(1., pages::MUTED));
                                        ui.label(format!("{:+.0}%", value * 100.));
                                    });
                                }
                                ui.small("Direction preview · game output remains separately controlled.");
                            }
                        } else {
                            pages::xy(ui, s, ui.available_width().min(360.));
                        }
                    });
                });
            if let Some(command) = deadzone_command {
                self.walking = false;
                self.send(json!({"command":"stop"}));
                self.send(command);
            }
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(pages::BG).inner_margin(if skinned_home {0} else {20})).show(ctx, |ui| {egui::ScrollArea::vertical().id_salt(format!("page_scroll_{:?}", self.page)).show(ui,|ui| {
            if self.show_hints && !matches!(self.page,pages::Page::Home|pages::Page::Settings|pages::Page::MovementSetup|pages::Page::TestRoom|pages::Page::Jump) && !(self.page==pages::Page::Main&&s["movement_engine"]=="body_tilt"&&!self.old_chest_main) && !(self.page==pages::Page::QuickSetup&&self.new_setup) {ui.colored_label(pages::MUTED,match self.page {pages::Page::Steps=>"Tune the detector while stopped. Preview runs independently from game output.",pages::Page::QuickSetup=>if s["movement_engine"]=="body_tilt" {"Tilt mode needs only the chest tracker. Stand upright and reset center."}else{"Assign three distinct sensors, then stand still and reset center."},pages::Page::Direction=>"Direction uses calibrated chest orientation. Angles below are degrees.",_=>"Settings changes pause game movement. Reset center before enabling game movement."});ui.add_space(10.);}
            match self.page {
                pages::Page::Home=>{
                    if self.bevy_room.sensor_calibration_open() {
                        if let Some(command)=self.bevy_room.show_sensor_calibration(ui,s,self.http_port,frame,
                            qa_capture::path().is_none() || std::env::var_os("ARCANEVR_EMBED_QA").is_some(),&status.error,status.online) {
                            self.walking=false;self.send(command);
                        }
                    } else {
                    if let Some(action)=home_dashboard::show(ui,s,status.online,busy,false,ready && !busy && !self.bevy_room.sensor_calibration_open() && s["calibrated"]==true && driver_ready && actions_output_ready && s["locomotion"]["input_ready"]==true){match action{
                        home_dashboard::Action::Command(command)=>{self.send(json!({"command":"stop"}));self.send(command);},
                        home_dashboard::Action::Navigate(page)=>{if page==Page::Settings {self.page=Page::TestRoom;self.show_movement_preview=true;} else {self.page=page;}},
                        home_dashboard::Action::Edit=>self.page=Page::QuickSetup,
                        home_dashboard::Action::Done=>{},
                        home_dashboard::Action::ForceSteamHome=>{
                            self.walking=false;self.send(json!({"command":"stop"}));
                            let result=std::env::current_exe().ok().and_then(|exe|std::process::Command::new(exe).arg("--force-steam-home").status().ok());
                            let message=if result.is_some_and(|s|s.success()){ "SteamVR Home requested. The current VR game may close." }else{"Could not request SteamVR Home. Check that SteamVR is running."};
                            ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new("force_home_result"),message.to_string()));
                        },
                        home_dashboard::Action::BoundarySetup=>{
                            self.walking=false;self.send(json!({"command":"stop"}));
                            let result=std::env::current_exe().ok().and_then(|exe|std::process::Command::new(exe).arg("--steam-room-setup").status().ok());
                            let message=if result.is_some_and(|s|s.success()){ "SteamVR Room Setup requested; Quest boundary may require headset settings." }else{"Could not open SteamVR Room Setup. Check SteamVR and use Quest boundary settings."};
                            ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new("force_home_result"),message.to_string()));
                        },
                        home_dashboard::Action::SetMovement(enabled)=>{if enabled {self.send(json!({"command":"arm"}));} else {self.walking=false;self.test_room.stop();self.send(json!({"command":"stop"}));}},
                        home_dashboard::Action::ResetCenter=>{self.walking=false;self.test_room.stop();self.send(json!({"command":"calibrate"}));},
                        home_dashboard::Action::SensorCalibration=>{self.walking=false;self.test_room.stop();self.send(json!({"command":"stop"}));self.bevy_room.start_sensor_calibration(s);}
                    }}
                    }
                },
                Page::Settings => {},
                Page::MovementSetup => {
                    if ui.button("Open step movement tuning").clicked() { self.page = Page::Steps; }
                    if let Some(command) = self.profile_builder.show(ui, s, status.online && !busy) {
                        self.walking = false;
                        self.send(json!({"command":"stop"}));
                        self.send(command);
                    }
                },
                pages::Page::GameProfiles=>{if let Some(action)=self.game_profiles.show(ui,s,status.online,busy){match action{game_profiles::Action::Apply(command)=>{self.walking=false;self.send(json!({"command":"stop"}));self.send(command);},game_profiles::Action::ResetCenter=>{self.walking=false;self.send(json!({"command":"calibrate"}));},game_profiles::Action::SensorCalibration=>{self.walking=false;self.send(json!({"command":"stop"}));self.bevy_room.start_sensor_calibration(s);}}}},
                pages::Page::Main=>{
                    if s["movement_engine"]=="body_tilt" {ui.horizontal(|ui| {ui.selectable_value(&mut self.old_chest_main,false,"New");ui.selectable_value(&mut self.old_chest_main,true,"Old");});}
                    if s["movement_engine"]=="body_tilt" && !self.old_chest_main { if let Some(action)=self.chest_tuning.show(ui,s,status.online,busy,&status.error){match action{chest_tuning::Action::Command(command)=>{self.walking=false;self.send(json!({"command":"stop"}));self.send(command);},chest_tuning::Action::ResetCenter=>{self.walking=false;self.send(json!({"command":"calibrate"}));},chest_tuning::Action::SensorCalibration=>{self.walking=false;self.send(json!({"command":"stop"}));self.bevy_room.start_sensor_calibration(s);},chest_tuning::Action::TestRoom=>{self.page=pages::Page::TestRoom;}}} } else {
                    if let Some(action)=self.game_profiles.show(ui,s,status.online,busy){match action{game_profiles::Action::Apply(command)=>{self.walking=false;self.send(json!({"command":"stop"}));self.send(command);},game_profiles::Action::ResetCenter=>{self.walking=false;self.send(json!({"command":"calibrate"}));},game_profiles::Action::SensorCalibration=>{self.walking=false;self.send(json!({"command":"stop"}));self.bevy_room.start_sensor_calibration(s);}}}
                    for action in pages::main_page(ui,s,status.online,busy,ready){match action{pages::Action::Command(command)=>{self.walking=false;self.send(json!({"command":"stop"}));self.send(command);},pages::Action::Navigate(page)=>self.page=page}}
                    }
                },
                pages::Page::Steps=>{
                    if s["sensor_layout"]["count"]==1 {
                        ui.heading("Learn walking - one chest sensor");
                        ui.label("Lean forward first, then walk in place. ArcaneVR learns your left/right chest sway.");
                        let learning=&s["walking_learning"];
                        if learning["active"]==true {
                            ui.heading(learning["text"].as_str().unwrap_or("Learning"));
                            if let Some(n)=learning["remaining"].as_u64(){ui.label(format!("{n} seconds remaining"));}
                            if ui.button("Cancel learning").clicked(){self.send(json!({"command":"walking_learn_cancel"}));}
                        }else{
                            if let Some(message)=learning["message"].as_str(){ui.label(message);}
                            let ready=status.online && s["step_input"]=="chest" && s["calibrated"]==true && s["mounting_calibration"]["valid"]==true;
                            if ui.add_enabled(ready,egui::Button::new("Learn walking")).clicked(){self.send(json!({"command":"walking_learn_start"}));}
                            if !ready {ui.label("Enable Walk in place, calibrate your chest sensor, and Reset Center to begin.");}
                        }
                        ui.separator();
                    }

                    if s["step_input"]=="chest" && s["walking_model"].is_object() {
                        ui.label("Adjust how much learned walking effort reaches full walking speed.");
                        let key=ui.make_persistent_id("walking_effort_draft");
                        let saved=s["walking_model"]["full_speed_effort"].as_f64().unwrap_or(0.5)*100.;
                        let mut effort=ui.data(|data|data.get_temp::<f64>(key)).unwrap_or(saved);
                        ui.add(egui::Slider::new(&mut effort,10.0..=200.0).suffix("%").text("Effort for full speed"));
                        ui.data_mut(|data|data.insert_temp(key,effort));
                        ui.small("50% reaches full speed at half your learned effort. Lower = fewer/slower steps needed. One chest sensor measures repeated sway, not exact footstep counts.");
                        if ui.add_enabled(status.online && !busy && (effort-saved).abs()>0.01,egui::Button::new("Save walking speed")).clicked(){self.send(json!({"command":"walking_full_speed","effort":effort/100.}));}

                        let sprint=s["game_output_preview"]["buttons"].as_u64().unwrap_or(0)&4!=0;
                        let axes=&s["game_output_preview"]["travel_xy"];
                        let speed=axes[0].as_f64().unwrap_or(0.).hypot(axes[1].as_f64().unwrap_or(0.)) * if sprint {1.2}else{1.};
                        ui.label(format!("Detected walking effort: {:.0}%",s["walking_effort"].as_f64().unwrap_or(0.)*100.));
                        ui.label(format!("{}: {:.0}%",if sprint {"Test movement · sprint"}else{"Test movement"},speed*100.));
                        ui.add(egui::ProgressBar::new((speed/1.2) as f32));
                        ui.small("100% = full walking speed. Sprint: exceed 120% walking effort OR the lean Sprint ring. Enable Sprint in Direction.");

                    } else if let Some(command)=self.locomotion_panel.show_steps(ui,s,status.online,busy){self.walking=false;self.send(json!({"command":"stop"}));self.send(command);}
                },
                pages::Page::Devices=>{if let Some(command)=pages::devices_page(ui,s,status.online,busy){self.walking=false;self.send(command);}},
                pages::Page::Direction|pages::Page::Jump|pages::Page::Crouch|pages::Page::Turn=>{
                    if let Some(command)=self.action_pages.show(ui,s,self.page,status.online,busy){self.walking=false;self.send(json!({"command":"stop"}));self.send(command);}
                },
                pages::Page::Profiles=>{if let Some(command)=self.action_pages.profiles(ui,s,status.online){self.walking=false;self.send(json!({"command":"stop"}));self.send(command);}},
                pages::Page::TestRoom=>{
                    if s["step_input"]=="chest" {
                        if s["walking_learned"]!=true {
                            ui.colored_label(pages::ORANGE,"Learn walking first: forward movement requires a saved walking pattern for this chest sensor.");
                            if ui.button("Learn walking in Steps").clicked(){self.page=Page::Steps;}
                        }else{ui.label("Walking pattern loaded. Lean forward and walk in place to move; stop walking to stop forward movement.");}
                        ui.small("The crosshair always shows lean. Sideways lean still strafes without walking.");
                    }

                    self.new_test=true;
                    if self.new_test{if let Some(command)=self.bevy_room.show(ui,s,self.http_port,frame,qa_capture::path().is_none() || std::env::var_os("ARCANEVR_EMBED_QA").is_some(),status.online){self.walking=false;self.send(command);}}else{if let Some(command)=self.test_room.show(ui,s){self.send(command);}}
                    ui.add_space(12.);
                    if let Some(command)=self.locomotion_panel.show_test_graph(ui,s,status.online,busy) {self.walking=false;self.send(json!({"command":"stop"}));self.send(command);}
                },
                pages::Page::QuickSetup=>{
            self.new_setup=true;
            if self.new_setup {
                if let Some(command)=setup_new::show(ui,s,&mut self.avatar_texture,status.online){
                    if command["command"]=="open_usb"{self.setup_open=true;let _=self.usb.tx.send(UsbAction::Refresh);}else{self.walking=false;self.send(json!({"command":"stop"}));self.send(command);self.initialized=false;}
                }
            }else{

            ui.add_space(5.);
            ui.add_enabled_ui(status.online,|ui| {ui.horizontal(|ui| {
                for (mode,label) in [("steamvr","SteamVR"),("meta","Meta Quest")] {
                    if ui.add_sized([190.,42.],egui::Button::new(label).selected(self.choice && s["output_mode"]==mode)).clicked() { self.switch("output_mode",mode); self.choice=true; }
                }
            });});
            if s["output_mode"]=="meta" {ui.colored_label(Color32::YELLOW,"Meta Quest · local protocol mock. Standalone Quest gameplay is not connected.");}
            ui.label(s["output_status"].as_str().unwrap_or("Waiting for receiver…"));
            if s["output_mode"]=="steamvr" { ui.horizontal(|ui| {for (key,label) in [("driver_connected","Driver"),("hmd_ready","Headset"),("driver_calibrated","Driver heading")] {let yes=s["steamvr"][key]==true;ui.colored_label(if yes {Color32::LIGHT_GREEN}else{Color32::YELLOW},format!("{label}: {}",if yes {"ready"}else{"waiting"}));}}); }
            ui.add_space(12.); ui.heading("Connect & place your sensors");
            if ui.add_sized([260.,36.],egui::Button::new("+ Add sensors / USB Wi-Fi setup")).clicked() {
                self.setup_open=true;self.walking=false;self.send(json!({"command":"stop"}));
                if demo {self.send(json!({"command":"mode","mode":"live"}));}
                let _=self.usb.tx.send(UsbAction::Refresh);
            }
            if ui.button("Sensor addresses").clicked(){self.network_open=true;for i in 0..3 {if self.network_ips[i].is_empty(){self.network_ips[i]=self.selected[i].rsplit_once(':').map(|(ip,_)|ip.to_string()).unwrap_or_default();}}}
            ui.label(RichText::new("Move a tracker to identify it. Drag its card onto either foot or your chest.").color(Color32::from_rgb(139,159,183)));
            ui.add_enabled_ui(status.online,|ui| {ui.horizontal(|ui| {
                if ui.selectable_label(!demo,"Live trackers").clicked() {self.switch("mode","live");}
                if ui.selectable_label(demo,"Synthetic demo").clicked() {self.switch("mode","demo");}
            });});
            if demo {ui.colored_label(Color32::YELLOW,"SYNTHETIC PREVIEW · live SteamVR output disabled");ui.horizontal(|ui| {
                let walk=ui.checkbox(&mut self.walking,"Simulate walking").changed();
                let yaw=ui.add(egui::Slider::new(&mut self.yaw,-180.0..=180.0).text("Chest direction °")).changed();
                if walk||yaw {self.send(json!({"command":"demo","walk":self.walking,"yaw":self.yaw}));}
            });}
            let mut body_state=s.clone();body_state["receiver_online"]=json!(status.online);
            let changed=ui.add_enabled_ui(status.online&&!demo&&!busy,|ui| body_setup::body_setup(ui,&body_state,&mut self.selected,&mut self.selected_sensor)).inner;
            let distinct=(0..3).all(|i|(i+1..3).all(|j|self.selected[i].is_empty()||self.selected[i]!=self.selected[j]));let complete=distinct&&if s["movement_engine"]=="body_tilt"{!self.selected[2].is_empty()}else{self.selected.iter().all(|id|!id.is_empty())};
            if changed {
                self.send(json!({"command":"stop"}));
                if complete {self.send(json!({"command":"assign","trackers":{"left":self.selected[0],"right":self.selected[1],"torso":self.selected[2]}}));}
            }
            ui.horizontal(|ui| {
                ui.colored_label(Color32::from_rgb(117,239,196),if assigned {"Body assignments saved"}else if complete {"Saving body assignments…"}else if s["movement_engine"]=="body_tilt"{"Place one sensor on CHEST; feet are optional"}else{"Place three different sensors to save your setup"});
                if complete&&!assigned&&ui.button("Save again").clicked(){self.send(json!({"command":"assign","trackers":{"left":self.selected[0],"right":self.selected[1],"torso":self.selected[2]}}));}
                ui.label(format!("{} packets received",s["received"].as_u64().unwrap_or(0)));
            });
            ui.add_space(14.);
            ui.separator();
            ui.horizontal(|ui| {
                ui.strong("Footstep preview");
                ui.colored_label(Color32::from_rgb(117,239,196),format!("Left  {}",s["preview_steps"]["left"].as_u64().unwrap_or(0)));
                ui.colored_label(Color32::from_rgb(86,199,245),format!("Right  {}",s["preview_steps"]["right"].as_u64().unwrap_or(0)));
                ui.small("Counts react while movement is stopped");
            });
            ui.add_space(14.);
            ui.heading("Calibrate & move");
            ui.label("Stand still with your chest and headset facing your intended forward direction.");
            ui.horizontal(|ui| {
                if ui.add_enabled(ready&&!busy,egui::Button::new("Calibrate · 3 second countdown")).clicked() {self.walking=false;self.send(json!({"command":"countdown"}));}
                if let Some(n)=status.countdown {ui.strong(format!("Stand still · {n}"));if ui.button("Cancel").clicked(){self.send(json!({"command":"stop"}));}}
                if ui.add_enabled(ready&&!busy&&s["calibrated"]==true&&driver_ready&&actions_output_ready&&s["locomotion"]["input_ready"]==true,egui::Button::new("Arm movement")).clicked(){self.send(json!({"command":"arm"}));}
            });
            if !self.choice {ui.label("Choose SteamVR or Meta Quest first.");} else if !ready {ui.label(if s["movement_engine"]=="body_tilt" {"Turn on the chest tracker and wait for live data before calibrating."}else{"Assign three different sensors and wait for fresh data before calibrating."});}
            else if s["locomotion"]["input_ready"]!=true {ui.colored_label(Color32::from_rgb(237,188,119),if s["movement_engine"]=="body_tilt" {"Waiting for fresh chest tilt data."}else{"Waiting for valid foot motion input. Check the standing-test status above before arming."});}
            if !status.error.is_empty(){ui.colored_label(Color32::YELLOW,&status.error);}
            for key in ["error","output_error"] {if let Some(e)=s[key].as_str().filter(|e|!e.is_empty()) {ui.colored_label(Color32::YELLOW,e);}}
            ui.collapsing("Movement monitor",|ui| {
            let speed=if status.online{s["speed"].as_f64().unwrap_or(0.) as f32}else{0.};
            ui.horizontal(|ui| {ui.label(RichText::new(format!("{:.0}% speed",speed*100.)).size(28.));ui.label(RichText::new(format!("{:.0}° chest heading",s["heading"].as_f64().unwrap_or(0.).to_degrees())).size(24.));});
            ui.add(egui::ProgressBar::new(speed).desired_width(ui.available_width()));
            self.history.push(speed);if self.history.len()>150 {self.history.remove(0);}
            let (rect,_)=ui.allocate_exact_size(egui::vec2(ui.available_width(),65.),egui::Sense::hover());
            let points:Vec<_>=self.history.iter().enumerate().map(|(i,s)|egui::pos2(rect.left()+i as f32*rect.width()/149.,rect.bottom()-s.clamp(0.,1.)*rect.height())).collect();
            ui.painter().add(egui::Shape::line(points,egui::Stroke::new(2.,Color32::from_rgb(124,229,193))));
            if ui.add_enabled(status.online&&!demo,egui::Button::new(if s["recording"].is_string(){"Stop recording"}else{"Record live sensors"})).clicked(){self.send(json!({"command":"record"}));}
            if let Some(path)=s["recording"].as_str(){ui.label(format!("Recording: {path}"));}
            });
            ui.collapsing("Detection settings",|ui| {
                ui.label(if s["locomotion"]["algorithm"]=="continuous" {"Continuous motion uses the standing-test acceleration controls above. World-up and acceleration format remain hardware settings."} else {"Alternating mode uses fresh acceleration first (1.2 m/s²), with tilt as a fallback. Alternate left and right steps to drive locomotion."});
                ui.checkbox(&mut self.gravity_input,"Legacy acceleration includes gravity (standard SlimeVR: leave off)");
                ui.horizontal(|ui| {ui.label("World up");ui.selectable_value(&mut self.axis,"y".into(),"Y");ui.selectable_value(&mut self.axis,"z".into(),"Z");ui.add(egui::Slider::new(&mut self.threshold,0.1..=10.).text("Tilt fallback (rad/s)"));});
                if ui.add_enabled(status.online&&!busy,egui::Button::new("Apply settings and recalibrate")).clicked(){self.send(json!({"command":"stop"}));self.send(json!({"command":"settings","axis":self.axis,"threshold":self.threshold,"acceleration_mode":if self.gravity_input {"gravity-baseline"}else{"linear"}}));}
            });
            ui.collapsing("Transport diagnostics",|ui| {ui.monospace(serde_json::to_string_pretty(&json!({"steamvr":s["steamvr"],"packet":s["packet"]})).unwrap_or_default());});
            ui.add_space(15.);
            }
                },

            }
            if !status.error.is_empty(){ui.colored_label(Color32::from_rgb(244,170,94),&status.error);}
            if self.debug {ui.collapsing("Debug state",|ui|{ui.monospace(serde_json::to_string_pretty(s).unwrap_or_default());});}
        });});
        if self.network_open {
            let mut open = true;
            egui::Window::new("Sensor network addresses").open(&mut open).default_width(650.).show(ctx,|ui| {
                ui.heading("Fixed addresses, familiar sensors");
                ui.label("For stock SlimeVR firmware, reserve each sensor's Wi-Fi MAC address in your router's DHCP settings. You can edit that reservation later.");
                ui.colored_label(Color32::from_rgb(237,188,119),"These fields update ArcaneVR's expected address. They do not change the tracker's network settings.");
                ui.add_space(10.);
                for (i,name) in ["Left foot / ankle","Right foot / ankle","Chest"].iter().enumerate(){
                    egui::Frame::group(ui.style()).show(ui,|ui|{
                        ui.strong(*name);ui.label(format!("Assigned sensor: {}",if self.selected[i].is_empty(){"Place a sensor on the body first"}else{&self.selected[i]}));
                        let old=self.selected[i].clone();
                        if let Some(record)=s["trackers"].as_array().and_then(|list|list.iter().find(|t|t["id"]==old)) {
                            if let Some(mac)=record["mac"].as_str(){ui.monospace(format!("Wi-Fi MAC: {mac}"));}
                        }
                        ui.horizontal(|ui|{
                            ui.label("Expected IPv4");ui.add(egui::TextEdit::singleline(&mut self.network_ips[i]).desired_width(180.));
                            let valid=self.network_ips[i].parse::<std::net::Ipv4Addr>().is_ok();
                            if ui.add_enabled(status.online&&!old.is_empty()&&valid,egui::Button::new("Update expected IP")).clicked(){
                                self.send(json!({"command":"stop"}));self.send(json!({"command":"remap_tracker","id":old,"ip":self.network_ips[i]}));
                                if let Some((_,sensor))=old.rsplit_once(':'){self.selected[i]=format!("{}:{sensor}",self.network_ips[i]);}
                            }
                        });
                    });
                }
                ui.add_space(8.);ui.small("After changing a DHCP reservation, reconnect the tracker. Movement stays stopped until the new address is streaming and you recalibrate.");
                if !status.error.is_empty(){ui.colored_label(Color32::YELLOW,&status.error);}
            });
            self.network_open = open;
        }
        if self.setup_open {
            self.usb_wizard.show(ctx, &mut self.setup_open, &self.usb, s, status.online, usb_qa());
        }
    }
}
fn launch_steam_tool(key:&std::ffi::CStr)->Result<(),String>{unsafe {
 let dll=std::path::PathBuf::from("C:/Program Files (x86)/Steam/steamapps/common/SteamVR/bin/win64/openvr_api.dll");
 let lib=libloading::Library::new(dll).map_err(|e|e.to_string())?;
 let init=*lib.get::<unsafe extern "C" fn(*mut i32,i32)->u32>(b"VR_InitInternal\0").map_err(|e|e.to_string())?;
 let shutdown=*lib.get::<unsafe extern "C" fn()>(b"VR_ShutdownInternal\0").map_err(|e|e.to_string())?;
 let get=*lib.get::<unsafe extern "C" fn(*const std::ffi::c_char,*mut i32)->*const std::ffi::c_void>(b"VR_GetGenericInterface\0").map_err(|e|e.to_string())?;
 let mut error=0;init(&mut error,3);if error!=0{return Err(format!("SteamVR init {error}"));}
 let table=get(c"FnTable:IVRApplications_008".as_ptr(),&mut error) as *const usize;
 if error!=0 || table.is_null(){shutdown();return Err(format!("SteamVR applications interface {error}"));}
 let launch:unsafe extern "system" fn(*const std::ffi::c_char)->i32=std::mem::transmute(*table.add(6));
 let code=launch(key.as_ptr());shutdown();
 if code==0 || code==106 {Ok(())}else{Err(format!("SteamVR launch error {code}"))}
}}
fn main() -> eframe::Result {
    if std::env::args().any(|a|a=="--force-steam-home"){std::process::exit(if launch_steam_tool(c"openvr.tool.steamvr_environments").is_ok(){0}else{1});}
    if std::env::args().any(|a|a=="--steam-room-setup"){std::process::exit(if launch_steam_tool(c"openvr.tool.steamvr_room_setup").is_ok(){0}else{1});}
    let qa = qa_capture::path().is_some();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(if qa && !matches!(qa_capture::page().as_deref(), Some("home-compact" | "test-compact")) {[1400., 940.]} else {[964., 705.]})
            .with_decorations(false)
            .with_resizable(qa)
            .with_icon(egui::IconData {
                rgba: include_bytes!("../../assets/arcanevr-app-icon.rgba").to_vec(),
                width: 256,
                height: 256,
            }),
        ..Default::default()
    };
    eframe::run_native(
        "ArcaneVR",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            let mut style = (*cc.egui_ctx.style()).clone();
            style
                .text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(15.));
            style
                .text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(14.));
            style.spacing.item_spacing = egui::vec2(10., 7.);
            style.spacing.button_padding = egui::vec2(14., 8.);
            style.visuals.panel_fill = pages::BG;
            style.visuals.window_fill = pages::CARD;
            style.visuals.selection.bg_fill = pages::ORANGE;
            style.visuals.selection.stroke = egui::Stroke::new(1., Color32::BLACK);
            style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(38, 48, 61);
            style.visuals.widgets.inactive.bg_stroke =
                egui::Stroke::new(1., Color32::from_rgb(69, 77, 89));
            style.visuals.widgets.active.bg_fill = pages::ORANGE;
            style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(73, 61, 47);
            style.visuals.slider_trailing_fill = true;
            cc.egui_ctx.set_style(style);
            Ok(Box::new(ArcaneVR::new()))
        }),
    )
}

#[cfg(windows)]
fn apply_rounded_window(ctx: &egui::Context, frame: &eframe::Frame) {
    #[repr(C)] struct Rect { left: i32, top: i32, right: i32, bottom: i32 }
    #[link(name = "user32")] extern "system" { fn GetWindowRect(hwnd: isize, rect: *mut Rect) -> i32; fn SetWindowRgn(hwnd: isize, region: isize, redraw: i32) -> i32; }
    #[link(name = "gdi32")] extern "system" { fn CreateRoundRectRgn(left:i32, top:i32, right:i32, bottom:i32, width:i32, height:i32) -> isize; fn DeleteObject(object:isize) -> i32; }
    let hwnd = match frame.window_handle().ok().map(|h| h.as_raw()) { Some(RawWindowHandle::Win32(h)) => h.hwnd.get(), _ => return };
    let mut rect=Rect{left:0,top:0,right:0,bottom:0};
    if unsafe { GetWindowRect(hwnd,&mut rect) } == 0 { return; }
    let (w,h)=(rect.right-rect.left,rect.bottom-rect.top);
    if w<=0 || h<=0 {return;}
    let key=egui::Id::new("rounded_window_region");
    if ctx.data(|d|d.get_temp::<(isize,i32,i32)>(key))==Some((hwnd,w,h)){return;}
    let radius=(20. * ctx.pixels_per_point()).round() as i32;
    let region=unsafe {CreateRoundRectRgn(0,0,w+1,h+1,radius,radius)};
    if region!=0 {
        if unsafe {SetWindowRgn(hwnd,region,1)} != 0 {ctx.data_mut(|d|d.insert_temp(key,(hwnd,w,h)));}
        else {unsafe {DeleteObject(region);}}
    }
}




