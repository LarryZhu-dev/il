import copy
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.p10 import compare, runner
from tools.p10.compare import G4_BASELINE_DATE, Trace, initialize_rust_repository


class G4ComparisonTests(unittest.TestCase):
    def test_trial_capture_preserves_all_18_failed_attempts_and_measured_metrics(self):
        # Exercise actual trial orchestration, Git repositories, edits and records.
        # Only the unavailable native compilers are replaced; these are failures,
        # not simulated passing acceptance receipts.
        execution, acceptance = compare.load_contracts()
        self.assertNotIn("g4", execution)
        run_process = subprocess.run

        def unavailable_compiler(argv, **kwargs):
            if str(argv[0]) == "git":
                return run_process(argv, **kwargs)
            raise FileNotFoundError(2, "compiler intentionally unavailable in capture regression")

        with tempfile.TemporaryDirectory(prefix="il-g4-capture-") as temporary:
            root = Path(temporary)
            rows = []
            with patch.object(compare.subprocess, "run", side_effect=unavailable_compiler):
                for task in execution["tasks"]:
                    for implementation in execution["implementations"]:
                        for index in range(1, execution["repetitions"] + 1):
                            directory = root / f"{task}-{implementation}-{index}"
                            row = compare.trial(directory, implementation, task, index,
                                                root / "unavailable-il", execution, acceptance)
                            rows.append(row)
                            self.assertFalse(row["success"])
                            self.assertEqual(row["failure_category"], "environment", row.get("error"))
                            self.assertTrue((directory / "trial.json").is_file())
                            raw_trace = json.loads((directory / "trace.json").read_bytes())
                            measured = [event for event in raw_trace if event["phase"] == "measured"]
                            exported = json.loads((directory / "g4_trace.json").read_bytes())
                            self.assertEqual(row["tool_calls"], len(measured))
                            self.assertEqual(len(exported["events"]), len(measured))
                            self.assertEqual([event["sequence"] for event in exported["events"]],
                                             list(range(1, len(measured) + 1)))
                            self.assertIn("error", raw_trace[-1]["response"])
                            record = json.loads((directory / "g4_trial.json").read_bytes())
                            counts = {assertion["id"]: assertion["count"] for assertion in record["assertions"]}
                            self.assertEqual(set(counts), set(acceptance["g4"]["tasks"][task]))
                            if task == "add_route":
                                self.assertEqual(counts["typecheck"], 0)
                            self.assertEqual(row["task_input_sha256"], compare.digest(compare.encoded({
                                "task": task, "execution_contract": execution,
                                "acceptance_contract": acceptance,
                            })))

            rust_rows = [row for row in rows if row["implementation"] == "rust"]
            self.assertEqual(len({row["starting_commit"] for row in rust_rows}), 1)
            manifest = {"source_commit": rows[0]["source_commit"], "comparison": {
                "baseline_commit": rust_rows[0]["starting_commit"],
                "trials": [row["g4_trial"] for row in rows],
            }}
            verified = runner.verify_trials(runner.BundleReader(root, 32 * 1024 * 1024), manifest, acceptance)
            self.assertEqual(len(verified["trials"]), 18)
            for captured, checked in zip(rows, verified["trials"]):
                self.assertFalse(checked["success"])
                self.assertEqual(captured["tool_calls"], checked["tool_calls"])
                self.assertEqual(captured["changes"], checked["changes"])
                if captured["elapsed_ns"] is not None:
                    self.assertEqual(captured["elapsed_ns"] / 1_000_000, checked["elapsed_ms"])
            summary = compare.aggregate(rows, execution, source_dirty=False)
            self.assertTrue(summary["matrix_complete"])
            self.assertFalse(summary["all_trials_succeeded"])
            self.assertFalse(summary["release_eligible"])

    def test_release_eligibility_requires_complete_success_and_clean_source(self):
        execution, _ = compare.load_contracts()
        rows = [{"implementation": impl, "task": task, "repetition": index,
                 "success": True, "failure_category": None, "elapsed_ns": 10}
                for impl in execution["implementations"] for task in execution["tasks"]
                for index in range(1, execution["repetitions"] + 1)]
        self.assertTrue(compare.aggregate(rows, execution, source_dirty=False)["release_eligible"])
        self.assertFalse(compare.aggregate(rows, execution, source_dirty=True)["release_eligible"])
        self.assertFalse(compare.aggregate(rows[:-1], execution, source_dirty=False)["release_eligible"])
        rows[0].update(success=False, failure_category="compile")
        self.assertFalse(compare.aggregate(rows, execution, source_dirty=False)["release_eligible"])

    def test_contract_mismatch_is_rejected_before_trials_start(self):
        execution, acceptance = compare.load_contracts()
        mismatched = copy.deepcopy(acceptance)
        mismatched["g4"]["tasks"].pop("rollback")
        with patch.object(Path, "read_bytes", side_effect=[compare.encoded(execution), compare.encoded(mismatched)]):
            with self.assertRaisesRegex(AssertionError, "task contracts differ"):
                compare.load_contracts()

    def test_rust_baseline_commit_is_reproducible_and_its_date_is_traced(self):
        with tempfile.TemporaryDirectory(prefix="il-g4-baseline-") as temporary:
            root = Path(temporary)
            commits = []
            traces = []
            for identity in ("first", "second"):
                repository = root / identity
                repository.mkdir()
                (repository / "main.rs").write_text("fn main() {}\n", encoding="utf-8")
                trace = Trace(root / f"{identity}-trace")
                _, commit = initialize_rust_repository(trace, repository)
                commits.append(commit)
                traces.append(trace)

            self.assertEqual(len(commits[0]), 40)
            self.assertEqual(commits[0], commits[1])
            for trace in traces:
                commit_event = next(
                    event for event in trace.events
                    if event["request"]["argv"][-2:] == ["-m", "test: pin G4 baseline"]
                )
                self.assertEqual(commit_event["request"]["environment"], {
                    "GIT_AUTHOR_DATE": G4_BASELINE_DATE,
                    "GIT_COMMITTER_DATE": G4_BASELINE_DATE,
                })


if __name__ == "__main__":
    unittest.main()
