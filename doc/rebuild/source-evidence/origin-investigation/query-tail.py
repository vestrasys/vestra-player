import json, socket, subprocess, time
from pathlib import Path
ROOT=Path(__file__).parent
FIXTURE='/var/folders/c9/l6fw8b9n69d_4030n4dcj2nh0000gn/T/vestra-native-burst-prepare-6eX4mS/fixtures/origin2.mkv'
results=[]
for demux in ['lavf']:
 for rebase in ['yes']:
  tag='tail'
  sockpath=str(ROOT/'tail.sock')
  log=open(ROOT/f'{tag}.log','wb')
  args=['mpv','--no-config','--idle=yes','--pause','--vo=null','--ao=null','--no-terminal','--input-ipc-server='+sockpath,'--demuxer='+demux,'--rebase-start-time='+rebase,'--msg-level=all=v','--demuxer-lavf-probe-info=yes','--keep-open=yes','--speed=100']
  p=subprocess.Popen(args,stdout=log,stderr=subprocess.STDOUT)
  conn=socket.socket(socket.AF_UNIX)
  try:
   for _ in range(100):
    try: conn.connect(sockpath); break
    except (FileNotFoundError,ConnectionRefusedError): time.sleep(.03)
   conn.settimeout(5)
   f=conn.makefile('rwb',buffering=0)
   n=0
   def command(*cmd):
    global n
    n+=1
    f.write((json.dumps({'command':list(cmd),'request_id':n})+'\n').encode())
    while True:
     item=json.loads(f.readline())
     if item.get('request_id')==n: return item
   def get(key): return command('get_property',key)
   command('loadfile',FIXTURE)
   for _ in range(100):
    if get('time-pos').get('data') is not None: break
    time.sleep(.03)
   keys=['current-demuxer','options/rebase-start-time','demuxer-start-time','time-pos','playback-time','duration','percent-pos','track-list']
   initial={key:get(key) for key in keys}
   seeks=[]
   for t in [7,7.96,8,9,10]:
    response=command('seek',t,'absolute+exact')
    time.sleep(.15)
    seeks.append({'requested':t,'response':response,'time':get('time-pos'),'playbackTime':get('playback-time')})
   command('seek',0,'absolute+exact')
   command('set_property','pause',False)
   for _ in range(200):
    if get('eof-reached').get('data') is True: break
    time.sleep(.02)
   eof={key:get(key) for key in ['time-pos','playback-time','duration','eof-reached','pause','demuxer-start-time']}
   results.append({'tag':tag,'args':args,'initial':initial,'seeks':seeks,'eof':eof})
   command('quit')
  finally:
   conn.close()
   try:p.wait(timeout=3)
   except subprocess.TimeoutExpired:p.kill();p.wait()
   log.close()
   Path(sockpath).unlink(missing_ok=True)
(ROOT/'results-tail.json').write_text(json.dumps(results,indent=2))
for row in results:
 print(row['tag'],{k:v.get('data') for k,v in row['initial'].items() if k!='track-list'},[(s['requested'],s['time'].get('data')) for s in row['seeks']])

print('EOF',json.dumps(results[0]['eof']))
