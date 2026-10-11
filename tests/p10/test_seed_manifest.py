from __future__ import annotations

import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "p10"))
import seed_manifest  # noqa: E402


def digest(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


class SeedManifestTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory(prefix="il-p10-seed-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.contract = json.loads((ROOT / "tools/p10/contract.json").read_text())
        (self.root / "build/ci").mkdir(parents=True)

    def write(self, relative: str, value: object) -> dict[str, str]:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        raw = (json.dumps(value, sort_keys=True) + "\n").encode()
        path.write_bytes(raw)
        return {"path": relative, "sha256": digest(raw)}

    def test_summary_is_blocked_without_predicate_inference(self) -> None:
        self.write("build/ci/result.json", {"status": "PASSED", "gates": [{"name": "all", "status": "PASSED", "test_count": 999}]})
        result = seed_manifest.build_seed(self.root, contract=self.contract)
        self.assertEqual(result["status"], "BLOCKED")
        self.assertEqual(set(result["blocked_predicates"]), set(seed_manifest._predicates(self.contract)))
        self.assertIn("explicit p10_acceptance_bundle", result["reasons"][0])

    def test_explicit_seed_checks_all_references(self) -> None:
        names = seed_manifest._predicates(self.contract)
        predicates = {name: self.write(f"records/{name}.json", {"predicate": name}) for name in names}
        artifacts = [{"role": role, **self.write(f"artifacts/{role}.bin", role)} for role in self.contract["required_artifact_roles"]]
        trials = [self.write(f"g4/{index}.json", {"kind": "g4_trial", "index": index}) for index in range(18)]
        manifest = {
            "schema_version": "1.0.0", "kind": "p10_acceptance_bundle", "source_commit": "a" * 40,
            "target": self.contract["target"], "toolchain_lock_sha256": "sha256:" + "b" * 64,
            "executables": [{"role": "application_release"}], "predicates": predicates,
            "artifacts": artifacts, "comparison": {"trials": trials},
        }
        self.write("build/ci/result.json", manifest)
        result = seed_manifest.build_seed(self.root, contract=self.contract)
        self.assertEqual(result["status"], "COMPLETE")
        self.assertEqual(len(result["manifest"]["predicates"]), 18)

    def test_hash_mismatch_blocks_and_never_returns_manifest(self) -> None:
        names = seed_manifest._predicates(self.contract)
        predicates = {name: self.write(f"records/{name}.json", {"predicate": name}) for name in names}
        predicates[names[0]]["sha256"] = "sha256:" + "0" * 64
        manifest = {
            "schema_version": "1.0.0", "kind": "p10_acceptance_bundle", "source_commit": "a" * 40,
            "target": self.contract["target"], "toolchain_lock_sha256": "sha256:" + "b" * 64,
            "executables": [{"role": "application_release"}], "predicates": predicates,
            "artifacts": [], "comparison": {"trials": []},
        }
        self.write("build/ci/result.json", manifest)
        result = seed_manifest.build_seed(self.root, contract=self.contract)
        self.assertEqual(result["status"], "BLOCKED")
        self.assertNotIn("manifest", result)


if __name__ == "__main__":
    unittest.main()

