"""Independent P08 worker boundary tests and real CLI acceptance."""
from __future__ import annotations
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('il_task_runner', ROOT / 'eval/runner/run.py')
runner = importlib.util.module_from_spec(spec); spec.loader.exec_module(runner)
REPORT = None
BINARY = None
CASES = []
CONTRACT_PATH = Path(__file__).with_name('ai_protocol_contracts.json')
CONTRACT = json.loads(CONTRACT_PATH.read_text(encoding='utf-8'))

# A fixed independent worker fixture tests launcher accounting and ownership, not
# compiler semantics. Language/HTTP acceptance below uses the actual il binary.
WORKER = r'''#!/usr/bin/python3
import hashlib,json,os,pathlib,sys,time
args=sys.argv;root=pathlib.Path(args[args.index('--repository')+1]);request=json.load(sys.stdin);tool=args[-1]
with (root/'calls.jsonl').open('a') as f:f.write(json.dumps({'tool':tool,'request':request})+'\n')
mode=(root/'mode').read_text() if (root/'mode').exists() else 'normal'
if mode=='cpu':
 while True:pass
if mode in ('children','groups'):
 child=os.fork()
 if child==0:
  if mode=='groups':os.setpgid(0,0)
  while True:pass
 (root/'child').write_text(str(child))
 while True:time.sleep(.05)
if mode=='memory':
 allocations=[]
 while True:allocations.append(bytearray(1024*1024))
if mode=='output':
 while True:sys.stdout.write('x'*65536);sys.stdout.flush()
revision=json.loads((root/'head').read_text());ok=True;diagnostics=[]
entity=request.get('entity_id');result={}
if tool=='inspect':
 if entity=='route.hello':ok=False;diagnostics=[{'code':'E_NAME_NOT_FOUND','retryable':True}]
 else:
  result={'entity':{'entity_id':entity}}
  if entity=='server' and request.get('fields')!=['entity_id']:result['entity']['subject']='dispatch'
  if entity=='hello' and request.get('fields')!=['entity_id']:result['entity'].update(parameters=[{'entity_id':'name','type':'http.PathRequest'}],result='http.ResponseResult',effects=['alloc'],capabilities=[],contracts=[])
if tool in ('dependencies','callers'):result={'entity_ids':[]}
if tool=='slice':result={'entities':[{'entity_id':i} for i in request['root_entities']],'node_count':len(request['root_entities'])}
if tool=='transact':
 if mode=='nonretryable':ok=False;diagnostics=[{'code':'E_CONTRACT_UNPROVEN','retryable':False}]
 else:revision+=1;(root/'head').write_text(json.dumps(revision))
response={'ok':ok,'tool':tool,'tool_version':'1.0.0','base_revision':revision,'result_revision':request.get('revision',revision),'diagnostics':diagnostics,'artifacts':[],'evidence_id':None,'result':result}
journal=pathlib.Path(args[args.index('--store')+1])/'.il-tools';(journal/'runs').mkdir(parents=True,exist_ok=True);(journal/'blobs').mkdir(exist_ok=True)
graph=json.dumps({'functions':[],'capabilities':[]}).encode();gh=hashlib.sha256(graph).hexdigest();(journal/'blobs'/gh).write_bytes(graph)
receipt=json.dumps({'artifacts':[{'role':'graph','path':'blobs/'+gh,'sha256':'sha256:'+gh}],'response':response},sort_keys=True).encode();identity='run_'+hashlib.sha256(receipt).hexdigest();(journal/'runs'/(identity+'.json')).write_bytes(receipt)
result['run_id']=identity
print(json.dumps(response));sys.exit(0 if ok else 1)
'''


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(runner.encoded(value))


@unittest.skipUnless(sys.platform == 'linux', 'Linux process ownership and rlimits required')
class TaskWorkerBoundary(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='il-ai-worker-'); self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name); self.repository = self.root / 'repository'; self.repository.mkdir()
        self.binary = self.root / 'fixed-worker'; self.binary.write_text(WORKER, encoding='utf-8'); self.binary.chmod(0o700)
        write(self.repository / 'head', 1); write(self.root / 'policy.json', {'schema_version':'1.0.0','grants':[],'test_faults':None})
        self.task = {'task_id':'P08-route','requires':['P02','P03','P07'],'base_revision':1,
                     'scope':['server','dispatch','route.hello'],'inputs':['server','hello','spec/http.yaml'],
                     'operations':[{'op':'add_route','route_table_id':'server','route':{'entity_id':'route.hello','method':'GET','path':'/hello/{name}','handler':'hello','parameters':[{'name':'name','type_ref':'String','source':'path','max_utf8_bytes':128}]}}],
                     'acceptance':['add_route_without_full_source_dump'],'resource_budget':{'context_tokens':65536,'tool_calls':40,'cpu_seconds':10,'memory_bytes':268435456},
                     'outputs':['route.hello'],'rollback_revision':1,'status':'READY'}
        self.task_path = self.root / 'task.json'; write(self.task_path,self.task)
        self.delivered=[]

    def worker(self):
        return runner.Session(binary=self.binary, repository=self.repository, store=self.root/'store',
                              policy=self.root/'policy.json', task=self.task_path, directory=self.root/'session')

    def step(self, tool='inspect', request=None):
        result=self.worker().step({'tool':tool,'request':request if request is not None else {'entity_id':'server','revision':1,'budget':4096}})
        self.delivered.append(result)
        return result

    def calls(self):
        path=self.repository/'calls.jsonl'
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def state(self):
        return json.loads((self.root/'session/session.json').read_text())

    def context(self):
        self.assertEqual(self.step()['status'],'RUNNING')
        self.assertEqual(self.step('inspect',{'entity_id':'hello','revision':1,'fields':['entity_id','parameters','result','effects','capabilities','contracts'],'budget':4096})['status'],'RUNNING')
        self.assertEqual(self.step('dependencies',{'entity_id':'hello','revision':1,'direction':'forward','budget':4096})['status'],'RUNNING')
        self.assertEqual(self.step('callers',{'entity_id':'hello','revision':1,'budget':4096})['status'],'RUNNING')
        result=self.step('slice',{'root_entities':['http.PathRequest','http.ResponseResult'],'revision':1,'max_nodes':20,'max_tokens':8192})
        self.assertEqual(result['status'],'RUNNING',result);self.assertEqual([x['phase'] for x in result['context']],CONTRACT['context_phases'][5:])
        self.assertEqual(self.state()['phases'],CONTRACT['context_phases'])

    def test_closed_requests_reject_before_worker_start(self):
        for step in [{'tool':'shell','request':{}},{'tool':'state','request':{},'argv':['touch','marker']},
                     {'tool':'state','request':{'command':'touch marker'}},
                     {'tool':'inspect','request':{'entity_id':'program','revision':1}},
                     {'tool':'schema-check','request':{'source':'module app {}'}}]:
            result=self.worker().step(step);self.assertEqual(result['status'],'BLOCKED');self.assertEqual(self.calls(),[])
        with self.assertRaises(runner.Rejected):runner.strict('{"tool":"state","tool":"build"}')
        with self.assertRaises(runner.Rejected):runner.strict('{"x":NaN}')

    def test_task_preflight_rejects_unknown_acceptance_and_unverified_prerequisite(self):
        for key,value in [('acceptance',['shell -c touch marker']),('inputs',['../../secret']),('scope',['program'])]:
            original=self.task[key];self.task[key]=value;write(self.task_path,self.task)
            result=self.step();self.assertEqual(result['status'],'BLOCKED');self.assertEqual(self.calls(),[])
            self.task[key]=original
        write(self.task_path,self.task)
        read_json=runner.schema.read_json
        def prerequisite(path):
            if path == ROOT/'eval/tasks/P07.json':
                return {'status':'BLOCKED'}
            return read_json(path)
        with patch.object(runner.schema,'read_json',side_effect=prerequisite):
            result=self.step();self.assertEqual(result['code'],'E_STATE_INCONSISTENT');self.assertEqual(self.calls(),[])

    def test_context_order_and_mutation_preflight(self):
        result=self.step('transact',{'task_id':'P08-route','base_revision':1,'scope':self.task['scope'],'operations':self.task['operations'],'required_checks':[]})
        self.assertEqual(result['code'],'E_CONTEXT_INSUFFICIENT');self.assertFalse(any(x['tool']=='transact' for x in self.calls()))

    def test_context_and_call_budgets_persist_across_process_restart(self):
        self.context();before=self.state();self.assertEqual(before['tool_calls'],len(self.calls()))
        self.assertGreater(before['context_bytes'],1000)
        result=self.step('state',{});self.assertEqual(result['status'],'RUNNING');after=self.state()
        self.assertEqual(after['tool_calls'],before['tool_calls']+1);self.assertGreater(after['context_bytes'],before['context_bytes'])
        # Independently count all delivered bytes plus explicitly billed trusted
        # preflight responses; normal step envelopes must not be double billed.
        expected=sum(item['response_bytes'] for item in after['history'] if item['kind']!='step')
        expected+=sum(len((json.dumps(item,sort_keys=True,ensure_ascii=False,separators=(',',':'))+'\n').encode('utf-8')) for item in self.delivered)
        self.assertEqual(after['context_bytes'],expected)
        # The task cannot be edited to silently reset its persisted budget.
        self.task['resource_budget']['tool_calls']=500;write(self.task_path,self.task)
        result=self.step('state',{});self.assertEqual(result['code'],'E_STATE_INCONSISTENT');self.assertEqual(len(self.calls()),after['tool_calls'])

    def test_call_limit_counts_preflight_and_stops_before_next_worker(self):
        self.task['resource_budget']['tool_calls']=4;write(self.task_path,self.task)
        result=self.step();self.assertEqual(result['code'],'E_RESOURCE_LIMIT');self.assertEqual(len(self.calls()),4)
        self.assertEqual(self.state()['tool_calls'],4);self.assertEqual(self.state()['status'],'BLOCKED')

    def test_context_exhaustion_withholds_data_and_retains_receipt(self):
        self.task['resource_budget']['context_tokens']=1;write(self.task_path,self.task)
        result=self.step();self.assertEqual(result['code'],'E_CONTEXT_INSUFFICIENT');self.assertNotIn('response',result)
        self.assertEqual(len(self.calls()),1);self.assertEqual(len(self.state()['runs']),1)
        self.assertTrue((self.root/'session/000001-stdout.json').is_file())

    def test_external_revision_change_blocks_mutation_without_transact(self):
        self.context();write(self.repository/'head',2)
        result=self.step('transact',{'task_id':'P08-route','base_revision':1,'scope':self.task['scope'],'operations':self.task['operations'],'required_checks':[]})
        self.assertEqual(result['code'],'E_STALE_REVISION');self.assertFalse(any(x['tool']=='transact' for x in self.calls()))

    def test_cpu_exhaustion_kills_owned_process_group(self):
        unrelated=subprocess.Popen([sys.executable,'-c','import time;time.sleep(30)'])
        self.addCleanup(lambda:(unrelated.terminate(),unrelated.wait(timeout=5)) if unrelated.poll() is None else None)
        self.task['resource_budget']['cpu_seconds']=1;write(self.task_path,self.task)
        (self.repository/'mode').write_text('children')
        started=__import__('time').monotonic();result=self.step()
        self.assertEqual(result['status'],'BLOCKED');self.assertLess(__import__('time').monotonic()-started,12)
        child=int((self.repository/'child').read_text());stat=Path('/proc')/str(child)/'stat'
        self.assertFalse(stat.exists(), 'owned descendants must be killed and reaped')
        self.assertEqual(self.state()['tool_calls'],1)
        self.assertIsNone(unrelated.poll(), 'another process group must survive task exhaustion')

    def test_cpu_exhaustion_covers_descendant_separate_process_group(self):
        self.task['resource_budget']['cpu_seconds']=1;write(self.task_path,self.task)
        (self.repository/'mode').write_text('groups');result=self.step()
        self.assertEqual(result['status'],'BLOCKED');child=int((self.repository/'child').read_text())
        stat=Path('/proc')/str(child)/'stat'
        self.assertFalse(stat.exists(), 'separate-group descendants must be killed and reaped')
        self.assertEqual(self.state()['tool_calls'],1)

    def test_interrupted_admission_is_retained_and_never_replayed(self):
        self.assertEqual(self.step()['status'],'RUNNING');state=self.state();before=len(self.calls())
        state['pending']={'index':state['tool_calls']+1,'tool':'transact','request':{},'kind':'step'}
        state['tool_calls']+=1
        write(self.root/'session/session.json',state)
        result=self.step('state',{});self.assertEqual(result['status'],'BLOCKED');self.assertEqual(len(self.calls()),before)
        self.assertIsNotNone(self.state()['pending'])

    def test_corrupt_session_schema_and_counters_cannot_reset_budgets(self):
        self.assertEqual(self.step()['status'],'RUNNING');original=self.state();calls=len(self.calls())
        for key,value in [('tool_calls',-1),('context_bytes',-1),('context_bytes',0),('cpu_seconds',-1),('schema_version','9.0.0'),('extra',True)]:
            with self.subTest(field=key,value=value):
                corrupted=json.loads(json.dumps(original));corrupted[key]=value
                write(self.root/'session/session.json',corrupted)
                result=self.step('state',{});self.assertEqual(result['code'],'E_STATE_INCONSISTENT');self.assertEqual(len(self.calls()),calls)
        write(self.root/'session/session.json',original)

    def test_nonretryable_transaction_requires_explanation_and_changed_repair(self):
        self.context();(self.repository/'mode').write_text('nonretryable')
        request={'task_id':'P08-route','base_revision':1,'scope':self.task['scope'],'operations':self.task['operations'],'required_checks':[]}
        rejected=self.step('transact',request);self.assertEqual(rejected['status'],'DESIGN_REQUIRED')
        identity=rejected['response']['result']['run_id'];before=len(self.calls())
        replay=self.step('transact',request);self.assertEqual(replay['status'],'DESIGN_REQUIRED');self.assertEqual(len(self.calls()),before)
        explanation=self.step('explain',{'run_id':identity,'diagnostic_id':'diag_a','context_budget':4096})
        self.assertEqual(explanation['status'],'DESIGN_REQUIRED');self.assertTrue(self.state()['design_failure']['explained'])
        changed=json.loads(json.dumps(request));changed['operations'][0]['route']['path']='/greeting/{name}'
        (self.repository/'mode').write_text('normal')
        fixed=self.step('transact',changed);self.assertEqual(fixed['status'],'RUNNING');self.assertEqual(self.state()['current_revision'],2)

    def test_memory_and_unbounded_output_fail_closed(self):
        for mode in ['memory','output']:
            with self.subTest(mode=mode):
                directory=self.root/('session-'+mode)
                (self.repository/'mode').write_text(mode)
                worker=runner.Session(binary=self.binary,repository=self.repository,store=self.root/'store',policy=self.root/'policy.json',task=self.task_path,directory=directory)
                result=worker.step({'tool':'inspect','request':{'entity_id':'server','revision':1}})
                self.assertEqual(result['status'],'BLOCKED');state=json.loads((directory/'session.json').read_text());self.assertEqual(state['tool_calls'],1)


@unittest.skipUnless(sys.platform == 'linux', 'real native CLI acceptance requires Linux')
class RealProtocolAcceptance(unittest.TestCase):
    def setUp(self):
        import graph_cli, runtime_cli
        self.assertIsNotNone(BINARY, 'real protocol tests require --binary')
        graph_cli.BINARY = BINARY
        self.fixture = runtime_cli.RuntimeCliAcceptance('test_real_files_handles_and_cleanup')
        self.fixture.setUp(); self.addCleanup(self.fixture.doCleanups)
        (self.fixture.repository/'spec').mkdir();shutil.copyfile(ROOT/'spec/http.yaml',self.fixture.repository/'spec/http.yaml')
        self.fixture.git('add','spec/http.yaml')
        self.fixture.git('-c','user.name=il test','-c','user.email=test@invalid.local','-c','commit.gpgsign=false','-c','core.hooksPath='+str(self.fixture.root/'no-hooks'),'commit','-m','test: bind locked HTTP contract to source fixture')
        write(self.fixture.bindings,{'schema_version':'1.0.0','bindings':[{'revision':0,'git_commit':self.fixture.git('rev-parse','HEAD'),'tree_hash':self.fixture.git('rev-parse','HEAD^{tree}')}]})
        self.directory = REPORT.parent / (REPORT.stem + '-artifacts') / str(time.time_ns()) / self._testMethodName
        self.directory.mkdir(parents=True, exist_ok=True)
        self.fixture.store = self.directory / 'store'
        self.fixture.policy_path = self.directory / 'policy.json'
        write(self.fixture.policy_path, {'schema_version':'1.0.0','grants':[
            {'entity_id':'demo.listen','kind':'Listen','scope':'127.0.0.1:8080'},
            {'entity_id':'demo.clock','kind':'ClockRead','scope':None}], 'test_faults':None})
        source='\n'.join((ROOT/'packages'/name/'lib.il').read_text(encoding='utf-8') for name in ['core','alloc','io','time','net','json','http','test','tracing'])
        demo=(ROOT/'examples/http_demo/main.il').read_text(encoding='utf-8')
        route=',{"entity_id":"demo.route.hello","method":"GET","path":"/hello/{name}","handler":"demo.hello","parameters":[{"name":"name","type_ref":"String","source":"path","max_utf8_bytes":128}]}'
        self.assertEqual(demo.count(route),1);demo=demo.replace(route,'')
        # Trusted setup publishes packages once; the subsequent AI transcript never
        # receives full source, graph snapshots, or formatter output.
        self.fixture.publish(source+'\n'+demo)
        self.task={'task_id':'P08-route','requires':['P02','P03','P07'],'base_revision':1,
                   'scope':['demo.server','demo.dispatch','demo.route.hello'],'inputs':['demo.server','demo.hello','spec/http.yaml'],
                   'operations':[{'op':'add_route','route_table_id':'demo.server','route':{'entity_id':'demo.route.hello','method':'GET','path':'/hello/{name}','handler':'demo.hello','parameters':[{'name':'name','type_ref':'String','source':'path','max_utf8_bytes':128}]}}],
                   'acceptance':sorted(runner.ACCEPTANCE),'resource_budget':{'context_tokens':131072,'tool_calls':64,'cpu_seconds':120,'memory_bytes':1073741824},
                   'outputs':['demo.route.hello'],'rollback_revision':1,'status':'READY'}
        self.task_path=self.directory/'task.json';write(self.task_path,self.task)
        self.session_directory=self.directory/'session'
        self.case={'id':self._testMethodName,'passed':False,'steps':[]};CASES.append(self.case)

    def invoke(self,tool,request):
        argv=[sys.executable,str(ROOT/'eval/runner/run.py'),'--binary',str(BINARY),'--repository',str(self.fixture.repository),
              '--store',str(self.fixture.store),'--policy',str(self.fixture.policy_path),'--task',str(self.task_path),'--directory',str(self.session_directory)]
        completed=subprocess.run(argv,input=runner.encoded({'tool':tool,'request':request}),capture_output=True,timeout=180)
        self.assertTrue(completed.stdout,completed.stderr.decode('utf-8',errors='replace'));result=json.loads(completed.stdout)
        self.case['steps'].append({'tool':tool,'request':request,'result':result,'stdout_sha256':runner.digest(completed.stdout),'exit_code':completed.returncode})
        write(self.directory/'transcript.json',self.case)
        self.assertEqual(result['status'],'RUNNING',result)
        self.receipt(result['response']['result']['run_id'])
        return result['response']

    def receipt(self,identity):
        path=self.fixture.store/'.il-tools/runs'/(identity+'.json');data=path.read_bytes()
        self.assertEqual(runner.digest(data)[7:],identity[4:]);receipt=json.loads(data)
        schema_path=ROOT/'schema/tool_run.schema.json'
        runner.schema.validate_instance(receipt,runner.schema.read_json(schema_path),schema_path)
        for item in receipt['artifacts']:
            self.assertEqual(item['path'],'blobs/'+item['sha256'][7:])
            self.assertEqual(runner.digest((self.fixture.store/'.il-tools'/item['path']).read_bytes()),item['sha256'])
        return receipt

    def live_route(self,executable,policy,label,status):
        # Reuse only the independent wire client/process owner, never the il parser.
        sys.path.insert(0,str(ROOT/'tests/http_blackbox'))
        module_spec=importlib.util.spec_from_file_location('p08_wire_client',ROOT/'tests/http_blackbox/cli.py')
        wire_client=importlib.util.module_from_spec(module_spec);module_spec.loader.exec_module(wire_client)
        with wire_client.server(executable,policy,self.directory/label):
            wire,elapsed=wire_client.exchange(wire_client.request(CONTRACT['scenario']['request']))
            code,headers,body=wire_client.response(wire);self.assertEqual(code,status)
            if status==CONTRACT['scenario']['restored_status']:self.assertEqual(body,b'')
            else:self.assertEqual(json.loads(body),CONTRACT['scenario']['body_json'])
            self.case.setdefault('wire_receipts',[]).append({'label':label,'status':code,'headers':headers,'body':list(body),'elapsed_ns':elapsed,'sha256':runner.digest(wire)})

    def rebuild_bundle(self,evidence,built):
        import graph_cli,runtime_cli
        identity=evidence['evidence_id'];original=self.fixture.store/'.il-tools/evidence'/identity
        copied=self.directory/'replay-bundle';shutil.copytree(original,copied)
        data=(copied/'manifest.json').read_bytes();self.assertEqual(runner.digest(data)[7:],identity[3:]);manifest=json.loads(data)
        schema_path=ROOT/'schema/application_evidence.schema.json';runner.schema.validate_instance(manifest,runner.schema.read_json(schema_path),schema_path)
        for digest in manifest['blobs']:self.assertEqual(runner.digest((copied/'blobs'/digest[7:]).read_bytes()),digest)
        run=json.loads((copied/'runs'/(built['result']['run_id']+'.json')).read_text(encoding='utf-8'))
        roles={item['role']:copied/item['path'] for item in run['artifacts']}
        for name in CONTRACT['reconstruction']['input_roles']:self.assertIn(name,roles)
        replay=runtime_cli.RuntimeCliAcceptance('test_real_files_handles_and_cleanup');replay.setUp();self.addCleanup(replay.doCleanups)
        replay.store=self.directory/'replay-store';replay.policy_path=self.directory/'replay-policy.json';shutil.copyfile(roles['policy'],replay.policy_path)
        graph=json.loads(roles['graph'].read_text(encoding='utf-8'));revision=graph['revision']
        (replay.repository/'examples/bootstrap/graph.json').write_bytes(roles['graph'].read_bytes())
        (replay.repository/'toolchain.lock').write_bytes(roles['toolchain_lock'].read_bytes())
        replay.state['head_revision']=revision;replay.state['last_verified_revision']=revision
        write(replay.repository/'repository_state.json',replay.state)
        replay.git('add','.');replay.git('-c','user.name=il test','-c','user.email=test@invalid.local','-c','commit.gpgsign=false','-c','core.hooksPath='+str(replay.root/'no-hooks'),'commit','-m','test: reconstruct application solely from evidence bundle')
        write(replay.bindings,{'schema_version':'1.0.0','bindings':[{'revision':revision,'git_commit':replay.git('rev-parse','HEAD'),'tree_hash':replay.git('rev-parse','HEAD^{tree}')}]})
        tool_directory=self.directory/'replay-tools';tool_directory.mkdir();compiler=tool_directory/'il';shutil.copyfile(roles['compiler'],compiler);compiler.chmod(0o700)
        shutil.copyfile(roles['runtime'],tool_directory/'libil_native_runtime.a')
        old_binary=graph_cli.BINARY;graph_cli.BINARY=compiler
        hidden=self.fixture.store.with_name('original-store-unavailable')
        self.fixture.store.rename(hidden)
        try:
            response=replay.invoke('build',{'revision':revision,'target':'x86_64-unknown-linux-gnu','profile':'release','runtime_profile':'full'})
            native=replay.validate_build(response,'full','release')
            expected=built['result']['native']['artifacts']
            for role in CONTRACT['reconstruction']['identical_artifacts']:self.assertEqual(native['artifacts'][role]['sha256'],expected[role]['sha256'],role)
            self.live_route(native['artifacts']['executable']['path'],replay.policy_path,'rebuilt-from-evidence',200)
            self.case['reconstruction']={'passed':True,'manifest_sha256':runner.digest(data),'artifacts':native['artifacts']}
        finally:
            hidden.rename(self.fixture.store);graph_cli.BINARY=old_binary

    def acquire(self):
        self.invoke('inspect',{'entity_id':'demo.server','revision':1,'budget':4096})
        signature=self.invoke('inspect',{'entity_id':'demo.hello','revision':1,'fields':['entity_id','parameters','result','effects','capabilities','contracts'],'budget':4096})
        self.assertNotIn('blocks',signature['result']['entity'])
        self.invoke('dependencies',{'entity_id':'demo.hello','revision':1,'direction':'forward','budget':8192})
        self.invoke('callers',{'entity_id':'demo.hello','revision':1,'budget':4096})
        self.invoke('slice',{'root_entities':['http.PathRequest','http.ResponseResult'],'revision':1,'max_nodes':64,'max_tokens':8192})

    def transaction(self,handler='demo.hello',base=1):
        operation=json.loads(json.dumps(self.task['operations'][0]));operation['route']['handler']=handler
        return {'task_id':'P08-route','base_revision':base,'scope':self.task['scope'],'operations':[operation],'required_checks':['types','contracts','ownership']}

    def direct(self,tool,request,ok=True):
        response=self.fixture.invoke(tool,request,ok)
        self.case['steps'].append({'tool':tool,'request':request,'response':response,'boundary':'direct_cli'})
        if response.get('result',{}):
            identity=response['result'].get('run_id')
            if identity:self.receipt(identity)
        return response

    def test_historical_budget_diagnostics_and_receipt_tamper(self):
        first=self.direct('transact',self.transaction(handler='demo.health'),False)
        second=self.direct('transact',self.transaction(handler='demo.serve_once'),False)
        self.assertNotEqual(first['result']['run_id'],second['result']['run_id'])
        for failed in [first,second]:
            self.assertEqual(failed['result_revision'],1)
            run=self.receipt(failed['result']['run_id']);self.assertIsNotNone(run['binding']['candidate_hash'])
            self.assertNotEqual(run['binding']['candidate_hash'],run['binding']['graph_hash'])
        self.direct('transact',self.transaction())
        historic=self.direct('inspect',{'entity_id':'missing','revision':1,'budget':4096},False)
        self.assertEqual((historic['base_revision'],historic['result_revision']),(2,1))
        self.assertTrue(all(item['base_revision']==1 for item in historic['diagnostics']))
        stale=self.direct('transact',self.transaction(),False)
        self.assertIn('E_STALE_REVISION',[d['code']for d in stale['diagnostics']]);self.assertEqual(stale['base_revision'],1)
        bound=self.receipt(stale['result']['run_id'])['binding'];self.assertEqual(bound['observed_head_revision'],2)
        budget=self.direct('inspect',{'entity_id':'demo.server','revision':1,'budget':1},False)
        self.assertEqual(budget['result']['status'],'BLOCKED');self.assertIn('demo.server',budget['result']['missing'])
        self.assertEqual((budget['base_revision'],budget['result_revision']),(2,1))
        explained=self.direct('explain',{'run_id':budget['result']['run_id'],'diagnostic_id':budget['diagnostics'][0]['diagnostic_id'],'context_budget':8192})
        self.assertEqual(explained['result']['diagnostic'],budget['diagnostics'][0])
        for failed in [first,second]:
            diagnostic=failed['diagnostics'][0]
            explanation=self.direct('explain',{'run_id':failed['result']['run_id'],'diagnostic_id':diagnostic['diagnostic_id'],'context_budget':8192})
            self.assertEqual(explanation['result']['diagnostic'],diagnostic);self.assertEqual(explanation['result_revision'],1)
        unknown=self.direct('explain',{'run_id':'run_'+'0'*64,'diagnostic_id':'diag_deadbeef','context_budget':4096},False)
        self.assertIn('E_NAME_NOT_FOUND',[d['code']for d in unknown['diagnostics']])
        identity=first['result']['run_id'];path=self.fixture.store/'.il-tools/runs'/(identity+'.json');original=path.read_bytes()
        try:
            path.write_bytes(original+b' ')
            tampered=self.fixture.invoke('explain',{'run_id':identity,'diagnostic_id':first['diagnostics'][0]['diagnostic_id'],'context_budget':4096},False)
            self.assertIn('E_STATE_INCONSISTENT',[d['code']for d in tampered['diagnostics']])
        finally:path.write_bytes(original)
        self.case['passed']=True;write(self.directory/'transcript.json',self.case)

    def test_bounded_route_diagnosis_repair_restore_and_receipts(self):
        self.acquire()
        rejected=self.invoke('transact',self.transaction(handler='demo.health'))
        self.assertFalse(rejected['ok']);self.assertEqual(rejected['result_revision'],1)
        diagnostic=rejected['diagnostics'][0]
        explanation=self.invoke('explain',{'run_id':rejected['result']['run_id'],'diagnostic_id':diagnostic['diagnostic_id'],'context_budget':8192})
        self.assertTrue(explanation['ok']);self.assertEqual(explanation['result']['diagnostic'],diagnostic)
        added=self.invoke('transact',self.transaction());self.assertTrue(added['ok']);self.assertEqual(added['result_revision'],2)
        built=self.invoke('build',{'revision':2,'target':'x86_64-unknown-linux-gnu','profile':'release','runtime_profile':'full'})
        self.assertTrue(built['ok']);self.fixture.validate_build(built,'full','release')
        checked=self.invoke('blackbox',{'revision':2,'contract':'http_mvp'});self.assertTrue(checked['ok'])
        # The source rejects forged or mismatched attestations before publication.
        invalid_evidence={'revision':1,'task_id':'P08-route','artifacts':[{'run_id':built['result']['run_id'],'role':'executable'}],
                          'tests':[checked['result']['run_id']]}
        rejected_evidence=self.fixture.invoke('evidence',invalid_evidence,False)
        self.assertIn('E_EVIDENCE_INCOMPLETE',[d['code']for d in rejected_evidence['diagnostics']])
        invalid_evidence['revision']=2;invalid_evidence['tests']=[built['result']['run_id']]
        rejected_evidence=self.fixture.invoke('evidence',invalid_evidence,False)
        self.assertIn('E_EVIDENCE_INCOMPLETE',[d['code']for d in rejected_evidence['diagnostics']])
        selected_run=self.receipt(built['result']['run_id']);exe=next(item for item in selected_run['artifacts'] if item['role']=='executable')
        blob=self.fixture.store/'.il-tools'/exe['path'];original=blob.read_bytes()
        try:
            blob.write_bytes(original+b'tampered')
            invalid_evidence['tests']=[checked['result']['run_id']]
            rejected_evidence=self.fixture.invoke('evidence',invalid_evidence,False)
            self.assertTrue({'E_STATE_INCONSISTENT','E_EVIDENCE_INCOMPLETE'}&{d['code']for d in rejected_evidence['diagnostics']})
        finally:blob.write_bytes(original)
        # Explain uses the failed candidate run after both a process restart and HEAD advancement.
        again=self.invoke('explain',{'run_id':rejected['result']['run_id'],'diagnostic_id':diagnostic['diagnostic_id'],'context_budget':8192})
        self.assertEqual(again['result_revision'],1);self.assertEqual(again['result']['diagnostic'],diagnostic)
        evidence=self.invoke('evidence',{'revision':2,'task_id':'P08-route','artifacts':[{'run_id':built['result']['run_id'],'role':'executable'}],
                                         'tests':[checked['result']['run_id']]})
        self.assertTrue(evidence['ok']);self.assertRegex(evidence['evidence_id'],r'^ev_[0-9a-f]{64}$')
        selection={'revision':2,'task_id':'P08-route','artifacts':[{'run_id':built['result']['run_id'],'role':'executable'}],'tests':[checked['result']['run_id']]}
        repeated=self.fixture.invoke('evidence',selection);self.assertEqual(repeated['evidence_id'],evidence['evidence_id'])
        exported=self.fixture.store/'.il-tools/evidence'/evidence['evidence_id']/'blobs'/exe['sha256'][7:]
        pristine=exported.read_bytes()
        try:
            exported.write_bytes(pristine+b'tampered')
            rejected_export=self.fixture.invoke('evidence',selection,False)
            self.assertIn('E_EVIDENCE_INCOMPLETE',[d['code']for d in rejected_export['diagnostics']])
        finally:exported.write_bytes(pristine)
        self.rebuild_bundle(evidence,built)
        restored=self.invoke('restore',{'revision':1,'reason':'P08 independent route rollback acceptance'});self.assertTrue(restored['ok']);self.assertEqual(restored['result_revision'],3)
        previous=self.invoke('inspect',{'entity_id':'demo.route.hello','revision':2,'budget':2048});self.assertTrue(previous['ok'])
        current=self.direct('inspect',{'entity_id':'demo.route.hello','revision':3,'budget':2048},False);self.assertFalse(current['ok'])
        self.assertEqual(current['result_revision'],3)
        rollback_build=self.invoke('build',{'revision':3,'target':'x86_64-unknown-linux-gnu','profile':'release','runtime_profile':'full'})
        self.live_route(rollback_build['result']['native']['artifacts']['executable']['path'],self.fixture.policy_path,'restored-new-revision',404)
        self.live_route(built['result']['native']['artifacts']['executable']['path'],self.fixture.policy_path,'old-verified-binary',200)
        state=json.loads((self.session_directory/'session.json').read_text(encoding='utf-8'))
        self.assertEqual(state['current_revision'],3);self.assertEqual(state['phases'],CONTRACT['context_phases'])
        self.assertLess(state['context_bytes'],self.task['resource_budget']['context_tokens'])
        self.assertEqual(state['tool_calls'],len(state['history']));self.assertEqual(len(state['runs']),state['tool_calls'])
        self.assertEqual(state['effects_delta'],[]);self.assertEqual(state['capabilities_delta'],[])
        self.assertFalse(any(row['tool']=='schema-check' or row['request'].get('entity_id')=='program' for row in self.case['steps']))
        self.case.update(passed=True,usage={key:state[key] for key in ['tool_calls','context_bytes','cpu_seconds']})
        write(self.directory/'transcript.json',self.case)


def main():
    global REPORT,BINARY
    parser=argparse.ArgumentParser();parser.add_argument('--binary',type=Path);parser.add_argument('--report',type=Path,required=True)
    args=parser.parse_args();REPORT=args.report.resolve();BINARY=args.binary.resolve() if args.binary else None
    suite=unittest.defaultTestLoader.loadTestsFromTestCase(TaskWorkerBoundary)
    if BINARY is not None:suite.addTests(unittest.defaultTestLoader.loadTestsFromTestCase(RealProtocolAcceptance))
    result=unittest.TextTestRunner(verbosity=2).run(suite)
    report={'suite':'ai_protocol_cli','scope':'worker_and_real_cli' if BINARY else 'worker_boundaries_only','state':'PASSED' if result.wasSuccessful() else 'FAILED','passed':result.wasSuccessful(),'count':result.testsRun,'cases':CASES,'contract_sha256':runner.digest(CONTRACT_PATH.read_bytes()),
            'failures':[{'test':str(t),'traceback':s}for t,s in result.failures],'errors':[{'test':str(t),'traceback':s}for t,s in result.errors]}
    write(REPORT,report);print(json.dumps({key:value for key,value in report.items() if key!='cases'}));return 0 if result.wasSuccessful() else 1


if __name__=='__main__':raise SystemExit(main())
