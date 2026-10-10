"""TCP evidence tests without a listener or native compiler dependency."""
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import socket
import subprocess
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
SPEC=importlib.util.spec_from_file_location('http_receipt_cli',HERE/'cli.py')
CLI=importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CLI)


class Peer:
    def __init__(self,response=b'',max_send=2,fail_after=None):
        self.response=response;self.sent=bytearray();self.max_send=max_send;self.fail_after=fail_after
    def __enter__(self):return self
    def __exit__(self,*args):pass
    def settimeout(self,value):pass
    def setsockopt(self,*args):pass
    def send(self,data):
        if self.fail_after is not None and len(self.sent)>=self.fail_after:raise BrokenPipeError('test peer closed')
        count=min(len(data),self.max_send);self.sent.extend(data[:count]);return count
    def recv(self,count):
        data=self.response[:3];self.response=self.response[3:];return data


class HttpReceiptTests(unittest.TestCase):
    def setUp(self):
        self.temporary=tempfile.TemporaryDirectory();self.addCleanup(self.temporary.cleanup)
        self.root=Path(self.temporary.name)
        self.patches=[patch.object(CLI,'REPORT',self.root/'report.json'),patch.object(CLI,'EXCHANGES',[]),
                      patch.object(CLI,'RECORDS',[]),patch.object(CLI,'ACTIVE_BINDING',None),patch.object(CLI,'LAST_EXCHANGE',None)]
        for replacement in self.patches:replacement.start();self.addCleanup(replacement.stop)

    def read_artifact(self,artifact):
        data=Path(artifact['path']).read_bytes()
        self.assertEqual(artifact['sha256'],'sha256:'+hashlib.sha256(data).hexdigest())
        self.assertEqual(artifact['size_bytes'],len(data))
        return data

    def test_actual_bytes_fragments_delays_and_monotonic_times_survive_partial_writes(self):
        peer=Peer(b'wire\x00\xffresponse')
        with patch.object(CLI.socket,'create_connection',return_value=peer):
            wire,elapsed=CLI.exchange(b'not the transmitted input',fragments=[b'GET ',b'/health\xff'],delays=[0,.001])
        record=CLI.EXCHANGES[0]
        self.assertEqual(self.read_artifact(record['request']),bytes(peer.sent))
        self.assertEqual(bytes(peer.sent),b'GET /health\xff')
        self.assertEqual(self.read_artifact(record['response']),wire)
        self.assertEqual(elapsed,record['ended_ns']-record['started_ns'])
        self.assertEqual(record['requested_delays_seconds'],[0,.001])
        for event in record['events']:
            self.assertGreaterEqual(event['started_ns'],record['started_ns'])
            self.assertLessEqual(event['ended_ns'],record['ended_ns'])
            self.assertGreaterEqual(event['ended_ns'],event['started_ns'])
        self.assertEqual(b''.join(self.read_artifact(e['bytes']) for e in record['events'] if e['direction']=='send'),bytes(peer.sent))
        self.assertEqual(b''.join(self.read_artifact(e['bytes']) for e in record['events'] if e['direction']=='receive'),wire)

    def test_failed_send_preserves_only_actual_sent_prefix_and_pending_suite(self):
        peer=Peer(fail_after=2)
        with patch.object(CLI.socket,'create_connection',return_value=peer):
            with self.assertRaises(BrokenPipeError):CLI.exchange(b'abcdef')
        record=CLI.EXCHANGES[0]
        self.assertEqual(self.read_artifact(record['request']),b'ab')
        self.assertEqual(record['error']['type'],'BrokenPipeError')
        report=json.loads(CLI.REPORT.read_text())
        self.assertFalse(report['passed']);self.assertEqual(report['state'],'RUNNING')
        self.assertEqual(report['exchanges'][0]['terminal'],'error')

    def test_os_socket_exchange_preserves_real_bytes_without_listening_port(self):
        left,right=socket.socketpair();self.addCleanup(left.close);self.addCleanup(right.close)
        outgoing=b'actual request\x00\xff';incoming=b'actual response\xff\x00'
        observed=[]
        def peer():
            with right:
                collected=bytearray()
                while len(collected)<len(outgoing):collected.extend(right.recv(len(outgoing)-len(collected)))
                observed.append(bytes(collected));right.sendall(incoming)
        worker=threading.Thread(target=peer,daemon=True);worker.start()
        with patch.object(CLI.socket,'create_connection',return_value=left):
            wire,_=CLI.exchange(outgoing)
        worker.join(timeout=3);self.assertFalse(worker.is_alive())
        self.assertEqual(observed,[outgoing]);self.assertEqual(wire,incoming)
        record=CLI.EXCHANGES[0]
        self.assertEqual(self.read_artifact(record['request']),outgoing)
        self.assertEqual(self.read_artifact(record['response']),incoming)

    def test_wrapper_receipt_retains_real_process_exit_and_streams(self):
        command=[sys.executable,'-c','import sys; print("stdout receipt"); print("stderr receipt", file=sys.stderr); sys.exit(7)']
        started=time.monotonic_ns()
        process=subprocess.Popen(command,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        stdout,stderr=process.communicate(timeout=10)
        binding={};receipt={'binding':binding}
        CLI.captured_process_receipt(process,command,started,stdout,stderr,self.root,[receipt],binding)
        retained=json.loads(self.read_artifact(binding['process_receipt']))
        self.assertEqual(retained['pid'],process.pid);self.assertEqual(retained['exit_code'],7)
        self.assertEqual(retained['kind'],'compiler_test_wrapper')
        self.assertEqual(self.read_artifact(retained['stdout']),stdout.encode())
        self.assertEqual(self.read_artifact(retained['stderr']),stderr.encode())
        self.assertEqual(receipt['wrapper_process_receipt'],binding['process_receipt'])
        self.assertEqual(retained['supervisor_cleanup'],
                         {'sigterm_sent':False,'sigkill_sent':False,'reaped':True,'unexpected_exit':False})

    def test_shared_wire_client_works_without_report_recorder(self):
        """P08 imports this module only for TCP I/O, without P10 recording."""
        with patch.object(CLI, 'REPORT', None), patch.object(CLI.socket, 'create_connection', return_value=Peer(b'')):
            wire, _ = CLI.exchange(b'wire-only-client')
        self.assertEqual(wire, b'')
        self.assertIsNone(CLI.EXCHANGES[0]['request'])

    def test_supervisor_cleanup_records_signal_and_reap(self):
        process=subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)'])
        cleanup=CLI.supervise_cleanup(process,timeout=3)
        self.assertTrue(cleanup['sigterm_sent'])
        self.assertFalse(cleanup['sigkill_sent'])
        self.assertTrue(cleanup['reaped'])
        self.assertFalse(cleanup['unexpected_exit'])

    def test_content_addressed_artifact_tampering_is_rejected(self):
        artifact=CLI.artifact_bytes(b'original')
        Path(artifact['path']).write_bytes(b'tampered')
        with self.assertRaisesRegex(AssertionError,'modified'):CLI.artifact_bytes(b'original')

    def test_startup_is_not_a_case_and_disconnect_does_not_claim_read_response(self):
        with patch.object(CLI.socket,'create_connection',side_effect=[Peer(b'ready'),Peer(b'unread')]):
            CLI.exchange(b'probe',phase='startup')
            CLI.exchange(b'GET /health',mode='reset')
        self.assertEqual(CLI.RECORDS,[])
        self.assertEqual(CLI.EXCHANGES[0]['phase'],'startup')
        self.assertFalse(CLI.EXCHANGES[1]['response_read'])
        self.assertEqual(CLI.EXCHANGES[1]['terminal'],'client_reset')
        self.assertEqual(self.read_artifact(CLI.EXCHANGES[1]['response']),b'')

    def test_captured_binding_uses_retained_variant_not_standalone_binary(self):
        policy=self.root/'policy.json';policy.write_text(json.dumps({'grants':[],'test_faults':None}))
        standalone=self.root/'standalone';standalone.write_bytes(b'standalone code')
        captured=self.root/'captured';captured.write_bytes(b'captured partial io code')
        with patch.object(CLI.socket,'create_connection',return_value=Peer(b'result')):
            CLI.exchange(b'request')
        self.assertIsNone(CLI.EXCHANGES[0]['binding'])
        retained={'native':{'argv':[str(captured)],'profile':'debug','artifacts':{'executable':{'sha256':CLI.sha(captured)}}},
                  'execution_input':{'policy_hash':CLI.runtime_cli.policy_hash(json.loads(policy.read_text()))}}
        binding=CLI.bind_captured(CLI.EXCHANGES,retained,policy,'debug')
        self.assertEqual(binding['executable']['sha256'],CLI.sha(captured))
        self.assertNotEqual(binding['executable']['sha256'],CLI.sha(standalone))
        self.assertEqual(json.loads(self.read_artifact(binding['native_result'])),retained)
        retained['native']['artifacts']['executable']['sha256']=CLI.sha(standalone)
        with self.assertRaisesRegex(AssertionError,'differs'):CLI.bind_captured(CLI.EXCHANGES,retained,policy,'debug')

    def test_status_record_is_pending_until_all_body_assertions_and_suite_pass(self):
        raw=b'HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nwrong'
        with patch.object(CLI.socket,'create_connection',return_value=Peer(raw)):
            wire,elapsed=CLI.exchange(b'GET /health')
        case=CLI.HttpAcceptance('test_external_client_has_no_internal_http_imports')
        _,body=case.record('health',wire,elapsed,200)
        with self.assertRaises(AssertionError):case.assertEqual(body,b'ok')
        self.assertEqual(CLI.RECORDS[0]['validation'],'observed_pending_suite')
        self.assertFalse(json.loads(CLI.REPORT.read_text())['passed'])


if __name__=='__main__':unittest.main()
