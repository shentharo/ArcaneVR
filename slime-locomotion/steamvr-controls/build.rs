// Generate the function-table layout from the checked-in Valve C ABI header.
// Unused slots stay pointer-sized; called entries have explicit typed signatures.
use std::{env, fs, path::PathBuf};
fn main() {
    let header = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../openvr/headers/openvr_capi.h");
    println!("cargo:rerun-if-changed={}", header.display());
    let text = fs::read_to_string(&header).expect("Valve openvr_capi.h is required alongside this crate");
    let version = text.split("IVROverlay_Version = \"").nth(1).unwrap().split('"').next().unwrap();
    let table = text.split("struct VR_IVROverlay_FnTable").nth(1).unwrap().split("\n};").next().unwrap();
    let mut generated = format!("pub const OVERLAY_VERSION: &[u8] = b\"FnTable:{version}\\0\";\n#[repr(C)]\n#[allow(non_snake_case)]\npub struct OverlayTable {{\n");
    for line in table.lines().filter(|s|s.contains("OPENVR_FNTABLE_CALLTYPE *")) {
        let name=line.split("OPENVR_FNTABLE_CALLTYPE *").nth(1).unwrap().split(')').next().unwrap();
        let ty=match name {
            "FindOverlay"=>"unsafe extern \"system\" fn(*const std::ffi::c_char, *mut u64) -> i32",
            "CreateDashboardOverlay"=>"unsafe extern \"system\" fn(*const std::ffi::c_char, *const std::ffi::c_char, *mut u64, *mut u64) -> i32",
            "DestroyOverlay"=>"unsafe extern \"system\" fn(u64) -> i32",
            "SetOverlayWidthInMeters"=>"unsafe extern \"system\" fn(u64, f32) -> i32",
            "SetOverlayInputMethod"=>"unsafe extern \"system\" fn(u64, i32) -> i32",
            "SetOverlayMouseScale"=>"unsafe extern \"system\" fn(u64, *const [f32;2]) -> i32",
            "SetOverlayRaw"=>"unsafe extern \"system\" fn(u64, *mut std::ffi::c_void, u32, u32, u32) -> i32",
            "PollNextOverlayEvent"=>"unsafe extern \"system\" fn(u64, *mut VrEvent, u32) -> bool",
            "GetOverlayTextureSize"=>"unsafe extern \"system\" fn(u64, *mut u32, *mut u32) -> i32",
            "GetOverlayImageData"=>"unsafe extern \"system\" fn(u64, *mut std::ffi::c_void, u32, *mut u32, *mut u32) -> i32",
            "IsOverlayVisible"=>"unsafe extern \"system\" fn(u64) -> bool",
            _=>"usize",
        };
        generated.push_str(&format!("pub {name}: {ty},\n"));
    }
    generated.push_str("}\n");
    fs::write(PathBuf::from(env::var("OUT_DIR").unwrap()).join("overlay_abi.rs"),generated).unwrap();
}
