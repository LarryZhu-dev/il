"""Independent ELF, semantic parity, ABI and reproducibility acceptance for P05."""
from __future__ import annotations

import argparse
import copy
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest

import execution_cli
import graph_cli

ROOT = Path(__file__).resolve().parents[1]
ABI_CONTRACT = Path(__file__).with_name("native_cli_contracts.json")
PROFILES = ("debug", "release")
ARTIFACTS = {"native_ir", "llvm_ir", "object", "executable"}
FAILURE_ARTIFACTS: Path | None = None
PROGRESS_REPORT: Path | None = None


def digest(path):
    return "sha256:" + hashlib.sha256(Path(path).read_bytes()).hexdigest()


def observables(execution):
    return {"status": execution["status"], "value": execution["value"],
            "stdout": execution["stdout"], "stderr": execution["stderr"],
            "live_allocations": execution["live_allocations"],
            "diagnostics": [{"code": diagnostic["code"], "entity_id": diagnostic["entity_id"]}
                            for diagnostic in execution["diagnostics"]]}


class NativeCliAcceptance(unittest.TestCase):
    setUp = graph_cli.GraphCliAcceptance.setUp
    git = graph_cli.GraphCliAcceptance.git
    invoke = graph_cli.GraphCliAcceptance.invoke
    assert_code = graph_cli.GraphCliAcceptance.assert_code
    head_revision = graph_cli.GraphCliAcceptance.head_revision
    records = []

    def save_progress(self):
        if PROGRESS_REPORT is not None:
            graph_cli.write_json(PROGRESS_REPORT, {"suite": "native_cli_blackbox", "state": "RUNNING",
                "passed": False, "cases": self.records, "numeric_contract_sha256": digest(execution_cli.CONTRACT),
                "abi_contract_sha256": digest(ABI_CONTRACT)})

    @contextmanager
    def isolated_case(self, identity):
        with tempfile.TemporaryDirectory(prefix="il-native-case-") as store:
            self.store = Path(store)
            try:
                yield
            except BaseException:
                if FAILURE_ARTIFACTS is not None:
                    destination = FAILURE_ARTIFACTS / identity
                    shutil.copytree(self.store, destination, dirs_exist_ok=True)
                    graph_cli.write_json(destination / "retention.json", {"case": identity,
                        "original_store": str(self.store), "retained_store": str(destination),
                        "reason": "native acceptance failed; paths in original evidence identify original execution"})
                raise

    def invoke_raw(self, command, request_text, expect_ok=True):
        completed = subprocess.run(
            [str(graph_cli.BINARY), "--repository", str(self.repository), "--store", str(self.store), command],
            input=request_text, capture_output=True, text=True, encoding="utf-8", timeout=180)
        try:
            response = json.loads(completed.stdout)
        except ValueError as error:
            self.fail(f"CLI emitted non-JSON stdout: {completed.stdout!r}; stderr={completed.stderr!r}: {error}")
        self.assertEqual(set(response), graph_cli.ENVELOPE)
        self.assertEqual(response["tool"], command)
        self.assertIs(response["ok"], expect_ok, response)
        self.assertEqual(completed.returncode == 0, expect_ok, response)
        for diagnostic in response["diagnostics"]:
            self.assertEqual(set(diagnostic), graph_cli.DIAGNOSTIC)
        if not expect_ok:
            self.assertTrue(response["diagnostics"], "failed invocation must preserve diagnostics")
        return response

    def publish(self, *, graph=None, source=None):
        self.assertNotEqual(graph is None, source is None)
        operation = ({"op": "replace_program", "graph": graph} if graph is not None
                     else {"op": "import_text", "source": source})
        return self.invoke("transact", {"task_id": "P05-native", "base_revision": 0,
            "scope": ["program"], "operations": [operation], "required_checks": []})

    def suite(self, arguments, limits=None):
        return {"entry": "main", "arguments": arguments,
                "limits": dict(execution_cli.LIMITS, **(limits or {}))}

    def execution_request(self, suite, isolation):
        return {"revision": 1, "suite": suite, "isolation": isolation}

    def validate_execution(self, execution):
        execution_cli.SCHEMA_CHECK.validate_file(ROOT, execution, "execution")
        self.assertIn(execution["status"], ("returned", "trapped"))

    def assert_elf(self, path, executable):
        header = Path(path).read_bytes()[:64]
        self.assertEqual(len(header), 64, "ELF header is truncated")
        self.assertEqual(header[:7], b"\x7fELF\x02\x01\x01", "artifact must be ELF64 little endian")
        kind, architecture = struct.unpack_from("<HH", header, 16)
        self.assertEqual(architecture, 62, "ELF architecture must be x86-64")
        self.assertIn(kind, (2, 3) if executable else (1,))

    def validate_native(self, response, profile, captured=True):
        self.assertEqual(response["result_revision"], 1)
        native = response["result"]["native"]
        self.assertEqual(native["target"], graph_cli.TARGET)
        self.assertEqual(native["profile"], profile)
        self.assertEqual(set(native["artifacts"]), ARTIFACTS)
        for name, artifact in native["artifacts"].items():
            self.assertEqual(set(artifact), {"path", "sha256"})
            path = Path(artifact["path"])
            self.assertTrue(path.is_absolute(), name)
            self.assertTrue(path.is_file(), name)
            self.assertEqual(artifact["sha256"], digest(path), name)
        record_path = Path(native["build_record"]["path"])
        self.assertTrue(record_path.is_absolute())
        self.assertEqual(native["build_record"]["sha256"], digest(record_path))
        record = json.loads(record_path.read_text(encoding="utf-8"))
        execution_cli.SCHEMA_CHECK.validate_file(ROOT, record, "native_build")
        native_program = json.loads(Path(native["artifacts"]["native_ir"]["path"]).read_text(encoding="utf-8"))
        execution_cli.SCHEMA_CHECK.validate_file(ROOT, native_program, "native_ir")
        self.assertEqual(record["status"], "VERIFIED")
        self.assertEqual(record["scope"], "native_compilation")
        self.assertEqual(record["revision"], 1)
        self.assertEqual(record["artifacts"], native["artifacts"])
        self.assertEqual(record["input"]["profile"], profile)
        self.assertEqual(record["input"]["native_ir_hash"], native["artifacts"]["native_ir"]["sha256"])
        self.assertEqual(record["input"]["compiler_hash"], digest(graph_cli.BINARY))
        self.assertEqual(record["input"]["runtime_hash"], digest(graph_cli.BINARY.with_name("libil_native_runtime.a")))
        driver_path = Path(record["driver_record"]["path"])
        self.assertTrue(driver_path.is_absolute())
        self.assertEqual(record["driver_record"]["sha256"], digest(driver_path))
        driver = json.loads(driver_path.read_text(encoding="utf-8"))
        self.assertEqual(record["input"]["tools"], driver["tools"])
        self.assertEqual(driver["profile"], profile)
        for name in ("llvm_ir", "object", "executable"):
            self.assertEqual(driver[name], native["artifacts"][name])
        self.assertEqual(driver["optimized_ir"]["sha256"], digest(driver["optimized_ir"]["path"]))
        self.assertEqual(driver["runtime"]["sha256"], record["input"]["runtime_hash"])
        self.assertEqual([command["stage"] for command in driver["commands"]],
                         ["identify_tool", "identify_tool", "verify_llvm", "optimize_llvm", "emit_object", "identify_tool", "link_runtime"])
        self.assertEqual([command["program"] for command in driver["commands"]],
                         ["/usr/bin/opt-14", "/usr/bin/llc-14", "/usr/bin/opt-14",
                          "/usr/bin/opt-14", "/usr/bin/llc-14", "/usr/bin/clang-14", "/usr/bin/clang-14"])
        for command in driver["commands"]:
            self.assertEqual(command["exit_code"], 0)
            self.assertTrue(command["arguments"])
        expected_optimization = "-O0" if profile == "debug" else "-O2"
        self.assertIn(expected_optimization, driver["commands"][3]["arguments"])
        self.assertIn(expected_optimization, driver["commands"][4]["arguments"])
        self.assertEqual({tool["path"] for tool in driver["tools"]},
                         {"/usr/bin/opt-14", "/usr/bin/llc-14", "/usr/bin/clang-14"})
        for tool in driver["tools"]:
            self.assertEqual(tool["sha256"], digest(tool["path"]))
            self.assertIn("14.0.6", tool["version"])
        self.assert_elf(native["artifacts"]["object"]["path"], False)
        self.assert_elf(native["artifacts"]["executable"]["path"], True)
        stages = response["result"]["stages"]
        expected_stages = ["lower_to_hir", "lower_to_mir", "verify_mir", "lower_to_native_ir",
                           "verify_native_ir", "codegen_x86_64", "verify_llvm", "optimize_llvm",
                           "emit_object", "link_runtime", "emit_elf"]
        self.assertEqual([stage["stage"] for stage in stages], expected_stages + (["execute_native"] if captured else []))
        self.assertEqual(record["stages"], stages[:len(expected_stages)])
        for stage in stages:
            self.assertRegex(stage["input_hash"], r"^sha256:[a-f0-9]{64}$")
            self.assertRegex(stage["output_hash"], r"^sha256:[a-f0-9]{64}$")
            if stage["stage"] != "execute_native":
                self.assertEqual(stage["diagnostics"], [], stage)
        self.assertEqual(stages[3]["output_hash"], native["artifacts"]["native_ir"]["sha256"])
        self.assertEqual(stages[4]["input_hash"], stages[3]["output_hash"])
        self.assertEqual(stages[4]["output_hash"], stages[3]["output_hash"])
        self.assertEqual(stages[5]["output_hash"], native["artifacts"]["llvm_ir"]["sha256"])
        self.assertEqual(stages[8]["output_hash"], native["artifacts"]["object"]["sha256"])
        self.assertEqual(stages[7]["output_hash"], driver["optimized_ir"]["sha256"])
        self.assertEqual(stages[8]["input_hash"], driver["optimized_ir"]["sha256"])
        self.assertEqual(stages[10]["output_hash"], native["artifacts"]["executable"]["sha256"])
        self.assertFalse([path for path in record_path.parent.rglob("*")
                          if path.is_file() and path.suffix.lower() in {".c", ".cc", ".cpp", ".rs"}],
                         "native compilation emitted forbidden high-level target source")
        if captured:
            self.assertEqual(native["report_fd"], 3)
            self.assertEqual(native["argv"], [native["artifacts"]["executable"]["path"]])
        return native

    def execute_elf(self, native):
        """Run the retained executable directly; no compiler process participates."""
        with tempfile.TemporaryDirectory(prefix="il-native-isolated-") as isolated:
            report_path = Path(isolated) / "fd3.json"
            with report_path.open("wb") as report:
                def report_descriptor():
                    os.dup2(report.fileno(), 3)
                    os.set_inheritable(3, True)
                completed = subprocess.run(native["argv"], cwd=isolated, env={},
                    stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                    preexec_fn=report_descriptor, close_fds=False, timeout=30)
            self.assertLessEqual(report_path.stat().st_size, 4 * 1024 * 1024)
            try:
                execution = json.loads(report_path.read_text(encoding="utf-8"))
            except ValueError as error:
                self.fail(f"ELF fd3 report malformed/missing: {error}; exit={completed.returncode}; "
                          f"stdout={completed.stdout!r}; stderr={completed.stderr!r}")
            self.validate_execution(execution)
            self.assertEqual(completed.returncode, 0 if execution["status"] == "returned" else 101)
            self.assertEqual(completed.returncode, native["exit_code"])
            self.assertEqual(completed.stdout, bytes(execution["stdout"]))
            self.assertEqual(completed.stderr, bytes(execution["stderr"]))
            return execution

    def assert_expected(self, execution, expected, numeric_type=None):
        self.assertEqual(execution["status"], expected["status"])
        self.assertEqual(execution["stdout"], list(expected.get("stdout", "").encode("utf-8")))
        self.assertEqual(execution["stderr"], [])
        self.assertEqual(execution["live_allocations"], expected.get("live_allocations", 0))
        if expected["status"] == "returned":
            if numeric_type is None:
                value = expected["value"]
            elif numeric_type == "Bool":
                value = {"type": "Bool", "data": {"kind": "bool", "value": expected["value"]}}
            else:
                value = execution_cli.integer(numeric_type, expected["value"])
            self.assertEqual(execution["value"], value)
            self.assertEqual(execution["diagnostics"], [])
        else:
            self.assertIsNone(execution["value"])
            self.assertEqual([d["code"] for d in execution["diagnostics"]], [expected["code"]])
            if "entity_id" in expected:
                self.assertEqual(execution["diagnostics"][0]["entity_id"], expected["entity_id"])

    def compare_case(self, case_id, suite, expected, numeric_type=None):
        record = {"id": case_id, "passed": False, "profiles": {}}
        self.records.append(record)
        self.save_progress()
        success = expected["status"] == "returned"
        interpreted = self.invoke("test", self.execution_request(suite, "captured"), success)["result"]["execution"]
        self.validate_execution(interpreted)
        self.assert_expected(interpreted, expected, numeric_type)
        profiles = record["profiles"]
        for profile in PROFILES:
            response = self.invoke("test", self.execution_request(suite, "native_" + profile), success)
            execution = response["result"]["execution"]
            self.validate_execution(execution)
            native = self.validate_native(response, profile)
            execution_input = response["result"]["execution_input"]
            self.assertEqual(execution_input["entry"], suite["entry"])
            self.assertEqual(execution_input["arguments"], suite["arguments"])
            self.assertEqual(execution_input["limits"], suite["limits"])
            self.assertEqual(execution_input["profile"], profile)
            self.assertEqual(execution_input["isolation"], "native_" + profile)
            self.assertEqual(execution_input["target"], graph_cli.TARGET)
            self.assertEqual(execution_input["native_ir_hash"], native["artifacts"]["native_ir"]["sha256"])
            self.assertEqual(execution_input["compiler_hash"], digest(graph_cli.BINARY))
            self.assertEqual(execution_input["runtime_hash"], digest(graph_cli.BINARY.with_name("libil_native_runtime.a")))
            self.assertEqual(execution_input["toolchain_lock_hash"], digest(self.repository / "toolchain.lock"))
            for tool in execution_input["tools"]:
                self.assertEqual(tool["sha256"], digest(tool["path"]))
            canonical_input = (json.dumps(execution_input, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode("utf-8")
            self.assertEqual(response["result"]["stages"][-1]["input_hash"],
                             "sha256:" + hashlib.sha256(canonical_input).hexdigest())
            direct = self.execute_elf(native)
            # The fd3 report preserves the locked struct/schema order. The CLI
            # envelope stores generic JSON maps, whose alphabetic order differs.
            canonical_execution = (json.dumps(direct, ensure_ascii=False, indent=2) + "\n").encode("utf-8")
            self.assertEqual(response["result"]["stages"][-1]["output_hash"],
                             "sha256:" + hashlib.sha256(canonical_execution).hexdigest())
            self.assertEqual(observables(execution), observables(direct), "CLI and direct ELF disagree")
            self.assertEqual(observables(execution), observables(interpreted), "native and MIR semantics disagree")
            self.assert_expected(direct, expected, numeric_type)
            profiles[profile] = {"artifacts": {name: artifact["sha256"] for name, artifact in native["artifacts"].items()},
                                 "exit_code": native["exit_code"], "observables": observables(direct)}
            self.save_progress()
        record["passed"] = True
        self.save_progress()

    def test_locked_numeric_execution_matrix(self):
        cases = json.loads(execution_cli.CONTRACT.read_text(encoding="utf-8"))["cases"]
        self.assertEqual(len(cases), 180)
        for case in cases:
            with self.subTest(case=case["id"]), self.isolated_case(case["id"]):
                self.publish(graph=execution_cli.numeric_graph(case))
                arguments = [execution_cli.integer(case["type"], value) for value in case["operands"]]
                expected = dict(case["expected"])
                if expected["status"] == "trapped":
                    expected["entity_id"] = "main.calculate"
                self.compare_case(case["id"], self.suite(arguments), expected, case.get("result_type", case["type"]))

    def test_locked_structured_and_abi_contract(self):
        cases = json.loads(ABI_CONTRACT.read_text(encoding="utf-8"))["cases"]
        for case in cases:
            with self.subTest(case=case["id"]), self.isolated_case(case["id"]):
                self.publish(source=case["source"])
                self.compare_case(case["id"], self.suite(case["arguments"], case.get("limits")), case["expected"])

    def test_preexisting_graph_semantics_and_aggregate_abi(self):
        integer = execution_cli.integer
        string = {"type": "String", "data": {"kind": "string", "value": "你好 il"}}
        unit = {"type": "Unit", "data": {"kind": "unit"}}
        def variant(ty, tag, fields):
            return {"type": ty, "data": {"kind": "variant", "value": {"tag": tag, "fields": fields}}}
        fixtures = [
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
        for index, (name, arguments, value) in enumerate(fixtures):
            with self.subTest(fixture=name, arguments=arguments), self.isolated_case(f"graph_{index}_{name}"):
                graph = json.loads((ROOT / "tests/fixtures/semantics" / (name + ".json")).read_text())
                self.publish(graph=graph)
                self.compare_case(f"graph_{index}_{name}", self.suite(arguments),
                    {"status": "returned", "value": value, "live_allocations": 1 if value["type"] == "String" else 0})

    def test_two_clean_stores_reproduce_native_artifacts(self):
        case = json.loads(ABI_CONTRACT.read_text(encoding="utf-8"))["cases"][0]
        builds = []
        directories = set()
        for index in range(2):
            self.store = self.root / f"clean-{index}"
            self.assertFalse(self.store.exists())
            self.publish(source=case["source"])
            hashes = {}
            for profile in PROFILES:
                response = self.invoke("test", self.execution_request(self.suite(case["arguments"]), "native_" + profile))
                native = self.validate_native(response, profile)
                directory = str(Path(native["artifacts"]["executable"]["path"]).parent)
                self.assertNotIn(directory, directories, "clean build reused another build's artifact directory")
                directories.add(directory)
                self.assert_expected(self.execute_elf(native), case["expected"])
                hashes[profile] = {name: artifact["sha256"] for name, artifact in native["artifacts"].items()}
            builds.append(hashes)
        self.assertEqual(builds[0], builds[1], "clean build paths changed native artifact hashes")
        self.records.append({"id": "two_clean_native_stores", "passed": True, "hashes": builds[0]})

    def test_application_entrypoint_and_rejected_build_preserve_artifacts(self):
        self.publish(source=(ROOT / "examples/core/native_hello.il").read_text(encoding="utf-8"))
        saved = []
        for profile in PROFILES:
            response = self.invoke("build", {"revision": 1, "target": graph_cli.TARGET, "profile": profile})
            native = self.validate_native(response, profile, captured=False)
            binary = native["artifacts"]["executable"]
            with tempfile.TemporaryDirectory(prefix="il-native-application-") as cwd:
                executed = subprocess.run([binary["path"]], cwd=cwd, env={}, capture_output=True, timeout=30)
            self.assertEqual((executed.returncode, executed.stdout, executed.stderr), (0, b"Hello from Intelligent language!\n", b""))
            saved.append(binary)
        request = {"revision": 1, "target": graph_cli.TARGET, "profile": "debug"}
        for changed in [dict(request, shell="forbidden"), dict(request, profile="unknown"),
                        dict(request, target="unconfigured-target")]:
            self.invoke("build", changed, False)
        self.assertEqual(self.head_revision(), 1)
        self.invoke("transact", {"task_id": "P05-invalid-entry", "base_revision": 1,
            "scope": ["program"], "operations": [{"op": "import_text", "source":
            'module app { @id("main") fn main()->Unit {return;} }'}], "required_checks": []})
        self.invoke("build", dict(request, revision=2), False)
        self.assertEqual(self.head_revision(), 2)
        for artifact in saved:
            self.assertEqual(digest(artifact["path"]), artifact["sha256"])
            completed = subprocess.run([artifact["path"]], cwd=self.root, env={}, capture_output=True, timeout=30)
            self.assertEqual((completed.returncode, completed.stdout, completed.stderr), (0, b"Hello from Intelligent language!\n", b""))

    def test_physical_llvm_verifier_rejects_invalid_ir(self):
        invalid = self.root / "invalid.ll"
        invalid.write_text('target triple = "x86_64-unknown-linux-gnu"\n'
                           'define i32 @main() {\n'
                           'entry: br i1 true, label %left, label %right\n'
                           'left: %x = add i32 1, 2\n br label %right\n'
                           'right: ret i32 %x\n}\n', encoding="utf-8")
        completed = subprocess.run(["/usr/bin/opt-14", "-verify", "-disable-output", str(invalid)],
                                   capture_output=True, timeout=30)
        self.assertNotEqual(completed.returncode, 0, "LLVM accepted invalid SSA dominance")
        self.assertIn(b"does not dominate", completed.stderr)
        self.records.append({"id": "invalid_llvm_dominance", "passed": True,
                             "exit_code": completed.returncode,
                             "stderr": completed.stderr.decode("utf-8", errors="replace")})

    def test_native_request_is_closed_and_wrong_argument_is_rejected(self):
        case = {"type": "I64", "opcode": "add", "operands": [20, 22]}
        self.publish(graph=execution_cli.numeric_graph(case))
        suite = self.suite([execution_cli.integer("I64", n) for n in case["operands"]])
        for profile in PROFILES:
            request = self.execution_request(suite, "native_" + profile)
            self.assert_code(self.invoke("test", dict(request, shell="forbidden"), False), "E_SCHEMA_INVALID")
            self.assert_code(self.invoke("test", dict(request, environment={"PATH": "/tmp"}), False), "E_SCHEMA_INVALID")
            self.assert_code(self.invoke("test", dict(request, isolation="native_shell"), False), "E_SCHEMA_INVALID")
            unknown_suite = copy.deepcopy(request)
            unknown_suite["suite"]["command"] = "forbidden"
            self.assert_code(self.invoke("test", unknown_suite, False), "E_SCHEMA_INVALID")
            unknown_limit = copy.deepcopy(request)
            unknown_limit["suite"]["limits"]["environment"] = {}
            self.assert_code(self.invoke("test", unknown_limit, False), "E_SCHEMA_INVALID")
            wrong = copy.deepcopy(request)
            wrong["suite"]["arguments"][0] = execution_cli.integer("U64", 20)
            self.assert_code(self.invoke("test", wrong, False), "E_TYPE_MISMATCH")
        self.assertEqual(self.head_revision(), 1)

    def closed_stdout_run(self, executable, report_path=None):
        """An inherited pipe with no reader tests the actual process I/O boundary."""
        with tempfile.TemporaryDirectory(prefix="il-native-closed-stdout-") as cwd:
            reader, writer = os.pipe()
            os.close(reader)
            try:
                if report_path is None:
                    return subprocess.run([executable], cwd=cwd, env={}, stdin=subprocess.DEVNULL,
                        stdout=writer, stderr=subprocess.PIPE, timeout=30, restore_signals=True)
                with report_path.open("wb") as report:
                    def report_descriptor():
                        os.dup2(report.fileno(), 3)
                        os.set_inheritable(3, True)
                    return subprocess.run([executable], cwd=cwd, env={}, stdin=subprocess.DEVNULL,
                        stdout=writer, stderr=subprocess.PIPE, timeout=30, restore_signals=True,
                        preexec_fn=report_descriptor, close_fds=False)
            finally:
                os.close(writer)

    def test_closed_stdout_returns_typed_write_error_and_continues(self):
        contract = json.loads(ABI_CONTRACT.read_text(encoding="utf-8"))
        for case in contract["closed_stdout"]:
            for mode in ("captured", "application"):
                identity = case["id"] + "_" + mode
                with self.subTest(case=identity), self.isolated_case(identity):
                    declaration = '@id("Out") type Out=Result<Unit,core.IoError>;'
                    if mode == "captured":
                        body = f'return {case["expression"]};'
                        result_type = "Out"
                    else:
                        # Only the typed Write branch succeeds. Both a false Ok
                        # and another error trap, proving execution continued.
                        failure = 'let fail:I32=1/0;return fail;'
                        body = (f'let output:Out={case["expression"]};'
                                'match output {Ok(unit)=>{' + failure + '}'
                                'Err(error)=>{match error {Write=>{return 0;}_=>{' + failure + '}}}}')
                        result_type = "I32"
                    source = ('module app {' + declaration + f'@id("main") fn main()->{result_type} '
                              f'effects [{case["effects"]}] {{' + body + '}}')
                    self.publish(source=source)
                    record = {"id": identity, "passed": False, "profiles": {}}
                    self.records.append(record)
                    self.save_progress()
                    for profile in PROFILES:
                        if mode == "captured":
                            response = self.invoke("test", self.execution_request(self.suite([]), "native_" + profile))
                            native = self.validate_native(response, profile)
                            open_execution = response["result"]["execution"]
                            self.assert_expected(open_execution, {"status": "returned", "live_allocations": 0,
                                "stdout": case["open_stdout"], "value": {"type": "Out", "data": {
                                "kind": "variant", "value": {"tag": "Ok", "fields": [
                                {"type": "Unit", "data": {"kind": "unit"}}]}}}})
                            report_path = self.store / ("closed-stdout-" + profile + ".json")
                            completed = self.closed_stdout_run(native["artifacts"]["executable"]["path"], report_path)
                            self.assertEqual(completed.returncode, 0, f"closed pipe killed or trapped process: {completed.stderr!r}")
                            self.assertEqual(completed.stderr, b"")
                            execution = json.loads(report_path.read_text(encoding="utf-8"))
                            self.validate_execution(execution)
                            self.assert_expected(execution, contract["closed_stdout_expected"])
                            result = {"exit_code": completed.returncode, "execution": observables(execution)}
                        else:
                            response = self.invoke("build", {"revision": 1, "target": graph_cli.TARGET, "profile": profile})
                            native = self.validate_native(response, profile, captured=False)
                            completed = self.closed_stdout_run(native["artifacts"]["executable"]["path"])
                            self.assertEqual(completed.returncode, contract["closed_stdout_application_exit"],
                                             f"application did not reach typed Write branch: {completed.stderr!r}")
                            self.assertEqual(completed.stderr, b"")
                            result = {"exit_code": completed.returncode}
                        result["executable_sha256"] = native["artifacts"]["executable"]["sha256"]
                        record["profiles"][profile] = result
                        self.save_progress()
                    record["passed"] = True
                    self.save_progress()

    def test_unknown_cli_flag_is_rejected_before_build(self):
        completed = subprocess.run([str(graph_cli.BINARY), "--repository", str(self.repository),
            "--store", str(self.store), "--shell", "build"], input="{}", text=True,
            capture_output=True, timeout=30)
        self.assertNotEqual(completed.returncode, 0)
        response = json.loads(completed.stdout)
        self.assertFalse(response["ok"])
        self.assert_code(response, "E_SCHEMA_INVALID")
        self.assertEqual(self.head_revision(), 0)


def main():
    global FAILURE_ARTIFACTS, PROGRESS_REPORT
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    FAILURE_ARTIFACTS = args.report.resolve().parent / (args.report.stem + "-failed-artifacts")
    PROGRESS_REPORT = args.report.resolve()
    graph_cli.BINARY = args.binary.resolve()
    report = {"suite": "native_cli_blackbox", "passed": False, "state": "RUNNING",
              "target": graph_cli.TARGET,
              "numeric_contract_sha256": digest(execution_cli.CONTRACT),
              "abi_contract_sha256": digest(ABI_CONTRACT), "cases": []}
    graph_cli.write_json(args.report, report)
    try:
        if platform.system() != "Linux" or platform.machine() != "x86_64":
            raise RuntimeError("P05 native acceptance requires real Linux x86-64 execution")
        if not graph_cli.BINARY.is_file():
            raise RuntimeError("il binary does not exist")
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(NativeCliAcceptance)
        result = unittest.TextTestRunner(verbosity=2, stream=sys.stderr).run(suite)
        report.update({"passed": result.wasSuccessful(), "state": "PASSED" if result.wasSuccessful() else "FAILED",
            "count": result.testsRun, "cases": NativeCliAcceptance.records,
            "failures": [{"test": str(test), "traceback": error} for test, error in result.failures],
            "errors": [{"test": str(test), "traceback": error} for test, error in result.errors]})
    except (OSError, RuntimeError) as error:
        report.update({"state": "FAILED", "error": str(error)})
    finally:
        graph_cli.write_json(args.report, report)
    print(json.dumps({key: value for key, value in report.items() if key != "cases"}, ensure_ascii=False))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
