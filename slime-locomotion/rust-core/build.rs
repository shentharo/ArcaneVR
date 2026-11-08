fn main() {
    let header=std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../openvr/headers/openvr_capi.h");
    println!("cargo:rerun-if-changed={}",header.display());
    let text=std::fs::read_to_string(header).expect("OpenVR C header");
    let version=text.split("IVRSystem_Version = \"").nth(1).unwrap().split('"').next().unwrap();
    let table=text.split("struct VR_IVRSystem_FnTable").nth(1).unwrap().split("\n};").next().unwrap();
    let mut out=format!("const VERSION: &[u8]=b\"FnTable:{version}\\0\";\n#[repr(C)] #[allow(non_snake_case)] struct System {{\n");
    for line in table.lines().filter(|s|s.contains("OPENVR_FNTABLE_CALLTYPE *")) {
        let name=line.split("OPENVR_FNTABLE_CALLTYPE *").nth(1).unwrap().split(')').next().unwrap();
        let ty=match name {"GetInt32TrackedDeviceProperty"=>"unsafe extern \"system\" fn(u32,i32,*mut i32)->i32", "GetStringTrackedDeviceProperty"=>"unsafe extern \"system\" fn(u32,i32,*mut std::ffi::c_char,u32,*mut i32)->u32","GetTrackedDeviceIndexForControllerRole"=>"unsafe extern \"system\" fn(i32)->u32", "GetTrackedDeviceClass"|"GetControllerRoleForTrackedDeviceIndex"=>"unsafe extern \"system\" fn(u32)->i32", "IsTrackedDeviceConnected"=>"unsafe extern \"system\" fn(u32)->bool",_=>"usize"};
        out.push_str(&format!("{name}: {ty},\n"));
    }
    out.push_str("}\n");std::fs::write(std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("system_abi.rs"),out).unwrap();

    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=assets/arcanevr-app-icon.ico");
        winresource::WindowsResource::new()
            .set_icon("assets/arcanevr-app-icon.ico")
            .compile()
            .expect("compile ArcaneVR Windows icon");
    }
}
