//! Protocol/session core. No KAT DLLs; unsafe calls are confined to Windows DPAPI storage.
//! Wire conventions remain provisional until verified against Quest Gateway.
#![deny(unsafe_code)]
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};
pub mod actions;
pub mod calibration;
pub mod continuous;
pub mod cadence;
pub mod gait;
pub mod gestures;
pub mod lab;
pub mod mounting;
pub mod profiles;
pub mod slime;
pub mod statistics;
pub mod usb_setup;
pub mod update_manifest;
pub mod wifi_profile;
pub mod wifi_scan;

#[derive(Debug, PartialEq)]
pub enum Command {
    Ping(u16),
    Connect { port: u16, force: bool },
    Disconnect(u16),
}

fn port_at(data: &[u8], offset: usize) -> Result<u16, &'static str> {
    let bytes: [u8; 4] = data
        .get(offset..offset + 4)
        .ok_or("short port")?
        .try_into()
        .unwrap();
    let value = i32::from_le_bytes(bytes);
    if !(1..=65535).contains(&value) {
        return Err("invalid port");
    }
    Ok(value as u16)
}

pub fn parse_command(data: &[u8]) -> Result<Command, &'static str> {
    if data.get(..2) != Some(&[0x54, 0x0b]) {
        return Err("bad header");
    }
    match data.get(2) {
        Some(0) => Ok(Command::Ping(port_at(data, 11)?)),
        Some(20) => {
            let port = port_at(data, 3)?;
            let force = match data.get(7) {
                Some(0) => false,
                Some(1) => true,
                _ => return Err("invalid force flag"),
            };
            Ok(Command::Connect { port, force })
        }
        Some(23) => Ok(Command::Disconnect(port_at(data, 3)?)),
        _ => Err("unsupported command"),
    }
}

/// Validate every advertised device before accessing its fields.
pub fn discovery_count(data: &[u8]) -> Result<usize, &'static str> {
    if data.get(..2) != Some(&[0x84, 0x11]) || data.len() < 22 {
        return Err("short discovery");
    }
    let count = u32::from_le_bytes(data[18..22].try_into().unwrap()) as usize;
    if count > 8 || data.len() != 22 + 116 * count {
        return Err("invalid device count/length");
    }
    Ok(count)
}

#[derive(Default)]
pub struct Sessions {
    owner: Option<IpAddr>,
    ports: Vec<(u16, Instant)>,
}
impl Sessions {
    pub fn expire(&mut self, now: Instant) {
        self.ports
            .retain(|(_, last)| now.saturating_duration_since(*last) < Duration::from_secs(5));
        if self.ports.is_empty() {
            self.owner = None;
        }
    }
    /// Returns status and reply destination. A rejection always goes to requester.
    /// Force takeover is intentionally unsupported until explicit pairing exists.
    pub fn connect(&mut self, source: IpAddr, port: u16, now: Instant) -> (u8, SocketAddr) {
        let reply = SocketAddr::new(source, port);
        if port == 0 {
            return (2, reply);
        }
        self.expire(now);
        if self.owner.is_some() && self.owner != Some(source) {
            return (11, reply);
        }
        if let Some((_, last)) = self.ports.iter_mut().find(|(p, _)| *p == port) {
            *last = now;
            return (3, reply);
        }
        if self.ports.len() == 8 {
            return (2, reply);
        }
        self.owner = Some(source);
        self.ports.push((port, now));
        (0, reply)
    }
    pub fn disconnect(&mut self, source: IpAddr, port: u16) {
        if self.owner != Some(source) {
            return;
        }
        self.ports.retain(|(p, _)| *p != port);
        if self.ports.is_empty() {
            self.owner = None;
        }
    }
    pub fn destinations(&mut self, now: Instant) -> Vec<SocketAddr> {
        self.expire(now);
        match self.owner {
            None => vec![],
            Some(ip) => self
                .ports
                .iter()
                .filter(|(_, last)| now.saturating_duration_since(*last) < Duration::from_secs(3))
                .map(|(p, _)| SocketAddr::new(ip, *p))
                .collect(),
        }
    }
}

pub fn encode_motion(
    speed: f32,
    yaw: f32,
    connected: bool,
    armed: bool,
    time: f64,
    seq: u64,
) -> Result<[u8; 261], &'static str> {
    if !speed.is_finite() || !(0.0..=1.0).contains(&speed) || !yaw.is_finite() || !time.is_finite()
    {
        return Err("invalid motion");
    }
    let mut packet = [0u8; 261];
    packet[..3].copy_from_slice(&[0x54, 0x0b, 99]);
    packet[3..11].copy_from_slice(&seq.to_le_bytes());
    let name = b"Arcane Slime locomotion";
    packet[11..11 + name.len()].copy_from_slice(name);
    packet[75] = connected as u8;
    packet[76..84].copy_from_slice(&time.to_le_bytes());
    let travel = if connected && armed { speed } else { 0.0 };
    let values = [
        0.0,
        (yaw / 2.0).sin(),
        0.0,
        (yaw / 2.0).cos(),
        0.0,
        0.0,
        -travel,
    ];
    for (i, value) in values.iter().enumerate() {
        packet[84 + i * 4..88 + i * 4].copy_from_slice(&value.to_le_bytes());
    }
    Ok(packet)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_truncation_and_ports() {
        for n in 0..8 {
            assert!(parse_command(&[0x54, 11, 20, 1, 0, 0, 0, 0][..n]).is_err());
        }
        for port in [-1i32, 0, 65536, i32::MAX] {
            let mut p = vec![0x54, 11, 20];
            p.extend(port.to_le_bytes());
            p.push(0);
            assert!(parse_command(&p).is_err());
        }
    }
    #[test]
    fn bounds_discovery() {
        assert!(discovery_count(&[0x84, 0x11]).is_err());
        let mut p = vec![0u8; 22];
        p[..2].copy_from_slice(&[0x84, 0x11]);
        assert_eq!(discovery_count(&p), Ok(0));
        p[18] = 1;
        assert!(discovery_count(&p).is_err());
        p.resize(138, 0);
        assert_eq!(discovery_count(&p), Ok(1));
        p[18] = 255;
        assert!(discovery_count(&p).is_err());
    }
    #[test]
    fn rejection_goes_to_requester() {
        let now = Instant::now();
        let a = "127.0.0.1".parse().unwrap();
        let b = "127.0.0.2".parse().unwrap();
        let mut sessions = Sessions::default();
        assert_eq!(sessions.connect(a, 4000, now).0, 0);
        assert_eq!(
            sessions.connect(b, 5000, now),
            (11, SocketAddr::new(b, 5000))
        );
    }
    #[test]
    fn timeout_independent_of_sensor_updates() {
        let now = Instant::now();
        let ip = "127.0.0.1".parse().unwrap();
        let mut s = Sessions::default();
        s.connect(ip, 4000, now);
        assert_eq!(s.destinations(now).len(), 1);
        assert!(s.destinations(now + Duration::from_secs(3)).is_empty());
        s.expire(now + Duration::from_secs(5));
        assert!(s.owner.is_none());
    }
    #[test]
    fn caps_clients_and_refreshes_reconnect() {
        let now = Instant::now();
        let ip = "127.0.0.1".parse().unwrap();
        let mut s = Sessions::default();
        for port in 4000..4008 {
            assert_eq!(s.connect(ip, port, now).0, 0);
        }
        assert_eq!(s.connect(ip, 5000, now).0, 2);
        assert_eq!(s.connect(ip, 4000, now + Duration::from_secs(4)).0, 3);
        assert_eq!(s.destinations(now + Duration::from_secs(6)).len(), 1);
    }
    #[test]
    fn packet_neutral_and_nonfinite() {
        let p = encode_motion(0.8, 0.0, true, false, 123., 42).unwrap();
        assert_eq!(&p[..3], &[0x54, 11, 99]);
        assert_eq!(p.len(), 261);
        assert_eq!(f32::from_le_bytes(p[108..112].try_into().unwrap()), 0.0);
        assert!(encode_motion(f32::NAN, 0.0, true, true, 0., 0).is_err());
    }
    #[test]
    fn malformed_corpus_never_panics() {
        let mut seed = 17u32;
        for len in 0..1024 {
            let data: Vec<u8> = (0..len)
                .map(|_| {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    (seed >> 24) as u8
                })
                .collect();
            let _ = parse_command(&data);
            let _ = discovery_count(&data);
        }
    }
}

pub mod profile_builder;


