"""Regression test for an isolated receiver's staged sensor calibration API."""
import argparse
import ctypes
import hashlib
import json
import math
import os
from pathlib import Path
import socket
import struct
import subprocess
import threading
import time
import urllib.error
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[3]
IDENTITY = '02:00:00:00:00:01:0'
BASE = 'http://127.0.0.1:18827/api/'
UDP = ('127.0.0.1', 16971)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def request(endpoint, value=None, reject=False):
    data = None if value is None else json.dumps(value).encode()
    req = urllib.request.Request(BASE + endpoint, data=data, headers={'Content-Type': 'application/json'})
    try:
        with urllib.request.urlopen(req, timeout=3) as response:
            body = json.load(response)
    except urllib.error.HTTPError as error:
        body = error.read().decode(errors='replace')
        if reject:
            return body
        raise AssertionError(f'{endpoint}: HTTP {error.code}: {body}') from error
    if reject:
        # Some receiver versions report command errors in a successful HTTP response.
        assert isinstance(body, dict) and (body.get('error') or body.get('ok') is False), body
    return body


def command(name, **fields):
    return request('command', {'command': name, **fields})


def rejected(name, **fields):
    return request('command', {'command': name, **fields}, reject=True)


def wait_for(predicate, description, timeout=5):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            value = request('state')
            if predicate(value):
                return value
        except (OSError, urllib.error.URLError):
            pass
        time.sleep(.05)
    raise AssertionError(f'timed out waiting for {description}')


def close_owned_window(pid):
    user32 = ctypes.windll.user32
    callback = ctypes.WINFUNCTYPE(ctypes.c_bool, ctypes.c_void_p, ctypes.c_void_p)
    @callback
    def visit(hwnd, _):
        owner = ctypes.c_ulong()
        user32.GetWindowThreadProcessId(ctypes.c_void_p(hwnd), ctypes.byref(owner))
        if owner.value == pid:
            user32.PostMessageW(ctypes.c_void_p(hwnd), 0x10, 0, 0)
        return True
    user32.EnumWindows(visit, 0)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--exe', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    config = output / 'isolated-sensor-calibration-config.json'
    assert not config.exists(), f'refusing to overwrite {config}'
    cfg = json.loads((ROOT / 'tracker-group.json').read_text())
    cfg.update(trackers={'left': '', 'right': '', 'torso': '127.0.0.1:0'},
               tracker_identities={'127.0.0.1:0': '02:00:00:00:00:01'},
               auto_add=False, movement_engine='body_tilt', step_input='off')
    cfg['detection']['heading_axis'] = 'z'
    direction = cfg['actions']['direction']
    direction.update(pitch_sign=-1.0, roll_sign=-1.0, smoothing_ms=0,
                     learned_forward=[0., 1., 0.], learned_right=[1., 0., 0.])
    cfg['mounting_calibration'] = {'learned_identity': IDENTITY, 'devices': {IDENTITY: dict(direction)}}
    config.write_text(json.dumps(cfg), encoding='utf-8')
    initial_hash = digest(config)
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    stop_stream = threading.Event()
    pose = [1., 0., 0., 0.]
    seq = 0

    def send(kind, body, hello=False):
        nonlocal seq
        seq = 0 if hello else seq + 1
        sock.sendto(struct.pack('>IQ', kind, seq) + body, UDP)

    def stream():
        while not stop_stream.is_set():
            w, x, y, z = pose
            send(1, struct.pack('>ffff', x, y, z, w))
            time.sleep(.008)

    app = None
    worker = None
    log = (output / 'isolated-app.log').open('w', encoding='utf-8')

    def launch():
        nonlocal app, worker, seq
        stop_stream.clear()
        app = subprocess.Popen([str(args.exe.resolve()), '--config', str(config),
            '--http-port', '18827', '--tracker-port', '16971', '--nexus-port', '13502',
            '--steam-port', '19743'], cwd=ROOT, stdout=log, stderr=log)
        wait_for(lambda s: True, 'receiver startup', 20)
        send(3, bytes(28) + bytes([3]) + b'0.7' + bytes([2, 0, 0, 0, 0, 1]) + b'\0', hello=True)
        worker = threading.Thread(target=stream, daemon=True)
        worker.start()
        wait_for(lambda s: s.get('roles', {}).get('torso', {}).get('fresh'), 'fresh tracker')

    def shutdown():
        nonlocal app, worker
        stop_stream.set()
        if worker:
            worker.join(timeout=2)
            worker = None
        if app:
            close_owned_window(app.pid)
            try:
                app.wait(timeout=12)
            except subprocess.TimeoutExpired:
                app.terminate()
                try:
                    app.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    app.kill()
                    app.wait(timeout=5)
            app = None

    def snapshot(session, stage):
        state = wait_for(lambda s: isinstance(s.get('sensor_calibration'), dict)
                         and s['sensor_calibration'].get('session') == session
                         and s['sensor_calibration'].get('stage') == stage,
                         f'{session} stage {stage}')
        calibration = state['sensor_calibration']
        assert calibration['identity'] == IDENTITY, calibration
        assert not state['armed'], state
        return calibration

    def capture(session, stage, q, next_stage):
        pose[:] = q
        time.sleep(.2)
        before = snapshot(session, stage)
        assert before['fresh'], before
        assert before['angles'] is None or len(before['angles']) == 2, before
        command('sensor_calibration_capture', session=session, stage=stage)
        return snapshot(session, next_stage)

    def begin(session):
        command('sensor_calibration_begin', session=session)
        return snapshot(session, 'center')

    def complete(session):
        c, s = math.cos(math.radians(10)), math.sin(math.radians(10))
        capture(session, 'center', [1., 0., 0., 0.], 'forward')
        capture(session, 'forward', [c, -s, 0., 0.], 'backward')
        capture(session, 'backward', [c, s, 0., 0.], 'left')
        capture(session, 'left', [c, 0., -s, 0.], 'right')
        capture(session, 'right', [c, 0., s, 0.], 'finish')
        pose[:] = [1., 0., 0., 0.]
        time.sleep(.2)
        command('sensor_calibration_capture', session=session, stage='finish')
        saved = output / 'calibrations' / f'calibration-{session}.json'
        deadline = time.monotonic() + 5
        while not saved.exists() and time.monotonic() < deadline:
            time.sleep(.05)
        assert saved.is_file(), saved
        record = json.loads(config.read_text(encoding='utf-8'))['mounting_calibration']['devices'][IDENTITY]
        assert record['session'] == session and Path(record['calibration_file']).name == saved.name, record
        return saved

    try:
        launch()
        cancelled = 'cancel_' + uuid.uuid4().hex
        begin(cancelled)
        rejected('arm')
        assert not request('state')['armed']
        rejected('sensor_calibration_capture', session=cancelled, stage='finish')
        stop_stream.set()
        worker.join(timeout=2)
        worker = None
        wait_for(lambda state: not state['sensor_calibration']['fresh'], 'stale rotation')
        rejected('sensor_calibration_capture', session=cancelled, stage='center')
        assert snapshot(cancelled, 'center')['fresh'] is False
        stop_stream.clear()
        worker = threading.Thread(target=stream, daemon=True)
        worker.start()
        wait_for(lambda state: state['sensor_calibration']['fresh'], 'new rotation')
        command('sensor_calibration_cancel', session=cancelled)
        assert digest(config) == initial_hash
        assert not (output / 'calibrations').exists() or not list((output / 'calibrations').iterdir())
        partial = 'partial_' + uuid.uuid4().hex
        begin(partial)
        capture(partial, 'center', [1., 0., 0., 0.], 'forward')
        shutdown()
        assert digest(config) == initial_hash
        launch()
        assert not request('state').get('sensor_calibration')
        first = 'first_' + uuid.uuid4().hex
        begin(first)
        first_file = complete(first)
        first_hash = digest(first_file)
        second = 'second_' + uuid.uuid4().hex
        begin(second)
        second_file = complete(second)
        assert digest(first_file) == first_hash and second_file != first_file
        result = {'passed': True, 'sessions': [first, second], 'first_file': str(first_file),
                  'second_file': str(second_file), 'physical_sensor_test': False,
                  'game_output_armed': False}
        (output / 'sensor-calibration-qa.json').write_text(json.dumps(result, indent=2), encoding='utf-8')
        print(json.dumps(result, indent=2))
    finally:
        shutdown()
        sock.close()
        log.close()


if __name__ == '__main__':
    main()
