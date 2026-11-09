"""Actual isolated receiver head-sweep recenter and embedded foot-epoch QA."""
import argparse,json,math,os,socket,struct,subprocess,threading,time,urllib.request
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True);p.add_argument('--capture',action='store_true');a=p.parse_args()
root=Path(r'C:\Users\Nexpo\Desktop\KATVR');out=a.output.resolve();out.mkdir(parents=True,exist_ok=True)
cfg=json.loads((root/'tracker-group.json').read_text(encoding='utf-8-sig'));ids=[f'127.0.0.{i}:0' for i in (2,3,4)]
cfg.update(auto_add=False,trackers=dict(left=ids[0],right=ids[1],torso=ids[2]),tracker_identities={v:f'02:00:00:00:00:{i+2:02X}' for i,v in enumerate(ids)},sensor_layout={'count':3},movement_engine='chest_heading_steps',step_input='feet',head_recenter={'enabled':False})
cfg['detection'].update(heading_axis='z',acceleration_mode='linear');cfg['locomotion'].update(algorithm='cadence',revision=2)
cfg['actions']['direction'].update(enabled=False,heading_strafe_enabled=False,sprint_full_forward=False,decouple=False,learned_forward=[1.,0.,0.],learned_right=[0.,1.,0.])
cfg['actions']['turn'].update(enabled=True,horizontal=True,vertical=False,deadzone=.01,max_angle=1.,exponent=1.)
cfg['mounting_calibration']={'learned_identity':'02:00:00:00:00:04:0','devices':{}}
config=out/'config.json';config.write_text(json.dumps(cfg));base='http://127.0.0.1:18886/api/'
def req(name,data=None):return json.load(urllib.request.urlopen(urllib.request.Request(base+name,data=None if data is None else json.dumps(data).encode(),headers={'Content-Type':'application/json'}),timeout=2))
def state():
 v=req('state');assert not v['armed'];return v
stop=threading.Event();ready=threading.Event();head=[2.9];chest=[.6]
socks=[]
for n in (2,3,4):
 s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);s.bind((f'127.0.0.{n}',0));socks.append(s)
driver=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);driver.bind(('127.0.0.1',19786));driver.settimeout(.1);packets=[]
def hmd():
 while not stop.is_set():
  try:packet,peer=driver.recvfrom(2048)
  except socket.timeout:continue
  if len(packet)<12:continue
  if packet[:4]==b'ALC2':
   assert packet[12:32]==bytes(20),'Unarmed QA must send neutral game axes/buttons'
  packets.append(packet)
  yaw=math.atan2(math.sin(head[0]),math.cos(head[0]))
  driver.sendto(b'AKV2'+packet[4:12]+struct.pack('<Iffff',7,1.7,yaw,0.,0.),peer)
def sensors():
 ready.wait();seq=1
 for i,s in enumerate(socks):s.sendto(struct.pack('>IQ',3,0)+bytes(28)+bytes([3])+b'0.7'+bytes([2,0,0,0,0,i+2])+b'\0',('127.0.0.1',16996))
 while not stop.is_set():
  for i,s in enumerate(socks):
   q=(math.sin(.1+i*.02),0.,0.,math.cos(.1+i*.02)) if i<2 else (0.,0.,math.sin(chest[0]/2),math.cos(chest[0]/2))
   s.sendto(struct.pack('>IQffff',1,seq,*q),('127.0.0.1',16996));seq+=1
   s.sendto(struct.pack('>IQfffB',4,seq,0.,0.,0.,0),('127.0.0.1',16996));seq+=1
  time.sleep(.008)
workers=[threading.Thread(target=hmd),threading.Thread(target=sensors)]
for w in workers:w.start()
exe=root/'slime-locomotion/rust-core/target/release/vrcraft.exe';env=dict(os.environ,ARCANEVR_EMBED_QA='1',ARCANEVR_EMBED_QA_REPORT=str(out/'embedding.json'),ARCANEVR_VIEWER_QA_REPORT=str(out/'renderer.json'),ARCANEVR_VIEWER_QA_SERIES=str(out/'series.jsonl'),ARCANEVR_VIEWER_QA_REPORT_AFTER='19',ARCANEVR_VIEWER_QA_SCREENSHOT=str(out/'viewer.png'),ARCANEVR_VIEWER_QA_CAPTURE_AT='13')
cmd=[str(exe),'--config',str(config),'--http-port','18886','--tracker-port','16996','--tracker-bind','127.0.0.1','--nexus-port','13536','--steam-port','19786','--qa-page','test-preview']
if a.capture:cmd+=['--qa-screenshot',str(out/'app.png')]
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
   if time.monotonic()>deadline:raise AssertionError('sensor readiness')
   time.sleep(.1)
  time.sleep(.5);assert req('command',{'command':'calibrate'})['ok'];time.sleep(.5)
  before=state();epoch=before['calibration_epoch'];generation=before['steamvr']['generation']
  assert req('command',{'command':'head_recenter_settings','settings':{'enabled':True}})['ok'];time.sleep(.8)
  v=state();assert v['calibration_epoch']==epoch and v['steamvr']['generation']==generation
  def go(relative,duration=.5,hold=.16):
   initial=head[0];target=2.9+math.radians(relative);steps=max(1,round(duration/.01))
   for i in range(1,steps+1):head[0]=initial+(target-initial)*i/steps;time.sleep(duration/steps)
   time.sleep(hold)
  count=2 if a.capture else 3
  for _ in range(count):go(-90);go(90)
  go(0,hold=.7);v=state()
  assert v['calibration_epoch']==epoch,(v['head_recenter'],v['calibration_epoch'])
  assert v['head_recenter']['completed_cycles']==count,v['head_recenter']
  if a.capture:
   assert app.wait(timeout=20)==0;print(json.dumps({'capture':str(out/'app.png'),'completed_pairs':count,'game_output_armed':False}));raise SystemExit(0)
  go(-90);go(90,hold=.35);assert state()['calibration_epoch']==epoch,'Cannot calibrate at side endpoint'
  chest[0]=.63;go(math.degrees(.1),hold=1.)
  v=state();assert v['calibration_epoch']==epoch+1,v['head_recenter']
  assert v['calibrated'] and v['actions']['preview']['calibrated'] and not v['armed']
  assert max(map(abs,v['actions']['preview']['turn_xy']))<.001,v['actions']['preview']
  assert abs(v['actions']['preview']['chest_yaw'])<.001,v['actions']['preview']
  assert v['head_recenter']['last_completed'] is not None
  assert {role:v['roles'][role]['id'] for role in ('left','right','torso')}=={role:before['roles'][role]['id'] for role in ('left','right','torso')}
  for key in ('learned_forward','learned_right','pitch_axis','roll_axis','pitch_sign','roll_sign'):
   assert v['actions']['settings']['direction'][key]==before['actions']['settings']['direction'][key],key
  end=time.monotonic()+15
  while not (out/'renderer.json').exists():
   if time.monotonic()>end:raise TimeoutError('renderer report')
   time.sleep(.1)
  render=json.loads((out/'renderer.json').read_text());assert render['foot_calibration_epoch']==epoch+1,render
  assert all(abs(x)<.001 for x in render['foot_pose']),render
  assert render['distance_m']<.001,render
  assert packets and all(p[:4]!=b'ALC2' or p[12:32]==bytes(20) for p in packets)
  result={'passed':True,'three_pairs_no_trigger':True,'side_endpoint_no_trigger':True,'four_pairs_forward_dwell_recentered':True,'yaw_wrap_tested':True,'head_and_chest_zeroed':True,'feet_renderer_epoch':render['foot_calibration_epoch'],'disarmed_session_remained_disarmed':True,'neutral_driver_packets':len(packets),'physical_hardware':'not tested'}
  (out/'result.json').write_text(json.dumps(result,indent=2));print(json.dumps(result,indent=2))
 finally:
  stop.set();ready.set()
  for w in workers:w.join(timeout=2)
  for s in socks:s.close()
  driver.close()
  if app.poll() is None:app.terminate();app.wait(timeout=10)
