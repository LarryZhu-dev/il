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
