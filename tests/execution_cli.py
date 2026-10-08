"""Independent P04 numeric and execution protocol acceptance through il test."""
from __future__ import annotations
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import unittest
import graph_cli

CONTRACT = Path(__file__).with_name("execution_cli_contracts.json")
ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("execution_schema_check", ROOT / "tools/bootstrap_check.py")
SCHEMA_CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SCHEMA_CHECK)
LIMITS = {"max_steps": 100000, "max_call_depth": 128,
          "max_heap_bytes": 16777216, "max_output_bytes": 65536}


def operation(identity, opcode, inputs, outputs, attributes):
    return {"entity_id": identity, "opcode": opcode, "inputs": inputs,
            "outputs": outputs, "attributes": attributes, "effects": [],
            "consumes": [], "produces": []}


def numeric_graph(case):
    parameters = [{"entity_id": f"arg{i}", "name": f"arg{i}", "type": case["type"]}
                  for i in range(len(case["operands"]))]
    result_type = case.get("result_type", case["type"])
    attributes = {"target_type": result_type} if case["opcode"] == "cast" else {}
    graph = {"project_id": "il", "graph_version": "1.0.0", "revision": 0,
             "target": graph_cli.TARGET, "modules": [{"entity_id": "app", "path": "app",
             "imports": [], "declarations": ["main"], "visibility": "public"}],
             "types": [], "functions": [], "capabilities": [], "packages": [], "contracts": []}
    graph["functions"] = [{"entity_id": "main", "name": "main", "parameters": parameters,
        "result": result_type, "effects": [], "capabilities": [], "contracts": [],
        "blocks": [{"entity_id": "main.entry", "arguments": [],
            "operations": [operation("main.calculate", case["opcode"],
                [p["entity_id"] for p in parameters], [{"entity_id": "value", "type": result_type}], attributes)],
            "terminator": operation("main.return", "return", ["value"], [], {})}]}]
    return graph


def integer(type_ref, value):
    return {"type": type_ref, "data": {"kind": "integer", "value": str(value)}}


class ExecutionCliAcceptance(unittest.TestCase):
    setUp = graph_cli.GraphCliAcceptance.setUp
    git = graph_cli.GraphCliAcceptance.git
    invoke_raw = graph_cli.GraphCliAcceptance.invoke_raw
    invoke = graph_cli.GraphCliAcceptance.invoke
    assert_code = graph_cli.GraphCliAcceptance.assert_code
    head_revision = graph_cli.GraphCliAcceptance.head_revision
    records = []

    def publish(self, graph):
        return self.invoke("transact", {"task_id": "P04-execution", "base_revision": 0,
            "scope": ["program"], "operations": [{"op": "replace_program", "graph": graph}],
            "required_checks": []})

    def request_for(self, case):
        return {"revision": 1, "isolation": "captured", "suite": {"entry": "main",
            "arguments": [integer(case["type"], v) for v in case["operands"]], "limits": dict(LIMITS)}}

    def test_locked_numeric_execution_matrix(self):
        cases = json.loads(CONTRACT.read_text(encoding="utf-8"))["cases"]
        for index, case in enumerate(cases):
            with self.subTest(case=case["id"]):
                self.store = self.root / f"numeric-{index}"
                self.publish(numeric_graph(case))
                expected = case["expected"]
                request = self.request_for(case)
                success = expected["status"] == "returned"
                response = self.invoke("test", request, success)
                execution = response["result"]["execution"]
                SCHEMA_CHECK.validate_file(ROOT, execution, "execution")
                self.assertEqual(execution["status"], expected["status"])
                self.assertEqual(execution["stdout"], [])
                self.assertEqual(execution["stderr"], [])
                self.assertEqual(execution["live_allocations"], 0)
                self.assertEqual(response["result_revision"], 1)
                if success:
                    expected_type = case.get("result_type", case["type"])
                    expected_value = ({"type": "Bool", "data": {"kind": "bool", "value": expected["value"]}}
                                      if expected_type == "Bool" else integer(expected_type, expected["value"]))
                    self.assertEqual(execution["value"], expected_value)
                    self.assertEqual(execution["diagnostics"], [])
                else:
                    self.assert_code(response, expected["code"])
                    self.assertEqual(execution["diagnostics"][0]["entity_id"], "main.calculate")
                repeated = self.invoke("test", request, success)
                self.assertEqual(repeated, response, "execution or evidence was nondeterministic")
                stages = response["result"]["stages"]
                self.assertEqual([s["stage"] for s in stages],
                                 ["lower_to_hir", "lower_to_mir", "verify_mir", "execute_mir"])
                for position, stage in enumerate(stages):
                    self.assertRegex(stage["output_hash"], r"^sha256:[a-f0-9]{64}$")
                    if 0 < position < 3:
                        self.assertEqual(stage["input_hash"], stages[position - 1]["output_hash"])
                self.assertEqual(response["result"]["execution_input"]["mir_hash"], stages[2]["output_hash"])
                execution_input_bytes = (json.dumps(response["result"]["execution_input"],
                    ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode("utf-8")
                self.assertEqual(stages[3]["input_hash"], "sha256:" + hashlib.sha256(execution_input_bytes).hexdigest())
                self.records.append({"id": case["id"], "passed": True,
                    "execution_hash": stages[-1]["output_hash"]})

    def test_test_request_is_closed_and_does_not_publish(self):
        case = {"type": "I64", "opcode": "add", "operands": [20, 22]}
        self.publish(numeric_graph(case))
        request = self.request_for(case)
        for changed in [dict(request, shell="forbidden"), dict(request, isolation="host")]:
            self.assert_code(self.invoke("test", changed, False), "E_SCHEMA_INVALID")
        self.assertEqual(self.head_revision(), 1)

    def test_preexisting_graph_semantics_execute_through_mir(self):
        string = {"type": "String", "data": {"kind": "string", "value": "你好 il"}}
        unit = {"type": "Unit", "data": {"kind": "unit"}}
        variant = lambda ty, tag, fields: {"type": ty, "data": {"kind": "variant", "value": {"tag": tag, "fields": fields}}}
        cases = [
            ("valid_integer_return", [], integer("I64", 42)),
            ("valid_typed_call", [], integer("I64", 42)),
            ("valid_record_and_field", [], integer("I64", 20)),
            ("valid_conditional_block_arguments", [], integer("I64", 20)),
            ("valid_option_constructor", [], variant("Wrapped", "Some", [integer("I64", 42)])),
            ("valid_result_constructor", [], variant("Wrapped", "Ok", [integer("I64", 42)])),
            ("valid_exhaustive_sum_switch", [variant("Choice", "Left", [])], integer("I64", 1)),
            ("valid_exhaustive_sum_switch", [variant("Choice", "Right", [])], integer("I64", 2)),
            ("valid_owned_move_return", [string], string),
            ("valid_lexical_borrow", [string], string),
            ("valid_declared_callee_effect", [], unit),
        ]
        for index, (name, arguments, expected) in enumerate(cases):
            with self.subTest(fixture=name, arguments=arguments):
                self.store = self.root / f"semantic-{index}"
                graph = json.loads((ROOT / "tests/fixtures/semantics" / (name + ".json")).read_text())
                self.publish(graph)
                request = {"revision": 1, "isolation": "captured", "suite":
                    {"entry": "main", "arguments": arguments, "limits": dict(LIMITS)}}
                response = self.invoke("test", request)
                execution = response["result"]["execution"]
                SCHEMA_CHECK.validate_file(ROOT, execution, "execution")
                self.assertEqual(execution["value"], expected)
                self.assertEqual(execution["stdout"], [])
                self.assertEqual(execution["stderr"], [])
                self.assertEqual(execution["live_allocations"], 1 if expected["type"] == "String" else 0)
                self.assertEqual(self.invoke("test", request), response)

    def test_argument_type_and_step_budget_are_checked(self):
        case = {"type": "I64", "opcode": "add", "operands": [20, 22]}
        self.publish(numeric_graph(case))
        wrong = self.request_for(case)
        wrong["suite"]["arguments"][0] = integer("U64", 20)
        response = self.invoke("test", wrong, False)
        self.assert_code(response, "E_TYPE_MISMATCH")
        execution = response["result"]["execution"]
        self.assertEqual(execution["status"], "rejected")
        self.assertEqual(execution["steps"], 0)
        self.assertEqual(execution["lifecycle"], [])
        limited = self.request_for(case)
        limited["suite"]["limits"]["max_steps"] = 1
        self.assert_code(self.invoke("test", limited, False), "E_RESOURCE_LIMIT")
        self.assertEqual(self.head_revision(), 1)

    def test_recursive_call_and_loop_budgets_terminate(self):
        case = {"type": "I64", "opcode": "add", "operands": [20, 22]}
        graph = numeric_graph(case)
        call = graph["functions"][0]["blocks"][0]["operations"][0]
        call["opcode"] = "call"
        call["attributes"] = {"callee": "main"}
        self.publish(graph)
        request = self.request_for(case)
        request["suite"]["limits"]["max_call_depth"] = 4
        self.assert_code(self.invoke("test", request, False), "E_RESOURCE_LIMIT")
        self.store = self.root / "loop"
        graph = numeric_graph(case)
        block = graph["functions"][0]["blocks"][0]
        block["operations"] = [operation("main.condition", "const", [],
            [{"entity_id": "condition", "type": "Bool"}], {"value": True})]
        block["terminator"] = operation("main.loop", "cond_branch", ["condition"], [],
            {"then_block": "main.entry", "else_block": "main.done", "then_arguments": [], "else_arguments": []})
        graph["functions"][0]["blocks"].append({"entity_id": "main.done", "arguments": [], "operations": [],
            "terminator": operation("main.done.return", "return", ["arg0"], [], {})})
        self.publish(graph)
        request = self.request_for(case)
        request["suite"]["limits"]["max_steps"] = 20
        self.assert_code(self.invoke("test", request, False), "E_RESOURCE_LIMIT")

    def test_integer_input_strings_are_canonical(self):
        case = {"type": "I64", "opcode": "add", "operands": [20, 22]}
        self.publish(numeric_graph(case))
        for invalid in ["+1", "01", "-0", " 1", "1e2"]:
            with self.subTest(integer=invalid):
                request = self.request_for(case)
                request["suite"]["arguments"][0]["data"]["value"] = invalid
                self.assert_code(self.invoke("test", request, False), "E_TYPE_MISMATCH")

    def test_structured_text_executes_after_transaction(self):
        cases = [
            ('''module app {
              @id("main") fn main() -> I64 {
                let mut total: I64 = 0;
                let mut i: I64 = 0;
                while i < 10 {
                  i = i + 1;
                  if i == 3 { continue; }
                  if i == 8 { break; }
                  total = total + i;
                }
                return total;
              }
            }''', "I64", 25),
            ('''module app {
              @id("main") fn main() -> I8 { return twice(7); }
              @id("twice") fn twice(x: I8) -> I8 { return x + x; }
            }''', "I8", 14),
            ('''module app {
              @id("Maybe") type Maybe = Option<String>;
              @id("main") fn main() -> I64 effects [alloc] {
                let value: Maybe = Some("abc");
                match value {
                  Some(text) => { let n: Usize = runtime.string_len(text); return cast<I64>(n); }
                  None => { return 0; }
                }
              }
            }''', "I64", 3),
            ('''module app {
              @id("Out") type Out = Result<I64, I64>;
              fn fail() -> Out { return Err(9); }
              @id("main") fn main() -> Out effects [alloc] {
                let owner: String = "alive";
                let n: I64 = fail()?;
                return Ok(n);
              }
            }''', "Out", {"type": "Out", "data": {"kind": "variant", "value":
                {"tag": "Err", "fields": [integer("I64", 9)]}}}),
            ('''module app {
              @id("Out") type Out = Result<I64, core.AllocError>;
              @id("main") fn main() -> Out effects [alloc] {
                let name: String = "World";
                let text: String = f"Hello ${name}"?;
                let n: Usize = runtime.string_len(text);
                return Ok(cast<I64>(n));
              }
            }''', "Out", {"type": "Out", "data": {"kind": "variant", "value":
                {"tag": "Ok", "fields": [integer("I64", 11)]}}}),
        ]
        for index, (source, result_type, expected) in enumerate(cases):
            with self.subTest(source=index):
                self.store = self.root / f"surface-{index}"
                self.invoke("transact", {"task_id": "P04-surface", "base_revision": 0,
                    "scope": ["program"], "operations": [{"op": "import_text", "source": source}],
                    "required_checks": []})
                response = self.invoke("test", {"revision": 1, "isolation": "captured", "suite":
                    {"entry": "main", "arguments": [], "limits": dict(LIMITS)}})
                self.assertEqual(response["result"]["execution"]["value"],
                    expected if isinstance(expected, dict) else integer(result_type, expected))
                self.assertEqual(response["result"]["execution"]["live_allocations"], 0)
                graph = self.invoke("inspect", {"entity_id": "program", "budget": 65536})["result"]["entity"]
                self.assertEqual(graph["revision"], 1)

    def test_runtime_output_is_utf8_captured_without_newlines(self):
        source = '''module app {
          @id("main") fn main() -> Unit effects [alloc, process] {
            runtime.print_i64(-42);
            runtime.print_string("\\n你好");
            return;
          }
        }'''
        self.invoke("transact", {"task_id": "P04-output", "base_revision": 0,
            "scope": ["program"], "operations": [{"op": "import_text", "source": source}],
            "required_checks": []})
        response = self.invoke("test", {"revision": 1, "isolation": "captured", "suite":
            {"entry": "main", "arguments": [], "limits": dict(LIMITS)}})
        execution = response["result"]["execution"]
        self.assertEqual(bytes(execution["stdout"]), "-42\n你好".encode("utf-8"))
        self.assertEqual(execution["stderr"], [])
        self.assertEqual(execution["live_allocations"], 0)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    graph_cli.BINARY = args.binary.resolve()
    if not graph_cli.BINARY.is_file():
        parser.error("binary does not exist")
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(ExecutionCliAcceptance)
    result = unittest.TextTestRunner(verbosity=2, stream=sys.stderr).run(suite)
    report = {"suite": "execution_cli_blackbox", "passed": result.wasSuccessful(),
        "count": result.testsRun, "numeric_scenarios": len(ExecutionCliAcceptance.records),
        "contract_sha256": "sha256:" + hashlib.sha256(CONTRACT.read_bytes()).hexdigest(),
        "cases": ExecutionCliAcceptance.records,
        "failures": [{"test": str(t), "traceback": e} for t, e in result.failures],
        "errors": [{"test": str(t), "traceback": e} for t, e in result.errors]}
    graph_cli.write_json(args.report, report)
    print(json.dumps({k:v for k,v in report.items() if k != "cases"}, ensure_ascii=False))
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    raise SystemExit(main())
