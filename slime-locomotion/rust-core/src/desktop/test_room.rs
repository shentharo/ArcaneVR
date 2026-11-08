//! Sensor-driven, isolated pixel-art movement room. Never arms VR output.
use eframe::egui::{self, Color32, Rect, TextureHandle, TextureOptions};
use serde_json::Value;
const W:usize=512; const H:usize=288;
#[derive(Default)]
pub struct TestRoom {
    running:bool, position:[f32;2], yaw:f32, distance:f32, phase:f32,
    camera_yaw:f32, last_snapshot:Option<f64>, same_snapshot_seconds:f32,
    texture:Option<TextureHandle>,
}
fn scalar(v:&Value)->f32{v.as_f64().filter(|n|n.is_finite()).unwrap_or(0.) as f32}
fn world_motion(travel:[f32;2],yaw:f32)->[f32;2]{let(s,c)=yaw.sin_cos();[travel[0]*c+travel[1]*s,-travel[0]*s+travel[1]*c]}
impl TestRoom {
    pub fn stop(&mut self){self.running=false;}
    fn advance(&mut self,travel:[f32;2],dt:f32){
        let v=world_motion(travel,self.yaw);let dt=dt.clamp(0.,0.05);
        self.position[0]+=v[0]*2.*dt;self.position[1]+=v[1]*2.*dt;
        let step=v[0].hypot(v[1])*2.*dt;self.distance+=step;self.phase+=step*15.;
    }
    pub fn show(&mut self,ui:&mut egui::Ui,state:&Value)->Option<Value>{
        let mut command=None;
        ui.horizontal(|ui|{ui.heading("Arcane Meadow");ui.label(egui::RichText::new("PIXEL MOVEMENT LAB").color(Color32::from_rgb(124,224,194)));});
        ui.label("An open field. Lean to walk, reverse or strafe. Turn your body: the character faces the chest sensor through a full 360°.");
        let dt=ui.input(|i|i.stable_dt).clamp(0.,0.05);
        let stamp=state["statistics"]["elapsed_seconds"].as_f64();
        if stamp!=self.last_snapshot{self.same_snapshot_seconds=0.;self.last_snapshot=stamp;}else{self.same_snapshot_seconds+=dt;}
        let fresh=state["locomotion"]["input_ready"]==true&&self.same_snapshot_seconds<0.5;
        let calibrated=if state["movement_engine"]=="body_tilt" || state["movement_engine"]=="body_tilt_steps" {state["actions"]["preview"]["calibrated"]==true && state["actions"]["preview"]["direction_status"]=="ready"}else{state["calibrated"]==true};let armed=state["armed"]==true;
        if fresh {self.yaw=scalar(&state["heading"]);}
        if !fresh||!calibrated||armed{self.stop();}
        let tilt=state["movement_engine"]=="body_tilt"||state["movement_engine"]=="body_tilt_steps";
        let preview=&state["actions"]["preview"];
        let speed=scalar(&state["locomotion"]["preview"]["speed"]).clamp(0.,1.);
        let mut travel=if tilt{[scalar(&preview["travel_xy"][0]),scalar(&preview["travel_xy"][1])]}else{[0.,speed]};
        if state["movement_engine"]=="body_tilt_steps"{travel=travel.map(|v|v*speed);}
        let slow=preview["speed_multiplier"].as_f64().unwrap_or(1.).clamp(0.,1.) as f32;travel=travel.map(|v|v*slow);
        ui.horizontal(|ui|{
            if ui.add_enabled(fresh&&calibrated&&!armed,egui::Button::new(if self.running{"Pause test"}else{"Start local test"})).clicked(){self.running=!self.running;}
            if ui.add_enabled(fresh&&!armed,egui::Button::new("Calibrate - stand upright")).clicked(){command=Some(serde_json::json!({"command":"calibrate"}));}
            if ui.button("Reset field").clicked(){self.position=[0.;2];self.distance=0.;self.phase=0.;self.camera_yaw=0.;}
            ui.label(format!("{}  •  {:.0}° facing  •  {:.0}% input  •  {:.1} m travelled",if self.running{"LIVE"}else{"PAUSED"},self.yaw.to_degrees().rem_euclid(360.),travel[0].hypot(travel[1])*100.,self.distance));
        });
        if armed{ui.colored_label(Color32::YELLOW,"Stop game output to use this isolated room.");}
        else if !fresh{ui.colored_label(Color32::YELLOW,"Waiting for live sensor data. Character movement paused.");}
        else if !calibrated{ui.colored_label(Color32::YELLOW,"Reset Center upright on Main before starting.");}
        if self.running {self.advance(travel,dt);}
        if !ui.ctx().wants_keyboard_input(){let turn=ui.input(|i|if i.key_down(egui::Key::Q){-1.}else if i.key_down(egui::Key::E){1.}else{0.});self.camera_yaw+=turn*dt;}
        let size=egui::vec2(ui.available_width().max(160.),ui.available_height().clamp(260.,720.));
        let(rect,response)=ui.allocate_exact_size(size,egui::Sense::click_and_drag());
        if response.dragged(){self.camera_yaw+=ui.input(|i|i.pointer.delta().x)*0.006;}
        ui.painter().rect_filled(rect,8.,Color32::from_rgb(12,24,33));
        let image=self.render(travel,self.running&&fresh);
        if let Some(texture)=&mut self.texture{texture.set(image,TextureOptions::NEAREST);}else{self.texture=Some(ui.ctx().load_texture("arcane-meadow",image,TextureOptions::NEAREST));}
        let ratio=W as f32/H as f32;let fit=egui::vec2(rect.width().min(rect.height()*ratio),rect.height().min(rect.width()/ratio));
        let image_rect=Rect::from_center_size(rect.center(),fit);
        ui.painter().image(self.texture.as_ref().unwrap().id(),image_rect,Rect::from_min_max(egui::pos2(0.,0.),egui::pos2(1.,1.)),Color32::WHITE);
        response.on_hover_text("Character follows your chest heading. Drag or Q/E orbits the camera only. No obstacles, walls or game output.");
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
        command
    }
    fn render(&self,travel:[f32;2],walking:bool)->egui::ColorImage{
        let mut c=Canvas::new();
        // Layered dusk sky, pixel clouds and distant mountains; none are collision objects.
        for y in 0..H {let t=(y as f32/140.).min(1.);let color=rgb((36.+t*92.) as u8,(53.+t*92.) as u8,(92.+t*49.) as u8);c.rect(0,y as i32,W as i32,1,color);}
        c.disc(390,54,23,20,rgb(255,219,147));c.rect(366,56,49,3,rgb(239,180,137));
        for (x,y) in [(45,35),(160,58),(290,24)] {let shift=(self.camera_yaw.sin()*12.) as i32;c.rect(x+shift,y,52,5,rgb(183,179,193));c.rect(x+10+shift,y-5,25,5,rgb(206,193,200));c.rect(x-9+shift,y+5,82,4,rgb(152,155,180));}
        for layer in 0..3 {for x in 0..W {let v=x as f32+(self.camera_yaw*20.);let peak=((v*0.025+layer as f32*2.).sin().abs()*24.+(v*0.057).cos()*9.) as i32;let y=104+layer*12-peak;let color=[rgb(78,98,128),rgb(62,100,116),rgb(41,88,91)][layer as usize];c.rect(x as i32,y,1,144-y,color);}}
        c.rect(0,130,512,6,rgb(120,167,153));c.rect(0,136,512,6,rgb(92,143,131));
        let (sn,cs)=self.camera_yaw.sin_cos();
        for y in 142..H {let depth=390./(y as f32-128.);for x in 0..W{
            let dx=(x as f32-256.)*depth/180.;let dz=depth-4.;
            let wx=self.position[0]+dx*cs+dz*sn;let wz=self.position[1]-dx*sn+dz*cs;
            let ix=(wx*5.).floor() as i32;let iz=(wz*5.).floor() as i32;
            let n=hash(ix,iz);let checker=((wx.floor() as i32).wrapping_add(wz.floor() as i32))&1;
            let mut col=if checker==0{rgb(60,111,91)}else{rgb(65,118,94)};
            if n%13==0{col=rgb(79,135,104);}else if n%19==0{col=rgb(48,98,82);}else if n%97==0{col=rgb(171,166,105);}
            let haze=((y-142) as f32/75.).min(1.);let a=col.to_array();col=rgb((85.+(a[0] as f32-85.)*haze) as u8,(139.+(a[1] as f32-139.)*haze) as u8,(122.+(a[2] as f32-122.)*haze) as u8);
            c.pixel(x as i32,y as i32,col);
        }}
        let facing=self.yaw-self.camera_yaw;let (s,co)=facing.sin_cos();
        // A compass footprint keeps full sensor rotation visible even between sprite directions.
        c.disc(256,232,25,10,rgb(49,87,79));
        for i in 0..32{let a=i as f32*std::f32::consts::TAU/32.;c.rect(256+(a.sin()*25.) as i32,232-(a.cos()*10.) as i32,2,1,rgb(117,167,137));}
        let tip=(256+(s*35.) as i32,232-(co*15.) as i32);c.line((256,232),tip,rgb(255,218,136));c.disc(tip.0,tip.1,2,2,rgb(255,235,174));
        let gait=if walking&&travel[0].hypot(travel[1])>0.015{self.phase.sin()}else{0.};
        avatar(&mut c,256,230,facing,gait);
        // Subtle decorative screen frame.
        c.rect(0,0,512,2,rgb(179,204,174));c.rect(0,286,512,2,rgb(26,66,63));
        egui::ColorImage::new([W,H],c.pixels)
    }
}
fn rgb(r:u8,g:u8,b:u8)->Color32{Color32::from_rgb(r,g,b)}
fn hash(x:i32,z:i32)->u32{let mut n=(x as u32).wrapping_mul(374761393).wrapping_add((z as u32).wrapping_mul(668265263));n=(n^(n>>13)).wrapping_mul(1274126177);n^(n>>16)}
struct Canvas{pixels:Vec<Color32>}
impl Canvas{
 fn new()->Self{Self{pixels:vec![Color32::BLACK;W*H]}}
 fn pixel(&mut self,x:i32,y:i32,color:Color32){if x>=0&&y>=0&&(x as usize)<W&&(y as usize)<H{self.pixels[y as usize*W+x as usize]=color;}}
 fn rect(&mut self,x:i32,y:i32,w:i32,h:i32,color:Color32){for yy in y.max(0)..(y+h).min(H as i32){for xx in x.max(0)..(x+w).min(W as i32){self.pixel(xx,yy,color);}}}
 fn disc(&mut self,x:i32,y:i32,rx:i32,ry:i32,color:Color32){for dy in -ry..=ry{for dx in -rx..=rx{if (dx*dx) as f32/(rx*rx) as f32+(dy*dy) as f32/(ry*ry) as f32<=1.{self.pixel(x+dx,y+dy,color);}}}}
 fn line(&mut self,a:(i32,i32),b:(i32,i32),color:Color32){let n=(a.0-b.0).abs().max((a.1-b.1).abs()).max(1);for i in 0..=n{self.pixel(a.0+(b.0-a.0)*i/n,a.1+(b.1-a.1)*i/n,color);}}
}
fn avatar(c:&mut Canvas,x:i32,y:i32,yaw:f32,gait:f32){
    let front=yaw.cos()< -0.25;let side=yaw.sin();let offset=(side*4.) as i32;let bob=(gait.abs()*2.) as i32;
    let ink=rgb(23,39,53);let teal=rgb(48,146,153);let light=rgb(102,201,185);let skin=rgb(239,190,143);let hair=rgb(65,50,62);
    c.disc(x,y+1,16,4,rgb(30,64,65));
    for (i,sign) in [-1,1].iter().enumerate(){let step=(gait*3.* *sign as f32) as i32;let footx=x-7+i as i32*9;c.rect(footx,y-15,7,13+step,ink);c.rect(footx+1,y-14,5,8+step,rgb(57,72,101));c.rect(footx-1,y-3+step,9,4,ink);c.rect(footx,y-3+step,7,1,rgb(155,125,97));}
    let top=y-42-bob;
    // Tunic, shoulder mantle, gold clasp and belt.
    c.rect(x-12,top+15,24,23,ink);c.rect(x-10,top+16,20,20,teal);c.rect(x-9,top+18,5,14,light);
    c.rect(x-11,top+32,22,4,rgb(95,65,58));c.rect(x-2,top+32,4,3,rgb(241,202,112));
    if !front{c.rect(x-7,top+17,15,16,rgb(35,91,121));c.rect(x-6,top+18,2,13,rgb(66,126,151));}
    for sign in [-1,1]{let swing=(gait*2.*sign as f32) as i32;c.rect(x+sign*13-3,top+18+swing,6,14,ink);c.rect(x+sign*13-2,top+19+swing,4,10,teal);c.rect(x+sign*13-2,top+29+swing,4,4,skin);}
    // 64-pixel-style head with directional face/hair, ear and rim light.
    c.rect(x-10+offset,top,20,19,ink);c.rect(x-8+offset,top+2,16,15,skin);c.rect(x-9+offset,top+1,18,6,hair);c.rect(x-7+offset,top,14,2,rgb(113,76,71));
    if front{c.rect(x-5+offset,top+9,3,3,ink);c.rect(x+3+offset,top+9,3,3,ink);c.rect(x-3+offset,top+14,6,1,rgb(158,99,86));c.pixel(x-4+offset,top+9,rgb(232,245,224));}
    else if yaw.cos()>0.25{c.rect(x-8+offset,top+5,16,11,hair);c.rect(x-7+offset,top+6,3,8,rgb(103,66,68));}
    else{let eye=x+if side>0.{6}else{-7};c.rect(eye+offset,top+9,2,3,ink);c.rect(x+if side>0.{9}else{-11}+offset,top+10,3,4,skin);}
    c.rect(x-11,top+17,22,3,light);c.rect(x-2,top+18,4,4,rgb(245,211,120));
}
#[cfg(test)]
mod tests{
 use super::*;
 #[test]fn sensor_heading_rotates_forward_and_preserves_strafe(){let v=world_motion([0.,1.],std::f32::consts::FRAC_PI_2);assert!((v[0]-1.).abs()<1e-5);assert!(v[1].abs()<1e-5);let v=world_motion([1.,0.],std::f32::consts::PI);assert!((v[0]+1.).abs()<1e-5);}
 #[test]fn open_field_allows_reverse_and_has_no_collision_limits(){let mut r=TestRoom::default();for _ in 0..1000{r.advance([0.,-1.],0.05);}assert!(r.position[1]< -90.);r.yaw=std::f32::consts::TAU;r.advance([0.,1.],0.05);assert!(r.position[0].abs()<1e-4);}
 #[test]fn rotation_in_place_does_not_translate(){let mut r=TestRoom::default();for i in 0..360{r.yaw=(i as f32).to_radians();r.advance([0.,0.],0.02);}assert_eq!(r.position,[0.;2]);assert_eq!(r.distance,0.);}
 #[test]fn pixel_scene_is_bounded_at_large_positions(){let r=TestRoom{position:[100000.,-100000.],..Default::default()};assert_eq!(r.render([0.,0.],false).pixels.len(),W*H);}
}
