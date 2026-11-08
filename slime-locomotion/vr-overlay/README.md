# ArcaneVR original OpenVR overlays

Build with `build.cmd` using the existing local Visual Studio C++ toolchain and OpenVR SDK. Output: `bin/vrcraft-vr-overlay.exe` plus `bin/openvr_api.dll`. No installation or downloads.

## Integration contract

- `--dashboard --parent-pid PID`: SteamVR dashboard tile with large Reset Center and STOP buttons. Open SteamVR dashboard and select ArcaneVR.
- `--self-test`: static argument and command allowlist tests; does not initialize VR, send network requests.
- Live modes emit flushed JSON lines: `{ "type": "status" | "error", "message": "..." }`. Startup/argument errors exit 2. Status polling occurs once per second. A successful HTTP response means backend reachable, not movement armed or tracking calibrated.
- Keep stdin piped and open. Newline commands: `reset-center`, `stop`, `quit`. Pipe closure exits. `--parent-pid` additionally watches the owning app and exits if it closes.
- Pass `--http-port PORT` on every launch to target the owning app's API, especially isolated test instances. The default is 8811; values must be 1â€“65535. The hostname is always numeric loopback.
- After an image loads, a `type: "texture"` JSON line reports actual `width`, `height`, numeric `size_error` and `image_error`, and `readback_verified` every two seconds. Verification requires successful bounded `GetOverlayImageData`, matching dimensions and nonzero RGB pixels. Submission success alone never means the image loaded.

Reset Center posts `stop`, waits three seconds, then posts `calibrate`. Stop cancels countdown. Reset is cancelled if stopping fails. Calibration does not arm movement. API hostname is compiled to numeric loopback `127.0.0.1`, with proxy bypass, redirect disabled, bounded timeouts, fixed `/api/state` and `/api/command` routes, and a fixed stop/calibrate/arm allowlist. Quit/pipe EOF/parent exit cancel an outstanding countdown without calibrating.

## Texture uploads



Raw texture uploads are asynchronous. Main and thumbnail buffers stay alive through overlay destruction and VR shutdown. The main surface is never rendered, resized or resubmitted while its image is in flight; `ImageLoaded` or `ImageFailed` releases that gate. Hidden dashboard panels retain their loaded image. Failed submissions/load events retry after 250 ms with backoff capped at two seconds, stopping after five consecutive failures. A missing completion event closes the overlay after five seconds without reusing the pending buffer. Thus 15/30 FPS are upper bounds, and SteamVR completion latency may reduce the frame rate.

## Verification boundary

Build and `--self-test` are suitable without a headset. SteamVR controller clicks, dashboard visibility, and VR comfort require a user-run headset check.

Dashboard updates are submitted only when status, countdown, or click acknowledgement changes. Stable panels retain their existing texture instead of re-uploading it continuously. Mouse presses use each cursor’s last move position.

Eye-level controls: use Place controls in front of me in the dashboard. This creates a separate upright panel 1.5 m ahead at current tracked eye height. It uses yaw only, rejects invalid/near-vertical poses, appears only while SteamVR dashboard is open, and does not send calibration or movement commands. The SteamVR-managed dashboard tab retains its own placement. Physical placement and controller interaction need headset verification.

Recovery update: eye-level controls remain visible outside the dashboard. ArcaneVR starts the controls helper and retries after exit every five seconds until the controls are dismissed or ArcaneVR closes. Valid tracking is required to place/recover the panel. A stopped SteamVR runtime cannot render overlays. Calibration failure now reports the missing tracker role. Sensor input remains direct SlimeVR UDP6969 in live mode.
