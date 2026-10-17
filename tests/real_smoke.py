#!/usr/bin/env python3
"""Read-only real-Codex protocol smoke: handshake, account/read, model/list, global thread/list."""
import json, os, pathlib, shutil, socket, subprocess, sys, tempfile, time
root=pathlib.Path(__file__).resolve().parents[1]
binary=pathlib.Path(sys.argv[1] if len(sys.argv)>1 else root/'target/debug/codex-deck').resolve()
codex=os.environ.get('CODEX_DECK_CODEX') or os.environ.get('SESSIONDESK_CODEX') or shutil.which('codex')
assert codex, 'Codex CLI is required'
with tempfile.TemporaryDirectory(prefix='sd-real-') as tmp:
    env=os.environ|{'CODEX_DECK_HOME':tmp,'CODEX_DECK_CODEX':codex}
    with open(pathlib.Path(tmp)/'smoke.log','w') as log:
        p=subprocess.Popen([str(binary),'daemon'],env=env,stdout=log,stderr=log)
        try:
            for _ in range(100):
                if pathlib.Path(tmp,'runtime.sock').exists():break
                assert p.poll() is None, 'daemon exited'
                time.sleep(.1)
            s=socket.socket(socket.AF_UNIX);s.settimeout(55);s.connect(str(pathlib.Path(tmp,'runtime.sock')))
            f=s.makefile('rwb',buffering=0)
            f.write(b'{"id":1,"op":"snapshot","args":{}}\n')
            while True:
                v=json.loads(f.readline())
                if v.get('id')==1:
                    snap=v['result'];break
            assert snap['connected'], snap['error']
            assert not snap['error'], snap['error']
            assert snap['models'], 'model/list returned no models'
            print('PASS: real Codex handshake, account/read, model/list, global thread/list; no prompts submitted; private session content omitted')
            f.write(b'{"id":2,"op":"shutdown","args":{}}\n')
            while True:
                v=json.loads(f.readline())
                if v.get('id')==2:break
            f.close();s.close();p.wait(timeout=10)
        finally:
            if p.poll() is None:p.terminate();p.wait(timeout=5)
