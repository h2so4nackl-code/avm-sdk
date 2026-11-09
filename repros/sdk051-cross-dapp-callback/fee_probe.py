from pathlib import Path
import importlib.util, json, struct, hashlib

ROOT=Path('/tmp/pr33-runtime')
spec=importlib.util.spec_from_file_location('journey',ROOT/'work/examples/staking-rewards/scripts/e2e.py')
j=importlib.util.module_from_spec(spec);spec.loader.exec_module(j)

def snapshot(path):
    b=path.read_bytes();p=308
    def uint(n):
        nonlocal p
        v=int.from_bytes(b[p:p+n],'little');p+=n;return v
    def take(n):
        nonlocal p
        v=b[p:p+n];p+=n;assert len(v)==n;return v
    assert b[:8]==b'AVMLSF01'
    for width in (33,32,32):
        for _ in range(uint(4)):
            take(width);take(uint(4))
    for _ in range(uint(4)):
        take(72);take(uint(4)*20)
    accounts={}
    for _ in range(uint(4)):
        a=take(64).hex();h=take(32).hex();paid=uint(8);last=uint(4);due=uint(16);unpaid=uint(1)
        image=take(uint(4));n=int.from_bytes(image[33:35],'little');pos=35
        extra={}
        for _ in range(n):
            extra[int.from_bytes(image[pos:pos+4],'little')]=int.from_bytes(image[pos+4:pos+20],'little');pos+=20
        length=int.from_bytes(image[pos:pos+4],'little');data=image[pos+4:];assert len(data)==length
        accounts[a[:64]+':'+a[64:]]={'extra':extra,'data':data.hex(),'account_hash':h,'storage_last_paid':paid,'last':last,'due':due,'unpaid':unpaid}
    take(uint(4)*104)
    queue=uint(4);assert queue==0,queue
    receipts=uint(4);take(receipts*136)
    wires=[take(uint(4)) for _ in range(uint(4))]
    return {'accounts':accounts,'wires':wires}

def wire(w):
    # Canonical internal header, optional callback header, ECC entries, body.
    if w[:1]!=b'\x00':return {'kind':'external','size':len(w)}
    flags=w[1];count=int.from_bytes(w[2:4],'little');p=176
    cb=None
    if flags&1:
        cb={'status':int.from_bytes(w[p+96:p+100],'little')};p+=100
    if flags&4:p+=8
    extra={}
    for _ in range(count):
        extra[int.from_bytes(w[p:p+4],'little')]=int.from_bytes(w[p+4:p+20],'little');p+=20
    return {'kind':'internal','flags':flags,'method':int.from_bytes(w[8:16],'little'),'fuel_fee_shells':int.from_bytes(w[16:32],'little'),'extra':extra,'callback':cb,'size':len(w)}

def run():
    out=ROOT/'fee-probe';out.mkdir(exist_ok=True)
    s=j.Stand(ROOT/'avm-sdk-linux-x86_64',out)
    assert s.command('init',dapp_id=j.DAPP)['status']=='ok'
    pool=s.deploy('staking-rewards','fee-pool',j.init_data(5,6,1000,1000))
    receiver=s.deploy('test-receiver','fee-receiver')
    pd,pa=pool.split(':');rd,ra=receiver.split(':')
    s.call(receiver,'staking.receiver.bind',j.bind_body(pd,pa))
    s.call(pool,'staking.rewards.stake',j.stake_body(0,5000),extra='5:5000')
    s.call(pool,'staking.rewards.fund_rewards',j.fund_body(1000),extra='6:1000')
    base=s.state.read_bytes();fixed=s.time;start=snapshot(s.state)
    results=[]
    for reserve in [0,1,100,10000,1000000,start['accounts'][pool]['extra'][2]-2]:
        s.state.write_bytes(base);s.time=fixed-1
        before=snapshot(s.state)
        r=s.command('call',address=pool,method='staking.rewards.stake',args_json=json.dumps('0x'+j.stake_body(1,1000).hex()),identity='developer',extra_currencies='',**s.timestamps(fuel_fee_shells=reserve,block_time=fixed))
        after=snapshot(s.state)
        row={'type':'guest-rejection','declared':reserve,'status':r['status'],'error':r.get('error'),'output_status':int.from_bytes(bytes.fromhex(r['raw_executor_output'])[4:8],'little') if r.get('raw_executor_output') else None,'shell_debit':before['accounts'][pool]['extra'].get(2,0)-after['accounts'][pool]['extra'].get(2,0),'data_unchanged':before['accounts'][pool]['data']==after['accounts'][pool]['data'],'persisted_bytes_unchanged':s.state.read_bytes()==base,'result':r}
        results.append(row);print(json.dumps({k:v for k,v in row.items() if k!='result'}),flush=True)
    for reject in [False,True]:
        s.state.write_bytes(base);s.time=fixed-1
        s.call(receiver,'staking.receiver.toggle',bytes([int(reject)]),block_time=fixed);s.time=fixed-1
        before=snapshot(s.state)
        r=s.call(pool,'staking.rewards.unstake',j.unstake_body(1,1000,rd,ra),extra='2:1000',fuel_fee_shells=100,block_time=fixed)
        after=snapshot(s.state)
        new_wires=after['wires'][len(before['wires']):]
        row={'type':'payout','reject':reject,'pool_shell_change':after['accounts'][pool]['extra'].get(2,0)-before['accounts'][pool]['extra'].get(2,0),'receiver_shell_change':after['accounts'][receiver]['extra'].get(2,0)-before['accounts'][receiver]['extra'].get(2,0),'wires':[wire(w) for w in new_wires],'raw_wires':[w.hex() for w in new_wires],'result':r,'before':before['accounts'],'after':after['accounts']}
        results.append(row);print(json.dumps({k:v for k,v in row.items() if k not in ['result','before','after']}),flush=True)
    (ROOT/'fee-results.json').write_text(json.dumps({'pool':pool,'receiver':receiver,'fixed_time':fixed,'results':results,'journal':s.log},indent=2)+'\n')

if __name__=='__main__':run()
