# ArcaneVR

I was frustrated that the VR movement options I tried did not match what I wanted, so I decided to create my own. My aim is to make movement feel natural and give people control over how it works.

I started with **one SlimeVR-compatible chest sensor** to get the foundations working. One-sensor movement is working really well for me so far. Three sensors are a straightforward setup to build toward, and I will continue working on support for foot sensors. The app also has layouts for three and five sensors, but those modes are still under development and need more real-world testing.

## Features so far

- **One-sensor locomotion:** lean to move, move backwards or strafe, with adjustable direction and sensitivity.
- **Walking in place:** learn the left/right chest sway of walking with one sensor, adjust the effort needed to reach full walking speed, and save the learned pattern.
- **Sprint controls:** enable sprint and tune the lean threshold; faster walking effort can also trigger sprint in one-sensor mode.
- **Movement tuning:** adjust the deadzone, full-speed and sprint rings, response and smoothing.
- **Test room:** preview movement and see live sensor and direction information before enabling game output.
- **Calibration:** guided chest sensor mounting calibration and a recenter control; jump and crouch learning and tuning are also available.
- **Sensor setup:** assign sensors to body positions, see connection status and use the USB Wi-Fi setup for compatible SlimeVR trackers.
- **Profiles:** save, load, import and export movement settings locally, including game-specific profiles. Community profiles are not available yet.
- **SteamVR integration:** a virtual locomotion device, controller and headset status, an optional dashboard overlay, and a button to return to SteamVR Home. Game compatibility and bindings vary.
- **Boundary setup:** opens SteamVR Room Setup. This does not automatically move the Quest safety boundary; adjust the Quest boundary in the headset when required.
- **Three- and five-sensor layouts:** selectable in the app; foot-based walking and additional sensor support remain a work in progress.
- **Updates:** check for published GitHub releases from the app. See the update limitations below.

This is an experimental project. My experience with one sensor is encouraging, but hardware, tracking and game behavior vary. Please test calibration, direction and stopping with your setup.

## Development

I expect updates to be frequent while the project is developing—possibly daily or weekly—and less frequent, perhaps monthly, as it matures. These are intentions, not a fixed release schedule. Feedback and reports are welcome.

## License

ArcaneVR project code is available free for noncommercial use under the [PolyForm Noncommercial License 1.0.0](LICENSE). Third-party components and assets retain their own licenses, including the Valve OpenVR headers and notices in `slime-locomotion/bevy-test-room/assets/`. Noncommercial source availability is not an OSI open-source license. Previously distributed copies retain their original license.

## Build

On Windows with Rust 1.95 or newer and the required Windows C++ build tools: `cargo build --release --manifest-path slime-locomotion/rust-core/Cargo.toml --bin vrcraft`. The receiver requires SteamVR and compatible sensors for real operation. SteamVR, headset software and drivers are separate dependencies.

No hardware or game compatibility is guaranteed. The release has automated coverage, but physical headset and game behavior still require testing on your setup.

## Support

If you would like to support development, you can [donate via PayPal](https://paypal.me/shentharolikesmoney). Donations are optional.

## Updates

In ArcaneVR choose **Updates → Check for updates**. The app checks this repository's published GitHub Releases, downloads a newer `ArcaneVR.exe`, verifies GitHub's SHA-256 digest, and asks you to install and restart. Source commits alone do not trigger an update.

For a new installation, download and extract the complete Windows ZIP from [Releases](https://github.com/shentharo/ArcaneVR/releases); the standalone executable does not include the driver or test-room files. Maintainers can use the **Prepare executable update** workflow to build and test a draft release after increasing the version in `slime-locomotion/rust-core/Cargo.toml`. Review the draft and publish it only after verifying that the update works on an installed copy. The current updater replaces the main executable only: do not publish an executable-only update if the driver, test-room renderer, assets, or other files must also change. Such updates need a separately verified complete package.
