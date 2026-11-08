//! Process-owned SteamVR controls.
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
fn is_dismissal(line: &str) -> bool { serde_json::from_str::<Value>(line).ok().is_some_and(|v| v["type"] == "dismissed") }

#[derive(Default)]
struct Session {
    child: Option<Child>,
    stdin: Option<mpsc::SyncSender<String>>,
    stopping: Option<mpsc::Receiver<()>>,
    events: Option<mpsc::Receiver<String>>,
    log: Vec<String>,
    last_message: Option<Instant>,
    exit: String,
    dismissed: bool,
}
impl Session {
    fn start(&mut self, path: &PathBuf, args: &[String]) -> Result<(), String> {
        self.poll();
        if self.child.is_some() || self.stopping.is_some() {
            return Err("This helper is already running.".into());
        }
        let mut command = Command::new(path);
        command
            .args(args)
            .args(["--parent-pid", &std::process::id().to_string()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("Cannot launch {}: {e}", path.display()))?;
        let (tx, rx) = mpsc::channel();
        if let Some(output) = child.stdout.take() {
            reader(output, tx.clone(), "");
        }
        if let Some(output) = child.stderr.take() {
            reader(output, tx, "stderr: ");
        }
        if let Some(mut input) = child.stdin.take() {
            let (tx, rx) = mpsc::sync_channel::<String>(8);
            std::thread::spawn(move || {
                while let Ok(text) = rx.recv() {
                    if input.write_all(text.as_bytes()).is_err() {break;}
                }
            });
            self.stdin = Some(tx);
        }
        self.child = Some(child);
        self.events = Some(rx);
        self.log.clear();
        self.last_message = None;
        self.exit.clear();
        self.dismissed = false;
        Ok(())
    }
    fn poll(&mut self) {
        if self.stopping.as_ref().is_some_and(|rx| !matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty))) {
            self.stopping = None;
        }
        if let Some(events) = &self.events {
            for line in events.try_iter() {
                if is_dismissal(&line) { self.dismissed = true; }
                self.last_message = Some(Instant::now());
                self.log.push(line);
                if self.log.len() > 40 {
                    self.log.remove(0);
                }
            }
        }
        if let Some(child) = &mut self.child {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if status.code() == Some(42) { self.dismissed = true; }
                    self.exit = format!("Helper exited: {status}");
                    self.child = None;
                    self.stdin = None;
                }
                Err(e) => self.exit = format!("Cannot query helper: {e}"),
                _ => {}
            }
        }
    }
    fn send(&mut self, text: &str) -> Result<(), String> {
        self.stdin
            .as_ref()
            .ok_or("Helper stdin is not connected")?
            .try_send(format!("{text}\n"))
            .map_err(|e| e.to_string())
    }
    fn stop(&mut self) {
        let _ = self.send("quit");
        self.stdin = None;
        if let Some(mut child) = self.child.take() {
            let (tx, rx) = mpsc::channel();
            self.stopping = Some(rx);
            std::thread::spawn(move || {
            let until = Instant::now() + Duration::from_millis(1800);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    _ if Instant::now() >= until => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                    _ => std::thread::sleep(Duration::from_millis(15)),
                }
            }
            let _ = tx.send(());
            });
            self.exit = "Stopped by ArcaneVR".into();
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn nonreading_helper_cannot_block_send_or_shutdown() {
        use std::os::windows::process::CommandExt;
        let mut child=Command::new("powershell.exe").args(["-NoProfile","-Command","Start-Sleep -Seconds 30"]).stdin(Stdio::piped()).creation_flags(0x08000000).spawn().unwrap();
        let mut input=child.stdin.take().unwrap();
        let (tx,rx)=mpsc::sync_channel::<String>(1);
        std::thread::spawn(move || {while let Ok(text)=rx.recv(){if input.write_all(text.as_bytes()).is_err(){break;}}});
        let mut session=Session::default();session.child=Some(child);session.stdin=Some(tx);
        let start=Instant::now();
        let payload="x".repeat(1024*1024);
        for _ in 0..10 {let _=session.send(&payload);}
        session.stop();
        assert!(start.elapsed()<Duration::from_secs(1),"GUI must not wait on pipe or child");
        let deadline=Instant::now()+Duration::from_secs(5);
        while session.stopping.is_some() && Instant::now()<deadline {session.poll();std::thread::sleep(Duration::from_millis(10));}
        assert!(session.stopping.is_none(),"wedged helper must be reaped");
    }
    #[test]
    fn dismissal_is_not_confused_with_status_or_malformed_output() {
        assert!(is_dismissal(r#"{"type":"dismissed","message":"closed"}"#));
        assert!(!is_dismissal(r#"{"type":"status","message":"dismissed"}"#));
        assert!(!is_dismissal("dismissed"));
    }
    #[cfg(windows)]
    #[test]
    fn dismissed_helper_exit_disables_automatic_restart() {
        use std::os::windows::process::CommandExt;
        let child=Command::new("cmd.exe").args(["/c","exit","42"]).creation_flags(0x08000000).spawn().unwrap();
        let mut ui=OverlayUi::new(8811);ui.dashboard.child=Some(child);
        let deadline=Instant::now()+Duration::from_secs(3);
        while ui.dashboard.child.is_some() && Instant::now()<deadline {ui.poll();std::thread::sleep(Duration::from_millis(10));}
        assert!(ui.dashboard.child.is_none());assert!(ui.dashboard.dismissed);assert!(!ui.dashboard_wanted);
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}
fn reader(
    output: impl std::io::Read + Send + 'static,
    tx: mpsc::Sender<String>,
    prefix: &'static str,
) {
    std::thread::spawn(move || {
        for line in BufReader::new(output).lines() {
            match line {
                Ok(line) => {
                    let line = line.chars().take(3000).collect::<String>();
                    if tx.send(format!("{prefix}{line}")).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
}
#[derive(Default)]
pub struct OverlayUi {
    http_port: u16,
    dashboard: Session,
    dashboard_wanted: bool,
    dashboard_retry: Option<Instant>,

}
impl OverlayUi {
    pub fn new(http_port: u16) -> Self {
        Self {
            http_port,
            dashboard_wanted: true,
            ..Default::default()
        }
    }
    fn helper() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let parent = exe.parent()?;
        [
            parent.join("vrcraft-vr-overlay.exe"),
            parent.join("vr-overlay/bin/vrcraft-vr-overlay.exe"),
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../vr-overlay/bin/vrcraft-vr-overlay.exe"),
        ]
        .into_iter()
        .find(|p| p.is_file())
    }
    pub fn stop(&mut self) {
        self.dashboard_wanted=false;
        self.dashboard.stop();
    }
    pub fn poll(&mut self) {
        self.dashboard.poll();
        if self.dashboard.dismissed { self.dashboard_wanted = false; }
        if self.dashboard_wanted && self.dashboard.child.is_none() && self.dashboard_retry.is_none_or(|last|last.elapsed()>=Duration::from_secs(5)) {
            self.dashboard_retry=Some(Instant::now());
            if let Some(helper)=Self::helper(){let _=self.dashboard.start(&helper,&["--dashboard".into(),"--http-port".into(),self.http_port.to_string()]);}
        }
    }
}
