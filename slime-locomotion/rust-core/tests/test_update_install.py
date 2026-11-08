"""Real PowerShell installer contract; fixtures never touch an installed application."""
from pathlib import Path
import hashlib
import json
import os
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "src/desktop/update_install.ps1"


@unittest.skipUnless(os.name == "nt", "Windows installer")
class InstallerTests(unittest.TestCase):
    def test_install_checksums_preservation_and_rollback(self):
        with tempfile.TemporaryDirectory(prefix="arcane-updater-test-") as temporary:
            root = Path(temporary)
            for case in ("success", "bad-hash", "changed-installed", "restart-failure"):
                with self.subTest(case=case):
                    directory = root / case
                    stage = directory / ".arcane-update" / "test"
                    stage.mkdir(parents=True)
                    old, new = b"MZ" + b"old" * 50, b"MZ" + b"new" * 50
                    target = directory / "VRCraft.exe"
                    target.write_bytes(old)
                    (stage / "candidate.exe").write_bytes(new)
                    settings = directory / "tracker-group.json"
                    settings.write_text('{"trackers":"unchanged","mount":"keep"}')
                    request = {"target": str(target), "sha256": hashlib.sha256(new).hexdigest(),
                               "original_sha256": hashlib.sha256(old).hexdigest(),
                               "parent_pid": 0, "parent_start_ticks": 0, "restart_args": []}
                    if case == "bad-hash":
                        request["sha256"] = "0" * 64
                    if case == "changed-installed":
                        request["original_sha256"] = "0" * 64
                    path = stage / "request.json"
                    path.write_text(json.dumps(request))
                    command = ["powershell.exe", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
                               "-File", str(SCRIPT), "-Request", str(path)]
                    if case != "restart-failure":
                        command.append("-NoRestart")
                    env = {k: v for k, v in os.environ.items() if k.lower() != "psmodulepath"}
                    completed = subprocess.run(command, env=env, capture_output=True, text=True, timeout=15)
                    result = json.loads((stage / "result.json").read_text(encoding="utf-8-sig"))
                    self.assertEqual(completed.returncode, 0 if case == "success" else 1, result)
                    self.assertEqual(target.read_bytes(), new if case == "success" else old)
                    self.assertEqual(settings.read_text(), '{"trackers":"unchanged","mount":"keep"}')
                    if case == "success":
                        self.assertEqual((stage / "previous.exe").read_bytes(), old)


if __name__ == "__main__":
    unittest.main(verbosity=2)
