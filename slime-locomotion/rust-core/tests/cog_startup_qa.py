"""Measure the actual Home cog destination while viewer startup is delayed."""
from pathlib import Path
import argparse,json,os,subprocess
p=argparse.ArgumentParser(description=__doc__);p.add_argument('--exe',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args()
exe,out=a.exe.resolve(),a.output.resolve();out.mkdir(parents=True,exist_ok=True)
report,capture=out/'cog.jsonl',out/'first-destination.png'
assert not report.exists(),'Use a fresh output directory'
env=dict(os.environ,ARCANEVR_EMBED_QA='1',ARCANEVR_COG_QA_REPORT=str(report),ARCANEVR_COG_QA_START_DELAY_MS='3000')
with (out/'app.log').open('w') as log:
 app=subprocess.Popen([str(exe),'--config',str(out/'config.json'),'--http-port','18892','--tracker-port','16998','--tracker-bind','127.0.0.1','--nexus-port','13540','--steam-port','19792','--qa-page','home-compact','--qa-cog','--qa-screenshot',str(capture)],cwd=exe.parent,env=env,stdout=log,stderr=log,creationflags=subprocess.CREATE_NO_WINDOW)
 try:assert app.wait(timeout=20)==0
 finally:
  if app.poll() is None:app.terminate();app.wait(timeout=10)
rows=[json.loads(line) for line in report.read_text().splitlines()]
frame=next(row['details'] for row in rows if row['event']=='first_test_ui_frame')
assert frame['startup_pending'] and not frame['embedded_ready'],frame
assert frame['action_to_ui_ms']<250 and frame['ui_update_ms']<250,frame
assert capture.is_file()
print(json.dumps({'passed':True,'deliberate_viewer_delay_ms':3000,**frame,'capture':str(capture)},indent=2))
