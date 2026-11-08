"""CLI QA of the actual ArcaneVR receiver and app-owned update dialog capture."""
from pathlib import Path
import argparse
import hashlib
import http.client
import json
import subprocess
import time
import urllib.request


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    exe, output = args.exe.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    settings = [exe.parent / name for name in ("tracker-group.json", "sensor-network-plan.json")]
    def hashes():
        return {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in settings if p.exists()}
    before = hashes()
    screenshot, report = output / "updates.png", output / "update-check.json"
    for path in (screenshot, report):
        if path.exists():
            path.rename(path.with_name(f"{path.stem}-previous-{time.time_ns()}{path.suffix}"))
    command = [str(exe), "--config", str(output / "isolated-tracker-group.json"), "--http-port", "18861",
               "--tracker-port", "19661", "--tracker-bind", "127.0.0.1", "--nexus-port", "19561",
               "--steam-port", "19761", "--qa-page", "updates", "--qa-screenshot", str(screenshot),
               "--qa-update-report", str(report)]
    base = "http://127.0.0.1:18861"
    with (output / "runtime.log").open("w") as log:
        app = subprocess.Popen(command, cwd=exe.parent, stdout=log, stderr=log,
                               creationflags=subprocess.CREATE_NO_WINDOW)
        try:
            for _ in range(100):
                try:
                    state = json.load(urllib.request.urlopen(base + "/api/state", timeout=1))
                    break
                except OSError:
                    time.sleep(.1)
            else:
                raise AssertionError("Receiver did not start")
            assert state["armed"] is False
            request = urllib.request.Request(base + "/api/command", data=b'{"command":"stop"}',
                                             headers={"Content-Type": "application/json"})
            assert json.load(urllib.request.urlopen(request, timeout=3)).get("ok")
            connection = http.client.HTTPConnection("127.0.0.1", 18861, timeout=3)
            connection.request("GET", "/api/state", headers={"Host": "example.invalid"})
            assert connection.getresponse().status == 403
            connection.close()
            assert json.load(urllib.request.urlopen(base + "/api/state", timeout=1))["armed"] is False
            assert app.wait(timeout=50) == 0
        finally:
            if app.poll() is None:
                app.terminate()
                app.wait(timeout=10)
    check = json.loads(report.read_text())
    assert check["state"] in ("no_newer_release", "available"), check
    assert screenshot.is_file()
    assert before == hashes(), "User sensor settings changed"
    result = {"receiver_contract": "passed: state, Stop, Host rejection; output remained disarmed",
              "live_github_check": check, "configuration_unchanged": True,
              "installed_sha256": hashlib.sha256(exe.read_bytes()).hexdigest(),
              "capture": str(screenshot), "physical_hardware": "not tested"}
    (output / "verification.json").write_text(json.dumps(result, indent=2))
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
