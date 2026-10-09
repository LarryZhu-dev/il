"""Negative contract tests for P00's read-only gate and schema vocabulary."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("bootstrap_check", ROOT / "tools/bootstrap_check.py")
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)


class BootstrapSchemaTests(unittest.TestCase):
    def state(self):
        return CHECK.read_json(ROOT / "repository_state.json")

    def task(self):
        return CHECK.read_json(ROOT / "eval/tasks/P00.json")

    def rejects(self, value, name):
        with self.assertRaises(CHECK.ValidationError):
            CHECK.validate_file(ROOT, value, name)

    def test_actual_state_and_all_tasks_validate(self):
        CHECK.validate_file(ROOT, self.state(), "repository_state")
        for path in (ROOT / "eval/tasks").glob("*.json"):
            CHECK.validate_file(ROOT, CHECK.read_json(path), "task")

    def test_state_rejects_unknown_fields(self):
        state = self.state()
        state["unreviewed_option"] = True
        self.rejects(state, "repository_state")

    def test_state_rejects_unknown_nested_lock_fields(self):
        state = self.state()
        state["locks"] = {"graph": {"owner": "worker", "revision": 0, "override": True}}
        self.rejects(state, "repository_state")

    def test_state_rejects_non_linux_first_target(self):
        state = self.state()
        state["target"] = "x86_64-pc-windows-msvc"
        self.rejects(state, "repository_state")

    def test_revision_is_uint64_not_boolean(self):
        for invalid in (-1, 2**64, True, "0"):
            state = self.state()
            state["head_revision"] = invalid
            self.rejects(state, "repository_state")

    def test_task_rejects_undefined_status(self):
        task = self.task()
        task["status"] = "DONE"
        self.rejects(task, "task")

    def test_task_requires_positive_finite_budgets(self):
        for invalid in (0, -1, 2**32, "unlimited"):
            task = self.task()
            task["resource_budget"]["tool_calls"] = invalid
            self.rejects(task, "task")

    def test_task_rejects_unknown_budget_fields(self):
        task = self.task()
        task["resource_budget"]["shell"] = True
        self.rejects(task, "task")

    def test_task_requires_nonempty_acceptance(self):
        task = self.task()
        task["acceptance"] = []
        self.rejects(task, "task")

    def test_host_policy_schema_rejects_unknown_and_unsafe_fault_controls(self):
        policy = {"schema_version": "1.0.0", "grants": [], "test_faults": None}
        CHECK.validate_file(ROOT, policy, "host_policy")
        for mutation in (dict(policy, shell="command"), dict(policy, test_faults={}),
                         dict(policy, test_faults={"allocation_fail_after": None, "io_max_chunk": 0, "io_fail_after": None}),
                         dict(policy, grants=[{"entity_id": "grant", "kind": "FileRead", "scope": "relative"}])):
            self.rejects(mutation, "host_policy")

    def test_runtime_build_request_rejects_mismatched_exports(self):
        schema_path = ROOT / "schema/tool.schema.json"
        schema = CHECK.read_json(schema_path)
        build = schema["$defs"]["build"]
        request = {"revision": 1, "target": CHECK.TARGET, "profile": "release"}
        CHECK.validate_instance(request, build, schema_path, root_schema=schema)
        CHECK.validate_instance(dict(request, runtime_profile="none", exports=["main"]), build, schema_path, root_schema=schema)
        for mutation in (dict(request, exports=["main"]), dict(request, runtime_profile="none"),
                         dict(request, runtime_profile="minimal", exports=["main"]),
                         dict(request, runtime_profile="none", exports=[])):
            with self.assertRaises(CHECK.ValidationError):
                CHECK.validate_instance(mutation, build, schema_path, root_schema=schema)

    def test_duplicate_keys_and_non_json_numbers_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "input.json"
            for text in ('{"x":1,"x":2}', '{"x":NaN}', '{"x":Infinity}'):
                path.write_text(text, encoding="utf-8")
                with self.assertRaises(CHECK.ValidationError):
                    CHECK.read_json(path)

    def test_unknown_schema_assertions_fail_closed_inside_condition(self):
        for schema in ({"imaginaryConstraint": True}, {"if": {"imaginaryConstraint": True}, "then": False}, {"anyOf": [{"imaginaryConstraint": True}, True]}):
            with self.assertRaises(CHECK.UnsupportedSchema):
                CHECK.validate_instance({}, schema, ROOT / "schema/test.json")

    def test_local_ref_sibling_and_condition_constraints(self):
        schema = {"$defs": {"count": {"type": "integer", "minimum": 0}}, "$ref": "#/$defs/count", "maximum": 2}
        CHECK.validate_instance(1, schema, ROOT / "schema/test.json")
        for value in (-1, 3, True):
            with self.assertRaises(CHECK.ValidationError):
                CHECK.validate_instance(value, schema, ROOT / "schema/test.json")

    def test_artifact_must_be_an_existing_repository_file(self):
        with self.assertRaises(CHECK.ValidationError):
            CHECK.safe_artifact(ROOT, "../outside")
        with self.assertRaises(CHECK.ValidationError):
            CHECK.safe_artifact(ROOT, "build/absent-artifact")
        self.assertEqual(CHECK.safe_artifact(ROOT, "repository_state.json"), ROOT / "repository_state.json")

    def evidence(self):
        hash_value = "sha256:" + "a" * 64
        return {"evidence_id": "ev_test", "task_id": "P00", "phase": "bootstrap", "revision": 0,
                "git_commit": "b" * 40, "graph_hash": hash_value, "compiler_hash": None,
                "runtime_hash": None, "host_compiler_hash": hash_value,
                "unavailable_components": ["compiler", "runtime"],
                "target": CHECK.TARGET, "toolchain_lock_hash": hash_value,
                "commands": [{"id": "bootstrap_check", "argv_hash": hash_value, "exit_code": 0}],
                "artifacts": [{"path": "examples/bootstrap/graph.json", "sha256": hash_value}],
                "tests": [{"suite": "bootstrap", "passed": True, "count": 1}],
                "effects_delta": [], "capabilities_delta": [], "known_limits": ["bootstrap_only"]}

    def test_bootstrap_evidence_explicitly_excludes_unavailable_compiler(self):
        evidence = self.evidence()
        CHECK.validate_file(ROOT, evidence, "evidence")
        evidence["compiler_hash"] = "sha256:" + "c" * 64
        self.rejects(evidence, "evidence")

    def test_application_evidence_cannot_claim_null_compiler(self):
        evidence = self.evidence()
        evidence.update(task_id="P07", phase="application")
        self.rejects(evidence, "evidence")

    def test_evidence_requires_commands_tests_and_hashes(self):
        for field in ("commands", "tests", "graph_hash", "compiler_hash", "target"):
            evidence = self.evidence()
            del evidence[field]
            self.rejects(evidence, "evidence")
        for field in ("commands", "tests", "artifacts"):
            evidence = self.evidence()
            evidence[field] = []
            self.rejects(evidence, "evidence")

    def test_failed_dependencies_cannot_be_claimed_verified(self):
        # run_checks also ensures state classifications match every persisted task.
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "schema").mkdir()
            (root / "eval/tasks").mkdir(parents=True)
            for path in (ROOT / "schema").glob("*.json"):
                (root / "schema" / path.name).write_bytes(path.read_bytes())
            for path in (ROOT / "eval/tasks").glob("*.json"):
                task = CHECK.read_json(path)
                task["status"] = "BLOCKED"
                if task["task_id"] == "P01":
                    task["status"] = "VERIFIED"
                (root / "eval/tasks" / path.name).write_text(json.dumps(task), encoding="utf-8")
            state = self.state()
            (root / "repository_state.json").write_text(json.dumps(state), encoding="utf-8")
            result = CHECK.run_checks(root, pre_commit=True)
            dependency = next(item for item in result["checks"] if item["id"] == "task_dependencies_and_state")
            self.assertFalse(dependency["passed"])
            self.assertIn("dependencies not VERIFIED", dependency["error"])



class EvidenceReportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location("record_evidence", ROOT / "tools/record_evidence.py")
        cls.recorder = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.recorder)

    def report(self):
        return {"source_commit": "a" * 40, "status": "PASSED", "gates": [
            {"name": "contract", "exit_code": 0, "is_test": True, "test_count": 1}]}

    def test_only_matching_complete_report_can_be_recorded(self):
        report = self.report()
        self.assertEqual(self.recorder.validate_report(report, "a" * 40), report["gates"])
        with self.assertRaisesRegex(ValueError, "source commit"):
            self.recorder.validate_report(report, "b" * 40)

    def test_interrupted_running_or_empty_evidence_cannot_be_promoted(self):
        variants = []
        for status in ["RUNNING", "INTERRUPTED", "FAILED"]:
            report = self.report(); report["status"] = status; variants.append(report)
        report = self.report(); report["active_gate"] = {"name": "native"}; variants.append(report)
        report = self.report(); report["gates"][0]["exit_code"] = 1; variants.append(report)
        report = self.report(); report["gates"][0]["test_count"] = 0; variants.append(report)
        report = self.report(); report["gates"] = []; variants.append(report)
        for report in variants:
            with self.subTest(report=report), self.assertRaises(ValueError):
                self.recorder.validate_report(report, "a" * 40)

if __name__ == "__main__":
    unittest.main()
