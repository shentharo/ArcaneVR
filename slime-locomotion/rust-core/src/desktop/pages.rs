//! Native page layout. Each action is handled by the desktop owner.
use eframe::egui::{self, Color32, RichText, Stroke};
use serde_json::{json, Value};

pub const BG: Color32 = Color32::from_rgb(11, 16, 22);
pub const CARD: Color32 = Color32::from_rgb(27, 35, 48);
pub const ORANGE: Color32 = Color32::from_rgb(239, 141, 32);
pub const CYAN: Color32 = Color32::from_rgb(32, 186, 243);
pub const MINT: Color32 = Color32::from_rgb(105, 222, 170);
pub const MUTED: Color32 = Color32::from_rgb(151, 160, 174);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    Home,
    Settings,
    MovementSetup,
    GameProfiles,
    QuickSetup,
    Devices,
    TestRoom,
    #[default]
    Main,
    Direction,
    Steps,
    Jump,
    Crouch,
    Turn,
    Profiles,
}
impl Page {
    pub const ALL: [Self; 14] = [
        Self::Home,
        Self::Settings,
        Self::MovementSetup,
        Self::GameProfiles,
        Self::QuickSetup,
        Self::Devices,
        Self::TestRoom,
        Self::Main,
        Self::Direction,
        Self::Steps,
        Self::Jump,
        Self::Crouch,
        Self::Turn,
        Self::Profiles,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Settings => "Settings",
            Self::MovementSetup => "Setup",
            Self::GameProfiles => "Game profile",
            Self::QuickSetup => "Quick Setup",
            Self::Devices => "Devices",
            Self::TestRoom => "Test Room",
            Self::Main => "Main",
            Self::Direction => "Direction",
            Self::Steps => "Steps",
            Self::Jump => "Jump",
            Self::Crouch => "Crouch",
            Self::Turn => "Turn",
            Self::Profiles => "Profiles",
        }
    }
}
pub enum Action {
    Command(Value),
    Navigate(Page),
}
pub fn card(ui: &mut egui::Ui, title: &str, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(2., Color32::from_rgb(66, 74, 85)))
        .corner_radius(12.)
        .inner_margin(16)
        .show(ui, |ui| {
            ui.label(RichText::new(title).size(21.).strong().color(ORANGE));
            ui.add_space(11.);
            content(ui);
        });
}
pub fn number(value: &Value) -> f32 {
    value.as_f64().filter(|x| x.is_finite()).unwrap_or(0.) as f32
}
pub fn setup_button(ui: &mut egui::Ui, title: &str) -> bool {
    ui.add_sized(
        [ui.available_width().clamp(80., 700.), 34.],
        egui::Button::new(RichText::new(title).strong().color(Color32::BLACK)).fill(CYAN),
    )
    .clicked()
}
pub fn warning(ui: &mut egui::Ui, state: &Value) {
    if state["mode"] == "demo" {
        ui.colored_label(
            ORANGE,
            RichText::new("SYNTHETIC DEMO — game output disabled").strong(),
        );
    }
}
pub fn xy(ui: &mut egui::Ui, state: &Value, side: f32) {
    let side = side.clamp(130., 440.);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
    let p = ui.painter_at(rect);
    let c = rect.center();
    let r = side * 0.44;
    p.circle_stroke(c, r, Stroke::new(2., Color32::from_rgb(103, 113, 127)));
    for f in [0.25, 0.5, 0.75] {
        p.circle_stroke(c, r * f, Stroke::new(1., Color32::from_rgb(48, 59, 73)));
    }
    p.line_segment(
        [c - egui::vec2(r, 0.), c + egui::vec2(r, 0.)],
        Stroke::new(1., Color32::from_rgb(68, 80, 97)),
    );
    p.line_segment(
        [c - egui::vec2(0., r), c + egui::vec2(0., r)],
        Stroke::new(1., Color32::from_rgb(68, 80, 97)),
    );
    let speed = number(&state["locomotion"]["preview"]["speed"]).clamp(0., 1.);
    let heading = number(&state["heading"]);
    let output = state["direction"]["preview_xy"].as_array().or_else(|| {
        if state["movement_engine"] == "body_tilt" || state["movement_engine"] == "body_tilt_steps"
        {
            state["actions"]["preview"]["travel_xy"].as_array()
        } else {
            None
        }
    });
    let x = output
        .and_then(|a| a.first())
        .map(number)
        .unwrap_or(heading.sin() * speed);
    let y = output
        .and_then(|a| a.get(1))
        .map(number)
        .unwrap_or(heading.cos() * speed);
    let tilt_mode=state["movement_engine"]=="body_tilt";
    let cap=number(&state["actions"]["settings"]["direction"]["max_output"]).max(0.05);
    let (x,y)=if tilt_mode {(x/cap,y/cap)}else{(x,y)};
    let point = c + egui::vec2(x.clamp(-1., 1.) * r, -y.clamp(-1., 1.) * r);
    p.line_segment([c, point], Stroke::new(2., ORANGE));
    p.circle_filled(point, 7., ORANGE);
    for (offset, text) in [
        (egui::vec2(0., -r - 7.), "FWD"),
        (egui::vec2(0., r + 8.), "BACK"),
        (egui::vec2(-r - 7., 0.), "L"),
        (egui::vec2(r + 7., 0.), "R"),
    ] {
        p.text(
            c + offset,
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(10.),
            MUTED,
        );
    }
}
pub fn has_ankle_trackers(state: &Value) -> bool {
    matches!(state["sensor_layout"]["count"].as_u64(), Some(3 | 5))
}

pub fn shows_step_settings(state: &Value) -> bool {
    has_ankle_trackers(state) || state["step_input"] == "chest"
}

pub fn main_page(
    ui: &mut egui::Ui,
    state: &Value,
    online: bool,
    busy: bool,
    can_calibrate: bool,
) -> Vec<Action> {
    let mut actions = vec![];
    warning(ui, state);
    ui.add_space(8.);
    let width = (ui.available_width() - 28.).max(600.);
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(width * 0.29, 0.),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                card(ui, "Engine & Output", |ui| {
                    if has_ankle_trackers(state) {
                        ui.label("Engine");
                        let mut engine = state["movement_engine"]
                            .as_str()
                            .unwrap_or("chest_heading_steps")
                            .to_string();
                        egui::ComboBox::from_id_salt("movement_engine")
                            .selected_text(match engine.as_str() {
                                "body_tilt_steps" => "Body Tilt + Leg Steps",
                                "body_tilt" => "Body Tilt",
                                _ => "Chest Heading + Leg Steps",
                            })
                            .show_ui(ui, |ui| {
                                for (value, label) in [
                                    ("chest_heading_steps", "Chest Heading + Leg Steps"),
                                    ("body_tilt_steps", "Body Tilt + Leg Steps"),
                                ] {
                                    if ui
                                        .selectable_value(&mut engine, value.to_string(), label)
                                        .changed()
                                    {
                                        actions.push(Action::Command(
                                            json!({"command":"movement_engine","engine":value}),
                                        ));
                                    }
                                }
                            });
                        ui.add_space(8.);
                    }
                    ui.label("Output");
                    let mut output = state["output_mode"]
                        .as_str()
                        .unwrap_or("steamvr")
                        .to_string();
                    egui::ComboBox::from_id_salt("main_output")
                        .selected_text(if output == "meta" {
                            "Meta Quest (experimental)"
                        } else {
                            "SteamVR"
                        })
                        .width(ui.available_width() - 20.)
                        .show_ui(ui, |ui| {
                            for (value, name) in [
                                ("steamvr", "SteamVR"),
                                ("meta", "Meta Quest (experimental)"),
                            ] {
                                if ui
                                    .selectable_value(&mut output, value.into(), name)
                                    .changed()
                                {
                                    actions.push(Action::Command(
                                        json!({"command":"output_mode","mode":value}),
                                    ));
                                }
                            }
                        });
                    ui.small(
                        state["output_status"]
                            .as_str()
                            .unwrap_or("Receiver offline"),
                    );
                    ui.add_space(12.);
                    if setup_button(ui, "SENSOR SETUP") {
                        actions.push(Action::Navigate(Page::QuickSetup));
                    }
                });
                if shows_step_settings(state) {
                    ui.add_space(14.);
                    card(ui, "Step Setup", |ui| {
                        let cfg = &state["locomotion"]["settings"];
                        for (name, value) in [
                            ("Intensity", format!("{:.2}×", number(&cfg["intensity"]))),
                            ("Deadzone", format!("{:.2}", number(&cfg["threshold"]))),
                            (
                                "Inertia",
                                format!("{:.0} ms", number(&cfg["inertia_window"]) * 1000.),
                            ),
                        ] {
                            ui.horizontal(|ui| {
                                ui.label(name);
                                ui.label(RichText::new(value).strong());
                            });
                        }
                        ui.add_space(10.);
                        if setup_button(ui, "STEP SETTINGS") {
                            actions.push(Action::Navigate(Page::Steps));
                        }
                        ui.small(
                            state["locomotion"]["preview"]["status"]
                                .as_str()
                                .unwrap_or("Waiting for sensor data"),
                        );
                    });
                }
            },
        );
        ui.allocate_ui_with_layout(
            egui::vec2(width * 0.44, 0.),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                card(ui, "Movement Visualizer", |ui| {
                    ui.label(RichText::new(if state["movement_engine"]=="body_tilt" {"Lean response (normalised); game output is capped separately"}else{"Chest direction + detector preview"}).color(MUTED));
                    if state["movement_engine"]=="body_tilt" {
                        ui.label("Use Sensor calibration in the test room to learn and save all directions.");
                        if ui.add_enabled(online&&!busy,egui::Button::new("Open sensor setup in test room")).clicked(){actions.push(Action::Navigate(Page::TestRoom));}
                    }
                    xy(ui, state, (ui.available_width() - 8.).min(390.));
                    ui.add_space(14.);
                    if ui
                        .add_enabled(
                            online && !busy && can_calibrate,
                            egui::Button::new(RichText::new("RESET CENTER").strong())
                                .min_size(egui::vec2(ui.available_width(), 42.)),
                        )
                        .clicked()
                    {
                        actions.push(Action::Command(json!({"command":"calibrate"})));
                    }
                    ui.small("Stand upright facing forward · resets immediately");
                    ui.add_space(8.);
                    ui.colored_label(
                        ORANGE,
                        format!(
                            "Chest heading  {:.0}°",
                            number(&state["heading"]).to_degrees()
                        ),
                    );
                    ui.small("Preview does not arm movement.");
                });
            },
        );
        ui.allocate_ui_with_layout(
            egui::vec2(width * 0.27, 0.),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                card(ui, "Live Motion", |ui| {
                    let speed = if online && state["mode"] != "demo" {
                        number(if state["output_mode"] == "steamvr" {
                            &state["steamvr"]["speed"]
                        } else {
                            &state["speed"]
                        })
                    } else {
                        0.
                    };
                    ui.label(RichText::new("Speed").color(ORANGE));
                    ui.label(RichText::new(format!("{:.0}%", speed * 100.)).size(38.));
                    ui.add(
                        egui::ProgressBar::new(speed.clamp(0., 1.))
                            .fill(ORANGE)
                            .desired_width(ui.available_width()),
                    );
                    ui.small("Actual movement output");
                    ui.add_space(18.);
                    ui.label(RichText::new("Preview speed").color(ORANGE));
                    ui.label(
                        RichText::new(format!(
                            "{:.0}%",
                            number(&state["locomotion"]["preview"]["speed"]) * 100.
                        ))
                        .size(28.),
                    );
                    for (name, key) in [("Crouching", "crouch"), ("Jump", "jump")] {
                        ui.add_space(18.);
                        ui.label(RichText::new(name).color(ORANGE));
                        if let Some(v) = state["actions"]["preview"][if key == "jump" {
                            "jump_pressed"
                        } else {
                            "crouching"
                        }]
                        .as_bool()
                        {
                            ui.add(
                                egui::ProgressBar::new(if v { 1. } else { 0. })
                                    .fill(CYAN)
                                    .desired_width(ui.available_width()),
                            );
                        } else {
                            ui.small("Preview not connected yet");
                        }
                    }
                    ui.add_space(16.);
                    if ui
                        .add_enabled(online, egui::Button::new("Reset standing test"))
                        .clicked()
                    {
                        actions.push(Action::Command(json!({"command":"reset_test"})));
                    }
                });
            },
        );
    });
    actions
}
/// Device corrections belong to hardware identity, rather than the game profile.
pub fn devices_page(ui: &mut egui::Ui, s: &Value, online: bool, busy: bool) -> Option<Value> {
    let mut command = None;
    ui.heading("Devices & calibration");
    ui.label("SlimeVR trackers. Mounting corrections follow the MAC address when the network address changes.");
    let identity=s["mounting_calibration"]["tracker_identity"].as_str().unwrap_or("");
    let d=&s["actions"]["settings"]["direction"];
    for (role,label) in [("torso","Chest"),("left","Left ankle"),("right","Right ankle")] {
        if role != "torso" && !has_ankle_trackers(s) { continue; }
        let sensor=&s["roles"][role];
        if sensor["id"].as_str().is_none_or(str::is_empty) {continue;}
        card(ui,label,|ui| {
            ui.label(format!("Address: {}",sensor["id"].as_str().unwrap_or("Unassigned")));
            ui.label(format!("MAC: {}",sensor["mac"].as_str().unwrap_or("Not yet identified")));
            ui.colored_label(if sensor["fresh"]==true {MINT}else{MUTED},if sensor["fresh"]==true {"Live data"}else{"Offline / stale"});
            if role!="torso" {return;}
            ui.label(format!("Device identity: {}",if identity.is_empty(){"Waiting for tracker identity"}else{identity}));
            ui.label(format!("Mounting calibration: {}",if s["mounting_calibration"]["valid"]==true {"Valid for this tracker"}else{"Required"}));
            ui.label(format!("Center: {}",if s["actions"]["preview"]["calibrated"]==true {"Calibrated for this session"}else{"Reset upright before movement"}));
            ui.horizontal_wrapped(|ui| {
                for (op,label) in [("flip_forward","Flip forward / backward"),("flip_lateral","Flip left / right")] {
                    if ui.add_enabled(online&&!busy&&!identity.is_empty(),egui::Button::new(label)).clicked(){command=Some(json!({"command":"device_mounting","operation":op,"identity":identity}));}
                }
            });
            ui.small("Each press reverses that axis, stops output, and saves the correction to this device.");
            ui.separator();
            let turned=s["device_calibrations"][identity]["face_turned_over"]==true;
            ui.label(if turned {"Mounting: opposite face outward"} else {"Mounting: calibrated face outward"});
            if ui.add_enabled(online&&!busy&&!identity.is_empty(),egui::Button::new(if turned {"Restore original face"} else {"Tracker turned over"})).clicked(){command=Some(json!({"command":"device_mounting","operation":"turn_over","identity":identity}));}
            ui.small("Use when the opposite face points outward and the top edge still points up. Reset center upright afterward. For any other rotation use Sensor calibration.");

            ui.collapsing("Saved mounting and motion settings",|ui| {
                for key in ["pitch_axis","pitch_sign","roll_axis","roll_sign","learned_forward","learned_right","forward_deadzone","backward_deadzone","left_deadzone","right_deadzone","deadzone","forward_offset","smoothing_ms"] {
                    ui.label(format!("{key}: {}",d[key]));
                }
                ui.small("Mounting axes are device-specific. Deadzones and response settings belong to the movement profile; angles above are radians.");
                if let Some(record)=s["device_calibrations"].get(identity) {ui.label(format!("Saved device record: {record}"));}
            });
            ui.collapsing("Reset device calibration to defaults",|ui| {
                ui.label("Clears this tracker's learned mounting and flip corrections. Calibrate the sensor again afterward. Wi-Fi and assignments are retained.");
                if ui.add_enabled(online&&!busy&&!identity.is_empty(),egui::Button::new("Reset this device to defaults")).clicked(){command=Some(json!({"command":"device_mounting","operation":"reset","identity":identity}));}
            });
        });
        ui.add_space(10.);
    }
    command
}
