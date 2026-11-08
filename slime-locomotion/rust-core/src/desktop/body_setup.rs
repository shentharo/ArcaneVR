//! Local assignment draft and raw sensor orientation; no backend side effects.
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use serde_json::Value;

const MINT: Color32 = Color32::from_rgb(117, 239, 196);
const CYAN: Color32 = Color32::from_rgb(86, 199, 245);
const VIOLET: Color32 = Color32::from_rgb(181, 153, 255);
const TEXT: Color32 = Color32::from_rgb(223, 234, 249);
const MUTED: Color32 = Color32::from_rgb(135, 156, 184);
const ROLES: [&str; 3] = ["left", "right", "torso"];
const COLORS: [Color32; 3] = [MINT, CYAN, VIOLET];

#[derive(Clone)]
struct SensorDrag(String);
#[derive(Clone)]
struct LastSample {
    q: [f64; 4],
    time: f64,
    speed: f32,
}

fn label(p: &egui::Painter, at: Pos2, text: impl ToString, size: f32, color: Color32) {
    p.text(
        at,
        Align2::LEFT_TOP,
        text.to_string(),
        FontId::proportional(size),
        color,
    );
}
fn short(id: &str, max: usize) -> String {
    let chars: Vec<_> = id.chars().collect();
    if chars.len() <= max {
        id.into()
    } else {
        format!(
            "{}…{}",
            chars[..max.saturating_sub(9)].iter().collect::<String>(),
            chars[chars.len() - 8..].iter().collect::<String>()
        )
    }
}
fn quaternion(value: &Value) -> Option<[f64; 4]> {
    let a = value.as_array()?;
    if a.len() != 4 {
        return None;
    }
    let mut q = [0.; 4];
    for i in 0..4 {
        q[i] = a[i].as_f64()?;
    }
    let n = q.iter().map(|v| v * v).sum::<f64>().sqrt();
    if !n.is_finite() || n < 0.0001 {
        return None;
    }
    for v in &mut q {
        *v /= n;
    }
    Some(q)
}
fn orientation(p: &egui::Painter, center: Pos2, q: [f64; 4], fresh: bool) {
    let [w, x, y, z] = q;
    let axes = [
        [
            1. - 2. * (y * y + z * z),
            2. * (x * y + w * z),
            2. * (x * z - w * y),
        ],
        [
            2. * (x * y - w * z),
            1. - 2. * (x * x + z * z),
            2. * (y * z + w * x),
        ],
        [
            2. * (x * z + w * y),
            2. * (y * z - w * x),
            1. - 2. * (x * x + y * y),
        ],
    ];
    p.circle_stroke(center, 24., Stroke::new(1., Color32::from_rgb(48, 69, 94)));
    for (i, a) in axes.iter().enumerate() {
        let end =
            center + Vec2::new((a[0] + a[2] * 0.43) as f32, (-a[1] + a[2] * 0.32) as f32) * 20.;
        let color = if fresh {
            [MINT, VIOLET, CYAN][i]
        } else {
            MUTED.gamma_multiply(0.6)
        };
        p.arrow(center, end - center, Stroke::new(2., color));
    }
    p.circle_filled(center, 3., TEXT);
}
fn assign(selected: &mut [String; 3], role: usize, sensor: &str) -> bool {
    if selected[role] == sensor {
        return false;
    }
    for id in selected.iter_mut() {
        if id == sensor {
            id.clear();
        }
    }
    selected[role] = sensor.into();
    true
}

fn library(state:&Value,selected:&[String;3])->Vec<Value>{
    let mut rows=state["trackers"].as_array().cloned().unwrap_or_default();
    for id in selected.iter().filter(|id|!id.is_empty()) {
        if !rows.iter().any(|row|row["id"].as_str()==Some(id.as_str())) {
            rows.push(serde_json::json!({"id":id,"missing":true,"fresh":false,"peer_active":false}));
        }
    }
    rows
}
fn connection_label(sensor:Option<&Value>)->&'static str{
    match sensor {
        Some(s) if s["fresh"]==true => "Live",
        Some(s) if s["sample_mode"]=="held" => "Connected / quiet",
        Some(s) if s["peer_active"]==true => "Waiting for pose",
        Some(s) if s["missing"]!=true => "Stale / offline",
        _ => "Missing / no packets",
    }
}
/// Edits a draft in left-foot, right-foot, chest order. Returns true on change.
pub fn body_setup(
    ui: &mut egui::Ui,
    state: &Value,
    selected: &mut [String; 3],
    selected_sensor: &mut Option<String>,
) -> bool {
    let width = ui.available_width().max(320.).min(1400.);
    let height = 446.;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 16., Color32::from_rgb(12, 23, 42));
    painter.rect_stroke(
        rect,
        16.,
        Stroke::new(1., Color32::from_rgb(39, 59, 84)),
        egui::StrokeKind::Inside,
    );
    let tray_width = (width * 0.39).clamp(238., 410.);
    let divider = rect.left() + tray_width;
    painter.line_segment(
        [
            Pos2::new(divider, rect.top() + 20.),
            Pos2::new(divider, rect.bottom() - 20.),
        ],
        Stroke::new(1., Color32::from_rgb(39, 59, 84)),
    );
    label(
        &painter,
        rect.min + Vec2::new(20., 18.),
        "SENSOR LIBRARY",
        11.,
        MINT,
    );
    label(
        &painter,
        rect.min + Vec2::new(20., 38.),
        "Give each sensor a place",
        20.,
        TEXT,
    );
    label(
        &painter,
        rect.min + Vec2::new(20., 66.),
        "Drag a card, or select it then click a target.",
        11.,
        MUTED,
    );
    let tray = Rect::from_min_max(
        rect.min + Vec2::new(14., 94.),
        Pos2::new(divider - 14., rect.bottom() - 33.),
    );
    let mut changed = false;
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("body_sensor_tray")
            .max_rect(tray)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    egui::ScrollArea::vertical().id_salt("raw_sensor_cards").max_height(tray.height()).show(&mut child,|ui| {
        let rows=library(state,selected);
        let trackers=Some(&rows);
        if trackers.is_none_or(|v|v.is_empty()) {
            ui.add_space(32.); ui.colored_label(MUTED,"Waiting for sensor packets");
            ui.label("Turn on your sensors or add one over USB.");
        }
        for sensor in trackers.into_iter().flatten() {
            let Some(id)=sensor["id"].as_str() else {continue};
            let age=sensor["age_ms"].as_f64().filter(|a|a.is_finite()&&*a>=0.);
            let fresh=age.is_some_and(|a|a<=250.) && state["receiver_online"]!=false;
            let held=sensor["sample_mode"]=="held" && state["receiver_online"]!=false;
            let q=quaternion(&sensor["q"]);
            let active=selected_sensor.as_deref()==Some(id);
            let role=selected.iter().position(|s|s==id);
            let accent=role.map(|i|COLORS[i]).unwrap_or(MINT);
            let (card,response)=ui.allocate_exact_size(Vec2::new(ui.available_width().max(100.),104.),Sense::click_and_drag());
            response.dnd_set_drag_payload(SensorDrag(id.into()));
            if response.clicked() { *selected_sensor=Some(id.into()); }
            if response.drag_started() { *selected_sensor=Some(id.into()); }
            if response.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::Grab); }
            let p=ui.painter();
            p.rect_filled(card,10.,if active{Color32::from_rgb(25,47,65)}else{Color32::from_rgb(20,35,55)});
            p.rect_stroke(card,10.,Stroke::new(if active{1.5}else{1.},if active{accent}else{Color32::from_rgb(42,61,83)}),egui::StrokeKind::Inside);
            p.circle_filled(card.min+Vec2::new(15.,18.),3.,if fresh{MINT}else{Color32::from_rgb(217,163,92)});
            label(p,card.min+Vec2::new(26.,9.),short(id,26),13.,TEXT);
            label(p,card.min+Vec2::new(14.,31.),format!("{} · {}",if fresh{"Streaming"}else if held{"Quiet · last pose held"}else if sensor["peer_active"]==true{"Connected · awaiting rotation"}else if sensor["missing"]==true{"Assigned but no packets received"}else{"Stale / unavailable"},age.map(|v|format!("{v:.0} ms")).unwrap_or("age unknown".into())),10.,if fresh{MINT}else{MUTED});
            let now=ui.input(|i|i.time);
            let key=ui.id().with(("orientation_history",id));
            let mut speed=0.;
            if let Some(q)=q {
                let prev=ui.ctx().data_mut(|d|d.get_temp::<LastSample>(key));
                if let Some(old)=prev {
                    let dt=now-old.time;
                    let dot=old.q.iter().zip(q.iter()).map(|(a,b)|a*b).sum::<f64>().abs().clamp(0.,1.);
                    speed=if dt>0.001{(2.*dot.acos()/dt) as f32}else{old.speed};
                    if !speed.is_finite(){speed=0.;}
                }
                ui.ctx().data_mut(|d|d.insert_temp(key,LastSample{q,time:now,speed}));
                orientation(p,Pos2::new(card.right()-36.,card.top()+43.),q,fresh);
                let angle=(2.*q[0].abs().clamp(0.,1.).acos()).to_degrees();
                label(p,card.min+Vec2::new(14.,48.),format!("Sensor orientation  {angle:.0}°"),10.,MUTED);
            } else { label(p,card.min+Vec2::new(14.,48.),"Orientation unavailable",10.,MUTED); }
            let accel_fresh=sensor["acceleration_age_ms"].as_f64().is_some_and(|age|age<250.) && state["receiver_online"]!=false;
            let magnitude=sensor["acceleration_magnitude"].as_f64().unwrap_or(0.);
            label(p,card.min+Vec2::new(14.,64.),if accel_fresh {format!("Acceleration  {magnitude:.2} m/s²")}else{"Acceleration quiet / unavailable".into()},10.,if accel_fresh{accent}else{MUTED});
            let bar=Rect::from_min_size(card.min+Vec2::new(14.,87.),Vec2::new((card.width()-80.).max(30.),4.));
            p.rect_filled(bar,2.,Color32::from_rgb(41,59,78));
            let fraction=if accel_fresh {(magnitude as f32/5.).clamp(0.,1.)}else if fresh {(sensor["angular_speed"].as_f64().unwrap_or(speed as f64) as f32/5.).clamp(0.,1.)} else {0.};
            if fraction>0. {p.rect_filled(Rect::from_min_size(bar.min,Vec2::new(bar.width()*fraction,4.)),2.,accent);}
            response.on_hover_text(format!("{id}\nDrag to chest or a foot. Orientation is raw sensor rotation, not a tracked joint pose."));
            ui.add_space(7.);
        }
    });
    label(
        &painter,
        Pos2::new(rect.left() + 20., rect.bottom() - 24.),
        "XYZ axes · raw rotation · motion intensity",
        10.,
        MUTED,
    );
    let body = Rect::from_min_max(
        Pos2::new(divider + 12., rect.top() + 14.),
        rect.max - Vec2::new(12., 12.),
    );
    label(
        &painter,
        body.min + Vec2::new(12., 3.),
        "BODY MAP",
        11.,
        CYAN,
    );
    label(
        &painter,
        body.min + Vec2::new(12., 24.),
        if state["movement_engine"]=="body_tilt" {"Tilt to Move: CHEST only. Feet optional."}else{"Three points. One connected you."},
        17.,
        TEXT,
    );
    let cx = body.center().x;
    let top = body.top() + 77.;
    let scale = (body.width() / 390.).min(1.);
    let pt = |x: f32, y: f32| Pos2::new(cx + x * scale, top + y);
    // A quiet front-view silhouette keeps the three assignment targets prominent.
    let outline = Color32::from_rgb(58, 85, 112);
    let fill = Color32::from_rgb(24, 44, 66);
    painter.circle_filled(pt(0., 20.), 23., fill);
    painter.circle_stroke(pt(0., 20.), 23., Stroke::new(1.5, outline));
    painter.add(egui::Shape::convex_polygon(
        vec![
            pt(-18., 49.),
            pt(-56., 66.),
            pt(-42., 150.),
            pt(-31., 176.),
            pt(31., 176.),
            pt(42., 150.),
            pt(56., 66.),
            pt(18., 49.),
        ],
        fill,
        Stroke::new(1.5, outline),
    ));
    for side in [-1., 1.] {
        for (a, b, w) in [
            ((side * 53., 72.), (side * 74., 129.), 16.),
            ((side * 74., 129.), (side * 85., 180.), 13.),
            ((side * 19., 177.), (side * 28., 229.), 23.),
            ((side * 28., 229.), (side * 32., 283.), 18.),
        ] {
            painter.line_segment([pt(a.0, a.1), pt(b.0, b.1)], Stroke::new(w, outline));
            painter.line_segment([pt(a.0, a.1), pt(b.0, b.1)], Stroke::new(w - 3., fill));
        }
        painter.circle_filled(pt(side * 85., 188.), 8., fill);
    }
    painter.line_segment(
        [pt(0., 62.), pt(0., 155.)],
        Stroke::new(1., Color32::from_rgb(45, 69, 94)),
    );
    for side in [-1., 1.] {
        painter.line_segment(
            [pt(side * 32., 269.), pt(side * 88., 285.)],
            Stroke::new(1., outline),
        );
    }
    let target_specs = [
        (0, pt(88., 290.), "YOUR LEFT FOOT"),
        (1, pt(-88., 290.), "YOUR RIGHT FOOT"),
        (2, pt(0., 95.), "CHEST"),
    ];
    for (role, center, title) in target_specs {
        let target = Rect::from_center_size(
            center,
            Vec2::new(
                if role == 2 { 142. } else { 135. } * scale.max(0.75),
                if role == 2 { 58. } else { 67. },
            ),
        );
        let response = ui.interact(target, ui.id().with(("body_target", role)), Sense::click());
        let hovered = response.dnd_hover_payload::<SensorDrag>().is_some();
        if let Some(payload) = response.dnd_release_payload::<SensorDrag>() {
            changed |= assign(selected, role, &payload.0);
            *selected_sensor = None;
        }
        if response.clicked() {
            if let Some(sensor) = selected_sensor.take() {
                changed |= assign(selected, role, &sensor);
            }
        }
        if role < 2 {
            let steps = state["preview_steps"][ROLES[role]].as_u64().unwrap_or(0);
            let foot = state["roles"][ROLES[role]].clone();
            let force = foot["acceleration_magnitude"]
                .as_f64()
                .unwrap_or(0.)
                .min(5.) as f32;
            let recent = foot["acceleration_age_ms"]
                .as_f64()
                .is_some_and(|age| age < 250.);
            let at = pt(if role == 0 { 32. } else { -32. }, 252.);
            painter.circle_filled(
                at,
                if recent { 8. + force * 2. } else { 8. },
                COLORS[role].gamma_multiply(if recent { 0.65 } else { 0.2 }),
            );
            label(
                &painter,
                at + Vec2::new(if role == 0 { 17. } else { -72. }, -6.),
                format!("{steps} steps"),
                11.,
                COLORS[role],
            );
        }
        let set = !selected[role].is_empty();
        let tracker=state["trackers"].as_array().and_then(|rows| rows.iter().find(|s|s["id"].as_str()==Some(selected[role].as_str())));
        let live=tracker.is_some_and(|s|s["fresh"]==true||s["sample_mode"]=="held");
        let color = if set&&!live {Color32::from_rgb(231,157,83)} else {COLORS[role]};
        painter.rect_filled(
            target,
            10.,
            if hovered {
                Color32::from_rgb(36, 68, 82)
            } else {
                Color32::from_rgb(16, 32, 50)
            },
        );
        painter.rect_stroke(
            target,
            10.,
            Stroke::new(
                if hovered { 2.5 } else { 1.5 },
                if set || hovered { color } else { outline },
            ),
            egui::StrokeKind::Inside,
        );
        label(&painter, target.min + Vec2::new(10., 8.), title, 10., color);
        label(
            &painter,
            target.min + Vec2::new(10., 25.),
            if set {
                short(&selected[role], 19)
            } else {
                "+ Drop sensor".into()
            },
            11.,
            if set { TEXT } else { MUTED },
        );
        if set {
            let saved = state["roles"][ROLES[role]]["id"].as_str() == Some(selected[role].as_str());
            label(
                &painter,
                target.min + Vec2::new(10., 42.),
                if saved {
                    connection_label(tracker)
                } else {
                    "Unsaved assignment"
                },
                9.,
                MUTED,
            );
            let clear =
                Rect::from_min_size(target.right_top() + Vec2::new(-19., 3.), Vec2::splat(16.));
            if ui
                .interact(clear, ui.id().with(("clear_role", role)), Sense::click())
                .on_hover_text("Clear assignment")
                .clicked()
            {
                selected[role].clear();
                changed = true;
            }
            label(&painter, clear.min, "×", 13., MUTED);
        }
    }
    label(
        &painter,
        Pos2::new(body.left() + 12., body.bottom() - 16.),
        if selected_sensor.is_some() {
            "Sensor selected · click a body target"
        } else {
            "Front view · place sensors on your body"
        },
        10.,
        MUTED,
    );
    changed
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    fn frame(
        ctx: &egui::Context,
        state: &Value,
        selected: &mut [String; 3],
        picked: &mut Option<String>,
        events: Vec<egui::Event>,
        time: f64,
    ) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000., 700.))),
            events,
            time: Some(time),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                body_setup(ui, state, selected, picked);
            });
        });
    }
    fn button(at: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }
    #[test]
    fn missing_assignment_stays_visible_without_fake_motion(){
        let state=serde_json::json!({"trackers":[{"id":"a","fresh":true},{"id":"b","fresh":true}]});
        let rows=library(&state,&["a".into(),"b".into(),"c".into()]);
        assert_eq!(rows.len(),3);assert_eq!(rows[2]["missing"],true);
        assert!(rows[2]["q"].is_null());assert_eq!(connection_label(Some(&rows[2])),"Missing / no packets");
        assert_eq!(connection_label(Some(&rows[0])),"Live");
    }
    #[test]
    fn click_and_drag_assign_real_widget_without_network_side_effects() {
        let ctx = egui::Context::default();
        let state = serde_json::json!({"trackers":[{"id":"10.0.0.101:0","age_ms":20.,"q":[1.,0.,0.,0.]},{"id":"192.0.2.12:0","age_ms":30.,"q":[1.,0.,0.,0.]}]});
        let mut selected: [String; 3] = Default::default();
        let mut picked = None;
        frame(&ctx, &state, &mut selected, &mut picked, vec![], 0.);
        let card = Pos2::new(145., 146.);
        let foot = Pos2::new(780., 389.);
        frame(
            &ctx,
            &state,
            &mut selected,
            &mut picked,
            vec![egui::Event::PointerMoved(card), button(card, true)],
            0.1,
        );
        frame(
            &ctx,
            &state,
            &mut selected,
            &mut picked,
            vec![button(card, false)],
            0.2,
        );
        assert_eq!(picked.as_deref(), Some("10.0.0.101:0"));
        frame(
            &ctx,
            &state,
            &mut selected,
            &mut picked,
            vec![egui::Event::PointerMoved(foot), button(foot, true)],
            0.3,
        );
        frame(
            &ctx,
            &state,
            &mut selected,
            &mut picked,
            vec![button(foot, false)],
            0.4,
        );
        assert_eq!(selected[0], "10.0.0.101:0");
        let card2 = Pos2::new(145., 241.);
        let other_foot = Pos2::new(604., 389.);
        frame(
            &ctx,
            &state,
            &mut selected,
            &mut picked,
            vec![egui::Event::PointerMoved(card2), button(card2, true)],
            0.5,
        );
        frame(
            &ctx,
            &state,
            &mut selected,
            &mut picked,
            vec![egui::Event::PointerMoved(Pos2::new(180., 241.))],
            0.6,
        );
        frame(
            &ctx,
            &state,
            &mut selected,
            &mut picked,
            vec![egui::Event::PointerMoved(other_foot)],
            0.7,
        );
        frame(
            &ctx,
            &state,
            &mut selected,
            &mut picked,
            vec![button(other_foot, false)],
            0.8,
        );
        assert_eq!(selected[1], "192.0.2.12:0");
    }
}
