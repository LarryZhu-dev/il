import hashlib
import json
import tempfile
import unittest
from pathlib import Path

from tools.p10.repro_records import derive_reproducible_hash


def digest(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


class ReproRecordsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "trusted-run"
        self.output = self.root / "reproducibility"
        self.output.mkdir(parents=True)
        self.commit = "a" * 40
        self.tree = "b" * 40
        self.lock_hash = digest(b"locked toolchain")
        self.report = self.output / "reproduction.json"
        self.value = self.make_report()

    def artifact(self, path: Path, data: bytes):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return {"path": str(path), "sha256": digest(data), "size_bytes": len(data)}

    def make_report(self):
        rows = []
        hashes = None
        archive = digest(b"identical source archive")
        for label in ("root_a", "root_b"):
            base = self.output / label
            source = base / "source"
            source.mkdir(parents=True)
            target = base / "target"
            target.mkdir()
            compiler = self.artifact(target / "release/il", b"compiler bytes")
            runtime = self.artifact(target / "release/libil_native_runtime.a", b"runtime bytes")
            app = base / "app"
            app.mkdir()
            executable = self.artifact(app / "application.elf", b"application ELF bytes")
            log = self.artifact(base / "cargo-build.log", b"build output")
            argv = ["/locked/bin/cargo", "build", "--workspace", "--release", "--locked"]
            execution = {
                "clean_environment": True, "health_check_passed": True, "error": None,
                "exit_code": -15, "termination_signal": "SIGTERM",
                "cleanup_intent": "SIGTERM_after_health", "cleanup_reaped": True,
                "executable_sha256": executable["sha256"],
            }
            root_hashes = {"compiler": compiler["sha256"], "runtime": runtime["sha256"],
                           "executable": executable["sha256"]}
            if hashes is None:
                hashes = root_hashes
            rows.append({
                "label": label,
                "materialized": {"tree": self.tree, "archive_sha256": archive},
                "build": {
                    "argv": argv, "cwd": str(source),
                    "env": {"CARGO_TARGET_DIR": str(target)},
                    "started_monotonic_ns": 100, "ended_monotonic_ns": 200,
                    "exit_code": 0, "error": None,
                    "log": {"path": str(base / "cargo-build.log"), "sha256": log["sha256"]},
                    "target_empty_before_build": True,
                },
                "artifacts": {"compiler": compiler, "runtime": runtime},
                "application": {"artifacts": {"executable": executable}, "execution": execution},
            })
        comparisons = [{"artifact": name, "left": identity, "right": identity, "equal": True}
                       for name, identity in sorted(hashes.items())]
        return {"schema_version": "1.0.0", "kind": "independent_rebuild", "status": "PASSED",
                "source": {"commit": self.commit, "tree": self.tree}, "roots": rows,
                "comparisons": comparisons}

    def save(self):
        self.report.write_text(json.dumps(self.value), encoding="utf-8")

    def derive(self):
        return derive_reproducible_hash(self.report, provenance={
            "source_commit": self.commit, "target": "x86_64-unknown-linux-gnu",
            "toolchain_lock_sha256": self.lock_hash,
        })

    def test_complete_receipt_references_actual_bytes_and_binds_execution(self):
        self.save()
        result = self.derive()
        self.assertEqual(result["status"], "COMPLETE")
        record = result["records"]["reproducible_hash"]
        self.assertEqual(record["predicate"], "reproducible_hash")
        self.assertEqual(record["test_suite"]["count"], 2)
        self.assertEqual([row["store_id"] for row in record["clean_rebuilds"]], ["root_a", "root_b"])
        for rebuild in record["clean_rebuilds"]:
            self.assertTrue(rebuild["clean"])
            self.assertTrue(rebuild["empty_environment"])
            self.assertEqual({item["role"] for item in rebuild["artifacts"]}, {"compiler", "runtime", "native_elf"})
            for artifact in rebuild["artifacts"]:
                path = self.output / Path(artifact["path"])
                self.assertEqual(digest(path.read_bytes()), artifact["sha256"])

    def test_current_report_without_explicit_empty_store_fact_is_blocked(self):
        for row in self.value["roots"]:
            row["build"].pop("target_empty_before_build")
        self.save()
        result = self.derive()
        self.assertEqual(result["status"], "BLOCKED")
        self.assertEqual(result["records"], {})
        self.assertIn("target_empty_before_build", result["blocked_predicates"]["reproducible_hash"])

    def test_changed_native_artifact_bytes_are_rejected(self):
        executable_path = Path(self.value["roots"][0]["application"]["artifacts"]["executable"]["path"])
        executable_path.write_bytes(b"tampered application bytes")
        self.save()
        result = self.derive()
        self.assertEqual(result["status"], "BLOCKED")
        self.assertIn("hash does not match", result["blocked_predicates"]["reproducible_hash"])

    def test_different_artifact_sets_are_rejected(self):
        self.value["roots"][1]["artifacts"]["runtime"]["sha256"] = digest(b"different runtime")
        self.save()
        result = self.derive()
        self.assertEqual(result["status"], "BLOCKED")


if __name__ == "__main__":
    unittest.main()
