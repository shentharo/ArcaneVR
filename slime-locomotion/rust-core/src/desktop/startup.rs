//! Desktop-only local preferences and Windows hotkeys. Never arms movement.
use eframe::egui;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};

#[derive(Clone, Copy)]
pub enum Hotkey {
    ResetCenter,
    Stop,
}
struct HotkeyWorker {
    halt: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    events: mpsc::Receiver<Hotkey>,
}
pub struct Startup {
    hotkeys_enabled: bool,
    reset_registered: bool,
    stop_registered: bool,
    minimize_pending: bool,
    registration_error: String,
    hotkey_worker: Option<HotkeyWorker>,
    commands: mpsc::Sender<Value>,
}
impl Startup {
    pub fn new(args: &[String], commands: mpsc::Sender<Value>) -> Self {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("VRCraft.exe"));
        let config = args
            .windows(2)
            .find(|p| p[0] == "--config")
            .map(|p| PathBuf::from(&p[1]))
            .unwrap_or_else(|| exe.with_file_name("tracker-group.json"));
        let prefs = std::fs::read_to_string(config.with_file_name("desktop-startup.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .unwrap_or_default();
        let mut s = Self {
            hotkeys_enabled: prefs["global_hotkeys"] == true,
            reset_registered: false,
            stop_registered: false,
            minimize_pending: args.iter().any(|a| a == "--start-minimized"),
            registration_error: String::new(),
            hotkey_worker: None,
            commands,
        };
        s.register_hotkeys();
        s
    }
    fn unregister_hotkeys(&mut self) {
        if let Some(mut worker) = self.hotkey_worker.take() {
            worker.halt.store(true, Ordering::Relaxed);
            if let Some(thread) = worker.thread.take() {
                let _ = thread.join();
            }
        }
        self.reset_registered = false;
        self.stop_registered = false;
    }
    fn register_hotkeys(&mut self) {
        self.unregister_hotkeys();
        self.registration_error.clear();
        if !self.hotkeys_enabled {
            return;
        }
        #[cfg(windows)]
        {
            let halt = Arc::new(AtomicBool::new(false));
            let stopped = halt.clone();
            let (tx, events) = mpsc::channel();
            let (ready_tx, ready_rx) = mpsc::sync_channel(1);
            let commands = self.commands.clone();
            let thread = std::thread::spawn(move || unsafe {
                // A dedicated thread owns these thread hotkeys and their message queue.
                // This prevents the window event loop from consuming WM_HOTKEY first.
                let reset =
                    win::RegisterHotKey(std::ptr::null_mut(), win::RESET_ID, 0x4000, 0x75) != 0;
                let mut error = if reset {
                    String::new()
                } else {
                    format!(
                        "F6 registration failed: {}",
                        std::io::Error::last_os_error()
                    )
                };
                let stop =
                    win::RegisterHotKey(std::ptr::null_mut(), win::STOP_ID, 0x4000, 0x77) != 0;
                if !stop {
                    error.push_str(&format!(
                        " F8 registration failed: {}",
                        std::io::Error::last_os_error()
                    ));
                }
                let _ = ready_tx.send((reset, stop, error));
                let mut message: win::Message = std::mem::zeroed();
                while !stopped.load(Ordering::Relaxed) {
                    while win::PeekMessageW(&mut message, std::ptr::null_mut(), 0x0312, 0x0312, 1)
                        != 0
                    {
                        match message.wparam as i32 {
                            win::RESET_ID if reset => {
                                let _ = commands.send(json!({"command":"calibrate"}));
                                let _ = tx.send(Hotkey::ResetCenter);
                            }
                            win::STOP_ID if stop => {
                                let _ = commands.send(json!({"command":"stop"}));
                                let _ = tx.send(Hotkey::Stop);
                            }
                            _ => {}
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(15));
                }
                if reset {
                    win::UnregisterHotKey(std::ptr::null_mut(), win::RESET_ID);
                }
                if stop {
                    win::UnregisterHotKey(std::ptr::null_mut(), win::STOP_ID);
                }
            });
            match ready_rx.recv_timeout(std::time::Duration::from_secs(1)) {
                Ok((reset, stop, error)) => {
                    self.reset_registered = reset;
                    self.stop_registered = stop;
                    self.registration_error = error;
                }
                Err(e) => {
                    self.registration_error = format!("Hotkey worker did not register: {e}");
                }
            }
            self.hotkey_worker = Some(HotkeyWorker {
                halt,
                thread: Some(thread),
                events,
            });
        }
        #[cfg(not(windows))]
        {
            self.registration_error = "Global hotkeys require Windows.".into();
        }
    }
    pub fn poll(&mut self, ctx: &egui::Context) -> Vec<Hotkey> {
        if self.minimize_pending {
            self.minimize_pending = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        let mut events = vec![];
        if let Some(worker) = &self.hotkey_worker {
            events.extend(worker.events.try_iter());
        }
        events
    }
}
impl Drop for Startup {
    fn drop(&mut self) {
        self.unregister_hotkeys();
    }
}
#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    pub const RESET_ID: i32 = 0x5646;
    pub const STOP_ID: i32 = 0x5648;
    #[repr(C)]
    pub struct Message {
        pub hwnd: *mut c_void,
        pub message: u32,
        pub wparam: usize,
        pub lparam: isize,
        pub time: u32,
        pub point: [i32; 2],
        pub private: u32,
    }
    #[link(name = "user32")]
    extern "system" {
        pub fn RegisterHotKey(hwnd: *mut c_void, id: i32, modifiers: u32, key: u32) -> i32;
        pub fn UnregisterHotKey(hwnd: *mut c_void, id: i32) -> i32;
        pub fn PeekMessageW(
            message: *mut Message,
            hwnd: *mut c_void,
            min: u32,
            max: u32,
            remove: u32,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
    }
    #[cfg(all(test, target_pointer_width = "64"))]
    #[test]
    fn message_matches_win64_abi() {
        assert_eq!(std::mem::size_of::<Message>(), 48);
        assert_eq!(std::mem::offset_of!(Message, wparam), 16);
        assert_eq!(std::mem::offset_of!(Message, lparam), 24);
        assert_eq!(std::mem::offset_of!(Message, point), 36);
        assert_eq!(std::mem::offset_of!(Message, private), 44);
    }
}
