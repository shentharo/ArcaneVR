use bevy::image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
use bevy::animation::AnimationTarget;
use bevy::animation::transition::AnimationTransitions;
use bevy::transform::TransformSystem;
use bevy::math::Affine2;
use bevy::{
    gltf::Gltf,
    input::mouse::MouseMotion,
    prelude::*,
    render::view::screenshot::{save_to_disk, Screenshot},
};
use serde_json::Value;
mod foot_pose;
#[derive(Resource)]
struct FootRigResource(foot_pose::FootRig);
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Resource)]
struct Launch {
    quit: Arc<AtomicBool>,
    screenshot: Option<String>,
    capture_at: Option<f32>,
    captured: bool,
    controls: Arc<Mutex<Vec<String>>>,
    report: Option<String>,
    report_after: f32,
    series: Option<String>,
    last_series: f32,
    reported: bool,
    calibration: bool,
}
#[derive(Resource, Clone)]
struct Feed(Arc<Mutex<Option<(Value, Instant)>>>);
#[derive(Resource)]
struct Room {
    paused: bool,
    yaw: f32,
    raw_yaw: f32,
    heading_origin: f32,
    virtual_turn: f32,
    generation: Option<u64>,
    jump: JumpState,
    jump_test: bool,
    clip: &'static str,
    clip_speed: f32,
    clips_seen: Vec<String>,
    anim: String,
    speed: f32,
    max_walk_speed: f32,
    max_sprint_speed: f32,
    local: Vec2,
    visual_local: Vec2,
    body_lean: Vec2,
    lean_min: Vec2,
    lean_max: Vec2,
    foot_pose: [f32;2],
    foot_pose_max: [f32;2],
    foot_direct: bool,
    foot_profiles: usize,
    foot_running: bool,
    reason: String,
    last_stamp: Option<f64>,
    stamp_changed: Instant,
    camera_yaw: f32,
    camera_pitch: f32,
    started: Instant,
    seconds: Option<f32>,
    parent: Option<u32>,
    parent_check: Instant,
    distance: f32,
    spin: f32,
    live_frames: u64,
    animations_seen: Vec<String>,
}
#[derive(Resource)]
struct CharacterAsset(Handle<Gltf>);
#[derive(Component)]
struct Avatar;
#[derive(Component)]
struct Ground;
#[derive(Component)]
struct Hud;
#[derive(Component)]
struct FollowCamera;
#[derive(Resource)]
struct Clips {
    graph: Handle<AnimationGraph>,
    clips: Vec<(String, AnimationNodeIndex)>,
}
#[derive(Resource, Default)]
struct StrideTelemetry { active: bool, bones: usize, left: f32, right: f32, lean: f32, residual: usize }
#[derive(Component)]
struct StrideOffset(Quat);
fn remove_stride_offset(rotation: Quat, offset: Quat) -> Quat { (rotation*offset.inverse()).normalize() }
fn clear_stride_offsets(mut bones: Query<(&mut Transform,&mut StrideOffset)>) {
    for (mut transform,mut offset) in &mut bones {
        transform.rotation=remove_stride_offset(transform.rotation,offset.0);offset.0=Quat::IDENTITY;
    }
}
#[derive(Debug,Clone,Copy)]
struct StridePose { left:f32, right:f32, left_knee:f32, right_knee:f32, lean:f32, arm:f32 }
fn stride_pose(local: Vec2, elapsed: f32, jumping: bool) -> Option<StridePose> {
    if jumping || !local.is_finite() || local.x.abs()<0.015 || local.length()<0.015 { return None; }
    let lateral=(local.x.abs()/local.length()).clamp(0.,1.);
    let strength=(local.x.abs()*4.).clamp(0.,1.)*lateral;
    let sign=local.x.signum(); let cycle=(elapsed*6.0).sin();
    Some(StridePose { left: sign*(0.09+0.34*cycle)*strength, right:sign*(0.09-0.34*cycle)*strength,
        left_knee:cycle.max(0.)*0.38*strength, right_knee:(-cycle).max(0.)*0.38*strength,
        lean:sign*0.12*strength, arm:sign*cycle*0.14*strength })
}
const REAR_CAMERA_YAW: f32 = 0.0;
const REAR_CAMERA_PITCH: f32 = 0.32;
#[derive(Default)]
struct JumpState { elapsed: Option<f32>, was_pressed: bool }
impl JumpState {
    fn update(&mut self, pressed: bool, test: bool, dt: f32) {
        if (pressed && !self.was_pressed || test) && self.elapsed.is_none() { self.elapsed = Some(0.); }
        self.was_pressed = pressed;
        if let Some(t) = &mut self.elapsed { *t += dt; if *t >= 0.85 { self.elapsed = None; } }
    }
    fn clip(&self) -> Option<&'static str> { self.elapsed.map(|t| if t < 0.18 { "Jump_Start" } else if t < 0.65 { "Jump_Loop" } else { "Jump_Land" }) }
    fn height(&self) -> f32 { self.elapsed.map(|t| (std::f32::consts::PI * t / 0.85).sin().max(0.) * 0.5).unwrap_or(0.) }
}
fn animation_for(local: Vec2, relative_speed: f32, crouching: bool, jump: Option<&'static str>) -> (&'static str, f32) {
    if let Some(clip) = jump { return (clip, 1.); }
    if local.length() < 0.015 { return (if crouching { "Crouch_Idle_Loop" } else { "Idle_Loop" }, 1.); }
    if local.x.abs() >= local.y.abs() { return (if crouching { "Crouch_Idle_Loop" } else { "Idle_Loop" },1.); }
    let backwards = local.y < -0.015;
    let clip = if crouching { "Crouch_Fwd_Loop" } else if backwards { "Walk_Loop" } else if relative_speed > 1.01 { "Sprint_Loop" } else if relative_speed >= 0.45 { "Jog_Fwd_Loop" } else { "Walk_Loop" };
    (clip, if backwards { -relative_speed.clamp(0.35, 1.4) } else { relative_speed.clamp(0.35, 1.4) })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let get = |s: &str| args.windows(2).find(|w| w[0] == s).map(|w| w[1].clone());
    let port = get("--http-port")
        .and_then(|x| x.parse().ok())
        .unwrap_or(8811);
    let seconds = get("--seconds").and_then(|x| x.parse().ok());
    let parent = get("--parent-pid").and_then(|x| x.parse().ok());
    let asset_root = get("--asset-root").unwrap_or_else(|| "assets".into());
    let screenshot = get("--screenshot");
    let capture_at = get("--qa-capture-at").and_then(|s|s.parse::<f32>().ok()).filter(|v|v.is_finite() && *v>=0.1 && *v<=60.);
    let embedded = args.iter().any(|s|s=="--embedded");
    let calibration = args.iter().any(|s|s=="--calibration-preview");
    let report = get("--qa-report");
    let report_after=get("--qa-report-after").and_then(|s|s.parse::<f32>().ok()).filter(|n|n.is_finite() && (1. ..=120.).contains(n)).unwrap_or(10.);
    let series=get("--qa-series");
    let foot_dir=get("--foot-calibration-dir").map(std::path::PathBuf::from).unwrap_or_else(||std::path::PathBuf::from("feet-calibrations"));
    let controls=Arc::new(Mutex::new(Vec::new()));
    let queued=controls.clone();
    let quit = Arc::new(AtomicBool::new(false));
    let stop = quit.clone();
    std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if line.trim() == "quit" {
                stop.store(true, Ordering::Relaxed);
                break;
            }
            queued.lock().unwrap().push(line.trim().to_owned());
        }
    });
    let feed = Feed(Arc::new(Mutex::new(None)));
    let worker = feed.clone();
    std::thread::spawn(move || loop {
        if let Some(v) = fetch(port) {
            *worker.0.lock().unwrap() = Some((v, Instant::now()));
        }
        std::thread::sleep(Duration::from_millis(33));
    });
    App::new()
        .insert_resource(Launch {
            quit,
            screenshot,
            capture_at,
            captured: false,
            controls,
            report,
            report_after,series,last_series:0.,
            reported: false,
            calibration,
        })
        .insert_resource(feed)
        .init_resource::<StrideTelemetry>()
        .insert_resource(FootRigResource(foot_pose::FootRig::load(&foot_dir)))
        .insert_resource(Room {
            paused: false,
            yaw: 0.,
            raw_yaw: 0., heading_origin: 0., virtual_turn: 0., generation: None,
            jump: JumpState::default(), jump_test: false, clip:"Idle_Loop", clip_speed:1., clips_seen:Vec::new(),
            anim: "Idle".into(),
            speed: 0.,max_walk_speed:0.,max_sprint_speed:0.,
            local: Vec2::ZERO,
            visual_local: Vec2::ZERO, body_lean: Vec2::ZERO,lean_min:Vec2::ZERO,lean_max:Vec2::ZERO,foot_pose:[0.;2],foot_pose_max:[0.;2],foot_direct:false,foot_profiles:0,foot_running:false,
            reason: "Connecting".into(),
            last_stamp: None,
            stamp_changed: Instant::now(),
            camera_yaw: REAR_CAMERA_YAW,
            camera_pitch: REAR_CAMERA_PITCH,
            started: Instant::now(),
            seconds,
            parent,
            parent_check: Instant::now(),
            distance: 0.,spin:0.,live_frames:0,animations_seen:Vec::new(),
        })
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: asset_root,
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Arcane Meadow — sensor test".into(),
                        resolution: (1280., 720.).into(),
                        visible: !embedded,
                        decorations: !embedded,
                        ..default()
                    }),
                    ..default()
                }),
        )
        .add_systems(Startup, setup)
        .add_systems(PreUpdate, clear_stride_offsets)
        .add_systems(
            Update,
            (
                load_character,
                hide_weapon,
                apply_controls,
                move_avatar,
                camera,
                animate,
                hud,
                exit_checks,
            ).chain(),
        )
        .add_systems(PostUpdate, procedural_strafe.after(bevy::app::Animation).before(TransformSystem::TransformPropagate))
        .run();
}
fn fetch(port: u16) -> Option<Value> {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().ok()?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(150)).ok()?;
    s.set_read_timeout(Some(Duration::from_millis(150))).ok()?;
    s.set_write_timeout(Some(Duration::from_millis(150))).ok()?;
    write!(
        s,
        "GET /api/state HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut b = Vec::new();
    s.take(2_000_000).read_to_end(&mut b).ok()?;
    let split = b.windows(4).position(|w| w == b"\r\n\r\n")?;
    if !b.starts_with(b"HTTP/1.1 200") {
        return None;
    }
    serde_json::from_slice(&b[split + 4..]).ok()
}
fn preview_calibrated(v: &Value) -> bool {
    // Local tilt preview does not require a connected SteamVR driver. Its own
    // calibration is cleared by tracker remapping/reconnection and settings changes.
    match v["movement_engine"].as_str() {
        Some("body_tilt" | "body_tilt_steps") => {
            v["actions"]["preview"]["calibrated"] == true
                && v["actions"]["preview"]["direction_status"] == "ready"
        }
        _ => v["calibrated"] == true,
    }
}
fn number(v: &Value) -> f32 {
    v.as_f64().filter(|x| x.is_finite()).unwrap_or(0.) as f32
}
fn motion(v: &Value) -> Vec2 {
    // The receiver owns all mapping, speed, and slowdown. These are the same
    // float32 axes packed into ALC2, before the game-output arm gate.
    // Foot-step speed is gated to zero in the receiver while output is disarmed.
    // Test Room itself requires disarmed output, so use the independent live
    // step preview below for these modes.
    let foot_steps = v["step_input"] == "feet" && matches!(v["movement_engine"].as_str(), Some("body_tilt_steps" | "chest_heading_steps"));
    if !foot_steps { if let Some(axes)=v["game_output_preview"]["travel_xy"].as_array() {
        if axes.len()==2 {return Vec2::new(number(&axes[0]),number(&axes[1])).clamp_length_max(1.);}
    }}

    let mode = v["movement_engine"].as_str().unwrap_or("");
    let speed = number(&v["locomotion"]["preview"]["speed"]).clamp(0., 1.);
    let p = &v["actions"]["preview"];
    let slow = number(&p["speed_multiplier"]).clamp(0., 1.);
    let x = number(&p["travel_xy"][0]).clamp(-1., 1.);
    let y = number(&p["travel_xy"][1]).clamp(-1., 1.);
    let m = match mode {
        "body_tilt" => Vec2::new(x, y),
        "body_tilt_steps" => Vec2::new(x, y) * speed,
        "chest_heading_steps" => Vec2::new(0., speed),
        _ => Vec2::ZERO,
    };
    m.clamp_length_max(1.) * slow
}
fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    server: Res<AssetServer>,
    launch: Res<Launch>,
) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0., 3., 6.).looking_at(Vec3::Y, Vec3::Y),
        FollowCamera,
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 12000.,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.5, 0.)),
    ));
    commands.insert_resource(ClearColor(if launch.calibration {Color::srgb(0.055,0.095,0.13)} else {Color::srgb(0.53, 0.72, 0.82)}));
    commands.insert_resource(AmbientLight {
        color: Color::srgb(0.75, 0.85, 1.),
        brightness: 350.,
    });
    if !launch.calibration {
    let ground = server.load_with_settings("grass.png", |s: &mut ImageLoaderSettings| {
        s.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            ..default()
        });
    });
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(10000., 10000.))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color_texture: Some(ground),
            uv_transform: Affine2::from_scale(Vec2::splat(2500.)),
            ..default()
        })),
        Ground,
    ));
    // Decorative distant trees: scenery has no colliders and never blocks movement.
    // Flat ground markings stay in world space so travel is visible against the floor.
    let grid_material=materials.add(StandardMaterial{base_color:Color::srgb(0.24,0.41,0.29),unlit:true,..default()});
    for i in -40..=40 {
        let p=i as f32*2.;
        for turn in [false,true] {
            let position=if turn {Vec3::new(p,0.006,0.)}else{Vec3::new(0.,0.006,p)};
            commands.spawn((Mesh3d(meshes.add(Plane3d::default().mesh().size(160.,0.018))),MeshMaterial3d(grid_material.clone()),Transform::from_translation(position).with_rotation(Quat::from_rotation_y(if turn{std::f32::consts::FRAC_PI_2}else{0.}))));
        }
    }
    for i in 0..80 {
        let a = i as f32 * 2.39996;
        let r = 55. + (i % 9) as f32 * 9.;
        let x = a.sin() * r;
        let z = a.cos() * r;
        let h = 4. + (i % 4) as f32;
        commands.spawn((
            Mesh3d(meshes.add(Cone::new(2.2, h))),
            MeshMaterial3d(materials.add(Color::srgb(0.12 + ((i % 3) as f32) * 0.02, 0.29, 0.21))),
            Transform::from_xyz(x, h / 2., z),
        ));
    }
    }
    commands.spawn((Transform::default(), Visibility::default(), Avatar));
    if !launch.calibration {
    commands.spawn((
        Text::new("Connecting..."),
        TextFont {
            font_size: 14.,
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(18.),
            top: Val::Px(18.),
            ..default()
        },
        Hud,
    ));
    }
    commands.insert_resource(CharacterAsset(server.load("character.glb")));
}
fn load_character(
    mut commands: Commands,
    asset: Res<CharacterAsset>,
    gltfs: Res<Assets<Gltf>>,

    mut graphs: ResMut<Assets<AnimationGraph>>,
    avatar: Query<Entity, With<Avatar>>,
    existing: Option<Res<Clips>>,
) {
    if existing.is_some() {
        return;
    }
    let Some(g) = gltfs.get(&asset.0) else { return };
    let Some(scene) = g
        .default_scene
        .clone()
        .or_else(|| g.scenes.first().cloned())
    else {
        return;
    };
    let mut graph = AnimationGraph::new();
    let mut clips = Vec::new();
    for (name, clip) in &g.named_animations {
        let n = graph.add_clip(clip.clone(), 1., graph.root);
        clips.push((name.to_string(), n));
    }
    if clips.is_empty() {
        for (i, clip) in g.animations.iter().enumerate() {
            let n = graph.add_clip(clip.clone(), 1., graph.root);
            clips.push((format!("clip{i}"), n));
        }
    }
    commands.entity(avatar.single()).with_children(|p| {
        p.spawn((
            SceneRoot(scene),
            Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::PI))
                .with_scale(Vec3::splat(0.96)),
        ));
    });
    commands.insert_resource(Clips {
        graph: graphs.add(graph),
        clips,
    });
}
fn hide_weapon(mut q: Query<(&Name, &mut Visibility), Added<Name>>) {
    for (name, mut visibility) in &mut q {
        if name.as_str().to_lowercase().contains("cannon") {
            *visibility = Visibility::Hidden;
        }
    }
}
fn wrap_angle(angle: f32) -> f32 { angle.sin().atan2(angle.cos()) }
impl Room {
    fn observe_heading(&mut self, raw: f32, generation: Option<u64>) -> f32 {
        // A newly opened preview has no prior viewing reference. Start behind
        // the current facing, even when the receiver was calibrated earlier.
        if self.generation.is_none() && self.live_frames==0 {
            self.heading_origin=raw;
            self.camera_yaw=REAR_CAMERA_YAW;
            self.camera_pitch=REAR_CAMERA_PITCH;
        }
        if let Some(generation) = generation {
            if self.generation.is_some_and(|old| old != generation) {
                self.heading_origin = 0.;
                // A centre reset also restores the viewing reference. Keeping an
                // old orbit can put the camera in front of a newly centred avatar.
                self.camera_yaw = REAR_CAMERA_YAW;
                self.camera_pitch = REAR_CAMERA_PITCH;
            }
            self.generation = Some(generation);
        }
        self.raw_yaw = raw;
        wrap_angle(raw - self.heading_origin)
    }
    fn reset_field(&mut self, avatar: &mut Transform) {
        self.heading_origin = self.raw_yaw;
        self.yaw = 0.; self.virtual_turn = 0.; self.distance = 0.;
        self.camera_yaw = REAR_CAMERA_YAW; self.camera_pitch = REAR_CAMERA_PITCH;
        self.jump = JumpState::default(); self.jump_test = false;
        avatar.translation = Vec3::ZERO; avatar.rotation = Quat::IDENTITY;
    }
}
fn apply_controls(launch:Res<Launch>,feed:Res<Feed>,mut room:ResMut<Room>,mut avatar:Query<&mut Transform,With<Avatar>>) {
    for command in launch.controls.lock().unwrap().drain(..) {
        match command.as_str() {
            "pause"=>room.paused=true,
            "resume"=>room.paused=false,
            "reset"=>{ update_reset_heading(&feed, &mut room); room.reset_field(&mut avatar.single_mut()); },
            "rear"=>{room.camera_yaw=REAR_CAMERA_YAW;room.camera_pitch=REAR_CAMERA_PITCH;},
            "jump"=>room.jump_test=true,
            _=>{},
        }
    }
}
fn update_reset_heading(feed: &Feed, room: &mut Room) {
    if let Some((v, at)) = feed.0.lock().unwrap().as_ref() {
        if at.elapsed() <= Duration::from_millis(500) {
            if let Some(raw) = v["heading"].as_f64().filter(|v| v.is_finite()) {
                room.observe_heading(raw as f32, v["steamvr"]["generation"].as_u64());
            }
        }
    }
}
// Receiver turn is a normalized horizontal stick; positive X turns right.
fn advance_virtual_turn(current:f32, turn:&Value, dt:f32, live:bool)->f32 {
    let x=turn.as_array().and_then(|a|a.first()).and_then(Value::as_f64).filter(|v|v.is_finite()).unwrap_or(0.).clamp(-1.,1.) as f32;
    if live {wrap_angle(current-x*std::f32::consts::FRAC_PI_2*dt.clamp(0.,0.05))} else {current}
}
// Movement and facing are independent, as in Doce Fernandes's twin-stick example.
// Facing defines the forward/right basis; dots select relative leg animation.
fn world_velocity(local:Vec2,yaw:f32)->Vec3 {
    let looking=Quat::from_rotation_y(yaw)*Vec3::NEG_Z;
    let right=looking.cross(Vec3::Y);
    (right*local.x+looking*local.y)*2.
}
fn move_avatar(
    launch: Res<Launch>,
    feed: Res<Feed>,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut foot_rig: ResMut<FootRigResource>,
    mut room: ResMut<Room>,
    mut q: Query<&mut Transform, With<Avatar>>,
) {
    if keys.just_pressed(KeyCode::Space) {
        room.paused = !room.paused;
    }
    if keys.just_pressed(KeyCode::KeyR) {
        update_reset_heading(&feed, &mut room);
        room.reset_field(&mut q.single_mut());
    }
    if keys.just_pressed(KeyCode::KeyJ) { room.jump_test = true; }
    let snap = feed.0.lock().unwrap().clone();
    let Some((v, at)) = snap else {
        room.reason = "Waiting for API".into();
        room.speed = 0.;
        return;
    };
    let stamp = v["statistics"]["elapsed_seconds"].as_f64();
    if stamp.is_some() && stamp != room.last_stamp {
        room.last_stamp = stamp;
        room.stamp_changed = Instant::now();
    }
    if launch.calibration {
        let valid=at.elapsed()<=Duration::from_millis(500)
            && room.stamp_changed.elapsed()<=Duration::from_millis(500)
            && v["roles"]["torso"]["fresh"]==true
            && v["sensor_calibration"]["fresh"]==true;
        let angles=&v["sensor_calibration"]["visual_lean"];
        room.local=if valid && angles.is_array() {
            Vec2::new(number(&angles[0]),number(&angles[1])).clamp_length_max(0.62)
        } else {Vec2::ZERO};
        room.body_lean=room.local;
        room.reason=if valid {"Calibrating"}else{"Waiting for sensor"}.into();
        room.speed=0.;
        room.anim="Idle".into();
        room.clip="Idle_Loop";
        room.clip_speed=1.;
        let mut avatar=q.single_mut();
        avatar.translation=Vec3::ZERO;
        avatar.rotation=Quat::IDENTITY;
        if valid {room.live_frames+=1;}
        return;
    }
    let reason = if at.elapsed() > Duration::from_millis(500)
        || room.stamp_changed.elapsed() > Duration::from_millis(500)
        || stamp.is_none()
    {
        "API stale"
    } else if !preview_calibrated(&v) {
        "Centering sensors"
    } else if v["armed"] != false {
        "VR output armed"
    } else if v["roles"]["torso"]["fresh"] != true {
        "Chest stale"
    } else if !v["heading"].as_f64().is_some_and(f64::is_finite) {
        "Heading unavailable"
    } else if room.paused {
        "Paused locally"
    } else {
        "Live"
    };
    room.reason = reason.into();
    room.speed = 0.;
    room.local = Vec2::ZERO;
    room.anim = "Idle".into();
    if reason != "Live" {
        room.visual_local=Vec2::ZERO;room.body_lean=Vec2::ZERO;room.foot_pose=[0.;2];room.foot_direct=false;room.foot_running=false;
        room.jump = JumpState::default(); room.jump_test = false; room.clip="Idle_Loop"; room.clip_speed=1.;
        q.single_mut().translation.y = 0.;
        return;
    }
    let physical_yaw = room.observe_heading(number(&v["heading"]), v["steamvr"]["generation"].as_u64());
    room.virtual_turn = advance_virtual_turn(room.virtual_turn, &v["actions"]["preview"]["turn_xy"], time.delta_secs(), true);
    let yaw = wrap_angle(physical_yaw + room.virtual_turn);
    let delta=yaw-room.yaw;
    if room.live_frames>0 {room.spin+=delta.sin().atan2(delta.cos());}
    room.yaw = yaw;
    room.live_frames+=1;
    // A stale foot must stop translation, but it must not freeze the fresh
    // chest sensor's facing and lean while the user is standing still.
    let m = if v["locomotion"]["input_ready"] == true {motion(&v)} else {Vec2::ZERO};
    room.foot_direct=v["step_input"]=="feet";
    if room.foot_direct {
        let (feet,_)=foot_rig.0.update(&v,time.delta_secs());room.foot_pose=feet;room.foot_profiles=foot_rig.0.profile_count();room.foot_running=foot_rig.0.running();
        for (peak,value) in room.foot_pose_max.iter_mut().zip(feet) {*peak=peak.max(value);}
    } else {room.foot_pose=[0.;2];room.foot_running=false;}
    room.local = m;
    let lean=&v["actions"]["preview"]["raw_direction_angles"];
    let lean_target=if lean.is_array() {Vec2::new(number(&lean[0]),number(&lean[1])-number(&v["actions"]["settings"]["direction"]["forward_offset"])).clamp_length_max(0.7)} else {Vec2::ZERO};
    let blend=1.-(-time.delta_secs().min(0.1)/0.12).exp();
    room.body_lean=room.body_lean.lerp(lean_target,blend);
    room.lean_min=room.lean_min.min(room.body_lean);room.lean_max=room.lean_max.max(room.body_lean);
    room.visual_local=room.visual_local.lerp(m,blend);
    let mut t = q.single_mut();
    t.rotation = Quat::from_rotation_y(yaw);
    let pressed = v["actions"]["preview"]["jump_pressed"] == true;
    let test = std::mem::take(&mut room.jump_test);
    room.jump.update(pressed, test, time.delta_secs().min(0.05));
    t.translation.y = room.jump.height();
    let forward = t.rotation * Vec3::NEG_Z;
    let right = t.rotation * Vec3::X;
    let sprint=v["game_output_preview"]["buttons"].as_u64().unwrap_or(0)&4!=0 && m.length()>0.015;
    let world = world_velocity(m,yaw)*if sprint {1.2}else{1.};
    if sprint {t.rotation *= Quat::from_rotation_x(-0.12);}

    let travel=world*time.delta_secs().min(0.05);
    t.translation += travel;
    room.distance+=travel.length();
    room.speed = world.length();
    if sprint {room.max_sprint_speed=room.max_sprint_speed.max(room.speed);}else{room.max_walk_speed=room.max_walk_speed.max(room.speed);}
    let max_output = v["actions"]["settings"]["direction"]["max_output"].as_f64().filter(|v| v.is_finite() && *v > 0.).unwrap_or(1.) as f32;
    let visual_speed=if sprint {1.2}else{(room.visual_local.length()/max_output).clamp(0.,1.)};
    (room.clip, room.clip_speed) = animation_for(room.visual_local, visual_speed, v["actions"]["preview"]["crouching"] == true, room.jump.clip());
    if room.foot_direct && room.jump.clip().is_none() && v["actions"]["preview"]["crouching"]!=true {room.clip="Idle_Loop";room.clip_speed=1.;}
    let current_clip = room.clip.to_owned();
    if !room.clips_seen.contains(&current_clip) { room.clips_seen.push(current_clip); }
    if room.speed > 0.015 {
        let x = world.dot(right);
        let y = world.dot(forward);
        room.anim = direction(x, y).into();
    }
    let animation=room.anim.clone();
    if !room.animations_seen.contains(&animation){room.animations_seen.push(animation);}
}
fn direction(x: f32, y: f32) -> &'static str {
    let index = ((x.atan2(y) / std::f32::consts::FRAC_PI_4).round() as i32).rem_euclid(8);
    [
        "Forward",
        "Forward Right",
        "Right",
        "Backward Right",
        "Backward",
        "Backward Left",
        "Left",
        "Forward Left",
    ][index as usize]
}
fn camera_target(position: Vec3) -> Vec3 { position + Vec3::Y }
fn camera(
    launch: Res<Launch>,
    mut motion: EventReader<MouseMotion>,
    buttons: Res<ButtonInput<MouseButton>>,
    time: Res<Time>,
    mut room: ResMut<Room>,
    avatar: Query<&Transform, (With<Avatar>, Without<FollowCamera>)>,
    mut camera: Query<&mut Transform, (With<FollowCamera>, Without<Avatar>)>,
) {
    if launch.calibration {
        motion.clear();
        *camera.single_mut()=Transform::from_xyz(0.,0.91,-3.3).looking_at(Vec3::new(0.,0.91,0.),Vec3::Y);
        return;
    }
    if buttons.pressed(MouseButton::Right) {
        for m in motion.read() {
            room.camera_yaw -= m.delta.x * 0.006;
            room.camera_pitch = (room.camera_pitch + m.delta.y * 0.004).clamp(-0.1, 1.0);
        }
    } else {
        motion.clear();
    }
    // Follow continuously in world space; the grid and scenery never move.
    let target = camera_target(avatar.single().translation);
    let offset = Quat::from_rotation_y(room.camera_yaw)
        * Vec3::new(
            0.,
            room.camera_pitch.sin() * 7.5,
            room.camera_pitch.cos() * 7.5,
        );
    let mut t = camera.single_mut();
    let desired=target+offset;
    let blend = 1. - (-6. * time.delta_secs().min(0.1)).exp();
    t.translation=t.translation.lerp(desired,blend);
    t.look_at(target, Vec3::Y);
}
fn animate(
    mut commands: Commands,
    clips: Option<Res<Clips>>,
    room: Res<Room>,
    mut players: Query<(Entity, &mut AnimationPlayer, Option<&AnimationGraphHandle>, Option<&mut AnimationTransitions>)>,
) {
    let Some(clips) = clips else { return };
    for (e, mut player, graph, transitions) in &mut players {
        if graph.is_none() {
            commands
                .entity(e)
                .insert((AnimationGraphHandle(clips.graph.clone()),AnimationTransitions::new()));
        }
        if let (Some(mut transitions),Some((_, idx))) = (transitions,clips.clips.iter().find(|(name, _)| name == room.clip)) {
            if transitions.get_main_animation()!=Some(*idx) {
                transitions.play(&mut player,*idx,Duration::from_millis(180)).repeat().set_speed(room.clip_speed);
            } else if let Some(animation)=player.animation_mut(*idx) {
                animation.set_speed(room.clip_speed);
            }
        }
    }
}
// Applied after the GLB's animation each frame and before transform propagation.
// The free mannequin has no authored strafe clip; these local joint offsets layer
// a side-step over idle (or the forward/back clip for a longitudinal diagonal).
fn procedural_strafe(
    mut commands: Commands, launch: Res<Launch>, room: Res<Room>, time: Res<Time>, mut evidence: ResMut<StrideTelemetry>,
    mut bones: Query<(Entity, &Name, &mut Transform, Option<&mut StrideOffset>), With<AnimationTarget>>,
) {
    evidence.active=false; evidence.bones=0;evidence.residual=0;
    if launch.calibration {
        for (entity,name,mut transform,offset) in &mut bones {
            let weight=match name.as_str() {"spine_01"=>0.4,"spine_02"=>0.35,"spine_03"=>0.25,_=>continue};
            let rotation=Quat::from_rotation_z(-room.local.x*weight)*Quat::from_rotation_x(room.local.y*weight);
            transform.rotation=(transform.rotation*rotation).normalize();
            if let Some(mut previous)=offset {previous.0=rotation;} else {commands.entity(entity).insert(StrideOffset(rotation));}
            evidence.bones+=1;
        }
        evidence.active=evidence.bones==3;
        evidence.lean=room.local.x;
        return;
    }
    let pose=if room.reason=="Live" && !room.foot_direct {stride_pose(room.visual_local,time.elapsed_secs(),room.jump.elapsed.is_some())} else {None};
    for (entity,name,mut transform,offset) in &mut bones {
        if matches!(name.as_str(),"spine_01"|"spine_02"|"spine_03") {
            let weight=match name.as_str(){"spine_01"=>0.4,"spine_02"=>0.35,_=>0.25};
            let strafe=if name.as_str()=="spine_01" {pose.map_or(0.,|p|p.lean)}else{0.};
            let rotation=Quat::from_rotation_z(-room.body_lean.x*weight+strafe)*Quat::from_rotation_x(room.body_lean.y*weight);
            transform.rotation=(transform.rotation*rotation).normalize();
            if let Some(mut previous)=offset {previous.0=rotation;} else {commands.entity(entity).insert(StrideOffset(rotation));}
            evidence.bones+=1;continue;
        }
        let rotation=if room.foot_direct && room.jump.elapsed.is_none() {
            let [left,right]=room.foot_pose;
            match name.as_str() {
                "thigh_l"=>Quat::from_rotation_x(-0.85*left),"thigh_r"=>Quat::from_rotation_x(-0.85*right),
                "calf_l"=>Quat::from_rotation_x(1.4*left),"calf_r"=>Quat::from_rotation_x(1.4*right),
                "foot_l"=>Quat::from_rotation_x(-0.5*left),"foot_r"=>Quat::from_rotation_x(-0.5*right),
                "upperarm_l"=>Quat::from_rotation_x(-0.25*right),"upperarm_r"=>Quat::from_rotation_x(-0.25*left),
                _=>continue,
            }
        } else if let Some(pose)=pose {match name.as_str() {
            "thigh_l"=>Quat::from_rotation_z(pose.left),
            "thigh_r"=>Quat::from_rotation_z(pose.right),
            "calf_l"=>Quat::from_rotation_x(pose.left_knee),
            "calf_r"=>Quat::from_rotation_x(pose.right_knee),
            "foot_l"=>Quat::from_rotation_x(-pose.left_knee*0.6),
            "foot_r"=>Quat::from_rotation_x(-pose.right_knee*0.6),
            "upperarm_l"=>Quat::from_rotation_z(pose.arm),
            "upperarm_r"=>Quat::from_rotation_z(-pose.arm),
            _=>continue,
        }} else {continue;};
        transform.rotation=(transform.rotation*rotation).normalize();
        if let Some(mut previous)=offset {previous.0=rotation;} else {commands.entity(entity).insert(StrideOffset(rotation));}
        evidence.bones+=1;
    }
    evidence.active=room.foot_direct || pose.is_some();evidence.left=if room.foot_direct {room.foot_pose[0]}else{pose.map_or(0.,|p|p.left)};evidence.right=if room.foot_direct {room.foot_pose[1]}else{pose.map_or(0.,|p|p.right)};evidence.lean=room.body_lean.x;
}
fn hud(
    room: Res<Room>,
    asset: Res<CharacterAsset>,
    server: Res<AssetServer>,
    clips: Option<Res<Clips>>,
    mut q: Query<&mut Text, With<Hud>>,
) {
    let asset_status = if clips.is_some() {
        ""
    } else if server.is_loaded_with_dependencies(&asset.0) {
        "character has no scene"
    } else {
        "loading character.glb (check asset exists)"
    };
    let note = if !asset_status.is_empty() { asset_status } else if room.jump.elapsed.is_none() && room.local.x.abs()>0.015 { "Side-step: procedural animation" } else if room.clip_speed < 0. { "Backward animation: reversed walk" } else { "" };
    if let Ok(mut text)=q.get_single_mut() {
        **text=format!("{} | {}\nFacing: {:.0} deg | Speed: {:.2} m/s | Distance: {:.1} m\nSpace: pause | R: reset | J: test jump | Right-drag: orbit{}{}",room.reason,room.anim,room.yaw.to_degrees().rem_euclid(360.),room.speed,room.distance,if note.is_empty() { "" } else { "\n" },note);
    }
}
fn exit_checks(
    mut commands: Commands,
    mut launch: ResMut<Launch>,
    mut room: ResMut<Room>,
    mut exit: EventWriter<AppExit>,
    avatar: Query<&Transform,With<Avatar>>,
    ground: Query<&Transform,(With<Ground>,Without<Avatar>)>,
    clips: Option<Res<Clips>>,
    stride: Res<StrideTelemetry>,
) {
    let finishing=launch.quit.load(Ordering::Relaxed)||room.seconds.is_some_and(|s|room.started.elapsed().as_secs_f32()>=s-0.1);
    let elapsed=room.started.elapsed().as_secs_f32();
    if let Some(path)=launch.series.clone() {if elapsed-launch.last_series>=0.1 {
        if let Ok(mut file)=std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _=writeln!(file,"{}",serde_json::json!({"elapsed":elapsed,"stamp":room.last_stamp,"foot_pose":room.foot_pose,"foot_direct":room.foot_direct,"foot_profiles":room.foot_profiles,"foot_running":room.foot_running,"bones":stride.bones,"reason":room.reason,"clip":room.clip}));
        }
        launch.last_series=elapsed;
    }}
    if !launch.reported && (finishing||(room.seconds.is_none()&&elapsed>launch.report_after)) {
        if let Some(path)=&launch.report {
            let pos=avatar.single().translation;
            let fixed_ground=ground.iter().all(|g|g.translation==Vec3::ZERO);
            let value=serde_json::json!({"position":[pos.x,pos.y,pos.z],"speed_mps":room.speed,"max_walk_speed":room.max_walk_speed,"max_sprint_speed":room.max_sprint_speed,"local_input":[room.local.x,room.local.y],"body_lean":[room.body_lean.x,room.body_lean.y],"foot_pose":room.foot_pose,"foot_pose_max":room.foot_pose_max,"foot_direct":room.foot_direct,"foot_profiles":room.foot_profiles,"foot_running":room.foot_running,"lean_min":[room.lean_min.x,room.lean_min.y],"lean_max":[room.lean_max.x,room.lean_max.y],"distance_m":room.distance,"rotation_degrees":room.spin.to_degrees(),"facing_degrees":room.yaw.to_degrees(),"virtual_turn_degrees":room.virtual_turn.to_degrees(),"live_frames":room.live_frames,"animations_seen":room.animations_seen,"clips_seen":room.clips_seen,"clip":room.clip,"clip_speed":room.clip_speed,"fixed_ground":fixed_ground,"reason":room.reason,"character_loaded":clips.is_some(),"calibration_preview":launch.calibration,"procedural_strafe":{"active":stride.active,"bones":stride.bones,"left":stride.left,"right":stride.right,"lean":stride.lean,"residual_offsets":stride.residual}});
            let _=std::fs::write(path,serde_json::to_vec_pretty(&value).unwrap());
        }
        launch.reported=true;
    }
    if launch.quit.load(Ordering::Relaxed) {
        exit.send(AppExit::Success);
    }
    if !launch.captured
        && room.started.elapsed().as_secs_f32()
            > launch.capture_at.unwrap_or_else(|| room
                .seconds
                .map(|s| (s - 1.0).max(0.1).min(5.0))
                .unwrap_or(5.0))
    {
        if let Some(path) = launch.screenshot.clone() {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path));
            launch.captured = true;
        }
    }
    if room
        .seconds
        .is_some_and(|s| room.started.elapsed().as_secs_f32() >= s)
    {
        exit.send(AppExit::Success);
    }
    if let Some(pid) = room.parent {
        if room.parent_check.elapsed() > Duration::from_secs(2) {
            room.parent_check = Instant::now();
            #[cfg(windows)]
            {
                let output = std::process::Command::new("tasklist")
                    .args(["/FI", &format!("PID eq {pid}"), "/NH"])
                    .creation_flags(0x08000000)
                    .output();
                if output
                    .ok()
                    .is_some_and(|o| !String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
                {
                    exit.send(AppExit::Success);
                }
            }
        }
    }
}
#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(test)]
mod connection_regressions {
    use super::*;
    #[test]
    fn camera_follows_without_grid_boundary_jumps() {
        let before = camera_target(Vec3::new(0., 0., -3.99));
        let after = camera_target(Vec3::new(0., 0., -4.01));
        assert!(((after - before).length() - 0.02).abs() < 1e-4);
        let mut previous = camera_target(Vec3::ZERO);
        for step in 1..=500 {
            let target = camera_target(Vec3::new(0., 0., -(step as f32) * 0.01));
            assert!(target.is_finite() && (target - previous).length() < 0.011);
            previous = target;
        }
        let offset = Quat::from_rotation_y(REAR_CAMERA_YAW) * Vec3::new(0., REAR_CAMERA_PITCH.sin() * 7.5, REAR_CAMERA_PITCH.cos() * 7.5);
        assert!(offset.is_finite() && offset.z > 0.);
    }
    #[test]
    fn virtual_turn_uses_preview_and_freezes_when_not_live() {
        let right=serde_json::json!([1.0,0.0]);
        let turned=advance_virtual_turn(0.,&right,0.05,true);
        assert!(turned<0., "positive stick X rotates toward world right");
        assert_eq!(advance_virtual_turn(turned,&right,0.05,false),turned);
        assert_eq!(advance_virtual_turn(turned,&Value::Null,0.05,true),turned);
        let left=advance_virtual_turn(turned,&serde_json::json!([-1.,0.]),0.05,true);
        assert!(left.abs()<1e-6);
        assert!(world_velocity(Vec2::Y,turned).x>0.);
    }
    #[test]
    fn reset_field_references_current_heading_and_restores_view() {
        let mut room = Room { paused:false, yaw:0., raw_yaw:0., heading_origin:0., virtual_turn:0., generation:None,
            jump:JumpState::default(), jump_test:false, clip:"Idle_Loop", clip_speed:1., clips_seen:Vec::new(), anim:String::new(), speed:0., max_walk_speed:0., max_sprint_speed:0., local:Vec2::ZERO, visual_local:Vec2::ZERO,body_lean:Vec2::ZERO,lean_min:Vec2::ZERO,lean_max:Vec2::ZERO,foot_pose:[0.;2],foot_pose_max:[0.;2],foot_direct:false,foot_profiles:0,foot_running:false,reason:String::new(), last_stamp:None,
            stamp_changed:Instant::now(), camera_yaw:2., camera_pitch:0.8, started:Instant::now(),
            seconds:None, parent:None, parent_check:Instant::now(), distance:5., spin:0., live_frames:0, animations_seen:vec![] };
        let mut avatar = Transform::from_xyz(3., 0., 4.);
        let quarter = std::f32::consts::FRAC_PI_2;
        room.virtual_turn=1.;
        let first=room.observe_heading(std::f32::consts::PI,Some(1));
        assert!(first.abs()<1e-6,"reopening at a half turn must start facing away from camera");
        assert_eq!(room.camera_yaw,REAR_CAMERA_YAW);
        assert!((room.observe_heading(std::f32::consts::PI+0.2,Some(1))-0.2).abs()<1e-6,"subsequent physical turns must remain visible");
        room.observe_heading(quarter, Some(1)); room.reset_field(&mut avatar);
        assert_eq!(room.virtual_turn,0.);
        assert_eq!(avatar.translation, Vec3::ZERO); assert_eq!(avatar.rotation, Quat::IDENTITY);
        assert_eq!((room.yaw, room.distance, room.camera_yaw, room.camera_pitch), (0.,0.,REAR_CAMERA_YAW,REAR_CAMERA_PITCH));
        assert!(room.observe_heading(quarter, Some(1)).abs() < 1e-6);
        assert!((room.observe_heading(quarter + 15_f32.to_radians(), Some(1)) - 15_f32.to_radians()).abs() < 1e-6);
        room.heading_origin = 359_f32.to_radians();
        assert!((room.observe_heading(1_f32.to_radians(), Some(1)) - 2_f32.to_radians()).abs() < 1e-6);
        room.camera_yaw=std::f32::consts::PI;
        room.camera_pitch=0.8;
        assert!(room.observe_heading(0., Some(2)).abs() < 1e-6, "new calibration generation clears local origin");
        assert_eq!(room.camera_yaw,REAR_CAMERA_YAW,"centre reset must restore the rear camera after orbiting");
        assert_eq!(room.camera_pitch,REAR_CAMERA_PITCH);
        let toward_camera=Quat::from_rotation_y(room.camera_yaw)*Vec3::Z;
        assert!(world_velocity(Vec2::Y,0.).dot(toward_camera)<0.,"forward travels away from the reset rear camera");
    }
    #[test]
    fn rear_camera_and_model_heading_project_turns_correctly() {
        // The GLB's front is local +Z and its scene child rotates PI around Y.
        let model_front = Quat::from_rotation_y(std::f32::consts::PI) * Vec3::Z;
        assert!((model_front - Vec3::NEG_Z).length() < 1e-5);
        let camera_offset = Quat::from_rotation_y(REAR_CAMERA_YAW) * Vec3::new(0., REAR_CAMERA_PITCH.sin(), REAR_CAMERA_PITCH.cos());
        assert!(camera_offset.z > 0. && model_front.dot(camera_offset) < 0., "camera sees the back at zero heading");
        let camera_right = Vec3::NEG_Z.cross(Vec3::Y);
        let left = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2) * model_front;
        let right = Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2) * model_front;
        assert!(left.dot(camera_right) < -0.99, "positive yaw faces screen left");
        assert!(right.dot(camera_right) > 0.99, "negative yaw faces screen right");
        for yaw in [0., std::f32::consts::FRAC_PI_2, -std::f32::consts::FRAC_PI_2] {
            assert_eq!(world_velocity(Vec2::ZERO, yaw).length(), 0., "turning in place does not travel");
        }
    }
    #[test] fn world_travel_and_facing_are_independent_through_full_turn() {
        for step in 0..=72 {
            let yaw=step as f32*std::f32::consts::TAU/72.;
            let f=Quat::from_rotation_y(yaw)*Vec3::NEG_Z;
            let r=f.cross(Vec3::Y);
            for (local,label) in [(Vec2::Y,"Forward"),(-Vec2::Y,"Backward"),(Vec2::X,"Right"),(-Vec2::X,"Left")] {
                let v=world_velocity(local,yaw);
                assert!((v.length()-2.).abs()<1e-5);
                assert_eq!(direction(v.dot(r),v.dot(f)),label);
                let after=Vec3::ZERO+v*0.25;
                assert!((after.length()-0.5).abs()<1e-5,"avatar must translate across ground");
            }
        }
    }
    #[test]
    fn request_includes_receiver_port() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            loop {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let expected = format!("\r\nHost: 127.0.0.1:{port}\r\n");
            let valid = String::from_utf8(request).unwrap().contains(&expected);
            let body = if valid { "{\"ok\":true}" } else { "{}" };
            write!(
                stream,
                "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                if valid { "200 OK" } else { "403 Forbidden" },
                body.len(),
                body
            )
            .unwrap();
            valid
        });
        assert_eq!(fetch(port).unwrap()["ok"], true);
        assert!(server.join().unwrap());
    }
    #[test]
    fn local_tilt_uses_own_calibration_but_does_not_bypass_reset() {
        let mut v = serde_json::json!({"movement_engine":"body_tilt","calibrated":false,"actions":{"preview":{"calibrated":true,"direction_status":"ready"}}});
        assert!(preview_calibrated(&v));
        v["actions"]["preview"]["calibrated"] = Value::Bool(false);
        assert!(!preview_calibrated(&v));
        v["movement_engine"] = Value::String("chest_heading_steps".into());
        assert!(!preview_calibrated(&v));
    }
    #[test]
    #[ignore = "requires the running ArcaneVR receiver on port 8811"]
    fn actual_receiver_connection() {
        let v = fetch(8811).expect("real receiver rejected or failed the request");
        assert!(v["statistics"]["elapsed_seconds"].is_number());
        // This test verifies the real HTTP contract, independently of tracker power.
        // Physical freshness is reported separately; it must never be fabricated here.
        assert!(v["roles"]["torso"]["fresh"].is_boolean());
        assert!(v["actions"]["preview"]["travel_xy"].is_array());
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sideways_pose_alternates_and_preserves_heading() {
        let left=Vec2::new(-0.35,0.);let right=Vec2::new(0.35,0.);
        assert_eq!(animation_for(left,1.,false,None).0,"Idle_Loop");
        assert_eq!(animation_for(right,1.,false,None).0,"Idle_Loop");
        let left_a=stride_pose(left,std::f32::consts::PI/12.,false).unwrap();
        let left_b=stride_pose(left,std::f32::consts::PI/4.,false).unwrap();
        let right_a=stride_pose(right,std::f32::consts::PI/12.,false).unwrap();
        assert!(left_a.lean<0. && right_a.lean>0.);
        assert!((left_a.left-left_a.right)*(left_b.left-left_b.right)<0.,"legs alternate during a stride");
        assert!((left_a.left+right_a.left).abs()<1e-6);
        assert!(stride_pose(Vec2::Y,1.,false).is_none());assert!(stride_pose(left,1.,true).is_none());
        assert_eq!(world_velocity(left,0.).z,0.);assert!(world_velocity(left,0.).x<0.);
        assert!((Quat::from_rotation_y(0.)*Vec3::NEG_Z-Vec3::NEG_Z).length()<1e-6);
    }
    #[test]
    fn procedural_joint_offsets_are_removed_even_without_animation_keys() {
        let original=Quat::from_euler(EulerRot::XYZ,0.2,-0.1,0.3);
        let mut current=original;let mut previous=Quat::IDENTITY;
        for frame in 0..1000 {
            current=remove_stride_offset(current,previous);
            let p=stride_pose(Vec2::new(-0.35,0.),frame as f32*0.016,false).unwrap();
            previous=Quat::from_rotation_z(p.left);
            current=(current*previous).normalize();
        }
        current=remove_stride_offset(current,previous);
        assert!(current.dot(original).abs()>0.99999,"stopping restores the exact unmodified base pose");
    }
    #[test]
    fn clip_selection_and_jump_edge() {
        assert_eq!(animation_for(Vec2::Y, 0.3, false, None).0, "Walk_Loop");
        assert_eq!(animation_for(Vec2::Y, 0.6, false, None).0, "Jog_Fwd_Loop");
        assert_eq!(animation_for(Vec2::Y, 1.0, false, None).0, "Jog_Fwd_Loop");
        let (clip, speed) = animation_for(-Vec2::Y, 0.8, false, None);
        assert_eq!(clip, "Walk_Loop"); assert!(speed < 0.);
        assert_eq!(animation_for(Vec2::ZERO, 0., true, None).0, "Crouch_Idle_Loop");
        let mut jump = JumpState::default();
        jump.update(true, false, 0.01); assert_eq!(jump.clip(), Some("Jump_Start"));
        jump.update(true, false, 0.3); assert_eq!(jump.clip(), Some("Jump_Loop")); assert!(jump.height() > 0.);
        jump.update(true, false, 0.4); assert_eq!(jump.clip(), Some("Jump_Land"));
        jump.update(true, false, 0.2); assert_eq!(jump.clip(), None);
        jump.update(true, false, 0.01); assert_eq!(jump.clip(), None, "held pulse cannot retrigger");
        jump.update(false, false, 0.01); jump.update(true, false, 0.01); assert_eq!(jump.clip(), Some("Jump_Start"));
    }
    #[test]
    fn eight_directions() {
        assert_eq!(direction(0., 1.), "Forward");
        assert_eq!(direction(1., 0.), "Right");
        assert_eq!(direction(0., -1.), "Backward");
        assert_eq!(direction(-1., 1.), "Forward Left");
    }
    #[test]
    fn mode_scaling() {
        let v = serde_json::json!({"movement_engine":"body_tilt_steps","locomotion":{"preview":{"speed":0.5}},"actions":{"preview":{"travel_xy":[0.4,-0.8],"speed_multiplier":0.5}}});
        assert!((motion(&v) - Vec2::new(0.1, -0.2)).length() < 0.0001);
        let v = serde_json::json!({"movement_engine":"body_tilt_steps","step_input":"feet","game_output_preview":{"travel_xy":[0,0],"armed":false},"locomotion":{"preview":{"speed":0.5}},"actions":{"preview":{"travel_xy":[0.4,-0.8],"speed_multiplier":0.5}}});
        assert!((motion(&v) - Vec2::new(0.1, -0.2)).length() < 0.0001);
    }
}



