from __future__ import annotations

import json
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "p10"))
import derive_records  # noqa: E402


def _bytes(value):
    raw = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return {"encoding": "utf-8", "data": raw.decode(), "sha256": derive_records.digest(raw), "size_bytes": len(raw)}


def _observation(command, request, response, tags, *, base=0, result=1):
    return {
        "kind": "il_graph_cli_observation", "tags": tags,
        "request": _bytes(request), "response": _bytes(response),
        "binding": {"base_revision": base, "result_revision": result},
        "process": {"argv": ["il", command], "exit_code": 0, "duration_ns": 10,
                     "stderr": _bytes("")},
    }


class DeriveRecordsTests(unittest.TestCase):
    def test_only_raw_observations_are_promoted(self):
        report = {
            "suite": "graph_cli_blackbox", "passed": True, "count": 3,
            "raw_observations": [
                _observation("inspect", {"entity_id": "app"}, {"ok": True}, ["inspect_succeeds"]),
                _observation("slice", {"root_entities": ["app"]}, {"ok": True}, ["slice_succeeds"]),
                _observation("transact", {"base_revision": 2},
                             {"ok": False, "diagnostics": [{"code": "E_STALE_REVISION"}]},
                             ["stale_transaction_rejected"], base=1, result=2),
            ],
        }
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            path = root / "report.json"
            path.write_text(json.dumps(report), encoding="utf-8")
            result = derive_records.derive_records(path, {
                "source_commit": "a" * 40, "target": "x86_64-unknown-linux-gnu",
                "toolchain_lock_sha256": "sha256:" + "b" * 64,
            })
        self.assertEqual(result["status"], "BLOCKED")
        self.assertEqual(set(result["records"]), {"inspect_succeeds", "slice_succeeds", "stale_transaction_rejected"})
        self.assertIn("transaction_add_route_succeeds", result["blocked_predicates"])

    def test_summary_without_raw_observations_blocks(self):
        with tempfile.TemporaryDirectory() as name:
            path = Path(name) / "summary.json"
            path.write_text(json.dumps({"suite": "graph_cli_blackbox", "passed": True, "count": 28}), encoding="utf-8")
            with self.assertRaisesRegex(derive_records.DeriveError, "raw_observations"):
                derive_records.derive_records(path, {"source_commit": "a" * 40,
                                                      "target": "x86_64-unknown-linux-gnu",
                                                      "toolchain_lock_sha256": "sha256:" + "b" * 64})

    def test_route_record_requires_successful_observed_transaction_and_measured_process(self):
        request = {"task_id": "P08-route", "base_revision": 4, "operations": [
            {"op": "add_route", "route_table_id": "server", "route": {
                "entity_id": "route.hello", "method": "GET", "path": "/hello/{name}",
                "handler": "hello", "parameters": [{"name": "name", "type_ref": "String",
                    "source": "path", "max_utf8_bytes": 128}],
            }}], "scope": ["server", "route.hello"]}
        response = {"ok": True, "result_revision": 5, "diagnostics": []}
        observation = _observation("transact", request, response, ["transaction_published_revision"], base=4, result=5)
        report = {"suite": "graph_cli_blackbox", "passed": True, "count": 1, "raw_observations": [observation]}
        with tempfile.TemporaryDirectory() as name:
            path = Path(name) / "report.json"
            path.write_text(json.dumps(report), encoding="utf-8")
            result = derive_records.derive_records(path, {
                "source_commit": "a" * 40, "target": "x86_64-unknown-linux-gnu",
                "toolchain_lock_sha256": "sha256:" + "b" * 64,
            })
        record = result["records"]["transaction_add_route_succeeds"]
        self.assertEqual(record["transaction"]["base_revision"], 4)
        self.assertEqual(record["transaction"]["result_revision"], 5)
        self.assertEqual(record["transaction"]["route_path"], "/hello/{name}")

    def test_missing_measured_duration_blocks_raw_predicate(self):
        observation = _observation("inspect", {"entity_id": "app"}, {"ok": True}, ["inspect_succeeds"])
        observation["process"].pop("duration_ns")
        with tempfile.TemporaryDirectory() as name:
            path = Path(name) / "report.json"
            path.write_text(json.dumps({"raw_observations": [observation]}), encoding="utf-8")
            result = derive_records.derive_records(path, {
                "source_commit": "a" * 40, "target": "x86_64-unknown-linux-gnu",
                "toolchain_lock_sha256": "sha256:" + "b" * 64,
            })
        self.assertNotIn("inspect_succeeds", result["records"])
        self.assertIn("inspect_succeeds", result["blocked_predicates"])

    def test_ai_protocol_route_record_binds_raw_request_to_measured_ci_command(self):
        route = {"entity_id": "route.hello", "method": "GET", "path": "/hello/{name}",
                 "handler": "hello", "parameters": [{"name": "name", "type_ref": "String",
                    "source": "path", "max_utf8_bytes": 128}]}
        request = {"task_id": "P08-route", "base_revision": 4, "operations": [
            {"op": "add_route", "route_table_id": "server", "route": route}]}
        ai = {"cases": [{"id": "route-transaction", "passed": True, "steps": [{
            "tool": "transact", "exit_code": 0, "request": request,
            "result": {"response": {"ok": True, "result_revision": 5}},
        }]}]}
        ci = {"gates": [{"name": "ai-protocol-cli", "status": "PASSED", "exit_code": 0,
                         "elapsed_seconds": 1.25, "command": ["python", "tests/ai_protocol_cli.py"]}]}
        graph = {"raw_observations": []}
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            paths = [root / "graph.json", root / "ai.json", root / "ci.json"]
            for path, value in zip(paths, (graph, ai, ci)):
                path.write_text(json.dumps(value), encoding="utf-8")
            result = derive_records.derive_records(paths[0], {
                "source_commit": "a" * 40, "target": "x86_64-unknown-linux-gnu",
                "toolchain_lock_sha256": "sha256:" + "b" * 64,
            }, ai_report=paths[1], ci_report=paths[2])
        record = result["records"]["transaction_add_route_succeeds"]
        self.assertEqual(record["command_id"], "ai-protocol-cli")
        self.assertEqual(record["duration_ns"], 1_250_000_000)
        self.assertEqual(record["transaction"]["result_revision"], 5)

    def test_rollback_record_binds_trial_binary_to_release_elf(self):
        release_sha = "sha256:" + "c" * 64
        graph_artifact = {"sha256": release_sha, "path": "il_server_x86_64"}
        observation = {
            "kind": "il_graph_cli_observation",
            "tags": ["rollback_trial_binds_to_release_elf"],
            "evidence": {
                "trial_binary_sha256": release_sha,
                "graph_evidence": {
                    "before_revision": 5,
                    "after_revision": 4,
                    "trial_exit_code": 0,
                },
            },
            "process": {"argv": ["il", "rollback"], "exit_code": 0, "duration_ns": 100000,
                        "stderr": _bytes("")},
        }
        report = {"raw_observations": [observation]}
        with tempfile.TemporaryDirectory() as name:
            path = Path(name) / "report.json"
            path.write_text(json.dumps(report), encoding="utf-8")
            result = derive_records.derive_records(path, {
                "source_commit": "a" * 40, "target": "x86_64-unknown-linux-gnu",
                "toolchain_lock_sha256": "sha256:" + "b" * 64,
            }, graph_artifact=graph_artifact)
        self.assertIn("rollback_trial_binds_to_release_elf", result["records"])
        record = result["records"]["rollback_trial_binds_to_release_elf"]
        rollback = record["rollback"]
        self.assertEqual(rollback["trial_binary_sha256"], release_sha)
        self.assertEqual(rollback["release_elf_sha256"], release_sha)
        self.assertEqual(rollback["graph_evidence"]["before_revision"], 5)
        self.assertEqual(rollback["graph_evidence"]["after_revision"], 4)

    def test_rollback_record_blocks_when_sha256_mismatch(self):
        graph_artifact = {"sha256": "sha256:" + "d" * 64, "path": "il_server_x86_64"}
        observation = {
            "kind": "il_graph_cli_observation",
            "tags": ["rollback_trial_binds_to_release_elf"],
            "evidence": {
                "trial_binary_sha256": "sha256:" + "e" * 64,
                "graph_evidence": {"before_revision": 5, "after_revision": 4, "trial_exit_code": 0},
            },
            "process": {"argv": ["il", "rollback"], "exit_code": 0, "duration_ns": 100000,
                        "stderr": _bytes("")},
        }
        report = {"raw_observations": [observation]}
        with tempfile.TemporaryDirectory() as name:
            path = Path(name) / "report.json"
            path.write_text(json.dumps(report), encoding="utf-8")
            result = derive_records.derive_records(path, {
                "source_commit": "a" * 40, "target": "x86_64-unknown-linux-gnu",
                "toolchain_lock_sha256": "sha256:" + "b" * 64,
            }, graph_artifact=graph_artifact)
        self.assertNotIn("rollback_trial_binds_to_release_elf", result["records"])
        self.assertIn("rollback_trial_binds_to_release_elf", result["blocked_predicates"])
        self.assertIn("does not match", result["blocked_predicates"]["rollback_trial_binds_to_release_elf"])


if __name__ == "__main__":
    unittest.main()
