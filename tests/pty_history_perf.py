#!/usr/bin/env python3
"""Measure actual TUI switching against 10.5 MB of isolated synthetic cached history."""
import errno, fcntl, json, os, pathlib, pty, select, signal, socket, sqlite3, struct, subprocess, sys, tempfile, termios, time
ROOT=pathlib.Path(__file__).resolve().parents[1]
BINARY=pathlib.Path(sys.argv[1] if len(sys.argv)>1 else ROOT/'target/release/codex-deck').resolve()
FAKE=pathlib.Path(sys.argv[2] if len(sys.argv)>2 else ROOT/'tests/fake_codex.py').resolve()
with tempfile.TemporaryDirectory(prefix='sd-history-') as tmp:
    state=pathlib.Path(tmp)/'state'; state.mkdir()
    data={'conversations':{},'projects':[],'drafts':{},'layouts':{},'active':'local-a','sidebar':True,'sidebar_width':28}
    markers=['麒','麟','霆','霄','霖']
    for letter,marker in zip('abcde',markers):
        key='local-'+letter
        messages=[{'id':f'm-{i}','role':'Codex','status':'','text':('History with words and 中文 👩‍💻. '*30+'\n')*1} for i in range(1000)]
        messages[-1]['text']+='\n'+marker
        data['conversations'][key]={'id':key,'title':'Session '+letter.upper(),'cwd':tmp,'messages':messages,'loaded':True}
    db=sqlite3.connect(state/'state.sqlite'); db.execute('CREATE TABLE state (id INTEGER PRIMARY KEY, version INTEGER NOT NULL, payload TEXT NOT NULL)')
    payload=json.dumps(data); db.execute('INSERT INTO state VALUES(1,1,?)',(payload,)); db.commit(); db.close()
    env=os.environ|{'TERM':'xterm-256color','CODEX_DECK_HOME':str(state),'CODEX_DECK_CODEX':str(FAKE),'PATH':str(pathlib.Path(sys.executable).parent)+':'+os.environ.get('PATH','')}
    with open(pathlib.Path(tmp)/'daemon.log','w') as log:
        daemon=subprocess.Popen([str(BINARY),'daemon'],env=env,stdout=log,stderr=log)
        pid=None; fd=None; reaped=False
        try:
            for _ in range(200):
                if (state/'runtime.sock').exists(): break
                assert daemon.poll() is None
                time.sleep(.02)
            pid,fd=pty.fork()
            if pid==0: os.execve(str(BINARY),[str(BINARY)],env)
            fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack('HHHH',40,160,0,0))
            def wait_for(marker,timeout=20):
                end=time.monotonic()+timeout; output=bytearray()
                while time.monotonic()<end:
                    ready,_,_=select.select([fd],[],[],.05)
                    if ready:
                        try: chunk=os.read(fd,65536)
                        except OSError as e:
                            if e.errno==errno.EIO: break
                            raise
                        output.extend(chunk)
                        if marker.encode() in output: return
                raise AssertionError('TUI marker timeout')
            wait_for(markers[0]); time.sleep(.3)
            def drain():
                while select.select([fd],[],[],0)[0]: os.read(fd,65536)
            timings=[]
            for letter,marker in list(zip('bcde',markers[1:]))+list(zip('abcde',markers)):
                drain(); started=time.monotonic()
                os.write(fd,b'\x07p'+('Session '+letter.upper()).encode()+b'\r')
                wait_for(marker)
                timings.append(round((time.monotonic()-started)*1000,2))
                time.sleep(.1)
            os.write(fd,b'\x07q')
            end=time.monotonic()+5
            while time.monotonic()<end:
                if select.select([fd],[],[],.05)[0]:
                    try: os.read(fd,65536)
                    except OSError: pass
                done,status=os.waitpid(pid,os.WNOHANG)
                if done: reaped=True; assert os.waitstatus_to_exitcode(status)==0; break
            assert reaped
            print(json.dumps({'history_utf8_bytes':len(payload.encode()),'switch_via_search_ms':timings,'warm_switch_mean_ms':round(sum(timings[4:])/5,2)}))
        finally:
            if pid and not reaped:
                try: os.kill(pid,signal.SIGTERM); os.waitpid(pid,0)
                except ProcessLookupError: pass
            if fd is not None: os.close(fd)
            if daemon.poll() is None: daemon.terminate(); daemon.wait(timeout=5)
