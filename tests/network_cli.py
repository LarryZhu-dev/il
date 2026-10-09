"""Independent P07 TCP process/authority/deadline acceptance; no HTTP parser."""
from __future__ import annotations
import argparse
import json
import socket
import sys
import threading
import time
import unittest
from pathlib import Path
import execution_cli
import graph_cli
import runtime_cli

ROOT = Path(__file__).resolve().parents[1]
CONTRACT = Path(__file__).with_name("network_cli_contracts.json")
REPORT = None
LOCKED = json.loads(CONTRACT.read_text(encoding="utf-8"))
CORE = runtime_cli.CORE_DECLARATIONS + '@id("net") module net visibility public {\n@id("net.Listener") type Listener=record {slot:U64,generation:U64} layout opaque;\n@id("net.Stream") type Stream=record {slot:U64,generation:U64} layout opaque;\n}'

def source(kind, endpoint, body, result="Bytes", parameters="data:Bytes", extra=""):
    return (CORE + '@id("app") module app imports [core,net] {'
        + '@id("Out") type Out=Result<' + result + ',core.IoError>;'
        + f'@id("endpoint") capability selected:{kind} scope {json.dumps(endpoint)};'
        + extra + '@id("main") fn main(' + parameters + ')->Out effects [net,alloc] capabilities [endpoint] {' + body + '}}')

def policy(kind, endpoint, faults=None):
    return {"schema_version":"1.0.0","grants":[{"entity_id":"endpoint","kind":kind,"scope":endpoint}],"test_faults":faults}

def bytes_arg(value):
    return {"type":"Bytes","data":{"kind":"bytes","value":value}}

class Peer:
    def __init__(self, action):
        self.action=action
        self.error=None
        self.record={}
        self.stop=threading.Event()
        self.thread=threading.Thread(target=self.run,daemon=True)
    def run(self):
        try:self.action(self.record,self.stop)
        except BaseException as error:self.error=error
    def __enter__(self):self.thread.start();return self
    def __exit__(self,kind,error,trace):
        if error is not None:self.stop.set()
        self.thread.join(6 if error is not None else 65)
        if self.thread.is_alive():raise AssertionError("external TCP peer did not stop")
        if error is None and self.error is not None:raise self.error

class NetworkCliAcceptance(unittest.TestCase):
    setUp=runtime_cli.RuntimeCliAcceptance.setUp
    git=graph_cli.GraphCliAcceptance.git
    head_revision=graph_cli.GraphCliAcceptance.head_revision
    invoke=runtime_cli.RuntimeCliAcceptance.invoke
    execute_request=runtime_cli.RuntimeCliAcceptance.execute_request
    scenario=runtime_cli.RuntimeCliAcceptance.scenario
    validate_build=runtime_cli.RuntimeCliAcceptance.validate_build
    records=[]
    def publish(self,text,expect_ok=True):
        return self.invoke("transact",{"task_id":"P07-network","base_revision":0,"scope":["program"],"operations":[{"op":"import_text","source":text}],"required_checks":[]},expect_ok)
    def progress(self):
        if REPORT is not None:runtime_cli.write_json(REPORT,{"suite":"network_cli_blackbox","state":"RUNNING","passed":False,"contract_sha256":runtime_cli.sha256(CONTRACT),"cases":self.records})
    def check(self,execution,expected):
        execution_cli.SCHEMA_CHECK.validate_file(ROOT,execution,"execution")
        self.assertEqual(execution["status"],"returned",execution)
        for field in ["live_handles","live_allocations"]:self.assertEqual(execution[field],expected[field],execution)
        self.assertEqual(execution["stdout"],[])
        self.assertEqual(execution["stderr"],[])
        self.assertEqual(execution["stack_trace"],[])
        self.assertEqual(execution["diagnostics"],[])
        value=execution["value"]["data"]["value"]
        self.assertEqual(value["tag"],expected["tag"])
        payload=value["fields"][0]["data"]
        if "bytes" in expected:self.assertEqual(payload,{"kind":"bytes","value":expected["bytes"]})
        elif "integer" in expected:self.assertEqual(payload,{"kind":"integer","value":expected["integer"]})
        else:self.assertEqual(payload,{"kind":"variant","value":{"tag":expected["error"],"fields":[]}})
    def test_connect_real_fragmented_roundtrip_and_native_startup_denial(self):
        with socket.socket() as listener:
            listener.bind(("127.0.0.1",0));listener.listen();listener.settimeout(1)
            endpoint="127.0.0.1:"+str(listener.getsockname()[1])
            faults={"allocation_fail_after":None,"io_max_chunk":LOCKED["io_max_chunk"],"io_fail_after":None,"accept_fail_after":None}
            body='let stream:net.Stream=runtime.net_connect[endpoint](core.Deadline::Infinite())?;let until:core.Deadline=core.Deadline::At(runtime.net_opened_at(stream)+5000000000);let mut offset:Usize=0;while offset<runtime.bytes_len(data){let count:Usize=runtime.net_write(stream,data,offset,until)?;if count==0{return Err(core.IoError::Write());}offset=offset+count;}let mut output:Bytes=runtime.bytes_slice(data,0,0)?;while runtime.bytes_len(output)<cast<Usize>(5){let chunk:Bytes=runtime.net_read(stream,5,until)?;if runtime.bytes_len(chunk)==cast<Usize>(0){return Err(core.IoError::Closed());}output=runtime.bytes_concat(output,chunk)?;}return Ok(output);'
            with self.scenario("connect_roundtrip") as row:
                self.policy_path=runtime_cli.write_json(self.root/"connect-policy.json",policy("Connect",endpoint,faults));self.publish(source("Connect",endpoint,body));row["executions"]={}
                for mode in LOCKED["modes"]:
                    def serve(record,stop):
                        deadline=time.monotonic()+60
                        while not stop.is_set():
                            try:accepted=listener.accept()[0];break
                            except socket.timeout:
                                if time.monotonic()>=deadline:raise
                        else:return
                        with accepted as peer:
                            peer.settimeout(5);received=b""
                            while len(received)<len(LOCKED["client_request"]):
                                chunk=peer.recv(64)
                                if not chunk:raise AssertionError("client closed before complete request")
                                received+=chunk
                            if received!=bytes(LOCKED["client_request"]):raise AssertionError(repr(received))
                            response=bytes(LOCKED["server_response"]);peer.sendall(response[:2]);time.sleep(.02);peer.sendall(response[2:]);record["received"]=list(received)
                    with Peer(serve) as peer:response=self.invoke("test",self.execute_request([bytes_arg(LOCKED["client_request"])],isolation=mode))
                    execution=response["result"]["execution"];self.check(execution,LOCKED["scenarios"]["connect_roundtrip"]);row["executions"][mode]={"execution":execution,"peer":peer.record}
                    if mode!="captured":
                        native=self.validate_build(response,"full",mode.removeprefix("native_"));executable=native["artifacts"]["executable"]["path"]
                        for label,grants in [("missing",[]),("wrong_kind",[{"entity_id":"endpoint","kind":"Listen","scope":endpoint}]),("wrong_scope",[{"entity_id":"endpoint","kind":"Connect","scope":"127.0.0.1:1"}])]:
                            deny=runtime_cli.write_json(self.root/(label+".json"),{"schema_version":"1.0.0","grants":grants,"test_faults":None});receipt=runtime_cli.run_native(executable,self.store/(mode+"-"+label),policy_path=deny);rejected=receipt["execution"]
                            self.assertEqual(receipt["exit_code"],101);self.assertEqual(rejected["diagnostics"][0]["code"],LOCKED["startup_error"]);self.assertEqual(rejected["steps"],0);self.assertEqual(rejected["live_handles"],0);self.assertEqual(rejected["live_allocations"],0);self.assertEqual(rejected["stdout"],[]);row.setdefault("startup_denials",[]).append({"mode":mode,"reason":label,"receipt":receipt})
                    self.progress()
    def test_accept_absolute_deadline_and_nested_owned_cleanup(self):
        for name in ["accept_timeout","nested_cleanup"]:
            with socket.socket() as probe:probe.bind(("127.0.0.1",0));port=probe.getsockname()[1]
            endpoint=f"127.0.0.1:{port}"
            start='let listener:net.Listener=runtime.net_listen[endpoint]()?;let stream:net.Stream=runtime.net_accept(listener,core.Deadline::Infinite())?;'
            if name=="accept_timeout":
                body=start+f'let until:core.Deadline=core.Deadline::At(runtime.net_opened_at(stream)+{LOCKED["accept_read_deadline_ms"]*1000000});return runtime.net_read(stream,1,until);';result="Bytes";extra=""
            else:
                body=start+'let pair:Pair=tuple(listener,stream);let count:I64=consume(pair);return Ok(count);';result="I64";extra='@id("Pair") type Pair=Tuple<net.Listener,net.Stream>;fn consume(pair:Pair)->I64 effects [net] {return 42;}'
            with self.scenario(name) as row:
                self.policy_path=runtime_cli.write_json(self.root/(name+"-policy.json"),policy("Listen",endpoint));self.publish(source("Listen",endpoint,body,result=result,parameters="",extra=extra));row["executions"]={}
                for mode in LOCKED["modes"]:
                    def client(record,stop):
                        deadline=time.monotonic()+60
                        while True:
                            if stop.is_set():return
                            peer=socket.socket();peer.settimeout(1)
                            try:peer.connect(("127.0.0.1",port));break
                            except ConnectionRefusedError:
                                peer.close()
                                if time.monotonic()>=deadline:raise
                                time.sleep(.01)
                        with peer:
                            record["connected_ns"]=time.monotonic_ns();peer.settimeout(5);received=peer.recv(1);record["closed_ns"]=time.monotonic_ns()
                            if received:raise AssertionError("fixture expected EOF without response")
                    with Peer(client) as peer:response=self.invoke("test",self.execute_request(isolation=mode))
                    execution=response["result"]["execution"];self.check(execution,LOCKED["scenarios"][name]);elapsed=(peer.record["closed_ns"]-peer.record["connected_ns"])/1000000
                    if name=="accept_timeout":self.assertGreaterEqual(elapsed,LOCKED["deadline_elapsed_min_ms"]);self.assertLess(elapsed,LOCKED["deadline_elapsed_max_ms"])
                    self.assertEqual(sum(event["kind"]=="opened" for event in execution["handle_events"]),2);self.assertEqual(sum(event["kind"]=="dropped" for event in execution["handle_events"]),2);row["executions"][mode]={"execution":execution,"peer":peer.record,"elapsed_ms":elapsed};self.progress()


def main():
    global REPORT
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--binary",type=Path,required=True);parser.add_argument("--report",type=Path,required=True);args=parser.parse_args();graph_cli.BINARY=args.binary.resolve();REPORT=args.report.resolve();runtime_cli.FAILURE_ROOT=REPORT.parent/(REPORT.stem+"-failed-artifacts")
    suite=unittest.defaultTestLoader.loadTestsFromTestCase(NetworkCliAcceptance);result=unittest.TextTestRunner(verbosity=2,stream=sys.stderr).run(suite)
    report={"suite":"network_cli_blackbox","state":"PASSED" if result.wasSuccessful() else "FAILED","passed":result.wasSuccessful(),"contract_sha256":runtime_cli.sha256(CONTRACT),"count":result.testsRun,"cases":NetworkCliAcceptance.records,"failures":[{"test":str(test),"traceback":message} for test,message in result.failures],"errors":[{"test":str(test),"traceback":message} for test,message in result.errors]};runtime_cli.write_json(REPORT,report);print(json.dumps({key:value for key,value in report.items() if key!="cases"}));return 0 if result.wasSuccessful() else 1
if __name__=="__main__":raise SystemExit(main())
