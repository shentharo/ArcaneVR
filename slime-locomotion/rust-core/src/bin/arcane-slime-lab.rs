//! Portable Rust workbench. Tracker UDP is LAN-facing; UI and Nexus mock stay local.
#[path = "../controller_status.rs"]
pub(crate) mod controller_status;
use arcane_bridge_core::{lab::Lab, parse_command, Command, Sessions};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream, UdpSocket},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
const MAX_HTTP_BODY: usize = 1024 * 1024 + 64 * 1024;
type Message = (Value, mpsc::Sender<Result<(), String>>);
fn wall() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
fn respond(stream: &mut TcpStream, status: &str, body: &[u8], mime: &str) {
    let h=format!("HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",body.len());
    let _ = stream.write_all(h.as_bytes());
    let _ = stream.write_all(body);
}
fn handle_http(mut s: TcpStream, port: u16, state: Arc<Mutex<String>>, tx: mpsc::Sender<Message>) {
    let _ = s.set_nonblocking(false); // Accepted sockets can inherit nonblocking mode on Windows.
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
    let mut data = vec![];
    let end = loop {
        let mut b = [0; 1024];
        let n = match s.read(&mut b) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        data.extend_from_slice(&b[..n]);
        if data.len() > 16384 {
            respond(&mut s, "413 Payload Too Large", b"{}", "application/json");
            return;
        }
        if let Some(i) = data.windows(4).position(|p| p == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let header = match std::str::from_utf8(&data[..end]) {
        Ok(v) => v,
        Err(_) => return,
    };
    let mut lines = header.lines();
    let mut first = lines.next().unwrap_or("").split_whitespace();
    let method = first.next().unwrap_or("").to_string();
    let path = first.next().unwrap_or("").to_string();
    let headers: Vec<_> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let get = |name: &str| {
        headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };
    let valid_hosts = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    if !get("host").is_some_and(|h| valid_hosts.iter().any(|v| v == h)) {
        respond(&mut s, "403 Forbidden", b"{}", "application/json");
        return;
    }
    if method == "GET" && path == "/" {
        respond(
            &mut s,
            "200 OK",
            include_bytes!("../../dashboard.html"),
            "text/html; charset=utf-8",
        );
        return;
    }
    if method == "GET" && path == "/api/state" {
        let body = state.lock().unwrap().clone();
        respond(&mut s, "200 OK", body.as_bytes(), "application/json");
        return;
    }
    if method != "POST" || path != "/api/command" {
        respond(&mut s, "404 Not Found", b"{}", "application/json");
        return;
    }
    if get("origin").is_some_and(|o| !valid_hosts.iter().any(|h| o == format!("http://{h}")))
        || get("content-type") != Some("application/json")
        || get("transfer-encoding").is_some()
    {
        respond(&mut s, "403 Forbidden", b"{}", "application/json");
        return;
    }
    let len = match get("content-length").and_then(|n| n.parse::<usize>().ok()) {
        Some(n) if n <= MAX_HTTP_BODY => n,
        _ => {
            respond(&mut s, "413 Payload Too Large", b"{}", "application/json");
            return;
        }
    };
    while data.len() < end + len {
        let mut b = [0; 1024];
        let n = match s.read(&mut b) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        data.extend_from_slice(&b[..n]);
    }
    let result = match serde_json::from_slice::<Value>(&data[end..end + len]) {
        Ok(v) => {
            let (reply, rx) = mpsc::channel();
            if tx.send((v, reply)).is_err() {
                Err("Receiver stopped".into())
            } else {
                rx.recv_timeout(Duration::from_secs(2))
                    .unwrap_or(Err("Receiver busy".into()))
            }
        }
        Err(_) => Err("Invalid JSON".into()),
    };
    let body = match &result {
        Ok(()) => json!({"ok":true}),
        Err(e) => json!({"error":e}),
    }
    .to_string();
    respond(
        &mut s,
        if result.is_ok() {
            "200 OK"
        } else {
            "400 Bad Request"
        },
        body.as_bytes(),
        "application/json",
    );
}
fn discovery(port: u16, t: f64, connected: bool) -> [u8; 138] {
    let mut p = [0; 138];
    p[..2].copy_from_slice(&[0x84, 0x11]);
    p[2..11].copy_from_slice(b"127.0.0.1");
    p[18..22].copy_from_slice(&1u32.to_le_bytes());
    let d = &mut p[22..];
    d[..8].copy_from_slice(&t.to_le_bytes());
    d[12..24].copy_from_slice(b"ARCANE-SIM01");
    d[25..28].fill(connected as u8);
    d[79..83].copy_from_slice(&(port as u32).to_le_bytes());
    p
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    run(std::env::args().collect(), Arc::new(AtomicBool::new(false)))
}

pub fn run(args: Vec<String>, shutdown: Arc<AtomicBool>) -> Result<(), Box<dyn std::error::Error>> {
    let arg = |key: &str, default: &str| {
        args.windows(2)
            .find(|a| a[0] == key)
            .map(|a| a[1].clone())
            .unwrap_or(default.into())
    };
    if args.iter().any(|a| a == "--help") {
        println!("VRCraft (Rust)\n--http-port 8811 --tracker-port 6969 --tracker-bind 0.0.0.0 --nexus-port 3500 --steam-port 29741 --config <file> [--seconds N]\nUI, Nexus mock, and experimental SteamVR output are loopback only.");
        return Ok(());
    }
    let http: u16 = arg("--http-port", "8811").parse()?;
    let tracker: u16 = arg("--tracker-port", "6969").parse()?;
    let nexus_port: u16 = arg("--nexus-port", "3500").parse()?;
    let steam_port: u16 = arg("--steam-port", "29741").parse()?;
    let default_config = std::env::current_exe()?
        .parent()
        .unwrap()
        .join("tracker-group.json");
    let mut lab = Lab::new(PathBuf::from(arg(
        "--config",
        &default_config.display().to_string(),
    )))?;
    lab.steam_port = steam_port;
    let tracker_udp = match UdpSocket::bind(format!(
        "{}:{tracker}",
        arg("--tracker-bind", "0.0.0.0")
    )) {
        Ok(u) => {
            u.set_nonblocking(true)?;
            Some(u)
        }
        Err(e) => {
            lab.error=format!("UDP {tracker} unavailable ({e}). Close the other SlimeVR receiver, then restart. Synthetic mode remains available.");
            None
        }
    };
    let steam = UdpSocket::bind(("127.0.0.1", 0))?;
    steam.set_nonblocking(true)?;
    let nexus = UdpSocket::bind(("127.0.0.1", nexus_port))?;
    nexus.set_nonblocking(true)?;
    let listener = TcpListener::bind(("127.0.0.1", http))?;
    listener.set_nonblocking(true)?;
    let state = Arc::new(Mutex::new(String::from("{}")));
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel::<Message>();
    let st = state.clone();
    let halt = stop.clone();
    let clients = Arc::new(AtomicUsize::new(0));
    thread::spawn(move || {
        while !halt.load(Ordering::Relaxed) {
            if let Ok((stream, _)) = listener.accept() {
                if clients.load(Ordering::Relaxed) >= 16 {
                    drop(stream);
                    continue;
                }
                clients.fetch_add(1, Ordering::Relaxed);
                let c = clients.clone();
                let state = st.clone();
                let tx = tx.clone();
                thread::spawn(move || {
                    handle_http(stream, http, state, tx);
                    c.fetch_sub(1, Ordering::Relaxed);
                });
            } else {
                thread::sleep(Duration::from_millis(5));
            }
        }
    });
    println!("VRCraft · Rust\nDashboard: http://127.0.0.1:{http}/\nRoles: LEFT ANKLE / RIGHT ANKLE / TORSO\nNexus mock: 127.0.0.1:{nexus_port}; discovery preview: 127.0.0.1:1181\nSteamVR output is experimental and connection is unverified. Close this console to exit.");
    let begin = Instant::now();
    let mut next = begin;
    let mut sessions = Sessions::default();
    let mut last_heartbeat = -10.;
    let mut controllers=controller_status::Monitor::default();
    let mut last_discovery = -10.;
    let mut buf = [0; 65535];
    let seconds = arg("--seconds", "0").parse::<f64>()?;
    loop {
        let now = Instant::now();
        let t = begin.elapsed().as_secs_f64();
        if shutdown.load(Ordering::Relaxed) || (seconds > 0. && t >= seconds) {
            break;
        }
        for _ in 0..64 {
            match steam.recv_from(&mut buf) {
                Ok((n, addr)) => lab.steam_ack(&buf[..n], addr, t),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    lab.steam_error = e.to_string();
                    break;
                }
            }
        }
        for (v, reply) in rx.try_iter().take(32) {
            let was_steam = lab.output_mode == "steamvr";
            let result = lab.command(&v, t);
            if was_steam && lab.output_mode != "steamvr" {
                let packet = lab.steam_packet(t);
                lab.steam_sent(steam.send_to(&packet, ("127.0.0.1", steam_port)));
            }
            let _ = reply.send(result);
        }
        if let Some(u) = &tracker_udp {
            for _ in 0..256 {
                match u.recv_from(&mut buf) {
                    Ok((n, a)) => match lab.ingest(&buf[..n], a, t) {
                        Ok(replies) => {
                            for r in replies {
                                if !lab.needs_identity_handshake(a.ip()) { let _ = u.send_to(&r, a); }
                            }
                        }
                        Err(_) => lab.rejected += 1,
                    },
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => {
                        lab.error = e.to_string();
                        break;
                    }
                }
            }
        }
        for _ in 0..32 {
            match nexus.recv_from(&mut buf) {
                Ok((n, a)) => {
                    if let Ok(cmd) = parse_command(&buf[..n]) {
                        match cmd {
                            Command::Connect { port, .. } => {
                                let (status, to) = sessions.connect(a.ip(), port, now);
                                let mut p = vec![
                                    0x54,
                                    11,
                                    21,
                                    if status == 0 || status == 3 { 0 } else { 1 },
                                    status,
                                ];
                                p.extend((nexus_port as u32).to_le_bytes());
                                p.extend(b"ARCANE-SIM01\0");
                                let _ = nexus.send_to(&p, to);
                            }
                            Command::Ping(port) => {
                                let (status, to) = sessions.connect(a.ip(), port, now);
                                let mut p = vec![0; 19];
                                p[..3].copy_from_slice(&[
                                    0x54,
                                    11,
                                    if status == 0 || status == 3 { 1 } else { 22 },
                                ]);
                                if status != 0 && status != 3 {
                                    p[3] = 2;
                                }
                                p[11..19].copy_from_slice(&wall().to_le_bytes());
                                let _ = nexus.send_to(&p, to);
                            }
                            Command::Disconnect(port) => sessions.disconnect(a.ip(), port),
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        if now >= next {
            lab.tick(t, wall());
            if lab.output_mode == "steamvr" {
                let packet = lab.steam_packet(t);
                lab.steam_sent(steam.send_to(&packet, ("127.0.0.1", steam_port)));
            } else {
                for to in sessions.destinations(now) {
                    let _ = nexus.send_to(&lab.packet, to);
                }
            }
            if t - last_discovery >= 1. {
                let _ = nexus.send_to(
                    &discovery(nexus_port, t, lab.engine.fresh(t)),
                    ("127.0.0.1", 1181),
                );
                last_discovery = t;
            }
            let mut snapshot = lab.snapshot(t);
            snapshot["controllers"]=controllers.snapshot(snapshot["steamvr"]["driver_connected"]==true);
            snapshot["tracker_port"] = json!(tracker);
            snapshot["nexus_port"] = json!(nexus_port);
            *state.lock().unwrap() = snapshot.to_string();
            next = now + Duration::from_millis(16);
        }
        if t - last_heartbeat > 1. {
            if let Some(u) = &tracker_udp {
                let mut p = vec![0; 12];
                p[..4].copy_from_slice(&0u32.to_be_bytes());
                for (a, seen) in &lab.peers {
                    if t - seen < 5. && !lab.needs_identity_handshake(a.ip()) {
                        let _ = u.send_to(&p, a);
                    }
                }
            }
            last_heartbeat = t;
        }
        thread::sleep(Duration::from_millis(2));
    }
    lab.engine.disarm();
    if lab.output_mode == "steamvr" {
        let packet = lab.steam_packet(begin.elapsed().as_secs_f64());
        lab.steam_sent(steam.send_to(&packet, ("127.0.0.1", steam_port)));
    }
    lab.tick(begin.elapsed().as_secs_f64(), wall());
    lab.flush_statistics();
    for to in sessions.destinations(Instant::now()) {
        let _ = nexus.send_to(&lab.packet, to);
    }
    stop.store(true, Ordering::Relaxed);
    Ok(())
}

#[cfg(test)]
mod http_tests {
    use super::*;
    fn exchange(body: &str, declared: usize, origin: &str) -> (String, Vec<Value>) {
        let listener=TcpListener::bind(("127.0.0.1",0)).unwrap();
        let port=listener.local_addr().unwrap().port();
        let (tx,rx)=mpsc::channel::<Message>();
        let server=thread::spawn(move || {let (socket,_)=listener.accept().unwrap();handle_http(socket,port,Arc::new(Mutex::new("{}".into())),tx);});
        let worker=thread::spawn(move || {let mut received=Vec::new();while let Ok((command,reply))=rx.recv(){received.push(command);reply.send(Ok(())).unwrap();}received});
        let mut client=TcpStream::connect(("127.0.0.1",port)).unwrap();
        client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let header=format!("POST /api/command HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: {origin}\r\nContent-Type: application/json\r\nContent-Length: {declared}\r\n\r\n");
        client.write_all(header.as_bytes()).unwrap();if !body.is_empty(){client.write_all(body.as_bytes()).unwrap();}
        let mut response=String::new();client.read_to_string(&mut response).unwrap();server.join().unwrap();(response,worker.join().unwrap())
    }
    #[test]
    fn large_command_is_dispatched_once() {
        let body=json!({"command":"profile_import","profile":{"padding":"x".repeat(30000)}}).to_string();
        // Omitted Origin remains valid for the local desktop client.
        let listener=TcpListener::bind(("127.0.0.1",0)).unwrap();let port=listener.local_addr().unwrap().port();let (tx,rx)=mpsc::channel::<Message>();
        let server=thread::spawn(move || {let (socket,_)=listener.accept().unwrap();handle_http(socket,port,Arc::new(Mutex::new("{}".into())),tx);});
        let worker=thread::spawn(move || {let mut received=Vec::new();while let Ok((v,reply))=rx.recv(){received.push(v);reply.send(Ok(())).unwrap();}received});
        let mut c=TcpStream::connect(("127.0.0.1",port)).unwrap();c.write_all(format!("POST /api/command HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",body.len()).as_bytes()).unwrap();c.write_all(body.as_bytes()).unwrap();let mut response=String::new();c.read_to_string(&mut response).unwrap();server.join().unwrap();let received=worker.join().unwrap();assert!(response.starts_with("HTTP/1.1 200"));assert_eq!(received.len(),1);assert_eq!(received[0]["command"],"profile_import");
    }
    #[test]
    fn oversized_and_cross_origin_do_not_reach_worker() {
        let (response,received)=exchange("",MAX_HTTP_BODY+1,"http://evil.example");assert!(response.starts_with("HTTP/1.1 403"));assert!(received.is_empty());
        // Non-browser local clients may omit Origin.
        let listener=TcpListener::bind(("127.0.0.1",0)).unwrap();let port=listener.local_addr().unwrap().port();let (tx,rx)=mpsc::channel::<Message>();
        let server=thread::spawn(move || {let (socket,_)=listener.accept().unwrap();handle_http(socket,port,Arc::new(Mutex::new("{}".into())),tx);});
        let mut c=TcpStream::connect(("127.0.0.1",port)).unwrap();c.write_all(format!("POST /api/command HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",MAX_HTTP_BODY+1).as_bytes()).unwrap();let mut response=String::new();c.read_to_string(&mut response).unwrap();server.join().unwrap();assert!(response.starts_with("HTTP/1.1 413"));assert!(rx.try_recv().is_err());
    }
}
