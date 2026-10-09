"""Independent P03 text preview/import acceptance through the built il CLI.

Only the earlier test suite's Git fixture and subprocess helpers are reused;
no frontend, formatter, graph-store or compiler implementation is imported.
"""
from __future__ import annotations

import argparse
import copy
import json
from pathlib import Path
import sys
import unittest

import graph_cli


CHECKS = ["schema", "names", "types", "ownership", "effects", "capabilities", "contracts"]
SOURCE = '''@id("app") module app {
  @id("app.main") fn main() -> I32 effects [] capabilities [] {
    @id("app.main.entry") block entry {
      @id("app.main.zero") let zero: I32 = 0;
      @id("app.main.return") return zero;
    }
  }
}
'''


def import_request(source=SOURCE, base=0, scope=None):
    return {
        "task_id": "P03-text-blackbox", "base_revision": base,
        "scope": ["program"] if scope is None else scope,
        "operations": [{"op": "import_text", "source": source}],
        "required_checks": CHECKS,
    }


def entity_ids(value):
    """Collect declared IDs without knowing any frontend implementation detail."""
    if isinstance(value, list):
        return set().union(*(entity_ids(item) for item in value)) if value else set()
    if isinstance(value, dict):
        nested = set().union(*(entity_ids(item) for item in value.values())) if value else set()
        return nested | ({value["entity_id"]} if "entity_id" in value else set())
    return set()


class TextCliAcceptance(unittest.TestCase):
    # Deliberately reuse only test infrastructure, not P02 test cases themselves.
    setUp = graph_cli.GraphCliAcceptance.setUp
    git = graph_cli.GraphCliAcceptance.git
    invoke_raw = graph_cli.GraphCliAcceptance.invoke_raw
    invoke = graph_cli.GraphCliAcceptance.invoke
    assert_code = graph_cli.GraphCliAcceptance.assert_code
    head_revision = graph_cli.GraphCliAcceptance.head_revision

    def preview(self, source=SOURCE):
        response = self.invoke("schema-check", {"source": source})
        self.assertEqual(set(response["result"]), {"graph", "source", "run_id"})
        self.assertIsInstance(response["result"]["source"], str)
        return response["result"]

    def program(self, revision=None):
        request = {"entity_id": "program", "budget": 65536}
        if revision is not None:
            request["revision"] = revision
        return self.invoke("inspect", request)["result"]["entity"]

    def test_preview_preserves_annotations_and_does_not_publish(self):
        preview = self.preview()
        graph = preview["graph"]
        self.assertEqual(graph["revision"], 0)
        self.assertEqual(graph["modules"][0]["entity_id"], "app")
        self.assertEqual(graph["functions"][0]["entity_id"], "app.main")
        self.assertEqual(graph["functions"][0]["blocks"][0]["entity_id"], "app.main.entry")
        operation = graph["functions"][0]["blocks"][0]["operations"][0]
        self.assertEqual(operation["entity_id"], "app.main.zero")
        self.assertEqual(operation["outputs"], [{"entity_id": "app.main.zero.value", "type": "I32"}])
        self.assertEqual(operation["attributes"], {"value": 0})
        self.assertEqual(self.head_revision(), 0)
        self.assertFalse((self.store / ".il/HEAD").exists(), "preview must not initialize/publish an application store")

    def test_canonical_projection_roundtrips_exact_graph(self):
        first = self.preview()
        second = self.preview(first["source"])
        self.assertEqual(second["graph"], first["graph"])
        self.assertEqual(second["source"], first["source"])
        self.assertEqual(self.head_revision(), 0)

    def test_comments_and_whitespace_do_not_change_stable_identity(self):
        first = self.preview()
        decorated = "// independent formatting variant\n\n" + SOURCE.replace("    @id", "\t\t@id")
        second = self.preview(decorated)
        self.assertEqual(second["graph"], first["graph"])
        self.assertEqual(second["source"], first["source"])

    def test_text_import_publishes_the_checked_graph(self):
        expected = self.preview()["graph"]
        result = self.invoke("transact", import_request())
        self.assertEqual(result["result_revision"], 1)
        expected["revision"] = 1
        self.assertEqual(self.program(), expected)
        self.assertEqual(self.head_revision(), 1)

    def test_literal_edit_retains_entity_ids_and_old_revision(self):
        self.invoke("transact", import_request())
        before = self.program()
        edited = SOURCE.replace("= 0;", "= 7;")
        result = self.invoke("transact", import_request(edited, 1))
        self.assertEqual(result["result_revision"], 2)
        after = self.program()
        self.assertEqual(entity_ids(after), entity_ids(before))
        self.assertEqual(after["functions"][0]["blocks"][0]["operations"][0]["attributes"], {"value": 7})
        self.assertEqual(self.program(1), before)

    def test_stale_text_import_cannot_replace_current_program(self):
        self.invoke("transact", import_request())
        before = self.program()
        rejected = self.invoke("transact", import_request(SOURCE.replace("= 0;", "= 9;"), 0), False)
        self.assert_code(rejected, "E_STALE_REVISION")
        self.assertEqual(self.head_revision(), 1)
        self.assertEqual(self.program(), before)

    def test_module_scope_does_not_grant_whole_program_replacement(self):
        rejected = self.invoke("transact", import_request(scope=["app"]), False)
        self.assert_code(rejected, "E_INVALID_SCOPE")
        self.assertEqual(self.head_revision(), 0)

    def test_type_error_preserves_published_graph(self):
        self.invoke("transact", import_request())
        before = self.program()
        invalid = SOURCE.replace("-> I32", "-> Bool")
        rejected = self.invoke("transact", import_request(invalid, 1), False)
        self.assert_code(rejected, "E_TYPE_MISMATCH")
        self.assertEqual(self.head_revision(), 1)
        self.assertEqual(self.program(), before)

    def test_unknown_syntax_preserves_published_graph(self):
        self.invoke("transact", import_request())
        before = self.program()
        invalid = SOURCE.replace('      @id("app.main.return")', '      eval("forbidden");\n      @id("app.main.return")')
        rejected = self.invoke("transact", import_request(invalid, 1), False)
        self.assert_code(rejected, "E_UNSUPPORTED_FEATURE")
        self.assertEqual(self.head_revision(), 1)
        self.assertEqual(self.program(), before)

    def test_missing_semicolon_is_a_structured_error(self):
        rejected = self.invoke("schema-check", {"source": SOURCE.replace("= 0;", "= 0")}, False)
        self.assert_code(rejected, "E_SCHEMA_INVALID")
        self.assertEqual(self.head_revision(), 0)

    def test_failed_text_transaction_retains_exact_source_and_diagnostics(self):
        self.invoke("transact", import_request())
        request = import_request(SOURCE.replace("= 0;", "= 0"), 1)
        rejected = self.invoke("transact", request, False)
        self.assert_code(rejected, "E_SCHEMA_INVALID")
        self.assertEqual(self.head_revision(), 1)
        candidate = self.store / rejected["result"]["candidate"]
        self.assertEqual(json.loads((candidate / "request.json").read_text()), request)
        self.assertEqual(json.loads((candidate / "diagnostics.json").read_text()), rejected["diagnostics"])
        repeated = self.invoke("transact", request, False)
        self.assertEqual(repeated["result"]["candidate"], rejected["result"]["candidate"])

    def test_graph_and_text_entrypoints_share_type_diagnostic(self):
        graph = copy.deepcopy(self.preview()["graph"])
        graph["functions"][0]["result"] = "Bool"
        textual = self.invoke("schema-check", {"source": SOURCE.replace("-> I32", "-> Bool")}, False)
        structural = self.invoke("validate", {"graph_or_revision": graph, "checks": CHECKS}, False)
        self.assert_code(textual, "E_TYPE_MISMATCH")
        self.assertEqual({item["code"] for item in textual["diagnostics"]},
                         {item["code"] for item in structural["diagnostics"]})

    def test_preview_request_rejects_unknown_fields(self):
        rejected = self.invoke("schema-check", {"source": SOURCE, "shell": "forbidden"}, False)
        self.assert_code(rejected, "E_SCHEMA_INVALID")
        self.assertEqual(self.head_revision(), 0)

    def test_valid_capability_syntax_cannot_forge_host_authority(self):
        self.invoke("transact", import_request())
        before = self.program()
        forged = '@id("clock") capability clock: ClockRead scope null;\n' + SOURCE
        rejected = self.invoke("transact", import_request(forged, 1), False)
        self.assert_code(rejected, "E_CAPABILITY_MISSING")
        self.assertEqual(self.head_revision(), 1)
        self.assertEqual(self.program(), before)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    arguments = parser.parse_args()
    graph_cli.BINARY = arguments.binary.resolve()
    if not graph_cli.BINARY.is_file():
        parser.error(f"binary not found: {graph_cli.BINARY}")
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(TextCliAcceptance)
    result = unittest.TextTestRunner(verbosity=2, stream=sys.stderr).run(suite)
    report = {
        "suite": "text_cli_blackbox", "passed": result.wasSuccessful(), "count": result.testsRun,
        "failures": [{"test": str(test), "traceback": trace} for test, trace in result.failures],
        "errors": [{"test": str(test), "traceback": trace} for test, trace in result.errors],
    }
    graph_cli.write_json(arguments.report, report)
    print(json.dumps(report, ensure_ascii=False))
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    raise SystemExit(main())
