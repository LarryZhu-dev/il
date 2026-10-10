"""Execute the locked G4 workflow matrix; preserve raw observations, never publish a release."""
from __future__ import annotations

import argparse
import contextlib
import copy
import difflib
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import statistics
import subprocess
import sys
import time
import traceback
import uuid

ROOT = Path(__file__).resolve().parents[2]
CONTRACT_PATH = Path(__file__).with_name("execution_contract.json")
TARGET = "x86_64-unknown-linux-gnu"
G4_BASELINE_DATE = "2000-01-01T00:00:00+00:00"
MODULES = ("core", "alloc", "io", "time", "net", "json", "http", "test", "tracing")
ROUTE = {"entity_id": "demo.route.hello", "method": "GET", "path": "/hello/{name}",
         "handler": "demo.hello", "parameters": [{"name": "name", "type_ref": "String",
         "source": "path", "max_utf8_bytes": 128}]}


def encoded(value):
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode()


def digest(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    pending = path.with_suffix(path.suffix + ".pending")
    pending.write_bytes(encoded(value))
    pending.replace(path)


def require(condition, message):
    if not condition:
        raise AssertionError(message)


class Trace:
    def __init__(self, directory):
        self.directory = directory
        self.run_id = str(uuid.uuid4())
        self.workspace_id = str(uuid.uuid4())
        self.events = []
        self.phase = "setup"
        (directory / "blobs").mkdir(parents=True)

    def blob(self, data):
        identity = digest(data)
        path = self.directory / "blobs" / identity[7:]
        if not path.exists():
            path.write_bytes(data)
        return {"path": path.relative_to(self.directory).as_posix(), "sha256": identity, "bytes": len(data)}

    def record(self, kind, request, response, started):
        event = {"sequence": len(self.events) + 1, "phase": self.phase, "kind": kind,
                 "started_monotonic_ns": started, "finished_monotonic_ns": time.monotonic_ns(),
                 "request": request, "response": response}
        self.events.append(event)
        save(self.directory / "trace.json", self.events)

    def process(self, argv, *, cwd=None, stdin=b"", expected=0, env=None):
        argv = [str(arg) for arg in argv]
        started = time.monotonic_ns()
        process_env = os.environ.copy()
        process_env.update(env or {})
        try:
            result = subprocess.run(argv, cwd=cwd, input=stdin, capture_output=True, timeout=180, env=process_env)
        except subprocess.TimeoutExpired as error:
            self.record("process", {"argv": argv, "cwd": str(cwd), "stdin": self.blob(stdin), "environment": env or {}},
                        {"timeout": True, "stdout": self.blob(error.stdout or b""),
                         "stderr": self.blob(error.stderr or b"")}, started)
            raise
        self.record("process", {"argv": argv, "cwd": str(cwd), "stdin": self.blob(stdin), "environment": env or {}},
                    {"exit_code": result.returncode, "stdout": self.blob(result.stdout),
                     "stderr": self.blob(result.stderr)}, started)
        if expected is not None:
            require(result.returncode == expected, f"{argv[0]} exited {result.returncode}: {result.stderr[-2000:]!r}")
        return result

    def read(self, path):
        started = time.monotonic_ns()
        data = path.read_bytes()
        self.record("read", {"path": str(path)}, self.blob(data), started)
        return data

    def edit(self, path, data):
        started = time.monotonic_ns()
        before = path.read_bytes()
        path.write_bytes(data)
        self.record("edit", {"path": str(path), "before": self.blob(before)}, self.blob(data), started)


def source_without_route():
    demo = (ROOT / "examples/http_demo/main.il").read_text(encoding="utf-8")
    route_text = "," + json.dumps(ROUTE, separators=(",", ":"))
    require(demo.count(route_text) == 1, "demo must contain exactly the locked hello route")
    return "\n".join((ROOT / "packages" / name / "lib.il").read_text(encoding="utf-8")
                     for name in MODULES) + "\n" + demo.replace(route_text, "")


class IlTrial:
    def __init__(self, trace, binary):
        self.trace, self.binary = trace, binary
        self.repository = trace.directory / "source"
        self.repository.mkdir()
        self.store = trace.directory / "store"
        self.policy = trace.directory / "policy.json"
        state = {"project_id": "il", "schema_version": "1.0.0", "head_revision": 0,
                 "last_verified_revision": 0, "compiler_version": None, "runtime_version": None,
                 "target": TARGET, "open_tasks": [], "blocked_tasks": [], "failed_tasks": [], "locks": {}}
        graph = {"project_id": "il", "graph_version": "1.0.0", "revision": 0, "target": TARGET,
                 **{name: [] for name in ("modules", "types", "functions", "capabilities", "packages", "contracts")}}
        save(self.repository / "repository_state.json", state)
        save(self.repository / "examples/bootstrap/graph.json", graph)
        shutil.copyfile(ROOT / "toolchain.lock", self.repository / "toolchain.lock")
        self.git("init", "--initial-branch=main")
        self.git("add", ".")
        self.git("-c", "user.name=il experiment", "-c", "user.email=experiment@invalid.local",
                 "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", "commit", "-m", "test: isolate G4 source")
        save(self.repository / ".git/il/revision_bindings.json", {"schema_version": "1.0.0", "bindings": [{
            "revision": 0, "git_commit": self.git("rev-parse", "HEAD").decode().strip(),
            "tree_hash": self.git("rev-parse", "HEAD^{tree}").decode().strip()}]})
        save(self.policy, {"schema_version": "1.0.0", "grants": [
            {"entity_id": "demo.listen", "kind": "Listen", "scope": "127.0.0.1:8080"},
            {"entity_id": "demo.clock", "kind": "ClockRead", "scope": None}], "test_faults": None})
        self.invoke("transact", {"task_id": "P10-setup", "base_revision": 0, "scope": ["program"],
                    "operations": [{"op": "import_text", "source": source_without_route()}], "required_checks": []})

    def git(self, *args):
        return self.trace.process(["git", "-C", self.repository, *args]).stdout

    def invoke(self, command, request, ok=True):
        result = self.trace.process([self.binary, "--repository", self.repository, "--store", self.store,
                                    "--host-policy", self.policy, command], stdin=encoded(request), expected=None)
        response = json.loads(result.stdout)
        require(response.get("ok") is ok and (result.returncode == 0) is ok, str(response)[:3000])
        if not ok:
            require(bool(response["diagnostics"]), "missing structured failure")
        return response

    def add(self, handler="demo.hello", ok=True):
        route = copy.deepcopy(ROUTE)
        route["handler"] = handler
        return self.invoke("transact", {"task_id": "P10-route", "base_revision": 1,
            "scope": ["demo.server", "demo.dispatch", "demo.route.hello"],
            "operations": [{"op": "add_route", "route_table_id": "demo.server", "route": route}],
            "required_checks": ["types", "contracts", "ownership"]}, ok)

    def snapshot(self, revision):
        return (self.store / f".il/revisions/{revision:020d}/graph.json").read_bytes()

    def build(self, revision):
        response = self.invoke("build", {"revision": revision, "target": TARGET,
                               "profile": "release", "runtime_profile": "full"})
        artifacts = response["result"]["native"]["artifacts"]
        for role, reference in artifacts.items():
            require(digest(Path(reference["path"]).read_bytes()) == reference["sha256"], f"invalid {role} hash")
        return Path(artifacts["executable"]["path"])


@contextlib.contextmanager
def service(trace, executable, policy, implementation):
    import fcntl
    started = time.monotonic_ns()
    directory = trace.directory / ("server-" + uuid.uuid4().hex)
    directory.mkdir()
    descriptor = None
    if implementation == "il":
        with policy.open("rb") as stream:
            descriptor = fcntl.fcntl(stream.fileno(), fcntl.F_DUPFD_CLOEXEC, 10)
        def prepare():
            os.dup2(descriptor, 4)
            os.set_inheritable(4, True)
        args = [str(executable)]
    else:
        prepare = None
        args = [str(executable), "--port", "8080", "--max-requests", "100", "--deadline-ms", "5000"]
    process = None
    try:
        # Refuse to contact a service owned by another trial/CI run.
        with socket.socket() as probe:
            probe.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            probe.bind(("127.0.0.1", 8080))
        with (directory / "stdout").open("wb") as out, (directory / "stderr").open("wb") as err:
            process = subprocess.Popen(args, stdin=subprocess.DEVNULL, stdout=out, stderr=err,
                                       env={}, close_fds=implementation != "il", preexec_fn=prepare)
            deadline = time.monotonic() + 10
            while True:
                require(process.poll() is None, "server exited during startup")
                try:
                    with socket.create_connection(("127.0.0.1", 8080), timeout=.2):
                        break
                except ConnectionRefusedError:
                    require(time.monotonic() < deadline, "server startup timeout")
                    time.sleep(.02)
            yield
    finally:
        if process is not None:
            if process.poll() is None:
                process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
            trace.record("service", {"argv": args, "binary": trace.blob(executable.read_bytes())},
                         {"exit_code": process.returncode, "reaped": True,
                          "stdout": trace.blob((directory / "stdout").read_bytes()),
                          "stderr": trace.blob((directory / "stderr").read_bytes())}, started)
        if descriptor is not None:
            os.close(descriptor)


def wire_checks(trace, contract, has_route):
    for expected in contract["requests"]:
        row = dict(expected)
        if row["path"].startswith("/hello/") and not has_route:
            row = {"method": "GET", "path": row["path"], "status": 404, "body": ""}
        request = f'{row["method"]} {row["path"]} HTTP/1.1\r\nHost: localhost\r\n\r\n'.encode()
        started = time.monotonic_ns()
        response = b""
        try:
            with socket.create_connection(("127.0.0.1", 8080), timeout=8) as client:
                client.sendall(request)
                while True:
                    chunk = client.recv(65536)
                    if not chunk:
                        break
                    response += chunk
                    require(len(response) <= 65536, "unbounded HTTP reply")
        finally:
            trace.record("http", {"wire": trace.blob(request), "expected": row},
                         {"wire": trace.blob(response)}, started)
        head, sep, body = response.partition(b"\r\n\r\n")
        require(bool(sep), "missing HTTP headers")
        lines = head.split(b"\r\n")
        require(int(lines[0].split()[1]) == row["status"], f"unexpected HTTP status: {lines[0]!r}")
        headers = dict(line.lower().split(b":", 1) for line in lines[1:])
        require(int(headers[b"content-length"]) == len(body), "HTTP content length differs")
        require(headers[b"connection"].strip() == b"close", "missing close header")
        if "json" in row:
            require(json.loads(body) == row["json"], "unexpected JSON body")
        else:
            require(body == row["body"].encode(), f"unexpected HTTP body: {body!r}")
    return len(contract["requests"])


def test_service(trace, contract, exe, policy, implementation, has_route):
    with service(trace, exe, policy, implementation):
        return wire_checks(trace, contract, has_route)


def patch_metrics(before, after):
    lines = list(difflib.unified_diff(before.decode().splitlines(True), after.decode().splitlines(True),
                                    fromfile="a/program", tofile="b/program"))
    patch = "".join(lines)
    if before != after:
        patch = "diff --git a/program b/program\n" + patch
    return patch.encode(), {"files_changed": int(before != after),
        "insertions": sum(line.startswith("+") and not line.startswith("+++") for line in lines),
        "deletions": sum(line.startswith("-") and not line.startswith("---") for line in lines)}


def initialize_rust_repository(trace, directory):
    def git(*args, expected=0):
        return trace.process(
            ["git", "-C", directory, *args],
            expected=expected,
            env={"GIT_AUTHOR_DATE": G4_BASELINE_DATE, "GIT_COMMITTER_DATE": G4_BASELINE_DATE},
        ).stdout

    git("init", "--initial-branch=main")
    git("config", "user.name", "il G4 baseline")
    git("config", "user.email", "g4-baseline@invalid.local")
    git("config", "commit.gpgsign", "false")
    git("add", "main.rs")
    git("commit", "-m", "test: pin G4 baseline")
    return git, git("rev-parse", "HEAD").decode().strip()


def graph_has_hello_route(graph):
    return b'"entity_id": "demo.route.hello"' in graph and b'"/hello/{name}"' in graph


def write_g4_record(directory, output_root, trace, *, task, implementation, index,
                    starting_commit, task_input_sha256, started, finished,
                    assertions, success, failure_category, patch):
    trace_events = {
        "schema_version": "1.0.0",
        "kind": "g4_trace",
        "run_id": trace.run_id,
        "events": [
            {
                "sequence": event["sequence"],
                "kind": "tool_call",
                "tool": event["kind"],
                "request_sha256": digest(encoded(event["request"])),
                "response_sha256": digest(encoded(event["response"])),
            }
            for event in trace.events
        ],
    }
    trace_path = directory / "g4_trace.json"
    save(trace_path, trace_events)
    patch_path = directory / "patch.diff"
    patch_path.write_bytes(patch)
    record = {
        "schema_version": "1.0.0",
        "kind": "g4_trial",
        "task_id": task,
        "implementation": implementation,
        "trial_index": index,
        "run_id": trace.run_id,
        "workspace_id": trace.workspace_id,
        "starting_commit": starting_commit,
        "task_input_sha256": task_input_sha256,
        "started_monotonic_ns": started,
        "finished_monotonic_ns": finished,
        "experiment_exit_code": 0 if success else 1,
        "failure_category": None if success else failure_category,
        "assertions": [
            {"id": identity, "exit_code": 0 if count else None, "count": count}
            for identity, count in assertions.items()
        ],
        "trace": {
            "role": "trace",
            "path": trace_path.relative_to(output_root).as_posix(),
            "sha256": digest(trace_path.read_bytes()),
        },
        "patch": {
            "role": "patch",
            "path": patch_path.relative_to(output_root).as_posix(),
            "sha256": digest(patch),
        },
    }
    record_path = directory / "g4_trial.json"
    save(record_path, record)
    return {
        "path": record_path.relative_to(output_root).as_posix(),
        "sha256": digest(record_path.read_bytes()),
    }


def trial(directory, implementation, task, index, binary, contract):
    directory.mkdir()
    trace = Trace(directory)
    attempt_started = time.monotonic_ns()
    task_input_sha256 = digest(encoded({"task": task, "requests": contract["requests"], "failure_categories": contract["failure_categories"]}))
    source_commit = subprocess.run(["git", "-C", str(ROOT), "rev-parse", "HEAD"], capture_output=True, check=True, text=True).stdout.strip()
    starting_commit = source_commit
    row = {"implementation": implementation, "task": task, "repetition": index,
           "directory": str(directory), "source_commit": source_commit,
           "starting_commit": starting_commit,
           "task_input_sha256": task_input_sha256, "success": False, "failure_category": None}
    before = after = b""
    started = None
    assertions = {identity: 0 for identity in contract["g4"]["tasks"][task]}

    def passed(identity, count=1):
        assertions[identity] += count

    try:
        if implementation == "il":
            app = IlTrial(trace, binary)
            old = None
            if task == "rollback":
                app.add()
                old = app.build(2)
                before = app.snapshot(2)
            else:
                before = app.snapshot(1)
            if task == "fix_type_error":
                old = app.build(1)
                old_hash = digest(old.read_bytes())
                test_service(trace, contract, old, app.policy, implementation, False)

            trace.phase = "measured"
            started = time.monotonic_ns()
            app.invoke("inspect", {"entity_id": "demo.server", "revision": 2 if task == "rollback" else 1, "budget": 4096})
            if task == "fix_type_error":
                rejected = app.add("demo.health", ok=False)
                require(rejected["result_revision"] == 1, "failed candidate changed HEAD")
                diagnostic = rejected["diagnostics"][0]
                require("TYPE" in diagnostic["code"] or "CONTRACT" in diagnostic["code"], "not a type/contract diagnostic")
                for field in ("diagnostic_id", "code", "stage", "entity_id"):
                    require(isinstance(diagnostic.get(field), str) and diagnostic[field], f"diagnostic omitted {field}")
                app.invoke("explain", {"run_id": rejected["result"]["run_id"], "diagnostic_id": diagnostic["diagnostic_id"], "context_budget": 8192})
                passed("diagnostic_machine_readable")
                require(digest(old.read_bytes()) == old_hash, "failed transaction replaced the previous executable")
                test_service(trace, contract, old, app.policy, implementation, False)
                app.add()
                after = app.snapshot(2)
                require(graph_has_hello_route(after), "repair did not install the hello route")
                passed("repair_applied")
                exe = app.build(2)
                passed("regression_suite", test_service(trace, contract, exe, app.policy, implementation, True))
            elif task == "add_route":
                result = app.add()
                require(result["result_revision"] == 2, "route transaction did not publish the next revision")
                after = app.snapshot(2)
                require(graph_has_hello_route(after), "route transaction did not install the hello route")
                passed("route_present")
                passed("typecheck")
                exe = app.build(2)
                passed("http_blackbox", test_service(trace, contract, exe, app.policy, implementation, True))
            if task == "rollback":
                restored = app.invoke("restore", {"revision": 1, "reason": "G4 rollback trial"})
                require(restored["result_revision"] == 3, "restore must create a new revision")
                passed("rollback_transaction")
                after = app.snapshot(3)
                a, b = json.loads(app.snapshot(1)), json.loads(after)
                a.pop("revision"); b.pop("revision")
                require(a == b, "restored graph differs from history")
                passed("restored_graph_matches")
                exe = app.build(3)
                test_service(trace, contract, exe, app.policy, implementation, False)
                passed("previous_binary_runs", test_service(trace, contract, old, app.policy, implementation, True))
        else:
            baseline = ROOT / "tests/p10/baseline"
            source = directory / "main.rs"
            source.write_bytes((baseline / "main.rs").read_bytes())
            initial = source.read_bytes()
            fragment = (baseline / "hello_route.rs.fragment").read_bytes()
            require(initial.count(b"// G4_HELLO_ROUTE") == 1, "missing Rust route anchor")
            installed = initial.replace(b"// G4_HELLO_ROUTE", fragment)
            git, starting_commit = initialize_rust_repository(trace, directory)
            row["starting_commit"] = starting_commit

            def compile_source(name, expected=0):
                exe = directory / name
                result = trace.process(["rustc", "--edition=2021", "-O", "--error-format=json", source, "-o", exe], expected=expected)
                return exe, result

            if task == "rollback":
                trace.edit(source, installed)
                git("add", "main.rs")
                git("commit", "-m", "feat: add hello route")
                old, _ = compile_source("previous")
                test_service(trace, contract, old, None, implementation, True)
                before = source.read_bytes()
            elif task == "fix_type_error":
                fixture = json.loads((baseline / "type_error.json").read_bytes())
                valid = fixture["valid"].encode()
                invalid = fixture["invalid"].encode()
                require(initial.count(valid) == 1, "Rust type-error fixture does not match the baseline")
                old, _ = compile_source("previous")
                old_hash = digest(old.read_bytes())
                test_service(trace, contract, old, None, implementation, False)
                trace.edit(source, initial.replace(valid, invalid))
                before = source.read_bytes()
            else:
                before = source.read_bytes()

            trace.phase = "measured"
            started = time.monotonic_ns()
            trace.read(source)
            if task == "fix_type_error":
                _, result = compile_source("rejected", expected=None)
                require(result.returncode != 0, "Rust type error unexpectedly compiled")
                diagnostics = [json.loads(line) for line in result.stderr.splitlines() if line]
                structured = [d for d in diagnostics if d.get("code") and d["code"].get("code") == "E0308"]
                require(bool(structured), "Rust E0308 missing")
                require(any(d.get("spans") and d.get("message") for d in structured), "Rust diagnostic omitted machine-readable location or message")
                passed("diagnostic_machine_readable")
                require(digest(old.read_bytes()) == old_hash, "failed compile replaced the previous executable")
                test_service(trace, contract, old, None, implementation, False)
                trace.edit(source, installed)
                git("add", "main.rs")
                git("commit", "-m", "fix: repair health handler and add route")
                after = source.read_bytes()
                exe, _ = compile_source("program")
                passed("repair_applied")
                passed("regression_suite", test_service(trace, contract, exe, None, implementation, True))
            elif task == "add_route":
                trace.edit(source, installed)
                git("add", "main.rs")
                git("commit", "-m", "feat: add hello route")
                after = source.read_bytes()
                require(b"/hello/{name}" in after, "Rust edit did not add the hello route")
                passed("route_present")
                passed("typecheck")
                exe, _ = compile_source("program")
                passed("http_blackbox", test_service(trace, contract, exe, None, implementation, True))
            if task == "rollback":
                before_commit = git("rev-parse", "HEAD").decode().strip()
                git("revert", "--no-edit", "HEAD")
                after = trace.read(source)
                require(after == initial, "Rust Git rollback differs from its pinned source")
                require(git("rev-parse", "HEAD").decode().strip() != before_commit, "Rust rollback did not create a new commit")
                passed("rollback_transaction")
                passed("restored_graph_matches")
                exe, _ = compile_source("program")
                test_service(trace, contract, exe, None, implementation, False)
                passed("previous_binary_runs", test_service(trace, contract, old, None, implementation, True))
        row["success"] = True
    except Exception as error:
        row["failure_category"] = "timeout" if isinstance(error, subprocess.TimeoutExpired) else "assertion" if isinstance(error, AssertionError) else "environment" if isinstance(error, OSError) else "other"
        row["error"] = str(error)
        row["traceback"] = traceback.format_exc()
    finally:
        finished = time.monotonic_ns()
        row["elapsed_ns"] = finished - started if started is not None else None
        patch, metrics = patch_metrics(before, after)
        row["changes"] = metrics
        row["before"] = trace.blob(before)
        row["after"] = trace.blob(after)
        row["patch"] = trace.blob(patch)
        row["tool_calls"] = sum(event["phase"] == "measured" for event in trace.events)
        row["trace"] = trace.blob(encoded(trace.events))
        row["change_representation"] = "canonical_graph" if implementation == "il" else "rust_source"
        save(directory / "trial.json", row)
        output_root = directory.parent
        row["g4_trial"] = write_g4_record(
            directory,
            output_root,
            trace,
            task=task,
            implementation=implementation,
            index=index,
            starting_commit=row["starting_commit"],
            task_input_sha256=row["task_input_sha256"],
            started=started if started is not None else attempt_started,
            finished=finished,
            assertions=assertions,
            success=row["success"],
            failure_category=row["failure_category"],
            patch=patch,
        )
        save(directory / "trial.json", row)
    return row


def aggregate(rows, contract):
    expected = {(impl, task, i) for impl in contract["implementations"]
                for task in contract["tasks"] for i in range(1, 4)}
    actual = [(r["implementation"], r["task"], r["repetition"]) for r in rows]
    complete = len(actual) == len(expected) and set(actual) == expected
    summaries = []
    for impl in contract["implementations"]:
        for task in contract["tasks"]:
            selected = [r for r in rows if r["implementation"] == impl and r["task"] == task]
            durations = [r["elapsed_ns"] for r in selected if r["elapsed_ns"] is not None]
            summaries.append({"implementation": impl, "task": task, "attempts": len(selected),
                "successes": sum(r["success"] for r in selected),
                "success_rate": sum(r["success"] for r in selected) / len(selected) if selected else None,
                "median_elapsed_ns": statistics.median(durations) if durations else None,
                "failure_distribution": {cat: sum(r["failure_category"] == cat for r in selected) for cat in contract["failure_categories"]}})
    return {"matrix_complete": complete, "all_trials_succeeded": complete and all(r["success"] for r in rows), "summaries": summaries}


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/il")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--implementation", choices=("rust", "il"))
    parser.add_argument("--task", choices=("add_route", "fix_type_error", "rollback"))
    parser.add_argument("--repetitions", type=int, choices=(1, 3), default=3)
    args = parser.parse_args()
    require(sys.platform == "linux", "real G4 execution requires locked Linux toolchain")
    contract = json.loads(CONTRACT_PATH.read_bytes())
    require(json.loads((ROOT / "eval/tasks/P09.json").read_bytes())["status"] == "VERIFIED", "P09 dependency not verified")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    binary = args.binary.resolve()
    metadata = Trace(output / "provenance")
    rust = metadata.process(["rustc", "--version"]).stdout.decode().strip()
    llvm = metadata.process(["llvm-config-14", "--version"]).stdout.decode().strip()
    require(rust.startswith("rustc 1.90.0 ") and llvm == "14.0.6", "toolchain version mismatch")
    head = metadata.process(["git", "-C", ROOT, "rev-parse", "HEAD"]).stdout.decode().strip()
    dirty = metadata.process(["git", "-C", ROOT, "status", "--porcelain"]).stdout.decode()
    inputs = {str(p.relative_to(ROOT)): digest(p.read_bytes()) for p in
              [CONTRACT_PATH, Path(__file__), ROOT / "toolchain.lock", ROOT / "examples/http_demo/main.il",
               *[ROOT / "packages" / name / "lib.il" for name in MODULES],
               *sorted((ROOT / "tests/p10/baseline").glob("*"))] if p.is_file()}
    report = {"schema_version": "1.0.0", "method": contract["method"], "source_commit": head,
              "baseline_commit": None,
              "source_dirty": bool(dirty), "source_status": dirty, "input_hashes": inputs,
              "compiler_sha256": digest(binary.read_bytes()), "trials": [], "status": "RUNNING",
              "scope": "Scripted workflow comparison; not an LLM benchmark or full HTTP acceptance"}
    save(output / "comparison.json", report)
    for task in ([args.task] if args.task else contract["tasks"]):
        for index in range(1, args.repetitions + 1):
            for impl in ([args.implementation] if args.implementation else contract["implementations"]):
                identity = f"{task}-{index}-{impl}"
                print(f"Running {identity}", flush=True)
                row = trial(output / identity, impl, task, index, binary, contract)
                report["trials"].append(row)
                if impl == "rust":
                    if report["baseline_commit"] is None:
                        report["baseline_commit"] = row["starting_commit"]
                    require(report["baseline_commit"] == row["starting_commit"], "Rust baseline commit changed between trials")
                save(output / "comparison.json", report)
                print(json.dumps({"trial": identity, "success": row["success"], "error": row.get("error")}), flush=True)
    report.update(aggregate(report["trials"], contract))
    report["status"] = "COMPLETE" if report["matrix_complete"] else "PARTIAL"
    report["release_eligible"] = report["matrix_complete"] and not report["source_dirty"]
    save(output / "comparison.json", report)
    return 0 if all(r["success"] for r in report["trials"]) else 1


if __name__ == "__main__":
    raise SystemExit(main())
