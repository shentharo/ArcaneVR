#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
//! SteamVR dashboard controls. No automatic arming and no motion injection.
use std::{ffi::{c_char,c_void}, io::{Read,Write}, net::TcpStream, path::PathBuf, sync::mpsc, thread, time::{Duration,Instant}};
use serde_json::{Value,json};
use font8x8::{UnicodeFonts,BASIC_FONTS};
const KEY: &[u8]=b"dev.vrcraft.controls\0";
const W:u32=768;
const H:u32=512;

// Windows x64 Valve ABI: 12-byte event header, 4-byte alignment padding,
// then VREvent_Data_t (six u64 reserved words). Checked against Valve header.
#[repr(C)]
#[derive(Default)]
pub struct VrEvent { kind:u32, device:u32, age:f32, padding:u32, data:[u64;6] }
impl VrEvent {
    fn mouse(&self)->(f32,f32,u32) {
        (f32::from_bits(self.data[0] as u32),f32::from_bits((self.data[0]>>32) as u32),self.data[1] as u32)
    }
}
include!(concat!(env!("OUT_DIR"),"/overlay_abi.rs"));

struct Vr { _library:libloading::Library, table:*const OverlayTable, shutdown:unsafe extern "C" fn() }
impl Vr {
    fn connect()->Result<Self,String> { unsafe {
        let exe=std::env::current_exe().map_err(|e|e.to_string())?;
        let dll_arg=std::env::args().collect::<Vec<_>>().windows(2).find(|s|s[0]=="--openvr-dll").map(|s|PathBuf::from(&s[1]));
        let candidates=[dll_arg,exe.parent().map(|p|p.join("openvr_api.dll")),Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../openvr/bin/win64/openvr_api.dll")),Some(PathBuf::from("C:/Program Files (x86)/Steam/steamapps/common/SteamVR/bin/win64/openvr_api.dll"))];
        let path=candidates.into_iter().flatten().find(|p|p.is_file()).ok_or("openvr_api.dll not found; place beside this EXE or pass --openvr-dll PATH")?;
        let library=libloading::Library::new(path).map_err(|e|e.to_string())?;
        let init=*library.get::<unsafe extern "C" fn(*mut i32,i32)->usize>(b"VR_InitInternal\0").map_err(|e|e.to_string())?;
        let shutdown=*library.get::<unsafe extern "C" fn()>(b"VR_ShutdownInternal\0").map_err(|e|e.to_string())?;
        let interface=*library.get::<unsafe extern "C" fn(*const c_char,*mut i32)->*const c_void>(b"VR_GetGenericInterface\0").map_err(|e|e.to_string())?;
        let mut error=0; init(&mut error,2); // VRApplication_Overlay
        if error!=0 {return Err(format!("SteamVR overlay initialization error {error}"));}
        let table=interface(OVERLAY_VERSION.as_ptr().cast(),&mut error).cast::<OverlayTable>();
        if error!=0 || table.is_null() {shutdown();return Err(format!("SteamVR overlay interface error {error}"));}
        Ok(Self{_library:library,table,shutdown})
    }}
    fn api(&self)->&OverlayTable {unsafe{&*self.table}}
}
impl Drop for Vr {fn drop(&mut self){unsafe{(self.shutdown)()}}}
fn check(code:i32,op:&str)->Result<(),String>{if code==0{Ok(())}else{Err(format!("{op}: OpenVR error {code}"))}}
struct OverlayLease<'a>{api:&'a OverlayTable,handle:u64}
impl Drop for OverlayLease<'_>{fn drop(&mut self){unsafe{(self.api.DestroyOverlay)(self.handle);}}}
fn unix_ms()->u64{std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis()as u64}
#[derive(Default)]
struct ImageFlow{pending:bool,failures:u32,submitted:u64,loaded:u64,last_error:i32,retry_at:Option<Instant>}
impl ImageFlow{
    fn ready(&self,now:Instant)->bool{!self.pending&&self.retry_at.is_none_or(|t|now>=t)}
    fn result(&mut self,code:i32,now:Instant){
        self.last_error=code;
        if code==0{self.pending=true;self.submitted+=1;self.retry_at=None;}else{
            self.pending=false;self.failures=self.failures.saturating_add(1);
            self.retry_at=Some(now+Duration::from_millis(250u64.saturating_mul(1<<self.failures.min(5))));
        }
    }
    fn complete(&mut self){self.pending=false;self.failures=0;self.loaded+=1;self.last_error=0;}
}

#[derive(Clone)]
struct Snapshot {state:Value,online:bool,deadline:Option<Instant>,message:String}
impl Default for Snapshot {fn default()->Self{Self{state:Value::Null,online:false,deadline:None,message:"Connecting to ArcaneVR...".into()}}}
#[derive(Clone,Copy,Debug,PartialEq)]
enum Action {Calibrate,Reset,Stop}
fn request(port:u16,command:Option<&str>)->Result<Value,String> {
    let mut stream=TcpStream::connect_timeout(&([127,0,0,1],port).into(),Duration::from_millis(200)).map_err(|_|"ArcaneVR receiver offline".to_string())?;
    stream.set_read_timeout(Some(Duration::from_millis(400))).map_err(|e|e.to_string())?;
    stream.set_write_timeout(Some(Duration::from_millis(200))).map_err(|e|e.to_string())?;
    let body=command.map(|c|json!({"command":c}).to_string()).unwrap_or_default();
    let(method,path)=if command.is_some(){("POST","/api/command")}else{("GET","/api/state")};
    write!(stream,"{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).map_err(|e|e.to_string())?;
    let mut response=String::new();stream.take(2_000_000).read_to_string(&mut response).map_err(|e|e.to_string())?;
    let(header,body)=response.split_once("\r\n\r\n").ok_or("Invalid receiver response")?;
    let value:Value=serde_json::from_str(body).map_err(|e|e.to_string())?;
    if !header.starts_with("HTTP/1.1 200") || command.is_some()&&value["error"].as_str().is_some_and(|s|!s.is_empty()) {return Err(value["error"].as_str().unwrap_or("Receiver rejected command").into());}
    Ok(value)
}
fn worker(port:u16)->(mpsc::Sender<Action>,mpsc::Receiver<Snapshot>) {
    let(tx,commands)=mpsc::channel();let(out,updates)=mpsc::channel();
    thread::spawn(move||{
        let mut snapshot=Snapshot::default();
        loop {
            match commands.recv_timeout(Duration::from_millis(100)) {
                Ok(action)=>{
                    snapshot.deadline=None;
                    let result=request(port,Some("stop")).and_then(|_|match action {
                        Action::Calibrate=>{snapshot.deadline=Some(Instant::now()+Duration::from_secs(3));Ok(())},
                        Action::Reset=>request(port,Some("reset_test")).map(|_|()),
                        Action::Stop=>Ok(()),
                    });
                    snapshot.message=match result{Ok(())=>match action{Action::Calibrate=>"Stand still and face forward".into(),Action::Reset=>"Test reset. Movement remains stopped".into(),Action::Stop=>"Stopped".into()},Err(e)=>e};
                },
                Err(mpsc::RecvTimeoutError::Disconnected)=>break,
                Err(mpsc::RecvTimeoutError::Timeout)=>{},
            }
            if snapshot.deadline.is_some_and(|d|Instant::now()>=d) {
                snapshot.deadline=None;
                snapshot.message=match request(port,Some("calibrate")){Ok(_)=>"Forward calibrated. Movement remains stopped".into(),Err(e)=>e};
            }
            match request(port,None){Ok(s)=>{snapshot.online=true;snapshot.state=s},Err(e)=>{snapshot.online=false;snapshot.deadline=None;snapshot.message=e;}}
            if out.send(snapshot.clone()).is_err(){break;}
        }
    });
    (tx,updates)
}

#[derive(Clone,Copy)]
struct Area {x:u32,y:u32,w:u32,h:u32}
impl Area {fn contains(self,x:f32,y:f32)->bool{x>=self.x as f32&&x<(self.x+self.w)as f32&&y>=self.y as f32&&y<(self.y+self.h)as f32}}
const BUTTONS:[(Action,Area);3]=[(Action::Calibrate,Area{x:32,y:214,w:704,h:74}),(Action::Reset,Area{x:32,y:301,w:342,h:74}),(Action::Stop,Area{x:390,y:301,w:346,h:74})];
type Color=[u8;4];
const BG:Color=[12,23,42,255]; const MINT:Color=[117,239,196,255];const WHITE:Color=[229,237,249,255];const MUTED:Color=[143,166,195,255];
struct Bitmap {pixels:Vec<u8>}
impl Bitmap {
    fn new()->Self{Self{pixels:BG.repeat((W*H)as usize)}}
    fn pixel(&mut self,x:u32,y:u32,c:Color){if x<W&&y<H{let i=((y*W+x)*4)as usize;self.pixels[i..i+4].copy_from_slice(&c);}}
    fn rect(&mut self,r:Area,c:Color){for y in r.y..(r.y+r.h).min(H){for x in r.x..(r.x+r.w).min(W){self.pixel(x,y,c)}}}
    fn text(&mut self,x:u32,y:u32,text:&str,size:u32,color:Color){let mut xx=x;for ch in text.chars(){if let Some(glyph)=BASIC_FONTS.get(ch){for(gy,row)in glyph.iter().enumerate(){for gx in 0..8{if(row>>gx)&1!=0{self.rect(Area{x:xx+gx*size,y:y+gy as u32*size,w:size,h:size},color);}}}}xx+=8*size;if xx+8*size>W{break;}}}
}
fn render(s:&Snapshot,hover:Option<Action>)->Bitmap{
    let mut b=Bitmap::new();
    b.rect(Area{x:0,y:0,w:W,h:7},MINT);
    b.text(32,29,"ArcaneVR",5,WHITE);
    b.text(34,80,"IN-HEADSET CONTROLS",2,MINT);
    let status=if !s.online{"RECEIVER OFFLINE"}else if s.state["armed"]==true{"MOVEMENT ARMED"}else if s.state["calibrated"]==true{"CALIBRATED / STOPPED"}else{"STOPPED / CALIBRATION NEEDED"};
    b.text(34,121,status,2,if s.online{MINT}else{[242,176,107,255]});
    let left=s.state["preview_steps"]["left"].as_u64().unwrap_or(0);let right=s.state["preview_steps"]["right"].as_u64().unwrap_or(0);
    b.text(34,158,&format!("STEPS  LEFT {left}    RIGHT {right}"),2,MUTED);
    for(action,r)in BUTTONS {
        let color=if action==Action::Stop{[135,44,65,255]}else if hover==Some(action){[40,78,94,255]}else{[25,49,70,255]};
        b.rect(r,color);b.rect(Area{x:r.x,y:r.y,w:4,h:r.h},if action==Action::Stop{[255,137,151,255]}else{MINT});
        let text=match action{Action::Calibrate=>"CALIBRATE FORWARD",Action::Reset=>"RESET TEST",Action::Stop=>"STOP"};
        b.text(r.x+22,r.y+19,text,if action==Action::Calibrate{3}else{3},WHITE);
        if action==Action::Calibrate{b.text(r.x+22,r.y+48,"Stops movement, then a 3-second countdown",1,MUTED);}
    }
    if let Some(d)=s.deadline {b.text(34,402,&format!("FACE FORWARD - {}",d.saturating_duration_since(Instant::now()).as_secs()+1),3,MINT)}else{b.text(34,401,&s.message,1,WHITE);}
    b.text(34,451,"STOP cancels the countdown.",2,MUTED);
    b.text(34,480,"Calibration and reset leave movement stopped.",1,MUTED);
    b
}
fn run()->Result<(),String>{
    if cfg!(not(all(target_os="windows",target_pointer_width="64"))){return Err("This companion supports Windows x64".into());}
    let args=std::env::args().collect::<Vec<_>>();
    let port=args.windows(2).find(|v|v[0]=="--http-port").and_then(|v|v[1].parse().ok()).unwrap_or(8811);
    let vr=Vr::connect()?;let api=vr.api();let mut handle=0;
    if args.iter().any(|v|v=="--probe") {
        unsafe{check((api.FindOverlay)(KEY.as_ptr().cast(),&mut handle),"FindOverlay")?;}
        let(mut width,mut height)=(0,0);let texture_result=unsafe{(api.GetOverlayTextureSize)(handle,&mut width,&mut height)};
        let report=json!({"overlay_key":"dev.vrcraft.controls","found":true,"handle":handle,"texture_width":width,"texture_height":height,"texture_query_result":texture_result,"event_size":std::mem::size_of::<VrEvent>(),"heartbeat_unix_ms":unix_ms()});
        if let Some(path)=args.windows(2).find(|v|v[0]=="--report").map(|v|&v[1]) {std::fs::write(path,report.to_string()).map_err(|e|e.to_string())?;}
        return Ok(());
    }
    unsafe{if(api.FindOverlay)(KEY.as_ptr().cast(),&mut handle)==0{return Ok(());}}
    let mut thumbnail=0;
    unsafe{
        check((api.CreateDashboardOverlay)(KEY.as_ptr().cast(),c"VRCraft".as_ptr(),&mut handle,&mut thumbnail),"CreateDashboardOverlay")?;
    }
    let _lease=OverlayLease{api,handle}; // Parent destruction also destroys its dashboard thumbnail.
    unsafe{
        check((api.SetOverlayWidthInMeters)(handle,1.45),"SetOverlayWidthInMeters")?;
        check((api.SetOverlayInputMethod)(handle,1),"SetOverlayInputMethod")?;
        check((api.SetOverlayMouseScale)(handle,&[W as f32,H as f32]),"SetOverlayMouseScale")?;
    }
    let mut icon=Bitmap::new();icon.text(110,145,"VR",24,MINT);icon.text(142,365,"VRCraft",6,WHITE);
    let mut thumb_flow=ImageFlow::default();let mut main_flow=ImageFlow::default();
    thumb_flow.result(unsafe{(api.SetOverlayRaw)(thumbnail,icon.pixels.as_mut_ptr().cast(),W,H,4)},Instant::now());
    let(tx,updates)=worker(port);let mut snapshot=Snapshot::default();let mut hover=None;let mut pressed=None;let mut quit=false;let mut last_render=Instant::now()-Duration::from_secs(1);
    let started=Instant::now();let mut last_report=started-Duration::from_secs(1);
    let mut image_pixels=vec![0u8;(W*H*4)as usize];
    let lifetime=args.windows(2).find(|v|v[0]=="--run-seconds").and_then(|v|v[1].parse::<u64>().ok()).map(|s|Instant::now()+Duration::from_secs(s));
    while !quit && !lifetime.is_some_and(|d|Instant::now()>=d) {
        for next in updates.try_iter(){snapshot=next;}
        let mut event=VrEvent::default();
        while unsafe{(api.PollNextOverlayEvent)(handle,&mut event,std::mem::size_of::<VrEvent>()as u32)} {
            if event.kind==700{quit=true;break;}
            if [300,301,302].contains(&event.kind){
                let(x,y,button)=event.mouse();
                hover=BUTTONS.iter().find(|(_,r)|r.contains(x,y)).map(|v|v.0);
                if event.kind==301 && button&1!=0 {pressed=hover;}
                if event.kind==302 && button&1!=0 {
                    if let Some(action)=hover.filter(|a|Some(*a)==pressed){let _=tx.send(action);}
                    pressed=None;
                }
            }
            if event.kind==304||event.kind==501{hover=None;pressed=None;}
            if event.kind==508{main_flow.complete();}
            if event.kind==517{main_flow.result(23,Instant::now());}
        }
        while unsafe{(api.PollNextOverlayEvent)(thumbnail,&mut event,std::mem::size_of::<VrEvent>()as u32)} {
            if event.kind==508{thumb_flow.complete();}
            if event.kind==517{thumb_flow.result(23,Instant::now());}
        }
        let visible=unsafe{(api.IsOverlayVisible)(handle)};
        if main_flow.ready(Instant::now()) && (main_flow.submitted==0||visible) && last_render.elapsed()>=Duration::from_millis(200){
            let mut bitmap=render(&snapshot,hover);
            main_flow.result(unsafe{(api.SetOverlayRaw)(handle,bitmap.pixels.as_mut_ptr().cast(),W,H,4)},Instant::now());
            last_render=Instant::now();
        }
        if thumb_flow.submitted==0 && thumb_flow.ready(Instant::now()) {
            thumb_flow.result(unsafe{(api.SetOverlayRaw)(thumbnail,icon.pixels.as_mut_ptr().cast(),W,H,4)},Instant::now());
        }
        if last_report.elapsed()>=Duration::from_secs(1){
            let(mut width,mut height)=(0,0);
            let texture_result=unsafe{(api.GetOverlayTextureSize)(handle,&mut width,&mut height)};
            let(mut image_width,mut image_height)=(0,0);
            let image_result=unsafe{(api.GetOverlayImageData)(handle,image_pixels.as_mut_ptr().cast(),image_pixels.len()as u32,&mut image_width,&mut image_height)};
            let image_verified=image_result==0&&image_width==W&&image_height==H&&image_pixels.iter().any(|v|*v!=0);
            if let Some(path)=args.windows(2).find(|v|v[0]=="--report").map(|v|&v[1]) {
                let report=json!({"overlay_key":"dev.vrcraft.controls","handle":handle,"thumbnail":thumbnail,"lifecycle":"running","heartbeat_unix_ms":unix_ms(),"uptime_ms":started.elapsed().as_millis()as u64,"raw_submissions":main_flow.submitted,"image_loaded_events":main_flow.loaded,"image_pending":main_flow.pending,"last_submit_error":main_flow.last_error,"consecutive_submit_failures":main_flow.failures,"visible":visible,"texture_query_result":texture_result,"texture_width":width,"texture_height":height,"owner_image_query_result":image_result,"owner_image_width":image_width,"owner_image_height":image_height,"owner_image_verified":image_verified,"receiver_online":snapshot.online,"pid":std::process::id()});
                let _=std::fs::write(path,report.to_string());
            }
            last_report=Instant::now();
        }
        thread::sleep(Duration::from_millis(12));
    }
    drop(tx);
    if let Some(path)=args.windows(2).find(|v|v[0]=="--report").map(|v|&v[1]) {
        let _=std::fs::write(path,json!({"lifecycle":"stopped","heartbeat_unix_ms":unix_ms(),"pid":std::process::id()}).to_string());
    }
    Ok(())
}
fn main(){if let Err(error)=run(){let path=std::env::temp_dir().join("vrcraft-steamvr-controls-error.txt");let _=std::fs::write(path,error);std::process::exit(1);}}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]fn valve_event_abi(){assert_eq!(std::mem::size_of::<VrEvent>(),64);assert_eq!(std::mem::offset_of!(VrEvent,data),16);}
    #[test]fn button_hit_regions_are_separate(){for(a,r)in BUTTONS{assert!(r.contains((r.x+2)as f32,(r.y+2)as f32));for(other,o)in BUTTONS{if a!=other{assert!(!o.contains((r.x+2)as f32,(r.y+2)as f32));}}}}
    #[test]fn raster_has_expected_size(){assert_eq!(render(&Snapshot::default(),None).pixels.len(),(W*H*4)as usize);}
    #[test]fn one_raw_image_in_flight(){let now=Instant::now();let mut flow=ImageFlow::default();assert!(flow.ready(now));flow.result(0,now);assert!(!flow.ready(now+Duration::from_secs(60)));flow.complete();assert!(flow.ready(now));assert_eq!(flow.loaded,1);}
    #[test]fn transient_submit_failure_backs_off(){let now=Instant::now();let mut flow=ImageFlow::default();flow.result(23,now);assert!(!flow.ready(now));assert!(flow.ready(now+Duration::from_secs(9)));assert_eq!(flow.submitted,0);}
}
