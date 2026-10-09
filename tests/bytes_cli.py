"""P07 byte failure contracts through interpreter and both LLVM profiles."""
from __future__ import annotations
import argparse
import json
import sys
import unittest
from pathlib import Path
import execution_cli
import graph_cli
import runtime_cli

ROOT = Path(__file__).resolve().parents[1]
CONTRACT = Path(__file__).with_name("bytes_cli_contracts.json")
LOCKED = json.loads(CONTRACT.read_text(encoding="utf-8"))
REPORT = None


def source(case):
    scalar = case["result_type"] == "U8"
    declarations = "" if scalar else "@id(\"Out\") type Out=Result<" + case["result_type"] + ",core.IoError>;"
    return (runtime_cli.CORE_DECLARATIONS + '@id("app") module app imports [core] {'
            + declarations + '@id("main") fn main(data:' + case["input_type"] + ')->'
            + ("U8" if scalar else "Out") + ' effects [alloc] {return ' + case["expression"] + ';}}')


class BytesCliAcceptance(unittest.TestCase):
    setUp = runtime_cli.RuntimeCliAcceptance.setUp
    git = graph_cli.GraphCliAcceptance.git
    head_revision = graph_cli.GraphCliAcceptance.head_revision
    invoke = runtime_cli.RuntimeCliAcceptance.invoke
    execute_request = runtime_cli.RuntimeCliAcceptance.execute_request
    scenario = runtime_cli.RuntimeCliAcceptance.scenario
    validate_build = runtime_cli.RuntimeCliAcceptance.validate_build
    records = []

    def progress(self):
        if REPORT is not None:
            runtime_cli.write_json(REPORT, {"suite":"bytes_cli_parity", "state":"RUNNING", "passed":False,
                "contract_sha256":runtime_cli.sha256(CONTRACT), "cases":self.records})

    def check(self, execution, expected):
        execution_cli.SCHEMA_CHECK.validate_file(ROOT, execution, "execution")
        self.assertEqual(execution["status"], expected["status"], execution)
        self.assertEqual(execution["live_allocations"], expected["live_allocations"], execution)
        self.assertEqual(execution["live_handles"], 0)
        for field in ["stdout", "stderr", "handle_events"]:
            self.assertEqual(execution[field], [])
        if expected["status"] == "trapped":
            self.assertEqual([item["code"] for item in execution["diagnostics"]], [expected["code"]])
            self.assertIsNone(execution["value"])
            self.assertTrue(execution["stack_trace"])
        else:
            self.assertEqual(execution["diagnostics"], [])
            self.assertEqual(execution["stack_trace"], [])
            value = execution["value"]["data"]["value"]
            self.assertEqual(value["tag"], "Err" if "error" in expected else "Ok")
            payload = value["fields"][0]["data"]
            if "error" in expected:
                self.assertEqual(payload, {"kind":"variant", "value":{"tag":expected["error"], "fields":[]}})
            else:
                kind = "bytes" if "bytes" in expected else "string"
                self.assertEqual(payload, {"kind":kind, "value":expected[kind]})

    def test_locked_byte_boundaries_and_failure_ordering(self):
        for case in LOCKED["cases"]:
            with self.subTest(case=case["id"]), self.scenario(case["id"]) as row:
                faults = None if "fail_after" not in case else {
                    "allocation_fail_after":case["fail_after"], "io_max_chunk":None, "io_fail_after":None}
                self.policy_path = runtime_cli.write_json(self.root/(case["id"]+"-policy.json"),
                    {"schema_version":"1.0.0", "grants":[], "test_faults":faults})
                self.invoke("transact", {"task_id":"P07-bytes", "base_revision":0, "scope":["program"],
                    "operations":[{"op":"import_text", "source":source(case)}], "required_checks":[]})
                arguments = [{"type":case["input_type"], "data":{
                    "kind":case["input_type"].lower(), "value":case["input"]}}]
                row["executions"] = {}
                reference = None
                for mode in LOCKED["modes"]:
                    reply = self.invoke("test", self.execute_request(arguments, case.get("limits"), mode),
                        case["expected"]["status"] == "returned")
                    execution = reply["result"]["execution"]
                    row["executions"][mode] = {"execution":execution}
                    if mode != "captured":
                        row["executions"][mode]["native"] = self.validate_build(reply, "full", mode.removeprefix("native_"))
                    self.progress()
                    self.check(execution, case["expected"])
                    observable = {key:execution[key] for key in ["status", "value", "stdout", "stderr",
                        "live_allocations", "peak_heap_bytes", "live_handles", "lifecycle", "handle_events", "stack_trace"]}
                    observable["diagnostics"] = [{"code":item["code"], "entity_id":item["entity_id"]}
                        for item in execution["diagnostics"]]
                    if reference is None:
                        reference = observable
                    else:
                        self.assertEqual(observable, reference)


def main():
    global REPORT
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    graph_cli.BINARY = args.binary.resolve()
    REPORT = args.report.resolve()
    runtime_cli.FAILURE_ROOT = REPORT.parent/(REPORT.stem+"-failed-artifacts")
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(BytesCliAcceptance)
    result = unittest.TextTestRunner(verbosity=2, stream=sys.stderr).run(suite)
    report = {"suite":"bytes_cli_parity", "state":"PASSED" if result.wasSuccessful() else "FAILED",
        "passed":result.wasSuccessful(), "contract_sha256":runtime_cli.sha256(CONTRACT),
        "count":result.testsRun, "cases":BytesCliAcceptance.records,
        "failures":[{"test":str(test), "traceback":message} for test,message in result.failures],
        "errors":[{"test":str(test), "traceback":message} for test,message in result.errors]}
    runtime_cli.write_json(REPORT, report)
    print(json.dumps({key:value for key,value in report.items() if key != "cases"}))
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    raise SystemExit(main())
