//! Opt-in app-only render capture for CLI QA; never captures other windows.
use eframe::egui;
pub fn path()->Option<String>{std::env::args().collect::<Vec<_>>().windows(2).find(|p|p[0]=="--qa-screenshot").map(|p|p[1].clone())}
pub fn page()->Option<String>{std::env::args().collect::<Vec<_>>().windows(2).find(|p|p[0]=="--qa-page").map(|p|p[1].clone())}
pub fn update(ctx:&egui::Context){
 let Some(path)=path() else{return};
 for event in ctx.input(|i|i.events.clone()) {if let egui::Event::Screenshot{image,..}=event {
  let result=(||->Result<(),Box<dyn std::error::Error>>{let file=std::fs::File::create(&path)?;let mut encoder=png::Encoder::new(file,image.size[0] as u32,image.size[1] as u32);encoder.set_color(png::ColorType::Rgba);encoder.set_depth(png::BitDepth::Eight);let mut writer=encoder.write_header()?;let bytes:Vec<u8>=image.pixels.iter().flat_map(|p|p.to_array()).collect();writer.write_image_data(&bytes)?;Ok(())})();
  eprintln!("App render capture {path}: {result:?}");ctx.send_viewport_cmd(egui::ViewportCommand::Close);return;
 }}
 let capture_frame=if std::env::var_os("ARCANEVR_EMBED_QA").is_some() || page().as_deref()==Some("updates"){180}else{20};
 if ctx.cumulative_frame_nr()==capture_frame{ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));}
 if ctx.cumulative_frame_nr()>capture_frame+180{eprintln!("App screenshot timed out");ctx.send_viewport_cmd(egui::ViewportCommand::Close);}
}
