# ArcaneVR working rules

- Verify changes before saying they work. Build the actual release artifacts, test the real receiver contract, and compare installed/build hashes.
- Before reporting a task complete, reread the user's instructions and compare each requested detail with the actual result. Inspect the rendered app for UI work and verify what each control does; fix omissions and misleading behaviour, or explicitly report anything that remains incomplete.
- After making an application change, install the verified build and launch the updated ArcaneVR app so the user can see it. Confirm the launched app and receiver are running; report any blocker instead of implying it launched.
- For movement changes, exercise the actual renderer and inspect its reported translation, forward/backward/strafe directions, stationary full rotation, stale-input stop, and pause/reset controls. Animation labels alone do not prove movement.
- For embedded UI changes, verify actual native parent/child handles, bounds, resizing, hiding on navigation, and returning to the page. Review app-owned render captures.
- Report automated results separately from physical sensor/headset results. Never claim a hardware test passed when trackers are offline or the Quest is disconnected.
- Preserve sensor assignments, learned mounting directions, and the existing firewall-approved executable path. Do not arm game output during desktop testing.
- Keep testing inside ArcaneVR. Respect the user's preference for command-line work without Computer Use.
- Use the user's supplied reference code and explain any unavailable source/assets accurately; do not describe a replacement as an exact restored project.
