"""Verify actual native Test Room embedding after application header changes."""
from pathlib import Path
import argparse
import json
import os
import subprocess


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--exe", type=Path, required=True)
    ap.add_argument("--output", type=Path, required=True)
    args = ap.parse_args()
    exe, output = args.exe.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = output / "embedding.json"
    # Use a fresh per-run report to avoid confusing old transitions with this run.
    history = report.with_suffix(".jsonl")
    if history.exists():
        history.rename(output / f"embedding-{history.stat().st_mtime_ns}.jsonl")
    env = dict(os.environ, ARCANEVR_EMBED_QA="1", ARCANEVR_EMBED_QA_NAVIGATE="1",
               ARCANEVR_EMBED_QA_REPORT=str(report),
               ARCANEVR_VIEWER_QA_SCREENSHOT=str(output / "viewer.png"),
               ARCANEVR_VIEWER_QA_REPORT=str(output / "viewer-report.json"))
    command = [str(exe), "--config", str(output / "isolated-tracker-group.json"), "--http-port", "18862",
               "--tracker-port", "19662", "--tracker-bind", "127.0.0.1", "--nexus-port", "19562",
               "--steam-port", "19762", "--qa-page", "test", "--qa-screenshot", str(output / "embedded-app.png")]
    with (output / "embedding.log").open("w") as log:
        app = subprocess.Popen(command, cwd=exe.parent, env=env, stdout=log, stderr=log,
                               creationflags=subprocess.CREATE_NO_WINDOW)
        try:
            assert app.wait(timeout=50) == 0
        finally:
            if app.poll() is None:
                app.terminate()
                app.wait(timeout=10)
    records = [json.loads(line) for line in history.read_text().splitlines()]
    positioned = [r for r in records if r["event"] == "positioned"]
    assert positioned
    assert all(r["get_parent"] == r["parent_hwnd"] and r["ws_child"] for r in positioned)
    assert all(r["rect"]["width"] > 0 and r["rect"]["height"] > 0 for r in positioned)
    assert len({(r["rect"]["width"], r["rect"]["height"]) for r in positioned}) >= 2
    hidden = next(i for i, r in enumerate(records) if r["event"] == "hidden" and not r["visible"])
    assert any(r["event"] == "positioned" and r["visible"] for r in records[hidden + 1:])
    print("PASS: native parent/child handles, WS_CHILD, bounds, resize, navigation hide and return")


if __name__ == "__main__":
    main()
