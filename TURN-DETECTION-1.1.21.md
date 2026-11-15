# ArcaneVR 1.1.21: walking versus turning

Prepared 15 November 2026. Source changes only. Nothing was built for Windows, installed, launched, or tested on trackers or a headset.

## What was wrong

**1.1.20 stopped walking whenever the chest moved.** It treated any chest heading change faster than 18° per second as a turn, cleared the step rhythm, and held movement at zero for 0.45 s. Ordinary chest sway while stepping is faster than that. Replaying the 8 November walking recording through the 1.1.20 code holds movement at zero for the first 2.5 seconds of running on the spot, with the status "body turn quiet hold". In a game, with the head and shoulders moving, that hold would rarely release.

**1.1.19 and earlier counted turning steps as walking.** In the two turning recordings from 9 November, the old build reached 0.9 to 1.0 forward speed while the player only turned on the spot.

**The automated tests could not see either problem.** They modelled a turn as the chest and both feet rotating together as one rigid body, and modelled walking with a perfectly still chest. Real bodies do neither.

## What the recordings show

Measured per step, from the moment a foot starts moving to the moment it is still again:

| | Foot twist about vertical | Foot lift (tilt) | Chest heading change |
|---|---|---|---|
| Walking and running on the spot | under 9° | 7° to 18° | a few degrees of sway |
| Turning on the spot | 50° to 155° | 10° to 34° | 30° to 110°, same direction |

Three more things are visible in the turning recordings:

- The planted foot does not rotate at all. Turning is a staircase: one foot lifts, twists, lands; then the other.
- The chest leads. In one recording it had turned 45° before the first foot moved.
- A turning step starts like a walking step. The foot lifts first and twists a moment later, which is why the old filter let it through.

So foot twist per step separates the two cases with a wide margin, and the chest gives early warning.

## How 1.1.21 decides

1. **Every step waits for a verdict.** When a foot's activity starts a step, the step is held as pending. The pose the foot started from is remembered.
2. **Twist since the step began is the evidence.**
   - 45° or more: turning step.
   - 12° to 45°: turning step only if the twist is more than the "foot turning tolerance" setting (default 2) times the lift. A foot that really lifted while landing a little further round is walking a curve.
   - Under 12°: not a turn.
3. **The chest sets how careful the check is.**
   - Chest steady: a step with under 10° of twist counts after 0.06 s, the same delay as before.
   - Chest turning (8° in 0.4 s, or 3° in 0.1 s): the step counts only after 0.2 s with the foot's heading settled, or when the foot is still again.
   - Chest movement alone never pauses movement. Looking over a shoulder while marching keeps walking.
4. **A turning step pauses movement for 0.5 s** after the last sign of twisting, because the supporting foot rolls enough during a turn to look like a small step. Walking steps count again after that.
5. **Waiting does not distort speed.** A step is entered into the rhythm at the time it began, and a step still being checked is not read as the walker slowing down.
6. **A foot that swivels on the floor** without lifting is also treated as turning.

7. **Tighter curves walk slower.** Between 12° and 45° of twist a step counts for less and less, so speed falls smoothly as a curve tightens instead of switching off at a threshold. In the synthetic model: straight and 12° per second give full speed for the cadence, 25° per second gives about three quarters, and 40° per second or more gives none.
8. **Steps taken during a turn are still watched.** Marching out of a turn is recognised as walking once the feet stop twisting.

The existing "Small steps & turning" settings keep their meaning and saved values. Its help text now describes this behaviour.

## Single chest sensor

With one sensor, chest sway (the learned "lateral wobble" model) or chest bounce stands in for footsteps. Turning on the spot sways and bounces the chest as much as walking does: measured on the chest tracker in the same recordings, the wobble signal was about 0.08 while running on the spot and 0.11 to 0.25 while turning. So a single sensor walks you forward through every turn.

The chest's own heading separates them. Over half a second, walking sway swings back and nets under 9°. Turning keeps going the same way: typically 20° to 50°, up to 85°.

1.1.21 adds a chest turn fade for single-sensor step modes only:

- Net chest heading change over 0.5 s under 10°: steps count in full.
- 10° to 25°: steps fade smoothly, so a curve slows the walk.
- 25° or more: steps are off, and stay off for 0.4 s after, because a stepping turn pauses between steps.

It applies to both the learned wobble speed and the chest-acceleration speed, clears the sprint button while steps are off, and never disarms. Lean-only mode (your current setting, step input off) is not changed: there the lean alone sets speed.

On the recordings, the fade never engaged during walking and running on the spot. In the two turning recordings it had steps fully off for roughly 80 to 90% of the time spent turning and eased for most of the rest. Those wobble figures use a rough neutral pose, because the recordings were not made in single-sensor mode, so treat them as a comparison and not a calibration.

## A bug the replay exposed

In 1.1.20, walking slowly straight after a turn ran at nearly full speed (0.96 where the cadence calls for 0.25) in a synthetic turn-then-walk sequence. While turning, 1.1.20 reset the step detector on every sensor packet, and it miscounted afterwards. 1.1.21 keeps following the feet through a turn and gives 0.25.

## Results

Replay of the private recordings through the real engine, at the captured rate and resampled to 60, 100 and 200 Hz:

| Recording | Filter off | 1.1.20 | 1.1.21 |
|---|---|---|---|
| Walking and running on the spot (8 Nov), travel | 7.3 to 7.4 | 4.7 to 4.9 | 7.0 to 7.4 |
| Turning on the spot, both directions (9 Nov), travel | 5.1 | 0.0 | 0.0 |
| Turning on the spot, second capture (9 Nov), travel | 7.1 | 0.0 | 0.0 |

Travel is forward speed summed over time, so it has no real-world unit. The captures are polled snapshots at 18 to 29 Hz, not the raw tracker stream, so fast detail is missing from them.

Automated tests on Linux: 254 core library tests and 65 desktop tests pass (2 desktop tests are skipped on Linux). The new tests model walking with chest sway and foot wobble, a stepping turn at 60, 100 and 170° per second in both directions and both heading axes, walk then turn then walk, walking round curves of increasing tightness, a chest twist with straight feet, and a single chest sensor walking, turning and walking again.

## Not verified

- **No Windows build.** The Windows compiler target could not be downloaded here, so the Windows-only code paths were not compiled. The same source compiles and passes on Linux.
- **No renderer, receiver QA, or installer run.** `tests/turn_in_place_receiver_qa.py` was edited (walk-to-turn stop allowance raised from 0.16 s to 0.7 s) but not run.
- **No tracker, headset or in-game test.**
- **Walking round a curve has no recording**, with feet sensors or with one chest sensor. Its thresholds come from reasoning and a synthetic model. This is the behaviour most likely to need tuning.
- **No single-sensor recording exists.** The chest turn fade was checked against the chest tracker in three-sensor recordings.

## Known trade-offs

- Stopping from a full run into a turn takes about 0.4 to 0.5 s, because it waits for the foot to visibly twist. 1.1.20 stopped faster by stopping on any chest movement.
- If the feet start a turn before the chest moves, the first turning step is counted for roughly 0.15 s before the twist is seen. On the first step of a walk speed ramps slowly, so this is a brief nudge of under a tenth of full speed.
- After a turn, the first walking step within 0.5 s is ignored.

## Files changed

In `slime-locomotion/rust-core`:

- `src/foot_motion.rs`: per-step walk or turn verdict, curve scale, and the single-sensor chest turn fade; chest-rate hold removed.
- `src/cadence.rs`: pending steps with commit and reject.
- `src/gait.rs`: engine wiring; realistic tests replace the test that required a 2° chest movement to stop walking.
- `src/lab.rs`: three lines. The wobble speed is scaled by the chest turn fade, and the state report gains `chest_turn`.
- `src/desktop/foot_motion_panel.rs`: help text only.
- `src/bin/replay-steps.rs`: new offline tool. `replay-steps <recording.jsonl> [--rate 100] [--series out.csv] [--filter-off]`.
- `tests/turn_in_place_receiver_qa.py`: stop allowance.
- `Cargo.toml`, `Cargo.lock`: version 1.1.21.

At the source root: `Build-1.1.21.ps1` builds and tests without installing. It was written without a Windows machine to run it on.

## Review scope

Read in full: `foot_motion.rs`, `cadence.rs`, `gait.rs`, `mounting.rs`, `slime.rs`, and the packet intake, travel composition and settings paths in `lab.rs`. Skimmed: the updater (download is pinned to the release repository and checked against its SHA-256 before install). Not reviewed: the rest of `lab.rs`, `actions.rs`, `continuous.rs`, the desktop pages, the renderer, the SteamVR driver and overlay. No other defects were found in what was read.

## Next steps, in order

1. **Record walking round a curve and a walk-stop-turn-walk sequence**, 30 s each, unarmed, once with feet sensors and once in single-sensor mode. Replay them with `replay-steps`. These decide the 12° and 45° thresholds.
2. **Record the raw tracker stream**, not polled snapshots, so replay sees what the engine sees.
3. **Quicker stop into a turn.** If 0.4 s feels late, ease speed down while the chest has turned more than about 25° in 0.4 s and no walking step has been confirmed since. Needs recording 1 first, because it also slows curves.
4. **Guided turn learning.** The app already learns walking, feet, jump and crouch from short guided sessions. Ten seconds of turning on the spot and ten of walking would set the twist and chest thresholds per person instead of from one person's recordings.
5. **Chest and feet alignment.** Track the heading offset between the chest and each planted foot. A step taken while the chest is well ahead of that foot is very likely a turning step, which would remove the brief nudge described above.
6. **Stopping.** Movement currently continues for 0.55 to 1.1 s after the last step. Ending it when both feet are planted and still would feel tighter, and needs slow-walking recordings to avoid dropouts.
