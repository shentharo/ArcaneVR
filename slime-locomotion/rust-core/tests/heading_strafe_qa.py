"""Real receiver/embedded renderer QA for optional lateral lean with feet-forward movement."""
from pathlib import Path
import argparse, json, math, os, socket, struct, subprocess, threading, time, urllib.request, urllib.error


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--exe',type=Path,required=True)
    parser.add_argument('--root',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    root,out,exe=args.root.resolve(),args.output.resolve(),args.exe.resolve()
    out.mkdir(parents=True,exist_ok=True)
    cfg=json.loads((root/'tracker-group.json').read_text(encoding='utf-8-sig'))
    ids=[f'127.0.0.{i}:0' for i in (2,3,4)]
    cfg.update(auto_add=False,trackers=dict(left=ids[0],right=ids[1],torso=ids[2]),tracker_identities={v:f'02:00:00:00:00:{i+2:02X}' for i,v in enumerate(ids)},sensor_layout={'count':3},movement_engine='chest_heading_steps',step_input='feet')
    cfg['detection'].update(heading_axis='z',acceleration_mode='linear')
    cfg['locomotion']['algorithm']='cadence'
    cfg['actions']['direction'].update(enabled=False,strafe_enabled=False,heading_strafe_enabled=False,sprint_full_forward=False,max_output=1.,max_angle=.4,learned_forward=[1.,0.,0.],learned_right=[0.,1.,0.],decouple=False,pitch_sign=1.,roll_sign=1.,smoothing_ms=0.)
    cfg['mounting_calibration']={'learned_identity':'02:00:00:00:00:04:0','devices':{}}
    config=out/'config.json';config.write_text(json.dumps(cfg))
    base='http://127.0.0.1:18883/api/'
    def req(name,data=None):
        try:
            return json.load(urllib.request.urlopen(urllib.request.Request(base+name,data=None if data is None else json.dumps(data).encode(),headers={'Content-Type':'application/json'}),timeout=2))
        except urllib.error.HTTPError as error:
            raise AssertionError(error.read().decode()) from error
    def state():
        v=req('state');assert not v['armed'];return v
    def set_gate(enabled):
        settings=state()['actions']['settings']
        settings['direction']['heading_strafe_enabled']=enabled
        assert req('command',{'command':'actions_settings','settings':settings})['ok']
        time.sleep(.15)
        current=state()
        assert current['movement_engine']=='chest_heading_steps'
        assert not current['actions']['settings']['direction']['enabled']
        assert not current['actions']['settings']['direction']['strafe_enabled']
        assert current['actions']['settings']['direction']['heading_strafe_enabled']==enabled
        assert current['actions']['preview']['calibrated']
        saved=json.loads(config.read_text())
        assert saved['actions']['direction']['heading_strafe_enabled']==enabled
        assert saved['trackers']==cfg['trackers']
    socks=[]
    for n in (2,3,4):
        sock=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);sock.bind((f'127.0.0.{n}',0));socks.append(sock)
    chest=[0.,0.,0.];walking=[False];stop=threading.Event();ready=threading.Event()
    def stream():
        ready.wait();seq=1
        for i,sock in enumerate(socks):sock.sendto(struct.pack('>IQ',3,0)+bytes(28)+bytes([3])+b'0.7'+bytes([2,0,0,0,0,i+2])+b'\0',('127.0.0.1',16993))
        while not stop.is_set():
            now=time.monotonic()
            for i,sock in enumerate(socks):
                if i==2:
                    x,y,z=chest;angle=math.sqrt(x*x+y*y+z*z);factor=math.sin(angle/2)/angle if angle else 0.
                    q=(x*factor,y*factor,z*factor,math.cos(angle/2))
                else:
                    a=.16*(1+math.sin(now*math.pi*3.5+(0 if i==0 else math.pi))) if walking[0] else 0.
                    q=(math.sin(a/2),0.,0.,math.cos(a/2))
                sock.sendto(struct.pack('>IQffff',1,seq,*q),('127.0.0.1',16993));seq+=1
                accel=4*max(0,math.sin(now*math.pi*3.5+(0 if i==0 else math.pi))) if walking[0] and i<2 else 0.
                sock.sendto(struct.pack('>IQfffB',4,seq,accel,0.,0.,0),('127.0.0.1',16993));seq+=1
            time.sleep(.01)
    worker=threading.Thread(target=stream);worker.start()
    env=dict(os.environ,ARCANEVR_EMBED_QA='1',ARCANEVR_VIEWER_QA_REPORT=str(out/'renderer.json'),ARCANEVR_VIEWER_QA_SERIES=str(out/'series.jsonl'),ARCANEVR_VIEWER_QA_REPORT_AFTER='22',ARCANEVR_EMBED_QA_REPORT=str(out/'embed.json'))
    with (out/'app.log').open('w') as log:
        app=subprocess.Popen([str(exe),'--config',str(config),'--http-port','18883','--tracker-port','16993','--tracker-bind','127.0.0.1','--nexus-port','13533','--steam-port','19783','--qa-page','test-preview'],cwd=root,env=env,stdout=log,stderr=log,creationflags=subprocess.CREATE_NO_WINDOW)
        try:
            deadline=time.monotonic()+15
            while True:
                try: state();break
                except OSError:
                    if time.monotonic()>deadline:raise
                    time.sleep(.1)
            ready.set()
            while not state()['calibration_ready']:
                if time.monotonic()>deadline:raise AssertionError('Synthetic trackers did not become ready')
                time.sleep(.1)
            assert req('command',{'command':'calibrate'})['ok'];walking[0]=True;time.sleep(4)
            checks=[]
            for enabled,name,pose,sign in [(False,'off-right',[0.,.5,0.],0),(True,'on-right',[0.,.5,0.],1),(True,'on-left',[0.,-.5,0.],-1),(True,'forward-lean',[.5,0.,0.],0),(True,'backward-lean',[-.5,0.,0.],0),(False,'off-again',[0.,.5,0.],0)]:
                set_gate(enabled);chest[:]=pose;time.sleep(.85);v=state();xy=v['game_output_preview']['travel_xy']
                assert (abs(xy[0])<.001 if sign==0 else xy[0]*sign>.2),(name,xy,v['actions']['preview'])
                assert xy[1]>.1 and math.hypot(*xy)<=1.00001,(name,xy)
                assert not v['game_output_preview']['buttons']&4,(name,'unexpected sprint')
                checks.append({'phase':name,'travel_xy':xy})
            set_gate(True);chest[:]=[0.,.5,0.];time.sleep(.5)
            walking[0]=False;chest[:]=[0.,0.,0.];time.sleep(2)
            assert state()['game_output_preview']['travel_xy']==[0.,0.]
            for i in range(121):
                chest[:]=[0.,0.,2*math.pi*i/120];time.sleep(.02)
            chest[:]=[0.,0.,0.];time.sleep(.3)
            assert state()['game_output_preview']['travel_xy']==[0.,0.]
            set_gate(False)
            end=time.monotonic()+15
            while not (out/'renderer.json').exists():
                if time.monotonic()>end:raise TimeoutError('renderer report')
                time.sleep(.2)
            render=json.loads((out/'renderer.json').read_text())
            assert render['distance_m']>1,render
            assert abs(render['rotation_degrees'])>350,render
            series=[json.loads(line) for line in (out/'series.jsonl').read_text().splitlines()]
            # Actual translation is reported by the renderer, not inferred from its animation labels.
            positions=[row['position'] for row in series]
            assert max(p[0] for p in positions)-min(p[0] for p in positions)>.2,positions
            assert all(any(side in name for name in render['animations_seen']) for side in ('Left','Right')),render
            result={'passed':True,'phases':checks,'renderer_distance_m':render['distance_m'],'actual_lateral_translation':True,'stationary_rotation':True,'feet_stop':True,'game_output_armed':False,'physical_hardware':'not tested'}
            (out/'result.json').write_text(json.dumps(result,indent=2));print(json.dumps(result,indent=2))
        finally:
            stop.set();ready.set();worker.join();[s.close() for s in socks]
            app.terminate();app.wait(timeout=10)

if __name__=='__main__':main()
