from __future__ import annotations

import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "p10"))
import produce  # noqa: E402


class ProducerTests(unittest.TestCase):
    def test_rollback_record_binds_three_trials_to_retained_release_bytes(self):
        with tempfile.TemporaryDirectory(prefix="il-p10-rollback-record-") as name:
            run = Path(name)
            (run / "evidence/gates").mkdir(parents=True)
            (run / "g4").mkdir()
            binary = b"\x7fELF" + bytes(range(32))
            binary_hash = produce.digest_bytes(binary)
            graph_hash = "sha256:" + "a" * 64
            (run / "reproducibility").mkdir()
            (run / "reproducibility/release.elf").write_bytes(binary)
            (run / "evidence/gates/reproducible_hash.json").write_text(json.dumps({
                "duration_ns": 3,
                "clean_rebuilds": [{"artifacts": [{"role": "native_elf", "sha256": binary_hash}]}],
            }), encoding="utf-8")
            trials = []
            for index in range(1, 4):
                directory = run / "g4" / f"rollback-il-{index}"
                (directory / "blobs").mkdir(parents=True)
                (directory / "blobs/previous.elf").write_bytes(binary)
                record = {
                    "experiment_exit_code": 0,
                    "started_monotonic_ns": index * 10,
                    "finished_monotonic_ns": index * 10 + 5,
                    "assertions": [{"id": identity, "exit_code": 0}
                                   for identity in ("rollback_transaction", "restored_graph_matches", "previous_binary_runs")],
                    "rollback_evidence": {
                        "source_revision": 2, "rollback_revision": 3,
                        "expected_graph_sha256": graph_hash, "restored_graph_sha256": graph_hash,
                        "previous_binary_sha256": binary_hash, "previous_binary_executed": True,
                    },
                    "rollback_binary": {"path": "blobs/previous.elf", "sha256": binary_hash,
                                         "bytes": len(binary), "role": "rollback_binary"},
                }
                raw = json.dumps(record, sort_keys=True).encode()
                (directory / "g4_trial.json").write_bytes(raw)
                trials.append({"task": "rollback", "implementation": "il", "g4_trial": {
                    "path": f"rollback-il-{index}/g4_trial.json", "sha256": produce.digest_bytes(raw),
                }})
            (run / "g4/comparison.json").write_text(json.dumps({"trials": trials}), encoding="utf-8")
            journal = {"source": {"commit": "b" * 40},
                       "toolchain": {"lock": {"sha256": "sha256:" + "c" * 64}}}

            result = produce._derive_rollback_record(run, journal)

            self.assertEqual(result, {"status": "COMPLETE", "blocked_predicates": []})
            evidence = json.loads((run / "evidence/gates/rollback_verified.json").read_text(encoding="utf-8"))
            self.assertEqual(evidence["rollback"]["previous_binary_sha256"], binary_hash)
            self.assertEqual(evidence["artifacts"][0]["sha256"], binary_hash)

    def test_fixed_plan_captures_real_ci_outputs_and_run_local_reports(self):
        plan = produce._fixed_plan(Path("p10-run"))
        self.assertEqual({item.command_id for item in plan}, {"ci", "http", "g4", "reproducibility"})
        ci = next(item for item in plan if item.command_id == "ci")
        self.assertIn("build/ci/result.json", ci.output_paths)
        self.assertIn("build/ci/*.log", ci.output_paths)
        self.assertIn("build/ci/*_report.json", ci.output_paths)
        self.assertNotIn("build/ci/manifest.json", ci.output_paths)
        self.assertEqual(next(item for item in plan if item.command_id == "http").run_output_paths,
                         ("http/http_report.json", "http/**"))
        self.assertEqual(next(item for item in plan if item.command_id == "g4").run_output_paths,
                         ("g4/comparison.json", "g4/**"))
        self.assertEqual(next(item for item in plan if item.command_id == "reproducibility").run_output_paths,
                         ("reproducibility/reproduction.json", "reproducibility/**"))

    def test_output_capture_binds_checkout_and_run_local_files(self):
        with tempfile.TemporaryDirectory(prefix="il-p10-output-capture-") as name:
            root = Path(name)
            checkout = root / "checkout"
            run = root / "run"
            (checkout / "build" / "ci").mkdir(parents=True)
            (checkout / "build" / "ci" / "result.json").write_bytes(b"ci result")
            (checkout / "build" / "ci" / "gate.log").write_bytes(b"gate log")
            (run / "http" / "http-receipts").mkdir(parents=True)
            (run / "http" / "http_report.json").write_bytes(b"http report")
            (run / "http" / "http-receipts" / "request").write_bytes(b"request bytes")
            with mock.patch.object(produce, "ROOT", checkout):
                checkout_outputs = produce._capture_checkout_paths(
                    run, ("build/ci/result.json", "build/ci/*.log"))
            run_outputs = produce._capture_run_paths(run, ("http/**",))

            self.assertEqual({row["path"] for row in checkout_outputs if row["present"]},
                             {"build/ci/result.json", "build/ci/gate.log"})
            self.assertEqual({row["path"] for row in run_outputs if row["present"]},
                             {"http/http_report.json", "http/http-receipts/request"})
            self.assertFalse(any(row["path"] == "http/**" and not row["present"] for row in run_outputs))
            for row in checkout_outputs + run_outputs:
                if row["present"]:
                    path = run / row["path"]
                    self.assertEqual(row["sha256"], produce.digest_file(path))

    def test_cli_accepts_only_output_and_rejects_caller_command(self):
        with tempfile.TemporaryDirectory(prefix="il-p10-cli-") as name:
            with self.assertRaises(SystemExit) as raised:
                produce.main(["--output", str(Path(name) / "run"), "--command", "echo unsafe"])
        self.assertEqual(raised.exception.code, 2)

    def test_atomic_json_and_blob_leave_complete_content_without_pending_files(self):
        with tempfile.TemporaryDirectory(prefix="il-p10-atomic-") as name:
            root = Path(name)
            target = root / "journal.json"
            blob_root = root / "evidence"
            replacements: list[tuple[Path, Path]] = []
            original_replace = produce.os.replace

            def record_replace(source: str | os.PathLike[str], destination: str | os.PathLike[str]) -> None:
                replacements.append((Path(source), Path(destination)))
                original_replace(source, destination)

            with mock.patch.object(produce.os, "replace", side_effect=record_replace):
                produce.atomic_json(target, {"status": "RUNNING", "value": 7})
                reference = produce._blob(blob_root, b"evidence bytes")

            self.assertEqual(json.loads(target.read_text(encoding="utf-8"))["value"], 7)
            self.assertEqual((blob_root / reference["path"]).read_bytes(), b"evidence bytes")
            self.assertGreaterEqual(len(replacements), 2)
            self.assertFalse(any(path.name.endswith(".pending") for path in root.rglob("*")))

    def test_failed_child_preserves_journal_and_stream_blobs(self):
        with tempfile.TemporaryDirectory(prefix="il-p10-failure-") as name:
            root = Path(name)
            script = root / "fail.py"
            script.write_text(
                "import sys\n"
                "sys.stdout.write('stdout evidence')\n"
                "sys.stderr.write('stderr evidence')\n"
                "raise SystemExit(7)\n",
                encoding="utf-8",
            )
            output = root / "run"
            spec = produce.CommandSpec(
                "failure", (sys.executable, str(script)), 10, "controlled failure"
            )
            source = {
                "repository": str(root),
                "commit": "a" * 40,
                "tree": "b" * 40,
                "dirty": False,
                "status": "",
            }
            with mock.patch.object(produce, "source_identity", return_value=source), \
                    mock.patch.object(produce, "_toolchain_identity", return_value={"lock": {}, "tools": {}}), \
                    mock.patch.object(produce, "_fixed_plan", return_value=(spec,)):
                result = produce.run_plan(output, root=root)

            self.assertEqual(result["status"], "FAILED")
            journal = json.loads((output / "journal.json").read_text(encoding="utf-8"))
            self.assertEqual(journal["status"], "FAILED")
            self.assertEqual(journal["failure"]["command_id"], "failure")
            observation = json.loads((output / "commands" / "failure.json").read_text(encoding="utf-8"))
            self.assertEqual(observation["status"], "FAILED")
            self.assertEqual(observation["exit_code"], 7)
            self.assertEqual((output / observation["stdout"]["path"]).read_bytes(), b"stdout evidence")
            self.assertEqual((output / observation["stderr"]["path"]).read_bytes(), b"stderr evidence")

    def test_complete_run_retains_fail_closed_gate_export_sidecar(self):
        with tempfile.TemporaryDirectory(prefix="il-p10-gate-export-") as name:
            root = Path(name)
            script = root / "ok.py"
            script.write_text("print('ok')\n", encoding="utf-8")
            output = root / "run"
            spec = produce.CommandSpec("ok", (sys.executable, str(script)), 10, "controlled success")
            source = {
                "repository": str(root), "commit": "a" * 40, "tree": "b" * 40,
                "dirty": False, "status": "",
            }
            with mock.patch.object(produce, "source_identity", return_value=source), \
                    mock.patch.object(produce, "_toolchain_identity", return_value={"lock": {}, "tools": {}}), \
                    mock.patch.object(produce, "_fixed_plan", return_value=(spec,)):
                result = produce.run_plan(output, root=root)
            self.assertEqual(result["status"], "COMPLETE")
            export = json.loads((output / "gate_records" / "export.json").read_text(encoding="utf-8"))
            self.assertEqual(export["status"], "BLOCKED")
            self.assertTrue(export["missing_predicates"])
            journal = json.loads((output / "journal.json").read_text(encoding="utf-8"))
            self.assertEqual(journal["gate_records"]["status"], "BLOCKED")
            self.assertEqual(journal["p10_report"]["kind"], "p10_producer_report")
            self.assertEqual(journal["p10_report"]["status"], "BLOCKED")
            report_path = output / journal["p10_report"]["path"]
            report = json.loads(report_path.read_text(encoding="utf-8"))
            self.assertEqual(report["code"], "E_EVIDENCE_INCOMPLETE")
            self.assertIn("CI result.json", report["error"])

    def test_producer_never_promotes_ordinary_ci_summary(self):
        with tempfile.TemporaryDirectory(prefix="il-p10-producer-summary-") as name:
            root = Path(name)
            run = root / "run"
            (run / "build" / "ci").mkdir(parents=True)
            (run / "build" / "ci" / "result.json").write_text(
                json.dumps({"status": "PASSED", "gates": {"G0": "passed"}}),
                encoding="utf-8",
            )
            (run / "journal.json").write_text(json.dumps({
                "source": {"commit": "a" * 40, "tree": "b" * 40},
                "toolchain": {"lock": {"sha256": "sha256:" + "c" * 64}},
            }), encoding="utf-8")
            report = produce._produce_p10_report(run, root=ROOT)
            self.assertEqual(report["kind"], "p10_producer_report")
            self.assertEqual(report["status"], "BLOCKED")
            self.assertIn("gate-record export", report["error"])

    def test_dirty_checkout_is_rejected_before_execution(self):
        with tempfile.TemporaryDirectory(prefix="il-p10-dirty-") as name:
            root = Path(name)
            output = root / "run"
            dirty = {
                "repository": str(root), "commit": "a" * 40, "tree": "b" * 40,
                "dirty": True, "status": "?? uncommitted.il",
            }
            with mock.patch.object(produce, "source_identity", return_value=dirty):
                with self.assertRaises(produce.ProducerError) as raised:
                    produce.run_plan(output, root=root)
            self.assertIn("clean committed checkout", str(raised.exception))
            self.assertFalse((output / "journal.json").exists())

    def test_existing_output_directory_is_never_overwritten(self):
        with tempfile.TemporaryDirectory(prefix="il-p10-existing-") as name:
            root = Path(name)
            output = root / "run"
            output.mkdir()
            marker = output / "marker.txt"
            marker.write_text("keep", encoding="utf-8")
            with self.assertRaises(produce.ProducerError):
                produce.run_plan(output, root=root)
            self.assertEqual(marker.read_text(encoding="utf-8"), "keep")

    def test_publication_is_immutable_and_idempotent(self):
        with tempfile.TemporaryDirectory(prefix="il-p10-publish-") as name:
            root = Path(name)
            run = root / "run"
            run.mkdir()
            command = {
                "schema_version": produce.SCHEMA_VERSION,
                "kind": "command_observation",
                "command_id": "ci",
                "status": "SUCCESS",
            }
            produce.atomic_json(run / "commands" / "ci.json", command)
            journal = {
                "schema_version": produce.SCHEMA_VERSION,
                "kind": "p10_trusted_run",
                "run_id": "p10_" + "1" * 32,
                "status": "COMPLETE",
                "source": {"commit": "a" * 40, "tree": "b" * 40},
                "toolchain": {"lock": {"sha256": "sha256:" + "c" * 64}},
                "commands": [{
                    "command_id": "ci",
                    "path": "commands/ci.json",
                    "sha256": produce.digest_file(run / "commands" / "ci.json"),
                }],
            }
            produce.atomic_json(run / "journal.json", journal)
            release = root / "release"
            first = produce.publish_release(run, release)
            second = produce.publish_release(run, release)
            self.assertEqual(second, first)

            journal["finished_at"] = "changed"
            produce.atomic_json(run / "journal.json", journal)
            with self.assertRaises(produce.ProducerError):
                produce.publish_release(run, release)


if __name__ == "__main__":
    unittest.main()
