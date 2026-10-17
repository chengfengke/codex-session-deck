#!/usr/bin/env python3
"""Existing daemon/state compatibility and new/legacy environment precedence."""
import json,os,pathlib,socket,subprocess,sys,tempfile,time
ROOT=pathlib.Path(__file__).resolve().parents[1]
BINARY=pathlib.Path(sys.argv[1] if len(sys.argv)>1 else ROOT/'target/debug/codex-deck').resolve()
LEGACY=pathlib.Path(sys.argv[2]).resolve() if len(sys.argv)>2 else BINARY
class IPC:
    def __init__(self,path):
        self.s=socket.socket(socket.AF_UNIX);self.s.settimeout(8);self.s.connect(str(path));self.f=self.s.makefile('rwb');self.f.readline();self.seq=0
    def call(self,op,**args):
        self.seq+=1;self.f.write((json.dumps({'id':self.seq,'op':op,'args':args})+'\n').encode());self.f.flush()
        while True:
            v=json.loads(self.f.readline())
            if v.get('type')=='response' and v.get('id')==self.seq:
                assert 'error' not in v,v.get('error');return v['result']
    def close(self):self.f.close();self.s.close()
with tempfile.TemporaryDirectory(prefix='deck-rename-') as tmp:
    home=pathlib.Path(tmp);base=home/'state';profile=home/'codex';profile.mkdir()
    env={k:v for k,v in os.environ.items() if not k.startswith(('SESSIONDESK_','CODEX_DECK_'))}
    env|={'CODEX_HOME':str(profile),'XDG_STATE_HOME':str(base),'FAKE_CODEX_LOG':str(home/'rpc.jsonl'),
        'PATH':str(pathlib.Path(sys.executable).parent)+':'+env.get('PATH','')}
    oldenv=env|{'SESSIONDESK_CODEX':str(ROOT/'tests/fake_codex.py')}
    newenv=env|{'CODEX_DECK_CODEX':str(ROOT/'tests/fake_codex.py'),'SESSIONDESK_CODEX':str(home/'nonexistent'/'legacy-codex')}
    process=None;client=None
    with open(home/'daemon.log','w') as log:
        def start(binary,settings,directory=None):
            global process,client
            process=subprocess.Popen([str(binary),'daemon'],env=settings,stdout=log,stderr=log)
            for _ in range(200):
                assert process.poll() is None,'daemon exited'
                paths=[directory/'runtime.sock'] if directory else list(base.glob('sessiondesk/*/runtime.sock'))
                for path in paths:
                    try:client=IPC(path);break
                    except (ConnectionRefusedError,FileNotFoundError):pass
                if client:break
                time.sleep(.025)
            assert client,'no daemon in the original state directory'
            for _ in range(200):
                snap=client.call('snapshot')
                if snap['connected']:return snap
                time.sleep(.025)
            raise AssertionError(snap['error'])
        def stop():
            global client
            client.call('shutdown');client.close();client=None;process.wait(timeout=5)
        try:
            start(LEGACY,oldenv)
            key=client.call('prepare',cwd=tmp,title='Existing draft')['thread']
            draft={'text':'KEEP_EXISTING_DRAFT','cursor':19}
            layout={'Tabs':{'ids':[key],'active':0}}
            client.call('draft',thread=key,draft=draft)
            client.call('view',root=key,layout=layout,collapsedSidePanels=[key])
            pid=process.pid
            status=subprocess.check_output([str(BINARY),'status'],env=newenv,text=True,timeout=8)
            assert 'connected=true' in status and 'sessiondesk' in pathlib.Path(status.split('state=',1)[1].strip()).parts,status
            assert process.pid==pid and process.poll() is None,'new command replaced the existing daemon'
            assert client.call('snapshot')['data']['drafts'][key]==draft
            stop()
            s=start(BINARY,newenv)
            assert s['data']['drafts'][key]==draft and s['data']['layouts'][key]==layout
            assert key in s['data']['collapsed_side_panels']
            stop()
            custom=home/'custom';blocked=home/'old-home-is-a-file';blocked.write_text('must not be used')
            customenv=newenv|{'CODEX_DECK_HOME':str(custom),'SESSIONDESK_HOME':str(blocked)}
            s=start(BINARY,customenv,custom);assert not s['data']['conversations'];stop()
            assert blocked.read_text()=='must not be used'
            calls=[json.loads(line) for line in (home/'rpc.jsonl').read_text().splitlines()]
            assert not any(v.get('method') in ['thread/start','turn/start','thread/resume'] for v in calls)
            assert subprocess.check_output([str(BINARY),'--version'],text=True).startswith('codex-deck ')
            assert 'Codex Deck' in subprocess.check_output([str(BINARY),'--help'],text=True)
            print('PASS: new CLI attaches to existing daemon; original default state, drafts, layout and collapsed panel survive restart; legacy env works; CODEX_DECK_* takes precedence; branding; no model tasks or new threads')
        finally:
            if client:client.close()
            if process and process.poll() is None:process.terminate();process.wait(timeout=5)
