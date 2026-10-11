from __future__ import annotations

import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "p10"))
import ci_records  # noqa: E402


PROVENANCE = {"source_commit": "a" * 40, "target": "x86_64-unknown-linux-gnu",
              "toolchain_lock_sha256": "sha256:" + "b" * 64}


def _sha(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


class CiRecordTests(unittest.TestCase):
    def test_graph_transaction_adapter_imports_as_package(self):
        from tools.p10 import ci_records as package_ci_records
        expected = {"records": {"inspect_succeeds": {}}, "blocked_predicates": {}}
        with mock.patch("tools.p10.derive_records.derive_records", return_value=expected) as derive:
            result = package_ci_records.derive_graph_transactions("unused.json", provenance=PROVENANCE)
        derive.assert_called_once()
        self.assertEqual(result["records"], {"inspect_succeeds": {}})
        self.assertEqual(result["status"], "COMPLETE")

    def _fixture(self, root: Path, *, omit: str | None = None, tamper_log: bool = False) -> Path:
        (root / "build" / "ci").mkdir(parents=True)
        test_names = set().union(*ci_records.G0_RUST_TESTS.values())
        rust_lines = [f"test crate::{name} ... ok" for name in sorted(test_names) if name != omit]
        rust_log = ("\n".join(rust_lines) + "\n").encode()
        (root / "build" / "ci" / "rust-tests.log").write_bytes(rust_log)
        if tamper_log:
            rust_log += b"tampered\n"
        graph_rows = [{"test_case": "test_validate_and_slice_use_structural_contract",
                       "process": {"exit_code": 0, "duration_ns": 200},
                       "response": {"encoding": "utf-8", "data": json.dumps({"ok": True})}}]
        (root / "build" / "graph_cli_report.json").write_text(json.dumps({
            "suite": "graph_cli_blackbox", "passed": True, "raw_observations": graph_rows,
        }), encoding="utf-8")
        semantic_cases = [
            {"id": name, "passed": True} for name in (
                "valid_integer_return", "valid_typed_call", "wrong_return_type",
                "mixed_integer_add", "unknown_value", "integer_literal_out_of_range")
        ]
        (root / "build" / "semantic_cli_report.json").write_text(json.dumps({
            "suite": "semantic_cli_blackbox", "passed": True, "count": len(semantic_cases),
            "cases": semantic_cases,
        }), encoding="utf-8")
        gates = []
        for name, command, log_name in (
            ("rust-tests", ["cargo", "test", "--workspace", "--locked"], "rust-tests.log"),
            ("graph-cli", ["python", "tests/graph_cli.py"], "graph-cli.log"),
            ("semantic-cli", ["python", "tests/semantic_cli.py"], "semantic-cli.log"),
        ):
            if log_name != "rust-tests.log":
                (root / "build" / "ci" / log_name).write_text("gate invocation\n", encoding="utf-8")
            raw = (root / "build" / "ci" / log_name).read_bytes()
            gates.append({"name": name, "status": "PASSED", "exit_code": 0,
                          "elapsed_seconds": 1.5, "command": command,
                          "log": f"build/ci/{log_name}", "log_sha256": hashlib.sha256(raw).hexdigest()})
        path = root / "build" / "ci" / "result.json"
        path.write_text(json.dumps({"status": "PASSED", "gates": gates}), encoding="utf-8")
        return path

    def test_derives_only_named_g0_observations_and_hash_binds_gate_log(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            report = self._fixture(root)
            result = ci_records.derive_g0(report, provenance=PROVENANCE)
        self.assertEqual(result["status"], "COMPLETE")
        self.assertEqual(set(result["records"]), {
            "graph_schema_valid", "type_checker_verified", "ownership_checker_verified",
            "MIR_verifier_verified", "interpreter_semantic_suite_passed",
        })
        for predicate, record in result["records"].items():
            self.assertEqual(record["predicate"], predicate)
            self.assertEqual(record["status"], "PASSED")
            self.assertEqual(record["assertions"], [{"id": predicate, "passed": True}])

    def test_missing_named_test_blocks_only_its_predicate(self):
        missing = next(iter(ci_records.G0_RUST_TESTS["ownership_checker_verified"]))
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            report = self._fixture(root, omit=missing)
            result = ci_records.derive_g0(report, provenance=PROVENANCE)
        self.assertNotIn("ownership_checker_verified", result["records"])
        self.assertIn("ownership_checker_verified", result["blocked_predicates"])
        self.assertIn("graph_schema_valid", result["records"])

    def test_changed_raw_log_blocks_rust_predicates(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            report = self._fixture(root)
            raw = (root / "build" / "ci" / "rust-tests.log").read_bytes() + b"tampered\n"
            (root / "build" / "ci" / "rust-tests.log").write_bytes(raw)
            result = ci_records.derive_g0(report, provenance=PROVENANCE)
        self.assertNotIn("ownership_checker_verified", result["records"])
        self.assertIn("log hash mismatch", result["blocked_predicates"]["ownership_checker_verified"])

    def test_summary_counts_cannot_replace_semantic_case_rows(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            report = self._fixture(root)
            (root / "build" / "semantic_cli_report.json").write_text(json.dumps({
                "suite": "semantic_cli_blackbox", "passed": True, "count": 231,
                "cases": [],
            }), encoding="utf-8")
            result = ci_records.derive_g0(report, provenance=PROVENANCE)
        self.assertNotIn("type_checker_verified", result["records"])
        self.assertIn("type_checker_verified", result["blocked_predicates"])


if __name__ == "__main__":
    unittest.main()
