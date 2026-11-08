//! Explicit GitHub release updates. No startup network calls or configuration writes.
use arcane_bridge_core::update_manifest::{parse_release, Release, CURRENT_VERSION};
use base64::Engine;
use eframe::egui;
use serde_json::{json, Value};
use std::{path::PathBuf, process::Command, sync::mpsc, thread};

const INSTALLER: &str = include_str!("update_install.ps1");
const API: &str = "https://api.github.com/repos/shentharo/VRCRAFT/releases/latest";

enum Event { Checked(Result<Option<Release>, String>), Staged(Result<PathBuf, String>) }
pub struct Updater {
    open: bool,
    busy: bool,
    message: String,
    release: Option<Release>,
    request: Option<PathBuf>,
    events: mpsc::Receiver<Event>,
    sender: mpsc::Sender<Event>,
}
impl Default for Updater {
    fn default() -> Self {
        let (sender, events) = mpsc::channel();
        let qa = std::env::args().collect::<Vec<_>>().windows(2).any(|p| p[0]=="--qa-page" && p[1]=="updates");
        let updater = Self { open: qa, busy: qa, message: if qa {"Checking GitHub releases…".into()} else {String::new()}, release: None, request: None, events, sender };
        if qa { let tx=updater.sender.clone(); thread::spawn(move || { let _=tx.send(Event::Checked(check())); }); }
        updater
    }
}
fn ps(script: &str) -> Command {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut cmd = Command::new("powershell.exe");
    // A PowerShell 7 launcher can export an incompatible Windows PowerShell module path.
    cmd.env_remove("PSModulePath");
    cmd.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", &base64::engine::general_purpose::STANDARD.encode(bytes)]);
    #[cfg(windows)] { use std::os::windows::process::CommandExt; cmd.creation_flags(0x08000000); }
    cmd
}
fn run_ps(script: &str) -> Result<String, String> {
    let output = ps(script).output().map_err(|e| format!("Could not start update helper: {e}"))?;
    if !output.status.success() { return Err(String::from_utf8_lossy(&output.stderr).trim().chars().take(600).collect()); }
    Ok(String::from_utf8_lossy(&output.stdout).trim_start_matches('\u{feff}').trim().to_owned())
}
fn literal(value: &str) -> String { format!("'{}'", value.replace('\'', "''")) }
fn check() -> Result<Option<Release>, String> {
    let text = run_ps(&format!(r#"
$ErrorActionPreference='Stop'; [Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::Tls12
try {{ $r=Invoke-WebRequest -UseBasicParsing -Uri '{API}' -Headers @{{'User-Agent'='ArcaneVR-Updater';'Accept'='application/vnd.github+json'}} -TimeoutSec 20; if($r.RawContentLength -gt 1048576){{throw 'Release response too large'}}; [Console]::Out.Write($r.Content) }}
catch {{ if ($_.Exception.Response -and [int]$_.Exception.Response.StatusCode -eq 404) {{ [Console]::Out.Write('null') }} else {{ throw }} }}
"#))?;
    let value: Value = serde_json::from_str(&text).map_err(|e| format!("Invalid release response: {e}"))?;
    if value.is_null() { return Ok(None); }
    parse_release(&value, CURRENT_VERSION)
}
fn stage(release: &Release) -> Result<PathBuf, String> {
    let target = std::env::current_exe().map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_nanos();
    let folder = target.parent().ok_or("Executable directory unavailable")?.join(".arcane-update").join(format!("{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&folder).map_err(|e| format!("Cannot stage update beside installed executable: {e}"))?;
    let candidate = folder.join("candidate.exe");
    let output = run_ps(&format!(r#"
$ErrorActionPreference='Stop'; [Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::Tls12
Invoke-WebRequest -UseBasicParsing -Uri {} -OutFile {} -TimeoutSec 120
$f=Get-Item -LiteralPath {}; if($f.Length -ne {}){{throw 'Download size mismatch'}}
$hash=(Get-FileHash -LiteralPath {} -Algorithm SHA256).Hash
if($hash -ine {}){{throw 'Download checksum mismatch'}}
$original=(Get-FileHash -LiteralPath {} -Algorithm SHA256).Hash
$ticks=(Get-Process -Id {}).StartTime.ToUniversalTime().Ticks
@{{original=$original;ticks=$ticks}} | ConvertTo-Json -Compress
"#, literal(&release.executable_url), literal(&candidate.to_string_lossy()), literal(&candidate.to_string_lossy()), release.size, literal(&candidate.to_string_lossy()), literal(&release.sha256), literal(&target.to_string_lossy()), std::process::id()))?;
    let details: Value = serde_json::from_str(&output).map_err(|e| e.to_string())?;
    let request = folder.join("request.json");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let data = json!({"target":target, "sha256":release.sha256, "original_sha256":details["original"], "parent_pid":std::process::id(), "parent_start_ticks":details["ticks"], "restart_args":args});
    std::fs::write(&request, serde_json::to_vec_pretty(&data).unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(folder.join("install.ps1"), INSTALLER).map_err(|e| e.to_string())?;
    Ok(request)
}
impl Updater {
    pub fn show(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, can_install: bool) -> bool {
        for event in self.events.try_iter() {
            self.busy = false;
            if let Event::Checked(result)=&event {
                let args: Vec<String>=std::env::args().collect();
                if let Some(pair)=args.windows(2).find(|p|p[0]=="--qa-update-report") {
                    let value=match result { Ok(Some(r))=>json!({"state":"available","version":r.version}), Ok(None)=>json!({"state":"no_newer_release","current":CURRENT_VERSION}), Err(e)=>json!({"state":"error","error":e}) };
                    let _=std::fs::write(&pair[1],value.to_string());
                }
            }
            match event {
                Event::Checked(Ok(Some(release))) => { self.message=format!("Version {} is available.", release.version); self.release=Some(release); }
                Event::Checked(Ok(None)) => self.message="No newer published release is available.".into(),
                Event::Checked(Err(e)) | Event::Staged(Err(e)) => self.message=format!("Update failed: {e}"),
                Event::Staged(Ok(request)) => { self.request=Some(request); self.message="Download verified. Ready to install and restart.".into(); }
            }
        }
        if ui.button("Updates").clicked() { self.open=true; }
        let mut install=false;
        egui::Window::new("ArcaneVR updates").open(&mut self.open).resizable(false).default_width(480.).show(ctx, |ui| {
            ui.label(format!("Installed version {CURRENT_VERSION}"));
            if ui.add_enabled(!self.busy, egui::Button::new("Check for updates")).clicked() {
                self.busy=true; self.release=None; self.request=None; self.message="Checking GitHub releases…".into();
                let tx=self.sender.clone(); thread::spawn(move || { let _=tx.send(Event::Checked(check())); });
            }
            ui.label(&self.message);
            if let Some(release)=&self.release {
                ui.hyperlink_to("Release notes", &release.notes_url);
                if self.request.is_none() && ui.add_enabled(!self.busy, egui::Button::new("Download update")).clicked() {
                    self.busy=true; self.message="Downloading and verifying…".into();
                    let tx=self.sender.clone(); let release=release.clone(); thread::spawn(move || { let _=tx.send(Event::Staged(stage(&release))); });
                }
            }
            if self.request.is_some() {
                ui.label("ArcaneVR will close and restart. Your sensor settings are kept.");
                if ui.add_enabled(can_install && !self.busy, egui::Button::new("Install and restart")).clicked() { install=true; }
                if !can_install { ui.label("Stop game movement and wait for the receiver before installing."); }
            }
        });
        install
    }
    pub fn launch_installer(&mut self) -> Result<(), String> {
        let request=self.request.as_ref().ok_or("No verified update staged")?;
        let script=request.with_file_name("install.ps1");
        ps(&format!("& {} -Request {}", literal(&script.to_string_lossy()), literal(&request.to_string_lossy())))
            .spawn().map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn error(&mut self, error: String) { self.message=error; }
}
