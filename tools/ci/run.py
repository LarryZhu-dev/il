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


def p09_report_count(report: dict, suite: str) -> int:
    """Validate the complete, machine emitted P09 process-fault report.

    The fault runners intentionally print a small JSON receipt instead of a
    unittest summary.  Treating a zero count as success would allow a runner
    which never reached its barrier to satisfy the CI gate, so the receipt is
    checked against the locked fault contract here.
    """
    if not isinstance(report, dict) or report.get("schema_version") != "1.0.0":
        raise ValueError("P09 report must use schema version 1.0.0")
    if report.get("suite") != suite or report.get("status") != "PASSED":
        raise ValueError(f"P09 {suite} report is not a passed receipt")
    if suite == "transaction_process":
        cases = report.get("cases")
        stages = ["transaction_before_snapshot", "transaction_after_snapshot",
                  "transaction_before_head_rename", "transaction_after_head_rename"]
        if (not isinstance(cases, list) or len(cases) != len(stages)
                or report.get("case_count") != len(stages)):
            raise ValueError("P09 transaction receipt lacks all four interruption cases")
        for case, stage in zip(cases, stages):
            event = case.get("event") if isinstance(case, dict) else None
            if (not isinstance(case, dict) or case.get("stage") != stage
                    or not isinstance(event, dict) or event.get("stage") != stage
                    or type(case.get("pid")) is not int or case["pid"] <= 0
                    or case.get("signal") != "SIGKILL"
                    or type(case.get("retry_revision")) is not int
                    or case["retry_revision"] <= 0
                    or case.get("cleanup", {}).get("owned_process_reaped") is not True):
                raise ValueError("P09 transaction receipt lacks a real interruption or retry observation")
        return len(cases)
    if suite == "build_process":
        worker = report.get("worker")
        if (not isinstance(worker, dict) or worker.get("stage") != "link_runtime"
                or type(worker.get("parent_pid")) is not int or worker["parent_pid"] <= 0
                or type(worker.get("worker_pid")) is not int or worker["worker_pid"] <= 0
                or report.get("signal") != "SIGKILL"
                or not isinstance(report.get("failure_reports"), list)
                or not report["failure_reports"]
                or not isinstance(report.get("retry"), dict)
                or report["retry"].get("verified") is not True):
            raise ValueError("P09 build receipt lacks the real link worker or verified retry")
        return 1 + len(report["failure_reports"])
    raise ValueError(f"unknown P09 report suite: {suite}")


def p09_report_from_output(output: str, suite: str) -> dict:
    """Extract the final JSON receipt without accepting launcher diagnostics."""
    for line in reversed(output.splitlines()):
        try:
            value = json.loads(line)
        except (TypeError, ValueError):
            continue
        if isinstance(value, dict) and value.get("suite") == suite:
            return value
    raise ValueError(f"P09 {suite} gate did not emit a JSON receipt")


def p09_contract_hash() -> str:
    """Validate the locked fault contract before accepting either process gate."""
    path = ROOT / "tests/fault_injection/contracts.json"
    data = path.read_bytes()
    contract = json.loads(data)
    suites = contract.get("suites") if isinstance(contract, dict) else None
    if (not isinstance(contract, dict) or contract.get("schema_version") != "1.0.0"
            or not isinstance(suites, list)):
        raise ValueError("P09 fault contract is not a valid schema 1.0.0 document")
    by_id = {item.get("id"): item for item in suites if isinstance(item, dict)}
    required = {
        "transaction_process": {
            "events": ["transaction_before_head_rename", "transaction_after_head_rename"],
            "assertions": ["SIGKILL", "fresh_process_reopen", "monotonic_revision", "no_revision_reuse"],
        },
        "build_process": {
            "events": ["link_runtime"],
            "assertions": ["SIGKILL_real_worker", "retained_failure_diagnostic", "old_verified_executable_runs", "successful_retry"],
        },
        "accept_failure_external_tcp": {
            "events": ["accept_fail_after_zero", "accept_fail_after_one"],
            "assertions": ["captured_native_debug_and_release", "real_tcp_health_before_second_accept",
                            "E_HTTP_ACCEPT_FAILED", "listener_released", "zero_live_allocations",
                            "execution_schema_valid", "process_binary_policy_hashes"],
        },
    }
    if set(by_id) != set(required) or len(by_id) != len(suites):
        raise ValueError("P09 fault contract is missing or duplicating a required suite")
    for suite, expected in required.items():
        value = by_id[suite]
        if value.get("events") != expected["events"] or value.get("assertions") != expected["assertions"]:
            raise ValueError(f"P09 fault contract changed required cases for {suite}")
    return hashlib.sha256(data).hexdigest()


def protocol_report_count(report: dict) -> int:
    """Require actual compiler acceptance, not just the launcher worker fixture."""
    if not isinstance(report, dict):
        raise ValueError("P08 acceptance report must be an object")
    expected = {
        "test_historical_budget_diagnostics_and_receipt_tamper",
        "test_bounded_route_diagnosis_repair_restore_and_receipts",
    }
    cases = report.get("cases")
    if (report.get("suite") != "ai_protocol_cli" or report.get("scope") != "worker_and_real_cli"
            or report.get("state") != "PASSED" or report.get("passed") is not True
            or type(report.get("count")) is not int or report["count"] < 12
            or report.get("failures") != [] or report.get("errors") != []
            or not isinstance(cases, list) or len(cases) != len(expected)
            or any(not isinstance(case, dict) or not isinstance(case.get("id"), str) or case.get("passed") is not True for case in cases)
            or {case.get("id") for case in cases} != expected):
        raise ValueError("P08 gate requires the complete worker and real CLI acceptance report")
    return report["count"]


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

    def publish_report() -> None:
        pending = OUTPUT / "result.pending"
        with pending.open("w", encoding="utf-8", newline="\n") as stream:
            stream.write(json.dumps(result, indent=2, sort_keys=True) + "\n")
            stream.flush()
            os.fsync(stream.fileno())
        pending.replace(OUTPUT / "result.json")

    publish_report()

    def run(name: str, arguments: list[str]) -> None:
        result["active_gate"] = {"name": name, "command": arguments}
        publish_report()
        if name == "ai-protocol-cli":
            (ROOT / "build/ai_protocol_cli_report.json").unlink(missing_ok=True)
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
        is_test = name in {"bootstrap-pre-commit", "bootstrap-strict", "bootstrap-unit-tests", "rust-tests", "p09-transaction-process", "p09-build-process", "native-probe", "graph-cli", "semantic-cli", "text-cli", "execution-cli", "execution-cli-debug", "interpreter-profile-parity", "native-cli", "runtime-cli", "network-cli", "bytes-cli", "http-cli", "ai-protocol-cli"}
        test_count = 0
        gate_error = None
        report_artifact = {}
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
        elif name in {"bootstrap-unit-tests", "graph-cli", "text-cli", "execution-cli", "execution-cli-debug", "native-cli", "runtime-cli", "network-cli", "bytes-cli", "http-cli"}:
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
        elif name == "ai-protocol-cli":
            report_path = ROOT / "build/ai_protocol_cli_report.json"
            try:
                data = report_path.read_bytes()
                report_artifact = {"report": report_path.relative_to(ROOT).as_posix(),
                                   "report_sha256": hashlib.sha256(data).hexdigest()}
                report = json.loads(data)
                count = report.get("count")
                test_count = count if type(count) is int and count >= 0 else 0
                if process.returncode == 0:
                    test_count = protocol_report_count(report)
            except (OSError, ValueError, AttributeError) as error:
                gate_error = str(error)
        elif name in {"p09-transaction-process", "p09-build-process"}:
            suite = "transaction_process" if name == "p09-transaction-process" else "build_process"
            try:
                report = p09_report_from_output(process.stdout, suite)
                test_count = p09_report_count(report, suite)
                encoded = json.dumps(report, sort_keys=True, separators=(",", ":")).encode("utf-8")
                contract_hash = p09_contract_hash()
                report_path = OUTPUT / f"{name}_report.json"
                report_bytes = encoded + b"\n"
                report_path.write_bytes(report_bytes)
                report_artifact = {
                    "report": report_path.relative_to(ROOT).as_posix(),
                    "report_sha256": hashlib.sha256(report_bytes).hexdigest(),
                    "contract": "tests/fault_injection/contracts.json",
                    "contract_sha256": contract_hash,
                }
            except (OSError, ValueError, TypeError, KeyError) as error:
                gate_error = str(error)
        result["gates"].append({
            "name": name,
            "command": arguments,
            "status": "PASSED" if process.returncode == 0 and gate_error is None else "FAILED",
            "exit_code": process.returncode,
            "is_test": is_test,
            "test_count": test_count,
            "elapsed_seconds": round(time.monotonic() - started, 6),
            "log": log.relative_to(ROOT).as_posix(),
            "log_sha256": hashlib.sha256(log.read_bytes()).hexdigest(),
            **report_artifact,
            **({"error": gate_error} if gate_error else {}),
        })
        result.pop("active_gate", None)
        publish_report()
        if process.returncode:
            raise RuntimeError(f"gate {name} failed with exit code {process.returncode}")
        if gate_error:
            raise RuntimeError(f"gate {name} report validation failed: {gate_error}")

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
            run("rust-tests", ["cargo", "test", "--workspace", "--locked", "--", "--nocapture"])
            run("p09-fault-build", ["cargo", "build", "--locked", "-p", "il", "--features", "process-test-barriers"])
            run("p09-transaction-process", [sys.executable, "tests/fault_injection/transaction_process.py", "--binary", "target/debug/il"])
            run("p09-build-process", [sys.executable, "tests/fault_injection/build_process.py", "--binary", "target/debug/il"])
            if (ROOT / "tests/execution_cli.py").is_file():
                run("rust-debug", ["cargo", "build", "--locked", "-p", "il"])
                run("full-runtime-debug", ["cargo", "build", "--locked", "-p", "il-native-runtime"])
                run("execution-cli-debug", [sys.executable, "tests/execution_cli.py", "--binary", "target/debug/il", "--report", "build/execution_cli_debug_report.json"])
            run("rust-release", ["cargo", "build", "--workspace", "--release", "--locked"])
            run("full-runtime-release", ["cargo", "build", "--locked", "-p", "il-native-runtime", "--release"])
            if (ROOT / "runtime/minimal/build.py").is_file():
                run("minimal-runtime", [sys.executable, "runtime/minimal/build.py", "--output", "target/release/libil_minimal_runtime.a"])
            if native_probe_enabled:
                run("native-probe", [sys.executable, "tools/test_native_probe.py"])
            if (ROOT / "tests/graph_cli.py").is_file():
                run("graph-cli", [sys.executable, "tests/graph_cli.py", "--binary", "target/release/il", "--report", "build/graph_cli_report.json"])
            for suite in ("semantic", "text", "execution", "native", "runtime", "network", "bytes"):
                if (ROOT / f"tests/{suite}_cli.py").is_file():
                    run(f"{suite}-cli", [sys.executable, f"tests/{suite}_cli.py", "--binary", "target/release/il", "--report", f"build/{suite}_cli_report.json"])
            if (ROOT / "tests/http_blackbox/cli.py").is_file():
                run("http-cli", [sys.executable, "tests/http_blackbox/cli.py", "--binary", "target/release/il", "--report", "build/http_cli_report.json"])
            if (ROOT / "tests/ai_protocol_cli.py").is_file():
                run("ai-protocol-cli", [sys.executable, "tests/ai_protocol_cli.py", "--binary", "target/release/il", "--report", "build/ai_protocol_cli_report.json"])
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
    except KeyboardInterrupt:
        result["status"] = "INTERRUPTED"
        result["error"] = "CI run interrupted before all required gates completed"
        return 130
    except (OSError, ValueError, subprocess.SubprocessError, RuntimeError) as error:
        result["status"] = "FAILED"
        result["error"] = str(error)
        print(f"CI gate failure: {error}", file=sys.stderr)
        return 1
    finally:
        publish_report()


if __name__ == "__main__":
    sys.exit(main())
