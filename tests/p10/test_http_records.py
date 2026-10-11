import hashlib
import json
import tempfile
import unittest
from pathlib import Path

from tools.p10 import http_records, runner


def digest(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


def minimal_elf():
    data = bytearray(64)
    data[:4] = b"\x7fELF"
    data[4:7] = bytes((2, 1, 1))
    data[16:18] = (2).to_bytes(2, "little")
    data[18:20] = (62).to_bytes(2, "little")
    return bytes(data)


class Fixture:
    def __init__(self, root):
        self.root = root
        self.contract, _ = runner.load_contract()
        self.spec_hash = digest((runner.ROOT / "spec/http.yaml").read_bytes())
        self.elf = self.file("bin/app.elf", minimal_elf())

    def file(self, name, data):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return path

    def ref(self, name, data, *, size=False):
        self.file(name, data)
        result = {"path": name, "sha256": digest(data)}
        if size:
            result["size_bytes"] = len(data)
        return result

    def build(self, *, omit=None):
        rows = []
        for profile in self.contract["required_http_profiles"]:
            for case in self.contract["http_cases"]:
                if (case, profile) == omit:
                    continue
                request = b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n"
                response = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                req = self.ref(f"bytes/{profile}-{case}-request", request, size=True)
                resp = self.ref(f"bytes/{profile}-{case}-response", response, size=True)
                planned = self.ref(f"bytes/{profile}-{case}-planned", request, size=True)
                policy_bytes = json.dumps({"profile": profile}, sort_keys=True).encode()
                policy = self.ref(f"policy/{profile}.json", policy_bytes)
                stdout = self.ref(f"bytes/{profile}-{case}-stdout", b"", size=True)
                stderr_data = b"E_HTTP_WRITE_TIMEOUT" if case == "partial_write" else b""
                stderr = self.ref(f"bytes/{profile}-{case}-stderr", stderr_data, size=True)
                exchange = {
                    "schema_version": "1.0.0", "kind": "tcp_exchange", "exchange_id": f"{profile}-{case}",
                    "phase": "case", "peer": ["127.0.0.1", 8080], "started_ns": 10, "ended_ns": 20,
                    "elapsed_ns": 10, "timeout_seconds": 8, "planned_fragments": [planned],
                    "requested_delays_ns": [0], "request": req, "response": resp,
                    "events": [
                        {"direction": "send", "timestamp_ns": 11, "bytes": req, "fragment_index": 0},
                        {"direction": "receive", "timestamp_ns": 19, "bytes": resp},
                    ],
                    "terminal": "eof", "error": None, "mode": "request_response", "response_read": True,
                    "status": 200, "headers": {"connection": "close", "content-length": "2"},
                    "body": self.ref(f"bytes/{profile}-{case}-body", b"ok", size=True),
                    "assertions": [{"id": "wire_contract", "passed": True}],
                }
                if case == "partial_write":
                    exchange.update(mode="slow_reader", response_read=False,
                                    terminal="client_close_without_read", status=None, headers={},
                                    response=self.ref(f"bytes/{profile}-{case}-response", b"", size=True),
                                    body=self.ref(f"bytes/{profile}-{case}-body", b"", size=True),
                                    events=[{"direction": "send", "timestamp_ns": 11, "bytes": req,
                                             "fragment_index": 0}],
                                    assertions=[{"id": "slow_reader_closed_without_response_read", "passed": True},
                                                {"id": "server_write_timeout_observed", "passed": True}])
                exchange_ref = self.ref_json(f"records/{profile}-{case}-exchange.json", exchange)
                process = {
                    "kind": "http_server_process", "argv": ["bin/app.elf"], "pid": 123,
                    "started_ns": 1, "ended_ns": 30, "sha256": digest(self.elf.read_bytes()),
                    "profile": profile, "policy_sha256": policy["sha256"], "exit_code": -15,
                    "stdout": stdout, "stderr": stderr, "termination_signal": "SIGTERM",
                    "cleanup_intent": "supervisor_terminate", "cleanup_reaped": True,
                }
                process_ref = self.ref_json(f"records/{profile}-{case}-process.json", process)
                receipt = {
                    "schema_version": "1.0.0", "kind": "http_exchange", "case_id": case,
                    "client": "external_tcp", "executable_role": "application_" + profile,
                    "profile": profile, "contract_sha256": self.spec_hash,
                    "exchange": exchange_ref, "process": process_ref, "policy": policy,
                    "assertions": [{"id": "case_contract", "passed": True}] + (
                        exchange["assertions"] if case == "partial_write" else []),
                }
                receipt_ref = self.ref_json(f"records/{profile}-{case}-receipt.json", receipt)
                rows.append({"id": case, "profile": profile, "exit_code": 0, "count": 1,
                             "receipt": receipt_ref})
        report = {"suite": "http_external_blackbox", "state": "PASSED", "passed": True,
                  "contract_sha256": self.spec_hash, "http": {"client": "external_tcp", "cases": rows},
                  "count": 9999}
        return self.file("http/http_report.json", json.dumps(report).encode())

    def ref_json(self, name, value):
        return self.ref(name, (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode())


class HttpRecordAdapterTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="il-p10-http-records-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.fixture = Fixture(self.root)
        self.provenance = {"source_commit": "a" * 40, "target": "x86_64-unknown-linux-gnu",
                           "toolchain_lock_sha256": digest(b"locked toolchain")}
        self.command = {"command_id": "trusted-http", "argv": ["python", "tests/http_blackbox/cli.py"],
                        "exit_code": 0, "duration_ns": 100}

    def derive(self, report):
        return http_records.derive_http_gate(report, run_root=self.root,
                                             provenance=self.provenance, command=self.command)

    def test_complete_hashed_receipt_matrix_derives_one_gate_record(self):
        report = self.fixture.build()
        result = self.derive(report)
        self.assertEqual(result["status"], "COMPLETE", result["blocked_reasons"])
        record = result["records"]["blackbox_tests_external"]
        expected_count = len(self.fixture.contract["http_cases"]) * len(self.fixture.contract["required_http_profiles"])
        self.assertEqual(len(record["http"]["cases"]), expected_count)
        self.assertEqual(record["test_suite"]["count"], expected_count)
        self.assertEqual(set(result["executables"]), {"application_debug", "application_release"})

    def test_missing_receipt_blocks_instead_of_using_report_count(self):
        report = self.fixture.build(omit=("health", "debug"))
        result = self.derive(report)
        self.assertEqual(result["status"], "BLOCKED")
        self.assertEqual(result["records"], {})
        self.assertIn("omits case/profile receipts", result["blocked_reasons"][0]["message"])

    def test_tampered_exchange_bytes_block_all_records(self):
        report = self.fixture.build()
        data = json.loads(report.read_text())
        first_ref = data["http"]["cases"][0]["receipt"]
        receipt_path = self.root / first_ref["path"]
        receipt = json.loads(receipt_path.read_text())
        exchange_path = self.root / receipt["exchange"]["path"]
        exchange = json.loads(exchange_path.read_text())
        exchange["request"]["sha256"] = digest(b"tampered")
        exchange_path.write_text(json.dumps(exchange))
        result = self.derive(report)
        self.assertEqual(result["status"], "BLOCKED")
        self.assertEqual(result["records"], {})
        self.assertIn("hash mismatch", result["blocked_reasons"][0]["message"])


if __name__ == "__main__":
    unittest.main()
