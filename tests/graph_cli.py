"""Independent P02 acceptance through the real il CLI and filesystem boundary.

Usage: python tests/graph_cli.py --binary /absolute/path/to/il [--report FILE]
No compiler or graph-store implementation is imported. Each test gets a fresh
Git source repository and a separate application-store directory.
"""
from __future__ import annotations

import argparse
import concurrent.futures
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


TARGET = "x86_64-unknown-linux-gnu"
BINARY: Path
ENVELOPE = {
    "ok", "tool", "tool_version", "base_revision", "result_revision",
    "diagnostics", "artifacts", "evidence_id", "result",
}
DIAGNOSTIC = {
    "diagnostic_id", "code", "stage", "severity", "entity_id",
    "related_entities", "expected", "actual", "cause",
    "suggested_operations", "retryable", "base_revision",
}


def write_json(path: Path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def module(entity_id="app", path="app", imports=None):
    return {
        "entity_id": entity_id,
        "path": path,
        "imports": [] if imports is None else imports,
        "declarations": [],
        "visibility": "private",
    }


def transaction(base=0, entity_id="app", path="app"):
    return {
        "task_id": "P02-blackbox",
        "base_revision": base,
        "scope": [entity_id],
        "operations": [{"op": "add_module", "module": module(entity_id, path)}],
        "required_checks": ["schema", "names"],
    }


class GraphCliAcceptance(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="il-graph-blackbox-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.repository = self.root / "source"
        self.repository.mkdir()
        self.store = self.root / "application"
        self.store.mkdir()
        self.git("init", "--initial-branch=main")
        self.state = {
            "project_id": "il", "schema_version": "1.0.0", "head_revision": 0,
            "last_verified_revision": 0, "compiler_version": None,
            "runtime_version": None, "target": TARGET,
            "open_tasks": ["P02"], "blocked_tasks": [],
            "failed_tasks": [], "locks": {},
        }
        self.graph = {
            "project_id": "il", "graph_version": "1.0.0", "revision": 0,
            "target": TARGET, "modules": [], "types": [], "functions": [],
            "capabilities": [], "packages": [], "contracts": [],
        }
        write_json(self.repository / "repository_state.json", self.state)
        write_json(self.repository / "examples/bootstrap/graph.json", self.graph)
        write_json(self.repository / "toolchain.lock", {
            "target": TARGET, "rust": "1.90.0", "llvm": "14.0.6",
        })
        self.git("add", "repository_state.json", "examples/bootstrap/graph.json", "toolchain.lock")
        self.git("-c", "user.name=il test", "-c", "user.email=test@invalid.local",
                 "-c", "commit.gpgsign=false", "-c", "core.hooksPath=" + str(self.root / "no-hooks"),
                 "commit", "-m", "test: initialize independent graph fixture")
        self.bindings = self.repository / ".git/il/revision_bindings.json"
        write_json(self.bindings, {
            "schema_version": "1.0.0",
            "bindings": [{
                "revision": 0,
                "git_commit": self.git("rev-parse", "HEAD"),
                "tree_hash": self.git("rev-parse", "HEAD^{tree}"),
            }],
        })

    def git(self, *arguments):
        completed = subprocess.run(
            ["git", "-C", str(self.repository), *arguments],
            capture_output=True, text=True, encoding="utf-8", timeout=20,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        return completed.stdout.strip()

    def commit_source_state(self, state):
        """Make a state fixture genuinely committed and bound, even when invalid."""
        write_json(self.repository / "repository_state.json", state)
        self.git("add", "repository_state.json")
        self.git("-c", "user.name=il test", "-c", "user.email=test@invalid.local",
                 "-c", "commit.gpgsign=false", "-c", "core.hooksPath=" + str(self.root / "no-hooks"),
                 "commit", "-m", "test: commit source state validation fixture")
        journal = json.loads(self.bindings.read_text(encoding="utf-8"))
        journal["bindings"].append({
            "revision": state["head_revision"],
            "git_commit": self.git("rev-parse", "HEAD"),
            "tree_hash": self.git("rev-parse", "HEAD^{tree}"),
        })
        write_json(self.bindings, journal)

    def invoke_raw(self, command, request_text, expect_ok=True):
        completed = subprocess.run(
            [str(BINARY), "--repository", str(self.repository),
             "--store", str(self.store), command],
            input=request_text, capture_output=True, text=True,
            encoding="utf-8", timeout=30,
        )
        try:
            response = json.loads(completed.stdout)
        except ValueError as error:
            self.fail(f"CLI emitted non-JSON stdout: {completed.stdout!r}; stderr={completed.stderr!r}: {error}")
        self.assertEqual(set(response), ENVELOPE)
        self.assertEqual(response["tool"], command)
        self.assertIs(response["ok"], expect_ok, response)
        self.assertEqual(completed.returncode == 0, expect_ok, response)
        self.assertIsInstance(response["tool_version"], str)
        self.assertIsInstance(response["diagnostics"], list)
        if expect_ok:
            self.assertRegex(response["result"]["run_id"], r"^run_[0-9a-f]{64}$")
        for diagnostic in response["diagnostics"]:
            self.assertEqual(set(diagnostic), DIAGNOSTIC)
        if not expect_ok:
            self.assertTrue(response["diagnostics"], "failure must provide a structured diagnostic")
        return response

    def invoke(self, command, arguments, expect_ok=True):
        return self.invoke_raw(command, json.dumps(arguments), expect_ok)

    def assert_code(self, response, code):
        self.assertIn(code, [item["code"] for item in response["diagnostics"]], response)

    def head_revision(self):
        return self.invoke("state", {})["result_revision"]

    def test_transaction_inspect_and_restore_create_monotonic_revisions(self):
        self.assertEqual(self.head_revision(), 0)
        added = self.invoke("transact", transaction())
        self.assertEqual(added["base_revision"], 0)
        self.assertEqual(added["result_revision"], 1)
        inspected = self.invoke("inspect", {"entity_id": "app", "revision": 1})
        self.assertEqual(inspected["result"]["entity"], module())
        delta = self.invoke("diff", {"base_revision": 0, "target_revision": 1, "scope": ["app"]})
        self.assertEqual(delta["result"], {
            "added": ["app"], "removed": [], "modified": [],
            "base_revision": 0, "target_revision": 1,
            "run_id": delta["result"]["run_id"],
        })
        restored = self.invoke("restore", {"revision": 0, "reason": "Independent restore acceptance"})
        self.assertEqual(restored["result_revision"], 2)
        self.assertEqual(self.head_revision(), 2)
        # History remains inspectable, while the restored current graph is empty.
        self.invoke("inspect", {"entity_id": "app", "revision": 1})
        missing = self.invoke("inspect", {"entity_id": "app", "revision": 2}, False)
        self.assert_code(missing, "E_NAME_NOT_FOUND")

    def test_stale_transaction_preserves_head(self):
        self.invoke("transact", transaction())
        rejected = self.invoke("transact", transaction(entity_id="other", path="other"), False)
        self.assert_code(rejected, "E_STALE_REVISION")
        self.assertEqual(self.head_revision(), 1)

    def test_validate_and_slice_use_structural_contract(self):
        self.invoke("transact", transaction())
        checked = self.invoke("validate", {
            "graph_or_revision": 1, "checks": ["schema", "names", "references"],
        })
        self.assertTrue(checked["result"]["valid"])
        selected = self.invoke("slice", {
            "root_entities": ["app"], "revision": 1, "max_nodes": 1, "max_tokens": 4096,
        })
        self.assertEqual(selected["result"], {
            "entities": [module()], "revision": 1, "node_count": 1,
            "run_id": selected["result"]["run_id"],
        })

    def test_context_budget_failure_is_explicit(self):
        self.invoke("transact", transaction())
        rejected = self.invoke("inspect", {
            "entity_id": "app", "revision": 1, "budget": 1,
        }, False)
        self.assert_code(rejected, "E_CONTEXT_INSUFFICIENT")
        self.assertEqual(self.head_revision(), 1)

    def test_scope_cannot_modify_unlisted_id(self):
        request = transaction()
        request["scope"] = ["different"]
        rejected = self.invoke("transact", request, False)
        self.assert_code(rejected, "E_INVALID_SCOPE")
        self.assertEqual(self.head_revision(), 0)

    def test_dangling_import_rejects_whole_transaction(self):
        request = transaction()
        request["operations"][0]["module"]["imports"] = ["absent"]
        self.invoke("transact", request, False)
        self.assertEqual(self.head_revision(), 0)

    def test_duplicate_id_rejects_whole_transaction(self):
        request = transaction()
        request["operations"].append({"op": "add_module", "module": module("app", "second")})
        self.invoke("transact", request, False)
        self.assertEqual(self.head_revision(), 0)

    def test_duplicate_module_name_rejected(self):
        self.invoke("transact", transaction())
        rejected = self.invoke("transact", transaction(1, "other", "app"), False)
        self.assert_code(rejected, "E_DUPLICATE_NAME")
        self.assertEqual(self.head_revision(), 1)

    def test_unknown_transaction_field_rejected(self):
        request = transaction()
        request["shell"] = "this field must never be executed"
        rejected = self.invoke("transact", request, False)
        self.assert_code(rejected, "E_SCHEMA_INVALID")
        self.assertEqual(self.head_revision(), 0)

    def test_duplicate_json_keys_are_rejected_before_mutation(self):
        request = json.dumps(transaction())
        request = request.replace('"base_revision": 0', '"base_revision": 99, "base_revision": 0')
        rejected = self.invoke_raw("transact", request, False)
        self.assert_code(rejected, "E_SCHEMA_INVALID")
        self.assertEqual(self.head_revision(), 0)
        nested = json.dumps({"graph_or_revision": self.graph, "checks": ["schema"]})
        nested = nested.replace('"project_id": "il"', '"project_id": "wrong", "project_id": "il"')
        rejected = self.invoke_raw("validate", nested, False)
        self.assert_code(rejected, "E_SCHEMA_INVALID")

    def test_unknown_operation_rejected(self):
        request = transaction()
        request["operations"] = [{"op": "execute_arbitrary_process", "argv": ["forbidden"]}]
        rejected = self.invoke("transact", request, False)
        self.assert_code(rejected, "E_SCHEMA_INVALID")
        self.assertEqual(self.head_revision(), 0)

    def test_full_semantic_checks_are_available_and_unknown_checks_rejected(self):
        request = transaction()
        request["required_checks"] = ["schema", "names", "types", "ownership", "effects", "capabilities", "contracts"]
        self.invoke("transact", request)
        request = transaction(1, "second")
        request["required_checks"] = ["schema", "disable_types"]
        rejected = self.invoke("transact", request, False)
        self.assert_code(rejected, "E_SCHEMA_INVALID")
        self.assertEqual(self.head_revision(), 1)

    def test_empty_checks_cannot_bypass_reference_validation(self):
        request = transaction()
        request["required_checks"] = []
        request["operations"][0]["module"]["imports"] = ["absent"]
        self.invoke("transact", request, False)
        self.assertEqual(self.head_revision(), 0)

    def test_removing_imported_module_does_not_silently_cascade(self):
        first = transaction(entity_id="dependency", path="dependency")
        first["operations"][0]["module"]["visibility"] = "public"
        self.invoke("transact", first)
        second = transaction(1)
        second["operations"][0]["module"]["imports"] = ["dependency"]
        self.invoke("transact", second)
        removal = transaction(2, "dependency", "dependency")
        removal["operations"] = [{"op": "remove_module", "entity_id": "dependency"}]
        self.invoke("transact", removal, False)
        self.assertEqual(self.head_revision(), 2)
        self.invoke("inspect", {"entity_id": "dependency", "revision": 2})
        self.invoke("inspect", {"entity_id": "app", "revision": 2})

    def test_repository_state_mismatch_blocks_dispatch(self):
        changed = dict(self.state, head_revision=1)
        write_json(self.repository / "repository_state.json", changed)
        rejected = self.invoke("state", {}, False)
        self.assert_code(rejected, "E_STATE_INCONSISTENT")

    def test_committed_state_requires_nullable_version_fields(self):
        for field in ("compiler_version", "runtime_version"):
            with self.subTest(missing=field):
                changed = dict(self.state)
                del changed[field]
                self.commit_source_state(changed)
                self.assert_code(self.invoke("state", {}, False), "E_STATE_INCONSISTENT")

    def test_committed_state_rejects_empty_versions(self):
        for field in ("compiler_version", "runtime_version"):
            with self.subTest(empty=field):
                self.commit_source_state(dict(self.state, **{field: ""}))
                self.assert_code(self.invoke("state", {}, False), "E_STATE_INCONSISTENT")

    def test_committed_state_enforces_closed_lock_schema(self):
        for label, lock in (
            ("unknown_field", {"owner": "task", "revision": 0, "shell": "forbidden"}),
            ("missing_owner", {"revision": 0}),
            ("empty_owner", {"owner": "", "revision": 0}),
        ):
            with self.subTest(lock=label):
                self.commit_source_state(dict(self.state, locks={"graph": lock}))
                self.assert_code(self.invoke("state", {}, False), "E_STATE_INCONSISTENT")

    def test_committed_state_rejects_invalid_or_repeated_tasks(self):
        for label, changed in (
            ("unknown_task", dict(self.state, open_tasks=["P99"])),
            ("repeated_task", dict(self.state, open_tasks=["P02", "P02"])),
            ("cross_classified", dict(self.state, blocked_tasks=["P02"])),
        ):
            with self.subTest(tasks=label):
                self.commit_source_state(changed)
                self.assert_code(self.invoke("state", {}, False), "E_STATE_INCONSISTENT")

    def test_committed_state_rejects_unknown_identity_target_or_field(self):
        for label, changed in (
            ("project", dict(self.state, project_id="other")),
            ("target", dict(self.state, target="unsupported-target")),
            ("unknown_field", dict(self.state, unrecognized=True)),
        ):
            with self.subTest(state=label):
                self.commit_source_state(changed)
                self.assert_code(self.invoke("state", {}, False), "E_STATE_INCONSISTENT")

    def test_repository_graph_mismatch_blocks_dispatch(self):
        changed = dict(self.graph, modules=[module()])
        write_json(self.repository / "examples/bootstrap/graph.json", changed)
        rejected = self.invoke("transact", transaction(), False)
        self.assert_code(rejected, "E_STATE_INCONSISTENT")

    def test_missing_git_binding_blocks_dispatch(self):
        self.bindings.unlink()
        rejected = self.invoke("state", {}, False)
        self.assert_code(rejected, "E_STATE_INCONSISTENT")

    def test_wrong_git_tree_binding_blocks_dispatch(self):
        record = json.loads(self.bindings.read_text(encoding="utf-8"))
        record["bindings"][0]["tree_hash"] = "0" * 40
        write_json(self.bindings, record)
        rejected = self.invoke("state", {}, False)
        self.assert_code(rejected, "E_STATE_INCONSISTENT")

    def test_legitimate_source_commit_preserves_store_and_updates_new_provenance(self):
        self.invoke("transact", transaction())
        old_manifest_path = self.store / ".il/revisions/00000000000000000001/manifest.json"
        original_manifest_bytes = old_manifest_path.read_bytes()
        original_manifest = json.loads(original_manifest_bytes)
        (self.repository / "compiler-note.txt").write_text("New verified compiler source context\n", encoding="utf-8")
        self.git("add", "compiler-note.txt")
        self.git("-c", "user.name=il test", "-c", "user.email=test@invalid.local",
                 "-c", "commit.gpgsign=false", "-c", "core.hooksPath=" + str(self.root / "no-hooks"),
                 "commit", "-m", "test: advance compiler source context")
        current_commit = self.git("rev-parse", "HEAD")
        current_tree = self.git("rev-parse", "HEAD^{tree}")
        journal = json.loads(self.bindings.read_text(encoding="utf-8"))
        journal["bindings"].append({"revision": 0, "git_commit": current_commit, "tree_hash": current_tree})
        write_json(self.bindings, journal)
        self.assertEqual(self.head_revision(), 1)
        self.invoke("inspect", {"entity_id": "app", "revision": 1})
        accepted = self.invoke("transact", transaction(1, "second", "second"))
        self.assertEqual(accepted["result_revision"], 2)
        new_manifest = json.loads((self.store / ".il/revisions/00000000000000000002/manifest.json").read_bytes())
        self.assertEqual(new_manifest["provenance"], {
            "source_git_commit": current_commit, "source_tree_hash": current_tree,
        })
        self.assertNotEqual(original_manifest["provenance"]["source_git_commit"], current_commit)
        self.assertEqual(old_manifest_path.read_bytes(), original_manifest_bytes)

    def test_concurrent_first_writers_initialize_once(self):
        def worker(index):
            completed = subprocess.run(
                [str(BINARY), "--repository", str(self.repository),
                 "--store", str(self.store), "transact"],
                input=json.dumps(transaction(0, f"first_{index}", f"first_{index}")),
                capture_output=True, text=True, encoding="utf-8", timeout=30,
            )
            return completed.returncode, json.loads(completed.stdout)

        with concurrent.futures.ThreadPoolExecutor(max_workers=6) as executor:
            responses = list(executor.map(worker, range(6)))
        successes = [(code, response) for code, response in responses if response["ok"]]
        self.assertEqual(len(successes), 1, responses)
        self.assertEqual(successes[0][0], 0)
        self.assertEqual(successes[0][1]["result_revision"], 1)
        for code, response in responses:
            if not response["ok"]:
                self.assertNotEqual(code, 0)
                self.assert_code(response, "E_STALE_REVISION")
        self.assertEqual(self.head_revision(), 1)

    def test_concurrent_same_base_has_one_published_success(self):
        # Initialize before concurrency so this checks publication, not setup races.
        self.invoke("transact", transaction())

        def worker(index):
            completed = subprocess.run(
                [str(BINARY), "--repository", str(self.repository),
                 "--store", str(self.store), "transact"],
                input=json.dumps(transaction(1, f"module_{index}", f"module_{index}")),
                capture_output=True, text=True, encoding="utf-8", timeout=30,
            )
            return completed.returncode, json.loads(completed.stdout)

        with concurrent.futures.ThreadPoolExecutor(max_workers=6) as executor:
            responses = list(executor.map(worker, range(6)))
        successes = [(code, response) for code, response in responses if response["ok"]]
        self.assertEqual(len(successes), 1, responses)
        self.assertEqual(successes[0][0], 0)
        self.assertEqual(successes[0][1]["result_revision"], 2)
        for code, response in responses:
            if not response["ok"]:
                self.assertNotEqual(code, 0)
                self.assert_code(response, "E_STALE_REVISION")
        self.assertEqual(self.head_revision(), 2)

    def test_corrupt_published_graph_is_rejected(self):
        self.invoke("transact", transaction())
        snapshot = self.store / ".il/revisions/00000000000000000001/graph.json"
        self.assertTrue(snapshot.is_file(), "published snapshot path is part of RFC 0004")
        changed = json.loads(snapshot.read_text(encoding="utf-8"))
        changed["modules"][0]["path"] = "tampered"
        write_json(snapshot, changed)
        rejected = self.invoke("state", {}, False)
        self.assert_code(rejected, "E_STATE_INCONSISTENT")

    def test_corrupt_head_is_not_recovered_by_guessing_latest_directory(self):
        self.invoke("transact", transaction())
        pointer = self.store / ".il/HEAD"
        self.assertTrue(pointer.is_file(), "publication path is part of RFC 0004")
        pointer.write_text("{broken", encoding="utf-8")
        rejected = self.invoke("state", {}, False)
        self.assert_code(rejected, "E_STATE_INCONSISTENT")


def main():
    global BINARY
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--report", type=Path)
    arguments = parser.parse_args()
    BINARY = arguments.binary.resolve()
    if not BINARY.is_file():
        parser.error(f"binary not found: {BINARY}")
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(GraphCliAcceptance)
    result = unittest.TextTestRunner(verbosity=2, stream=sys.stderr).run(suite)
    report = {
        "suite": "graph_cli_blackbox", "passed": result.wasSuccessful(),
        "count": result.testsRun,
        "failures": [{"test": str(test), "traceback": trace} for test, trace in result.failures],
        "errors": [{"test": str(test), "traceback": trace} for test, trace in result.errors],
    }
    if arguments.report:
        write_json(arguments.report, report)
    print(json.dumps(report, ensure_ascii=False))
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    raise SystemExit(main())
