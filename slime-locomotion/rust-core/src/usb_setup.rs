//! SlimeVR USB provisioning. Never retain raw serial output or log credentials.
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{
    io::{Read, Write},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub struct SetupState {
    pub ports: Vec<(String, String)>,
    pub connected: String,
    pub identified: bool,
    pub busy: bool,
    pub accepted: bool,
    pub address: Option<std::net::Ipv4Addr>,
    pub message: String,
    pub networks: Vec<String>,
    pub device_id: Option<String>,
    pub auto_enabled: bool,
    pub auto_status: String,
    pub completed: Vec<(String, std::net::Ipv4Addr)>,
}
fn remember_connection(state:&mut SetupState) {
    if let Some(ip)=state.address {
        let id=state.device_id.clone().unwrap_or_else(||state.connected.clone());
        state.completed.retain(|(old,_)|old!=&id);
        if state.completed.len()>=32 {state.completed.remove(0);}
        state.completed.push((id,ip));
    }
}
pub enum Action {
    Refresh,
    ReloadProfile,
    Connect(String),
    Disconnect,
    Scan,
    Wifi { ssid: String, password: String },
}

pub fn wifi_command(ssid: &str, password: &str) -> Result<String, String> {
    if ssid.is_empty() || ssid.len() > 32 {
        return Err("Network name must contain 1â€“32 UTF-8 bytes.".into());
    }
    if password.len() > 64 {
        return Err("Wi-Fi password is longer than 64 bytes.".into());
    }
    if ssid.chars().chain(password.chars()).any(char::is_control) {
        return Err("Wi-Fi details cannot contain control characters.".into());
    }
    if ssid.contains('"')
        || password.contains('"')
        || ssid.contains('\\')
        || password.contains('\\')
    {
        // Modern firmware's base64 form avoids ambiguous quoted command parsing.
        return Ok(format!(
            "SET BWIFI {} {}\n",
            STANDARD.encode(ssid),
            STANDARD.encode(password)
        ));
    }
    Ok(format!("SET WIFI \"{ssid}\" \"{password}\"\n"))
}

fn consume_line(state: &mut SetupState, line: &str) {
    if line.contains("SlimeVR Tracker,") && !state.identified {
        state.identified = true;
        state.message = "SlimeVR tracker identified. Enter its 2.4 GHz Wi-Fi details.".into();
    }
    if let Some(part) = line.split("address: ").nth(1) {
        state.address = part
            .split(',')
            .next()
            .unwrap_or_default()
            .trim()
            .parse::<std::net::Ipv4Addr>().ok().filter(|ip| !ip.is_unspecified());
    }
    if let Some(part) = line
        .split("MAC address:")
        .nth(1)
        .or_else(|| line.split("MAC:").nth(1))
        .or_else(|| line.split("mac:").nth(1))
    {
        let value = part
            .trim()
            .split(|c: char| c == ',' || c.is_whitespace())
            .next()
            .unwrap_or("");
        if value.len() == 17
            && value.chars().enumerate().all(|(i, c)| {
                if i % 3 == 2 {
                    c == ':'
                } else {
                    c.is_ascii_hexdigit()
                }
            })
        {
            state.device_id = Some(value.to_ascii_uppercase());
        }
    }
    if line.contains("CMD SET WIFI OK") || line.contains("CMD SET BWIFI OK") {
        // Require a fresh address after the acknowledgement, not the old connection.
        if !state.accepted {
            state.address = None;
        }
        state.accepted = true;
        state.busy = false;
        state.message =
            "Tracker accepted the Wi-Fi details. Waiting for live sensor packets on this PCâ€¦"
                .into();
    } else if line.contains("CMD SET WIFI ERROR")
        || line.contains("CMD SET BWIFI ERROR")
        || line.contains("CMD SET ERROR")
    {
        state.accepted = false;
        state.busy = false;
        state.message =
            "Tracker rejected the Wi-Fi command. Check its firmware and Wi-Fi details.".into();
    }
    if line.contains("[WSCAN]") {
        if let (Some(start), Some(end)) = (line.find('\''), line.rfind('\'')) {
            if end > start {
                let ssid = &line[start + 1..end];
                if !state.networks.iter().any(|s| s == ssid) && state.networks.len() < 100 {
                    state.networks.push(ssid.into());
                }
            }
        }
        if line.contains("Scan failed") {
            state.message = "Wi-Fi scan failed. Enter the 2.4 GHz network name manually.".into();
        }
    }
    // No raw console output is retained: firmware may echo credentials.
}

#[cfg(windows)]
mod attachment_lock {
    pub(super) struct AttachmentLock(*mut std::ffi::c_void);
    #[cfg(windows)]
    #[allow(unsafe_code)]
    impl AttachmentLock {
        pub(super) fn acquire(name: &str) -> Option<Self> {
            use std::os::windows::ffi::OsStrExt;
            #[link(name = "kernel32")]
            extern "system" {
                fn CreateMutexW(
                    a: *mut std::ffi::c_void,
                    owner: i32,
                    name: *const u16,
                ) -> *mut std::ffi::c_void;
                fn WaitForSingleObject(h: *mut std::ffi::c_void, ms: u32) -> u32;
                fn CloseHandle(h: *mut std::ffi::c_void) -> i32;
            }
            let key: Vec<u16> = std::ffi::OsStr::new(&format!("Local\\ArcaneVRUsbAuto-{name}"))
                .encode_wide()
                .chain(Some(0))
                .collect();
            unsafe {
                let h = CreateMutexW(std::ptr::null_mut(), 0, key.as_ptr());
                if h.is_null() {
                    return None;
                }
                let result = WaitForSingleObject(h, 0);
                if result == 0 || result == 0x80 {
                    Some(Self(h))
                } else {
                    CloseHandle(h);
                    None
                }
            }
        }
    }
    #[cfg(windows)]
    #[allow(unsafe_code)]
    impl Drop for AttachmentLock {
        fn drop(&mut self) {
            #[link(name = "kernel32")]
            extern "system" {
                fn ReleaseMutex(h: *mut std::ffi::c_void) -> i32;
                fn CloseHandle(h: *mut std::ffi::c_void) -> i32;
            }
            unsafe {
                ReleaseMutex(self.0);
                CloseHandle(self.0);
            }
        }
    }
}
#[cfg(windows)]
use attachment_lock::AttachmentLock;
#[cfg(not(windows))]
struct AttachmentLock;
#[cfg(not(windows))]
impl AttachmentLock {
    fn acquire(_: &str) -> Option<Self> {
        Some(Self)
    }
}

fn trusted_port(p: &serialport::SerialPortInfo) -> bool {
    matches!(&p.port_type, serialport::SerialPortType::UsbPort(u) if [0x1a86,0x10c4,0x303a,0x0403].contains(&u.vid))
}
fn fingerprint(p: &serialport::SerialPortInfo) -> String {
    match &p.port_type {
        serialport::SerialPortType::UsbPort(u) => format!(
            "{}-{:04x}-{:04x}-{}",
            p.port_name,
            u.vid,
            u.pid,
            u.serial_number.as_deref().unwrap_or("")
        ),
        _ => p.port_name.clone(),
    }
}
#[derive(Default)]
struct AutoPlanner {
    attempted: std::collections::HashSet<String>,
    retry: std::collections::HashMap<String, Instant>,
}
impl AutoPlanner {
    fn network_changed(&mut self) {self.attempted.clear();self.retry.clear();}
    fn retain(&mut self, present: &std::collections::HashSet<String>) {
        self.attempted.retain(|k| present.contains(k));
        self.retry.retain(|k, _| present.contains(k));
    }
    fn candidate(&self, key: &str, now: Instant) -> bool {
        !self.attempted.contains(key) && self.retry.get(key).is_none_or(|t| now >= *t)
    }
    fn backoff(&mut self, key: &str, now: Instant) {
        self.retry.insert(key.into(), now + Duration::from_secs(15));
    }
    fn written(&mut self, key: &str) {
        self.attempted.insert(key.into());
    }
}
fn may_auto_write(identified: bool, sent: bool, enabled: bool) -> bool {
    identified && !sent && enabled
}

pub struct UsbSetup {
    pub tx: mpsc::Sender<Action>,
    updates: mpsc::Receiver<SetupState>,
    pub state: SetupState,
}
impl Default for UsbSetup {
    fn default() -> Self {
        Self::new()
    }
}
impl UsbSetup {
    /// Inert setup for screenshot/QA runs: no worker, profile access, or USB enumeration.
    pub fn disabled() -> Self {
        let (tx, _rx) = mpsc::channel();
        let (_updates_tx, updates) = mpsc::sync_channel(1);
        Self { tx, updates, state: SetupState::default() }
    }
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<Action>();
        let (updates_tx, updates) = mpsc::sync_channel(4);
        thread::spawn(move || {
            let mut state = SetupState::default();
            let mut port: Option<Box<dyn serialport::SerialPort>> = None;
            let mut pending = String::new();
            let mut deadline = None;
            let mut last_info = Instant::now();
            let mut last_enumerate = Instant::now() - Duration::from_secs(2);
            let mut profile: Option<crate::wifi_profile::Profile> = None;
            let mut planner = AutoPlanner::default();
            let mut locks: std::collections::HashMap<String, AttachmentLock> =
                std::collections::HashMap::new();
            let mut auto_key: Option<String> = None;
            let mut auto_sent = false;
            let mut sent_profile_hash:Option<u64> = None;
            let mut identified_at: Option<Instant> = None;
            let mut successful: std::collections::HashMap<String, u64> =
                std::collections::HashMap::new();
            loop {
                let action = match rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(a) => Some(a),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(_) => break,
                };
                let mut publish = action.is_some();
                if let Some(action) = action {
                    match action {
                        Action::ReloadProfile => {
                            last_enumerate = Instant::now() - Duration::from_secs(2);
                        }
                        Action::Refresh => {
                            match serialport::available_ports() {
                                Ok(ports) => {
                                    state.ports = ports
                                        .into_iter()
                                        .map(|p| {
                                            let label = match p.port_type {
                                                serialport::SerialPortType::UsbPort(u) => u
                                                    .product
                                                    .or(u.manufacturer)
                                                    .unwrap_or_else(|| "USB serial device".into()),
                                                _ => "Serial device".into(),
                                            };
                                            (p.port_name, label)
                                        })
                                        .collect();
                                    if state.ports.is_empty() {
                                        state.message="No USB serial ports detected. Power on one tracker, use a data USB cable, then click Refresh USB ports.".into();
                                    }
                                }
                                Err(_) => state.message =
                                    "Unable to enumerate USB ports. Check the tracker USB driver."
                                        .into(),
                            }
                        }
                        Action::Connect(name) => {
                            if let Some(key) = auto_key.take() {
                                if !auto_sent {
                                    locks.remove(&key);
                                }
                            }
                            auto_sent = false;
                            port = None;
                            pending.clear();
                            deadline = None;
                            state.connected.clear();
                            state.identified = false;
                            state.accepted = false;
                            state.address = None;
                            state.device_id = None;
                            state.networks.clear();
                            state.busy = false;
                            match serialport::new(&name,115200).timeout(Duration::from_millis(30)).open() {
                                Ok(mut p)=>{
                                    let _=p.write_request_to_send(false);let _=p.write_data_terminal_ready(false);
                                    match p.write_all(b"\nGET INFO\n") {Ok(())=>{state.connected=name;state.message="USB connected. Identifying SlimeVR firmwareâ€¦".into();port=Some(p);last_info=Instant::now();deadline=Some(Instant::now()+Duration::from_secs(10));},Err(_)=>state.message="Could not communicate with the USB tracker.".into()}
                                },
                                Err(_)=>state.message="Could not open USB port. Close SlimeVR Server or another serial console, then reconnect.".into(),
                            }
                        }
                        Action::Disconnect => {
                            if let Some(key) = auto_key.take() {
                                if !auto_sent {
                                    locks.remove(&key);
                                }
                            }
                            auto_sent = false;
                            port = None;
                            pending.clear();
                            deadline = None;
                            state.connected.clear();
                            state.identified = false;
                            state.accepted = false;
                            state.address = None;
                            state.device_id = None;
                            state.busy = false;
                            state.message =
                                "USB disconnected. Trackers already on Wi-Fi keep streaming."
                                    .into();
                        }
                        Action::Scan => {
                            if let Some(p) = port.as_mut().filter(|_| state.identified) {
                                state.networks.clear();
                                if p.write_all(b"GET WIFISCAN\n").is_ok() {
                                    state.message="Scanning from the tracker. You can also enter the network name manually.".into();
                                }
                            }
                        }
                        Action::Wifi { ssid, password } => {
                            if !state.identified {
                                state.message =
                                    "Connect and identify a SlimeVR tracker first.".into();
                            } else {
                                match wifi_command(&ssid, &password) {
                                    Err(e) => state.message = e,
                                    Ok(mut command) => {
                                        if port.as_mut().is_some_and(|p| {
                                            p.write_all(command.as_bytes()).is_ok()
                                        }) {
                                            state.accepted = false;
                                            state.address = None;
                                            state.busy = true;
                                            state.message="Sending Wi-Fi details to the selected USB trackerâ€¦".into();
                                            deadline =
                                                Some(Instant::now() + Duration::from_secs(12));
                                        } else {
                                            state.message="USB write failed. Reconnect the tracker and retry.".into();
                                        }
                                        command.clear();
                                    }
                                }
                            }
                        }
                    }
                }
                if last_enumerate.elapsed() >= Duration::from_secs(2) {
                    last_enumerate = Instant::now();
                    let loaded=crate::wifi_profile::load().ok().flatten();
                    if loaded!=profile {planner.network_changed();}
                    profile = loaded;
                    let enabled = profile.as_ref().is_some_and(|p| p.auto_setup);
                    if state.auto_enabled != enabled {
                        state.auto_enabled = enabled;
                        publish = true;
                    }
                    if let Ok(ports) = serialport::available_ports() {
                        let listed:Vec<_>=ports.iter().map(|p|(p.port_name.clone(),match &p.port_type{serialport::SerialPortType::UsbPort(u)=>u.product.clone().or_else(||u.manufacturer.clone()).unwrap_or_else(||"USB serial device".into()),_=>"Serial device".into()})).collect();
                        if listed!=state.ports {state.ports=listed;publish=true;}
                        let candidates: Vec<_> = ports.iter().filter(|p| trusted_port(p)).collect();
                        let present: std::collections::HashSet<_> =
                            candidates.iter().map(|p| fingerprint(p)).collect();
                        planner.retain(&present);
                        locks.retain(|k, _| present.contains(k));
                        if let Some(key) = auto_key.as_ref() {
                            if !present.contains(key) {
                                port = None;
                                auto_key = None;
                                auto_sent = false;
                                state.connected.clear();
                                state.identified = false;
                                state.accepted = false;
                                state.address = None;
                                state.device_id = None;
                                deadline = None;
                                publish = true;
                            }
                        }
                        if port.is_none() && state.auto_enabled {
                            if let Some(info) = candidates
                                .iter()
                                .find(|p| planner.candidate(&fingerprint(p), Instant::now()))
                            {
                                let key = fingerprint(info);
                                if let Some(lock) = AttachmentLock::acquire(&info.port_name) {
                                    match serialport::new(&info.port_name, 115200)
                                        .timeout(Duration::from_millis(30))
                                        .open()
                                    {
                                        Ok(mut p) => {
                                            let _ = p.write_request_to_send(false);
                                            let _ = p.write_data_terminal_ready(false);
                                            if p.write_all(b"\nGET INFO\n").is_ok() {
                                                state.connected = info.port_name.clone();
                                                state.identified = false;
                                                state.accepted = false;
                                                state.address = None;
                                                state.device_id = None;
                                                state.busy = false;
                                                pending.clear();
                                                auto_key = Some(key.clone());
                                                auto_sent = false;
                                                identified_at = None;
                                                locks.insert(key, lock);
                                                port = Some(p);
                                                last_info = Instant::now();
                                                deadline =
                                                    Some(Instant::now() + Duration::from_secs(10));
                                                state.auto_status =
                                                    "Identifying USB tracker…".into();
                                                publish = true;
                                            } else {
                                                planner.backoff(&key, Instant::now());
                                            }
                                        }
                                        Err(_) => planner.backoff(&key, Instant::now()),
                                    }
                                } else {
                                    planner.backoff(&key, Instant::now());
                                }
                            }
                        }
                    }
                }
                if !state.auto_enabled && auto_key.is_some() && !auto_sent {
                    if let Some(k) = auto_key.take() {
                        locks.remove(&k);
                    }
                    port = None;
                    deadline = None;
                    state.connected.clear();
                    state.identified = false;
                    state.address = None;
                    state.device_id = None;
                    publish = true;
                }
                if let Some(p) = port.as_mut() {
                    let mut bytes = [0; 2048];
                    match p.read(&mut bytes) {
                        Ok(n) if n > 0 => {
                            pending.push_str(&String::from_utf8_lossy(&bytes[..n]));
                            while let Some(end) = pending.find('\n') {
                                let line = pending[..end].to_string();
                                pending.drain(..=end);
                                consume_line(&mut state, &line);
                                publish = true;
                            }
                            if pending.len() > 8192 {
                                pending.clear();
                            }
                        }
                        Err(e)
                            if !matches!(
                                e.kind(),
                                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                            ) =>
                        {
                            port = None;
                            if let Some(key) = auto_key.take() {
                                if !auto_sent {
                                    planner.backoff(&key, Instant::now());
                                    locks.remove(&key);
                                }
                            }
                            auto_sent = false;
                            state.connected.clear();
                            state.identified = false;
                            state.accepted = false;
                            state.address = None;
                            state.device_id = None;
                            state.busy = false;
                            deadline = None;
                            state.message="USB tracker disconnected. Plug in the next tracker or reconnect this one.".into();
                            publish = true;
                        }
                        _ => {}
                    }
                }
                if let Some(key) = auto_key.clone() {
                    if state.identified && identified_at.is_none() {
                        identified_at = Some(Instant::now());
                    }
                    let info_settled =
                        identified_at.is_some_and(|at| at.elapsed() >= Duration::from_secs(1));
                    let profile_hash = profile.as_ref().map(|p| {
                        use std::hash::{Hash, Hasher};
                        let mut h = std::collections::hash_map::DefaultHasher::new();
                        p.ssid.hash(&mut h);
                        p.password.hash(&mut h);
                        h.finish()
                    });
                    // Only skip a known successful attachment when it has a stable tracker MAC.
                    if info_settled
                        && state.identified
                        && !auto_sent
                        && state.address.is_some()
                        && state
                            .device_id
                            .as_ref()
                            .is_some_and(|id| successful.get(id).copied() == profile_hash)
                    {
                        remember_connection(&mut state);
                        planner.written(&key);
                        port = None;
                        auto_key = None;
                        deadline = None;
                        state.auto_status = "Tracker already connected to the saved Wi-Fi.".into();
                        state.connected.clear();
                        state.identified = false;
                        state.accepted = false;
                        state.address = None;
                        state.device_id = None;
                        publish = true;
                    }
                    if info_settled
                        && may_auto_write(state.identified, auto_sent, state.auto_enabled)
                    {
                        if let Some(ref saved) = profile {
                            if let Ok(mut command) = wifi_command(&saved.ssid, &saved.password) {
                                // Mark before the write: a partial write must never cause repeated credentials.
                                planner.written(&key);
                                auto_sent = true;
                                sent_profile_hash=profile_hash;
                                state.address = None;
                                state.accepted = false;
                                if port
                                    .as_mut()
                                    .is_some_and(|p| p.write_all(command.as_bytes()).is_ok())
                                {
                                    state.busy = true;
                                    deadline = Some(Instant::now() + Duration::from_secs(45));
                                    state.auto_status =
                                        "Waiting for tracker acknowledgement and Wi-Fi address…"
                                            .into();
                                } else {
                                    state.busy = false;
                                    deadline = Some(Instant::now());
                                    state.auto_status =
                                        "USB write failed; reconnect tracker to retry.".into();
                                }
                                command.clear();
                                publish = true;
                            } else {
                                planner.backoff(&key, Instant::now());
                                auto_key = None;
                                locks.remove(&key);
                                port = None;
                                deadline = None;
                                state.auto_status = "Saved Wi-Fi profile is invalid.".into();
                                publish = true;
                            }
                        }
                    }
                    if auto_sent && state.accepted && state.address.is_some() {
                        remember_connection(&mut state);
                        if let (Some(id), Some(hash)) = (state.device_id.clone(), sent_profile_hash) {
                            successful.insert(id, hash);
                        }
                        state.auto_status =
                            "Tracker connected to Wi-Fi. Plug in the next tracker.".into();
                        port = None;
                        auto_key = None;
                        deadline = None;
                        state.connected.clear();
                        state.identified = false;
                        state.busy = false;
                        state.accepted = false;
                        state.address = None;
                        state.device_id = None;
                        publish = true;
                    }
                }
                if let Some(p) = port.as_mut() {
                    if last_info.elapsed() > Duration::from_secs(2) {
                        let _ = p.write_all(b"GET INFO\n");
                        last_info = Instant::now();
                    }
                }
                if deadline.is_some_and(|d| Instant::now() > d) {
                    deadline = None;
                    if let Some(key) = auto_key.take() {
                        if !auto_sent {
                            planner.backoff(&key, Instant::now());
                            locks.remove(&key);
                        }
                        port = None;
                        state.connected.clear();
                        state.identified = false;
                        state.accepted = false;
                        state.address = None;
                        state.device_id = None;
                        state.busy = false;
                        state.auto_status =
                            "Tracker setup timed out; reconnect to retry if needed.".into();
                        auto_sent = false;
                        publish = true;
                    }
                    if state.busy {
                        state.busy = false;
                        state.message="No Wi-Fi command acknowledgement received. Check that the tracker is powered on and has SlimeVR firmware.".into();
                        publish = true;
                    } else if !state.identified {
                        state.message="USB port opened, but SlimeVR firmware did not respond. Check the selected port and power switch.".into();
                        publish = true;
                    }
                }
                if publish {
                    let _ = updates_tx.try_send(state.clone());
                }
            }
        });
        let setup = Self {
            tx,
            updates,
            state: SetupState::default(),
        };
        let _ = setup.tx.send(Action::Refresh);
        setup
    }
    pub fn poll(&mut self) {
        for state in self.updates.try_iter() {
            self.state = state;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_credentials_and_blocks_command_injection() {
        assert!(wifi_command("", "password").is_err());
        assert!(wifi_command(&"Ã©".repeat(17), "password").is_err());
        assert!(wifi_command("network", "a\nFRST\n").is_err());
        assert!(wifi_command("network", &"x".repeat(65)).is_err());
        assert_eq!(
            wifi_command("My network", "a b c").unwrap(),
            "SET WIFI \"My network\" \"a b c\"\n"
        );
        let cmd = wifi_command("My \"network\"", "slash\\password").unwrap();
        assert!(cmd.starts_with("SET BWIFI "));
        assert_eq!(cmd.lines().count(), 1);
    }
    #[test]
    fn separates_firmware_acknowledgement_from_network_connection() {
        let mut state = SetupState::default();
        consume_line(
            &mut state,
            "[INFO] CMD SET WIFI OK: New wifi credentials set, reconnecting",
        );
        assert!(state.accepted);
        assert!(state.address.is_none());
        consume_line(
            &mut state,
            "SlimeVR Tracker, board: 9, address: 192.168.1.12, status: 0",
        );
        assert!(state.identified);
        assert_eq!(state.address.unwrap().to_string(), "192.168.1.12");
        consume_line(&mut state, "[ERROR] CMD SET WIFI ERROR: Too few arguments");
        assert!(!state.accepted);
    }
    #[test]
    fn discovery_planner_uses_all_trusted_ports_and_resets_on_unplug() {
        use serialport::{SerialPortInfo, SerialPortType, UsbPortInfo};
        let port = |name: &str, vid| SerialPortInfo {
            port_name: name.into(),
            port_type: SerialPortType::UsbPort(UsbPortInfo {
                vid,
                pid: 1,
                serial_number: None,
                manufacturer: None,
                product: None,
            }),
        };
        let a = port("COM8", 0x1a86);
        let b = port("COM17", 0x303a);
        let other = port("COM3", 0xffff);
        assert!(trusted_port(&a) && trusted_port(&b));
        assert!(!trusted_port(&other));
        let mut plan = AutoPlanner::default();
        let now = Instant::now();
        let key = fingerprint(&a);
        assert!(plan.candidate(&key, now));
        plan.backoff(&key, now);
        assert!(!plan.candidate(&key, now));
        assert!(plan.candidate(&fingerprint(&b), now));
        assert!(plan.candidate(&key, now + Duration::from_secs(16)));
        plan.written(&key);
        assert!(!plan.candidate(&key, now + Duration::from_secs(60)));
        plan.network_changed();
        assert!(plan.candidate(&key, now));
        plan.written(&key);
        plan.retain(&std::collections::HashSet::new());
        assert!(plan.candidate(&key, now));
    }
    #[test]
    fn real_tracker_info_mac_and_ip_unset_are_not_stale() {
        let mut state=SetupState::default();
        consume_line(&mut state,"[INFO ] [SerialCommands] SlimeVR Tracker, board: 22, address: 10.0.0.191, mac: 02:00:00:82:6C:63, status: 0, wifi state: 5");
        assert_eq!(state.device_id.as_deref(),Some("02:00:00:82:6C:63"));
        remember_connection(&mut state);
        assert_eq!(state.completed[0].1.to_string(),"10.0.0.191");
        consume_line(&mut state,"SlimeVR Tracker, address: (IP unset), mac: 02:00:00:82:6C:63, status: 0");
        assert!(state.address.is_none());
        assert_eq!(state.completed.len(),1);
    }
    #[test]
    fn auto_write_requires_identification_and_enabled_profile_and_is_once() {
        assert!(!may_auto_write(false, false, true));
        assert!(!may_auto_write(true, false, false));
        assert!(!may_auto_write(true, true, true));
        assert!(may_auto_write(true, false, true));
    }
    #[test]
    fn info_clears_unset_address_and_parses_mac() {
        let mut s = SetupState::default();
        consume_line(&mut s, "SlimeVR Tracker, address: 192.168.1.9, status: 0");
        consume_line(&mut s, "MAC address: aa:bb:cc:dd:ee:ff");
        assert_eq!(s.device_id.as_deref(), Some("AA:BB:CC:DD:EE:FF"));
        consume_line(&mut s, "SlimeVR Tracker, address: 0.0.0.0, status: 0");
        assert!(s.address.is_none());
    }
}
