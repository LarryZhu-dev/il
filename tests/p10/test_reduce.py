from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "p10"))
import reduce as reducer  # noqa: E402


def digest(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


class ReduceTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="il-p10-reduce-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.contract = json.loads((ROOT / "tools" / "p10" / "contract.json").read_text())
        self.manifest = self._write_complete_manifest()

    def _write_complete_manifest(self) -> dict:
        predicates = {}
        names = reducer.required_predicates(self.contract)
        for index, predicate in enumerate(names):
            record = {
                "schema_version": "1.0.0",
                "kind": "p10_gate_record",
                "predicate": predicate,
                "source_commit": "a" * 40,
                "target": self.contract["target"],
                "toolchain_lock_sha256": "sha256:" + "b" * 64,
                "status": "PASSED",
            }
            path = self.root / "gates" / f"{index:02d}-{predicate}.json"
            path.parent.mkdir(parents=True, exist_ok=True)
            raw = (json.dumps(record, sort_keys=True) + "\n").encode()
            path.write_bytes(raw)
            predicates[predicate] = {"path": path.relative_to(self.root).as_posix(), "sha256": digest(raw)}
        trials = []
        for task_id in self.contract["g4"]["tasks"]:
            for implementation in self.contract["g4"]["implementations"]:
                for trial_index in range(1, self.contract["g4"]["trial_count"] + 1):
                    record = {"schema_version": "1.0.0", "kind": "g4_trial", "task_id": task_id,
                              "implementation": implementation, "trial_index": trial_index}
                    path = self.root / "g4" / f"{task_id}-{implementation}-{trial_index}.json"
                    path.parent.mkdir(parents=True, exist_ok=True)
                    raw = (json.dumps(record, sort_keys=True) + "\n").encode()
                    path.write_bytes(raw)
                    trials.append({"path": path.relative_to(self.root).as_posix(), "sha256": digest(raw)})
        manifest = {
            "schema_version": "1.0.0",
            "kind": "p10_acceptance_bundle",
            "source_commit": "a" * 40,
            "target": self.contract["target"],
            "toolchain_lock_sha256": "sha256:" + "b" * 64,
            "executables": [{"role": "application_release"}],
            "predicates": predicates,
            "artifacts": [{"role": role} for role in self.contract["required_artifact_roles"]],
            "comparison": {"trials": trials},
        }
        (self.root / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
        return manifest

    def test_complete_manifest_and_gate_records_are_ready(self) -> None:
        report = {"status": "PASSED", "p10_manifest": "manifest.json"}
        result = reducer.reduce_report(report, contract=self.contract, base=self.root)
        self.assertEqual(result, self.manifest)

    def test_gate_summary_cannot_be_promoted_to_evidence(self) -> None:
        report = {"status": "PASSED", "gates": {"G0": "passed", "G1": "passed"}}
        with self.assertRaisesRegex(reducer.ReductionError, "complete p10_manifest"):
            reducer.reduce_report(report, contract=self.contract, base=self.root)

    def test_missing_predicate_blocks_reduction(self) -> None:
        manifest = dict(self.manifest)
        manifest["predicates"] = dict(manifest["predicates"])
        manifest["predicates"].pop(next(iter(manifest["predicates"])))
        with self.assertRaisesRegex(reducer.ReductionError, "predicate records are incomplete"):
            reducer.reduce_report({"status": "PASSED", "manifest": manifest}, contract=self.contract, base=self.root)

    def test_gate_record_hash_or_kind_mismatch_blocks_reduction(self) -> None:
        predicate, reference = next(iter(self.manifest["predicates"].items()))
        path = self.root / reference["path"]
        path.write_text(json.dumps({"kind": "ordinary_ci_summary"}) + "\n", encoding="utf-8")
        with self.assertRaisesRegex(reducer.ReductionError, "hash mismatch"):
            reducer.reduce_report({"status": "PASSED", "manifest": self.manifest}, contract=self.contract, base=self.root)

    def test_gate_record_with_different_source_identity_blocks_reduction(self) -> None:
        predicate, reference = next(iter(self.manifest["predicates"].items()))
        path = self.root / reference["path"]
        record = json.loads(path.read_text(encoding="utf-8"))
        record["source_commit"] = "c" * 40
        raw = (json.dumps(record, sort_keys=True) + "\n").encode()
        path.write_bytes(raw)
        self.manifest["predicates"][predicate]["sha256"] = digest(raw)
        with self.assertRaisesRegex(reducer.ReductionError, "provenance or status"):
            reducer.reduce_report({"status": "PASSED", "manifest": self.manifest}, contract=self.contract, base=self.root)

    def test_g4_trial_symlink_is_rejected(self) -> None:
        trial = self.root / self.manifest["comparison"]["trials"][0]["path"]
        link = self.root / "g4" / "redirect.json"
        try:
            os.symlink(trial, link)
        except OSError as error:
            self.skipTest(f"symlinks are unavailable on this host: {error}")
        self.manifest["comparison"]["trials"][0] = {
            "path": link.relative_to(self.root).as_posix(),
            "sha256": digest(trial.read_bytes()),
        }
        with self.assertRaisesRegex(reducer.ReductionError, "symbolic link"):
            reducer.reduce_report({"status": "PASSED", "manifest": self.manifest}, contract=self.contract, base=self.root)


if __name__ == "__main__":
    unittest.main()
