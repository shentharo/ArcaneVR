//! Explicit GitHub release updates. No startup network calls or configuration writes.
use arcane_bridge_core::update_manifest::{parse_release, Release, CURRENT_VERSION};
use base64::Engine;
use eframe::egui;
use serde_json::{json, Value};
use std::{path::PathBuf, process::Command, sync::mpsc, thread};

const INSTALLER: &str = include_str!("update_install.ps1");
const API: &str = "https://api.github.com/repos/shentharo/ArcaneVR/releases/latest";

enum Event { Checked(Result<Option<Release>, String>), Staged(Result<PathBuf, String>) }
pub struct Updater {
    qa_fixture: bool,
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
        let args: Vec<String> = std::env::args().collect();
        let qa = args.windows(2).any(|p| p[0]=="--qa-page" && p[1]=="updates");
        let fixture = args.windows(2).find(|p| p[0]=="--qa-update-state")
            .filter(|_| qa || args.iter().any(|a| a=="--qa-screenshot"))
            .map(|p| p[1].as_str());
        let mut updater = Self { qa_fixture: false, busy: qa, message: if qa {"Checking GitHub releases…".into()} else {String::new()}, release: None, request: None, events, sender };
        if let Some(state) = fixture { updater.set_qa_fixture(state); return updater; }
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
    let restart_working_directory = std::env::current_dir().map_err(|e| format!("Cannot preserve the application's working directory: {e}"))?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_nanos();
    let folder = target.parent().ok_or("Executable directory unavailable")?.join(".arcane-update").join(format!("{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&folder).map_err(|e| format!("Cannot stage update beside installed executable: {e}"))?;
    let candidate = folder.join("candidate.exe");
    let output = run_ps(&format!(r#"
$ErrorActionPreference='Stop'; [Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::Tls12
$url={}; $candidate={}; $expectedSize={}; $expectedHash={}
$downloadError=$null
for($attempt=0; $attempt -lt 3; $attempt++) {{
    try {{
        $downloadUrl = if ($attempt -eq 0) {{ $url }} else {{ $url + '?arcane_retry=' + [guid]::NewGuid().ToString('N') }}
        Invoke-WebRequest -UseBasicParsing -Uri $downloadUrl -OutFile $candidate -TimeoutSec 120
        $f=Get-Item -LiteralPath $candidate; if($f.Length -ne $expectedSize){{throw 'Download size mismatch'}}
        $hash=(Get-FileHash -LiteralPath $candidate -Algorithm SHA256).Hash
        if($hash -ine $expectedHash){{throw 'Download checksum mismatch'}}
        $downloadError=$null; break
    }} catch {{ $downloadError=$_.Exception.Message; if ($attempt -lt 2) {{ Start-Sleep -Seconds 2 }} }}
}}
if($downloadError){{throw "Download failed after retries: $downloadError"}}
$original=(Get-FileHash -LiteralPath {} -Algorithm SHA256).Hash
$ticks=(Get-Process -Id {}).StartTime.ToUniversalTime().Ticks
@{{original=$original;ticks=$ticks}} | ConvertTo-Json -Compress
"#, literal(&release.executable_url), literal(&candidate.to_string_lossy()), release.size, literal(&release.sha256), literal(&target.to_string_lossy()), std::process::id()))?;
    let details: Value = serde_json::from_str(&output).map_err(|e| e.to_string())?;
    let request = folder.join("request.json");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let data = json!({"target":target, "sha256":release.sha256, "original_sha256":details["original"], "parent_pid":std::process::id(), "parent_start_ticks":details["ticks"], "restart_args":args, "restart_working_directory":restart_working_directory});
    std::fs::write(&request, serde_json::to_vec_pretty(&data).unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(folder.join("install.ps1"), INSTALLER).map_err(|e| e.to_string())?;
    Ok(request)
}
impl Updater {
    // Opt-in rendering fixtures never contact GitHub or launch an installer.
    fn set_qa_fixture(&mut self, state: &str) {
        self.qa_fixture = true;
        self.busy = state == "downloading";
        let (prefix, patch) = CURRENT_VERSION.rsplit_once('.').unwrap_or((CURRENT_VERSION, "0"));
        let version = format!("{prefix}.{}", patch.parse::<u32>().unwrap_or(0) + 1);
        if matches!(state, "available" | "downloading" | "ready") {
            self.release = Some(Release { version: version.clone(), executable_url: String::new(), sha256: String::new(), notes_url: "https://github.com/shentharo/ArcaneVR/releases".into(), size: 0 });
        }
        if state == "ready" { self.request = Some(PathBuf::from("qa-fixture-not-an-install-request")); }
        self.message = match state {
            "available" => format!("Version {version} is available."),
            "current" => "No newer published release is available.".into(),
            "error" => "Update failed: QA connection error".into(),
            "downloading" => "Downloading and verifying…".into(),
            "ready" => "Download verified. Ready to install and restart.".into(),
            _ => "Unknown QA update state".into(),
        };
    }
    pub fn show(&mut self, ui: &mut egui::Ui, can_install: bool) -> bool {
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
                Event::Checked(Ok(Some(release))) => { self.request=None; self.message=format!("Version {} is available.", release.version); self.release=Some(release); }
                Event::Checked(Ok(None)) => { self.release=None; self.request=None; self.message="No newer published release is available.".into(); }
                Event::Checked(Err(e)) => { self.release=None; self.request=None; self.message=format!("Update failed: {e}"); }
                Event::Staged(Err(e)) => { self.request=None; self.message=format!("Update failed: {e}"); }
                Event::Staged(Ok(request)) => { self.request=Some(request); self.message="Download verified. Ready to install and restart.".into(); }
            }
        }
        let mut install=false;
        ui.horizontal_wrapped(|ui| {
            if self.qa_fixture { ui.label("QA preview"); }
            ui.label(format!("Installed version {CURRENT_VERSION}"));
            if ui.add_enabled(!self.busy && !self.qa_fixture, egui::Button::new("Check for updates")).clicked() {
                self.busy=true; self.release=None; self.request=None; self.message="Checking GitHub releases…".into();
                let tx=self.sender.clone(); thread::spawn(move || { let _=tx.send(Event::Checked(check())); });
            }
            if self.busy { ui.spinner(); }
            if !self.message.is_empty() { ui.label(&self.message); }
            if let Some(release)=&self.release {
                ui.hyperlink_to("Release notes", &release.notes_url);
                if self.request.is_none() && ui.add_enabled(!self.busy && !self.qa_fixture, egui::Button::new("Download update")).clicked() {
                    self.busy=true; self.message="Downloading and verifying…".into();
                    let tx=self.sender.clone(); let release=release.clone(); thread::spawn(move || { let _=tx.send(Event::Staged(stage(&release))); });
                }
            }
            if self.request.is_some() {
                ui.label("ArcaneVR will close and restart. Your sensor settings are kept.");
                if ui.add_enabled(can_install && !self.busy && !self.qa_fixture, egui::Button::new("Install and restart")).clicked() { install=true; }
                if !can_install { ui.label("Stop game movement and wait for the receiver before installing."); }
            }
        });
        install
    }
    pub fn launch_installer(&mut self) -> Result<(), String> {
        if self.qa_fixture { return Err("QA update previews cannot install updates".into()); }
        let request=self.request.as_ref().ok_or("No verified update staged")?;
        let script=request.with_file_name("install.ps1");
        ps(&format!("& {} -Request {}", literal(&script.to_string_lossy()), literal(&request.to_string_lossy())))
            .spawn().map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn error(&mut self, error: String) { self.message=error; }
}

#[cfg(test)]
mod download_tests {
    use super::*;
    fn release() -> Release {
        Release { version: "9.9.9".into(), executable_url: String::new(), sha256: String::new(), notes_url: "https://github.com/shentharo/ArcaneVR/releases".into(), size: 0 }
    }
    fn frame(updater: &mut Updater, ctx: &egui::Context, can_install: bool, events: Vec<egui::Event>) -> (bool, Vec<(String, egui::Rect)>) {
        let mut install = false;
        let output = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200., 800.))), events, ..Default::default() }, |ctx| {
            egui::TopBottomPanel::bottom("updates").show(ctx, |ui| { install = updater.show(ui, can_install); });
            egui::CentralPanel::default().show(ctx, |ui| { ui.label("Home page content"); });
        });
        fn texts(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
            match shape {
                egui::Shape::Text(t) => out.push((t.galley.job.text.clone(), egui::Rect::from_min_size(t.pos, t.galley.size()))),
                egui::Shape::Vec(shapes) => for shape in shapes { texts(shape, out); },
                _ => (),
            }
        }
        let mut text = Vec::new();
        for clipped in output.shapes { texts(&clipped.shape, &mut text); }
        assert!(ctx.memory(|m| m.areas().visible_layer_ids().iter().all(|layer| layer.order != egui::Order::Middle)), "updater must not create a floating window");
        (install, text)
    }
    fn render(updater: &mut Updater) -> Vec<String> {
        frame(updater, &egui::Context::default(), true, vec![]).1.into_iter().map(|(text, _)| text).collect()
    }
    #[test]
    fn renders_installed_version_and_available_controls_inline_with_home() {
        let mut updater = Updater::default();
        let text = render(&mut updater);
        assert!(text.contains(&format!("Installed version {CURRENT_VERSION}")));
        assert!(text.contains(&"Check for updates".into()));
        assert!(text.contains(&"Home page content".into()));
        assert!(!text.contains(&"Download update".into()));
        updater.sender.send(Event::Checked(Ok(Some(release())))).unwrap();
        let text = render(&mut updater);
        for expected in ["Version 9.9.9 is available.", "Release notes", "Download update"] { assert!(text.contains(&expected.into()), "missing {expected}"); }
        assert!(!text.contains(&"Install and restart".into()));
    }
    #[test]
    fn check_results_clear_stale_release_and_staging_errors_allow_download_retry() {
        let mut updater = Updater::default();
        for event in [Event::Checked(Ok(None)), Event::Checked(Err("offline".into()))] {
            updater.release = Some(release()); updater.request = Some("stale.json".into()); updater.busy = true;
            updater.sender.send(event).unwrap();
            render(&mut updater);
            assert!(!updater.busy); assert!(updater.release.is_none()); assert!(updater.request.is_none());
        }
        updater.release = Some(release()); updater.request = Some("stale.json".into()); updater.busy = true;
        updater.sender.send(Event::Staged(Err("checksum mismatch".into()))).unwrap();
        let text = render(&mut updater);
        assert!(text.contains(&"Update failed: checksum mismatch".into()));
        assert!(text.contains(&"Download update".into()));
        assert!(!updater.busy); assert!(updater.request.is_none());
    }
    #[test]
    fn verified_install_requires_guard_and_survives_launch_error_for_retry() {
        let mut updater = Updater::default();
        updater.release = Some(release());
        updater.sender.send(Event::Staged(Ok("verified-request.json".into()))).unwrap();
        let ctx = egui::Context::default();
        frame(&mut updater, &ctx, false, vec![]);
        let (_, text) = frame(&mut updater, &ctx, false, vec![]);
        let pos = text.iter().find(|(t, _)| t == "Install and restart").unwrap().1.center();
        let click = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        frame(&mut updater, &ctx, false, vec![egui::Event::PointerMoved(pos), click(true)]);
        assert!(!frame(&mut updater, &ctx, false, vec![click(false)]).0);
        frame(&mut updater, &ctx, true, vec![egui::Event::PointerMoved(pos)]);
        frame(&mut updater, &ctx, true, vec![click(true)]);
        assert!(frame(&mut updater, &ctx, true, vec![click(false)]).0);
        updater.error("Update was not installed: helper unavailable".into());
        let text = render(&mut updater);
        assert!(text.contains(&"Install and restart".into()));
        assert!(updater.request.is_some());
    }
    #[test]
    fn qa_fixture_cannot_launch_installer() {
        let mut updater = Updater::default();
        updater.set_qa_fixture("ready");
        assert!(render(&mut updater).contains(&"QA preview".into()));
        assert_eq!(updater.launch_installer().unwrap_err(), "QA update previews cannot install updates");
    }
    #[test]
    #[ignore = "downloads the public release for updater integration QA"]
    fn stages_real_release_with_verified_checksum() {
        let release = Release {
            version: "1.1.5".into(),
            executable_url: "https://github.com/shentharo/ArcaneVR/releases/download/v1.1.5/ArcaneVR.exe".into(),
            sha256: "fc2f74eec8a702faefae5364e0e2c2dc1b87bd25c66d70ebde5e16a741921fa0".into(),
            notes_url: "https://github.com/shentharo/ArcaneVR/releases/tag/v1.1.5".into(),
            size: 70119424,
        };
        let request = stage(&release).expect("release download and verification");
        assert!(request.exists());
    }
}
