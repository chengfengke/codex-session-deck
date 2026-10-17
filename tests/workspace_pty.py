#!/usr/bin/env python3
"""Actual terminal: hide/restore side, full-message search context, live task navigation."""
import errno,fcntl,json,os,pathlib,pty,select,signal,socket,sqlite3,struct,subprocess,sys,tempfile,termios,time
ROOT=pathlib.Path(__file__).resolve().parents[1]
BINARY=pathlib.Path(sys.argv[1] if len(sys.argv)>1 else ROOT/'target/debug/codex-deck').resolve()

class IPC:
    def __init__(self,state):
        self.s=socket.socket(socket.AF_UNIX);self.s.settimeout(10);self.s.connect(str(state/'runtime.sock'))
        self.f=self.s.makefile('rwb');self.f.readline();self.seq=0
    def call(self,op,**args):
        self.seq+=1;self.f.write((json.dumps({'id':self.seq,'op':op,'args':args})+'\n').encode());self.f.flush()
        while True:
            v=json.loads(self.f.readline())
            if v.get('type')=='response' and v.get('id')==self.seq:
                assert 'error' not in v,v.get('error')
                return v['result']
    def close(self):self.f.close();self.s.close()

with tempfile.TemporaryDirectory(prefix='sd-workspace-') as tmp:
    home=pathlib.Path(tmp);state=home/'state';state.mkdir()
    def chat(key,title,parent=None,**extra):
        return {'id':key,'title':title,'cwd':str(home/key),'parent':parent,'readonly':True,
            'messages':[{'id':key+'-message','role':'Codex','text':key.upper()+'_BODY'}],**extra}
    layout={'Split':{'vertical':False,'ratio':600,'first':{'Tabs':{'ids':['local-main'],'active':0}},'second':{'Tabs':{'ids':['local-side'],'active':0}}}}
    drafts={'local-main':{'text':'KEEP_MAIN_DRAFT','cursor':15},'local-side':{'text':'KEEP_SIDE_DRAFT','cursor':15}}
    data={'conversations':{'local-main':chat('local-main','Main A'),
        'local-side':chat('local-side','Side A','local-main'),
        'local-queue':chat('local-queue','Held queue',queued=[{'id':'held-1','text':'QUEUED_TEXT','cwd':str(home/'local-queue'),'readonly':True,'state':'held'}]),
        'local-failed':chat('local-failed','Failure fixture',status='failed')},
        'projects':[],'drafts':drafts,'layouts':{'local-main':layout},'active':'local-main','sidebar':True,'sidebar_width':26}
    with sqlite3.connect(state/'state.sqlite') as db:
        db.execute('CREATE TABLE state(id INTEGER PRIMARY KEY,version INTEGER NOT NULL,payload TEXT NOT NULL)')
        db.execute('INSERT INTO state VALUES(1,1,?)',(json.dumps(data),))
    env=os.environ|{'TERM':'xterm-256color','CODEX_DECK_HOME':str(state),'CODEX_DECK_CODEX':str(ROOT/'tests/fake_codex.py'),
        'FAKE_CODEX_SEARCH_HISTORY':'1','FAKE_CODEX_TURN_DELAY':'60','FAKE_CODEX_LOG':str(home/'rpc.jsonl'),
        'PATH':str(pathlib.Path(sys.executable).parent)+':'+os.environ.get('PATH','')}
    with open(home/'daemon.log','w') as log:
        daemon=subprocess.Popen([str(BINARY),'daemon'],env=env,stdout=log,stderr=log)
        client=None;pid=None;fd=None;finished=False;output=bytearray()
        try:
            for _ in range(150):
                if (state/'runtime.sock').exists():break
                assert daemon.poll() is None;time.sleep(.03)
            client=IPC(state)
            def wait(predicate):
                end=time.monotonic()+12
                while time.monotonic()<end:
                    if fd is not None:read_for(.025)
                    s=client.call('snapshot')
                    if predicate(s):return s
                    time.sleep(.025)
                raise AssertionError('Timed out; terminal='+output[-3000:].decode(errors='replace'))
            wait(lambda s:s['connected'])
            workers=[]
            for title,prompt in [('Worker fixture','keep running'),('Attention fixture','approval requested')]:
                cwd=home/title.split()[0];cwd.mkdir()
                key=client.call('prepare',cwd=str(cwd),title=title,readonly=True)['thread']
                client.call('send',thread=key,text=prompt)
                s=wait(lambda s:key in s['data']['aliases'])
                workers.append(s['data']['aliases'][key])
            running,attention=workers
            wait(lambda s:any(p['thread']==attention for p in s['pending']))
            pid,fd=pty.fork()
            if pid==0:os.execve(str(BINARY),[str(BINARY)],env)
            fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack('HHHH',40,160,0,0));output=bytearray()
            def read_for(seconds):
                end=time.monotonic()+seconds
                while time.monotonic()<end:
                    if select.select([fd],[],[],.015)[0]:
                        try:output.extend(os.read(fd,65536))
                        except OSError as e:
                            if e.errno==errno.EIO:return
                            raise
            def command(text):os.write(fd,b'\x07:'+text.encode()+b'\r');read_for(.12)
            read_for(.4)
            assert b'LOCAL-MAIN_BODY' in output and b'LOCAL-SIDE_BODY' in output
            os.write(fd,b'\x07o');s=wait(lambda s:'local-main' in s['data']['collapsed_side_panels'])
            assert s['data']['layouts']['local-main']==layout and s['data']['drafts']==drafts
            os.write(fd,b'\x07o');wait(lambda s:'local-main' not in s['data']['collapsed_side_panels'])
            os.write(fd,b'\x07l\x07w');s=wait(lambda s:'local-main' in s['data']['collapsed_side_panels'])
            assert 'local-side' in s['data']['conversations'] and s['data']['drafts']['local-side']==drafts['local-side']
            os.write(fd,b'\x07pSide A\r');wait(lambda s:'local-main' not in s['data']['collapsed_side_panels'])
            # Toolbar toggles also preserve the saved side layout and its drafts.
            for hidden in [True,False]:
                os.write(fd,b'\x1b[<0;34;2M\x1b[<0;34;2m')
                wait(lambda s:('local-main' in s['data']['collapsed_side_panels'])==hidden)
            command('find hidden-needle')
            s=wait(lambda s:s['search']['query']=='hidden-needle' and not s['search']['running'])
            assert len(s['search']['hits'])==3 and not s['search']['errors']
            archived=next(i for i,h in enumerate(s['search']['hits']) if h['archived'])
            target=s['search']['hits'][archived]['thread']
            os.write(fd,b'\x1b[B'*archived+b'\r');s=wait(lambda s:s['data']['active']==target)
            read_for(.15)
            assert b'Search context' in output and b'Hidden-NEEDLE' in output
            assert not any('Hidden-NEEDLE' in m['text'] for m in s['data']['conversations'][target]['messages']),'search replaced the live cache'
            command('live')
            assert s['data']['drafts']['local-main']==drafts['local-main']
            for keys,target in [(b'\tWorker',running),(b'\t\tHeld','local-queue'),(b'\t\t\tAttention',attention),
                                (b'\t\t\t\tFailure','local-failed')]:
                os.write(fd,b'\x07t');read_for(.12);os.write(fd,keys+b'\r')
                wait(lambda s:s['data']['active']==target)
            os.write(fd,b'\x07t');read_for(.1);os.write(fd,b'Attention\x1bOQ');read_for(.15)
            assert b'Request' in output
            os.write(fd,b'\x1b');read_for(.06)
            for rows,cols in [(24,80),(24,100),(40,160)]:
                for shortcut in [b'\x07/',b'\x07t']:
                    os.write(fd,shortcut);fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack('HHHH',rows,cols,0,0))
                    os.kill(pid,signal.SIGWINCH);read_for(.08);os.write(fd,b'\x1b');read_for(.06)
            s=client.call('snapshot')
            assert s['data']['conversations'][running]['status']=='running'
            assert any(p['thread']==attention for p in s['pending'])
            assert s['data']['conversations']['local-queue']['queued'][0]['state']=='held'
            assert all(s['data']['drafts'][k]==v for k,v in drafts.items())
            os.write(fd,b'\x07q');read_for(.4)
            done,status=os.waitpid(pid,os.WNOHANG);assert done==pid;finished=True;assert os.waitstatus_to_exitcode(status)==0
            assert b'\x1b[?1049l' in output
            calls=[json.loads(s) for s in (home/'rpc.jsonl').read_text().splitlines()]
            assert len([v for v in calls if v.get('method')=='turn/start'])==2,'navigation submitted an extra task'
            assert not any(v.get('method')=='turn/interrupt' for v in calls),'navigation stopped a task'
            print('PASS: full side collapse/restore, last close, reopen and toolbar; archived full-text hit/context without replacing cache; /live; live task filters/navigation/requests; held queues, drafts and running work preserved; 80/100/160 columns; clean detach')
        finally:
            if pid and not finished:
                try:os.kill(pid,signal.SIGTERM);os.waitpid(pid,0)
                except ProcessLookupError:pass
            if fd is not None:os.close(fd)
            if client:client.close()
            if daemon.poll() is None:daemon.terminate();daemon.wait(timeout=5)
