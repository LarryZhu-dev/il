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
        self.manifest_path = self.root / "ci" / "manifest.json"
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
        trials = [self._ref("g4/trial-01.json") for _ in range(18)]
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
        write_json(self.factory.manifest_path, value)

    def test_successful_assembly_rebases_outputs_and_hashes(self) -> None:
        result = assemble.assemble(self.factory.journal_path, self.output)
        self.assertEqual(result["kind"], "p10_acceptance_bundle")
        self.assertTrue((self.output / "manifest.json").is_file())
        self.assertTrue((self.output / ".assemble-source").is_file())
        self.assertFalse((self.output / "manifest.json").resolve() == self.factory.manifest_path.resolve())
        artifact = result["artifacts"][0]
        self.assertFalse(Path(artifact["path"]).is_absolute())
        self.assertEqual(digest((self.output / artifact["path"]).read_bytes()), artifact["sha256"])

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
