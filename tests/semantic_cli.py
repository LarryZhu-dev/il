"""Independent P03 contracts and bounded fuzzing through the il process boundary.

Usage: python tests/semantic_cli.py --binary /absolute/path/to/il --report FILE
Expected behavior comes from committed fixtures, never compiler implementation
imports. All validation requests ask for the complete semantic checking pipeline.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
from pathlib import Path
import random
import subprocess
import sys
import tempfile
import time


FIXTURES = Path(__file__).resolve().parent / "fixtures" / "semantics"
TARGET = "x86_64-unknown-linux-gnu"
FUZZ_SEED = 20261008
FUZZ_COUNT = 200
ENVELOPE = {
    "ok", "tool", "tool_version", "base_revision", "result_revision",
    "diagnostics", "artifacts", "evidence_id", "result",
}
DIAGNOSTIC = {
    "diagnostic_id", "code", "stage", "severity", "entity_id",
    "related_entities", "expected", "actual", "cause",
    "suggested_operations", "retryable", "base_revision",
}


def require(predicate, message):
    if not predicate:
        raise AssertionError(message)


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


class SourceFixture:
    def __init__(self, root, binary):
        self.repository = root / "source"
        self.repository.mkdir()
        self.store = root / "application"
        self.store.mkdir()
        self.binary = binary
        self.calls = 0
        self.git("init", "--initial-branch=main")
        state = {
            "project_id": "il", "schema_version": "1.0.0", "head_revision": 0,
            "last_verified_revision": 0, "compiler_version": None,
            "runtime_version": None, "target": TARGET, "open_tasks": ["P03"],
            "blocked_tasks": [], "failed_tasks": [], "locks": {},
        }
        seed = {
            "project_id": "il", "graph_version": "1.0.0", "revision": 0,
            "target": TARGET, "modules": [], "types": [], "functions": [],
            "capabilities": [], "packages": [], "contracts": [],
        }
        write_json(self.repository / "repository_state.json", state)
        write_json(self.repository / "examples/bootstrap/graph.json", seed)
        write_json(self.repository / "toolchain.lock", {
            "target": TARGET, "rust": "1.90.0", "llvm": "14.0.6",
        })
        self.git("add", "repository_state.json", "examples/bootstrap/graph.json", "toolchain.lock")
        self.git("-c", "user.name=il semantic test", "-c", "user.email=test@invalid.local",
                 "-c", "commit.gpgsign=false", "-c", "core.hooksPath=" + str(root / "no-hooks"),
                 "commit", "-m", "test: initialize independent semantic fixture")
        write_json(self.repository / ".git/il/revision_bindings.json", {
            "schema_version": "1.0.0", "bindings": [{
                "revision": 0, "git_commit": self.git("rev-parse", "HEAD"),
                "tree_hash": self.git("rev-parse", "HEAD^{tree}"),
            }],
        })

    def git(self, *arguments):
        process = subprocess.run(["git", "-C", str(self.repository), *arguments],
                                 capture_output=True, text=True, encoding="utf-8", timeout=20)
        require(process.returncode == 0, f"fixture Git failure: {process.stderr}")
        return process.stdout.strip()

    def invoke(self, request_text):
        self.calls += 1
        process = subprocess.run(
            [str(self.binary), "--repository", str(self.repository),
             "--store", str(self.store), "validate"],
            input=request_text, capture_output=True, text=True, encoding="utf-8", timeout=15,
        )
        require(process.returncode in (0, 1),
                f"CLI crashed/exited unexpectedly ({process.returncode}): {process.stderr[:2000]}")
        require(len(process.stdout.encode("utf-8")) <= 262145, "CLI exceeded its output bound")
        try:
            response = json.loads(process.stdout)
        except ValueError as error:
            raise AssertionError(f"non-JSON response: {process.stdout[:2000]!r}; stderr={process.stderr[:2000]!r}") from error
        require(isinstance(response, dict) and set(response) == ENVELOPE,
                f"invalid response envelope: {response!r}")
        require(response["tool"] == "validate", "response is not tied to validate")
        require(type(response["ok"]) is bool, "ok must be a JSON boolean")
        require((process.returncode == 0) == response["ok"], "exit code contradicts structured result")
        require(isinstance(response["diagnostics"], list), "diagnostics must be an array")
        if not response["ok"]:
            require(bool(response["diagnostics"]), "failed validation requires a machine-readable diagnostic")
        for diagnostic in response["diagnostics"]:
            require(isinstance(diagnostic, dict) and set(diagnostic) == DIAGNOSTIC,
                    f"invalid diagnostic shape: {diagnostic!r}")
            require(isinstance(diagnostic["code"], str) and diagnostic["code"].startswith("E_"),
                    "diagnostic requires a stable error code")
            require(type(diagnostic["base_revision"]) is int and diagnostic["base_revision"] >= 0,
                    "diagnostic must identify its revision")
        return response


def fuzz_requests(checks, valid_graph):
    rng = random.Random(FUZZ_SEED)
    baseline = json.dumps({"graph_or_revision": valid_graph, "checks": checks})
    alphabet = ' abcdef0123456789{}[],:"\\\n\t!?'
    for index in range(FUZZ_COUNT):
        if index < 40:
            yield "raw", "".join(rng.choice(alphabet) for _ in range(rng.randrange(0, 512)))
            continue
        if index < 80:
            cut = rng.randrange(len(baseline))
            yield "truncated_json", baseline[:cut]
            continue
        graph = copy.deepcopy(valid_graph)
        if index < 140:
            # Concrete graph shapes, including long value-type chains and cycles,
            # exercise the actual structural/type checker beyond the JSON parser.
            length = rng.randrange(1, 48)
            graph["types"] = []
            for position in range(length):
                identity = f"FuzzType{position}"
                reference = rng.choice(["I64", "Bool", "String", "Missing", f"FuzzType{rng.randrange(length)}"])
                kind = rng.choice(["record", "option", "result", "tuple"])
                item = {"entity_id": identity, "kind": kind, "layout": "inferred", "parameters": []}
                if kind == "record":
                    item["fields"] = [{"name": "value", "type": reference}]
                elif kind == "result":
                    item["parameters"] = [reference, "Bool"]
                else:
                    item["parameters"] = [reference]
                graph["types"].append(item)
            graph["modules"][0]["declarations"].extend(item["entity_id"] for item in graph["types"])
            yield "type_graph", json.dumps({"graph_or_revision": graph, "checks": checks})
            continue
        operation = graph["functions"][0]["blocks"][0]["operations"][0]
        mutation = (index - 140) % 10
        if mutation == 0:
            operation["attributes"]["value"] = rng.choice([True, None, "hello", -129, 2**64, []])
        elif mutation == 1:
            operation["outputs"][0]["type"] = rng.choice(["Bool", "I8", "U64", "String", "Missing"])
        elif mutation == 2:
            operation["opcode"] = rng.choice(["unknown", "return", "add", "borrow", "call", "switch"])
        elif mutation == 3:
            operation["inputs"] = rng.choice([[], ["missing"], [operation["outputs"][0]["entity_id"]]])
        elif mutation == 4:
            operation["effects"] = rng.choice([["net"], ["invented"], ["alloc", "alloc"]])
        elif mutation == 5:
            operation["unexpected"] = "must be rejected"
        elif mutation == 6:
            del operation[rng.choice(list(operation))]
        elif mutation == 7:
            graph["functions"][0]["blocks"][0]["operations"].append(copy.deepcopy(operation))
        elif mutation == 8:
            graph["functions"][0]["result"] = rng.choice(["Bool", "Unit", "String", "Missing"])
        else:
            graph["functions"][0]["blocks"][0]["terminator"]["inputs"] = ["missing"]
        yield "operation_graph", json.dumps({"graph_or_revision": graph, "checks": checks})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    arguments = parser.parse_args()
    binary = arguments.binary.resolve()
    if not binary.is_file():
        parser.error(f"binary not found: {binary}")
    contract = json.loads((FIXTURES / "cases.json").read_text(encoding="utf-8"))
    checks = contract["checks"]
    records = []
    started = time.monotonic()
    fixture_hashes = {}

    with tempfile.TemporaryDirectory(prefix="il-semantics-blackbox-") as directory:
        context = SourceFixture(Path(directory), binary)
        for case in contract["cases"]:
            path = FIXTURES / case["file"]
            fixture_bytes = path.read_bytes()
            fixture_hashes[case["file"]] = "sha256:" + hashlib.sha256(fixture_bytes).hexdigest()
            graph = json.loads(fixture_bytes)
            request_text = json.dumps({"graph_or_revision": graph, "checks": checks})
            record = {"id": case["id"], "kind": "contract", "passed": False}
            try:
                response = context.invoke(request_text)
                require(response["ok"] is case["valid"],
                        f"expected valid={case['valid']}: {response['diagnostics']}")
                require(response["result_revision"] == graph["revision"], "validation result uses another graph revision")
                if case["valid"]:
                    require(not response["diagnostics"], "positive fixture emitted diagnostics")
                    require(response["result"].get("valid") is True, "positive fixture lacks explicit validity")
                else:
                    codes = [item["code"] for item in response["diagnostics"]]
                    require(case["diagnostic_code"] in codes,
                            f"expected {case['diagnostic_code']}, got {response['diagnostics']}")
                    repeated = context.invoke(request_text)
                    require(repeated["diagnostics"] == response["diagnostics"],
                            "identical invalid input produced nondeterministic diagnostics")
                    record["diagnostic_codes"] = codes
                record["passed"] = True
            except (AssertionError, OSError, ValueError, subprocess.SubprocessError) as error:
                record["error"] = str(error)
            records.append(record)
            print(f"{'PASS' if record['passed'] else 'FAIL'} {case['id']}" +
                  (f": {record['error']}" if "error" in record else ""), file=sys.stderr, flush=True)

        valid_graph = json.loads((FIXTURES / "valid_integer_return.json").read_text(encoding="utf-8"))
        for index, (kind, request_text) in enumerate(fuzz_requests(checks, valid_graph)):
            record = {"id": f"fuzz_{index:03}", "kind": kind, "passed": False,
                      "input_hash": "sha256:" + hashlib.sha256(request_text.encode("utf-8")).hexdigest()}
            try:
                response = context.invoke(request_text)
                record["diagnostic_codes"] = [item["code"] for item in response["diagnostics"]]
                record["passed"] = True
            except (AssertionError, OSError, ValueError, subprocess.SubprocessError) as error:
                record["error"] = str(error)
                # Preserve the exact reproducer on failure, never replace it with
                # a passing generated value or weaken the contract expectation.
                record["input"] = request_text
                print(f"FAIL fuzz_{index:03}: {error}", file=sys.stderr, flush=True)
            records.append(record)

        report = {
            "suite": "semantic_cli_blackbox", "passed": all(record["passed"] for record in records),
            "count": len(records), "contract_count": len(contract["cases"]),
            "fuzz_count": FUZZ_COUNT, "fuzz_seed": FUZZ_SEED, "process_count": context.calls,
            "elapsed_seconds": round(time.monotonic() - started, 6),
            "fixture_hashes": fixture_hashes, "cases": records,
        }
    write_json(arguments.report, report)
    print(json.dumps({key: value for key, value in report.items() if key not in ("fixture_hashes", "cases")},
                     ensure_ascii=False))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
