#!/usr/bin/env python3
"""Deterministic app-server fixture. Never contacts OpenAI or touches Codex home."""
import json, os, pathlib, sys, tempfile, threading, time
lock = threading.RLock()
threads = {}
serial = 0
turn_serial = 0
pending = {}
log_path = os.environ.get('FAKE_CODEX_LOG')

def emit(v):
    with lock:
        print(json.dumps(v), flush=True)

def event(method, **params):
    emit({'method': method, 'params': params})

def reply(v, result):
    emit({'id': v['id'], 'result': result})

def make_thread(cwd, parent=None):
    global serial
    serial += 1
    key = f'thread-{serial}'
    t = {'id': key, 'sessionId': 'shared-tree-id', 'name': f'Conversation {serial}',
         'cwd': cwd, 'preview': '', 'updatedAt': int(time.time()), 'status': {'type':'idle'},
         'historyMode':'paginated', 'turns': [], 'forkedFromId': parent}
    threads[key] = t
    return t

def complete(key, turn_id, status='completed'):
    with lock:
        t = threads.get(key)
        if not t:
            return
        turn = next((x for x in t['turns'] if x['id'] == turn_id), None)
        if not turn or turn['status'] != 'inProgress':
            return
        turn['status'] = status
        turn['items'].append({'id': turn_id+'-assistant', 'type':'agentMessage', 'text':'Reply from '+key, 'status':'completed'})
        event('item/completed', threadId=key, turnId=turn_id, item=turn['items'][-1])
        event('turn/completed', threadId=key, turn=turn)
        t['status']={'type':'idle'}

def handle(v):
    global turn_serial
    method = v.get('method')
    if log_path:
        with open(log_path, 'a') as f:
            f.write(json.dumps(v)+'\n')
    if not method:
        pair = pending.pop(str(v.get('id')), None)
        if pair:
            complete(*pair)
        return
    p = v.get('params', {})
    if method == 'initialize':
        reply(v, {'userAgent':'fixture', 'platformFamily':'unix'})
    elif method == 'initialized':
        pass
    elif method == 'account/read':
        reply(v, {'account':{'type':'apiKey'}})
    elif method == 'model/list':
        reply(v, {'data':json.loads(os.environ.get('FAKE_CODEX_MODELS', '[{"id":"test-model","displayName":"Fixture model"}]')), 'nextCursor':None})
    elif method == 'thread/list':
        assert 'cwd' not in p, 'global index must not be cwd filtered'
        reply(v, {'data':[{k:x for k,x in t.items() if k!='turns'} for t in threads.values() if bool(t.get('archived'))==bool(p.get('archived'))], 'nextCursor':None})
    elif method == 'thread/start':
        t = make_thread(p['cwd'])
        reply(v, {'thread':t, 'model':p.get('model') or 'test-model', 'sandbox':{'type':'readOnly' if p.get('sandbox')=='read-only' else 'workspaceWrite'}})
    elif method == 'thread/fork':
        parent = threads[p['threadId']]
        t = make_thread(parent['cwd'], parent['id'])
        boundary = p['lastTurnId']
        for turn in parent['turns']:
            t['turns'].append(json.loads(json.dumps(turn)))
            if turn['id'] == boundary:
                break
        assert all(x['status'] != 'inProgress' for x in t['turns'])
        reply(v, {'thread':t, 'model':'test-model', 'sandbox':{'type':'readOnly'}})
    elif method == 'thread/resume':
        t=threads[p['threadId']]
        if os.environ.get('FAKE_CODEX_LOCKED_THREAD')==p['threadId']:
            emit({'id':v['id'],'error':{'code':-32600,'message':'thread already has an active writer'}})
            return
        def resumed():
            reply(v, {'thread':{k:x for k,x in t.items() if k!='turns'},'model':'test-model','sandbox':{'type':'workspaceWrite'}})
        if os.environ.get('FAKE_CODEX_SLOW_HISTORY'):
            threading.Timer(1, resumed).start()
        else:
            resumed()
    elif method == 'thread/turns/list':
        t=threads[p['threadId']]
        turns=list(reversed(t['turns']))
        start=int(p.get('cursor') or 0); end=start+p.get('limit',30)
        reply(v, {'data':turns[start:end], 'nextCursor':str(end) if end<len(turns) else None})
    elif method == 'thread/read':
        t=threads[p['threadId']]
        def read_history():
            reply(v, {'thread':{k:value for k,value in t.items() if p.get('includeTurns') or k!='turns'}})
        if os.environ.get('FAKE_CODEX_SLOW_HISTORY') and not p.get('includeTurns'):
            threading.Timer(1, read_history).start()
        else:
            read_history()
    elif method == 'thread/name/set':
        threads[p['threadId']]['name']=p['name']
        reply(v,{})
    elif method == 'turn/start':
        assert p['sandboxPolicy']['type'] in ['workspaceWrite','readOnly']
        turn_serial += 1
        key = p['threadId']
        tid = f'turn-{turn_serial}'
        text = p['input'][0]['text']
        turn = {'id':tid,'status':'inProgress','items':[{'id':tid+'-user','type':'userMessage','content':[{'type':'text','text':text}]}]}
        threads[key]['turns'].append(turn)
        threads[key]['status']={'type':'active'}
        if text=='ambiguous':
            # Response is lost. The client must not automatically replay this message.
            sys.stdout.flush()
            os._exit(0)
        reply(v, {'turn':turn})
        event('turn/started', threadId=key, turn=turn)
        event('item/completed', threadId=key, turnId=tid, item=turn['items'][0])
        event('item/agentMessage/delta', threadId=key, turnId=tid, itemId=tid+'-assistant', delta='Stream from '+key)
        if 'approval' in text:
            rid='request-'+tid
            pending[rid]=(key,tid)
            emit({'id':rid,'method':'item/commandExecution/requestApproval','params':{'threadId':key,'turnId':tid,'itemId':'cmd-'+tid,'command':'echo fixture','cwd':p['cwd'],'reason':'fixture approval'}})
        else:
            delay=float(os.environ.get('FAKE_CODEX_TURN_DELAY',1.5 if 'slow' in text else .12))
            threading.Timer(delay, complete, args=(key,tid)).start()
    elif method == 'turn/steer':
        assert p['expectedTurnId']
        reply(v, {'turnId':p['expectedTurnId']})
    elif method == 'turn/interrupt':
        complete(p['threadId'],p['turnId'],'interrupted')
        reply(v,{})
    elif method in ['thread/archive','thread/unarchive']:
        threads[p['threadId']]['archived']=method=='thread/archive'
        reply(v,{})
    elif method == 'account/login/start':
        reply(v,{'verificationUrl':'https://example.invalid/device','userCode':'FIXTURE'})
    else:
        emit({'id':v.get('id'),'error':{'code':-32601,'message':'unknown method '+str(method)}})

if os.environ.get('FAKE_CODEX_SEARCH_HISTORY'):
    for n in range(3):
        seeded=make_thread(str(pathlib.Path(tempfile.gettempdir())/f'search-{n}'))
        seeded['archived']=n==2
        for i in range(125):
            body=f'Ordinary body {i}'
            if i==0:body='Ancient message: Hidden-NEEDLE 全文目标 中文 İSTANBUL % _ "literal"'
            seeded['turns'].append({'id':f"{seeded['id']}-turn-{i}",'status':'completed','items':[
                {'id':f"{seeded['id']}-old-{i}",'type':'userMessage','content':[{'type':'text','text':body}]}]})

if os.environ.get('FAKE_CODEX_SEED_HISTORY'):
    for _ in range(2):
        seeded=make_thread(tempfile.gettempdir())
        for i in range(12):
            seeded['turns'].append({'id':f"{seeded['id']}-turn-{i}",'status':'completed',
                'items':[{'id':f"{seeded['id']}-item-{i}-{j}",'type':'agentMessage',
                    'text':('fixture history 中文 with wrapping words. '*30+'\n')*20,'status':'completed'} for j in range(8)]})

for line in sys.stdin:
    with lock:
        handle(json.loads(line))
