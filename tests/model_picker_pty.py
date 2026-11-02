#!/usr/bin/env python3
"""Real terminal model selection, mouse, filtering, persistence, and fake turn routing."""
import errno,fcntl,json,os,pathlib,pty,select,signal,socket,sqlite3,struct,subprocess,sys,tempfile,termios,time
ROOT=pathlib.Path(__file__).resolve().parents[1]
BINARY=pathlib.Path(sys.argv[1] if len(sys.argv)>1 else ROOT/'target/debug/codex-deck').resolve()

class IPC:
    def __init__(self,state):
        self.s=socket.socket(socket.AF_UNIX);self.s.settimeout(8);self.s.connect(str(state/'runtime.sock'))
        self.f=self.s.makefile('rwb');self.f.readline();self.seq=0
    def call(self,op,**args):
        self.seq+=1;self.f.write((json.dumps({'id':self.seq,'op':op,'args':args})+'\n').encode());self.f.flush()
        while True:
            v=json.loads(self.f.readline())
            if v.get('type')=='response' and v.get('id')==self.seq:
                assert 'error' not in v,v.get('error')
                return v['result']
    def close(self):self.f.close();self.s.close()

with tempfile.TemporaryDirectory(prefix='sd-models-') as tmp:
    home=pathlib.Path(tmp);state=home/'state';state.mkdir()
    models=[{'id':f'catalog-{i:02}','model':f'fixture-model-{i:02}','displayName':f'Fixture choice {i:02}',
             'description':f'Description for model {i:02}','isDefault':i==0} for i in range(12)]
    data={'conversations':{key:{'id':key,'title':title,'cwd':tmp,'model':model} for key,title,model in
        [('local-a','Main A','fixture-model-03'),('local-b','Main B','fixture-model-09')]},
        'projects':[],'drafts':{'local-a':{'text':'KEEP_MY_DRAFT','cursor':13}},'layouts':{},'active':'local-a',
        'sidebar':True,'sidebar_width':26}
    with sqlite3.connect(state/'state.sqlite') as db:
        db.execute('CREATE TABLE state(id INTEGER PRIMARY KEY,version INTEGER NOT NULL,payload TEXT NOT NULL)')
        db.execute('INSERT INTO state VALUES(1,1,?)',(json.dumps(data),))
    env=os.environ|{'TERM':'xterm-256color','CODEX_DECK_HOME':str(state),'CODEX_DECK_CODEX':str(ROOT/'tests/fake_codex.py'),
        'FAKE_CODEX_MODELS':json.dumps(models),'FAKE_CODEX_LOG':str(home/'rpc.jsonl'),
        'PATH':str(pathlib.Path(sys.executable).parent)+':'+os.environ.get('PATH','')}
    with open(home/'daemon.log','w') as log:
        def start():
            process=subprocess.Popen([str(BINARY),'daemon'],env=env,stdout=log,stderr=log)
            for _ in range(100):
                if (state/'runtime.sock').exists():break
                assert process.poll() is None;time.sleep(.03)
            return process,IPC(state)
        daemon,client=start();pid=None;fd=None;finished=False
        try:
            pid,fd=pty.fork()
            if pid==0:os.execve(str(BINARY),[str(BINARY)],env)
            fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack('HHHH',24,80,0,0));output=bytearray()
            def read_for(seconds):
                until=time.monotonic()+seconds
                while time.monotonic()<until:
                    if select.select([fd],[],[],.02)[0]:
                        try:output.extend(os.read(fd,65536))
                        except OSError as e:
                            if e.errno==errno.EIO:return
                            raise
            def wait_model(model):
                until=time.monotonic()+8
                while time.monotonic()<until:
                    read_for(.05);snap=client.call('snapshot')
                    if snap['data']['conversations']['local-a']['model']==model:
                        # Observe the normal daemon snapshot reaching the TUI before reopening.
                        read_for(.15)
                        return snap
                raise AssertionError(f"model did not update to {model}; actual={snap['data']['conversations']['local-a']['model']}; terminal={output[-2500:].decode(errors='replace')}")
            def open_picker(command='/model'):
                os.write(fd,b'\x07:'+command.removeprefix('/').encode()+b'\r');read_for(.12)
            # Wait for the composer instead of a fixed delay: the startup banner holds the TUI back.
            until=time.monotonic()+8
            while b'Message' not in output and time.monotonic()<until:read_for(.05)
            assert b'Message' in output, output.decode(errors='replace')
            open_picker()
            assert b'Select model' in output and b'fixture-model-03' in output and b'Fixture choice 03' in output, output.decode(errors='replace')
            os.write(fd,b'\x1b[B\r');wait_model('fixture-model-04')
            open_picker('/models');os.write(fd,b'\x1b[200~fixture-model-10\x1b[201~\r');wait_model('fixture-model-10')
            open_picker();os.write(fd,b'\x1b');read_for(.1)
            assert client.call('snapshot')['data']['conversations']['local-a']['model']=='fixture-model-10'
            open_picker();os.write(fd,b'no-matching-model\r');read_for(.1)
            assert b'No matching models' in output
            assert client.call('snapshot')['data']['conversations']['local-a']['model']=='fixture-model-10'
            os.write(fd,b'\x15\x1b[6~\r');wait_model('fixture-model-05')
            open_picker()
            # At 80x24 the model viewport has 5 rows; model 05 is at the bottom,
            # so the first visible row is model 01 (x=6,y=9 in terminal coordinates).
            os.write(fd,b'\x1b[<0;6;9M\x1b[<0;6;9m');snap=wait_model('fixture-model-01')
            assert snap['data']['conversations']['local-b']['model']=='fixture-model-09'
            assert snap['data']['drafts']['local-a']['text']=='KEEP_MY_DRAFT'
            for rows,cols in [(24,100),(40,160),(24,80)]:
                open_picker();fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack('HHHH',rows,cols,0,0))
                os.kill(pid,signal.SIGWINCH);read_for(.1);os.write(fd,b'\x1b');read_for(.05)
            os.write(fd,b'\x07q');read_for(.4)
            done,status=os.waitpid(pid,os.WNOHANG);assert done==pid;finished=True;assert os.waitstatus_to_exitcode(status)==0
            assert b'\x1b[?1049l' in output
            calls=[json.loads(s) for s in (home/'rpc.jsonl').read_text().splitlines()]
            assert not any(v.get('method')=='turn/start' for v in calls),'selecting a model submitted a task'
            client.call('shutdown');client.close();daemon.wait(timeout=5)
            daemon,client=start();snap=client.call('snapshot')
            assert snap['data']['conversations']['local-a']['model']=='fixture-model-01','model not persisted'
            client.call('send',thread='local-a',text='verify selected model in isolated fixture')
            until=time.monotonic()+8
            while time.monotonic()<until:
                snap=client.call('snapshot');key=snap['data']['aliases'].get('local-a')
                if key and snap['data']['conversations'][key]['status']=='completed':break
                time.sleep(.03)
            assert key and snap['data']['conversations'][key]['status']=='completed'
            calls=[json.loads(s) for s in (home/'rpc.jsonl').read_text().splitlines()]
            turns=[v for v in calls if v.get('method')=='turn/start']
            assert len(turns)==1 and turns[0]['params']['model']=='fixture-model-01'
            client.call('shutdown');client.close();daemon.wait(timeout=5)
            print('PASS: /model and /models; arrows, click, paste/filter, empty results, Esc, scrolling; 80/100/160 columns; drafts and other chat unchanged; saved model reaches next fake turn')
        finally:
            if pid and not finished:
                try:os.kill(pid,signal.SIGTERM);os.waitpid(pid,0)
                except ProcessLookupError:pass
            if fd is not None:os.close(fd)
            if daemon.poll() is None:daemon.terminate();daemon.wait(timeout=5)
