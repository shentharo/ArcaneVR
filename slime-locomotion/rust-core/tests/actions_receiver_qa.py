"""Isolated synthetic chest/action exercise against the actual embedded renderer.
Run only after the owner installs the intended app and viewer builds. Never arms output.
"""
import ctypes, json, math, os, socket, struct, subprocess, threading, time, urllib.request
from pathlib import Path

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'release'/'actions-receiver-qa'/time.strftime('qa-%Y%m%d-%H%M%S')
OUT.mkdir(parents=True,exist_ok=True)
cfg=json.loads((ROOT/'tracker-group.json').read_text(encoding='utf-8-sig'))
cfg.update(movement_engine='body_tilt_steps',step_input='chest',auto_add=False,output_mode='steamvr')
cfg['sensor_layout']={'count':1,'thighs':['','']}
cfg['trackers']={'left':'','right':'','torso':'127.0.0.1:0'}
cfg['tracker_identities']={'127.0.0.1:0':'02:00:00:00:00:01'}
cfg['mounting_calibration']={'learned_identity':'02:00:00:00:00:01:0'}
cfg['locomotion']['algorithm']='continuous'
cfg['locomotion']['settings'].update(threshold=1.,max_signal=5.,intensity=1.,fast_window=.04,slow_window=.1,inertia_window=.05,adaptive_per_unit=0.,sprint_threshold=8.)
cfg['detection']['acceleration_mode']='linear'
d=cfg['actions']['direction'];d.update(enabled=True,decouple=False,smoothing_ms=0)
cfg['actions']['jump'].update(enabled=True,source='acceleration',accel_axis='y',threshold=2.,release=0.5,accel_window_seconds=0.06,pulse_seconds=0.5)
cfg['actions']['crouch'].update(enabled=True,enter=1.1,exit=1.4,mode='hold',use_reset_offset=False)
cfg['actions']['turn'].update(enabled=True,horizontal=True,vertical=False,deadzone=0.1,max_angle=0.8,exponent=1.,yaw_sign=1.)
config=OUT/'isolated-config.json';config.write_text(json.dumps(cfg),encoding='utf-8')
env=os.environ.copy();env.update(ARCANEVR_EMBED_QA='1',ARCANEVR_EMBED_QA_REPORT=str(OUT/'embed.json'),ARCANEVR_VIEWER_QA_SCREENSHOT=str(OUT/'renderer.png'),ARCANEVR_VIEWER_QA_REPORT=str(OUT/'renderer.json'))
base='http://127.0.0.1:18826/api/'
def state():return json.load(urllib.request.urlopen(base+'state',timeout=2))
def command(v):
 req=urllib.request.Request(base+'command',data=json.dumps(v).encode(),headers={'Content-Type':'application/json'})
 result=json.load(urllib.request.urlopen(req,timeout=2));assert not result.get('error'),result;return result

def until(predicate, timeout=4):
 end=time.monotonic()+timeout;last=None
 while time.monotonic()<end:
  last=state()
  if predicate(last):return last
  time.sleep(.04)
 raise AssertionError(('timed out',last))

sock=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);driver=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);driver.bind(('127.0.0.1',19742));driver.settimeout(.1)
halt=threading.Event();tracker_on=True;seq=0;pose=[1.,0.,0.,0.];accel=[0.,0.,0.];head=[1.7,0.,0.,0.];oscillate=False

def send(kind,body,hello=False):
 global seq
 seq=0 if hello else seq+1;sock.sendto(struct.pack('>IQ',kind,seq)+body,('127.0.0.1',16970))
def tracker():
 while not halt.is_set():
  if tracker_on:
   w,x,y,z=pose;send(1,struct.pack('>ffff',x,y,z,w));a=list(accel)
   if oscillate:a[2]=4.*math.sin(time.monotonic()*math.tau*2.)
   send(4,struct.pack('>fffB',*a,0))
  time.sleep(.01)
def fake_hmd():
 while not halt.is_set():
  try:packet,peer=driver.recvfrom(2048)
  except socket.timeout:continue
  except OSError:break
  if len(packet)>=12:driver.sendto(b'AKV2'+packet[4:12]+struct.pack('<Iffff',7,*head),peer)
def close_owned_window(pid):
 user32=ctypes.windll.user32;callback=ctypes.WINFUNCTYPE(ctypes.c_bool,ctypes.c_void_p,ctypes.c_void_p)
 @callback
 def visit(hwnd,_):
  owner=ctypes.c_ulong();user32.GetWindowThreadProcessId(ctypes.c_void_p(hwnd),ctypes.byref(owner))
  if owner.value==pid:user32.PostMessageW(ctypes.c_void_p(hwnd),0x10,0,0)
  return True
 user32.EnumWindows(visit,0)

with (OUT/'app.log').open('w') as log:
 app=subprocess.Popen([os.environ.get('ARCANEVR_QA_EXE',str(ROOT/'VRCraft.exe')),'--config',str(config),'--http-port','18826','--tracker-port','16970','--nexus-port','13501','--steam-port','19742','--qa-page','test'],cwd=ROOT,env=env,stdout=log,stderr=log)
 try:
  deadline=time.monotonic()+15
  while True:
   try:state();break
   except Exception:
    if time.monotonic()>deadline:raise
    time.sleep(.1)
  threading.Thread(target=fake_hmd,daemon=True).start()
  send(3,bytes(28)+bytes([3])+b'0.7'+bytes([2,0,0,0,0,1])+b'\0',hello=True)
  threading.Thread(target=tracker,daemon=True).start()
  until(lambda v:v['calibration_ready']==True,5);command({'command':'calibrate'})
  until(lambda v:v['actions']['preview']['calibrated']==True,3)
  checks={}; assert state()['armed']==False
  # Acceleration jump after a fresh neutral period.
  time.sleep(1.75);accel=[0.,5.,0.]
  v=until(lambda v:v['actions']['preview']['jump_pressed']==True,2);checks['jump']=v['actions']['preview'];accel=[0.,0.,0.];time.sleep(.9)
  head[0]=1.0;v=until(lambda v:v['actions']['preview']['crouching']==True,2);checks['crouch']=v['actions']['preview'];time.sleep(.5);head[0]=1.7
  until(lambda v:v['actions']['preview']['crouching']==False,2)
  # Chest acceleration drives continuous movement without assigned feet.
  c,s=math.cos(.15),math.sin(.15);pose=[c,-s,0.,0.];oscillate=True
  v=until(lambda v:v['locomotion']['preview']['speed']>0.02,3);checks['chest_steps']={'speed':v['locomotion']['preview']['speed'],'roles':v['roles'],'input_ready':v['locomotion']['input_ready']};time.sleep(.6);oscillate=False;accel=[0.,0.,0.]
  # Keep turn applied for one second; renderer accumulates and reports it after 10 seconds.
  pose=[1.,0.,0.,0.];head[1]=0.6
  v=until(lambda v:abs(v['actions']['preview']['turn_xy'][0])>0.1,2);checks['turn_xy']=v['actions']['preview']['turn_xy'];time.sleep(1.2);head[1]=0.
  tracker_on=False;v=until(lambda v:v['roles']['torso']['fresh']==False,3);time.sleep(.6)
  v=state();checks['stale']={'fresh':v['roles']['torso']['fresh'],'speed':v['locomotion']['preview']['speed'],'travel_xy':v['actions']['preview']['travel_xy']}
  assert v['armed']==False and v['actions']['preview']['travel_xy']==[0.,0.],checks['stale']
  deadline=time.monotonic()+18
  while not (OUT/'renderer.json').exists():
   if time.monotonic()>deadline:raise TimeoutError('renderer report')
   time.sleep(.1)
  render=json.loads((OUT/'renderer.json').read_text());embed=json.loads((OUT/'embed.json').read_text())
  assert embed['ws_child'] and embed['get_parent']==embed['parent_hwnd'] and embed['visible'],embed
  assert render['character_loaded'] and render['fixed_ground'] and render['live_frames']>0,render
  assert abs(render['virtual_turn_degrees'])>2,render
  assert 'Jump_Start' in render['clips_seen'] and 'Crouch_Idle_Loop' in render['clips_seen'],render
  assert render['speed_mps']==0 and render['reason'] in ('Chest stale','Input not ready','Reset Center required'),render
  result={'passed':True,'checks':checks,'renderer':render,'embed':embed,'physical_sensor_test':False,'headset_test':False,'game_output_armed':False}
  (OUT/'actions-qa.json').write_text(json.dumps(result,indent=2));print(json.dumps(result,indent=2))
 finally:
  halt.set();sock.close();driver.close();close_owned_window(app.pid);app.wait(timeout=12)
