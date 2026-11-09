"""SDK 0.5.1 stock deploy-callback-repro: public-CLI route/callback matrix.

The official guest source, manifest, resources and dependency are unmodified.
The imported helper supplies CLI orchestration and an independent state decoder.
No account or transcript bytes are constructed or edited in persisted state.
"""
from pathlib import Path
import json, hashlib
from fee_probe import j, snapshot, wire

ROOT=Path('/tmp/pr33-runtime')
PING=0x96ece1cde222841c

def args(dest,method,callback,body=b''):
    return bytes.fromhex(dest.replace(':',''))+method.to_bytes(8,'little')+bytes([callback])+body

def run():
    out=ROOT/'stock-probe';out.mkdir(exist_ok=True)
    s=j.Stand(ROOT/'avm-sdk-linux-x86_64',out)
    assert s.command('init',dapp_id=j.DAPP)['status']=='ok'
    parent=s.deploy('stock-refund-probe','stock-parent')
    child=s.deploy('stock-refund-probe','stock-child')
    w=snapshot(s.state)['wires'][-1]
    count=int.from_bytes(w[2:4],'little');length=int.from_bytes(w[4:8],'little')
    method=int.from_bytes(w[8:16],'little')
    body=w[192+20*count:192+20*count+length]
    assert body[:4]==b'ACDP'
    remote='22'*32+':'+child.split(':')[1]
    assert s.command('fund-user',address=remote,shells=10_000_000)['status']=='ok'
    s.call(parent,'repro.forward',args(remote,method,1,body))
    snap=snapshot(s.state);assert remote in snap['accounts']
    deployment={
        'wires':[wire(w) for w in snap['wires'][-3:]],
        'callback_shell_recorded':int.from_bytes(bytes.fromhex(snap['accounts'][parent]['data'])[56:64],'little'),
        'receiver_init_caller_kind':bytes.fromhex(snap['accounts'][remote]['data'])[1],
    }
    assert deployment['callback_shell_recorded']==999
    base=s.state.read_bytes();fixed=s.time;rows=[]
    for route,destination in [('same-dapp',child),('cross-dapp',remote)]:
        for reject in [False,True]:
            for cb in [0,1]:
                s.state.write_bytes(base);s.time=fixed-1
                before=snapshot(s.state)
                payload=args(destination,123 if reject else PING,cb)
                result=s.command('call',address=parent,method='repro.forward',args_json=json.dumps('0x'+payload.hex()),identity='developer',extra_currencies='',**s.timestamps(fuel_fee_shells=100,block_time=fixed))
                after=snapshot(s.state)
                wires=after['wires'][len(before['wires']):]
                row={
                    'route':route,'reject':reject,'callback_requested':cb,
                    'outer_status':result['status'],'error':result.get('error'),
                    'state_file_unchanged':s.state.read_bytes()==base,
                    'parent_shell_debit':before['accounts'][parent]['extra'][2]-after['accounts'][parent]['extra'][2],
                    'wires':[wire(w) for w in wires],
                    'raw_wires':[w.hex() for w in wires],
                    'callback_shell_recorded':int.from_bytes(bytes.fromhex(after['accounts'][parent]['data'])[56:64],'little') if cb and result['status']=='ok' else None,
                }
                replay=s.command('replay');ins=s.command('inspect')
                row['replay_matches']=replay['status']=='ok' and all(replay[k]==ins[k] for k in ['account_state_root','registry_root','cumulative_fee_shells','accepted_input_count'])
                assert row['replay_matches']
                rows.append(row)
                print(json.dumps({k:v for k,v in row.items() if k not in ['wires','raw_wires']}),flush=True)
    sdk=ROOT/'avm-sdk-linux-x86_64/examples/deploy-callback-repro'
    src=ROOT/'probe-sdk/examples/deploy-callback-repro'
    identities={}
    for rel in ['src/lib.rs','avm-contract.toml','avm-memory.json','Cargo.toml','Cargo.lock']:
        assert (src/rel).read_bytes()==(sdk/rel).read_bytes()
        identities[rel]=hashlib.sha256((src/rel).read_bytes()).hexdigest()
    result={'fixture':'official unmodified deploy-callback-repro','source_sha256':identities,'parent':parent,'same_dapp_child':child,'cross_dapp_child':remote,'deployment':deployment,'results':rows,'journal':s.log}
    assert sum(r['outer_status']=='error' for r in rows)==2
    assert all(r['route']=='cross-dapp' and r['callback_requested']==1 and r['state_file_unchanged'] for r in rows if r['outer_status']=='error')
    (ROOT/'stock-results.json').write_text(json.dumps(result,indent=2)+'\n')

if __name__=='__main__':run()
