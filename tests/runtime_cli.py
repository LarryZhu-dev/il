"""Independent P06 host fixtures, process boundaries and runtime-profile checks."""
from __future__ import annotations

from contextlib import contextmanager
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import time
import unittest

import execution_cli
import graph_cli

CONTRACT_PATH = Path(__file__).with_name("runtime_cli_contracts.json")
ROOT = Path(__file__).resolve().parents[1]
REPORT_PATH = None
FAILURE_ROOT = None


def contract():
    return json.loads(CONTRACT_PATH.read_text(encoding="utf-8"))


def sha256(path):
    return "sha256:" + hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write_json(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return path


def policy_hash(policy):
    encoded = (json.dumps(policy, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode("utf-8")
    return "sha256:" + hashlib.sha256(encoded).hexdigest()


def returned_buffer_owners(value):
    if value is None:
        return 0
    data = value["data"]
    if data["kind"] in ("string", "bytes"):
        return 1
    if data["kind"] in ("record", "tuple"):
        return sum(returned_buffer_owners(field) for field in data["value"])
    if data["kind"] == "variant":
        return sum(returned_buffer_owners(field) for field in data["value"]["fields"])
    return 0


CORE_DECLARATIONS = '''@id("core") module core visibility public {
  @id("core.File") type File=record {slot:U64,generation:U64} layout opaque;
  @id("core.Deadline") type Deadline=sum {Infinite,At(U64)};
  @id("core.IoError") type IoError=sum {Read,Write,Closed,Timeout,PermissionDenied,InvalidData,Other};
  @id("core.AllocError") type AllocError=sum {OutOfMemory,CapacityOverflow};
}'''


def file_source(host, case):
    """Spell out the independent language program for one locked file scenario."""
    operation = case["operation"]
    writes = operation in {"file_write", "file_write_some", "two_file_writes"}
    grant_name = "write" if writes else "read"
    grant = host.grant(grant_name)
    path = case.get("path", contract()["fixture"]["write_file" if writes else "read_file"])
    deadline = ('core.Deadline::At(0)' if case.get("deadline") == {"At": "0"}
                else 'core.Deadline::Infinite()')
    success = "Usize" if writes else "Bytes"
    parameters = 'path:String,data:Bytes' if writes else 'path:String'
    declarations = '@id("Open") type Open=Result<core.File,core.IoError>;'
    if operation == "file_read":
        body = f'return runtime.file_read[{grant["entity_id"]}](path);'
    elif operation == "file_write":
        body = f'return runtime.file_write[{grant["entity_id"]}](path,data);'
    else:
        open_call = f'runtime.file_open_{"write" if writes else "read"}[{grant["entity_id"]}](path)'
        if operation == "return_file":
            success = "core.File"
            body = 'return ' + open_call + ';'
        elif operation == "question_error_cleanup":
            success = "I64"
            declarations += '@id("Failure") type Failure=Result<I64,I64>;fn fail()->Failure{return Err(9);}'
            body = ('let opened:Open=' + open_call + ';match opened {Ok(file)=>{let n:I64=fail()?;return Ok(n);}'
                    'Err(error)=>{return Err(8);}}')
        else:
            body = f'let file:core.File={open_call}?;'
            if operation == "file_close":
                success = "Unit"
                body += 'return runtime.file_close(file);'
            elif operation == "drop_at_return":
                success = "I64"
                body += 'return Ok(42);'
            elif operation == "drop_aggregate_at_return":
                success = "I64"
                declarations += '@id("Owned") type Owned=record {file:core.File};'
                body += 'let aggregate:Owned=Owned {file:file};return Ok(42);'
            elif writes:
                offset = case.get("offset", 0)
                if operation == "two_file_writes":
                    body += f'let first:Usize=runtime.file_write_some(file,data,0,{deadline})?;'
                    body += f'let second:Out=runtime.file_write_some(file,data,2,{deadline});return Ok(tuple(first,second));'
                else:
                    body += f'return runtime.file_write_some(file,data,{offset},{deadline});'
            else:
                maximum = case.get("maximum", 6)
                if operation == "two_file_reads":
                    body += f'let first:Bytes=runtime.file_read_some(file,6,{deadline})?;'
                if case.get("read_before"):
                    body += f'let ignored:Bytes=runtime.file_read_some(file,{case["read_before"]},{deadline})?;'
                if operation == "two_file_reads":
                    body += f'let second:Out=runtime.file_read_some(file,{maximum},{deadline});return Ok(tuple(first,second));'
                else:
                    body += f'return runtime.file_read_some(file,{maximum},{deadline});'
    result_type = "Failure" if operation == "question_error_cleanup" else "Out"
    declarations += f'@id("Out") type Out=Result<{success},core.IoError>;'
    if operation in {"two_file_reads", "two_file_writes"}:
        declarations += f'@id("Pair") type Pair=Tuple<{success},Out>;@id("Combined") type Combined=Result<Pair,core.IoError>;'
        result_type = "Combined"
    capability = f'@id("{grant["entity_id"]}") capability selected:{grant["kind"]} scope {json.dumps(grant["scope"])};'
    source = (CORE_DECLARATIONS + '@id("app") module app imports ["core"] {' + declarations + capability
              + f'@id("main") fn main({parameters})->{result_type} effects [fs,alloc] capabilities ["{grant["entity_id"]}"] {{'
              + body + '}}')
    arguments = [{"type": "String", "data": {"kind": "string", "value": path}}]
    if writes:
        arguments.append({"type": "Bytes", "data": {"kind": "bytes", "value": contract()["fixture"]["write_bytes"]}})
    return source, arguments, [grant_name]


class HostFixtures:
    """Independent host state, recreated before each backend execution."""

    def __init__(self, directory):
        self.root = Path(directory).resolve()
        self.read_root = self.root / "read"
        self.write_root = self.root / "write"
        self.other_read_root = self.root / "other-read"
        self.outside_root = self.root / "outside"
        self.definition = contract()["fixture"]
        for path in (self.read_root, self.write_root, self.other_read_root, self.outside_root):
            path.mkdir(parents=True, exist_ok=False)
        for directory in (self.read_root, self.write_root):
            (directory / self.definition["symlink"]).symlink_to(self.outside_root, target_is_directory=True)
        self.reset()

    def reset(self):
        data = self.definition
        (self.read_root / data["read_file"]).write_bytes(bytes(data["read_bytes"]))
        (self.other_read_root / data["read_file"]).write_bytes(b"wrong grant directory")
        (self.write_root / data["write_file"]).write_bytes(bytes(data["existing_bytes"]))
        (self.outside_root / data["outside_file"]).write_bytes(bytes(data["outside_bytes"]))

    def grant(self, identity):
        rows = {
            "read": ("read_grant", "FileRead", str(self.read_root)),
            "write": ("write_grant", "FileWrite", str(self.write_root)),
            "other_read": ("other_read_grant", "FileRead", str(self.other_read_root)),
            "clock": ("clock_grant", "ClockRead", None),
        }
        entity_id, kind, scope = rows[identity]
        return {"entity_id": entity_id, "kind": kind, "scope": scope}

    def policy(self, names=(), faults=None):
        return {"schema_version": "1.0.0", "grants": [self.grant(name) for name in names],
                "test_faults": faults}

    def write_policy(self, names=(), faults=None, filename="host-policy.json"):
        return write_json(self.root / filename, self.policy(names, faults))

    def assert_outside_unchanged(self):
        actual = (self.outside_root / self.definition["outside_file"]).read_bytes()
        expected = bytes(self.definition["outside_bytes"])
        if actual != expected:
            raise AssertionError(f"grant escape modified outside sentinel: {actual!r}")

    def written_bytes(self):
        return (self.write_root / self.definition["write_file"]).read_bytes()


@contextmanager
def input_pipe(*, nonblocking, data=b"", writer_open=True):
    """Keep an empty pipe distinct from EOF without starting a service."""
    read_fd, write_fd = os.pipe()
    try:
        os.set_blocking(read_fd, not nonblocking)
        if data:
            count = os.write(write_fd, data)
            if count != len(data):
                raise AssertionError("small fixture pipe was only partially initialized")
        if not writer_open:
            os.close(write_fd)
            write_fd = None
        yield read_fd
    finally:
        os.close(read_fd)
        if write_fd is not None:
            os.close(write_fd)


def run_native(executable, directory, *, policy_path=None, captured=True,
               stdin_fd=None, stdout_fd=None, timeout=10):
    """Launch only the artifact, with explicit fd 3/4 and no ambient environment."""
    import fcntl

    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    report_path = directory / "execution-fd3.json"
    handles = []
    inherited = []
    try:
        report_fd = None
        policy_fd = None
        if captured:
            handle = report_path.open("xb")
            handles.append(handle)
            report_fd = fcntl.fcntl(handle.fileno(), fcntl.F_DUPFD_CLOEXEC, 10)
            inherited.append(report_fd)
        if policy_path is not None:
            handle = Path(policy_path).open("rb")
            handles.append(handle)
            policy_fd = fcntl.fcntl(handle.fileno(), fcntl.F_DUPFD_CLOEXEC, 10)
            inherited.append(policy_fd)

        def descriptors():
            # Sources are >= 10, so fd 3/4 mappings cannot clobber each other.
            for source, target in ((report_fd, 3), (policy_fd, 4)):
                if source is None:
                    try:
                        os.close(target)
                    except OSError:
                        pass
                else:
                    os.dup2(source, target)
                    os.set_inheritable(target, True)

        started = time.monotonic_ns()
        argv = [str(Path(executable).resolve())]
        try:
            completed = subprocess.run(argv, cwd=directory, env={},
                stdin=subprocess.DEVNULL if stdin_fd is None else stdin_fd,
                stdout=subprocess.PIPE if stdout_fd is None else stdout_fd, stderr=subprocess.PIPE,
                preexec_fn=descriptors, close_fds=False, restore_signals=True, timeout=timeout)
        except subprocess.TimeoutExpired as failure:
            write_json(directory / "process-receipt.json", {"argv": argv, "status": "TIMEOUT",
                "elapsed_ns": time.monotonic_ns() - started,
                "stdout": list(failure.stdout or b""), "stderr": list(failure.stderr or b"")})
            raise
        elapsed_ns = time.monotonic_ns() - started
        receipt = {"argv": argv, "executable_sha256": sha256(executable),
            "policy_sha256": sha256(policy_path) if policy_path is not None else None,
            "exit_code": completed.returncode, "elapsed_ns": elapsed_ns,
            "stdout": list(completed.stdout) if completed.stdout is not None else None,
            "stderr": list(completed.stderr), "execution": None}
        write_json(directory / "process-receipt.json", receipt)
        report = None
        if captured:
            if report_path.stat().st_size > 8 * 1024 * 1024:
                raise AssertionError("native report exceeds acceptance limit")
            report = json.loads(report_path.read_text(encoding="utf-8"))
            if stdout_fd is None and completed.stdout != bytes(report["stdout"]):
                raise AssertionError("native report misstates actual stdout")
            if completed.stderr != bytes(report["stderr"]):
                raise AssertionError("native report misstates actual stderr")
        receipt["execution"] = report
        write_json(directory / "process-receipt.json", receipt)
        return receipt
    finally:
        for descriptor in inherited:
            os.close(descriptor)
        for handle in handles:
            handle.close()


def inspect_elf(path, directory):
    """Retain actual nm/readelf output independently of compiler build evidence."""
    path = Path(path).resolve()
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    header = path.read_bytes()[:64]
    if len(header) != 64 or header[:7] != b"\x7fELF\x02\x01\x01":
        raise AssertionError("runtime profile did not emit a little-endian ELF64 artifact")
    elf_type, machine = struct.unpack_from("<HH", header, 16)
    if machine != 62:
        raise AssertionError("P06 artifact is not x86-64")
    records = {}
    commands = {
        "program_headers": ["/usr/bin/readelf", "-lW", str(path)],
        "dynamic": ["/usr/bin/readelf", "-dW", str(path)],
        "symbols": ["/usr/bin/nm", "-g", str(path)],
        "undefined": ["/usr/bin/nm", "-u", str(path)],
    }
    for label, command in commands.items():
        completed = subprocess.run(command, capture_output=True, text=True, timeout=30)
        row = {"argv": command, "tool_sha256": sha256(command[0]), "exit_code": completed.returncode,
               "stdout": completed.stdout, "stderr": completed.stderr}
        write_json(directory / (label + ".json"), row)
        if completed.returncode:
            raise AssertionError(f"{label} inspection failed: {completed.stderr}")
        records[label] = row
    records["artifact"] = {"path": str(path), "sha256": sha256(path), "elf_type": elf_type, "machine": machine}
    write_json(directory / "inspection.json", records)
    return records


def assert_reduced_dependencies(inspection, runtime_profile):
    expected = contract()["profiles"][runtime_profile]
    if expected["elf_type"] == "relocatable":
        if inspection["artifact"]["elf_type"] != 1:
            raise AssertionError("None profile must emit an ELF relocatable object")
    elif inspection["artifact"]["elf_type"] != 2:
        raise AssertionError("Minimal profile must emit a static ELF executable")
    if "INTERP" in inspection["program_headers"]["stdout"]:
        raise AssertionError("reduced profile depends on a dynamic loader")
    if "(NEEDED)" in inspection["dynamic"]["stdout"]:
        raise AssertionError("reduced profile has a dynamic dependency")
    if inspection["undefined"]["stdout"].strip():
        raise AssertionError("reduced profile contains undefined symbols")
    for fragment in expected.get("forbidden_symbol_fragments", []):
        if fragment in inspection["symbols"]["stdout"]:
            raise AssertionError(f"reduced profile retained forbidden dependency {fragment}")


def link_scalar_export_harness(object_path, directory, *, overflow=False):
    """An independent LLVM host calls the freestanding scalar ABI directly."""
    expected = contract()["profiles"]["none"]
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    source = directory / "host.ll"
    executable = directory / "host.elf"
    symbol = expected["export_symbol"]
    arguments = [9223372036854775807, 1] if overflow else expected["arguments"]
    source.write_text(
        'target triple = "x86_64-unknown-linux-gnu"\n'
        f'declare i64 @{symbol}(i64, i64)\n'
        'define i32 @main() {\nentry:\n'
        f'  %answer = call i64 @{symbol}(i64 {arguments[0]}, i64 {arguments[1]})\n'
        f'  %ok = icmp eq i64 %answer, {expected["return_integer"]}\n'
        '  %status = select i1 %ok, i32 0, i32 1\n'
        '  ret i32 %status\n}\n', encoding="utf-8")
    command = ["/usr/bin/clang-14", "--target=x86_64-unknown-linux-gnu", str(source.resolve()),
               str(Path(object_path).resolve()), "-Wl,--build-id=none", "-o", str(executable.resolve())]
    completed = subprocess.run(command, capture_output=True, text=True, timeout=30)
    write_json(directory / "host-link.json", {"argv": command, "tool_sha256": sha256(command[0]),
        "source_sha256": sha256(source), "input_object_sha256": sha256(object_path),
        "exit_code": completed.returncode, "stdout": completed.stdout, "stderr": completed.stderr})
    if completed.returncode:
        raise AssertionError("freestanding object failed independent host linkage: " + completed.stderr)
    receipt = run_native(executable, directory / "execution", captured=False)
    if overflow:
        if receipt["exit_code"] not in (-4, -5):
            raise AssertionError("None profile overflow did not deliver a machine trap")
    elif receipt["exit_code"] != 0:
        raise AssertionError("None profile export returned the wrong scalar ABI result")
    return receipt


class RuntimeCliAcceptance(unittest.TestCase):
    git = graph_cli.GraphCliAcceptance.git
    head_revision = graph_cli.GraphCliAcceptance.head_revision
    assert_code = graph_cli.GraphCliAcceptance.assert_code
    records = []

    def setUp(self):
        graph_cli.GraphCliAcceptance.setUp(self)
        self.host = HostFixtures(self.root / "host")
        self.policy_path = None

    def invoke(self, command, request, expect_ok=True):
        arguments = [str(graph_cli.BINARY), "--repository", str(self.repository), "--store", str(self.store)]
        if self.policy_path is not None:
            arguments.extend(["--host-policy", str(self.policy_path)])
        arguments.append(command)
        completed = subprocess.run(arguments, input=json.dumps(request), capture_output=True,
                                   text=True, encoding="utf-8", timeout=180)
        response = json.loads(completed.stdout)
        self.assertEqual(set(response), graph_cli.ENVELOPE)
        self.assertEqual(response["tool"], command)
        self.assertIs(response["ok"], expect_ok, response)
        self.assertEqual(completed.returncode == 0, expect_ok, response)
        if not expect_ok:
            self.assertTrue(response["diagnostics"], response)
        return response

    def publish(self, source, expect_ok=True):
        return self.invoke("transact", {"task_id": "P06-runtime", "base_revision": 0, "scope": ["program"],
            "operations": [{"op": "import_text", "source": source}], "required_checks": []}, expect_ok)

    def execute_request(self, arguments=None, limits=None, isolation="captured"):
        return {"revision": 1, "isolation": isolation, "suite": {"entry": "main",
            "arguments": arguments or [], "limits": dict(execution_cli.LIMITS, **(limits or {}))}}

    @contextmanager
    def scenario(self, identity):
        previous_store = self.store
        self.store = self.root / ("store-" + identity)
        row = {"id": identity, "passed": False}
        self.records.append(row)
        self.progress()
        try:
            yield row
            row["passed"] = True
        except BaseException:
            if FAILURE_ROOT is not None:
                import shutil
                destination = FAILURE_ROOT / identity
                shutil.copytree(self.root, destination, dirs_exist_ok=True)
                row["retained_directory"] = str(destination)
            raise
        finally:
            self.store = previous_store
            self.progress()

    def progress(self):
        if REPORT_PATH is not None:
            write_json(REPORT_PATH, {"suite": "runtime_cli_blackbox", "state": "RUNNING", "passed": False,
                                    "contract_sha256": sha256(CONTRACT_PATH), "cases": self.records})

    def validate_build(self, response, runtime_profile, optimization):
        native = response["result"]["native"]
        self.assertEqual(native["runtime_profile"], runtime_profile)
        self.assertEqual(native["profile"], optimization)
        self.assertEqual(native["target"], graph_cli.TARGET)
        self.assertEqual(set(native["artifacts"]), {"native_ir", "llvm_ir", "object", "executable"})
        for kind, artifact in native["artifacts"].items():
            if kind == "executable" and runtime_profile == "none":
                self.assertIsNone(artifact)
                continue
            self.assertEqual(set(artifact), {"path", "sha256"})
            self.assertTrue(Path(artifact["path"]).is_absolute())
            self.assertEqual(artifact["sha256"], sha256(artifact["path"]))
        record_artifact = native["build_record"]
        self.assertEqual(record_artifact["sha256"], sha256(record_artifact["path"]))
        record = json.loads(Path(record_artifact["path"]).read_text(encoding="utf-8"))
        execution_cli.SCHEMA_CHECK.validate_file(ROOT, record, "native_build")
        self.assertEqual(record["input"]["runtime_profile"], runtime_profile)
        self.assertEqual(record["artifacts"], native["artifacts"])
        self.assertEqual(record["input"]["compiler_hash"], sha256(graph_cli.BINARY))
        policy = self.host.policy() if self.policy_path is None else json.loads(self.policy_path.read_text(encoding="utf-8"))
        self.assertEqual(record["input"]["policy_hash"], policy_hash(policy))
        if runtime_profile == "none":
            self.assertIsNone(record["input"]["runtime_hash"])
        else:
            archive = "libil_native_runtime.a" if runtime_profile == "full" else "libil_minimal_runtime.a"
            self.assertEqual(record["input"]["runtime_hash"], sha256(graph_cli.BINARY.with_name(archive)))
        native_ir = json.loads(Path(native["artifacts"]["native_ir"]["path"]).read_text(encoding="utf-8"))
        execution_cli.SCHEMA_CHECK.validate_file(ROOT, native_ir, "native_ir")
        self.assertEqual(native_ir["runtime_profile"], runtime_profile)
        return native

    def build_request(self, runtime_profile, optimization):
        request = {"revision": 1, "target": graph_cli.TARGET, "profile": optimization,
                   "runtime_profile": runtime_profile}
        if runtime_profile == "none":
            request["exports"] = [contract()["profiles"]["none"]["export_entity"]]
        return request

    def check_host_execution(self, execution, expected):
        execution_cli.SCHEMA_CHECK.validate_file(ROOT, execution, "execution")
        self.assertEqual(execution["status"], expected.get("status", "returned"))
        self.assertEqual(execution["live_handles"], expected.get("live_handles", 0))
        self.assertEqual(execution["stdout"], expected.get("stdout", []))
        self.assertEqual(execution["stderr"], [])
        if execution["status"] == "returned":
            self.assertEqual(execution["diagnostics"], [])
            self.assertEqual(execution["stack_trace"], [])
            self.assertEqual(execution["live_allocations"], returned_buffer_owners(execution["value"]),
                             "successful execution leaked an unreturned String or Bytes owner")
            result = execution["value"]["data"]["value"]
            if "first_bytes" in expected or "first_count" in expected:
                self.assertEqual(result["tag"], "Ok")
                pair = result["fields"][0]
                self.assertEqual(pair["type"], "Pair")
                self.assertEqual(pair["data"]["kind"], "tuple")
                first, second = pair["data"]["value"]
                if "first_bytes" in expected:
                    self.assertEqual(first, {"type": "Bytes", "data": {"kind": "bytes", "value": expected["first_bytes"]}})
                else:
                    self.assertEqual(first, {"type": "Usize", "data": {"kind": "integer", "value": str(expected["first_count"])}})
                result = second["data"]["value"]
            expected_tag = expected.get("tag", "Ok")
            self.assertEqual(result["tag"], expected_tag)
            payload = result["fields"][0]
            if "error" in expected:
                self.assertEqual(payload["type"], "core.IoError")
                self.assertEqual(payload["data"], {"kind": "variant", "value": {"tag": expected["error"], "fields": []}})
            elif "bytes" in expected:
                self.assertEqual(payload, {"type": "Bytes", "data": {"kind": "bytes", "value": expected["bytes"]}})
            elif "count" in expected or "return_integer" in expected or "error_integer" in expected:
                count = expected.get("count", expected.get("return_integer", expected.get("error_integer")))
                self.assertEqual(payload["data"], {"kind": "integer", "value": str(count)})
            elif expected.get("unit"):
                self.assertEqual(payload, {"type": "Unit", "data": {"kind": "unit"}})
            elif "resource_kind" in expected:
                self.assertEqual(payload["type"], "core.File")
                self.assertEqual(payload["data"]["kind"], "resource")
                resource = payload["data"]["value"]
                self.assertEqual(set(resource), {"kind", "slot", "generation"})
                self.assertEqual(resource["kind"], expected["resource_kind"])
                self.assertGreaterEqual(resource["slot"], 0)
                self.assertGreaterEqual(resource["generation"], 0)
        if "file_bytes" in expected:
            self.assertEqual(list(self.host.written_bytes()), expected["file_bytes"])
        self.host.assert_outside_unchanged()
        for event in execution["handle_events"]:
            self.assertEqual(set(event), {"kind", "entity_id", "slot", "generation", "error"})
            self.assertIn(event["kind"], ("opened", "closed", "dropped"))
            self.assertTrue(event["entity_id"])

    def compare_host_case(self, case, row):
        source, arguments, grants = file_source(self.host, case)
        self.policy_path = self.host.write_policy(grants, case.get("faults"))
        self.publish(source)
        row["executions"] = {}
        reference = None
        for isolation in ("captured", "native_debug", "native_release"):
            self.host.reset()
            response = self.invoke("test", self.execute_request(arguments, isolation=isolation))
            execution = response["result"]["execution"]
            self.check_host_execution(execution, case["expected"])
            observable = {key: execution[key] for key in ("status", "value", "stdout", "stderr", "live_handles")}
            if reference is None:
                reference = observable
            else:
                self.assertEqual(observable, reference)
                native = self.validate_build(response, "full", isolation.removeprefix("native_"))
                self.host.reset()
                receipt = run_native(native["artifacts"]["executable"]["path"], self.store / ("direct-" + isolation),
                                     policy_path=self.policy_path)
                self.assertEqual(receipt["exit_code"], 0)
                self.check_host_execution(receipt["execution"], case["expected"])
                self.assertEqual({key: receipt["execution"][key] for key in observable}, observable)
            row["executions"][isolation] = observable

    def test_real_files_handles_and_cleanup(self):
        for case in contract()["io_cases"]:
            with self.subTest(case=case["id"]), self.scenario(case["id"]) as row:
                self.compare_host_case(case, row)

    def test_directory_containment_uses_selected_grant(self):
        for locked in contract()["containment_cases"]:
            case = {"id": locked["id"], "operation": "file_write" if locked["grant"] == "write" else "file_read",
                    "path": locked.get("relative_path", str(self.host.outside_root / "sentinel.bin")),
                    "expected": {"tag": "Err", "error": locked["expected_error"], "live_handles": 0}}
            with self.subTest(case=case["id"]), self.scenario(case["id"]) as row:
                self.compare_host_case(case, row)

    def test_file_chunk_and_io_faults(self):
        for case in contract()["fault_cases"]:
            if case["operation"] not in {"file_read_some", "file_write_some", "two_file_reads", "two_file_writes"}:
                continue
            with self.subTest(case=case["id"]), self.scenario(case["id"]) as row:
                self.compare_host_case(case, row)

    def test_grants_and_selectors_cannot_be_forged(self):
        source, _, _ = file_source(self.host, contract()["io_cases"][0])
        policies = {
            "missing_selected_grant": self.host.policy(),
            "same_kind_wrong_grant": self.host.policy(["other_read"]),
            "selected_grant_scope_mismatch": {"schema_version": "1.0.0", "test_faults": None,
                "grants": [dict(self.host.grant("read"), scope=str(self.host.other_read_root))]},
        }
        for identity, policy in policies.items():
            with self.subTest(case=identity), self.scenario(identity):
                self.policy_path = write_json(self.host.root / (identity + ".json"), policy)
                self.publish(source, False)
                self.assertEqual(self.head_revision(), 0)
                self.host.assert_outside_unchanged()
        with self.scenario("missing_capability_selector"):
            self.policy_path = self.host.write_policy(["read"])
            self.publish(source.replace("runtime.file_read[read_grant]", "runtime.file_read"), False)
            self.assertEqual(self.head_revision(), 0)
        with self.scenario("extra_capability_selector"):
            self.policy_path = None
            self.publish('module app {@id("main") fn main(text:String)->Usize {return runtime.string_len[anything](text);}}', False)
            self.assertEqual(self.head_revision(), 0)

    def test_host_policy_schema_and_request_authority_are_closed(self):
        empty = self.host.policy()
        duplicate = self.host.policy(["read"])
        duplicate["grants"].append(copy.deepcopy(duplicate["grants"][0]))
        malformed = {
            "policy_duplicate_grant_id": duplicate,
            "policy_unknown_field": dict(empty, command="forbidden"),
            "fault_zero_io_chunk": dict(empty, test_faults={"allocation_fail_after": None, "io_max_chunk": 0, "io_fail_after": None, "accept_fail_after": None}),
            "fault_missing_accept_field": dict(empty, test_faults={"allocation_fail_after": None, "io_max_chunk": 1, "io_fail_after": None}),
        }
        for identity, policy in malformed.items():
            with self.subTest(case=identity), self.scenario(identity):
                self.policy_path = write_json(self.host.root / (identity + ".json"), policy)
                self.invoke("state", {}, False)
        with self.scenario("policy_exceeds_64kib"):
            self.policy_path = self.host.root / "oversize-policy.json"
            self.policy_path.write_text(json.dumps(empty) + " " * 65536, encoding="utf-8")
            self.invoke("state", {}, False)
        with self.scenario("request_injects_authority"):
            self.policy_path = None
            self.publish('module app {@id("main") fn main()->I32 {return 0;}}')
            for field in ("host_policy", "policy", "test_faults", "grants"):
                request = self.execute_request()
                request[field] = empty
                self.invoke("test", request, False)
            self.assertEqual(self.head_revision(), 1)

    def test_language_stack_is_innermost_first(self):
        rules = contract()["stack"]
        self.policy_path = None
        with self.scenario("language_stack") as row:
            self.publish(rules["source"])
            row["executions"] = {}
            for isolation in ("captured", "native_debug", "native_release"):
                response = self.invoke("test", self.execute_request(isolation=isolation), False)
                execution = response["result"]["execution"]
                execution_cli.SCHEMA_CHECK.validate_file(ROOT, execution, "execution")
                self.assertEqual(execution["status"], rules["expected"]["status"])
                self.assertEqual(execution["diagnostics"][0]["code"], rules["expected"]["code"])
                self.assertEqual(execution["diagnostics"][0]["entity_id"], rules["expected"]["entity_id"])
                frames = execution["stack_trace"]
                self.assertEqual([frame["function_id"] for frame in frames], rules["expected"]["function_order"])
                self.assertEqual([frame["call_site"] for frame in frames[:-1]], rules["expected"]["call_sites"])
                self.assertIsNone(frames[-1]["call_site"])
                self.assertEqual(frames[0]["entity_id"], rules["expected"]["entity_id"])
                self.assertEqual(execution["live_handles"], 0)
                row["executions"][isolation] = frames

    def test_long_recursive_call_ids_keep_a_bounded_stack_report(self):
        call_id = "recursive_" + "x" * 20000
        source = f'module app {{@id("main") fn main()->I64 {{@id("{call_id}") let n:I64=main();return n;}}}}'
        with self.scenario("long_recursive_stack_budget") as row:
            self.publish(source)
            row["executions"] = {}
            for isolation in ("captured", "native_debug", "native_release"):
                before = set(self.store.glob(".il/builds/candidates/native-*"))
                response = self.invoke("test", self.execute_request(isolation=isolation), False)
                self.assert_code(response, "E_CONTEXT_INSUFFICIENT")
                if isolation == "captured":
                    row["executions"][isolation] = {"protocol_code": "E_CONTEXT_INSUFFICIENT"}
                    continue
                candidates = set(self.store.glob(".il/builds/candidates/native-*")) - before
                self.assertEqual(len(candidates), 1)
                retained = json.loads((candidates.pop() / "test-result.json").read_text(encoding="utf-8"))
                artifact = retained["native"]["artifacts"]["executable"]
                self.assertEqual(artifact["sha256"], sha256(artifact["path"]))
                receipt = run_native(artifact["path"], self.store / isolation)
                self.assertEqual(receipt["exit_code"], 101)
                execution = receipt["execution"]
                self.assertEqual(execution["status"], "trapped")
                self.assertEqual(execution["diagnostics"][0]["code"], "E_RESOURCE_LIMIT")
                self.assertEqual(execution["diagnostics"][0]["cause"], "stack trace budget exhausted")
                frames = execution["stack_trace"]
                self.assertGreater(len(frames), 1)
                self.assertLess(len(frames), 128)
                self.assertLessEqual(len(json.dumps(frames, ensure_ascii=False, separators=(",", ":")).encode()), 1048576)
                self.assertIsNone(frames[-1]["call_site"])
                self.assertTrue(all(frame["call_site"] == call_id for frame in frames[:-1]))
                row["executions"][isolation] = {"frames": len(frames), "steps": execution["steps"]}

    def test_native_startup_rechecks_descriptor_policy(self):
        grant = self.host.grant("write")
        source = (CORE_DECLARATIONS + '@id("app") module app imports ["core"] {'
                  '@id("Open") type Open=Result<core.File,core.IoError>;'
                  f'@id("write_grant") capability selected:FileWrite scope {json.dumps(grant["scope"])};'
                  '@id("main") fn main()->I32 effects [alloc,fs] capabilities ["write_grant"] {'
                  'let opened:Open=runtime.file_open_write[write_grant]("output.bin");return 0;}}')
        with self.scenario("startup_policy_boundary") as row:
            self.policy_path = self.host.write_policy(["write"])
            self.publish(source)
            native = self.validate_build(self.invoke("build", self.build_request("full", "debug")), "full", "debug")
            executable = native["artifacts"]["executable"]["path"]
            malformed = self.host.root / "malformed-policy.json"
            malformed.write_text("{", encoding="utf-8")
            insufficient = self.host.write_policy([], filename="insufficient-policy.json")
            fault_policy = self.host.write_policy(["write"], {"allocation_fail_after": None,
                "io_max_chunk": 1, "io_fail_after": None, "accept_fail_after": 0}, filename="application-fault-policy.json")
            policies = {"missing": None, "malformed": malformed, "insufficient": insufficient, "faults": fault_policy}
            row["rejected"] = {}
            for name, path in policies.items():
                self.host.reset()
                receipt = run_native(executable, self.store / name, policy_path=path, captured=False)
                self.assertNotEqual(receipt["exit_code"], 0)
                self.assertEqual(receipt["stdout"], [])
                self.assertEqual(list(self.host.written_bytes()), contract()["fixture"]["existing_bytes"],
                                 "guest open/truncate ran before startup rejected authority")
                self.host.assert_outside_unchanged()
                row["rejected"][name] = receipt
            self.host.reset()
            accepted = run_native(executable, self.store / "accepted", policy_path=self.policy_path, captured=False)
            self.assertEqual(accepted["exit_code"], 0)
            self.assertEqual(self.host.written_bytes(), b"")
            row["accepted"] = accepted

    def test_resource_descriptors_cannot_be_captured_arguments(self):
        resource = {"type": "core.File", "data": {"kind": "resource", "value": {"kind": "file", "slot": 0, "generation": 1}}}
        cases = {
            "captured_resource_argument": ("core.File", "", resource),
            "nested_resource_argument": ("Owned", '@id("Owned") type Owned=Tuple<I64,core.File>;',
                {"type": "Owned", "data": {"kind": "tuple", "value": [execution_cli.integer("I64", 42), resource]}}),
        }
        self.policy_path = None
        for identity, (type_ref, declaration, argument) in cases.items():
            with self.subTest(case=identity), self.scenario(identity):
                source = (CORE_DECLARATIONS + '@id("app") module app imports ["core"] {' + declaration
                          + f'@id("main") fn main(value:{type_ref})->I32 effects [fs] {{return 0;}}}}')
                self.publish(source)
                for isolation in ("captured", "native_debug", "native_release"):
                    self.invoke("test", self.execute_request([argument], isolation=isolation), False)
                self.assertEqual(self.head_revision(), 1)

    def test_file_opaque_representation_and_consumption_are_enforced(self):
        case = next(item for item in contract()["io_cases"] if item["operation"] == "drop_at_return")
        source, _, grants = file_source(self.host, case)
        self.policy_path = self.host.write_policy(grants)
        with self.scenario("file_use_after_close"):
            invalid = source.replace('return Ok(42);', 'runtime.file_close(file);runtime.file_close(file);return Ok(42);')
            self.publish(invalid, False)
            self.assertEqual(self.head_revision(), 0)
        with self.scenario("opaque_file_graph_rejections"):
            self.publish(source)
            graph = self.invoke("inspect", {"entity_id": "program", "budget": 65536})["result"]["entity"]
            function = next(function for function in graph["functions"] if function["entity_id"] == "main")
            producer = next((block["entity_id"], index, output["entity_id"])
                for block in function["blocks"] for index, operation in enumerate(block["operations"])
                for output in operation["outputs"] if output["type"] == "core.File")
            for operation_kind in ("clone", "field", "record"):
                with self.subTest(operation=operation_kind):
                    candidate = copy.deepcopy(graph)
                    target = next(block for item in candidate["functions"] if item["entity_id"] == "main"
                                  for block in item["blocks"] if block["entity_id"] == producer[0])
                    if operation_kind == "clone":
                        extra = execution_cli.operation("forged.clone", "clone", [producer[2]],
                            [{"entity_id": "forged.owner", "type": "core.File"}], {})
                        extra["effects"] = ["alloc"]
                        extra["produces"] = list(extra["outputs"])
                        additions = [extra]
                    elif operation_kind == "field":
                        additions = [execution_cli.operation("forged.field", "field", [producer[2]],
                            [{"entity_id": "forged.slot", "type": "U64"}], {"field": "slot"})]
                    else:
                        zero = execution_cli.operation("forged.zero", "const", [],
                            [{"entity_id": "forged.zero.value", "type": "U64"}], {"value": 0})
                        extra = execution_cli.operation("forged.record", "record", ["forged.zero.value", "forged.zero.value"],
                            [{"entity_id": "forged.owner", "type": "core.File"}], {"type_id": "core.File"})
                        extra["produces"] = list(extra["outputs"])
                        additions = [zero, extra]
                    target["operations"][producer[1] + 1:producer[1] + 1] = additions
                    self.invoke("transact", {"task_id": "P06-reject-opaque-" + operation_kind, "base_revision": 1,
                        "scope": ["program"], "operations": [{"op": "replace_program", "graph": candidate}],
                        "required_checks": []}, False)
                    self.assertEqual(self.head_revision(), 1)

    def test_real_nonblocking_stream_deadlines(self):
        for case in contract()["stream_cases"]:
            if case.get("execution_owner") == "runtime_handles_module":
                # The module's isolated process installs a real non-SA_RESTART
                # handler; no signal-control language API exists or is invented.
                continue
            with self.subTest(case=case["id"]), self.scenario(case["id"]) as row:
                self.policy_path = self.host.write_policy(["clock"])
                source = (CORE_DECLARATIONS + '@id("app") module app imports ["core"] {'
                    '@id("Out") type Out=Result<Bytes,core.IoError>;'
                    '@id("clock_grant") capability clock:ClockRead scope null;'
                    '@id("main") fn main()->Out effects [clock,process,alloc] capabilities ["clock_grant"] {'
                    'let now:U64=runtime.clock_now[clock_grant]();'
                    f'let until:U64=now+{case["deadline_after_ms"] * 1_000_000};'
                    'let deadline:core.Deadline=core.Deadline::At(until);return runtime.stdin_read(16,deadline);}}')
                self.publish(source)
                row["profiles"] = {}
                for optimization in ("debug", "release"):
                    # CLI fd 0 is its JSON protocol stream. The independent run
                    # below supplies the test stream on fd 0 after compilation.
                    response = self.invoke("test", self.execute_request(isolation="native_" + optimization))
                    native = self.validate_build(response, "full", optimization)
                    with input_pipe(nonblocking=case["nonblocking"], data=bytes(case.get("stdin", [])),
                                    writer_open=case["writer_open"]) as stream:
                        receipt = run_native(native["artifacts"]["executable"]["path"], self.store / optimization,
                            policy_path=self.policy_path, stdin_fd=stream)
                    self.assertEqual(receipt["exit_code"], 0)
                    self.check_host_execution(receipt["execution"], case["expected"])
                    elapsed_ms = receipt["elapsed_ns"] / 1_000_000
                    if "elapsed_min_ms" in case["expected"]:
                        self.assertGreaterEqual(elapsed_ms, case["expected"]["elapsed_min_ms"])
                    if "elapsed_max_ms" in case["expected"]:
                        self.assertLessEqual(elapsed_ms, case["expected"]["elapsed_max_ms"])
                    row["profiles"][optimization] = receipt

    def test_complete_print_reports_only_actual_partial_bytes(self):
        case = next(item for item in contract()["fault_cases"] if item["operation"] == "print_i64")
        with self.scenario(case["id"]) as row:
            self.policy_path = self.host.write_policy([], case["faults"])
            source = (CORE_DECLARATIONS + '@id("app") module app imports ["core"] {'
                '@id("Out") type Out=Result<Unit,core.IoError>;'
                f'@id("main") fn main()->Out effects [process] {{return runtime.print_i64({case["integer"]});}}}}')
            self.publish(source)
            row["executions"] = {}
            for isolation in ("captured", "native_debug", "native_release"):
                response = self.invoke("test", self.execute_request(isolation=isolation))
                execution = response["result"]["execution"]
                self.check_host_execution(execution, case["expected"])
                row["executions"][isolation] = execution
                if isolation.startswith("native_"):
                    native = self.validate_build(response, "full", isolation.removeprefix("native_"))
                    receipt = run_native(native["artifacts"]["executable"]["path"], self.store / isolation,
                                         policy_path=self.policy_path)
                    self.assertEqual(receipt["exit_code"], 0)
                    self.check_host_execution(receipt["execution"], case["expected"])

    def test_captured_stream_finite_deadline_matches_blocking_pipe(self):
        source = (CORE_DECLARATIONS + '@id("app") module app imports ["core"] {'
                  '@id("Out") type Out=Result<Usize,core.IoError>;'
                  '@id("main") fn main(data:Bytes)->Out effects [process] {'
                  'return runtime.stdout_write(data,0,core.Deadline::At(18446744073709551615));}}')
        with self.scenario("captured_stdout_finite_deadline") as row:
            self.publish(source)
            row["executions"] = {}
            for isolation in ("captured", "native_debug", "native_release"):
                response = self.invoke("test", self.execute_request(
                    [{"type": "Bytes", "data": {"kind": "bytes", "value": [65, 0, 255]}}], isolation=isolation))
                execution = response["result"]["execution"]
                self.check_host_execution(execution, {"tag": "Err", "error": "InvalidData", "stdout": [], "live_handles": 0})
                row["executions"][isolation] = execution

    def test_read_allocation_failure_is_explicit_io_error(self):
        case = next(item for item in contract()["fault_cases"] if item["operation"] == "stdin_read")
        with self.scenario(case["id"]) as row:
            self.policy_path = self.host.write_policy([], case["faults"])
            source = (CORE_DECLARATIONS + '@id("app") module app imports ["core"] {'
                '@id("Out") type Out=Result<Bytes,core.IoError>;'
                f'@id("main") fn main()->Out effects [process,alloc] {{return runtime.stdin_read({case["maximum"]},core.Deadline::Infinite());}}}}')
            self.publish(source)
            row["profiles"] = {}
            captured = self.invoke("test", self.execute_request(isolation="captured"))["result"]["execution"]
            self.check_host_execution(captured, case["expected"])
            row["captured"] = captured
            for optimization in ("debug", "release"):
                response = self.invoke("test", self.execute_request(isolation="native_" + optimization))
                native = self.validate_build(response, "full", optimization)
                with input_pipe(nonblocking=True, data=bytes(case["stdin"]), writer_open=False) as stream:
                    receipt = run_native(native["artifacts"]["executable"]["path"], self.store / optimization,
                                         policy_path=self.policy_path, stdin_fd=stream)
                self.assertEqual(receipt["exit_code"], 0)
                self.check_host_execution(receipt["execution"], case["expected"])
                row["profiles"][optimization] = receipt

    def test_profiles_execute_and_clean_builds_reproduce(self):
        rules = contract()
        for runtime_profile in ("full", "minimal", "none"):
            source_key = "none_add" if runtime_profile == "none" else "minimal_output"
            hashes = {}
            for clean in range(2):
                identity = f"{runtime_profile}_clean_{clean}"
                with self.subTest(profile=runtime_profile, clean=clean), self.scenario(identity) as row:
                    self.publish(rules["profile_sources"][source_key])
                    row["optimizations"] = {}
                    for optimization in ("debug", "release"):
                        response = self.invoke("build", self.build_request(runtime_profile, optimization))
                        native = self.validate_build(response, runtime_profile, optimization)
                        artifact = native["artifacts"]["object" if runtime_profile == "none" else "executable"]
                        evidence = self.store / ("independent-" + optimization)
                        inspection = inspect_elf(artifact["path"], evidence / "inspection")
                        if runtime_profile != "full":
                            assert_reduced_dependencies(inspection, runtime_profile)
                        if runtime_profile == "none":
                            expected_symbol = rules["profiles"]["none"]["export_symbol"]
                            symbols = [line.split()[-1] for line in inspection["symbols"]["stdout"].splitlines() if line.split()]
                            self.assertIn(expected_symbol, symbols)
                            receipt = link_scalar_export_harness(artifact["path"], evidence / "host")
                            link_scalar_export_harness(artifact["path"], evidence / "overflow-host", overflow=True)
                        else:
                            receipt = run_native(artifact["path"], evidence / "application", captured=False)
                            self.assertEqual(receipt["exit_code"], 0)
                            self.assertEqual(bytes(receipt["stdout"]), b"42")
                            self.assertEqual(receipt["stderr"], [])
                        current = {name: value["sha256"] if value else None for name, value in native["artifacts"].items()}
                        key = (runtime_profile, optimization)
                        if clean == 0:
                            hashes[key] = current
                        else:
                            self.assertEqual(current, hashes[key], "clean store changed profile artifact hash")
                        row["optimizations"][optimization] = {"hashes": current, "exit_code": receipt["exit_code"]}

    def test_minimal_trap_has_exit_101(self):
        with self.scenario("minimal_trap") as row:
            self.publish(contract()["profile_sources"]["minimal_trap"])
            row["profiles"] = {}
            for optimization in ("debug", "release"):
                response = self.invoke("build", self.build_request("minimal", optimization))
                native = self.validate_build(response, "minimal", optimization)
                receipt = run_native(native["artifacts"]["executable"]["path"],
                                     self.store / optimization, captured=False)
                self.assertEqual(receipt["exit_code"], 101)
                self.assertEqual(receipt["stdout"], [])
                row["profiles"][optimization] = receipt

    def test_build_profile_requests_are_closed(self):
        with self.scenario("profile_request_validation"):
            self.publish(contract()["profile_sources"]["none_add"])
            request = self.build_request("none", "debug")
            cases = [dict(request, runtime_profile="unknown"), dict(request, exports=[]),
                     dict(request, exports=["math.add", "math.add"]), dict(request, runtime_profile="full"),
                     dict(request, runtime_profile="minimal"), dict(request, policy={}),
                     dict(request, command="forbidden")]
            no_exports = dict(request)
            del no_exports["exports"]
            cases.append(no_exports)
            for rejected in cases:
                with self.subTest(request=rejected):
                    self.invoke("build", rejected, False)
            self.assertEqual(self.head_revision(), 1)

    def test_reduced_profiles_reject_unavailable_features_in_all_functions(self):
        sources = {
            "managed_string": 'module app visibility public {@id("main") fn main()->I32 effects [alloc] {let text:String="owned";return 0;}}',
            "managed_bytes": 'module app visibility public {@id("main") fn main(data:Bytes)->I32 {return 0;}}',
            "unused_managed_function": 'module app visibility public {@id("main") fn main()->I32 {return 0;} fn unused()->String effects [alloc] {return "owned";}}',
            "fs_effect": 'module app visibility public {@id("main") fn main()->I32 effects [fs] {return 0;}}',
            "net_effect": 'module app visibility public {@id("main") fn main()->I32 effects [net] {return 0;}}',
            "clock_effect": 'module app visibility public {@id("main") fn main()->I32 effects [clock] {return 0;}}',
        }
        for label, source in sources.items():
            with self.subTest(feature=label), self.scenario("reduced_reject_" + label):
                self.publish(source)
                for runtime_profile in ("minimal", "none"):
                    request = self.build_request(runtime_profile, "debug")
                    if runtime_profile == "none":
                        request["exports"] = ["main"]
                    response = self.invoke("build", request, False)
                    self.assertTrue(response["diagnostics"])
                self.assertEqual(self.head_revision(), 1)

    def test_none_rejects_nonpublic_or_aggregate_exports(self):
        cases = {
            "private": 'module app {@id("main") fn main()->I32 {return 0;}}',
            "aggregate_argument": 'module app visibility public {@id("Pair") type Pair=Tuple<I64,I64>; @id("main") fn main(value:Pair)->I32 {return 0;}}',
            "aggregate_result": 'module app visibility public {@id("Pair") type Pair=Tuple<I64,I64>; @id("main") fn main()->Pair {return tuple(20,22);}}',
            "host_output": 'module app visibility public {@id("main") fn main()->I32 effects [process] {runtime.print_i64(42);return 0;}}',
        }
        for label, source in cases.items():
            with self.subTest(feature=label), self.scenario("none_export_reject_" + label):
                self.publish(source)
                request = self.build_request("none", "debug")
                request["exports"] = ["main"]
                self.invoke("build", request, False)
                request["exports"] = ["undeclared"]
                self.invoke("build", request, False)
                self.assertEqual(self.head_revision(), 1)


def main():
    global REPORT_PATH, FAILURE_ROOT
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    graph_cli.BINARY = args.binary.resolve()
    REPORT_PATH = args.report.resolve()
    FAILURE_ROOT = REPORT_PATH.parent / (REPORT_PATH.stem + "-failed-artifacts")
    report = {"suite": "runtime_cli_blackbox", "state": "RUNNING", "passed": False,
              "contract_sha256": sha256(CONTRACT_PATH), "cases": []}
    write_json(REPORT_PATH, report)
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(RuntimeCliAcceptance)
    result = unittest.TextTestRunner(verbosity=2, stream=sys.stderr).run(suite)
    report.update({"state": "PASSED" if result.wasSuccessful() else "FAILED", "passed": result.wasSuccessful(),
        "count": result.testsRun, "cases": RuntimeCliAcceptance.records,
        "failures": [{"test": str(test), "traceback": message} for test, message in result.failures],
        "errors": [{"test": str(test), "traceback": message} for test, message in result.errors]})
    write_json(REPORT_PATH, report)
    print(json.dumps({key: value for key, value in report.items() if key != "cases"}))
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    raise SystemExit(main())
