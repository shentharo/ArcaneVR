# ArcaneVR SteamVR dashboard controls

Native Windows x64 Rust companion; creates the `ArcaneVR` SteamVR dashboard tab under overlay key `dev.vrcraft.controls`. CPU RGBA rendering is submitted through `SetOverlayRaw`; controller pointer events come from `PollNextOverlayEvent`. No Python, browser, scene app, or GPU graphics context.

Build: `cargo +1.95.0 build --release`. Run `target/release/vrcraft-steamvr-controls.exe` from the desktop app using a hidden child process. Place Valve's `openvr_api.dll` alongside it when packaging. Development also finds the checked-in `../openvr/bin/win64/openvr_api.dll`; the standard SteamVR install path is a final fallback. The Valve header in `../openvr/headers/openvr_capi.h` is required at build time and generates the exact `IVROverlay_028` function-table layout.

Arguments: `--http-port 8811`, `--openvr-dll PATH`, `--report PATH` (runtime JSON), `--probe --report PATH` (read-only overlay lookup), `--run-seconds N` (bounded runtime check). Duplicate instances exit without creating another tab. Error details go to `%TEMP%/vrcraft-steamvr-controls-error.txt`.

API: GET `/api/state`; POST `/api/command` with `{ "command": "stop" }`, `calibrate`, or `reset_test`. Calibrate Forward first stops, counts down three seconds without blocking the UI, then calibrates. Reset Test first stops then resets. STOP cancels a pending countdown. There is no arm action and no automatic calibration or reset on startup. Network work uses a worker thread with bounded timeouts. Status shows receiver connectivity, calibration/armed state, and per-foot preview counts.

Raw uploads are asynchronous: only one image is allowed in flight until `VREvent_ImageLoaded` / `VREvent_ImageFailed`. Hidden panels keep their last image instead of queuing uploads. Transient upload failures retry with capped backoff without destroying the panel. An RAII lease destroys the main overlay (and its dashboard thumbnail) when the owning process exits normally or unwinds an error.

The owner report includes a Unix-millisecond heartbeat, uptime and lifecycle, plus actual `GetOverlayTextureSize` and `GetOverlayImageData` results. Treat a report as live only while its PID exists and its heartbeat advances. `owner_image_verified` requires successful readback at the expected 768x512 size with nonzero pixels; submission success alone is not texture verification. A separate `--probe` checks registration, but cross-process texture queries can fail until the image has loaded. Controller interaction must be checked by the user in the dashboard; development verification does not inject clicks or execute calibration/reset commands.

Primary ABI reference: [Valve OpenVR](https://github.com/ValveSoftware/openvr/blob/master/headers/openvr_capi.h). Event/input pattern: [Valve overlay sample](https://github.com/ValveSoftware/openvr/blob/master/samples/helloworldoverlay/openvroverlaycontroller.cpp).
