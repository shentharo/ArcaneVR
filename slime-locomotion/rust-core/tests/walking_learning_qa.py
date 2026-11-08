"""Isolated synthetic chest/action exercise against the actual embedded renderer.
Run only after the owner installs the intended app and viewer builds. Never arms output.
"""
import ctypes, json, math, os, socket, struct, subprocess, threading, time, urllib.request
from pathlib import Path

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'release'/'walking-learning-qa'/time.strftime('qa-%Y%m%d-%H%M%S')
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
d=cfg['actions']['direction'];d.update(enabled=True,decouple=False,smoothing_ms=0,learned_forward=[-1.,0.,0.],learned_right=[0.,-1.,0.],pitch_sign=1.,roll_sign=1.)
cfg.pop('walking_model',None)
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
wobble=False
halt=threading.Event();tracker_on=True;seq=0;pose=[1.,0.,0.,0.];accel=[0.,0.,0.];head=[1.7,0.,0.,0.];oscillate=False

def send(kind,body,hello=False):
 global seq
 seq=0 if hello else seq+1;sock.sendto(struct.pack('>IQ',kind,seq)+body,('127.0.0.1',16970))
def tracker():
 while not halt.is_set():
  if tracker_on:
   w,x,y,z=pose
   if wobble:
    h=.055*math.sin(time.monotonic()*8.)
    c,b=math.cos(h),math.sin(h)
    w,x,y,z=c*w-b*y,c*x+b*z,c*y+b*w,c*z-b*x
   send(1,struct.pack('>ffff',x,y,z,w));a=list(accel)
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
  checks={};assert not state()['armed']
  pose=[math.cos(.2),-math.sin(.2),0.,0.];time.sleep(.5)
  v=state();assert v['game_output_preview']['travel_xy'][1]==0 and v['actions']['preview']['raw_direction_angles'][1]>0
  pose=[1.,0.,0.,0.];time.sleep(.3)
  command({'command':'walking_learn_start'})
  time.sleep(.6);assert state()['walking_learning']['active']
  assert 'Lean forward' in state()['walking_learning']['text']
  c,b=math.cos(.15),math.sin(.15);pose=[c,-b,0.,0.]
  until(lambda v:'Okay' in v['walking_learning'].get('text',''),3)
  wobble=True
  v=until(lambda v:v['walking_learning'].get('active')==False,13)
  assert 'saved' in v['walking_learning']['message'],v['walking_learning']
  settings=state()['actions']['settings']
  settings['direction']['sprint_full_forward']=True
  settings['direction']['sprint_angle']=3.1 # Lean cannot trigger sprint in this fixture.
  command({'command':'actions_settings','settings':settings})
  command({'command':'walking_full_speed','effort':2.0})
  v=until(lambda v:v['walking_speed']>0 and v['walking_effort']<1.2,3)
  assert v['game_output_preview']['buttons']&4==0
  command({'command':'walking_full_speed','effort':0.5})
  v=until(lambda v:v['walking_effort']>1.2 and v['game_output_preview']['buttons']&4!=0,3)
  checks['step_sprint_without_lean']={'effort':v['walking_effort'],'output':v['game_output_preview']}
  # Restart only the owned embedded viewer after learning so its report covers learned walking.
  subprocess.run(['powershell','-NoProfile','-Command',f"Get-CimInstance Win32_Process | Where-Object {{$_.ParentProcessId -eq {app.pid} -and $_.Name -eq 'arcane-test-room.exe'}} | ForEach-Object {{Stop-Process -Id $_.ProcessId}}"],check=True)
  (OUT/'renderer.json').unlink(missing_ok=True)
  deadline=time.monotonic()+22
  while not (OUT/'renderer.json').exists():
   assert not state()['armed']
   if time.monotonic()>deadline:raise TimeoutError('learned walking renderer report')
   time.sleep(.2)
  render=json.loads((OUT/'renderer.json').read_text())
  assert render['distance_m']>1 and render['local_input'][1]>0 and abs(render['local_input'][0])<1e-6,render
  assert render['max_sprint_speed']>0,render
  checks['renderer']=render
  v=until(lambda v:v['walking_speed']>.1,3);assert v['game_output_preview']['travel_xy'][1]>0,v['game_output_preview']
  assert abs(v['game_output_preview']['travel_xy'][0])<1e-6
  checks['learned']=v['walking_model'];checks['moving']=v['game_output_preview']
  wobble=False;v=until(lambda v:v['walking_speed']==0,3);time.sleep(.5);assert max(map(abs,state()['game_output_preview']['travel_xy']))<.001
  pose=[1.,0.,0.,0.];v=until(lambda v:max(map(abs,v['game_output_preview']['travel_xy']))<.001,3)
  assert state()['game_output_preview']['buttons']&4==0
  checks['stopped']=True
  assert json.loads(config.read_text())['walking_model']==checks['learned']
  command({'command':'walking_learn_start'});command({'command':'walking_learn_cancel'});assert state()['walking_model']==checks['learned']
  tracker_on=False;time.sleep(.5);assert state()['game_output_preview']['travel_xy']==[0.,0.]
  assert not state()['armed']
  print(json.dumps({'passed':True,'checks':checks,'physical_sensor_test':False},indent=2))
 finally:
  halt.set();sock.close();driver.close();close_owned_window(app.pid);app.wait(timeout=12)
