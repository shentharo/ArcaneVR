"""Real PowerShell installer contract; fixtures never touch an installed application."""
from pathlib import Path
import hashlib
import json
import os
import subprocess
import tempfile
import time
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "src/desktop/update_install.ps1"


@unittest.skipUnless(os.name == "nt", "Windows installer")
class InstallerTests(unittest.TestCase):
    def test_restart_preserves_relative_config_from_original_working_directory(self):
        with tempfile.TemporaryDirectory(prefix="arcane-restart-test-") as temporary:
            root = Path(temporary)
            install = root / "installed app"
            original_working_directory = root / "user settings"
            stage = install / ".arcane-update" / "test"
            stage.mkdir(parents=True)
            original_working_directory.mkdir()
            config_name = "custom trackers.json"
            expected_config = '{"mount":"learned","trackers":"keep"}'
            (original_working_directory / config_name).write_text(expected_config)
            (install / config_name).write_text('{"mount":"wrong configuration"}')
            observed = root / "observed.txt"
            source = root / "RestartProbe.cs"
            source.write_text('''using System;
using System.IO;
class RestartProbe {
    static void Main(string[] args) {
        File.WriteAllLines(args[2], new[] {
            Environment.CurrentDirectory, args[0], args[1], File.ReadAllText(args[1])
        });
    }
}
''')
            compiler = Path(os.environ["WINDIR"]) / "Microsoft.NET/Framework64/v4.0.30319/csc.exe"
            candidate = stage / "candidate.exe"
            subprocess.run([str(compiler), "/nologo", "/target:winexe",
                            f"/out:{candidate}", str(source)], check=True,
                           capture_output=True, text=True, timeout=30)
            target = install / "VRCraft.exe"
            old = b"MZ" + b"old" * 50
            target.write_bytes(old)
            request = {"target": str(target), "sha256": hashlib.sha256(candidate.read_bytes()).hexdigest(),
                       "original_sha256": hashlib.sha256(old).hexdigest(),
                       "parent_pid": 0, "parent_start_ticks": 0,
                       "restart_working_directory": str(original_working_directory),
                       "restart_args": ["--config", config_name, str(observed)]}
            request_path = stage / "request.json"
            request_path.write_text(json.dumps(request))
            env = {k: v for k, v in os.environ.items() if k.lower() != "psmodulepath"}
            completed = subprocess.run(["powershell.exe", "-NoProfile", "-NonInteractive",
                                        "-ExecutionPolicy", "Bypass", "-File", str(SCRIPT),
                                        "-Request", str(request_path)], env=env,
                                       capture_output=True, text=True, timeout=15)
            self.assertEqual(completed.returncode, 0, completed.stderr)
            deadline = time.monotonic() + 5
            while not observed.exists() and time.monotonic() < deadline:
                time.sleep(0.05)
            self.assertTrue(observed.exists(), "Updated executable did not produce its startup report")
            lines = observed.read_text(encoding="utf-8-sig").splitlines()
            self.assertEqual(Path(lines[0]), original_working_directory)
            self.assertEqual(lines[1:], ["--config", config_name, expected_config])
            self.assertEqual((original_working_directory / config_name).read_text(), expected_config)

    def test_missing_original_working_directory_installs_without_restarting(self):
        with tempfile.TemporaryDirectory(prefix="arcane-missing-cwd-test-") as temporary:
            root = Path(temporary)
            install = root / "installed app"
            stage = install / ".arcane-update" / "test"
            stage.mkdir(parents=True)
            old = b"MZ" + b"old" * 50
            new = b"MZ" + b"new" * 50
            target = install / "VRCraft.exe"
            target.write_bytes(old)
            (stage / "candidate.exe").write_bytes(new)
            request = {"target": str(target), "sha256": hashlib.sha256(new).hexdigest(),
                       "original_sha256": hashlib.sha256(old).hexdigest(),
                       "parent_pid": 0, "parent_start_ticks": 0,
                       "restart_working_directory": str(root / "deleted folder"),
                       "restart_args": ["--config", "relative.json"]}
            request_path = stage / "request.json"
            request_path.write_text(json.dumps(request))
            env = {k: v for k, v in os.environ.items() if k.lower() != "psmodulepath"}
            completed = subprocess.run(["powershell.exe", "-NoProfile", "-NonInteractive",
                                        "-ExecutionPolicy", "Bypass", "-File", str(SCRIPT),
                                        "-Request", str(request_path)], env=env,
                                       capture_output=True, text=True, timeout=15)
            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertEqual(target.read_bytes(), new, "update was not installed")
            result = json.loads((stage / "result.json").read_text(encoding="utf-8-sig"))
            self.assertEqual(result["status"], "installed")
            self.assertIn("not restarted", result["note"])

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
