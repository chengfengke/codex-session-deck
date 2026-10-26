#!/usr/bin/env python3
"""Native Unix socket / Windows named-pipe smoke test with a fake Codex backend."""
import json
import os
import pathlib
import socket
import subprocess
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
BINARY = pathlib.Path(sys.argv[1] if len(sys.argv)>1 else ROOT / 'target/debug' / ('codex-deck.exe' if os.name=='nt' else 'codex-deck')).resolve()

class IPC:
    def __init__(self, path):
        self.socket = None
        if os.name == 'nt':
            self.file = open(path, 'r+b', buffering=0)
        else:
            self.socket = socket.socket(socket.AF_UNIX)
            self.socket.settimeout(10)
            self.socket.connect(path)
            self.file = self.socket.makefile('rwb', buffering=0)
        self.seq = 0
        assert json.loads(self.file.readline())['type'] == 'snapshot'
    def call(self, op, **args):
        self.seq += 1
        self.file.write((json.dumps({'id':self.seq,'op':op,'args':args})+'\n').encode())
        while True:
            value = json.loads(self.file.readline())
            if value.get('type') == 'response' and value.get('id') == self.seq:
                assert 'error' not in value, value
                return value['result']
    def close(self):
        self.file.close()
        if self.socket:self.socket.close()

with tempfile.TemporaryDirectory(prefix='deck-') as tmp:
    home = pathlib.Path(tmp)
    state = home/'state'; state.mkdir()
    shim = home/('fake.exe' if os.name=='nt' else 'fake')
    subprocess.run(['rustc',str(ROOT/'tests/fixtures/fake-launcher.rs'),'-o',str(shim)], check=True)
    env = os.environ | {'CODEX_DECK_HOME':str(state), 'CODEX_DECK_CODEX':str(shim),
        'DECK_TEST_PYTHON':sys.executable, 'DECK_TEST_FIXTURE':str(ROOT/'tests/fake_codex.py')}
    endpoint = subprocess.check_output([str(BINARY),'--ipc-path'],env=env,text=True).strip()
    with open(home/'daemon.log','w') as log:
        daemon = subprocess.Popen([str(BINARY),'daemon'],env=env,stdout=log,stderr=log)
        ipc = None
        try:
            end = time.monotonic()+15
            while time.monotonic()<end:
                assert daemon.poll() is None, (home/'daemon.log').read_text()
                try:ipc=IPC(endpoint);break
                except OSError:time.sleep(.05)
            assert ipc, 'IPC listener did not start'
            while time.monotonic()<end:
                if ipc.call('snapshot')['connected']:break
                time.sleep(.05)
            assert ipc.call('snapshot')['connected'], (home/'daemon.log').read_text()
            # A second daemon must not steal the endpoint or corrupt the store.
            duplicate = subprocess.run([str(BINARY),'daemon'],env=env,timeout=10,capture_output=True)
            assert duplicate.returncode == 0, duplicate.stderr
            assert ipc.call('snapshot')['connected']
            chat = ipc.call('prepare',cwd=str(home),title='Portable fixture')['thread']
            ipc.call('send',thread=chat,text='portable smoke')
            end = time.monotonic()+15
            while time.monotonic()<end:
                snapshot = ipc.call('snapshot')
                messages = [m for c in snapshot['data']['conversations'].values() for m in c.get('messages',[])]
                if any('Reply from' in m.get('text','') for m in messages):break
                time.sleep(.05)
            else:raise AssertionError('Fake backend reply missing')
            ipc.close();ipc=None
            # Attaching again must preserve the running background service.
            status = subprocess.check_output([str(BINARY),'status'],env=env,text=True)
            assert 'connected=true' in status, status
            subprocess.run([str(BINARY),'shutdown'],env=env,check=True,timeout=10)
            assert daemon.wait(timeout=10)==0
            print('PASS: native IPC, private endpoint, daemon lock, backend reply, reconnect and shutdown')
        finally:
            if ipc:ipc.close()
            if daemon.poll() is None:
                daemon.terminate();daemon.wait(timeout=10)
