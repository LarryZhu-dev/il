"""Perform the locked two-root compiler/runtime/application rebuild for P10.

The supervisor calls this script from its fixed plan.  It accepts only an
output directory and derives both source roots from the current committed
checkout; no compiler, runtime, graph, or command path can be supplied by a
caller.  A failed root keeps its logs and partial hashes for diagnosis.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
TARGET = "x86_64-unknown-linux-gnu"
SCHEMA_VERSION = "1.0.0"


class ReproductionError(RuntimeError):
    pass


def sha_bytes(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


def sha_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return "sha256:" + h.hexdigest()


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    pending = path.with_name(path.name + ".pending")
    pending.write_text(json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    pending.replace(path)


def git(*args: str, cwd: Path = ROOT, check: bool = True) -> subprocess.CompletedProcess[bytes]:
    result = subprocess.run(["git", *args], cwd=cwd, capture_output=True, check=False)
    if check and result.returncode != 0:
        raise ReproductionError(result.stderr.decode(errors="replace").strip() or "git command failed")
    return result


def run_command(argv: list[str], cwd: Path, env: dict[str, str], log: Path, timeout: int) -> dict[str, Any]:
    started = time.monotonic_ns()
    result: subprocess.CompletedProcess[bytes] | None = None
    error = None
    try:
        result = subprocess.run(argv, cwd=cwd, env=env, capture_output=True, timeout=timeout, check=False)
    except subprocess.TimeoutExpired as exc:
        error = f"timeout after {timeout}s"
        result = subprocess.CompletedProcess(argv, 124, exc.stdout or b"", exc.stderr or b"")
    except OSError as exc:
        error = str(exc)
        result = subprocess.CompletedProcess(argv, 127, b"", str(exc).encode())
    assert result is not None
    log.parent.mkdir(parents=True, exist_ok=True)
    log.write_bytes(result.stdout + (b"\n--- stderr ---\n" if result.stderr else b"") + result.stderr)
    return {
        "argv": argv,
        "cwd": str(cwd),
        "env": {key: env[key] for key in sorted(env) if key in {"CARGO_HOME", "CARGO_TARGET_DIR", "CARGO_NET_OFFLINE", "LC_ALL", "LANG", "SOURCE_DATE_EPOCH"}},
        "started_monotonic_ns": started,
        "ended_monotonic_ns": time.monotonic_ns(),
        "exit_code": result.returncode if result.returncode >= 0 else None,
        "termination_signal": f"SIG{-result.returncode}" if result.returncode < 0 else None,
        "stdout_sha256": sha_bytes(result.stdout),
        "stderr_sha256": sha_bytes(result.stderr),
        "log": {"path": str(log), "sha256": sha_file(log), "size_bytes": log.stat().st_size},
        "error": error,
    }


def materialize(source: Path, destination: Path) -> dict[str, Any]:
    destination.mkdir(parents=True)
    archive = subprocess.check_output(["git", "-C", str(source), "archive", "--format=tar", "HEAD"])
    archive_hash = sha_bytes(archive)
    with tarfile.open(fileobj=__import__("io").BytesIO(archive), mode="r:") as tar:
        tar.extractall(destination)
    return {"archive_sha256": archive_hash, "tree": git("rev-parse", "HEAD^{tree}", cwd=source).stdout.decode().strip()}


def fixture_source(root: Path) -> str:
    modules = ("core", "alloc", "io", "time", "net", "json", "http", "test", "tracing")
    parts = [(root / "packages" / name / "lib.il").read_text(encoding="utf-8") for name in modules]
    parts.append((root / "examples" / "http_demo" / "main.il").read_text(encoding="utf-8"))
    return "\n".join(parts)


def fixture_repository(root: Path, directory: Path) -> tuple[Path, Path, Path]:
    repository = directory / "application-source"
    repository.mkdir(parents=True)
    (repository / "examples" / "bootstrap").mkdir(parents=True)
    state = {
        "project_id": "il", "schema_version": "1.0.0", "head_revision": 0,
        "last_verified_revision": 0, "compiler_version": None, "runtime_version": None,
        "target": TARGET, "open_tasks": [], "blocked_tasks": [], "failed_tasks": [], "locks": {},
    }
    graph = json.loads((root / "examples" / "bootstrap" / "graph.json").read_text(encoding="utf-8"))
    (repository / "repository_state.json").write_text(json.dumps(state, indent=2) + "\n", encoding="utf-8")
    (repository / "examples" / "bootstrap" / "graph.json").write_text(json.dumps(graph, indent=2) + "\n", encoding="utf-8")
    shutil.copyfile(root / "toolchain.lock", repository / "toolchain.lock")
    subprocess.run(["git", "init", "--initial-branch=main"], cwd=repository, capture_output=True, check=True)
    subprocess.run(["git", "add", "."], cwd=repository, capture_output=True, check=True)
    subprocess.run(["git", "-c", "user.name=il reproduction", "-c", "user.email=reproduction@invalid.local", "-c", "commit.gpgsign=false", "commit", "-m", "initial application source"], cwd=repository, capture_output=True, check=True)
    binding = repository / ".git" / "il" / "revision_bindings.json"
    binding.parent.mkdir(parents=True, exist_ok=True)
    binding.write_text(json.dumps({"schema_version": "1.0.0", "bindings": [{
        "revision": 0,
        "git_commit": git("rev-parse", "HEAD", cwd=repository).stdout.decode().strip(),
        "tree_hash": git("rev-parse", "HEAD^{tree}", cwd=repository).stdout.decode().strip(),
    }]}, indent=2) + "\n", encoding="utf-8")
    policy = directory / "policy.json"
    policy.write_text(json.dumps({"schema_version": "1.0.0", "grants": [
        {"entity_id": "demo.listen", "kind": "Listen", "scope": "127.0.0.1:8080"},
        {"entity_id": "demo.clock", "kind": "ClockRead", "scope": None}], "test_faults": None}, indent=2) + "\n", encoding="utf-8")
    return repository, directory / "application-store", policy


def build_application(root: Path, compiler: Path, directory: Path, log: Path) -> dict[str, Any]:
    repository, store, policy = fixture_repository(root, directory)
    source = fixture_source(root)
    request = {
        "task_id": "P10-reproducibility", "base_revision": 0, "scope": ["program"],
        "operations": [{"op": "import_text", "source": source}], "required_checks": [],
    }
    environment = {"LC_ALL": "C", "LANG": "C", "PATH": os.environ.get("PATH", "")}
    transaction = subprocess.run([str(compiler), "--repository", str(repository), "--store", str(store), "transact"],
                                 cwd=root, input=json.dumps(request).encode(), capture_output=True, env=environment, check=False)
    log.write_bytes(transaction.stdout + b"\n--- stderr ---\n" + transaction.stderr)
    if transaction.returncode != 0:
        raise ReproductionError(f"application transaction failed: {transaction.stderr.decode(errors='replace')[-2000:]}")
    build_request = {"revision": 1, "target": TARGET, "profile": "release", "runtime_profile": "full"}
    built = subprocess.run([str(compiler), "--repository", str(repository), "--store", str(store), "--host-policy", str(policy), "build"],
                           cwd=root, input=json.dumps(build_request).encode(), capture_output=True, env=environment, check=False)
    log.write_bytes(log.read_bytes() + b"\n--- build stdout ---\n" + built.stdout + b"\n--- build stderr ---\n" + built.stderr)
    if built.returncode != 0:
        raise ReproductionError(f"application build failed: {built.stderr.decode(errors='replace')[-2000:]}")
    response = json.loads(built.stdout)
    artifacts = response["result"]["native"]["artifacts"]
    result = {"build_response_sha256": sha_bytes(built.stdout), "artifacts": {}}
    for name, artifact in artifacts.items():
        if not artifact:
            continue
        path = Path(artifact["path"])
        result["artifacts"][name] = {"path": str(path), "sha256": sha_file(path), "size_bytes": path.stat().st_size}
    return result


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args(argv)
    output = args.output.resolve()
    report: dict[str, Any] = {"schema_version": SCHEMA_VERSION, "kind": "independent_rebuild", "status": "RUNNING", "roots": []}
    if output.exists():
        raise SystemExit("refusing to overwrite an existing reproduction directory")
    output.mkdir(parents=True)
    write_json(output / "reproduction.json", report)
    try:
        if platform.system() != "Linux" or platform.machine() not in ("x86_64", "AMD64"):
            raise ReproductionError("independent native rebuild requires Linux x86-64")
        status = git("status", "--porcelain", "--untracked-files=all").stdout.decode()
        if status:
            raise ReproductionError("source checkout is dirty")
        source_identity = {"commit": git("rev-parse", "HEAD").stdout.decode().strip(), "tree": git("rev-parse", "HEAD^{tree}").stdout.decode().strip()}
        report["source"] = source_identity
        for label in ("root_a", "root_b"):
            root = output / label
            source = root / "source"
            materialized = materialize(ROOT, source)
            target = root / "target"
            cargo_home = os.environ.get("CARGO_HOME")
            environment = {"PATH": os.environ.get("PATH", ""), "CARGO_TARGET_DIR": str(target), "CARGO_NET_OFFLINE": "true", "LC_ALL": "C", "LANG": "C", "SOURCE_DATE_EPOCH": "946684800"}
            if cargo_home:
                environment["CARGO_HOME"] = cargo_home
            build_log = root / "cargo-build.log"
            command = run_command(["cargo", "build", "--workspace", "--release", "--locked"], source, environment, build_log, 1800)
            row: dict[str, Any] = {"label": label, "materialized": materialized, "build": command, "artifacts": {}}
            if command["exit_code"] != 0:
                raise ReproductionError(f"{label} bootstrap build failed")
            compiler = target / "release" / "il"
            runtime = target / "release" / "libil_native_runtime.a"
            for name, path in (("compiler", compiler), ("runtime", runtime)):
                if not path.is_file():
                    raise ReproductionError(f"{label} missing required artifact: {path}")
                row["artifacts"][name] = {"path": str(path), "sha256": sha_file(path), "size_bytes": path.stat().st_size}
            app = build_application(source, compiler, root / "app", root / "application.log")
            row["application"] = app
            report["roots"].append(row)
            write_json(output / "reproduction.json", report)
        first, second = report["roots"]
        comparable = {"compiler", "runtime"}
        comparable.update(first["application"]["artifacts"])
        report["comparisons"] = []
        for name in sorted(comparable):
            left = first["artifacts"].get(name, first["application"]["artifacts"].get(name, {})).get("sha256")
            right = second["artifacts"].get(name, second["application"]["artifacts"].get(name, {})).get("sha256")
            report["comparisons"].append({"artifact": name, "left": left, "right": right, "equal": left == right and left is not None})
        report["status"] = "PASSED" if all(item["equal"] for item in report["comparisons"]) else "FAILED"
    except (OSError, ValueError, KeyError, ReproductionError) as error:
        report["status"] = "FAILED"
        report["error"] = str(error)
    write_json(output / "reproduction.json", report)
    print(json.dumps({"status": report["status"], "output": str(output)}, ensure_ascii=False))
    return 0 if report["status"] == "PASSED" else 1


if __name__ == "__main__":
    raise SystemExit(main())
