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
import assemble  # noqa: E402


HASH = "sha256:"
COMMIT = "a" * 40
TREE = "b" * 40
LOCK = HASH + "c" * 64
PREDICATES = (
    "graph_schema_valid",
    "type_checker_verified",
    "ownership_checker_verified",
    "MIR_verifier_verified",
    "interpreter_semantic_suite_passed",
    "native_elf_generated",
    "clean_environment_runs",
    "no_high_level_target_source",
    "reproducible_hash",
    "inspect_succeeds",
    "slice_succeeds",
    "transaction_add_route_succeeds",
    "stale_transaction_rejected",
    "failure_preserves_previous_revision",
    "blackbox_tests_external",
    "all_artifacts_hashed",
    "diagnostics_machine_readable",
    "rollback_verified",
)


def digest(data: bytes) -> str:
    return HASH + hashlib.sha256(data).hexdigest()


def write_json(path: Path, value: object) -> bytes:
    data = (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    return data


class RunFactory:
    """Build a complete trusted-run fixture with no root-level seed manifest.

    The manifest is deliberately emitted as a CI command output. The assembler
    must discover and rebase command outputs; callers cannot provide a finished
    bundle directly through the journal.
    """

    def __init__(self, root: Path):
        self.root = root
        self.root.mkdir(parents=True)
        self._make_commands()
        self.manifest_path = self.root / "ci" / "result.json"
        self._make_manifest()
        self._make_journal()

    def _make_commands(self) -> None:
        self.command_refs = []
        for command_id in ("ci", "http", "g4", "reproducibility"):
            data = write_json(
                self.root / "commands" / f"{command_id}.json",
                {
                    "schema_version": "1.0.0",
                    "kind": "command_observation",
                    "command_id": command_id,
                    "status": "SUCCESS",
                    "exit_code": 0,
                    "outputs": [],
                },
            )
            self.command_refs.append(
                {"command_id": command_id, "path": f"commands/{command_id}.json", "sha256": digest(data)}
            )

    def _ref(self, relative: str, data: bytes | None = None) -> dict[str, str]:
        path = self.root / relative
        if data is None:
            data = write_json(path, {"kind": "p10_evidence", "status": "PASSED"})
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        return {"path": relative, "sha256": digest(data)}

    def _make_manifest(self) -> None:
        predicates = {
            name: self._ref(f"gates/{name}.json") for name in PREDICATES
        }
        artifacts = [
            {"role": role, **self._ref(f"artifacts/{role}.bin", role.encode())}
            for role in ("graph", "compiler", "runtime", "native_elf", "toolchain_lock")
        ]
        trials = []
        for index in range(18):
            trials.append(self._ref(f"g4/trial-{index + 1:02d}.json", json.dumps({
                "task_id": ("add_route", "fix_type_error", "rollback")[index // 6],
                "implementation": "rust" if index % 2 == 0 else "il",
                "trial_index": index % 3 + 1,
            }, sort_keys=True).encode() + b"\n"))
        manifest = {
            "schema_version": "1.0.0",
            "kind": "p10_acceptance_bundle",
            "source_commit": COMMIT,
            "target": assemble.TARGET,
            "toolchain_lock_sha256": LOCK,
            "executables": [],
            "predicates": predicates,
            "artifacts": artifacts,
            "comparison": {"task_contract_sha256": digest(b"contract"), "baseline_commit": TREE, "trials": trials},
        }
        # This is a command-produced report, not an already assembled bundle.
        write_json(self.manifest_path, manifest)
        http_report = write_json(self.root / "http" / "http_report.json", {
            "suite": "http_external_blackbox", "state": "PASSED", "passed": True,
            "cases": [],
        })
        g4_report = write_json(self.root / "g4" / "comparison.json", {
            "status": "COMPLETE", "trials": [{"g4_trial": trial} for trial in trials],
        })
        repro_report = write_json(self.root / "reproducibility" / "reproduction.json", {
            "status": "PASSED",
        })
        output_paths = {
            "ci": [("ci/result.json", self.manifest_path.read_bytes())],
            "http": [("http/http_report.json", http_report)],
            "g4": [("g4/comparison.json", g4_report)] + [
                (f"g4/trial-{index + 1:02d}.json", (self.root / f"g4/trial-{index + 1:02d}.json").read_bytes())
                for index in range(18)
            ],
            "reproducibility": [("reproducibility/reproduction.json", repro_report)],
        }
        for command_id, outputs in output_paths.items():
            path = self.root / "commands" / f"{command_id}.json"
            observation = json.loads(path.read_text(encoding="utf-8"))
            observation["outputs"] = [
                {"path": relative, "present": True, "sha256": digest(data), "size_bytes": len(data)}
                for relative, data in outputs
            ]
            write_json(path, observation)
        self.command_refs = [
            {"command_id": command_id, "path": f"commands/{command_id}.json",
             "sha256": digest((self.root / "commands" / f"{command_id}.json").read_bytes())}
            for command_id in ("ci", "http", "g4", "reproducibility")
        ]

    def _make_journal(self) -> None:
        journal = {
            "schema_version": "1.0.0",
            "kind": "p10_trusted_run",
            "run_id": "p10_" + "1" * 32,
            "status": "COMPLETE",
            "source": {"repository": str(self.root), "commit": COMMIT, "tree": TREE, "dirty": False, "status": ""},
            "toolchain": {"lock": {"path": "toolchain.lock", "sha256": LOCK}},
            "plan": {"commands": [{"command_id": key} for key in ("ci", "http", "g4", "reproducibility")]},
            "commands": self.command_refs,
        }
        self.journal_path = self.root / "journal.json"
        write_json(self.journal_path, journal)


class AssembleTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="il-p10-assemble-")
        self.addCleanup(self.temp.cleanup)
        self.run = Path(self.temp.name) / "run"
        self.factory = RunFactory(self.run)
        self.output = Path(self.temp.name) / "bundle"

    def _manifest(self) -> dict:
        return json.loads(self.factory.manifest_path.read_text(encoding="utf-8"))

    def _write_manifest(self, value: dict) -> None:
        data = write_json(self.factory.manifest_path, value)
        path = self.run / "commands" / "ci.json"
        observation = json.loads(path.read_text(encoding="utf-8"))
        for output in observation["outputs"]:
            if output["path"] == "ci/result.json":
                output["sha256"] = digest(data)
                output["size_bytes"] = len(data)
        write_json(path, observation)
        self.factory.command_refs = [
            {"command_id": command_id, "path": f"commands/{command_id}.json",
             "sha256": digest((self.run / "commands" / f"{command_id}.json").read_bytes())}
            for command_id in ("ci", "http", "g4", "reproducibility")
        ]
        journal = json.loads(self.factory.journal_path.read_text(encoding="utf-8"))
        journal["commands"] = self.factory.command_refs
        write_json(self.factory.journal_path, journal)

    def test_successful_assembly_rebases_outputs_and_hashes(self) -> None:
        result = assemble.assemble(self.factory.journal_path, self.output)
        self.assertEqual(result["kind"], "p10_acceptance_bundle")
        self.assertTrue((self.output / "manifest.json").is_file())
        self.assertTrue((self.output / ".assemble-source").is_file())
        self.assertFalse((self.output / "manifest.json").resolve() == self.factory.manifest_path.resolve())
        artifact = result["artifacts"][0]
        self.assertFalse(Path(artifact["path"]).is_absolute())
        self.assertEqual(digest((self.output / artifact["path"]).read_bytes()), artifact["sha256"])

    def test_unreferenced_large_run_file_is_not_archived(self) -> None:
        large = self.run / "http" / "temporary" / "compiler-cache.bin"
        large.parent.mkdir(parents=True)
        with large.open("wb") as stream:
            stream.truncate(assemble.MAX_BUNDLE_BYTES + 1)

        assemble.assemble(self.factory.journal_path, self.output)

        self.assertFalse((self.output / "records" / "http" / "temporary" / "compiler-cache.bin").exists())

    def test_failed_journal_is_rejected(self) -> None:
        journal = json.loads(self.factory.journal_path.read_text(encoding="utf-8"))
        journal["status"] = "FAILED"
        write_json(self.factory.journal_path, journal)
        with self.assertRaises(assemble.AssembleError):
            assemble.assemble(self.factory.journal_path, self.output)
        self.assertFalse(self.output.exists())

    def test_missing_referenced_output_fails_closed(self) -> None:
        self.factory.manifest_path.unlink()
        with self.assertRaises(assemble.AssembleError):
            assemble.assemble(self.factory.journal_path, self.output)
        self.assertFalse(self.output.exists())

    def test_plain_ci_summary_is_not_promoted_to_acceptance_manifest(self) -> None:
        summary = {
            "schema_version": "1.0.0",
            "status": "PASSED",
            "scope": "ci",
            "source_commit": COMMIT,
            "target": assemble.TARGET,
            "gates": [{"name": "rust-tests", "status": "PASSED", "test_count": 236}],
        }
        self._write_manifest(summary)
        with self.assertRaises(assemble.AssembleError) as raised:
            assemble.assemble(self.factory.journal_path, self.output)
        self.assertIn("explicit p10_acceptance_bundle manifest", str(raised.exception))
        self.assertFalse(self.output.exists())

    def test_derived_ci_and_http_records_without_inventory_report_blocked_sections(self) -> None:
        self._write_manifest({"schema_version": "1.0.0", "status": "PASSED", "gates": []})
        record_refs = {}
        for predicate in ("graph_schema_valid", "blackbox_tests_external"):
            record = {
                "schema_version": "1.0.0", "kind": "p10_gate_record", "predicate": predicate,
                "source_commit": COMMIT, "target": assemble.TARGET,
                "toolchain_lock_sha256": LOCK, "status": "PASSED",
            }
            relative = f"evidence/gates/{predicate}.json"
            raw = write_json(self.run / relative, record)
            record_refs[predicate] = {"path": relative, "sha256": digest(raw)}
        export = {
            "schema_version": "1.0.0",
            "kind": "p10_gate_record_export",
            "status": "BLOCKED",
            "records": record_refs,
            "missing_predicates": sorted(set(PREDICATES) - {"graph_schema_valid", "blackbox_tests_external"}),
            "malformed_records": [],
            "source_refs": [],
        }
        export_path = self.run / "gate_records" / "export.json"
        export_data = write_json(export_path, export)
        journal = json.loads(self.factory.journal_path.read_text(encoding="utf-8"))
        journal["gate_records"] = {
            "path": "gate_records/export.json",
            "sha256": digest(export_data),
            "status": "BLOCKED",
            "missing_predicates": export["missing_predicates"],
        }
        write_json(self.factory.journal_path, journal)

        with self.assertRaises(assemble.AssembleError) as raised:
            assemble.assemble(self.factory.journal_path, self.output)

        message = str(raised.exception)
        self.assertIn("P10 bundle BLOCKED", message)
        self.assertIn("missing predicates=", message)
        self.assertIn("missing inventory sections=['executables', 'artifacts']", message)
        self.assertIn("gate_records/export.json", message)
        self.assertIn("http/http_report.json", message)
        self.assertFalse(self.output.exists())

    def test_absent_output_inventory_entries_are_not_captured(self) -> None:
        path = self.run / "commands" / "http.json"
        observation = json.loads(path.read_text(encoding="utf-8"))
        observation["outputs"] = [
            {"path": "http/http_report.json", "present": False,
             "sha256": digest((self.run / "http" / "http_report.json").read_bytes())},
        ]
        write_json(path, observation)
        self.factory.command_refs = [
            {"command_id": command_id, "path": f"commands/{command_id}.json",
             "sha256": digest((self.run / "commands" / f"{command_id}.json").read_bytes())}
            for command_id in ("ci", "http", "g4", "reproducibility")
        ]
        journal = json.loads(self.factory.journal_path.read_text(encoding="utf-8"))
        journal["commands"] = self.factory.command_refs
        write_json(self.factory.journal_path, journal)
        with self.assertRaises(assemble.AssembleError) as raised:
            assemble.assemble(self.factory.journal_path, self.output)
        self.assertIn("trusted http output", str(raised.exception))
        self.assertFalse(self.output.exists())

    def test_command_hash_tampering_is_rejected(self) -> None:
        path = self.run / "commands" / "http.json"
        path.write_text('{"kind":"command_observation","command_id":"http","status":"SUCCESS","tampered":true}\n', encoding="utf-8")
        with self.assertRaises(assemble.AssembleError) as raised:
            assemble.assemble(self.factory.journal_path, self.output)
        self.assertIn("hash mismatch", str(raised.exception))
        self.assertFalse(self.output.exists())

    def test_referenced_file_hash_tampering_is_rejected(self) -> None:
        artifact = self.run / "artifacts" / "graph.bin"
        artifact.write_bytes(b"tampered")
        with self.assertRaises(assemble.AssembleError):
            assemble.assemble(self.factory.journal_path, self.output)
        self.assertFalse(self.output.exists())

    def test_path_traversal_reference_is_rejected(self) -> None:
        manifest = self._manifest()
        manifest["predicates"][PREDICATES[0]]["path"] = "../outside.json"
        manifest["predicates"][PREDICATES[0]]["sha256"] = digest(b"outside")
        self._write_manifest(manifest)
        with self.assertRaises(assemble.AssembleError):
            assemble.assemble(self.factory.journal_path, self.output)
        self.assertFalse(self.output.exists())

    def test_absolute_reference_inside_trusted_run_is_rebased(self) -> None:
        absolute = self.run / "http" / "absolute-receipt.json"
        data = write_json(absolute, {"kind": "http_receipt", "passed": True})
        manifest = self._manifest()
        manifest["predicates"][PREDICATES[0]] = {"path": str(absolute), "sha256": digest(data)}
        self._write_manifest(manifest)
        result = assemble.assemble(self.factory.journal_path, self.output)
        reference = result["predicates"][PREDICATES[0]]
        self.assertFalse(Path(reference["path"]).is_absolute())
        self.assertEqual(digest((self.output / reference["path"]).read_bytes()), reference["sha256"])

    def test_container_absolute_reference_rebases_by_trusted_run_name(self) -> None:
        source = self.run / "http" / "receipt.json"
        data = write_json(source, {"kind": "http_receipt", "passed": True})
        manifest = self._manifest()
        manifest["predicates"][PREDICATES[0]] = {
            "path": f"/workspace/build/p10/{self.run.name}/http/receipt.json",
            "sha256": digest(data),
        }
        self._write_manifest(manifest)
        result = assemble.assemble(self.factory.journal_path, self.output)
        reference = result["predicates"][PREDICATES[0]]
        self.assertEqual(digest((self.output / reference["path"]).read_bytes()), reference["sha256"])

    @unittest.skipUnless(hasattr(Path, "symlink_to"), "symlink support unavailable")
    def test_symbolic_link_in_trusted_run_is_rejected(self) -> None:
        target = self.run / "real.json"
        target.write_text("{}\n", encoding="utf-8")
        link = self.run / "linked.json"
        try:
            link.symlink_to(target)
        except (OSError, NotImplementedError):
            self.skipTest("symbolic links are unavailable")
        with self.assertRaises(assemble.AssembleError):
            assemble.assemble(self.factory.journal_path, self.output)
        self.assertFalse(self.output.exists())

    def test_existing_output_is_never_overwritten(self) -> None:
        self.output.mkdir()
        marker = self.output / "keep.txt"
        marker.write_text("keep", encoding="utf-8")
        with self.assertRaises(assemble.AssembleError):
            assemble.assemble(self.factory.journal_path, self.output)
        self.assertEqual(marker.read_text(encoding="utf-8"), "keep")

    def test_failed_assembly_cleans_partial_output_atomically(self) -> None:
        original = assemble.atomic_write

        def fail_once(path: Path, data: bytes) -> None:
            raise OSError("injected atomic write failure")

        with mock.patch.object(assemble, "atomic_write", side_effect=fail_once):
            with self.assertRaises(OSError):
                assemble.assemble(self.factory.journal_path, self.output)
        self.assertFalse(self.output.exists())
        self.assertEqual(list(Path(self.temp.name).glob("*.pending")), [])
        self.assertIsNotNone(original)


if __name__ == "__main__":
    unittest.main()
