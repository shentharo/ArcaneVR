"""Isolated receiver and renderer pivot/walk QA; never arms game output."""
import argparse,json,math,os,socket,struct,subprocess,threading,time,urllib.request
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True);p.add_argument('--capture',action='store_true');a=p.parse_args()
root=Path(r'C:\Users\Nexpo\Desktop\KATVR');out=a.output.resolve();out.mkdir(parents=True,exist_ok=True)
cfg=json.loads((root/'tracker-group.json').read_text(encoding='utf-8-sig'))
ids=[f'127.0.0.{i}:0' for i in (2,3,4)]
cfg.update(auto_add=False,trackers=dict(left=ids[0],right=ids[1],torso=ids[2]),tracker_identities={v:f'02:00:00:00:00:{i+2:02X}' for i,v in enumerate(ids)},sensor_layout={'count':3},movement_engine='chest_heading_steps',step_input='feet',foot_motion={'enabled':True,'min_swing_degrees':2.,'pivot_ratio':2.})
cfg['detection'].update(heading_axis='z',acceleration_mode='linear')
cfg['locomotion'].update(algorithm='cadence',revision=2)
cfg['actions']['direction'].update(enabled=False,strafe_enabled=False,heading_strafe_enabled=False,sprint_full_forward=False,max_output=1.,learned_forward=[1.,0.,0.],learned_right=[0.,1.,0.],decouple=False)
cfg['mounting_calibration']={'learned_identity':'02:00:00:00:00:04:0','devices':{}}
config=out/'config.json';config.write_text(json.dumps(cfg));base='http://127.0.0.1:18884/api/'
def req(name,data=None):return json.load(urllib.request.urlopen(urllib.request.Request(base+name,data=None if data is None else json.dumps(data).encode(),headers={'Content-Type':'application/json'}),timeout=2))
def state():
 v=req('state');assert not v['armed'];return v
socks=[]
for n in (2,3,4):
 s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);s.bind((f'127.0.0.{n}',0));socks.append(s)
stop=threading.Event();ready=threading.Event();phase=['still'];origin=[time.monotonic()]
def stream():
 ready.wait();seq=1
 for i,s in enumerate(socks):s.sendto(struct.pack('>IQ',3,0)+bytes(28)+bytes([3])+b'0.7'+bytes([2,0,0,0,0,i+2])+b'\0',('127.0.0.1',16994))
 while not stop.is_set():
  t=time.monotonic()-origin[0];mode=phase[0]
  for i,s in enumerate(socks):
   tilt=yaw=acc=0.
   if mode=='tiny' and i<2:tilt=.005*math.sin(t*50+i);acc=4*max(0,math.sin(t*12+i*math.pi))
   if mode=='pivot':
    yaw=t*2.3
    if i<2:tilt=.03*(1+math.sin(t*10+i*math.pi));acc=5*max(0,math.sin(t*10+i*math.pi))
   if mode in ('slow','run','curved'):
    rate=1.2 if mode=='slow' else 3.5
    yaw=t*.35 if mode=='curved' else 0.
    if i<2:tilt=(.14 if mode=='slow' else .3)*(1+math.sin(t*math.pi*rate+i*math.pi));acc=(3. if mode=='slow' else 5.)*max(0,math.sin(t*math.pi*rate+i*math.pi))
   c,d=math.cos(yaw/2),math.sin(yaw/2);u,v=math.cos(tilt/2),math.sin(tilt/2)
   s.sendto(struct.pack('>IQffff',1,seq,c*v,d*v,d*u,c*u),('127.0.0.1',16994));seq+=1
   s.sendto(struct.pack('>IQfffB',4,seq,acc,0.,0.,0),('127.0.0.1',16994));seq+=1
  time.sleep(.008)
worker=threading.Thread(target=stream);worker.start();exe=root/'slime-locomotion/rust-core/target/release/vrcraft.exe'
env=dict(os.environ,ARCANEVR_EMBED_QA='1',ARCANEVR_EMBED_QA_REPORT=str(out/'embedding.json'),ARCANEVR_VIEWER_QA_REPORT=str(out/'renderer.json'),ARCANEVR_VIEWER_QA_SERIES=str(out/'series.jsonl'),ARCANEVR_VIEWER_QA_REPORT_AFTER='24',ARCANEVR_VIEWER_QA_SCREENSHOT=str(out/'viewer.png'),ARCANEVR_VIEWER_QA_CAPTURE_AT='11')
cmd=[str(exe),'--config',str(config),'--http-port','18884','--tracker-port','16994','--tracker-bind','127.0.0.1','--nexus-port','13534','--steam-port','19784','--qa-page','test-preview']
if a.capture:cmd+=['--qa-foot-motion-open','--qa-screenshot',str(out/'app.png')]
with (out/'app.log').open('w') as log:
 app=subprocess.Popen(cmd,cwd=root,env=env,stdout=log,stderr=log,creationflags=subprocess.CREATE_NO_WINDOW)
 try:
  deadline=time.monotonic()+15
  while True:
   try:state();break
   except OSError:
    if time.monotonic()>deadline:raise
    time.sleep(.1)
  ready.set()
  while not state()['calibration_ready']:
   if time.monotonic()>deadline:raise AssertionError('synthetic tracker readiness')
   time.sleep(.1)
  time.sleep(.5);assert req('command',{'command':'calibrate'})['ok'];time.sleep(.5)
  if a.capture:
   phase[0]='pivot';origin[0]=time.monotonic();assert app.wait(timeout=25)==0
   print(json.dumps({'capture':str(out/'app.png'),'physical_hardware':False}));raise SystemExit(0)
  checks=[]
  def sample(mode,duration,settle=.3):
   phase[0]=mode;origin[0]=time.monotonic();time.sleep(settle);start=time.monotonic();values=[]
   while time.monotonic()-start<duration:
    v=state();xy=v['game_output_preview']['travel_xy'];values.append(math.hypot(*xy));assert min(xy)>-1e-6 and max(xy)<=1.0001
    time.sleep(.04)
   checks.append({'mode':mode,'mean':sum(values)/len(values),'max':max(values)})
   return values
  assert max(sample('tiny',1.2))<.001,checks
  assert max(sample('pivot',1.8))<.001,checks
  slow=sample('slow',2.4,.8);assert .08<sum(slow)/len(slow)<.55,checks
  run=sample('run',2.4,1.3);assert sum(run)/len(run)>.8,checks
  curved=sample('curved',1.5,.8);assert sum(curved)/len(curved)>.8,checks
  assert max(sample('pivot',.5,.45))<.001,checks
  # Dedicated live command changes only this filter; session remains calibrated.
  before=state();settings=before['foot_motion']['settings'];settings['enabled']=False
  assert req('command',{'command':'foot_motion_settings','settings':settings})['ok'];after=state()
  assert after['calibrated'] and after['actions']['preview']['calibrated']
  assert after['steamvr']['generation']==before['steamvr']['generation']
  assert after['roles']['torso']['id']==before['roles']['torso']['id']
  legacy=sample('pivot',1.5,.2);assert max(legacy)>.1,checks
  settings['enabled']=True;assert req('command',{'command':'foot_motion_settings','settings':settings})['ok']
  assert max(sample('pivot',.5,.45))<.001,checks
  assert max(sample('still',.4,.3))<.001,checks
  end=time.monotonic()+15
  while not (out/'renderer.json').exists():
   if time.monotonic()>end:raise TimeoutError('renderer report')
   time.sleep(.1)
  render=json.loads((out/'renderer.json').read_text());assert render['distance_m']>1 and render['max_walk_speed']>1.6,render
  rows=[json.loads(s) for s in (out/'series.jsonl').read_text().splitlines()]
  assert any(b['position']!=a['position'] for a,b in zip(rows,rows[1:])),rows
  still_pairs=[(a,b) for a,b in zip(rows,rows[1:]) if a['speed_mps']<.001 and b['speed_mps']<.001 and a['reason']==b['reason']=='Live']
  assert len(still_pairs)>3
  assert all(math.dist(a['position'],b['position'])<.001 for a,b in still_pairs),'Zero input must produce stationary rendered positions'
  print(json.dumps({'passed':True,'phases':checks,'actual_renderer_distance':render['distance_m'],'live_apply_preserved_session':True,'game_output_armed':False,'physical_hardware':'not tested'},indent=2))
 finally:
  stop.set();ready.set();worker.join();[s.close() for s in socks]
  if app.poll() is None:app.terminate();app.wait(timeout=10)
