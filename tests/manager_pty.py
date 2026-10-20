#!/usr/bin/env python3
"""Actual TUI: mouse directory navigation, inline search/paste, side routing, 3 columns."""
import errno,fcntl,json,os,pathlib,pty,select,signal,socket,sqlite3,struct,subprocess,sys,tempfile,termios,time
ROOT=pathlib.Path(__file__).resolve().parents[1]
BINARY=pathlib.Path(sys.argv[1] if len(sys.argv)>1 else ROOT/'target/debug/codex-deck').resolve()
with tempfile.TemporaryDirectory(prefix='sd-manager-') as tmp:
    home=pathlib.Path(tmp);state=home/'state';state.mkdir()
    directory_a=str(home/'a');directory_b=str(home/'b')
    def chat(key,title,cwd,marker,parent=None):
        return {'id':key,'title':title,'cwd':cwd,'parent':parent,'readonly':parent is not None,
            'messages':[{'id':key+'-message','role':'Codex','text':marker,'status':'completed'}]}
    layout={'Split':{'vertical':False,'ratio':600,'first':{'Tabs':{'ids':['local-a'],'active':0}},'second':{'Tabs':{'ids':['local-side'],'active':0}}}}
    data={'conversations':{'local-a':chat('local-a','Main A',directory_a,'PRIMARY_A'),
        'local-side':chat('local-side','Side A',directory_a,'SIDE_A','local-a'),
        'local-b':chat('local-b','Main B',directory_b,'PRIMARY_B')},'projects':[],'drafts':{},
        'layouts':{'local-a':layout},'active':'local-a','sidebar':True,'sidebar_width':26}
    db=sqlite3.connect(state/'state.sqlite');db.execute('CREATE TABLE state(id INTEGER PRIMARY KEY,version INTEGER NOT NULL,payload TEXT NOT NULL)')
    db.execute('INSERT INTO state VALUES(1,1,?)',(json.dumps(data),));db.commit();db.close()
    env=os.environ|{'TERM':'xterm-256color','CODEX_DECK_HOME':str(state),'CODEX_DECK_CODEX':str(ROOT/'tests/fake_codex.py'),
        'FAKE_CODEX_LOG':str(home/'rpc.jsonl'),'PATH':str(pathlib.Path(sys.executable).parent)+':'+os.environ.get('PATH','')}
    with open(home/'daemon.log','w') as log:
        daemon=subprocess.Popen([str(BINARY),'daemon'],env=env,stdout=log,stderr=log)
        pid=None;fd=None;finished=False;client=None
        try:
            for _ in range(100):
                if (state/'runtime.sock').exists():break
                assert daemon.poll() is None;time.sleep(.03)
            client=socket.socket(socket.AF_UNIX);client.settimeout(8);client.connect(str(state/'runtime.sock'));f=client.makefile('rwb');f.readline();seq=0
            def snapshot():
                global seq
                seq+=1;f.write((json.dumps({'id':seq,'op':'snapshot','args':{}})+'\n').encode());f.flush()
                while True:
                    v=json.loads(f.readline())
                    if v.get('type')=='response' and v.get('id')==seq:return v['result']
            pid,fd=pty.fork()
            if pid==0:os.execve(str(BINARY),[str(BINARY)],env)
            output=bytearray()
            fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack('HHHH',24,80,0,0))
            def read_for(seconds):
                end=time.monotonic()+seconds
                while time.monotonic()<end:
                    if select.select([fd],[],[],.02)[0]:
                        try:output.extend(os.read(fd,65536))
                        except OSError as e:
                            if e.errno==errno.EIO:return
                            raise
            def wait(predicate):
                until=time.monotonic()+8
                while time.monotonic()<until:
                    read_for(.05);s=snapshot()
                    if predicate(s):return s
                raise AssertionError('TUI state timed out')
            until=time.monotonic()+8
            while b'PRIMARY_A' not in output and time.monotonic()<until:read_for(.05)
            assert b'PRIMARY_A' in output and b'SIDE_A' not in output,'side should start hidden'
            assert snapshot()['data']['layouts']['local-a']==layout
            os.write(fd,b'\x07:side-panel show\r')
            until=time.monotonic()+8
            while b'SIDE_A' not in output and time.monotonic()<until:read_for(.05)
            assert b'CONVERSATIONS' in output and b'PRIMARY_A' in output and b'SIDE_A' in output,'3 columns missing at 80'
            os.write(fd,b'\x1b[<0;4;14M\x1b[<0;4;14m')
            wait(lambda s:s['data']['active']=='local-b')
            os.write(fd,b'\x1b[<0;4;11M\x1b[<0;4;11m');read_for(.15)
            os.write(fd,b'SIDE_DRAFT')
            wait(lambda s:s['data']['drafts'].get('local-side',{}).get('text')=='SIDE_DRAFT')
            assert snapshot()['data']['active']=='local-a','side selection replaced main root'
            os.write(fd,b'\x07p\x1b[200~'+directory_b.encode()+b'\x1b[201~\r')
            s=wait(lambda s:s['data']['active']=='local-b')
            assert s['data']['drafts']['local-side']['text']=='SIDE_DRAFT'
            assert 'local-b' not in s['data']['drafts'],'filter paste modified conversation draft'
            os.write(fd,b'\x07p');read_for(.1)
            os.write(fd,b'\x1b[<0;4;8M\x1b[<0;4;8m')
            wait(lambda s:directory_a in s['data']['collapsed_dirs'])
            for rows,cols in [(24,100),(40,160),(24,80)]:
                fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack('HHHH',rows,cols,0,0));os.kill(pid,signal.SIGWINCH);read_for(.1)
            os.write(fd,b'\x07q')
            until=time.monotonic()+5
            while time.monotonic()<until:
                read_for(.05);done,status=os.waitpid(pid,os.WNOHANG)
                if done:finished=True;assert os.waitstatus_to_exitcode(status)==0;break
            assert finished and b'\x1b[?1049l' in output
            calls=[json.loads(line) for line in (home/'rpc.jsonl').read_text().splitlines()]
            assert not any(v.get('method')=='turn/start' for v in calls),'navigation submitted a model task'
            print('PASS: 80-column manager/main/side; mouse selection; directory collapse persistence; side preserves parent; inline pasted filter leaves drafts intact; 80/100/160 resize; clean detach; no model prompts')
        finally:
            if pid and not finished:
                try:os.kill(pid,signal.SIGTERM);os.waitpid(pid,0)
                except ProcessLookupError:pass
            if fd is not None:os.close(fd)
            if client:client.close()
            if daemon.poll() is None:daemon.terminate();daemon.wait(timeout=5)
            if daemon.returncode not in [0,-signal.SIGTERM]:print((home/'daemon.log').read_text(),file=sys.stderr)
