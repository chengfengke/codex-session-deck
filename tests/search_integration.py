#!/usr/bin/env python3
"""Full history, archived chats, literal Unicode search, context, cancellation, cached index."""
import json,os,pathlib,socket,subprocess,sys,tempfile,time
ROOT=pathlib.Path(__file__).resolve().parents[1]
BINARY=pathlib.Path(sys.argv[1] if len(sys.argv)>1 else ROOT/'target/debug/codex-deck').resolve()
class IPC:
    def __init__(self,path):
        self.s=socket.socket(socket.AF_UNIX);self.s.settimeout(10);self.s.connect(str(path));self.f=self.s.makefile('rwb');self.f.readline();self.seq=0
    def call(self,op,**args):
        self.seq+=1;self.f.write((json.dumps({'id':self.seq,'op':op,'args':args})+'\n').encode());self.f.flush()
        while True:
            v=json.loads(self.f.readline())
            if v.get('type')=='response' and v.get('id')==self.seq:
                assert 'error' not in v,v.get('error')
                return v['result']
    def wait(self,predicate):
        end=time.monotonic()+15
        while time.monotonic()<end:
            s=self.call('snapshot')
            if predicate(s):return s
            time.sleep(.025)
        raise AssertionError(f'timed out: {s.get("search")}')
    def close(self):self.f.close();self.s.close()
with tempfile.TemporaryDirectory(prefix='sd-search-') as tmp:
    home=pathlib.Path(tmp);state=home/'state'
    env=os.environ|{'CODEX_DECK_HOME':str(state),'CODEX_DECK_CODEX':str(ROOT/'tests/fake_codex.py'),
        'FAKE_CODEX_SEARCH_HISTORY':'1','FAKE_CODEX_SLOW_HISTORY':'1','FAKE_CODEX_LOCKED_THREAD':'thread-1','FAKE_CODEX_LOG':str(home/'rpc.jsonl')}
    with open(home/'daemon.log','w') as log:
        def start():
            p=subprocess.Popen([str(BINARY),'daemon'],env=env,stdout=log,stderr=log)
            for _ in range(100):
                if (state/'runtime.sock').exists():break
                assert p.poll() is None;time.sleep(.03)
            c=IPC(state/'runtime.sock');c.wait(lambda s:s['connected']);return p,c
        process,client=start()
        try:
            client.call('open',thread='thread-1');client.wait(lambda s:not s['loading'])
            s=client.call('snapshot');assert not any('Hidden-NEEDLE' in m['text'] for m in s['data']['conversations']['thread-1']['messages'])
            obsolete=client.call('search',query='replaced while reading')['job'];time.sleep(.1)
            start_time=time.monotonic();job=client.call('search',query='hidden-needle')['job']
            ack=(time.monotonic()-start_time)*1000;assert ack<500,ack
            client.call('search_cancel',job=obsolete)
            local=client.call('prepare',cwd=tmp,title='draft while searching')['thread']
            client.call('draft',thread=local,draft={'text':'PRESERVE_DRAFT','cursor':14})
            s=client.wait(lambda s:s['search']['job']==job and not s['search']['running'])
            assert not s['search']['errors'],s['search']
            assert len(s['search']['hits'])==3,s['search']
            assert any(h['archived'] for h in s['search']['hits'])
            hit=next(h for h in s['search']['hits'] if h['thread']=='thread-1')
            context=client.call('search_context',thread=hit['thread'],message=hit['message'])
            assert any(m['id']==hit['message'] and 'Hidden-NEEDLE' in m['text'] for m in context['messages'])
            for query in ['全文目标','中文','İSTANBUL','%','"literal"']:
                client.call('search',query=query)
                s=client.wait(lambda s:s['search']['query']==query and not s['search']['running'])
                assert len(s['search']['hits'])==3,(query,s['search'])
            calls=[json.loads(line) for line in (home/'rpc.jsonl').read_text().splitlines()]
            pages=[v for v in calls if v.get('method')=='thread/turns/list' and v['params'].get('limit')==100]
            assert len(pages)==6,'index cache failed or history was not fully paginated'
            assert not any(v.get('method') in ['thread/resume','turn/start'] for v in calls)
            assert s['data']['drafts'][local]['text']=='PRESERVE_DRAFT'
            job=client.call('search',query='cancel me')['job'];client.call('search_cancel',job=job)
            time.sleep(.15);assert not client.call('snapshot')['search']['job'],'cancelled progress replaced current search'
            client.call('view',root='thread-1',layout={'Tabs':{'ids':['thread-1'],'active':0}},collapsedSidePanels=['thread-1'])
            client.call('shutdown');client.close();process.wait(timeout=5)
            process,client=start();s=client.call('snapshot')
            assert 'thread-1' in s['data']['collapsed_side_panels']
            client.call('search',query='hidden-needle');s=client.wait(lambda s:s['search']['query']=='hidden-needle' and not s['search']['running'])
            assert len(s['search']['hits'])==3 and not s['search']['errors']
            client.call('shutdown');client.close();process.wait(timeout=5)
            print(f'PASS: full 125-turn histories beyond recent cache; archived chats; Unicode/literal search; context; cached index; cancel; drafts; panel persistence; search_ack={ack:.1f} ms; no model prompts or execution writers')
        finally:
            if process.poll() is None:process.terminate();process.wait(timeout=5)
