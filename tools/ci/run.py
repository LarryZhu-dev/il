"""Run fixed bootstrap gates; no caller-provided commands are accepted."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import time


ROOT = Path(__file__).resolve().parents[2]
OUTPUT = ROOT / "build" / "ci"


def main() -> int:
    if len(sys.argv) != 1:
        print("ci runner accepts no arguments", file=sys.stderr)
        return 2
    OUTPUT.mkdir(parents=True, exist_ok=True)
    result = {
        "schema_version": "1.0.0",
        "scope": "bootstrap-linux-x86_64",
        "target": "x86_64-unknown-linux-gnu",
        "status": "RUNNING",
        "gates": [],
    }

    def run(name: str, arguments: list[str]) -> None:
        started = time.monotonic()
        process = subprocess.run(
            arguments, cwd=ROOT, text=True, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, check=False,
        )
        log = OUTPUT / f"{name}.log"
        log.write_text(process.stdout, encoding="utf-8")
        if name == "environment-packages":
            print(f"Recorded installed package versions in {log.relative_to(ROOT).as_posix()}", flush=True)
        else:
            print(process.stdout, end="", flush=True)
        is_test = name in {"bootstrap-pre-commit", "bootstrap-strict", "bootstrap-unit-tests", "rust-tests", "native-probe", "graph-cli", "semantic-cli", "text-cli", "execution-cli", "execution-cli-debug", "interpreter-profile-parity", "native-cli"}
        test_count = 0
        if name in {"bootstrap-pre-commit", "bootstrap-strict", "native-probe"}:
            test_count = 1
            try:
                report = json.loads(process.stdout)
                checks = report.get("checks") if isinstance(report, dict) else None
                if isinstance(checks, (list, dict)):
                    test_count = len(checks)
            except (ValueError, TypeError):
                pass
        elif name == "rust-tests":
            test_count = sum(
                int(passed) + int(failed)
                for passed, failed in re.findall(
                    r"test result: \w+\. (\d+) passed; (\d+) failed;", process.stdout
                )
            )
        elif name in {"bootstrap-unit-tests", "graph-cli", "text-cli", "execution-cli", "execution-cli-debug", "native-cli"}:
            counts = re.findall(r"Ran (\d+) tests? in", process.stdout)
            test_count = int(counts[-1]) if counts else 0
        elif name == "semantic-cli":
            report_path = ROOT / "build/semantic_cli_report.json"
            if report_path.is_file():
                test_count = json.loads(report_path.read_text(encoding="utf-8"))["count"]
        elif name == "interpreter-profile-parity":
            try:
                test_count = json.loads(process.stdout)["count"]
            except (ValueError, KeyError):
                pass
        result["gates"].append({
            "name": name,
            "command": arguments,
            "status": "PASSED" if process.returncode == 0 else "FAILED",
            "exit_code": process.returncode,
            "is_test": is_test,
            "test_count": test_count,
            "elapsed_seconds": round(time.monotonic() - started, 6),
            "log": log.relative_to(ROOT).as_posix(),
            "log_sha256": hashlib.sha256(log.read_bytes()).hexdigest(),
        })
        if process.returncode:
            raise RuntimeError(f"gate {name} failed with exit code {process.returncode}")

    try:
        if platform.system() != "Linux" or platform.machine() not in ("x86_64", "AMD64"):
            raise RuntimeError("bootstrap acceptance requires a real Linux x86-64 execution environment")
        # A bind mount owned by the checkout user may differ from the container user.
        os.environ["GIT_CONFIG_COUNT"] = "1"
        os.environ["GIT_CONFIG_KEY_0"] = "safe.directory"
        os.environ["GIT_CONFIG_VALUE_0"] = str(ROOT)
        result["source_commit"] = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True,
        ).strip()
        run("toolchain-rust", ["rustc", "--version", "--verbose"])
        run("toolchain-llvm", ["llvm-config-14", "--version"])
        run("environment-packages", ["dpkg-query", "--show", "--showformat=${Package}\t${Version}\n"])
        # The binding is an external log, deliberately reconstructed after checkout
        # rather than committed into the tree whose Git identity it describes.
        run("git-revision-binding", [sys.executable, "tools/bind_revision.py"])
        state = json.loads((ROOT / "repository_state.json").read_text(encoding="utf-8"))
        manifest = ROOT / "Cargo.toml"
        compiler_expected = state.get("compiler_version") is not None or any(
            (ROOT / "compiler").rglob("*.rs")
        )
        native_probe_enabled = False
        for task_path in sorted((ROOT / "eval" / "tasks").glob("*.json")):
            task = json.loads(task_path.read_text(encoding="utf-8"))
            if task["task_id"] != "P00" and task["status"] in ("RUNNING", "VERIFIED"):
                compiler_expected = True
            if task["task_id"] == "P01" and task["status"] in ("RUNNING", "VERIFIED"):
                native_probe_enabled = True
        if not manifest.exists() and compiler_expected:
            raise RuntimeError("compiler implementation exists but Cargo.toml is missing")
        if manifest.exists():
            if not (ROOT / "Cargo.lock").exists():
                raise RuntimeError("locked Rust builds require checked-in Cargo.lock")
            run("rust-tests", ["cargo", "test", "--workspace", "--locked"])
            if (ROOT / "tests/execution_cli.py").is_file():
                run("rust-debug", ["cargo", "build", "--locked", "-p", "il"])
                run("execution-cli-debug", [sys.executable, "tests/execution_cli.py", "--binary", "target/debug/il", "--report", "build/execution_cli_debug_report.json"])
            run("rust-release", ["cargo", "build", "--workspace", "--release", "--locked"])
            if native_probe_enabled:
                run("native-probe", [sys.executable, "tools/test_native_probe.py"])
            if (ROOT / "tests/graph_cli.py").is_file():
                run("graph-cli", [sys.executable, "tests/graph_cli.py", "--binary", "target/release/il", "--report", "build/graph_cli_report.json"])
            for suite in ("semantic", "text", "execution", "native"):
                if (ROOT / f"tests/{suite}_cli.py").is_file():
                    run(f"{suite}-cli", [sys.executable, f"tests/{suite}_cli.py", "--binary", "target/release/il", "--report", f"build/{suite}_cli_report.json"])
            if (ROOT / "tests/execution_cli.py").is_file():
                run("interpreter-profile-parity", [sys.executable, "tools/ci/compare_execution.py"])
        else:
            result["gates"].append({
                "name": "rust-compiler",
                "status": "NOT_APPLICABLE_P00",
                "reason": "No Rust compiler implementation exists; this is bootstrap acceptance only.",
            })
        # Build recorded artifacts first: a fresh checkout has no build directory.
        # Evidence is verified against these real reconstructed bytes, never skipped.
        run("bootstrap-pre-commit", [sys.executable, "tools/bootstrap_check.py", "--pre-commit"])
        run("bootstrap-unit-tests", [sys.executable, "-m", "unittest", "discover", "-s", "tools/ci", "-p", "test_bootstrap.py", "-v"])
        run("bootstrap-strict", [sys.executable, "tools/bootstrap_check.py"])
        result["status"] = "PASSED"
        return 0
    except (OSError, ValueError, subprocess.SubprocessError, RuntimeError) as error:
        result["status"] = "FAILED"
        result["error"] = str(error)
        print(f"CI gate failure: {error}", file=sys.stderr)
        return 1
    finally:
        (OUTPUT / "result.json").write_text(
            json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8",
        )


if __name__ == "__main__":
    sys.exit(main())
