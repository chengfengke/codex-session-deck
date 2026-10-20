#!/usr/bin/env python3
"""Stream public reasoning, plan and tools through the real daemon and terminal."""
import errno, fcntl, json, os, pathlib, pty, select, signal, socket, struct, subprocess, sys, tempfile, termios, time

ROOT = pathlib.Path(__file__).resolve().parents[1]
BINARY = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ROOT/'target/debug/codex-deck').resolve()
with tempfile.TemporaryDirectory(prefix='deck-activity-') as tmp:
    home = pathlib.Path(tmp)
    state = home/'state'
    env = os.environ | {'TERM':'xterm-256color', 'CODEX_DECK_HOME':str(state),
        'CODEX_DECK_CODEX':str(ROOT/'tests/fake_codex.py'), 'FAKE_CODEX_ACTIVITY':'1',
        'FAKE_CODEX_LOG':str(home/'rpc.jsonl'),
        'PATH':str(pathlib.Path(sys.executable).parent)+':'+os.environ.get('PATH','')}
    with open(home/'daemon.log','w') as log:
        daemon = subprocess.Popen([str(BINARY),'daemon'],env=env,stdout=log,stderr=log)
        pid = fd = client = None
        finished = False
        output = bytearray()
        try:
            for _ in range(150):
                if (state/'runtime.sock').exists(): break
                assert daemon.poll() is None
                time.sleep(.025)
            client = socket.socket(socket.AF_UNIX)
            client.settimeout(10)
            client.connect(str(state/'runtime.sock'))
            stream = client.makefile('rwb')
            stream.readline()
            sequence = 0
            def call(op, **args):
                global sequence
                sequence += 1
                stream.write((json.dumps({'id':sequence,'op':op,'args':args})+'\n').encode())
                stream.flush()
                while True:
                    v = json.loads(stream.readline())
                    if v.get('id') == sequence:
                        assert 'error' not in v, v
                        return v['result']
            def read_for(seconds):
                end = time.monotonic()+seconds
                while fd is not None and time.monotonic()<end:
                    if select.select([fd],[],[],.015)[0]:
                        try: output.extend(os.read(fd,65536))
                        except OSError as e:
                            if e.errno == errno.EIO: return
                            raise
            def wait(predicate):
                until = time.monotonic()+12
                while time.monotonic()<until:
                    read_for(.025)
                    snap = call('snapshot')
                    if predicate(snap): return snap
                    time.sleep(.015)
                raise AssertionError('Activity state timed out')
            wait(lambda s:s['connected'])
            local = call('prepare',cwd=tmp,title='Activity fixture',readonly=True)['thread']
            call('view',root=local,layout={'Tabs':{'ids':[local],'active':0}})
            pid,fd = pty.fork()
            if pid == 0: os.execve(str(BINARY),[str(BINARY)],env)
            fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack('HHHH',45,160,0,0))
            read_for(.25)
            call('send',thread=local,text='activity fixture')
            snap = wait(lambda s:local in s['data']['aliases'])
            thread = snap['data']['aliases'][local]
            snap = wait(lambda s:any(m['role']=='Command' and 'ACTIVITY_OUTPUT' in m['text'] and m['status']=='inProgress'
                                      for m in s['data']['conversations'][thread]['messages']))
            messages = snap['data']['conversations'][thread]['messages']
            assert any(m['role']=='Thinking' and 'SECOND_SUMMARY' in m['text'] for m in messages)
            assert any(m['role']=='Plan' and '[>] Verify output' in m['text'] for m in messages)
            assert not any('PRIVATE_RAW' in m['text'] for m in messages)
            snap = wait(lambda s:s['data']['conversations'][thread]['status']=='completed')
            messages = snap['data']['conversations'][thread]['messages']
            command = next(m for m in messages if m['role']=='Command')
            assert command['status']=='completed'
            assert all(text in command['text'] for text in ['ACTIVITY_OUTPUT','exit: 0','duration: 325 ms'])
            assert any(m['role']=='Tool' and 'TOOL_PROGRESS' in m['text'] and 'TOOL_RESULT' in m['text'] for m in messages)
            assert any(m['role']=='Plan' and '[x] Verify output' in m['text'] for m in messages)
            assert any(m['role']=='Codex' and '\\operatorname' in m['text'] for m in messages), 'Stored source should retain original math'
            read_for(.25)
            assert b'PUBLIC_SUMMARY' in output and b'Command' in output and b'Running' in output, ([(m,m in output) for m in [b'PUBLIC_SUMMARY',b'Command',b'Running']],output[-3500:].decode(errors='replace'))
            assert b'PRIVATE_RAW' not in output
            assert b'\\operatorname' not in output and b'\\boxed' not in output
            assert 'clip'.encode() in output and '[0,2]'.encode() in output
            os.write(fd,b'\x07q')
            for _ in range(100):
                read_for(.025)
                done,status = os.waitpid(pid,os.WNOHANG)
                if done:
                    finished = True
                    assert os.waitstatus_to_exitcode(status)==0
                    break
            assert finished
            calls = [json.loads(line) for line in (home/'rpc.jsonl').read_text().splitlines()]
            turns = [v for v in calls if v.get('method')=='turn/start']
            assert len(turns)==1 and turns[0]['params']['summary']=='auto'
            call('shutdown')
            daemon.wait(timeout=5)
            print('PASS: live public summary parts, plan, command output/exit/duration, tool progress/results; no raw reasoning; terminal math with stored source preserved; completion retains streamed output; one simulated task')
        finally:
            if pid and not finished:
                try: os.kill(pid,signal.SIGTERM); os.waitpid(pid,0)
                except ProcessLookupError: pass
            if fd is not None: os.close(fd)
            if client: client.close()
            if daemon.poll() is None: daemon.terminate(); daemon.wait(timeout=5)
            if daemon.returncode not in [0,-signal.SIGTERM]: print((home/'daemon.log').read_text(),file=sys.stderr)
