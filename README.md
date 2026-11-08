# ArcaneVR / VRCraft

Windows VR locomotion software using SlimeVR-compatible sensors and SteamVR. Experimental: hardware and game compatibility vary; verify calibration and stopping before use.

## License

ArcaneVR project code is available free for noncommercial use under the [PolyForm Noncommercial License 1.0.0](LICENSE). Third-party components and assets retain their own licenses, including the Valve OpenVR headers and notices in `slime-locomotion/bevy-test-room/assets/`. Noncommercial source availability is not an OSI open-source license. Previously distributed copies retain their original license.

## Build

On Windows with Rust 1.95 or newer and the required Windows C++ build tools: `cargo build --release --manifest-path slime-locomotion/rust-core/Cargo.toml --bin vrcraft`. The receiver requires SteamVR and compatible sensors for real operation. SteamVR, headset software and drivers are separate dependencies.

No hardware or game compatibility is guaranteed. The release build, installation packaging and physical headset behavior need independent verification before distributing binaries.

## Support

If you would like to support development, you can [donate via PayPal](https://paypal.me/shentharolikesmoney). Donations are optional.

## Updates

In ArcaneVR choose **Updates → Check for updates**. The app checks this repository's published GitHub Releases, downloads a newer `ArcaneVR.exe`, verifies GitHub's SHA-256 digest, and asks you to install and restart. Source commits alone do not trigger an update.

There is no published executable release yet. Maintainers can use the **Prepare executable update** workflow to build and test a draft release after increasing the version in `slime-locomotion/rust-core/Cargo.toml`. Review the draft and publish it only after verifying that the update works on an installed copy. The current updater replaces the main executable only: do not publish an executable-only update if the driver, test-room renderer, assets, or other files must also change. Such updates need a separately verified complete package.
