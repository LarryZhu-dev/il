import hashlib
import json
import re
import tempfile
import unittest
from pathlib import Path

from tools.p10 import runner


SOURCE_COMMIT = "a" * 40
BASELINE_COMMIT = "b" * 40


def digest(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


class BundleFactory:
    def __init__(self, root):
        self.root = root
        self.root.mkdir(parents=True)
        self.files = {}

    def file(self, relative, data):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        self.files[relative] = data
        return path

    def ref(self, relative, role=None, data=None):
        if data is not None:
            self.file(relative, data)
        content = (self.root / relative).read_bytes()
        result = {"path": relative, "sha256": digest(content)}
        if role is not None:
            result["role"] = role
        return result

    def json_ref(self, relative, value, role=None):
        raw = (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()
        return self.ref(relative, role=role, data=raw)


def minimal_elf():
    data = bytearray(64)
    data[:4] = b"\x7fELF"
    data[4] = 2
    data[5] = 1
    data[6] = 1
    data[16:18] = (2).to_bytes(2, "little")
    data[18:20] = (62).to_bytes(2, "little")
    return bytes(data)


def build_synthetic_bundle(root):
    bundle = BundleFactory(root)
    contract, contract_bytes = runner.load_contract()
    lock_data = b'{"schema_version":"1.0.0","target":"x86_64-unknown-linux-gnu"}\n'
    graph_data = b'{"revision":7,"graph":"verified"}\n'
    compiler_data = b"compiler executable"
    runtime_data = b"runtime archive"
    elf_data = minimal_elf()
    top_artifacts = [
        bundle.ref("artifacts/graph.json", "graph", graph_data),
        bundle.ref("artifacts/compiler", "compiler", compiler_data),
        bundle.ref("artifacts/runtime.a", "runtime", runtime_data),
        bundle.ref("artifacts/il-http", "native_elf", elf_data),
        bundle.ref("artifacts/il-http-debug", "application_debug", elf_data),
        bundle.ref("artifacts/il-http-release", "application_release", elf_data),
        bundle.ref("artifacts/toolchain.lock", "toolchain_lock", lock_data),
    ]
    artifact_hash = {row["role"]: row["sha256"] for row in top_artifacts}
    executable_inventory = [
        {
            "role": "application_debug",
            "kind": "application",
            "profile": "debug",
            "target": contract["target"],
            "entry_point": "demo.serve",
            "graph_revision": 7,
            "graph_sha256": artifact_hash["graph"],
            "runtime_profile": "full",
            "build_invocation_sha256": digest(b"build debug"),
            "artifact_role": "application_debug",
        },
        {
            "role": "application_release",
            "kind": "application",
            "profile": "release",
            "target": contract["target"],
            "entry_point": "demo.serve",
            "graph_revision": 7,
            "graph_sha256": artifact_hash["graph"],
            "runtime_profile": "full",
            "build_invocation_sha256": digest(b"build release"),
            "artifact_role": "application_release",
        },
    ]

    def byte_ref(relative, data):
        bundle.file(relative, data)
        return {"path": relative, "sha256": digest(data), "size_bytes": len(data)}

    def http_receipt(case_id, profile):
        executable_role = "application_" + profile
        request_data = (case_id + " request").encode()
        response_data = (case_id + " response").encode()
        policy_data = json.dumps({"profile": profile, "allow": ["127.0.0.1:8080"]}, sort_keys=True).encode()
        stdout_data = (profile + " stdout").encode()
        stderr_data = (profile + " stderr").encode()
        started = 1000
        ended = 2000
        request_ref = byte_ref(f"http/{profile}/{case_id}.request", request_data)
        response_ref = byte_ref(f"http/{profile}/{case_id}.response", response_data)
        planned_ref = byte_ref(f"http/{profile}/{case_id}.planned", request_data)
        stdout_ref = byte_ref(f"http/{profile}/{case_id}.stdout", stdout_data)
        stderr_ref = byte_ref(f"http/{profile}/{case_id}.stderr", stderr_data)
        policy_ref = bundle.ref(f"http/{profile}/{case_id}.policy", data=policy_data)
        exchange = {
            "schema_version": "1.0.0", "kind": "tcp_exchange", "exchange_id": f"exchange-{profile}-{case_id}",
            "phase": "case", "peer": ["127.0.0.1", 8080], "started_ns": started, "ended_ns": ended,
            "elapsed_ns": ended - started, "timeout_seconds": 8, "planned_fragments": [planned_ref],
            "requested_delays_ns": [0], "request": request_ref, "response": response_ref,
            "events": [
                {"direction": "send", "timestamp_ns": 1100, "bytes": request_ref, "fragment_index": 0},
                {"direction": "receive", "timestamp_ns": 1900, "bytes": response_ref},
            ],
            "terminal": "eof", "error": None, "mode": "request_response", "response_read": True,
            "status": 200, "headers": {"connection": "close"}, "body": response_ref,
            "assertions": [{"id": "wire_contract", "passed": True}],
        }
        process = {
            "kind": "http_server_process", "argv": ["./demo.elf"], "pid": 1234, "started_ns": 900,
            "ended_ns": 2100, "sha256": artifact_hash[executable_role], "profile": profile,
            "policy_sha256": policy_ref["sha256"], "exit_code": -15, "stdout": stdout_ref,
            "stderr": stderr_ref, "termination_signal": "SIGTERM", "cleanup_intent": "supervisor_terminate",
            "cleanup_reaped": True,
        }
        exchange_ref = bundle.json_ref(f"http/{profile}/{case_id}.exchange.json", exchange)
        process_ref = bundle.json_ref(f"http/{profile}/{case_id}.process.json", process)
        receipt = {
            "schema_version": "1.0.0", "kind": "http_exchange", "case_id": case_id,
            "client": "external_tcp", "executable_role": executable_role, "profile": profile,
            "contract_sha256": digest(contract_bytes), "exchange": exchange_ref, "process": process_ref,
            "policy": policy_ref, "assertions": [{"id": "case_contract", "passed": True}],
        }
        return bundle.json_ref(f"http/{profile}/{case_id}.receipt.json", receipt)

    predicates = {}
    for group, names in contract["gates"].items():
        for predicate in names:
            record = {
                "schema_version": "1.0.0",
                "kind": "p10_gate_record",
                "predicate": predicate,
                "source_commit": SOURCE_COMMIT,
                "target": contract["target"],
                "toolchain_lock_sha256": artifact_hash["toolchain_lock"],
                "status": "PASSED",
                "command_id": "locked-acceptance-command",
                "argv_sha256": digest(b"fixed argv"),
                "exit_code": 0,
                "duration_ns": 1000000,
                "test_suite": {"id": predicate + "-suite", "exit_code": 0, "count": 1},
                "artifacts": [],
                "assertions": [{"id": predicate, "passed": True}],
            }
            if predicate == "native_elf_generated":
                record["artifacts"] = [top_artifacts[3]]
            elif predicate == "clean_environment_runs":
                record["execution"] = {
                    "empty_environment": True,
                    "exit_code": 0,
                    "executable_sha256": artifact_hash["application_release"],
                    "stdout_sha256": digest(b"service ready"),
                }
            elif predicate == "no_high_level_target_source":
                record["target_generated_sources"] = []
            elif predicate == "reproducible_hash":
                record["clean_rebuilds"] = [
                    {
                        "store_id": "clean-store-a",
                        "clean": True,
                        "empty_environment": True,
                        "exit_code": 0,
                        "execution_exit_code": 0,
                        "artifacts": [top_artifacts[3]],
                    },
                    {
                        "store_id": "clean-store-b",
                        "clean": True,
                        "empty_environment": True,
                        "exit_code": 0,
                        "execution_exit_code": 0,
                        "artifacts": [top_artifacts[3]],
                    },
                ]
            elif predicate == "blackbox_tests_external":
                cases = []
                for case_id in contract["http_cases"]:
                    for profile in contract["required_http_profiles"]:
                        cases.append({"id": case_id, "profile": profile, "exit_code": 0, "count": 1, "receipt": http_receipt(case_id, profile)})
                record["http"] = {"client": "external_tcp", "cases": cases}
            elif predicate == "transaction_add_route_succeeds":
                record["transaction"] = {
                    "base_revision": 7,
                    "result_revision": 8,
                    "route_path": "/hello/{name}",
                    "route_sha256": digest(b"hello route"),
                }
            elif predicate == "stale_transaction_rejected":
                record["transaction"] = {
                    "requested_base_revision": 7,
                    "observed_head_revision": 8,
                    "result_code": "E_STALE_REVISION",
                    "head_before": 8,
                    "head_after": 8,
                }
            elif predicate == "failure_preserves_previous_revision":
                record["transaction"] = {
                    "base_revision": 8,
                    "head_before": 8,
                    "head_after": 8,
                    "base_graph_sha256": artifact_hash["graph"],
                    "candidate_sha256": digest(b"failed candidate graph"),
                    "failure_code": "E_TYPE_MISMATCH",
                }
            elif predicate == "all_artifacts_hashed":
                record["artifacts"] = top_artifacts
            elif predicate == "diagnostics_machine_readable":
                record["diagnostics"] = [{
                    "diagnostic_id": "diag_01",
                    "code": "E_TYPE_MISMATCH",
                    "stage": "type_check",
                    "entity_id": "fn_hello",
                }]
            elif predicate == "rollback_verified":
                record["rollback"] = {
                    "source_revision": 9,
                    "rollback_revision": 10,
                    "expected_graph_sha256": artifact_hash["graph"],
                    "restored_graph_sha256": artifact_hash["graph"],
                    "previous_binary_sha256": artifact_hash["native_elf"],
                    "previous_binary_executed": True,
                }
            predicates[predicate] = bundle.json_ref(f"gates/{predicate}.json", record)

    trials = []
    for task_id, assertion_ids in contract["g4"]["tasks"].items():
        for implementation in contract["g4"]["implementations"]:
            for index in range(1, contract["g4"]["trial_count"] + 1):
                run_id = f"run_{task_id}_{implementation}_{index}"
                failed = task_id == "fix_type_error" and implementation == "rust" and index == 2
                assertions = [
                    {"id": assertion_id, "exit_code": 1 if failed and position == 0 else 0, "count": 1}
                    for position, assertion_id in enumerate(assertion_ids)
                ]
                trace_events = [
                    {
                        "sequence": seq,
                        "kind": "tool_call",
                        "tool": "inspect" if seq == 1 else "test",
                        "request_sha256": digest(f"{run_id}:{seq}:request".encode()),
                        "response_sha256": digest(f"{run_id}:{seq}:response".encode()),
                    }
                    for seq in range(1, 3 if implementation == "il" else 2)
                ]
                bundle.json_ref(f"traces/{run_id}.json", {
                    "schema_version": "1.0.0",
                    "kind": "g4_trace",
                    "run_id": run_id,
                    "events": trace_events,
                })
                patch_data = (
                    f"diff --git a/{task_id}.txt b/{task_id}.txt\n"
                    f"--- a/{task_id}.txt\n+++ b/{task_id}.txt\n@@ -1 +1 @@\n-old {index}\n+new {index}\n"
                ).encode()
                bundle.file(f"patches/{run_id}.patch", patch_data)
                trial = {
                    "schema_version": "1.0.0",
                    "kind": "g4_trial",
                    "task_id": task_id,
                    "implementation": implementation,
                    "trial_index": index,
                    "run_id": run_id,
                    "workspace_id": f"workspace_{task_id}_{implementation}_{index}",
                    "starting_commit": BASELINE_COMMIT if implementation == "rust" else SOURCE_COMMIT,
                    "task_input_sha256": digest(f"locked task input {task_id}".encode()),
                    "started_monotonic_ns": 100000000,
                    "finished_monotonic_ns": 2100000000,
                    "experiment_exit_code": 1 if failed else 0,
                    "failure_category": "test" if failed else None,
                    "assertions": assertions,
                    "trace": {"path": f"traces/{run_id}.json", "sha256": digest((root / f"traces/{run_id}.json").read_bytes()), "role": "trace"},
                    "patch": {"path": f"patches/{run_id}.patch", "sha256": digest(patch_data), "role": "patch"},
                }
                trials.append(bundle.json_ref(f"trials/{run_id}.json", trial))

    manifest = {
        "schema_version": "1.0.0",
        "kind": "p10_acceptance_bundle",
        "source_commit": SOURCE_COMMIT,
        "target": contract["target"],
        "toolchain_lock_sha256": artifact_hash["toolchain_lock"],
        "executables": executable_inventory,
        "predicates": predicates,
        "artifacts": top_artifacts,
        "comparison": {
            "task_contract_sha256": digest(contract_bytes),
            "baseline_commit": BASELINE_COMMIT,
            "trials": trials,
        },
    }
    bundle.file("manifest.json", (json.dumps(manifest, sort_keys=True, indent=2) + "\n").encode())
    return bundle, manifest


class P10RunnerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="il-p10-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / "bundle"
        self.bundle, self.manifest = build_synthetic_bundle(self.root)

    def write_manifest(self):
        (self.root / "manifest.json").write_text(
            json.dumps(self.manifest, sort_keys=True, indent=2) + "\n", encoding="utf-8"
        )

    def test_full_bundle_derives_claimed_comparison_without_changing_task_status(self):
        result = runner.validate_bundle_structure(self.root, expected_source_commit=SOURCE_COMMIT)
        self.assertTrue(result["ok"], result)
        self.assertEqual(result["result"], "evidence_structure_validated")
        self.assertTrue(result["task_status_unchanged"])
        self.assertEqual(result["claimed_comparison"]["trial_count"], 18)
        self.assertEqual(result["claimed_comparison"]["tasks"]["add_route"]["il"]["success_rate"], 1.0)
        self.assertEqual(result["claimed_comparison"]["tasks"]["fix_type_error"]["rust"]["successes"], 2)
        self.assertEqual(result["claimed_comparison"]["failure_distribution"]["rust"]["test"], 1)
        self.assertEqual(result["claimed_comparison"]["tasks"]["rollback"]["il"]["mean_change_size"]["insertions"], 1.0)
        self.assertNotIn("release", {path.name for path in self.root.iterdir()})

    def test_missing_manifest_fails_closed(self):
        empty = Path(self.temporary.name) / "empty"
        empty.mkdir()
        result = runner.validate_bundle_structure(empty)
        self.assertFalse(result["ok"])
        self.assertEqual(result["code"], "E_EVIDENCE_INCOMPLETE")
        self.assertEqual(result["status"], "BLOCKED")

    def test_self_asserted_success_cannot_verify_p10(self):
        # Every claimed result, timestamp, trace and ELF header in this fixture
        # was constructed without running il, LLVM or a G4 trial.
        result = runner.validate_bundle_structure(self.root)
        self.assertTrue(result["ok"], result)
        self.assertEqual(result["result"], "evidence_structure_validated")
        self.assertEqual(result["status"], "PROVISIONAL")
        self.assertIs(result["acceptance_verified"], False)
        self.assertIs(result["provenance_verified"], False)
        self.assertNotIn("gates", result)
        self.assertNotIn("comparison", result)
        self.assertIn("do not prove commands ran", result["provenance_limitation"])

    def test_hash_mismatch_rejects_artifact_bytes(self):
        (self.root / "artifacts" / "runtime.a").write_bytes(b"tampered runtime")
        result = runner.validate_bundle_structure(self.root)
        self.assertFalse(result["ok"])
        self.assertEqual(result["diagnostics"][0]["code"], "E_EVIDENCE_INCOMPLETE")
        self.assertIn("hash does not match", result["diagnostics"][0]["message"])

    def test_incomplete_g4_matrix_does_not_pass(self):
        self.manifest["comparison"]["trials"].pop()
        self.write_manifest()
        result = runner.validate_bundle_structure(self.root)
        self.assertFalse(result["ok"])
        self.assertEqual(result["code"], "E_EVIDENCE_INCOMPLETE")
        self.assertIn("requires 18 raw isolated trial records", result["diagnostics"][0]["message"])

    def test_duplicate_trial_workspace_is_not_isolated(self):
        trial_ref = self.manifest["comparison"]["trials"][1]
        trial_path = self.root / trial_ref["path"]
        trial = json.loads(trial_path.read_text(encoding="utf-8"))
        trial["workspace_id"] = "workspace_add_route_rust_1"
        trial_path.write_text(json.dumps(trial, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
        trial_ref["sha256"] = digest(trial_path.read_bytes())
        self.write_manifest()
        result = runner.validate_bundle_structure(self.root)
        self.assertFalse(result["ok"])
        self.assertIn("reused a run or isolation workspace", result["diagnostics"][0]["message"])

    def test_provenance_mismatch_is_rejected(self):
        result = runner.validate_bundle_structure(self.root, expected_source_commit="c" * 40)
        self.assertFalse(result["ok"])
        self.assertIn("differs from current HEAD", result["diagnostics"][0]["message"])

    def test_release_executable_inventory_is_required(self):
        self.manifest["executables"] = [row for row in self.manifest["executables"] if row["role"] != "application_release"]
        self.write_manifest()
        result = runner.validate_bundle_structure(self.root)
        self.assertFalse(result["ok"])
        self.assertIn("release executable role is absent", result["diagnostics"][0]["message"])

    def test_http_matrix_requires_debug_and_release_for_each_case(self):
        gate_ref = self.manifest["predicates"]["blackbox_tests_external"]
        gate_path = self.root / gate_ref["path"]
        gate = json.loads(gate_path.read_text(encoding="utf-8"))
        gate["http"]["cases"] = [case for case in gate["http"]["cases"] if not (case["id"] == "health" and case["profile"] == "debug")]
        gate_path.write_text(json.dumps(gate, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
        gate_ref["sha256"] = digest(gate_path.read_bytes())
        self.write_manifest()
        result = runner.validate_bundle_structure(self.root)
        self.assertFalse(result["ok"])
        self.assertIn("HTTP blackbox evidence omits cases", result["diagnostics"][0]["message"])

    def test_http_exchange_rejects_tampered_wire_hash(self):
        gate_ref = self.manifest["predicates"]["blackbox_tests_external"]
        gate_path = self.root / gate_ref["path"]
        gate = json.loads(gate_path.read_text(encoding="utf-8"))
        receipt_ref = next(case["receipt"] for case in gate["http"]["cases"] if case["id"] == "health" and case["profile"] == "debug")
        receipt_path = self.root / receipt_ref["path"]
        receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
        exchange_path = self.root / receipt["exchange"]["path"]
        exchange = json.loads(exchange_path.read_text(encoding="utf-8"))
        exchange["request"]["sha256"] = digest(b"tampered wire")
        exchange_path.write_text(json.dumps(exchange, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
        receipt["exchange"]["sha256"] = digest(exchange_path.read_bytes())
        receipt_path.write_text(json.dumps(receipt, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
        receipt_ref["sha256"] = digest(receipt_path.read_bytes())
        gate_ref["sha256"] = digest(gate_path.read_bytes())
        self.write_manifest()
        result = runner.validate_bundle_structure(self.root)
        self.assertFalse(result["ok"])
        self.assertIn("hash does not match", result["diagnostics"][0]["message"])

    def test_http_process_rejects_rewritten_signal_exit(self):
        gate_ref = self.manifest["predicates"]["blackbox_tests_external"]
        gate_path = self.root / gate_ref["path"]
        gate = json.loads(gate_path.read_text(encoding="utf-8"))
        receipt_ref = next(case["receipt"] for case in gate["http"]["cases"] if case["id"] == "health" and case["profile"] == "release")
        receipt_path = self.root / receipt_ref["path"]
        receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
        process_path = self.root / receipt["process"]["path"]
        process = json.loads(process_path.read_text(encoding="utf-8"))
        process["exit_code"] = 0
        process_path.write_text(json.dumps(process, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
        receipt["process"]["sha256"] = digest(process_path.read_bytes())
        receipt_path.write_text(json.dumps(receipt, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
        receipt_ref["sha256"] = digest(receipt_path.read_bytes())
        gate_path.write_text(json.dumps(gate, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
        gate_ref["sha256"] = digest(gate_path.read_bytes())
        self.write_manifest()
        result = runner.validate_bundle_structure(self.root)
        self.assertFalse(result["ok"])
        self.assertIn("signal exit", result["diagnostics"][0]["message"])

    def test_bundle_and_contract_schemas_are_machine_readable_json(self):
        bundle_schema = json.loads(runner.SCHEMA_PATH.read_text(encoding="utf-8"))
        contract = json.loads(runner.CONTRACT_PATH.read_text(encoding="utf-8"))
        self.assertEqual(bundle_schema["$schema"], "https://json-schema.org/draft/2020-12/schema")
        self.assertEqual(contract["g4"]["trial_count"], 3)
        self.assertEqual(len(contract["g4"]["tasks"]), 3)
        predicates = bundle_schema["properties"]["predicates"]
        self.assertEqual(set(predicates["propertyNames"]["enum"]), set(predicates["required"]))
        pattern = bundle_schema["$defs"]["relativePath"]["pattern"]
        self.assertIsNotNone(re.fullmatch(pattern, "gates/result.json"))
        for invalid in ("/absolute", "a/../escape", "a//file", "a/./file", "a/", "C:/file", "a\\file"):
            self.assertIsNone(re.fullmatch(pattern, invalid), invalid)


if __name__ == "__main__":
    unittest.main()
