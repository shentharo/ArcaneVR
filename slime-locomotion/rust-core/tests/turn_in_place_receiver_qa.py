"""Exercise the real receiver and embedded renderer; never arm game output.

Run against a freshly built production executable, for example:
  python tests/turn_in_place_receiver_qa.py --output <new-output-directory>
Add --replay <private-jsonl> to replay captured raw sensor samples as well.
The live tracker config is only read; all commands persist to the isolated copy.
"""
import argparse
import copy
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

PORT, HTTP, STEAM, NEXUS = 16997, 18887, 19787, 13537
IDS = [f'127.0.0.{n}:0' for n in (2, 3, 4)]
MACS = [f'02:00:00:00:00:{n:02X}' for n in (2, 3, 4)]


def load_json(path):
    return json.loads(path.read_text(encoding='utf-8-sig'))


def series(path):
    if not path.exists():
        return []
    # A concurrently appended final line may not be complete yet.
    rows = []
    for line in path.read_text().splitlines():
        try:
            rows.append(json.loads(line))
        except json.JSONDecodeError:
            pass
    return rows


def make_config(original, replay=False):
    cfg = copy.deepcopy(original)
    cfg.update(auto_add=False, trackers=dict(zip(('left', 'right', 'torso'), IDS)),
               tracker_identities=dict(zip(IDS, MACS)), sensor_layout={'count': 3},
               movement_engine='chest_heading_steps', step_input='feet',
               foot_motion={'enabled': True, 'min_swing_degrees': 2., 'pivot_ratio': 2.})
    cfg['detection'].update(heading_axis='z', acceleration_mode='linear')
    cfg['locomotion'].update(algorithm='cadence', revision=2)
    cfg['actions']['direction'].update(enabled=False, strafe_enabled=False,
        heading_strafe_enabled=False, sprint_full_forward=False, max_output=1., decouple=False)
    for name in ('turn', 'jump', 'crouch'):
        cfg['actions'].get(name, {}).update(enabled=False)
    mounting = original.get('mounting_calibration', {})
    learned = copy.deepcopy(mounting.get('devices', {}).get(mounting.get('learned_identity'), {}))
    if not replay:
        cfg['actions']['direction'].update(learned_forward=[1., 0., 0.], learned_right=[0., 1., 0.],
            pitch_sign=1., roll_sign=1., smoothing_ms=0.)
        learned = {}
    # Keep the learned physical basis for replay, changing only the private identity.
    cfg['mounting_calibration'] = {'learned_identity': MACS[2]+':0',
        'devices': {MACS[2]+':0': learned} if learned else {}}
    return cfg


class Session:
    def __init__(self, args, original, replay=False):
        self.args, self.out = args, args.output / ('replay' if replay else 'synthetic')
        self.out.mkdir(parents=True, exist_ok=True)
        self.config = self.out / 'config.json'
        self.config.write_text(json.dumps(make_config(original, replay)))
        self.seq, self.socks = 0, []
        self.halt, self.ready = threading.Event(), threading.Event()
        self.phase = {'mode': 'still', 'yaw': 0., 'rate': 0., 'tilt': [0., 0., 0.]}
        self.log = self.app = self.worker = None
        for n in (2, 3, 4):
            s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            s.bind((f'127.0.0.{n}', 0))
            self.socks.append(s)

    def req(self, endpoint, data=None):
        request = urllib.request.Request(f'http://127.0.0.1:{HTTP}/api/{endpoint}',
            data=None if data is None else json.dumps(data).encode(),
            headers={'Content-Type': 'application/json'})
        try:
            with urllib.request.urlopen(request, timeout=2) as response:
                return json.load(response)
        except urllib.error.HTTPError as e:
            raise AssertionError(e.read().decode()) from e

    def state(self):
        value = self.req('state')
        assert value['armed'] is False, 'QA must never arm game output'
        return value

    def command(self, name, **fields):
        result = self.req('command', dict(command=name, **fields))
        assert result['ok'], result

    def send(self, side, kind, body):
        self.seq += 1
        self.socks[side].sendto(struct.pack('>IQ', kind, self.seq)+body, ('127.0.0.1', PORT))

    def hello(self):
        for i in range(3):
            self.send(i, 3, bytes(28)+bytes([3])+b'0.7'+bytes([2, 0, 0, 0, 0, i+2])+b'\0')

    def raw(self, side, q, acceleration):
        # Captures store [w,x,y,z]; SlimeVR's rotation packet requires [x,y,z,w].
        w, x, y, z = q
        self.send(side, 1, struct.pack('>ffff', x, y, z, w))
        self.send(side, 4, struct.pack('>fffB', *acceleration, 0))

    def stream(self):
        self.ready.wait()
        self.hello()
        last = time.monotonic()
        while not self.halt.is_set():
            now = time.monotonic()
            dt, last = now-last, now
            p = self.phase
            p['yaw'] += p['rate']*dt
            if p['mode'] != 'stale':
                for i in range(3):
                    yaw, tilt, acc = p['yaw'], 0., 0.
                    if p['mode'] == 'lean' and i == 2:
                        x, y, z = p['tilt']
                        angle = math.sqrt(x*x+y*y+z*z)
                        scale = math.sin(angle/2)/angle if angle else 0.
                        self.raw(i, [math.cos(angle/2), x*scale, y*scale, z*scale], [0., 0., 0.])
                        continue
                    if i < 2 and p['mode'] in ('slow', 'run', 'pivot', 'opposite-qualified'):
                        rate = 1.2 if p['mode'] == 'slow' else 3.5
                        wave = math.sin(now*math.pi*rate+i*math.pi)
                        tilt = (.14 if p['mode'] == 'slow' else .30)*(1+wave)
                        acc = (3. if p['mode'] == 'slow' else 5.)*max(0., wave)
                        if p['mode'] == 'opposite-qualified' and i == 1:
                            # The opposite foot stays yaw-stable while rolling and stepping.
                            yaw = 0.
                    c, s = math.cos(yaw/2), math.sin(yaw/2)
                    u, v = math.cos(tilt/2), math.sin(tilt/2)
                    self.raw(i, [c*u, c*v, s*v, s*u], [acc, 0., 0.])
            self.halt.wait(.008)

    def start(self, controls=False, stream=True, capture=False):
        env = dict(os.environ, ARCANEVR_EMBED_QA='1',
            ARCANEVR_EMBED_QA_REPORT=str(self.out/'embedding.json'),
            ARCANEVR_VIEWER_QA_REPORT=str(self.out/'renderer.json'),
            ARCANEVR_VIEWER_QA_SERIES=str(self.out/'series.jsonl'),
            ARCANEVR_VIEWER_QA_REPORT_AFTER='25',
            ARCANEVR_VIEWER_QA_SCREENSHOT=str(self.out/'viewer.png'),
            ARCANEVR_VIEWER_QA_CAPTURE_AT='16')
        env.pop('ARCANEVR_EMBED_QA_CONTROLS', None)
        if controls:
            env['ARCANEVR_EMBED_QA_CONTROLS'] = '1'
        self.log = (self.out/'app.log').open('w')
        command = [str(self.args.exe), '--config', str(self.config),
            '--http-port', str(HTTP), '--tracker-port', str(PORT), '--tracker-bind', '127.0.0.1',
            '--nexus-port', str(NEXUS), '--steam-port', str(STEAM), '--qa-page', 'test-preview']
        if capture:
            command += ['--qa-foot-motion-open', '--qa-screenshot', str(self.out/'app.png')]
        self.app = subprocess.Popen(command,
            cwd=self.args.root, env=env, stdout=self.log, stderr=self.log,
            creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
        until = time.monotonic()+20
        while True:
            try:
                self.state()
                break
            except OSError:
                assert self.app.poll() is None, 'Receiver exited; inspect app.log'
                if time.monotonic() > until:
                    raise
                time.sleep(.1)
        if stream:
            self.worker = threading.Thread(target=self.stream)
            self.worker.start()
            self.ready.set()
        else:
            self.hello()

    def calibrate(self):
        deadline = time.monotonic()+10
        while not self.state()['calibration_ready']:
            assert time.monotonic() < deadline, 'Synthetic trackers not ready'
            time.sleep(.04)
        self.command('calibrate')

    def close(self):
        self.halt.set()
        self.ready.set()
        if self.worker:
            self.worker.join(timeout=3)
        for s in self.socks:
            s.close()
        if self.app and self.app.poll() is None:
            # WM_CLOSE is handled by the app and reaps its owned renderer child.
            import ctypes
            user32 = ctypes.windll.user32
            callback_type = ctypes.WINFUNCTYPE(ctypes.c_bool, ctypes.c_void_p, ctypes.c_void_p)
            @callback_type
            def visit(hwnd, unused):
                pid = ctypes.c_ulong()
                user32.GetWindowThreadProcessId(ctypes.c_void_p(hwnd), ctypes.byref(pid))
                if pid.value == self.app.pid:
                    user32.PostMessageW(ctypes.c_void_p(hwnd), 0x10, 0, 0)
                return True
            user32.EnumWindows(visit, 0)
            try:
                self.app.wait(timeout=10)
            except subprocess.TimeoutExpired:
                subprocess.run(['taskkill', '/PID', str(self.app.pid), '/T', '/F'], capture_output=True, check=True)
        if self.app:
            # Match both executable location and the exact owned parent argument.
            runtime = str(self.args.exe.parent/'.arcane-runtime').replace("'", "''")
            script = ("Get-CimInstance Win32_Process | Where-Object { "
                + "$_.ExecutablePath -like '"+runtime+"\\*' -and "
                + "$_.CommandLine -match '(?:^| )--parent-pid "+str(self.app.pid)+"(?: |$)' "
                + "} | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }")
            subprocess.run(['powershell', '-NoProfile', '-Command', script], capture_output=True, check=True)
        if self.log:
            self.log.close()

    def sample(self, mode, duration, rate=0., settle=.25):
        self.phase.update(mode=mode, rate=rate)
        time.sleep(settle)
        wall_start = time.monotonic()
        rows = []
        while time.monotonic()-wall_start < duration:
            v = self.state()
            xy = v['game_output_preview']['travel_xy']
            assert math.hypot(*xy) <= 1.00001, xy
            rows.append({'at': time.monotonic(), 'magnitude': math.hypot(*xy),
                'heading': v['heading'], 'foot_motion': v['foot_motion'], 'travel_xy': xy})
            time.sleep(.025)
        return rows

    def renderer_latest(self):
        deadline = time.monotonic()+10
        while True:
            rows = series(self.out/'series.jsonl')
            if rows:
                return rows[-1]
            assert time.monotonic() < deadline, 'Renderer did not publish actual translation'
            time.sleep(.1)

    def stationary_renderer(self, rows, name):
        # Use synchronized latest renderer stamps, excluding the receiver poll latency.
        start, end = rows[0]['at'], rows[-1]['at']
        matching = [r for at, r in self.renderer_samples if start+.15 <= at <= end]
        assert len(matching) >= 4, (name, 'no renderer samples')
        origin = matching[0]['position']
        drift = max(math.dist(origin, r['position']) for r in matching)
        assert drift < .002, (name, 'actual renderer moved during turn', drift)
        return drift


def synthetic(args, original):
    s = Session(args, original)
    checks = []
    try:
        s.start(controls=True)
        s.calibrate()
        # Let the real embedded UI exercise its command pipe and page lifecycle.
        s.phase['mode'] = 'run'
        deadline = time.monotonic()+22
        while True:
            events = series(s.out/'embedding.jsonl')
            hidden = [r for r in events if r['event'] == 'hidden']
            if hidden and any(r['event'] == 'positioned' and r['pid'] != hidden[0]['pid'] for r in events):
                # The navigation control can run while no child handle exists;
                # native hide then fresh embedded child is the observable return.
                break
            assert time.monotonic() < deadline, 'Embedded pause/reset/navigation QA did not finish'
            s.state()
            time.sleep(.1)
        history = series(s.out/'series.jsonl')
        paused = [r for r in history if r['paused']]
        assert len(paused) >= 3, 'Real renderer pause not observed'
        assert max(math.dist(paused[0]['position'], r['position']) for r in paused) < .002
        assert any(a['distance_m'] > .1 and b['distance_m'] < a['distance_m']*.4
                   for a, b in zip(history, history[1:])), 'Real renderer reset not observed'
        assert any(r['event'] == 'qa_resume' for r in events)
        embed = load_json(s.out/'embedding.json')
        assert embed['ws_child'] and embed['get_parent'] == embed['parent_hwnd'], embed
        assert embed['rect']['width'] > 100 and embed['rect']['height'] > 100, embed
        s.sample('still', .6, settle=.8)
        s.renderer_samples = []
        monitor_halt = threading.Event()
        def monitor():
            while not monitor_halt.wait(.05):
                s.renderer_samples.append((time.monotonic(), s.renderer_latest()))
        monitor_thread = threading.Thread(target=monitor)
        monitor_thread.start()
        try:
            for mode, duration, minimum in [('slow', 3., .06), ('run', 2., .7)]:
                rows = s.sample(mode, duration, settle=1.)
                mean = sum(r['magnitude'] for r in rows)/len(rows)
                assert mean > minimum, (mode, mean)
                checks.append({'phase': mode, 'mean_output': mean})
            rows = s.sample('run', 2., rate=math.radians(8), settle=.6)
            assert sum(r['magnitude'] for r in rows)/len(rows) > .65, 'Gentle curve stopped walking'
            checks.append({'phase': 'gentle_curve_8deg_per_s', 'mean_output': sum(r['magnitude'] for r in rows)/len(rows)})
            # First detected body rotation must immediately stop an already running gait.
            start = time.monotonic()
            rows = s.sample('pivot', .65, rate=math.radians(90), settle=0.)
            first_zero = next((r['at']-start for r in rows if r['magnitude'] < .001), None)
            assert first_zero is not None and first_zero < .16, ('walk-to-turn latency', first_zero)
            assert max(r['magnitude'] for r in rows if r['at']-start > .16) < .001
            checks.append({'phase': 'walk_to_turn', 'stop_latency_seconds': first_zero})
            for name, rate, mode in [('clockwise_full_rotation', 90., 'pivot'),
                                     ('counterclockwise_full_rotation', -90., 'opposite-qualified')]:
                rows = s.sample(mode, 4.25, rate=math.radians(rate), settle=.3)
                assert max(r['magnitude'] for r in rows) < .001, name
                # Heading wrap itself must be crossed, not merely simulated in labels.
                assert any(abs(a['heading']-b['heading']) > 5 for a,b in zip(rows, rows[1:])), (name, 'no +/-pi wrap')
                drift = s.stationary_renderer(rows, name)
                assert any(any(f['qualified'] for f in r['foot_motion']['feet']) for r in rows), (name, 'no qualified rolling foot')
                checks.append({'phase': name, 'actual_renderer_drift_m': drift, 'qualified_foot_seen': True})
            # Stop all stepping during the heading-stable hold. Old cadence may not resume.
            rows = s.sample('still', 1.1, settle=.04)
            assert max(r['magnitude'] for r in rows) < .001, 'Residual cadence resumed after turning'
            assert any(r['foot_motion']['body_turn']['hold_seconds'] > .1 for r in rows), 'No heading-stable body-turn hold observed'
            checks.append({'phase': 'stable_heading_hold_and_no_new_steps', 'max_output': max(r['magnitude'] for r in rows)})
            rows = s.sample('run', 2., settle=.7)
            assert max(r['magnitude'] for r in rows) > .8, 'Fresh post-turn steps did not restore walking'
            rows = s.sample('stale', .65, settle=.5)
            assert max(r['magnitude'] for r in rows) < .001, 'Stale-input stop failed'
            checks.append({'phase': 'stale_input', 'actual_renderer_drift_m': s.stationary_renderer(rows, 'stale_input')})
            s.sample('still', .4, settle=.4)
            # Verify signed receiver output AND rendered displacement on all four axes.
            s.phase.update(yaw=0., mode='still', rate=0.)
            time.sleep(.7)
            s.command('movement_engine', engine='body_tilt')
            settings = s.state()['actions']['settings']
            settings['direction'].update(enabled=True, decouple=False, strafe_enabled=True,
                heading_strafe_enabled=False, learned_forward=[1.,0.,0.], learned_right=[0.,1.,0.],
                pitch_sign=1., roll_sign=1., smoothing_ms=0., max_angle=.4,
                horizontal_max_angle=.4, forward_max_angle=.4, forward_offset=0., sprint_angle=.6,
                deadzone=.05, forward_deadzone=.05, backward_deadzone=.05,
                left_deadzone=.05, right_deadzone=.05)
            s.command('actions_settings', settings=settings)
            s.calibrate()
            s.phase['mode'] = 'lean'
            for name, pose, axis, sign, world_axis, world_sign in [
                ('forward', [.35,0.,0.], 1,1,2,-1), ('backward', [-.35,0.,0.],1,-1,2,1),
                ('right', [0.,.35,0.],0,1,0,1), ('left',[0.,-.35,0.],0,-1,0,-1)]:
                s.phase['tilt'] = pose
                time.sleep(.4)
                before = s.renderer_latest()
                rows = s.sample('lean', .75, settle=.1)
                after = s.renderer_latest()
                assert rows[-1]['travel_xy'][axis]*sign > .1, (name, rows[-1])
                delta = after['position'][world_axis]-before['position'][world_axis]
                assert delta*world_sign > .1, (name, before, after)
                checks.append({'phase': name, 'travel_xy': rows[-1]['travel_xy'], 'renderer_axis_displacement_m': delta})
            s.phase.update(mode='still', tilt=[0.,0.,0.])
            s.sample('still', .2, settle=.3)
        finally:
            monitor_halt.set()
            monitor_thread.join(timeout=3)
        renderer = load_json(s.out/'renderer.json')
        assert renderer['character_loaded'] and renderer['fixed_ground'], renderer
        result = {'passed': True, 'phases': checks, 'pause_resume_reset_actual_renderer': True,
            'embedded_native_parent_verified': True, 'stale_input_stop': True,
            'game_output_armed': False, 'physical_hardware': 'not tested'}
        (s.out/'result.json').write_text(json.dumps(result, indent=2))
        return result
    finally:
        s.close()


def replay(args, original):
    capture = [json.loads(line) for line in args.replay.read_text().splitlines() if line.strip()]
    assert len(capture) > 20, 'Replay needs actual raw sensor samples'
    s = Session(args, original, replay=True)
    checks = []
    try:
        s.start(stream=False)
        first = capture[0]
        def send_row(row):
            for i, role in enumerate(('left','right','torso')):
                sensor = row['roles'][role]
                if sensor.get('fresh', True):
                    s.raw(i, sensor['quaternion'], sensor.get('acceleration', [0.,0.,0.]))
        # First raw sample establishes a center with the captured mounting basis.
        for unused in range(35):
            send_row(first)
            time.sleep(.01)
        s.calibrate()
        start, capture_start = time.monotonic(), capture[0]['t']
        for row in capture:
            due = start+row['t']-capture_start
            time.sleep(max(0., due-time.monotonic()))
            send_row(row)
            v = s.state()
            checks.append({'t': row['t']-capture_start, 'heading': v['heading'],
                'output': math.hypot(*v['game_output_preview']['travel_xy']),
                'foot_motion': v['foot_motion'],
                'original_output': math.hypot(*row.get('game_output_preview', {}).get('travel_xy',[0.,0.]))})
        # Deliberate rotation: evaluate actual receiver heading, including wrapped deltas.
        turning = []
        for previous, current in zip(checks, checks[1:]):
            dt = current['t']-previous['t']
            delta = math.atan2(math.sin(current['heading']-previous['heading']), math.cos(current['heading']-previous['heading']))
            if dt > 0 and abs(delta)/dt > math.radians(20):
                turning.append(current)
        assert len(turning) > 10, 'Replay did not exercise deliberate chest turning'
        leaking = [r for r in checks if r['output'] > .001]
        (s.out/'receiver-samples.json').write_text(json.dumps(checks, indent=2))
        whole_capture = {'samples': len(checks), 'max_output': max(r['output'] for r in checks),
            'nonzero_samples': len(leaking), 'original_peak_output': max(r['original_output'] for r in checks)}
        (s.out/'whole-capture.json').write_text(json.dumps(whole_capture, indent=2))
        assert not leaking, ('Private physical-pivot capture translated; entire recording is standing/turning', whole_capture, leaking[:4])
        result = {'passed': True, 'samples': len(checks), 'duration_seconds': checks[-1]['t'],
            'deliberate_turn_samples': len(turning), 'maximum_turn_output': max(r['output'] for r in turning),
            'original_capture_turn_peak': max(r['original_output'] for r in turning),
            'entire_capture': whole_capture,
            'recorded_raw_sensor_replay': True, 'game_output_armed': False,
            'physical_hardware': 'recorded sensors replayed; no new physical test'}
        (s.out/'receiver-samples.json').write_text(json.dumps(checks, indent=2))
        (s.out/'result.json').write_text(json.dumps(result, indent=2))
        return result
    finally:
        s.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(r'C:\Users\Nexpo\Desktop\KATVR'))
    parser.add_argument('--exe', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--replay', type=Path)
    parser.add_argument('--capture', action='store_true', help='Separate app-owned foot-motion panel capture; use a fresh output directory')
    args = parser.parse_args()
    args.root, args.output = args.root.resolve(), args.output.resolve()
    args.exe = (args.exe or args.root/'slime-locomotion/rust-core/target/release/vrcraft.exe').resolve()
    assert args.exe.is_file(), f'Build production executable first: {args.exe}'
    if args.replay is not None:
        args.replay = args.replay.resolve()
        assert args.replay.is_file(), f'Explicitly requested replay is missing: {args.replay}'
    assert not args.output.exists() or not any(args.output.iterdir()), 'Use an empty output directory to avoid stale QA evidence'
    args.output.mkdir(parents=True, exist_ok=True)
    config_path = args.root/'tracker-group.json'
    config_before = config_path.read_bytes()
    original = load_json(config_path)
    result = {'executable_sha256': hashlib.sha256(args.exe.read_bytes()).hexdigest()}
    if args.capture:
        session = Session(args, original)
        try:
            session.phase.update(mode='pivot', rate=math.radians(90))
            session.start(capture=True)
            session.calibrate()
            assert session.app.wait(timeout=35) == 0
            assert (session.out/'app.png').is_file(), 'App-owned panel capture missing'
            result['capture'] = str(session.out/'app.png')
        finally:
            session.close()
    else:
        result['synthetic'] = synthetic(args, original)
    if args.replay:
        result['replay'] = replay(args, original)
    assert config_path.read_bytes() == config_before, 'Live config unexpectedly changed'
    result['live_config_unchanged'] = True
    (args.output/'result.json').write_text(json.dumps(result, indent=2))
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
