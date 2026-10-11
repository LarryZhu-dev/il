from __future__ import annotations

import json
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "p10"))
import gate_records  # noqa: E402


def _contract() -> dict:
    return {"gates": {"G0": ["graph_schema_valid", "type_checker_verified"]}}


def _record(predicate: str, *, status: str = "PASSED") -> dict:
    return {
        "schema_version": "1.0.0",
        "kind": "p10_gate_record",
        "predicate": predicate,
        "source_commit": "a" * 40,
        "target": "x86_64-unknown-linux-gnu",
        "toolchain_lock_sha256": "sha256:" + "b" * 64,
        "status": status,
        "command_id": "locked-ci",
        "argv_sha256": "sha256:" + "c" * 64,
        "exit_code": 0,
    }


class GateRecordExportTests(unittest.TestCase):
    def test_ci_summary_is_not_promoted_to_predicate(self) -> None:
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            (root / "result.json").write_text(json.dumps({"status": "PASSED", "gates": []}), encoding="utf-8")
            result = gate_records.export_gate_records(root, contract=_contract())
            self.assertEqual(result["status"], "BLOCKED")
            self.assertEqual(result["missing_predicates"], ["graph_schema_valid", "type_checker_verified"])
            self.assertEqual(result["records"], {})

    def test_explicit_records_are_copied_with_content_hashes(self) -> None:
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            source = root / "observations"
            source.mkdir()
            for predicate in gate_records.required_predicates(_contract()):
                (source / f"{predicate}.json").write_text(
                    json.dumps(_record(predicate), sort_keys=True), encoding="utf-8"
                )
            output = root / "gates"
            result = gate_records.export_gate_records(source, contract=_contract(), output_root=output)
            self.assertEqual(result["status"], "COMPLETE")
            self.assertEqual(set(result["records"]), set(gate_records.required_predicates(_contract())))
            for predicate, reference in result["records"].items():
                data = (output / reference["path"]).read_bytes()
                self.assertEqual(reference["sha256"], gate_records.sha256_bytes(data))

    def test_failed_or_duplicate_record_blocks_and_retains_source_refs(self) -> None:
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            (root / "one.json").write_text(json.dumps(_record("graph_schema_valid")), encoding="utf-8")
            (root / "duplicate.json").write_text(json.dumps(_record("graph_schema_valid")), encoding="utf-8")
            (root / "failed.json").write_text(
                json.dumps(_record("type_checker_verified", status="FAILED")), encoding="utf-8"
            )
            result = gate_records.export_gate_records(root, contract=_contract())
            self.assertEqual(result["status"], "BLOCKED")
            self.assertEqual(result["missing_predicates"], ["type_checker_verified"])
            self.assertTrue(any("duplicate predicate" in row["reason"] for row in result["malformed_records"]))
            self.assertTrue(any(row.get("predicate") == "type_checker_verified" for row in result["source_refs"]))

    def test_unknown_predicate_cannot_expand_contract(self) -> None:
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            (root / "unknown.json").write_text(json.dumps(_record("future_gate")), encoding="utf-8")
            result = gate_records.export_gate_records(root, contract=_contract())
            self.assertEqual(result["status"], "BLOCKED")
            self.assertEqual(result["records"], {})
            self.assertEqual(result["malformed_records"][0]["reason"], "unknown predicate")


if __name__ == "__main__":
    unittest.main()
