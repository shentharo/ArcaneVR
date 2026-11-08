//! Parent-owned embedded Bevy viewer. The Rust receiver alone owns sensors.
use eframe::egui::{self, Color32};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use serde_json::{json, Value};
use std::{io::Write, path::PathBuf, process::{Child, Command, Stdio}, time::{Duration, Instant}};

#[derive(Default)]
pub struct BevyRoom {
    child: Option<Child>, stdin: Option<std::sync::mpsc::SyncSender<String>>, message: String,
    retry_after: Option<Instant>, stopping: Option<std::sync::mpsc::Receiver<()>>,
    attempted: bool, paused: bool, page_visible: bool,
    calibration_preview: bool,
    sensor_calibration: crate::sensor_calibration::SensorCalibration,
    qa_calibration_opened: bool,
    #[cfg(windows)] window: Option<isize>,
    #[cfg(windows)] shown: bool,
    #[cfg(windows)] last_rect: Option<(i32, i32, i32, i32)>,
    #[cfg(windows)] parent_window: Option<isize>,
}
impl BevyRoom {
    pub fn sensor_calibration_open(&self) -> bool { self.sensor_calibration.open }
    pub fn start_sensor_calibration(&mut self, state: &Value) { self.sensor_calibration.start_with_state(state); }
    pub fn open_qa_calibration(&mut self) {
        if !self.qa_calibration_opened && std::env::args().any(|arg| arg == "--qa-sensor-calibration") {
            self.qa_calibration_opened=true;
            self.sensor_calibration.start();
        }
    }
    pub fn show_sensor_calibration(&mut self, ui:&mut egui::Ui, state:&Value, port:u16, frame:&eframe::Frame, allow_spawn:bool, backend_error:&str, online:bool) -> Option<Value> {
        let (command,rect)=self.sensor_calibration.show(ui, state, backend_error, online);
        if !self.sensor_calibration.open {
            self.stop_calibration_preview();
            return command;
        }
        if !self.calibration_preview {
            self.stop();
            self.calibration_preview=true;
            self.attempted=false;
        }
        self.page_visible=true;
        if allow_spawn && self.stopping.is_none() && (!self.attempted || self.retry_after.is_some_and(|t| Instant::now() >= t)) {
            self.attempted=true; self.retry_after=Some(Instant::now()+Duration::from_secs(3));
            if let Err(e)=self.start(port) {self.message=e;}
        }
        if let Some(rect)=rect {
            if self.child.is_none() {
                ui.painter().text(rect.center(),egui::Align2::CENTER_CENTER,
                    if self.message.is_empty() {"Starting character…"} else {&self.message},
                    egui::FontId::proportional(16.),Color32::LIGHT_GRAY);
            }
            #[cfg(windows)] self.embed(frame,rect,ui.ctx().pixels_per_point());
            #[cfg(not(windows))] {let _=frame;}
        }
        if self.child.is_some() {ui.ctx().request_repaint_after(Duration::from_millis(100));}
        command
    }
    pub fn stop_calibration_preview(&mut self) {
        if self.calibration_preview {
            #[cfg(windows)] self.hide();
            self.stop();
            self.calibration_preview=false;
            self.attempted=false;
        }
    }
    fn paths() -> Option<(PathBuf, PathBuf)> {
        let root = std::env::current_exe().ok()?.parent()?.to_path_buf();
        let installed = root.join("bevy-test-room");
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bevy-test-room");
        for base in [installed, dev] {
            for exe in [base.join("bin/arcane-test-room.exe"), base.join("target/release/arcane-test-room.exe")] {
                if exe.is_file() { return Some((exe, base.join("assets"))); }
            }
        }
        None
    }
    pub fn poll(&mut self) {
        if self.stopping.as_ref().is_some_and(|rx| !matches!(rx.try_recv(),Err(std::sync::mpsc::TryRecvError::Empty))) { self.stopping=None; }
        if let Some(child) = &mut self.child {
            if let Ok(Some(code)) = child.try_wait() {
                self.message = format!("Movement viewer exited: {code}");
                self.child = None; self.stdin = None; self.retry_after=Some(Instant::now()+Duration::from_secs(3));
                #[cfg(windows)] { self.window = None; self.shown = false; self.last_rect = None; }
            }
        }
    }
    fn start(&mut self, port: u16) -> Result<(), String> {
        if self.child.is_some() { return Ok(()); }
        let (exe, assets) = Self::paths().ok_or("Bevy movement viewer is not installed")?;
        let log = std::env::current_exe().map_err(|e| e.to_string())?.parent().ok_or("No executable directory")?.join("bevy-test-room.log");
        let file = std::fs::File::create(log).map_err(|e| e.to_string())?;
        let mut cmd = Command::new(exe);
        cmd.args(["--embedded", "--http-port", &port.to_string(), "--parent-pid", &std::process::id().to_string(), "--asset-root", &assets.to_string_lossy()])
            .stdin(Stdio::piped()).stdout(Stdio::from(file.try_clone().map_err(|e| e.to_string())?)).stderr(Stdio::from(file));
        if self.calibration_preview {cmd.arg("--calibration-preview");}
        if std::env::var_os("ARCANEVR_EMBED_QA").is_some() {
            if let Some(path) = std::env::var_os("ARCANEVR_VIEWER_QA_SCREENSHOT") { cmd.arg("--screenshot").arg(path); }
            if let Some(path) = std::env::var_os("ARCANEVR_VIEWER_QA_REPORT") { cmd.arg("--qa-report").arg(path); }
        }
        #[cfg(windows)] { use std::os::windows::process::CommandExt; cmd.creation_flags(0x08000000); }
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        if let Some(mut input)=child.stdin.take() {
            let (tx,rx)=std::sync::mpsc::sync_channel::<String>(8);
            std::thread::spawn(move || { while let Ok(command)=rx.recv() { if writeln!(input,"{command}").and_then(|_|input.flush()).is_err() {break;} } });
            self.stdin=Some(tx);
        }
        self.retry_after=None; self.child = Some(child); self.paused = false;
        self.message = "Starting movement viewer…".into(); Ok(())
    }
    fn send(&mut self, command: &str) {
        if let Some(stdin) = &mut self.stdin {
            if let Err(e) = stdin.try_send(command.to_string()) { self.message = format!("Viewer command failed: {e}"); }
        }
    }
    /// Call on every app update, including when the room page is not drawn.
    pub fn set_page_visible(&mut self, visible: bool) {
        self.page_visible = visible;
        #[cfg(windows)] if !visible { self.hide(); }
    }
    pub fn stop(&mut self) {
        self.send("quit"); self.stdin = None;
        if let Some(mut child) = self.child.take() {
            let (tx,rx)=std::sync::mpsc::channel(); self.stopping=Some(rx);
            std::thread::spawn(move || {
            let until = Instant::now() + Duration::from_secs(2);
            while Instant::now() < until {
                if child.try_wait().ok().flatten().is_some() { break; }
                std::thread::sleep(Duration::from_millis(20));
            }
            if child.try_wait().ok().flatten().is_none() { let _ = child.kill(); let _ = child.wait(); }
            let _=tx.send(());
            });
        }
        #[cfg(windows)] { self.window = None; self.shown = false; self.last_rect = None; }
    }
    /// `allow_spawn` must be false for normal QA captures; opt in explicitly for embedding QA.
    pub fn show(&mut self, ui: &mut egui::Ui, state: &Value, port: u16, frame: &eframe::Frame, allow_spawn: bool, online: bool) -> Option<Value> {
        self.stop_calibration_preview();
        let mut command = None;
        if !self.sensor_calibration.open && !self.sensor_calibration.notice().is_empty() { ui.colored_label(Color32::YELLOW,self.sensor_calibration.notice()); }
        if allow_spawn && self.stopping.is_none() && (!self.attempted || self.retry_after.is_some_and(|t| Instant::now() >= t)) {
            self.attempted = true; self.retry_after=Some(Instant::now()+Duration::from_secs(3));
            if let Err(e) = self.start(port) { self.message = e; }
        }
        let size = egui::vec2(ui.available_width().max(1.0), 420.0);
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        ui.painter().rect_filled(rect, 4.0, Color32::from_rgb(12, 22, 33));
        #[cfg(windows)] if self.sensor_calibration.open { self.hide(); } else { self.embed(frame, rect.intersect(ui.clip_rect()), ui.ctx().pixels_per_point()); }
        #[cfg(not(windows))] { let _ = frame; self.message = "Embedded movement viewer is available on Windows.".into(); }
        if !self.message.is_empty() { ui.label(&self.message); }
        if state["hmd_pose"]["fresh"] != true {
            ui.colored_label(Color32::from_rgb(237,188,119), "Headset disconnected · chest-only test. Connect the headset and recenter for headset controls.");
        }
        ui.label(if state["step_input"]=="chest" {"Stand upright and Reset Center. Lean to steer; walk in place to move forward. Learn walking in Steps first."} else {"Stand upright and Reset Center before moving. Lean to move; turn your body to change facing."});
        if self.child.is_some() { ui.ctx().request_repaint_after(Duration::from_millis(100)); }
        command
    }
    #[cfg(windows)]
    fn hide(&mut self) {
        if self.shown { if let Some(hwnd) = self.window { unsafe { win::ShowWindow(hwnd, 0); } self.report(hwnd, "hidden"); } self.shown = false; }
    }
    #[cfg(windows)]
    fn report(&self, hwnd: isize, event: &str) {
        let Some(path) = std::env::var_os("ARCANEVR_EMBED_QA_REPORT") else { return; };
        let (parent, style, visible, rect) = unsafe {
            let parent = win::GetParent(hwnd);
            let mut rect = win::Rect::default();
            let valid = win::GetWindowRect(hwnd, &mut rect) != 0;
            if valid { win::MapWindowPoints(0, parent, &mut rect as *mut _ as *mut win::Point, 2); }
            (parent, win::GetWindowLongPtrW(hwnd, -16), win::IsWindowVisible(hwnd) != 0, rect)
        };
        let data = json!({"event":event,"pid":self.child.as_ref().map(Child::id),"child_hwnd":hwnd,"parent_hwnd":self.parent_window,"get_parent":parent,"ws_child":style & 0x40000000 != 0,"visible":visible,"rect":{"x":rect.left,"y":rect.top,"width":rect.right-rect.left,"height":rect.bottom-rect.top}});
        let history=std::path::PathBuf::from(&path).with_extension("jsonl");
        if let Ok(mut file)=std::fs::OpenOptions::new().create(true).append(true).open(history){let _=writeln!(file,"{data}");}
        if let Err(e) = std::fs::write(path, data.to_string()) { eprintln!("Bevy embed QA report failed: {e}"); }
    }
    #[cfg(windows)]
    fn embed(&mut self, frame: &eframe::Frame, rect: egui::Rect, scale: f32) {
        if !self.page_visible || !rect.is_positive() { self.hide(); return; }
        let Some(pid) = self.child.as_ref().map(Child::id) else { return; };
        let parent = match frame.window_handle().ok().map(|h| h.as_raw()) {
            Some(RawWindowHandle::Win32(h)) => h.hwnd.get(),
            _ => { self.message = "Unable to obtain the ArcaneVR window handle.".into(); return; }
        };
        if self.window.is_none() {
            let mut search = win::Search { pid, hwnd: 0 };
            unsafe { win::EnumWindows(Some(win::find_window), &mut search as *mut _ as isize); }
            if search.hwnd == 0 { return; }
            let hwnd = search.hwnd;
            unsafe {
                let style = win::GetWindowLongPtrW(hwnd, -16);
                // A child must have WS_CHILD set and WS_POPUP and window decorations cleared before SetParent.
                let child_style = (style | 0x40000000) & !(0x80000000u32 as isize | 0x00c00000 | 0x00040000 | 0x00080000 | 0x00020000 | 0x00010000);
                win::SetLastError(0);
                if win::SetWindowLongPtrW(hwnd, -16, child_style) == 0 && win::GetLastError() != 0 { self.message = "Could not set viewer child style.".into(); return; }
                win::SetLastError(0);
                if win::SetParent(hwnd, parent) == 0 && win::GetLastError() != 0 { self.message = format!("Could not embed viewer (Windows error {}).", win::GetLastError()); return; }
            }
            self.window = Some(hwnd); self.parent_window = Some(parent); self.message.clear();
            eprintln!("Bevy embed: pid={pid} child_hwnd=0x{hwnd:x} parent_hwnd=0x{parent:x}");
        }
        let hwnd = self.window.unwrap();
        // Winit can finish applying its initial window style after the first
        // SetParent call. Recheck the actual child relationship, rather than
        // trusting a successful call made during renderer startup.
        let detached = unsafe { win::GetParent(hwnd) != parent || win::GetWindowLongPtrW(hwnd,-16) & 0x40000000 == 0 };
        if detached {
            unsafe {
                let style=win::GetWindowLongPtrW(hwnd,-16);
                let child_style=(style | 0x40000000) & !(0x80000000u32 as isize | 0x00c00000 | 0x00040000 | 0x00080000 | 0x00020000 | 0x00010000);
                win::SetLastError(0);
                if win::SetWindowLongPtrW(hwnd,-16,child_style)==0 && win::GetLastError()!=0 { self.message="Could not restore viewer child style".into();return; }
                win::SetLastError(0);
                if win::SetParent(hwnd,parent)==0 && win::GetLastError()!=0 { self.message="Could not restore embedded viewer".into();return; }
            }
            self.last_rect=None;self.shown=false;
        }
        let bounds = ((rect.min.x * scale).round() as i32, (rect.min.y * scale).round() as i32, (rect.width() * scale).round().max(1.0) as i32, (rect.height() * scale).round().max(1.0) as i32);
        if self.last_rect != Some(bounds) || !self.shown {
            let (x, y, w, h) = bounds;
            // SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED; show only on transition.
            let flags = 0x0004 | 0x0010 | 0x0020 | if self.shown { 0 } else { 0x0040 };
            if unsafe { win::SetWindowPos(hwnd, 0, x, y, w, h, flags) } != 0 {
                self.shown = true; self.last_rect = Some(bounds);
                eprintln!("Bevy embed rect: child_hwnd=0x{hwnd:x} parent_hwnd=0x{parent:x} x={x} y={y} width={w} height={h} scale={scale}");
                self.report(hwnd, "positioned");
            } else { self.message = format!("Could not position viewer (Windows error {}).", unsafe { win::GetLastError() }); }
        }
    }
}
impl Drop for BevyRoom { fn drop(&mut self) { self.stop(); } }

#[cfg(windows)]
mod win {
    pub struct Search { pub pid: u32, pub hwnd: isize }
    #[repr(C)] #[derive(Default)] pub struct Rect { pub left:i32, pub top:i32, pub right:i32, pub bottom:i32 }
    #[repr(C)] pub struct Point { pub x:i32, pub y:i32 }
    #[link(name = "user32")]
    extern "system" {
        pub fn EnumWindows(callback: Option<unsafe extern "system" fn(isize, isize) -> i32>, param: isize) -> i32;
        pub fn GetWindowThreadProcessId(hwnd: isize, pid: *mut u32) -> u32;
        pub fn GetWindow(hwnd: isize, command: u32) -> isize;
        pub fn GetWindowTextW(hwnd: isize, text: *mut u16, max_count: i32) -> i32;
        pub fn GetWindowLongPtrW(hwnd: isize, index: i32) -> isize;
        pub fn SetWindowLongPtrW(hwnd: isize, index: i32, value: isize) -> isize;
        pub fn SetParent(hwnd: isize, parent: isize) -> isize;
        pub fn GetParent(hwnd: isize) -> isize;
        pub fn GetWindowRect(hwnd: isize, rect: *mut Rect) -> i32;
        pub fn MapWindowPoints(from: isize, to: isize, points: *mut Point, count: u32) -> i32;
        pub fn IsWindowVisible(hwnd: isize) -> i32;
        pub fn SetWindowPos(hwnd: isize, after: isize, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
        pub fn ShowWindow(hwnd: isize, command: i32) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" { pub fn SetLastError(code: u32); pub fn GetLastError() -> u32; }
    pub unsafe extern "system" fn find_window(hwnd: isize, param: isize) -> i32 {
        let search = &mut *(param as *mut Search);
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == search.pid && GetWindow(hwnd, 4) == 0 {
            let mut text = [0u16; 256];
            let len = GetWindowTextW(hwnd, text.as_mut_ptr(), text.len() as i32).max(0) as usize;
            if String::from_utf16_lossy(&text[..len]).starts_with("Arcane Meadow") { search.hwnd = hwnd; return 0; }
        }
        1
    }
}

