"""Independent TCP acceptance of compiled Intelligent language HTTP packages."""
from __future__ import annotations
import argparse, contextlib, hashlib, json, os, socket, struct, subprocess, sys, time, unittest
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'tests'))
import graph_cli, runtime_cli
import execution_cli
REPORT=None
RECORDS=[]
MODULES=('core','alloc','io','time','net','json','http','test','tracing')

def write_json(path,value):
    path=Path(path);path.parent.mkdir(parents=True,exist_ok=True);path.write_text(json.dumps(value,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
def package_source():return '\n'.join((ROOT/'packages'/name/'lib.il').read_text(encoding='utf-8') for name in MODULES)
def sha(path):return 'sha256:'+hashlib.sha256(Path(path).read_bytes()).hexdigest()
def locked():
    from contract import read_contract
    return read_contract(ROOT/'spec/http.yaml')

def internal_http_imports():
    roots=(ROOT/'tests/http_blackbox').glob('*.py')
    offenders=[]
    for path in roots:
        text=path.read_text(encoding='utf-8')
        for number,line in enumerate(text.splitlines(),1):
            stripped=line.strip()
            if stripped.startswith(('import packages.http','from packages.http','import http_runtime','from http_runtime')):
                offenders.append(f'{path.relative_to(ROOT)}:{number}')
    return offenders

def fixture_source():
    source=(ROOT/'examples/http_demo/main.il').read_text(encoding='utf-8')
    old='"handler":"demo.hello","parameters":[{"name":"name","type_ref":"String","source":"path","max_utf8_bytes":128}]}]'
    new='"handler":"demo.hello","parameters":[{"name":"name","type_ref":"String","source":"path","max_utf8_bytes":128}]},{"entity_id":"demo.route.failure","method":"GET","path":"/failure","handler":"demo.failure","parameters":[]},{"entity_id":"demo.route.large","method":"GET","path":"/large","handler":"demo.large","parameters":[]}]'
    if old not in source:raise AssertionError('Cannot locate the explicit demo route declaration')
    source=source.replace(old,new)
    definitions="""
 @id("demo.failure") fn failure(request:http.EmptyRequest)->http.ResponseResult effects [alloc] {return Err(http.HttpError::Handler());}
 @id("demo.large") fn large(request:http.EmptyRequest)->http.ResponseResult effects [alloc] {
  let initial:http.BytesResult=runtime.string_to_bytes("BODY_SEED");match initial{Err(error)=>{return Err(http.HttpError::Io(error));}Ok(bytes)=>{
   let mut body:Bytes=bytes;let mut count:Usize=0;while count<10 {let next:http.BytesResult=runtime.bytes_concat(body,body);match next{Err(error)=>{return Err(http.HttpError::Io(error));}Ok(value)=>{body=value;}}count=count+1;}
   return Ok(http.Response::Response(200,"application/octet-stream",body));
  }}
 }
"""
    source=source.rstrip();assert source.endswith('}')
    return package_source()+'\n'+source[:-1]+definitions.replace('BODY_SEED','x'*8192)+'\n}\n'

def request(target,method='GET',headers=None):
    entries=[('Host','localhost')]+([] if headers is None else headers)
    return (f'{method} {target} HTTP/1.1\r\n'+''.join(f'{name}: {value}\r\n' for name,value in entries)+'\r\n').encode()

def exchange(raw,*,fragments=None,delays=None,timeout=8):
    started=time.monotonic_ns()
    with socket.create_connection(('127.0.0.1',8080),timeout=timeout) as client:
        client.settimeout(timeout);chunks=[raw] if fragments is None else fragments
        for index,chunk in enumerate(chunks):
            if delays is not None and delays[index]:time.sleep(delays[index])
            client.sendall(chunk)
        data=bytearray()
        while True:
            try:chunk=client.recv(65536)
            except ConnectionResetError:
                if data:break
                raise
            if not chunk:break
            data.extend(chunk)
    return bytes(data),time.monotonic_ns()-started

def response(wire):
    head,sep,body=wire.partition(b'\r\n\r\n')
    if not sep:raise AssertionError(f'No HTTP header terminator: {wire[:200]!r}')
    lines=head.split(b'\r\n');parts=lines[0].split(b' ',2)
    if len(parts)!=3 or parts[0]!=b'HTTP/1.1':raise AssertionError('Invalid response status line')
    status=int(parts[1]);headers={}
    for line in lines[1:]:
        name,colon,value=line.partition(b':')
        if not colon:raise AssertionError('Invalid response header')
        key=name.decode('ascii').lower()
        if key in headers:raise AssertionError('Duplicate response header')
        headers[key]=value.strip().decode('ascii')
    rules=locked()['responses']
    for name in rules['mandatory_headers']:
        if name.lower() not in headers:raise AssertionError('Missing mandatory header: '+name)
    if headers.get('connection')!=rules['connection']:raise AssertionError('Missing close contract')
    if int(headers.get('content-length','-1'))!=len(body):raise AssertionError('Content-Length differs from actual bytes')
    return status,headers,body

@contextlib.contextmanager
def server(executable,policy,directory):
    directory=Path(directory);directory.mkdir(parents=True,exist_ok=True)
    probe=socket.socket()
    # Match the listener's bind rules: TIME_WAIT is reusable, a live listener is not.
    probe.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1)
    try:probe.bind(('127.0.0.1',8080))
    finally:probe.close()
    import fcntl
    policy_file=Path(policy).open('rb')
    source_fd=fcntl.fcntl(policy_file.fileno(),fcntl.F_DUPFD_CLOEXEC,10)
    stderr=(directory/'stderr.log').open('wb');stdout=(directory/'stdout.log').open('wb')
    def descriptors():os.dup2(source_fd,4);os.set_inheritable(4,True)
    process=subprocess.Popen([str(executable)],stdin=subprocess.DEVNULL,stdout=stdout,stderr=stderr,env={},close_fds=False,preexec_fn=descriptors)
    try:
        deadline=time.monotonic()+10
        while True:
            if process.poll() is not None:raise AssertionError(f'Application exited before listen: {process.returncode}')
            try:
                wire,_=exchange(request('/health'));status,_,body=response(wire)
                health=locked()['responses']['health']
                if status!=health['status'] or body!=health['body'].encode():raise AssertionError('Startup health contract failed')
                break
            except ConnectionRefusedError:
                if time.monotonic()>deadline:raise
                time.sleep(.02)
        yield process
    finally:
        if process.poll() is None:process.terminate()
        try:process.wait(timeout=5)
        except subprocess.TimeoutExpired:process.kill();process.wait(timeout=5)
        stdout.close();stderr.close();policy_file.close();os.close(source_fd)
        write_json(directory/'process.json',{'argv':[str(executable)],'sha256':sha(executable),'exit_code':process.returncode,'stdout_sha256':sha(directory/'stdout.log'),'stderr_sha256':sha(directory/'stderr.log')})

class HttpAcceptance(unittest.TestCase):
    def test_external_client_has_no_internal_http_imports(self):
        self.assertEqual(internal_http_imports(),[])

    def fixture(self,name,source,*,faults=None):
        fixture=runtime_cli.RuntimeCliAcceptance('test_real_files_handles_and_cleanup');fixture.setUp();self.addCleanup(fixture.doCleanups)
        directory=REPORT.parent/'http-artifacts'/str(time.time_ns())/name;directory.mkdir(parents=True,exist_ok=True);fixture.store=directory/'store'
        fixture.policy_path=directory/'policy.json'
        write_json(fixture.policy_path,{'schema_version':'1.0.0','grants':[{'entity_id':'demo.listen','kind':'Listen','scope':'127.0.0.1:8080'},{'entity_id':'demo.clock','kind':'ClockRead','scope':None}],'test_faults':faults})
        fixture.publish(source)
        return fixture,directory
    def record(self,identity,wire,elapsed,expected):
        status,headers,body=response(wire);self.assertEqual(status,expected)
        row={'id':identity,'status':status,'headers':headers,'body':list(body),'elapsed_ns':elapsed,'wire_sha256':'sha256:'+hashlib.sha256(wire).hexdigest()};RECORDS.append(row)
        write_json(REPORT,{'state':'RUNNING','passed':False,'cases':RECORDS});return headers,body
    def test_native_external_tcp_contracts(self):
        rules=locked();source=fixture_source();errors=rules['responses']['errors'];health=rules['responses']['health'];hello=rules['responses']['hello'];limits=rules['limits'];read_seconds=limits['read_deadline_ms']/1000;write_seconds=limits['write_deadline_ms']/1000;empty_body=rules['responses']['error_body'];assert empty_body=='empty'
        for profile in ('debug','release'):
            with self.subTest(profile=profile):
                fixture,directory=self.fixture(profile,source)
                native=fixture.validate_build(fixture.invoke('build',fixture.build_request('full',profile)),'full',profile)
                executable=native['artifacts']['executable']['path']
                with server(executable,fixture.policy_path,directory) as process:
                    happy=[('/health',health['status'])]+[(path,hello['status']) for path in ['/hello/Larry','/hello/%E4%B8%AD','/hello/%22%5C%0A','/hello/%2541']]+[(path,errors['unknown_path']) for path in ['/unknown','/health/','/hello/','/hello/a/b']]+[('/failure',errors['handler_error'])]
                    for target,expected in happy:
                        wire,elapsed=exchange(request(target));headers,body=self.record(profile+target,wire,elapsed,expected)
                        if target=='/health':self.assertEqual(body,rules['responses']['health']['body'].encode());self.assertEqual(headers['content-type'],rules['responses']['health']['content_type'])
                        if target.startswith('/hello/') and expected==hello['status']:
                            import urllib.parse
                            name=urllib.parse.unquote(target.split('/')[-1]);self.assertEqual(json.loads(body),{key:value.replace('Larry',name) for key,value in hello['body_json'].items()});self.assertEqual(headers['content-type'],rules['responses']['hello']['content_type'])
                        if expected>=400:self.assertEqual(body,b'')
                    cases=[
                        ('method',request('/health','POST'),errors['wrong_method']),
                        ('connect',request('host:443','CONNECT'),errors['connect']),
                        ('query',request('/health?route=/unknown'),health['status']),
                        ('percent',request('/hello/%GG'),errors['invalid_percent_encoding']),
                        ('slash',request('/hello/%2F'),errors['invalid_percent_encoding']),
                        ('nul',request('/hello/%00'),errors['invalid_percent_encoding']),
                        ('utf8',request('/hello/%FF'),errors['invalid_utf8']),
                        ('long_parameter',request('/hello/'+'a'*(limits['path_parameter_max_utf8_bytes']+1)),errors['parameter_too_long']),
                        ('no_host',b'GET /health HTTP/1.1\r\n\r\n',errors['malformed_request']),
                        ('two_host',request('/health',headers=[('hOsT','other')]),errors['malformed_request']),
                        ('empty_host',b'GET /health HTTP/1.1\r\nHost: \t\r\n\r\n',errors['malformed_request']),
                        ('two_lengths',request('/health',headers=[('Content-Length','0'),('content-length','0')]),errors['conflicting_length_headers']),
                        ('conflict',request('/health',headers=[('Content-Length','1'),('Content-Length','0')]),errors['conflicting_length_headers']),
                        ('body',request('/health',headers=[('Content-Length','1')]),errors['nonzero_body']),
                        ('transfer',request('/health',headers=[('Transfer-Encoding','chunked')]),errors['transfer_encoding']),
                        ('upgrade',request('/health',headers=[('Upgrade','websocket')]),errors['upgrade']),
                        ('fold',b'GET /health HTTP/1.1\r\nHost: x\r\n folded\r\n\r\n',errors['malformed_request']),
                        ('bare_lf',b'GET /health HTTP/1.1\nHost: x\n\n',errors['malformed_request']),
                        ('header_bare_lf',b'GET /health HTTP/1.1\r\nHost: x\n\n',errors['malformed_request']),
                        ('header_bad_cr',b'GET /health HTTP/1.1\r\nHost: x\rX',errors['malformed_request']),
                        ('version',b'GET /health HTTP/1.0\r\nHost: x\r\n\r\n',errors['unsupported_http_version']),
                        ('bad_version',b'GET /health nonsense\r\nHost: x\r\n\r\n',errors['malformed_request']),
                        ('line_limit',request('/'+'x'*limits['request_line_max_bytes']),errors['request_line_too_long']),
                        ('header_limit',request('/health',headers=[('X-Fill','x'*limits['header_section_max_bytes'])]),errors['headers_too_large']),
                    ]
                    for name,raw,expected in cases:
                        wire,elapsed=exchange(raw);headers,body=self.record(profile+':'+name,wire,elapsed,expected)
                        if expected==errors['wrong_method']:
                            name,_,value=rules['responses']['method_not_allowed_header'].partition(':');self.assertEqual(headers[name.lower()],value.strip())
                        if expected>=400:self.assertEqual(body,b'')
                    wire,elapsed=exchange(b'G'*(limits['request_line_max_bytes']+1));self.record(profile+':immediate_line_limit',wire,elapsed,errors['request_line_too_long']);self.assertLess(elapsed,2_000_000_000)
                    prefix=b'GET /health?';suffix=b' HTTP/1.1';line=prefix+b'x'*(limits['request_line_max_bytes']-len(prefix)-len(suffix))+suffix
                    wire,elapsed=exchange(b'',fragments=[line+b'\r',b'\nHost: x\r\n\r\n'],delays=[0,.05]);self.record(profile+':exact_line_limit_fragmented_crlf',wire,elapsed,health['status'])
                    raw=request('/hello/fragmented');wire,elapsed=exchange(raw,fragments=[raw[i:i+1] for i in range(len(raw))]);self.record(profile+':fragmented',wire,elapsed,hello['status'])
                    wire,elapsed=exchange(b'GET /health HTTP/1.1\r\nHost:');self.record(profile+':read_timeout',wire,elapsed,errors['read_timeout']);self.assertGreaterEqual(elapsed,int((read_seconds-.5)*1e9));self.assertLess(elapsed,int((read_seconds+3)*1e9))
                    wire,elapsed=exchange(b'',fragments=[b'GET /health HTTP/1.1\r\n',b'Host:',b' x'],delays=[0,read_seconds*.4,read_seconds*.4]);self.record(profile+':absolute_deadline',wire,elapsed,errors['read_timeout']);self.assertLess(elapsed,int((read_seconds+1.5)*1e9))
                    with socket.create_connection(('127.0.0.1',8080)) as peer:peer.sendall(b'GET /health');peer.setsockopt(socket.SOL_SOCKET,socket.SO_LINGER,struct.pack('ii',1,0))
                    wire,elapsed=exchange(request('/health'));self.record(profile+':after_disconnect',wire,elapsed,health['status'])
                    before=(directory/'stderr.log').stat().st_size;started=time.monotonic()
                    with socket.socket() as slow:
                        slow.setsockopt(socket.SOL_SOCKET,socket.SO_RCVBUF,1024);slow.connect(('127.0.0.1',8080));slow.sendall(request('/large'))
                        while b'E_HTTP_WRITE_TIMEOUT' not in (directory/'stderr.log').read_bytes()[before:]:
                            self.assertIsNone(process.poll());self.assertLess(time.monotonic()-started,write_seconds+4);time.sleep(.05)
                        self.assertGreater(time.monotonic()-started,write_seconds-.5)
                    wire,elapsed=exchange(request('/health'));self.record(profile+':after_write_timeout',wire,elapsed,health['status'])
                    logs=(directory/'stderr.log').read_text();self.assertIn('E_HTTP_REQUEST_FAILED',logs);self.assertIn('E_HTTP_WRITE_TIMEOUT',logs)
                    for line in logs.splitlines():self.assertIn('code',json.loads(line))
    def test_captured_partial_io_uses_real_tcp_and_releases_resources(self):
        source=package_source()+'\n'+(ROOT/'examples/http_demo/main.il').read_text(encoding='utf-8')
        for profile in ('debug','release'):
            faults={'allocation_fail_after':None,'io_max_chunk':2,'io_fail_after':None,'accept_fail_after':None}
            fixture,directory=self.fixture(profile+'-partial',source,faults=faults)
            request_data=fixture.execute_request(isolation='native_'+profile);request_data['suite']['entry']='demo.serve_once';request_data['suite']['limits']['max_steps']=1_000_000;request_data['suite']['limits']['max_output_bytes']=1_048_576
            command=[str(graph_cli.BINARY),'--repository',str(fixture.repository),'--store',str(fixture.store),'--host-policy',str(fixture.policy_path),'test']
            process=subprocess.Popen(command,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
            try:
                process.stdin.write(json.dumps(request_data));process.stdin.close();process.stdin=None
                end=time.monotonic()+240
                while True:
                    if process.poll() is not None:raise AssertionError('Captured server exited before accept: '+process.stdout.read()[:2000])
                    try:wire,elapsed=exchange(request('/health'));break
                    except ConnectionRefusedError:self.assertLess(time.monotonic(),end);time.sleep(.05)
                self.record(profile+':partial_tcp',wire,elapsed,locked()['responses']['health']['status'])
                stdout,stderr=process.communicate(timeout=15);reply=json.loads(stdout)
                if reply['ok']:
                    self.assertEqual(process.returncode,0);retained=reply['result']
                else:
                    self.assertEqual([item['code'] for item in reply['diagnostics']],['E_CONTEXT_INSUFFICIENT'])
                    candidates=list(fixture.store.glob('.il/builds/candidates/native-*/test-result.json'))
                    retained_path=max(candidates,key=lambda path:path.stat().st_mtime_ns)
                    retained=json.loads(retained_path.read_text(encoding='utf-8'))
                    self.assertEqual(retained['native']['profile'],profile)
                execution=retained['execution'];self.assertEqual(execution['status'],'returned');self.assertEqual(execution['live_handles'],0);self.assertEqual(execution['live_allocations'],0)
                self.assertGreaterEqual(len(execution['handle_events']),4);write_json(directory/(profile+'-execution.json'),retained)
            finally:
                if process.poll() is None:process.kill();process.wait(timeout=5)

    def test_captured_accept_failure_is_external_diagnostic_and_cleans_up(self):
        """Exercise accept faults through the compiled captured service boundary.

        A zero threshold must return without waiting for a client.  A threshold
        of one accepts a real external `/health` request and then fails on the
        next accept.  Both cases run the native captured executable, so the
        report is independent of the interpreter and retains process evidence.
        """
        rules=locked()['responses'];diagnostic_code=rules['diagnostics']['accept_failure']
        source=package_source()+'\n'+(ROOT/'examples/http_demo/main.il').read_text(encoding='utf-8')
        for profile in ('debug','release'):
            for threshold,maximum in ((0,1),(1,2)):
                identity=f'accept-failure-{profile}-{threshold}'
                faults={'allocation_fail_after':None,'io_max_chunk':None,'io_fail_after':None,'accept_fail_after':threshold}
                fixture,directory=self.fixture(identity,source,faults=faults)
                request_data=fixture.execute_request(isolation='native_'+profile)
                request_data['suite']['entry']='demo.serve'
                request_data['suite']['arguments']=[{'type':'Usize','data':{'kind':'integer','value':str(maximum)}}]
                request_data['suite']['limits']['max_steps']=1_000_000
                request_data['suite']['limits']['max_output_bytes']=1_048_576
                command=[str(graph_cli.BINARY),'--repository',str(fixture.repository),'--store',str(fixture.store),'--host-policy',str(fixture.policy_path),'test']
                process=subprocess.Popen(command,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
                started=time.monotonic_ns()
                try:
                    process.stdin.write(json.dumps(request_data));process.stdin.close();process.stdin=None
                    wire=None;elapsed=None
                    if threshold==1:
                        # The first accepted stream is driven by an independent
                        # TCP client; after it closes the second accept fails.
                        deadline=time.monotonic()+30
                        while True:
                            self.assertIsNone(process.poll(),'native service exited before external accept')
                            try:
                                wire,elapsed=exchange(request('/health'),timeout=2)
                                break
                            except (ConnectionRefusedError,ConnectionResetError,socket.timeout,TimeoutError):
                                self.assertLess(time.monotonic(),deadline,'native service did not open a TCP listener')
                                time.sleep(.02)
                        self.record(identity+':health',wire,elapsed,rules['health']['status'])
                    # Running the complete captured native request reparses and
                    # lowers the large HTTP package before executing it. Keep
                    # this deadline aligned with the other native black-box
                    # subprocesses so a slow clean build is not misreported as
                    # a service hang.
                    stdout,stderr=process.communicate(timeout=180)
                    reply=json.loads(stdout)
                    self.assertEqual(process.returncode,0,stderr)
                    self.assertTrue(reply['ok'],reply)
                    retained=reply['result'];execution=retained['execution']
                    execution_cli.SCHEMA_CHECK.validate_file(ROOT,execution,'execution')
                    self.assertEqual(execution['status'],'returned')
                    self.assertEqual(execution['value']['data'],{'kind':'integer','value':'1'})
                    self.assertEqual(execution['live_handles'],0)
                    self.assertEqual(execution['live_allocations'],0)
                    self.assertEqual(execution['diagnostics'],[])
                    diagnostic=json.loads(bytes(execution['stderr']))
                    self.assertEqual(diagnostic,{'code':diagnostic_code})
                    events=execution['handle_events']
                    # Every owned resource is reported: the listener plus one
                    # accepted stream when the positive threshold is used.
                    expected_resources=1 + (1 if threshold else 0)
                    self.assertEqual(sum(event['kind']=='opened' for event in events),expected_resources)
                    self.assertEqual(sum(event['kind']=='dropped' for event in events),expected_resources)
                    native=retained['native'];executable=Path(native['argv'][0])
                    self.assertTrue(executable.is_file())
                    executable_sha=sha(executable);policy_sha=sha(fixture.policy_path)
                    policy_value=json.loads(fixture.policy_path.read_text(encoding='utf-8'))
                    self.assertEqual(retained['execution_input']['policy_hash'],runtime_cli.policy_hash(policy_value))
                    evidence=directory/'accept-failure.json'
                    write_json(evidence,{'schema_version':'1.0.0','case':identity,'threshold':threshold,'maximum':maximum,
                        'command':command,'pid':process.pid,'exit_code':process.returncode,'elapsed_ns':time.monotonic_ns()-started,
                        'executable':{'path':str(executable),'sha256':executable_sha},'policy':{'path':str(fixture.policy_path),'sha256':policy_sha},
                        'report_path':str(evidence),'expected_code':diagnostic_code,'actual_code':diagnostic['code'],
                        'execution':execution})
                    RECORDS.append({'id':identity+':fault','threshold':threshold,'status':'returned','diagnostic':diagnostic_code,
                                    'execution_schema':'execution.schema.json','executable_sha256':executable_sha,'policy_sha256':policy_sha,
                                    'report_sha256':sha(evidence),'process_pid':process.pid,'exit_code':process.returncode})
                    write_json(REPORT,{'suite':'http_external_blackbox','state':'RUNNING','passed':False,'cases':RECORDS})
                finally:
                    if process.poll() is None:
                        process.kill();process.wait(timeout=5)
                # A returned accept failure must release its listener.  This
                # also ensures threshold zero never leaves a hidden blocker.
                with socket.socket() as probe:
                    probe.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1)
                    probe.bind(('127.0.0.1',8080))

def main():
    global REPORT
    parser=argparse.ArgumentParser();parser.add_argument('--binary',type=Path,required=True);parser.add_argument('--report',type=Path,required=True);args=parser.parse_args();REPORT=args.report.resolve();graph_cli.BINARY=args.binary.resolve()
    report={'suite':'http_external_blackbox','state':'RUNNING','passed':False,'contract_sha256':sha(ROOT/'spec/http.yaml'),'cases':[]};write_json(REPORT,report)
    result=unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(HttpAcceptance))
    report.update(state='PASSED' if result.wasSuccessful() else 'FAILED',passed=result.wasSuccessful(),count=result.testsRun,cases=RECORDS,failures=[{'test':str(t),'traceback':s} for t,s in result.failures],errors=[{'test':str(t),'traceback':s} for t,s in result.errors]);write_json(REPORT,report);print(json.dumps({k:v for k,v in report.items() if k!='cases'}));return 0 if result.wasSuccessful() else 1
if __name__=='__main__':raise SystemExit(main())
