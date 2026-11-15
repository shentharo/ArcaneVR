# ArcaneVR 1.1.21 Windows validation — 16 November 2026

Built with Rust 1.95.0 on Windows using Build-1.1.21.ps1. This supersedes the Windows-not-built status in the earlier TURN-DETECTION-1.1.21.md report; its physical-test and tuning limitations remain.

- Renderer: 23 tests passed, 1 ignored.
- Core library: 256 tests passed.
- Desktop: 70 tests passed, 2 ignored.
- Installer: 3 tests passed, including checksums, rollback and restart paths.
- Actual receiver and embedded renderer: slow walking, running, gentle curve, clockwise/counterclockwise stationary full rotations, forward/backward/left/right displacement, stale stop, pause/resume/reset passed; game output remained disarmed.
- Run-to-turn stop measured approximately 0.501 seconds. Both stationary full rotations had zero measured renderer drift.
- Native embedding: parent/child handles, WS_CHILD, bounds, resize, hide and return passed. App-owned captures of the changed help panel and renderer were inspected.
- Recorded turning capture through the actual UDP receiver: 400 samples, zero nonzero-output samples. These are replayed recorded sensor poses, not a new physical test or a full-rate raw tracker capture.
- Windows replay-steps at 100 Hz: saved walking capture produced 7.3756 speed-seconds of travel; saved turning capture produced zero. The unit is integrated normalized speed, not measured metres.
- The live tracker configuration was unchanged by QA. Private recordings and detailed local QA outputs are not published.

Receiver QA corrections made during Windows validation: stationary full-turn fixtures now use alternating planted/lifted feet rather than keeping one repeatedly stepping foot at world yaw zero throughout a rotation. The four-direction fixture explicitly disables inherited forward-only mode and allows the GUI's automatic neutral recenter to settle before presenting lean poses. Zero-translation and signed-displacement assertions remain in place. The original mixed walk/pivot fixture failed before this correction; this is not evidence that every mixed movement pattern is suppressed.

Application Rust logic is unchanged from the staged 1.1.21 source. No new physical tracker/headset/in-game testing was performed. Curved walking, single-sensor tuning and the approximately half-second turning stop still require physical evaluation.

Verified executable SHA-256:
`B9EFEEDB2FD0707D2B791C4A2D159FF70B8F859DA4950A1B1C803D5FDE6F10EB`
