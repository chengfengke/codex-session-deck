#!/usr/bin/env python3
"""Slow real JSON-RPC fixture: switching stays responsive, history pages, deltas omit old text."""
import json, os, pathlib, socket, subprocess, sys, tempfile, time
ROOT = pathlib.Path(__file__).resolve().parents[1]
BINARY = pathlib.Path(sys.argv[1] if len(sys.argv)>1 else ROOT/'target/debug/codex-deck').resolve()

class IPC:
    def __init__(self, path):
        self.s=socket.socket(socket.AF_UNIX); self.s.settimeout(8); self.s.connect(str(path))
        self.f=self.s.makefile('rwb'); self.seq=0
        self.initial=self.receive()
    def receive(self): return json.loads(self.f.readline())
    def call(self, op, **args):
        self.seq+=1
        self.f.write((json.dumps({'id':self.seq,'op':op,'args':args})+'\n').encode()); self.f.flush()
        while True:
            v=self.receive()
            if v.get('type')=='response' and v.get('id')==self.seq:
                assert 'error' not in v, v.get('error')
                return v['result']
    def close(self): self.f.close(); self.s.close()

with tempfile.TemporaryDirectory(prefix='sd-perf-') as tmp:
    home=pathlib.Path(tmp); state=home/'state'
    env=os.environ|{'CODEX_DECK_HOME':str(state),'CODEX_DECK_CODEX':str(ROOT/'tests/fake_codex.py'),
        'FAKE_CODEX_LOG':str(home/'rpc.jsonl'),'FAKE_CODEX_SLOW_HISTORY':'1','FAKE_CODEX_SEED_HISTORY':'1','FAKE_CODEX_LOCKED_THREAD':'thread-1'}
    with open(home/'daemon.log','w') as log:
        process=subprocess.Popen([str(BINARY),'daemon'],env=env,stdout=log,stderr=log)
        try:
            for _ in range(100):
                if (state/'runtime.sock').exists(): break
                assert process.poll() is None
                time.sleep(.03)
            client=IPC(state/'runtime.sock')
            for _ in range(100):
                snapshot=client.call('snapshot')
                if snapshot['connected']: break
                time.sleep(.03)
            assert snapshot['connected']
            started=time.monotonic(); response=client.call('open',thread='thread-1')
            open_ms=(time.monotonic()-started)*1000
            assert response['loading'] and open_ms<500, (response,open_ms)
            started=time.monotonic(); response=client.call('open',thread='thread-2')
            switch_ms=(time.monotonic()-started)*1000
            assert response['loading'] and switch_ms<500, (response,switch_ms)
            snapshot=client.call('snapshot'); assert len(snapshot['loading'])==2
            until=time.monotonic()+8
            while time.monotonic()<until:
                snapshot=client.call('snapshot')
                if not snapshot['loading']: break
                time.sleep(.03)
            assert not snapshot['loading']
            c=snapshot['data']['conversations']['thread-1']
            assert len(c['messages'])==64 and c['history_cursor']=='8'
            client.call('history',thread='thread-1')
            until=time.monotonic()+8
            while time.monotonic()<until:
                snapshot=client.call('snapshot')
                if not snapshot['loading']: break
                time.sleep(.03)
            c=snapshot['data']['conversations']['thread-1']
            assert len(c['messages'])==96 and c['history_cursor'] is None
            observer=IPC(state/'runtime.sock')
            observed=observer.initial['snapshot']
            while len(observed['data']['conversations']['thread-1']['messages'])!=96 or observed['loading']:
                update=observer.receive()
                if update.get('type')=='snapshot': observed=update['snapshot']
            observer.call('subscribe',delta=True)
            client.call('draft',thread='thread-1',draft={'text':'new draft','cursor':9})
            while True:
                update=observer.receive()
                if update.get('type')=='delta' and update['delta']['metadata']['data']['drafts'].get('thread-1',{}).get('text')=='new draft': break
            assert not update['delta']['histories'], 'draft edit retransmitted history'
            delta_bytes=len(json.dumps(update).encode())
            assert delta_bytes<10000, delta_bytes
            calls=[json.loads(x) for x in (home/'rpc.jsonl').read_text().splitlines()]
            limits=[x['params']['limit'] for x in calls if x.get('method')=='thread/turns/list']
            assert limits==[8,8,30], limits
            assert not any(x.get('method')=='turn/start' for x in calls), 'test submitted a model task'
            assert not any(x.get('method')=='thread/resume' for x in calls), 'browsing history acquired a writer'
            try:
                client.call('send',thread='thread-1',text='explicit locked send')
                raise AssertionError('locked thread accepted execution')
            except AssertionError as e:
                assert 'active writer' in str(e),e
            client.call('send',thread='thread-2',text='explicit unlocked send')
            until=time.monotonic()+8
            while time.monotonic()<until:
                snapshot=client.call('snapshot')
                if snapshot['data']['conversations']['thread-2']['status']=='completed':break
                time.sleep(.03)
            assert snapshot['data']['conversations']['thread-2']['status']=='completed'
            observer.close()
            client.call('shutdown'); client.close(); process.wait(timeout=5)
            print(f'PASS: 1-second history backend; open_ack={open_ms:.1f} ms; second_switch_ack={switch_ms:.1f} ms; concurrent read-only browsing of locked thread; resume only on explicit send; 8/30-turn paging; draft_delta={delta_bytes} bytes without history; simulated execution only')
        finally:
            if process.poll() is None: process.terminate(); process.wait(timeout=5)
            if process.returncode: print((home/'daemon.log').read_text(),file=sys.stderr)
