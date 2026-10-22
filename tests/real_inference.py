import argparse,io,json,os,secrets,shutil,subprocess,sys,time,urllib.request,zipfile
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor
root=Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser(description='Full real-model offline integration test; takes several minutes')
parser.add_argument('--models-data',type=Path,required=True,help='Application data directory containing models/')
parser.add_argument('--python',type=Path,required=True,help='Configured Python runtime executable')
parser.add_argument('--work',type=Path,required=True,help='New empty directory for the test library and artifacts')
parser.add_argument('--compare-qwen',action='store_true',help='Also test Qwen 0.6B and 1.7B; both must be installed explicitly')
parser.add_argument('--cosy-runtime-data',type=Path,help='Optional already-verified runtime source, independent of model storage')
args=parser.parse_args();data=args.work.resolve();data.mkdir(parents=True,exist_ok=True)
if (data/'library.sqlite3').exists():raise SystemExit('Use a fresh --work directory; existing data will not be overwritten')
shutil.copytree(args.models_data.resolve()/'llama',data/'llama',dirs_exist_ok=True)
shutil.copytree((args.cosy_runtime_data or args.models_data).resolve()/'cosy',data/'cosy',dirs_exist_ok=True)
artifacts=data/'artifacts';artifacts.mkdir(exist_ok=True);python=args.python.resolve()
token=secrets.token_hex(32);process=None
guard="""import socket,runpy,sys
original=socket.socket.connect
def connect(self,address):
    if isinstance(address,tuple) and address[0] not in ('127.0.0.1','::1'):raise RuntimeError('External network disabled by integration test')
    return original(self,address)
socket.socket.connect=connect
runpy.run_path(sys.argv.pop(1),run_name='__main__')
"""
def start():
    global process,url
    process=subprocess.Popen([str(python),'-B','-c',guard,str(root/'backend/server.py'),'--root',str(root),'--data',str(data)],env=dict(os.environ,KAVYCHKA_TOKEN=token,PYTHONIOENCODING='utf-8'),stdout=subprocess.PIPE,stderr=(data/'server.log').open('a',encoding='utf-8'),text=True,encoding='utf-8')
    line=process.stdout.readline();assert line,'server failed'
    url='http://127.0.0.1:'+str(json.loads(line)['port'])
def stop():
    if process:process.terminate();process.wait(timeout=10)
def api(path,body=None,binary=False):
    req=urllib.request.Request(url+path,data=None if body is None else json.dumps(body).encode(),headers={'Authorization':'Bearer '+token,'Content-Type':'application/json'})
    with urllib.request.urlopen(req,timeout=120) as r:return r.read() if binary else json.load(r)
def job(kind,args):
    row=api('/jobs',{'kind':kind,'args':args});ident=row['id'];last=''
    if not ident:return row
    start_time=time.monotonic()
    while row['state'] not in ('done','failed','cancelled'):
        if row['message']!=last:print(kind,row['message'],flush=True);last=row['message']
        assert time.monotonic()-start_time<1800,'job timeout'
        time.sleep(1);row=api('/jobs/'+ident)
    assert row['state']=='done',row
    return row
try:
    start()
    models=args.models_data.resolve()/'models'
    api('/settings',{'batch_count':13,'swear':False,'tts_model':'tts-cosyvoice3','cosy_style':'confident','model_paths':{key:str(models/key) for key in ['tts-cosyvoice3','tts-0.6b','tts-1.7b','text-gemma4-12b','text-gemma4-e4b']}})
    started=time.monotonic();daily=job('daily',{'day':'2026-10-18'});daily_seconds=time.monotonic()-started
    rows=api('/library')['rows'];assert len(rows)==13 and len({r['normalized'] for r in rows})==13
    assert all(r['approved'] and r['audio_path'] for r in rows)
    sample=next(r for r in rows if r['category']=='lunch')
    wav=api('/audio/'+sample['id'],binary=True);assert wav[:4]==b'RIFF'
    assert not next(r for r in api('/library')['rows'] if r['id']==sample['id'])['used_at']
    api('/select',{'ids':[sample['id']],'action':'exclude'});assert api('/consume',{'category':'lunch','day':'2026-10-18'})['row'] is None
    api('/select',{'ids':[sample['id']],'action':'keep'})
    with ThreadPoolExecutor(max_workers=12) as pool:
        consumed=list(pool.map(lambda _:api('/consume',{'category':'break','day':'2026-10-18'})['row'],range(12)))
    ids=[r['id'] for r in consumed if r];assert len(ids)==len(set(ids))==9
    archive=api('/export',{'ids':[r['id'] for r in rows[:2]]},binary=True)
    with zipfile.ZipFile(io.BytesIO(archive)) as z:assert 'phrases.json' in z.namelist() and len(z.namelist())==3
    (artifacts/'daily-plan.zip').write_bytes(api('/export',{},binary=True))
    stop();start()
    assert api('/consume',{'category':'break','day':'2026-10-18'})['row'] is None
    assert api('/consume',{'category':'break','day':'2026-10-19'})['row'] is None
    repeat=api('/jobs',{'kind':'daily','args':{'day':'2026-10-18'}});assert repeat['id'] is None
    runtime=job('runtime',{'category':'break','day':'2026-10-18'})['result'];assert runtime['used_at'] and runtime['audio_path']
    (artifacts/'runtime-sample.wav').write_bytes(api('/audio/'+runtime['id'],binary=True))
    chosen=next(r for r in rows if r['category']=='exercise')
    (artifacts/'CosyVoice3.wav').write_bytes(api('/audio/'+chosen['id'],binary=True))
    assert chosen['voice_style']=='confident' and chosen['voice_instruction']
    api('/settings',{'cosy_style':'calm'})
    calmer=job('speech',{'id':chosen['id']})['result'];assert calmer['voice_style']=='calm' and calmer['cache_key']!=chosen['cache_key']
    (artifacts/'CosyVoice3-calm.wav').write_bytes(api('/audio/'+chosen['id'],binary=True))
    api('/settings',{'cosy_style':'auto'})
    automatic=job('runtime',{'category':'break','day':'2026-10-18'})['result'];assert automatic['voice_style'] in ('confident','calm','warm','amused') and automatic['voice_instruction']
    if args.compare_qwen:
        for key in ['tts-0.6b','tts-1.7b']:
            api('/settings',{'tts_model':key})
            bigger=job('speech',{'id':chosen['id']})['result'];assert bigger['model']==key and bigger['cache_key']!=chosen['cache_key'] and not bigger['voice_instruction']
            (artifacts/(key+'.wav')).write_bytes(api('/audio/'+chosen['id'],binary=True))
    report=dict(passed=True,day='2026-10-18',daily_count=13,nonrepeating_breaks=9,daily_seconds=daily_seconds,offline_external_connections_blocked=True,voice_model='tts-cosyvoice3',style_change=['confident','calm'],auto_style=automatic['voice_style'],runtime_text=runtime['text'],comparison_text=chosen['text'],texts=[r['text'] for r in rows])
    (artifacts/'integration-result.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
    print(json.dumps(report,ensure_ascii=False),flush=True)
finally:stop()
