#!/usr/bin/env python3
"""Exercise the actual daemon, Unix IPC, SQLite store, and JSON-RPC adapter."""
import json, os, pathlib, socket, subprocess, sys, tempfile, time
ROOT = pathlib.Path(__file__).resolve().parents[1]
BINARY = pathlib.Path(sys.argv[1] if len(sys.argv)>1 else ROOT/'target/debug/codex-deck').resolve()

class IPC:
    def __init__(self, path):
        self.socket=socket.socket(socket.AF_UNIX)
        self.socket.settimeout(8)
        self.socket.connect(str(path))
        self.file=self.socket.makefile('rwb', buffering=0)
        self.seq=0
        self.snapshot={}
    def receive(self):
        v=json.loads(self.file.readline())
        if v.get('type')=='snapshot':
            self.snapshot=v['snapshot']
        return v
    def call(self, op, **args):
        self.seq+=1
        self.file.write((json.dumps({'id':self.seq,'op':op,'args':args})+'\n').encode())
        while True:
            v=self.receive()
            if v.get('id')==self.seq:
                if 'error' in v:
                    raise RuntimeError(v['error'])
                return v['result']
    def wait(self, predicate, timeout=8):
        deadline=time.monotonic()+timeout
        while time.monotonic()<deadline:
            self.snapshot=self.call('snapshot')
            if predicate(self.snapshot):
                return self.snapshot
            time.sleep(.04)
        raise AssertionError('condition timed out')
    def close(self):
        self.file.close()
        self.socket.close()

with tempfile.TemporaryDirectory(prefix='sd-') as tmp:
    home=pathlib.Path(tmp)
    a=home/'a'; b=home/'b'; a.mkdir(); b.mkdir()
    env=os.environ|{'CODEX_DECK_HOME':str(home/'state'),'CODEX_DECK_CODEX':str(ROOT/'tests/fake_codex.py'),'FAKE_CODEX_LOG':str(home/'rpc.jsonl')}
    with open(home/'daemon.log','w') as log:
        process=subprocess.Popen([str(BINARY),'daemon'],env=env,stdout=log,stderr=log)
        try:
            for _ in range(100):
                if (home/'state/runtime.sock').exists():
                    break
                assert process.poll() is None, 'daemon exited'
                time.sleep(.05)
            client=IPC(home/'state/runtime.sock')
            client.wait(lambda s:s['connected'])
            local_a=client.call('prepare',cwd=str(a))['thread']
            local_b=client.call('prepare',cwd=str(b))['thread']
            events=[json.loads(x) for x in (home/'rpc.jsonl').read_text().splitlines()]
            assert not any(x.get('method')=='thread/start' for x in events), 'empty drafts must stay local'
            ta=client.call('send',thread=local_a,text='slow initial')['thread']
            tb=client.call('send',thread=local_b,text='approval')['thread']
            assert ta!=tb, 'thread IDs must stay distinct even with a shared sessionId'
            client.call('send',thread=ta,text='queued second')
            snapshot=client.wait(lambda s:bool(s['pending']))
            assert snapshot['pending'][0]['thread']==tb
            request=snapshot['pending'][0]['id']
            try:
                client.call('resolve',requestId=request,generation=0,thread=ta,result={'decision':'accept'})
                raise AssertionError('stale or cross-thread approval was accepted')
            except RuntimeError:
                pass
            client.call('resolve',requestId=request,generation=snapshot['pending'][0]['generation'],thread=tb,result={'decision':'accept'})
            client.close()  # detach while tasks are running
            client=IPC(home/'state/runtime.sock')
            snapshot=client.wait(lambda s:s['data']['conversations'][ta]['last_turn'] is not None and not s['data']['conversations'][ta]['queued'] and s['data']['conversations'][ta]['status']=='completed')
            assert any('Reply from '+tb in m['text'] for m in snapshot['data']['conversations'][tb]['messages'])
            assert all(('Reply from '+tb) not in m['text'] for m in snapshot['data']['conversations'][ta]['messages']), 'cross-thread output contamination'
            last=snapshot['data']['conversations'][ta]['last_turn']
            client.call('send',thread=ta,text='slow followup')
            side=client.call('side',thread=ta)
            assert side['parent']==ta and side['boundary']==last
            ts=side['thread']
            snapshot=client.call('snapshot')
            assert snapshot['data']['conversations'][ts]['readonly'], 'side must default to discussion'
            # A writer in the same workspace must wait; read-only side may run concurrently.
            tc=client.call('new',cwd=str(a))['thread']
            client.call('send',thread=tc,text='same workspace writer')
            client.call('send',thread=ts,text='side read only')
            snapshot=client.wait(lambda s:s['data']['conversations'][ts]['status']=='completed')
            assert snapshot['data']['conversations'][tc]['queued'], 'overlapping writers must be serialized'
            client.call('stop',thread=ta)
            client.wait(lambda s:s['data']['conversations'][tc]['status']=='completed')
            client.call('draft',thread=tc,draft={'text':'独立草稿 👩‍💻','cursor':len('独立草稿 👩‍💻'.encode())})
            layout={'Split':{'vertical':False,'ratio':600,'first':{'Tabs':{'ids':[ta],'active':0}},'second':{'Tabs':{'ids':[ts],'active':0}}}}
            client.call('view',root=ta,layout=layout,sidebar=True,collapsedDirectories=[str(b)])
            client.call('scroll',thread=ta,offset=18)
            client.call('pin',thread=tb)
            client.call('promote',thread=ts)
            client.call('refresh')
            snapshot=client.call('snapshot')
            assert snapshot['data']['conversations'][ts]['parent'] is None
            # Final submission intentionally loses its acknowledgement. It must remain uncertain.
            client.call('send',thread=tb,text='ambiguous')
            snapshot=client.wait(lambda s:not s['connected'])
            uncertain=snapshot['data']['conversations'][tb]['queued']
            assert uncertain and uncertain[0]['state']=='uncertain'
            events=[json.loads(x) for x in (home/'rpc.jsonl').read_text().splitlines()]
            ambiguous=[x for x in events if x.get('method')=='turn/start' and x['params']['input'][0]['text']=='ambiguous']
            assert len(ambiguous)==1, 'uncertain submission was replayed'
            assert any(x.get('id')==request and x.get('result')=={'decision':'accept'} for x in events)
            client.call('shutdown')
            process.wait(timeout=5)
            import sqlite3
            data=json.loads(sqlite3.connect(home/'state/state.sqlite').execute('select payload from state').fetchone()[0])
            assert data['drafts'][tc]['text']=='独立草稿 👩‍💻'
            assert data['scroll'][ta]==18 and data['layouts'][ta]==layout
            assert data['collapsed_dirs']==[str(b)]
            assert data['conversations'][tb]['pinned']
            process=subprocess.Popen([str(BINARY),'daemon'],env=env,stdout=log,stderr=log)
            for _ in range(100):
                if (home/'state/runtime.sock').exists():break
                time.sleep(.05)
            restarted=IPC(home/'state/runtime.sock')
            recovered=restarted.wait(lambda s:s['connected'])
            assert recovered['data']['drafts'][tc]['text']=='独立草稿 👩‍💻'
            assert recovered['data']['layouts'][ta]==layout
            assert recovered['data']['conversations'][tb]['queued'][0]['state']=='held'
            assert recovered['data']['conversations'][ts]['parent'] is None
            restarted.call('shutdown');restarted.close();process.wait(timeout=5)

            print('PASS: local drafts, global index, independent thread routing, scoped approval, queue, detach/reconnect, fork boundary, read-only side, workspace serialization, promotion, uncertain-send protection, SQLite persistence')
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=5)
            if process.returncode:
                print((home/'daemon.log').read_text(),file=sys.stderr)
