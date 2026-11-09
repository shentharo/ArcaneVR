//! Opt-in app-only render capture for CLI QA; never captures other windows.
use eframe::egui;
use std::time::{Duration, Instant};
pub fn path()->Option<String>{std::env::args().collect::<Vec<_>>().windows(2).find(|p|p[0]=="--qa-screenshot").map(|p|p[1].clone())}
pub fn page()->Option<String>{std::env::args().collect::<Vec<_>>().windows(2).find(|p|p[0]=="--qa-page").map(|p|p[1].clone())}
pub fn update(ctx:&egui::Context){update_embedded(ctx,false);}
pub fn update_embedded(ctx:&egui::Context, embedded_ready:bool){
 let Some(path)=path() else{return};
 for event in ctx.input(|i|i.events.clone()) {if let egui::Event::Screenshot{image,..}=event {
  let result=(||->Result<(),Box<dyn std::error::Error>>{let file=std::fs::File::create(&path)?;let mut encoder=png::Encoder::new(file,image.size[0] as u32,image.size[1] as u32);encoder.set_color(png::ColorType::Rgba);encoder.set_depth(png::BitDepth::Eight);let mut writer=encoder.write_header()?;let bytes:Vec<u8>=image.pixels.iter().flat_map(|p|p.to_array()).collect();writer.write_image_data(&bytes)?;Ok(())})();
  eprintln!("App render capture {path}: {result:?}");ctx.send_viewport_cmd(egui::ViewportCommand::Close);return;
 }}
 // The cog QA action requests its capture after drawing the first destination
 // frame, while the deliberately delayed renderer is still loading.
 if std::env::args().any(|arg|arg=="--qa-cog"){return;}
 if std::env::var_os("ARCANEVR_EMBED_QA").is_some() {
  // QA-only capture waits for real native embedding and navigation completion;
  // frame counts can race a slow GPU/child process during DX12 initialization.
  let started_key=egui::Id::new("embed qa capture started");
  let started=ctx.data_mut(|d|{let started=d.get_temp::<Instant>(started_key).unwrap_or_else(Instant::now);d.insert_temp(started_key,started);started});
  let nav_complete=std::env::var_os("ARCANEVR_EMBED_QA_NAVIGATE").is_none()
   || ctx.data(|d|d.get_temp::<bool>(egui::Id::new("embed qa navigation complete"))).unwrap_or(false);
  let requested_key=egui::Id::new("embed qa screenshot requested");
  let requested=ctx.data(|d|d.get_temp::<bool>(requested_key)).unwrap_or(false);
  if !requested && embed_capture_ready(started.elapsed(),embedded_ready,nav_complete) {
   ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
   ctx.data_mut(|d|d.insert_temp(requested_key,true));
  }
  if started.elapsed()>Duration::from_secs(40){eprintln!("Embedded app screenshot timed out waiting for child/navigation readiness");ctx.send_viewport_cmd(egui::ViewportCommand::Close);}
  return;
 }
 let capture_frame=if page().as_deref()==Some("updates"){180}else{20};
 if ctx.cumulative_frame_nr()==capture_frame{ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));}
 if ctx.cumulative_frame_nr()>capture_frame+180{eprintln!("App screenshot timed out");ctx.send_viewport_cmd(egui::ViewportCommand::Close);}
}

fn embed_capture_ready(elapsed:Duration, embedded_ready:bool, navigation_complete:bool)->bool {
 elapsed>=Duration::from_secs(8) && embedded_ready && navigation_complete
}

#[cfg(test)]
#[test]
fn embedded_capture_waits_for_wall_time_and_returned_native_child(){
 assert!(!embed_capture_ready(Duration::from_secs(7),true,true));
 assert!(!embed_capture_ready(Duration::from_secs(8),false,true));
 assert!(!embed_capture_ready(Duration::from_secs(8),true,false));
 assert!(embed_capture_ready(Duration::from_secs(8),true,true));
}
