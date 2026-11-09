"""Exercise an isolated receiver and its real embedded renderer, never VR output."""
import ctypes
import json
import math
import os
from pathlib import Path
import socket
import struct
import subprocess
import threading
import time
import urllib.request

ROOT = Path(os.environ.get('ARCANEVR_QA_ROOT', str(Path(__file__).resolve().parents[3])))
OUT = ROOT / 'release/lean-output-contract' / time.strftime('qa-%H%M%S')
OUT.mkdir(parents=True,exist_ok=True)
cfg = json.loads((ROOT / 'tracker-group.json').read_text())
cfg['trackers'] = {'left': '', 'right': '', 'torso': '127.0.0.1:0'}
cfg['tracker_identities'] = {'127.0.0.1:0': '02:00:00:00:00:01'}
cfg['mounting_calibration'] = {'learned_identity': '02:00:00:00:00:01:0'}
cfg['auto_add'] = False
cfg['step_input'] = 'off'
cfg['movement_engine'] = 'body_tilt'
cfg['actions']['direction']['smoothing_ms'] = 0
cfg['actions']['direction']['decouple'] = True
cfg['actions']['direction']['strafe_enabled'] = True
# Known test mounting: physical direction assignments never come from user's live flips.
cfg['actions']['direction']['pitch_sign'] = -1.
cfg['actions']['direction']['roll_sign'] = -1.
cfg['mounting_calibration']['devices'] = {}

config = OUT / 'isolated-direction-config.json'
config.write_text(json.dumps(cfg))
env = os.environ.copy()
env.update(ARCANEVR_EMBED_QA='1',
           ARCANEVR_EMBED_QA_REPORT=str(OUT / 'embed.json'),
           ARCANEVR_VIEWER_QA_SCREENSHOT=str(OUT / 'renderer.png'),
           ARCANEVR_VIEWER_QA_REPORT=str(OUT / 'renderer.json'),
           ARCANEVR_VIEWER_QA_SERIES=str(OUT / 'series.jsonl'),
           ARCANEVR_VIEWER_QA_REPORT_AFTER='22')
base = 'http://127.0.0.1:18825/api/'

def state():
    return json.load(urllib.request.urlopen(base+'state', timeout=2))

def command(v):
    req=urllib.request.Request(base+'command',data=json.dumps(v).encode(),headers={'Content-Type':'application/json'})
    json.load(urllib.request.urlopen(req,timeout=2))
    time.sleep(.1)

sock=socket.socket(socket.AF_INET,socket.SOCK_DGRAM)
seq=0
pose=[1.,0.,0.,0.]
halt=threading.Event()

driver=socket.socket(socket.AF_INET,socket.SOCK_DGRAM)
driver.bind(('127.0.0.1',19741));driver.settimeout(.1)
def hmd():
    while not halt.is_set():
        try: packet,peer=driver.recvfrom(2048)
        except socket.timeout: continue
        if len(packet)>=12: driver.sendto(b'AKV2'+packet[4:12]+struct.pack('<Iffff',7,1.7,0.,0.,0.),peer)
threading.Thread(target=hmd,daemon=True).start()

def send(kind, body, hello=False):
    global seq
    seq=0 if hello else seq+1
    sock.sendto(struct.pack('>IQ',kind,seq)+body,('127.0.0.1',16969))

def stream():
    while not halt.is_set():
        w,x,y,z=pose
        send(1,struct.pack('>ffff',x,y,z,w))
        time.sleep(.008)

def close_owned_window(pid):
    # Normal WM_CLOSE, scoped to the QA process created above.
    user32=ctypes.windll.user32
    callback=ctypes.WINFUNCTYPE(ctypes.c_bool,ctypes.c_void_p,ctypes.c_void_p)
    @callback
    def visit(hwnd, _):
        owner=ctypes.c_ulong()
        user32.GetWindowThreadProcessId(ctypes.c_void_p(hwnd),ctypes.byref(owner))
        if owner.value==pid:
            user32.PostMessageW(ctypes.c_void_p(hwnd),0x10,0,0)
        return True
    user32.EnumWindows(visit,0)

with (OUT / 'isolated-app.log').open('w') as log:
    app=subprocess.Popen([os.environ.get('ARCANEVR_QA_EXE',str(ROOT/'VRCraft.exe')),'--config',str(config),
        '--http-port','18825','--tracker-port','16969','--nexus-port','13500',
        '--steam-port','19741','--qa-page','test'],cwd=ROOT,env=env,stdout=log,stderr=log)
    try:
        deadline=time.monotonic()+15
        while True:
            try:
                state();break
            except Exception:
                if time.monotonic()>deadline:raise
                time.sleep(.1)
        send(3,bytes(28)+bytes([3])+b'0.7'+bytes([2,0,0,0,0,1])+b'\0',hello=True)
        worker=threading.Thread(target=stream,daemon=True);worker.start()
        time.sleep(.4);command({'command':'calibrate'})
        c,s=math.cos(.18),math.sin(.18)
        checks=[]
        for name,q,axis,sign in [('forward',[c,-s,0.,0.],1,1),
                               ('backward',[c,s,0.,0.],1,-1),
                               ('right',[c,0.,s,0.],0,1),
                               ('left',[c,0.,-s,0.],0,-1)]:
            pose=q;time.sleep(.9);v=state()
            xy=v['actions']['preview']['travel_xy']
            assert xy[axis]*sign>.1,(name,xy)
            assert not v['armed']
            final=v['game_output_preview']['travel_xy']
            assert max(abs(a-b) for a,b in zip(final,xy))<1e-6,(final,xy)
            checks.append({'direction':name,'travel_xy':xy})
        # The Test checkbox sends this exact settings command. Toggle only the
        # existing gate, retaining the learned mounting, center and forward input.
        baseline=state()
        settings=baseline['actions']['settings']
        settings['direction']['strafe_enabled']=False
        command({'command':'actions_settings','settings':settings})
        for name,q,axis,sign in [('right-disabled',[c,0.,s,0.],0,0),
                               ('left-disabled',[c,0.,-s,0.],0,0),
                               ('forward-still-enabled',[c,-s,0.,0.],1,1)]:
            pose=q;time.sleep(.5);v=state()
            xy=v['game_output_preview']['travel_xy']
            assert (abs(xy[axis])<.001 if sign==0 else xy[axis]*sign>.1),(name,xy)
            assert v['actions']['preview']['calibrated'],name
            assert not v['armed']
            checks.append({'direction':name,'travel_xy':xy})
        saved=json.loads(config.read_text())
        assert saved['actions']['direction']['strafe_enabled'] is False
        assert saved['trackers']==cfg['trackers']
        assert v['actions']['settings']['direction']['learned_forward']==baseline['actions']['settings']['direction']['learned_forward']
        assert v['actions']['settings']['direction']['learned_right']==baseline['actions']['settings']['direction']['learned_right']
        settings['direction']['strafe_enabled']=True
        command({'command':'actions_settings','settings':settings})
        pose=[c,0.,s,0.];time.sleep(.5);v=state()
        assert v['game_output_preview']['travel_xy'][0]>.1
        assert json.loads(config.read_text())['actions']['direction']['strafe_enabled'] is True
        checks.append({'strafe_toggle':'off suppresses left/right, preserves forward and center; on restores right; persisted without assignment changes'})
        settings=state()['actions']['settings']
        d=settings['direction']
        d.update(deadzone=0.1,forward_deadzone=0.1,backward_deadzone=0.1,left_deadzone=0.1,right_deadzone=0.1,max_angle=0.4,horizontal_max_angle=0.4,forward_max_angle=0.4,forward_offset=0.,exponent=1.,max_output=1.,smoothing_ms=0.,sprint_full_forward=True,sprint_angle=0.6)
        command({'command':'actions_settings','settings':settings})
        for angle,speed,sprint in [(0.09,0.,False),(0.1,0.,False),(0.25,0.5,False),(0.4,1.,False),(0.59,1.,False),(0.61,1.,True)]:
            pose=[math.cos(angle/2),-math.sin(angle/2),0.,0.];time.sleep(.25);v=state()
            out=v['game_output_preview']
            assert abs(out['travel_xy'][1]-speed)<0.015,(angle,out)
            assert bool(out['buttons'] & 4)==sprint,(angle,out)
            assert not v['armed']
        checks.append({'rings':'deadzone zero, midpoint half, full speed, sprint threshold verified'})
        pose=[1.,0.,0.,0.];time.sleep(.3)
        # Rotation in place must not turn into lean translation.
        for i in range(121):
            a=math.pi*i/120
            pose=[math.cos(a),0.,0.,math.sin(a)]
            time.sleep(.02)
        pose=[1.,0.,0.,0.];time.sleep(.3)
        assert max(map(abs,state()['actions']['preview']['travel_xy']))<.001
        halt.set();worker.join();time.sleep(.4)
        assert state()['actions']['preview']['travel_xy']==[0.,0.]
        deadline=time.monotonic()+18
        while not (OUT/'renderer.json').exists():
            if time.monotonic()>deadline:raise TimeoutError('Embedded renderer report')
            time.sleep(.2)
        render=json.loads((OUT/'renderer.json').read_text())
        embed=json.loads((OUT/'embed.json').read_text())
        assert embed['ws_child'] and embed['get_parent']==embed['parent_hwnd'],embed
        assert render['character_loaded'] and render['fixed_ground'],render
        assert {'Forward','Backward','Left','Right'} <= set(render['animations_seen']),render
        assert abs(render['max_sprint_speed']/render['max_walk_speed']-1.2)<0.02,render
        assert render['distance_m']>1 and abs(render['rotation_degrees'])>350,render
        result={'passed':True,'receiver_checks':checks,'renderer':render,'embedded':True,
                'physical_sensor_test':False,'game_output_armed':False}
        (OUT/'direction-qa.json').write_text(json.dumps(result,indent=2))
        print(json.dumps(result,indent=2))
    finally:
        halt.set();sock.close();close_owned_window(app.pid)
        app.wait(timeout=12)
