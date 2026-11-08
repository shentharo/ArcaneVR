//! Read-only OpenVR background client, owned by the receiver thread.
use std::{ffi::{c_char,c_void},time::{Instant,Duration},path::PathBuf};
include!(concat!(env!("OUT_DIR"),"/system_abi.rs"));
/// Use the shipped OpenVR client, which discovers the user's registered runtime.
/// The development fallback allows running directly from Cargo's target directory.
pub(crate) fn openvr_library_path() -> Result<PathBuf, String> {
 let executable = std::env::current_exe().map_err(|e| e.to_string())?;
 let root = executable.parent().ok_or("Executable directory unavailable")?;
 [root.join("openvr_api.dll"), PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../openvr/bin/win64/openvr_api.dll")]
  .into_iter().find(|path| path.is_file()).ok_or_else(|| "OpenVR runtime library unavailable".into())
}
struct Session { _library:libloading::Library, system:*const System, shutdown:unsafe extern "C" fn() }
impl Drop for Session {fn drop(&mut self){unsafe{(self.shutdown)()}}}
impl Session {
 fn connect()->Result<Self,String>{unsafe{
  let path=openvr_library_path()?;
  let library=libloading::Library::new(path).map_err(|e|e.to_string())?;
  let init=*library.get::<unsafe extern "C" fn(*mut i32,i32)->u32>(b"VR_InitInternal\0").map_err(|e|e.to_string())?;
  let shutdown=*library.get::<unsafe extern "C" fn()>(b"VR_ShutdownInternal\0").map_err(|e|e.to_string())?;
  let get=*library.get::<unsafe extern "C" fn(*const c_char,*mut i32)->*const c_void>(b"VR_GetGenericInterface\0").map_err(|e|e.to_string())?;
  let mut error=0;init(&mut error,3); // Background: never starts SteamVR or takes input focus.
  if error!=0{return Err(format!("SteamVR unavailable ({error})"));}
  let system=get(VERSION.as_ptr().cast(),&mut error).cast::<System>();
  if error!=0 || system.is_null(){shutdown();return Err(format!("SteamVR interface unavailable ({error})"));}
  Ok(Self{_library:library,system,shutdown})
 }}
}
#[derive(Default)] pub struct Monitor {session:Option<Session>,retry:Option<Instant>}
impl Monitor {
 pub fn snapshot(&mut self,runtime_live:bool)->serde_json::Value {
  if !runtime_live {self.session=None;return serde_json::json!({"available":false});}
  if self.session.is_none() && self.retry.is_none_or(|t|Instant::now()>=t) {self.session=Session::connect().ok();self.retry=Some(Instant::now()+Duration::from_secs(3));}
  let Some(session)=&self.session else{return serde_json::json!({"available":false})};
  unsafe {let api=&*session.system;let connected=|role|{let index=(api.GetTrackedDeviceIndexForControllerRole)(role);index<64 && (api.IsTrackedDeviceConnected)(index)};
   let mut left=connected(1);let mut right=connected(2);
   let devices:Vec<_>=(0..64).filter(|&index|(api.IsTrackedDeviceConnected)(index)).map(|index|{
    let class=(api.GetTrackedDeviceClass)(index);let role=(api.GetControllerRoleForTrackedDeviceIndex)(index);
    let mut error=0;let hint=(api.GetInt32TrackedDeviceProperty)(index,3007,&mut error);
    if class==2 && error==0 {left|=hint==1;right|=hint==2;}
    let mut name=[0u8;256];(api.GetStringTrackedDeviceProperty)(index,1001,name.as_mut_ptr().cast(),256,&mut error);
    let name=String::from_utf8_lossy(&name[..name.iter().position(|b|*b==0).unwrap_or(256)]);
    serde_json::json!({"index":index,"class":class,"role":role,"role_hint":hint,"model":name})
   }).collect();
   serde_json::json!({"available":true,"left":left,"right":right,"devices":devices})}

 }
}
