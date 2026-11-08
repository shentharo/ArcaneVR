//! Explicit, asynchronous scan of the PC's Wi-Fi adapter. No saved profiles are read.
use std::sync::mpsc;

pub struct WifiScan {
    pub networks: Vec<String>,
    pub busy: bool,
    pub status: String,
    receiver: Option<mpsc::Receiver<Result<Vec<String>, String>>>,
}
impl Default for WifiScan {
    fn default() -> Self { Self { networks: Vec::new(), busy: false, status: "Scan for nearby 2.4 GHz networks, or enter a name manually.".into(), receiver: None } }
}
impl WifiScan {
    pub fn start(&mut self) {
        if self.busy { return; }
        let (tx,rx)=mpsc::channel(); self.receiver=Some(rx);self.busy=true;
        self.status="Scanning the PC's Wi-Fi adapter…".into();
        std::thread::spawn(move || { let _=tx.send(scan()); });
    }
    pub fn poll(&mut self) {
        let result=self.receiver.as_ref().and_then(|r| match r.try_recv() {Ok(v)=>Some(v),Err(mpsc::TryRecvError::Disconnected)=>Some(Err("Wi-Fi scan stopped unexpectedly.".into())),Err(mpsc::TryRecvError::Empty)=>None});
        if let Some(result)=result { self.receiver=None;self.busy=false;match result {
            Ok(networks)=>{ self.status=if networks.is_empty(){"No nearby 2.4 GHz networks found. Enter a name manually.".into()}else{format!("Found {} nearby 2.4 GHz network(s).",networks.len())};self.networks=networks; },
            Err(error)=>{self.networks.clear();self.status=error;}
        }}
    }
}
#[cfg(windows)]
fn scan()->Result<Vec<String>,String> {
    use std::os::windows::process::CommandExt;
    let output=std::process::Command::new("netsh").args(["wlan","show","networks","mode=bssid"]).creation_flags(0x08000000).output().map_err(|_|"Could not start the Windows Wi-Fi scan. Enter the network name manually.".to_string())?;
    if !output.status.success() {return Err("Windows could not scan Wi-Fi. Check that the PC Wi-Fi adapter is enabled, or enter the name manually.".into());}
    let text=String::from_utf8_lossy(&output.stdout);
    parse_networks(&text).ok_or_else(||"Windows returned an unrecognized Wi-Fi scan format or language. Enter the network name manually.".into())
}
#[cfg(not(windows))]
fn scan()->Result<Vec<String>,String> {Err("PC Wi-Fi scanning is only available on Windows. Enter the network name manually.".into())}

fn parse_networks(text:&str)->Option<Vec<String>> {
    let mut networks=Vec::new();let mut ssid=String::new();let mut band:Option<String>=None;let mut channel:Option<u16>=None;let mut bssid=false;let mut recognized=false;
    fn finish(out:&mut Vec<String>, name:&str, bssid:bool, band:&Option<String>, channel:Option<u16>) {
        let eligible=match band {Some(b)=>b.replace(' ',"").eq_ignore_ascii_case("2.4GHz"),None=>channel.is_some_and(|c|(1..=14).contains(&c))};
        if bssid && eligible && !name.is_empty() && !out.iter().any(|x|x==name) {out.push(name.to_owned());}
    }
    for line in text.lines() {
        let line=line.trim();
        if line.starts_with("There are ") && line.contains("networks currently visible") {recognized=true;}
        let Some((key,value))=line.split_once(':') else {continue}; let key=key.trim();let value=value.trim();
        if let Some(num)=key.strip_prefix("SSID ") {
            if num.trim().parse::<u32>().is_ok() {finish(&mut networks,&ssid,bssid,&band,channel);ssid=value.to_string();band=None;channel=None;bssid=false;recognized=true;}
        } else if let Some(num)=key.strip_prefix("BSSID ") {
            if num.trim().parse::<u32>().is_ok() {finish(&mut networks,&ssid,bssid,&band,channel);band=None;channel=None;bssid=true;recognized=true;}
        } else if key.eq_ignore_ascii_case("Band") && bssid {band=Some(value.to_owned());}
        else if key.eq_ignore_ascii_case("Channel") && bssid {channel=value.parse().ok();}
    }
    finish(&mut networks,&ssid,bssid,&band,channel);
    recognized.then_some(networks)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn scans_multiband_and_rejects_six_ghz_channel_one() {
        let data="There are 5 networks currently visible.\nSSID 1 : Mixed\n    BSSID 1 : 00:11:22:33:44:55\n    Band : 5 GHz\n    Channel : 36\n    BSSID 2 : 00:11:22:33:44:56\n    Band : 2.4 GHz\n    Channel : 6\nSSID 2 : Only5\n    BSSID 1 : aa:bb:cc:dd:ee:ff\n    Channel : 44\nSSID 3 : Six\n    BSSID 1 : aa:bb:cc:dd:ee:00\n    Band : 6 GHz\n    Channel : 1\nSSID 4 : Legacy\n    BSSID 1 : aa:bb:cc:dd:ee:01\n    Channel : 11\nSSID 5 :\n    BSSID 1 : aa:bb:cc:dd:ee:02\n    Band : 2.4 GHz\n    Channel : 1\nSSID 6 : Mixed\n    BSSID 1 : aa:bb:cc:dd:ee:03\n    Channel : 1\n";
        assert_eq!(parse_networks(data).unwrap(),vec!["Mixed","Legacy"]);
    }
    #[test] fn distinguishes_empty_from_unrecognized_output() {
        assert_eq!(parse_networks("There are 0 networks currently visible."),Some(vec![]));
        assert!(parse_networks("Keine Netzwerke verfügbar").is_none());
        assert!(parse_networks("There is no wireless interface on the system.").is_none());
    }
    #[test] fn default_is_inert() { let scan=WifiScan::default();assert!(!scan.busy);assert!(scan.networks.is_empty()); }
}
