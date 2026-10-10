#!/usr/bin/env python3
"""Validate P10 bundle structure and byte consistency, not execution provenance."""

from __future__ import annotations

import argparse
import hashlib
import json
import statistics
import subprocess
import sys
from pathlib import Path, PurePosixPath
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
CONTRACT_PATH = Path(__file__).with_name("contract.json")
SCHEMA_PATH = Path(__file__).with_name("bundle.schema.json")
MAX_RECORD_BYTES = 16 * 1024 * 1024


class EvidenceError(Exception):
    def __init__(self, message: str, *, code: str = "E_EVIDENCE_INCOMPLETE", path: str | None = None):
        super().__init__(message)
        self.code = code
        self.path = path


def _pairs_no_duplicates(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise EvidenceError(f"duplicate JSON object key: {key}")
        value[key] = item
    return value


def _reject_json_constant(value: str) -> None:
    raise EvidenceError(f"invalid JSON constant: {value}")


def strict_json(data: bytes, *, path: str) -> Any:
    try:
        return json.loads(
            data.decode("utf-8"),
            object_pairs_hook=_pairs_no_duplicates,
            parse_constant=_reject_json_constant,
        )
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise EvidenceError(f"invalid UTF-8 JSON: {error}", path=path) from error


def sha256(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


def is_sha256(value: Any) -> bool:
    return (
        isinstance(value, str)
        and len(value) == 71
        and value.startswith("sha256:")
        and all(character in "0123456789abcdef" for character in value[7:])
    )


def is_commit(value: Any) -> bool:
    return isinstance(value, str) and len(value) == 40 and all(character in "0123456789abcdef" for character in value)


def require(condition: bool, message: str, *, path: str | None = None) -> None:
    if not condition:
        raise EvidenceError(message, path=path)


def exact_fields(value: Any, required: set[str], *, path: str) -> dict[str, Any]:
    require(isinstance(value, dict), "expected a JSON object", path=path)
    actual = set(value)
    missing = sorted(required - actual)
    extra = sorted(actual - required)
    require(not missing and not extra, f"object fields differ; missing={missing}, extra={extra}", path=path)
    return value


def nonempty_string(value: Any, *, path: str) -> str:
    require(isinstance(value, str) and bool(value.strip()), "expected a nonempty string", path=path)
    return value


class BundleReader:
    def __init__(self, root: Path, limit: int):
        try:
            self.root = root.resolve(strict=True)
        except OSError as error:
            raise EvidenceError(f"bundle directory is unavailable: {error}", path=str(root)) from error
        require(self.root.is_dir(), "bundle path is not a directory", path=str(root))
        self.limit = limit
        self.used_paths: set[Path] = set()
        self.total_bytes = 0

    def read_ref(self, reference: Any, *, role: str | None = None) -> tuple[bytes, dict[str, Any]]:
        allowed = {"path", "sha256"} if role is None else {"role", "path", "sha256"}
        reference = exact_fields(reference, allowed, path="reference")
        path_text = nonempty_string(reference["path"], path="reference.path")
        require("\\" not in path_text and ":" not in path_text, "evidence path must use relative POSIX components", path=path_text)
        pure = PurePosixPath(path_text)
        require(not pure.is_absolute() and all(part not in {"", ".", ".."} for part in path_text.split("/")), "evidence path escapes the bundle or is not canonical", path=path_text)
        current = self.root
        try:
            for component in pure.parts:
                current = current / component
                if current.is_symlink():
                    raise EvidenceError("evidence path contains a symbolic link", path=path_text)
            resolved = current.resolve(strict=True)
            require(resolved.is_relative_to(self.root) and resolved.is_file(), "evidence path is not a regular file inside the bundle", path=path_text)
            size = resolved.stat().st_size
            require(size <= MAX_RECORD_BYTES, "single evidence file exceeds the 16 MiB limit", path=path_text)
            if resolved not in self.used_paths:
                self.total_bytes += size
                self.used_paths.add(resolved)
                require(self.total_bytes <= self.limit, "evidence bundle exceeds the 512 MiB limit", path=path_text)
            data = resolved.read_bytes()
        except EvidenceError:
            raise
        except OSError as error:
            raise EvidenceError(f"cannot read evidence file: {error}", path=path_text) from error
        expected = reference["sha256"]
        require(is_sha256(expected), "invalid SHA-256 identity", path=path_text)
        require(sha256(data) == expected, "evidence hash does not match file bytes", path=path_text)
        if role is not None:
            require(reference["role"] == role, f"expected artifact role {role}", path=path_text)
        return data, reference

    def read_json_ref(self, reference: Any, *, role: str | None = None) -> tuple[Any, dict[str, Any]]:
        data, normalized = self.read_ref(reference, role=role)
        return strict_json(data, path=normalized["path"]), normalized


def load_contract() -> tuple[dict[str, Any], bytes]:
    try:
        data = CONTRACT_PATH.read_bytes()
    except OSError as error:
        raise EvidenceError(f"P10 contract is unavailable: {error}", path=str(CONTRACT_PATH)) from error
    value = strict_json(data, path="tools/p10/contract.json")
    exact_fields(value, {"schema_version", "target", "gates", "required_artifact_roles", "http_cases", "g4", "limits"}, path="contract")
    return value, data


def load_bundle(reader: BundleReader) -> dict[str, Any]:
    manifest_path = reader.root / "manifest.json"
    require(not manifest_path.is_symlink(), "manifest must not be a symbolic link", path="manifest.json")
    try:
        data = manifest_path.read_bytes()
    except OSError as error:
        raise EvidenceError(f"manifest.json is missing or unreadable: {error}", path="manifest.json") from error
    require(len(data) <= MAX_RECORD_BYTES, "manifest exceeds the 16 MiB limit", path="manifest.json")
    reader.total_bytes += len(data)
    reader.used_paths.add(manifest_path.resolve())
    require(reader.total_bytes <= reader.limit, "evidence bundle exceeds the 512 MiB limit", path="manifest.json")
    return strict_json(data, path="manifest.json")


def validate_manifest(manifest: Any, contract: dict[str, Any], contract_bytes: bytes) -> dict[str, Any]:
    manifest = exact_fields(
        manifest,
        {"schema_version", "kind", "source_commit", "target", "toolchain_lock_sha256", "predicates", "artifacts", "comparison"},
        path="manifest.json",
    )
    require(manifest["schema_version"] == "1.0.0" and manifest["kind"] == "p10_acceptance_bundle", "unsupported P10 evidence manifest")
    require(is_commit(manifest["source_commit"]), "source_commit must be a full lowercase Git commit hash", path="manifest.json")
    require(manifest["target"] == contract["target"], "evidence target differs from the P10 target", path="manifest.json")
    require(is_sha256(manifest["toolchain_lock_sha256"]), "invalid toolchain lock hash", path="manifest.json")
    predicate_names = {name for group in contract["gates"].values() for name in group}
    predicates = exact_fields(manifest["predicates"], predicate_names, path="manifest.predicates")
    require(isinstance(manifest["artifacts"], list), "artifacts must be an array", path="manifest.artifacts")
    comparison = exact_fields(manifest["comparison"], {"task_contract_sha256", "baseline_commit", "trials"}, path="manifest.comparison")
    require(comparison["task_contract_sha256"] == sha256(contract_bytes), "G4 task contract hash differs from the locked contract", path="manifest.comparison.task_contract_sha256")
    require(is_commit(comparison["baseline_commit"]), "baseline_commit must be a full lowercase Git commit hash", path="manifest.comparison.baseline_commit")
    require(isinstance(comparison["trials"], list), "comparison.trials must be an array", path="manifest.comparison.trials")
    return manifest


def read_artifacts(reader: BundleReader, references: Any, contract: dict[str, Any], *, owner: str) -> dict[str, tuple[str, bytes]]:
    require(isinstance(references, list), "artifacts must be an array", path=owner)
    result: dict[str, tuple[str, bytes]] = {}
    for reference in references:
        require(isinstance(reference, dict) and isinstance(reference.get("role"), str), "artifact role is required", path=owner)
        role = reference["role"]
        require(role not in result, f"duplicate artifact role: {role}", path=owner)
        data, normalized = reader.read_ref(reference, role=role)
        result[role] = (normalized["sha256"], data)
    return result


def require_elf_x86_64(data: bytes, *, path: str) -> None:
    require(len(data) >= 20 and data[:4] == b"\x7fELF", "native artifact is not an ELF file", path=path)
    require(data[4] == 2 and data[5] == 1, "native artifact must be 64-bit little-endian ELF", path=path)
    require(int.from_bytes(data[18:20], "little") == 62, "native artifact is not x86-64 ELF", path=path)


def check_provenance(record: dict[str, Any], manifest: dict[str, Any], *, path: str) -> None:
    require(record.get("schema_version") == "1.0.0", "unsupported evidence record schema", path=path)
    require(record.get("source_commit") == manifest["source_commit"], "record source commit differs from the bundle", path=path)
    require(record.get("target") == manifest["target"], "record target differs from the bundle", path=path)
    require(record.get("toolchain_lock_sha256") == manifest["toolchain_lock_sha256"], "record toolchain differs from the bundle", path=path)


def check_gate_record(
    reader: BundleReader,
    reference: Any,
    predicate: str,
    manifest: dict[str, Any],
    contract: dict[str, Any],
    top_artifacts: dict[str, tuple[str, bytes]],
) -> dict[str, Any]:
    record, normalized = reader.read_json_ref(reference)
    path = normalized["path"]
    required = {
        "schema_version", "kind", "predicate", "source_commit", "target", "toolchain_lock_sha256",
        "status", "command_id", "argv_sha256", "exit_code", "duration_ns", "test_suite", "artifacts", "assertions",
    }
    optional = {"execution", "target_generated_sources", "clean_rebuilds", "http", "transaction", "rollback", "diagnostics"}
    require(isinstance(record, dict), "gate record must be an object", path=path)
    missing = required - set(record)
    extra = set(record) - required - optional
    require(not missing and not extra, f"gate record fields differ; missing={sorted(missing)}, extra={sorted(extra)}", path=path)
    require(record["kind"] == "p10_gate_record" and record["predicate"] == predicate, "gate record identifies a different predicate", path=path)
    check_provenance(record, manifest, path=path)
    require(record["status"] == "PASSED" and record["exit_code"] == 0, "gate did not pass", path=path)
    nonempty_string(record["command_id"], path=f"{path}.command_id")
    require(is_sha256(record["argv_sha256"]), "command argument hash is invalid", path=path)
    require(type(record["duration_ns"]) is int and record["duration_ns"] > 0, "gate duration must be measured and positive", path=path)
    suite = exact_fields(record["test_suite"], {"id", "exit_code", "count"}, path=f"{path}.test_suite")
    nonempty_string(suite["id"], path=f"{path}.test_suite.id")
    require(suite["exit_code"] == 0 and type(suite["count"]) is int and suite["count"] > 0, "gate has no passing executed test cases", path=path)
    assertions = record["assertions"]
    require(isinstance(assertions, list), "assertions must be an array", path=path)
    assertion_ids: set[str] = set()
    for assertion in assertions:
        row = exact_fields(assertion, {"id", "passed"}, path=f"{path}.assertions")
        identity = nonempty_string(row["id"], path=f"{path}.assertions.id")
        require(identity not in assertion_ids and type(row["passed"]) is bool, "duplicate or invalid assertion", path=path)
        assertion_ids.add(identity)
        require(row["passed"], f"assertion did not pass: {identity}", path=path)
    require(predicate in assertion_ids, "gate record does not attest its named predicate", path=path)
    artifacts = read_artifacts(reader, record["artifacts"], contract, owner=f"{path}.artifacts")
    if predicate == "native_elf_generated":
        require("native_elf" in artifacts, "native ELF gate is missing the executable bytes", path=path)
        require_elf_x86_64(artifacts["native_elf"][1], path=path)
    elif predicate == "clean_environment_runs":
        execution = exact_fields(record.get("execution"), {"empty_environment", "exit_code", "executable_sha256", "stdout_sha256"}, path=f"{path}.execution")
        require(execution["empty_environment"] is True and execution["exit_code"] == 0, "clean-environment execution was not successful", path=path)
        require(execution["executable_sha256"] == top_artifacts["native_elf"][0], "executed binary differs from the release ELF", path=path)
        require(is_sha256(execution["stdout_sha256"]), "clean-environment stdout hash is invalid", path=path)
    elif predicate == "no_high_level_target_source":
        require(record.get("target_generated_sources") == [], "high-level target source was emitted", path=path)
    elif predicate == "reproducible_hash":
        rebuilds = record.get("clean_rebuilds")
        require(isinstance(rebuilds, list) and len(rebuilds) == 2, "reproducibility requires exactly two clean builds", path=path)
        identities: set[str] = set()
        hash_sets = []
        for index, build in enumerate(rebuilds):
            build_path = f"{path}.clean_rebuilds[{index}]"
            build = exact_fields(build, {"store_id", "clean", "empty_environment", "exit_code", "execution_exit_code", "artifacts"}, path=build_path)
            store_id = nonempty_string(build["store_id"], path=f"{build_path}.store_id")
            require(store_id not in identities, "clean builds did not use separate stores", path=build_path)
            identities.add(store_id)
            require(build["clean"] is True and build["empty_environment"] is True and build["exit_code"] == 0 and build["execution_exit_code"] == 0, "clean rebuild or execution failed", path=build_path)
            built = read_artifacts(reader, build["artifacts"], contract, owner=f"{build_path}.artifacts")
            require("native_elf" in built, "clean build omitted its ELF artifact", path=build_path)
            require_elf_x86_64(built["native_elf"][1], path=build_path)
            hash_sets.append({role: identity for role, (identity, _) in built.items()})
        require(hash_sets[0] == hash_sets[1], "independent clean builds produced different artifact hashes", path=path)
    elif predicate == "blackbox_tests_external":
        http = exact_fields(record.get("http"), {"client", "cases"}, path=f"{path}.http")
        require(http["client"] == "external_tcp", "HTTP evidence did not use an external TCP client", path=path)
        cases = http["cases"]
        require(isinstance(cases, list), "HTTP cases must be an array", path=path)
        seen: set[str] = set()
        for case in cases:
            case = exact_fields(case, {"id", "exit_code", "count", "receipt"}, path=f"{path}.http.cases")
            identity = nonempty_string(case["id"], path=f"{path}.http.cases.id")
            require(identity not in seen and case["exit_code"] == 0 and case["count"] == 1, "HTTP case receipt is invalid or failed", path=path)
            receipt, _ = reader.read_json_ref(case["receipt"], role=f"http_receipt:{identity}")
            receipt = exact_fields(receipt, {"schema_version", "kind", "case_id", "client", "server_executable_sha256", "request_sha256", "response_sha256", "exit_code", "elapsed_ns"}, path=f"{path}.http.{identity}")
            require(
                receipt["schema_version"] == "1.0.0" and receipt["kind"] == "http_exchange" and
                receipt["case_id"] == identity and receipt["client"] == "external_tcp" and
                receipt["server_executable_sha256"] == top_artifacts["native_elf"][0] and
                is_sha256(receipt["request_sha256"]) and is_sha256(receipt["response_sha256"]) and
                receipt["exit_code"] == 0 and type(receipt["elapsed_ns"]) is int and receipt["elapsed_ns"] > 0,
                "HTTP exchange receipt does not bind a real successful external request", path=path,
            )
            seen.add(identity)
        missing_cases = sorted(set(contract["http_cases"]) - seen)
        require(not missing_cases, f"HTTP blackbox evidence omits cases: {missing_cases}", path=path)
    elif predicate == "transaction_add_route_succeeds":
        transaction = exact_fields(record.get("transaction"), {"base_revision", "result_revision", "route_path", "route_sha256"}, path=f"{path}.transaction")
        require(type(transaction["base_revision"]) is int and transaction["result_revision"] == transaction["base_revision"] + 1, "route transaction did not create the next revision", path=path)
        require(transaction["route_path"] == "/hello/{name}" and is_sha256(transaction["route_sha256"]), "route transaction does not bind the expected route", path=path)
    elif predicate == "stale_transaction_rejected":
        transaction = exact_fields(record.get("transaction"), {"requested_base_revision", "observed_head_revision", "result_code", "head_before", "head_after"}, path=f"{path}.transaction")
        require(transaction["requested_base_revision"] < transaction["observed_head_revision"] and transaction["result_code"] == "E_STALE_REVISION", "stale revision was not rejected with its stable diagnostic", path=path)
        require(transaction["head_before"] == transaction["observed_head_revision"] == transaction["head_after"], "stale transaction changed HEAD", path=path)
    elif predicate == "failure_preserves_previous_revision":
        transaction = exact_fields(record.get("transaction"), {"base_revision", "head_before", "head_after", "base_graph_sha256", "candidate_sha256", "failure_code"}, path=f"{path}.transaction")
        require(transaction["base_revision"] == transaction["head_before"] == transaction["head_after"], "failed transaction changed the published revision", path=path)
        require(transaction["base_graph_sha256"] == top_artifacts["graph"][0] and is_sha256(transaction["candidate_sha256"]), "failed candidate is not bound to the preserved graph", path=path)
        require(transaction["candidate_sha256"] != transaction["base_graph_sha256"] and nonempty_string(transaction["failure_code"], path=f"{path}.transaction.failure_code"), "failed candidate evidence is incomplete", path=path)
    elif predicate == "all_artifacts_hashed":
        roles = {role for role, _ in artifacts.items()}
        require(set(contract["required_artifact_roles"]).issubset(roles), "artifact inventory omits a required hashed artifact", path=path)
    elif predicate == "diagnostics_machine_readable":
        rows = record.get("diagnostics")
        require(isinstance(rows, list) and rows, "diagnostic evidence is empty", path=path)
        for row in rows:
            row = exact_fields(row, {"diagnostic_id", "code", "stage", "entity_id"}, path=f"{path}.diagnostics")
            for name in row:
                nonempty_string(row[name], path=f"{path}.diagnostics.{name}")
    elif predicate == "rollback_verified":
        rollback = exact_fields(record.get("rollback"), {"source_revision", "rollback_revision", "expected_graph_sha256", "restored_graph_sha256", "previous_binary_sha256", "previous_binary_executed"}, path=f"{path}.rollback")
        require(type(rollback["source_revision"]) is int and type(rollback["rollback_revision"]) is int and rollback["rollback_revision"] > rollback["source_revision"], "rollback did not create a new monotonic revision", path=path)
        require(is_sha256(rollback["expected_graph_sha256"]) and rollback["restored_graph_sha256"] == rollback["expected_graph_sha256"], "rollback graph differs from its requested historical graph", path=path)
        require(rollback["previous_binary_sha256"] == top_artifacts["native_elf"][0] and rollback["previous_binary_executed"] is True, "previous verified binary was not rerun after rollback", path=path)
    return record


def patch_counts(data: bytes, *, path: str) -> dict[str, int]:
    try:
        lines = data.decode("utf-8").splitlines()
    except UnicodeDecodeError as error:
        raise EvidenceError("G4 patch is not UTF-8 text", path=path) from error
    require(not any(line.startswith("Binary files ") or line.startswith("GIT binary patch") for line in lines), "binary changes are not accepted in G4 source patches", path=path)
    files = [line for line in lines if line.startswith("diff --git ")]
    additions = sum(1 for line in lines if line.startswith("+") and not line.startswith("+++"))
    deletions = sum(1 for line in lines if line.startswith("-") and not line.startswith("---"))
    require(len(files) == len(set(files)), "G4 patch contains duplicate file sections", path=path)
    return {"files_changed": len(files), "insertions": additions, "deletions": deletions}


def check_trace(reader: BundleReader, reference: Any, run_id: str) -> int:
    trace, normalized = reader.read_json_ref(reference, role="trace")
    path = normalized["path"]
    trace = exact_fields(trace, {"schema_version", "kind", "run_id", "events"}, path=path)
    require(trace["schema_version"] == "1.0.0" and trace["kind"] == "g4_trace" and trace["run_id"] == run_id, "tool trace identity differs from trial", path=path)
    require(isinstance(trace["events"], list), "trace events must be an array", path=path)
    for index, event in enumerate(trace["events"], 1):
        event = exact_fields(event, {"sequence", "kind", "tool", "request_sha256", "response_sha256"}, path=f"{path}.events[{index - 1}]")
        require(event["sequence"] == index and event["kind"] == "tool_call", "trace sequence or event kind is invalid", path=path)
        nonempty_string(event["tool"], path=f"{path}.events[{index - 1}].tool")
        require(is_sha256(event["request_sha256"]) and is_sha256(event["response_sha256"]), "trace call does not bind request and response bytes", path=path)
    return len(trace["events"])


def verify_trials(reader: BundleReader, manifest: dict[str, Any], contract: dict[str, Any]) -> dict[str, Any]:
    comparison = manifest["comparison"]
    required_count = len(contract["g4"]["tasks"]) * len(contract["g4"]["implementations"]) * contract["g4"]["trial_count"]
    require(len(comparison["trials"]) == required_count, f"G4 requires {required_count} raw isolated trial records", path="manifest.comparison.trials")
    task_ids = set(contract["g4"]["tasks"])
    implementations = set(contract["g4"]["implementations"])
    trial_keys: set[tuple[str, str, int]] = set()
    run_ids: set[str] = set()
    workspaces: set[str] = set()
    task_inputs: dict[str, str] = {}
    trial_data: list[dict[str, Any]] = []
    for reference in comparison["trials"]:
        trial, normalized = reader.read_json_ref(reference)
        path = normalized["path"]
        fields = {
            "schema_version", "kind", "task_id", "implementation", "trial_index", "run_id", "workspace_id", "starting_commit",
            "task_input_sha256", "started_monotonic_ns", "finished_monotonic_ns", "experiment_exit_code", "failure_category", "assertions", "trace", "patch",
        }
        trial = exact_fields(trial, fields, path=path)
        require(trial["schema_version"] == "1.0.0" and trial["kind"] == "g4_trial", "unsupported G4 trial record", path=path)
        task_id = trial["task_id"]
        implementation = trial["implementation"]
        index = trial["trial_index"]
        require(task_id in task_ids and implementation in implementations and type(index) is int and 1 <= index <= contract["g4"]["trial_count"], "unknown G4 task, implementation or trial index", path=path)
        key = (task_id, implementation, index)
        require(key not in trial_keys, "duplicate G4 task/implementation/trial", path=path)
        trial_keys.add(key)
        run_id = nonempty_string(trial["run_id"], path=f"{path}.run_id")
        workspace = nonempty_string(trial["workspace_id"], path=f"{path}.workspace_id")
        require(run_id not in run_ids and workspace not in workspaces, "G4 trials reused a run or isolation workspace", path=path)
        run_ids.add(run_id)
        workspaces.add(workspace)
        expected_commit = comparison["baseline_commit"] if implementation == "rust" else manifest["source_commit"]
        require(trial["starting_commit"] == expected_commit, "trial started from the wrong pinned source revision", path=path)
        require(is_sha256(trial["task_input_sha256"]), "trial task input hash is invalid", path=path)
        prior_input = task_inputs.setdefault(task_id, trial["task_input_sha256"])
        require(prior_input == trial["task_input_sha256"], "paired trials used different task inputs", path=path)
        started = trial["started_monotonic_ns"]
        finished = trial["finished_monotonic_ns"]
        require(type(started) is int and type(finished) is int and started >= 0 and finished > started, "trial monotonic duration is missing or invalid", path=path)
        elapsed_ms = (finished - started) / 1_000_000
        assertion_rows = trial["assertions"]
        require(isinstance(assertion_rows, list), "trial assertions must be an array", path=path)
        assertion_by_id: dict[str, dict[str, Any]] = {}
        for row in assertion_rows:
            row = exact_fields(row, {"id", "exit_code", "count"}, path=f"{path}.assertions")
            identity = nonempty_string(row["id"], path=f"{path}.assertions.id")
            require(identity not in assertion_by_id, "duplicate G4 assertion", path=path)
            require((row["exit_code"] is None or type(row["exit_code"]) is int) and type(row["count"]) is int and row["count"] >= 0, "invalid G4 assertion execution result", path=path)
            assertion_by_id[identity] = row
        expected_assertions = set(contract["g4"]["tasks"][task_id])
        require(set(assertion_by_id) == expected_assertions, "G4 trial assertion set differs from the locked task", path=path)
        experiment_exit = trial["experiment_exit_code"]
        require(type(experiment_exit) is int, "trial experiment exit code is missing", path=path)
        trial_success = experiment_exit == 0 and all(row["exit_code"] == 0 and row["count"] > 0 for row in assertion_by_id.values())
        failure_category = trial["failure_category"]
        if trial_success:
            require(failure_category is None, "successful trial cannot carry a failure category", path=path)
        else:
            require(failure_category in contract["g4"]["failure_categories"], "failed trial lacks a registered failure category", path=path)
        tool_calls = check_trace(reader, trial["trace"], run_id)
        patch, patch_ref = reader.read_ref(trial["patch"], role="patch")
        changes = patch_counts(patch, path=patch_ref["path"])
        trial_data.append({
            "task_id": task_id,
            "implementation": implementation,
            "trial_index": index,
            "run_id": run_id,
            "workspace_id": workspace,
            "success": trial_success,
            "failure_category": failure_category,
            "elapsed_ms": elapsed_ms,
            "tool_calls": tool_calls,
            "changes": changes,
            "trial_record_sha256": normalized["sha256"],
            "patch_sha256": patch_ref["sha256"],
        })
    expected_keys = {
        (task, implementation, trial_index)
        for task in task_ids
        for implementation in implementations
        for trial_index in range(1, contract["g4"]["trial_count"] + 1)
    }
    require(trial_keys == expected_keys, "G4 trial matrix is incomplete", path="manifest.comparison.trials")
    summaries: dict[str, Any] = {}
    failure_distribution = {impl: {category: 0 for category in contract["g4"]["failure_categories"]} for impl in implementations}
    for task_id in sorted(task_ids):
        summaries[task_id] = {}
        for implementation in sorted(implementations):
            rows = [row for row in trial_data if row["task_id"] == task_id and row["implementation"] == implementation]
            elapsed = [row["elapsed_ms"] for row in rows]
            calls = [row["tool_calls"] for row in rows]
            summaries[task_id][implementation] = {
                "attempts": len(rows),
                "successes": sum(row["success"] for row in rows),
                "success_rate": sum(row["success"] for row in rows) / len(rows),
                "median_elapsed_ms": statistics.median(elapsed),
                "mean_tool_calls": statistics.fmean(calls),
                "mean_change_size": {
                    key: statistics.fmean(row["changes"][key] for row in rows)
                    for key in ("files_changed", "insertions", "deletions")
                },
            }
            for row in rows:
                if not row["success"]:
                    failure_distribution[implementation][row["failure_category"]] += 1
    return {
        "tasks": summaries,
        "failure_distribution": failure_distribution,
        "trials": trial_data,
    }


def _check_expected_inputs(
    manifest: dict[str, Any],
    *,
    expected_source_commit: str | None,
    expected_toolchain_lock_sha256: str | None,
) -> None:
    if expected_source_commit is not None:
        require(manifest["source_commit"] == expected_source_commit, "bundle source commit differs from current HEAD", path="manifest.json.source_commit")
    if expected_toolchain_lock_sha256 is not None:
        require(manifest["toolchain_lock_sha256"] == expected_toolchain_lock_sha256, "bundle toolchain lock differs from the checked-in toolchain.lock", path="manifest.json.toolchain_lock_sha256")


def validate_bundle_structure(
    root: Path,
    *,
    expected_source_commit: str | None = None,
    expected_toolchain_lock_sha256: str | None = None,
) -> dict[str, Any]:
    try:
        contract, contract_bytes = load_contract()
        reader = BundleReader(root, contract["limits"]["bundle_bytes"])
        manifest = validate_manifest(load_bundle(reader), contract, contract_bytes)
        _check_expected_inputs(
            manifest,
            expected_source_commit=expected_source_commit,
            expected_toolchain_lock_sha256=expected_toolchain_lock_sha256,
        )
        top_artifacts = read_artifacts(reader, manifest["artifacts"], contract, owner="manifest.artifacts")
        require(set(contract["required_artifact_roles"]).issubset(top_artifacts), "bundle omits one or more required P10 artifacts", path="manifest.artifacts")
        require(top_artifacts["toolchain_lock"][0] == manifest["toolchain_lock_sha256"], "toolchain_lock artifact hash differs from the manifest", path="manifest.artifacts")
        require_elf_x86_64(top_artifacts["native_elf"][1], path="manifest.artifacts.native_elf")
        used_predicate_paths: set[str] = set()
        checked_records: dict[str, list[str]] = {}
        for group, predicates in contract["gates"].items():
            checked_records[group] = []
            for predicate in predicates:
                reference = manifest["predicates"][predicate]
                require(reference["path"] not in used_predicate_paths, "one record cannot attest multiple predicates", path=reference["path"])
                used_predicate_paths.add(reference["path"])
                check_gate_record(reader, reference, predicate, manifest, contract, top_artifacts)
                checked_records[group].append(predicate)
        comparison = verify_trials(reader, manifest, contract)
        return {
            "ok": True,
            "result": "evidence_structure_validated",
            "status": "PROVISIONAL",
            "acceptance_verified": False,
            "provenance_verified": False,
            "provenance_limitation": (
                "caller-authored status fields and hashes establish bundle consistency only; "
                "they do not prove commands ran or artifacts came from those runs"
            ),
            "task_status_unchanged": True,
            "structurally_checked_predicate_records": checked_records,
            "claimed_comparison": {
                "tasks": comparison["tasks"],
                "failure_distribution": comparison["failure_distribution"],
                "trial_count": len(comparison["trials"]),
            },
            "bundle_bytes": reader.total_bytes,
        }
    except EvidenceError as error:
        return {
            "ok": False,
            "code": error.code,
            "status": "BLOCKED" if error.code == "E_EVIDENCE_INCOMPLETE" else "FAILED",
            "diagnostics": [{"code": error.code, "message": str(error), "path": error.path}],
        }
    except (KeyError, TypeError, ValueError, OSError) as error:
        return {
            "ok": False,
            "code": "E_EVIDENCE_INCOMPLETE",
            "status": "BLOCKED",
            "diagnostics": [{"code": "E_EVIDENCE_INCOMPLETE", "message": f"malformed P10 evidence: {error}", "path": None}],
        }


def _git_value(*arguments: str) -> str:
    result = subprocess.run(
        ["git", *arguments],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
        text=True,
        timeout=10,
    )
    if result.returncode != 0:
        raise EvidenceError("cannot establish repository provenance for P10 evidence", path="repository")
    return result.stdout.strip()


def run_cli(bundle: Path) -> int:
    try:
        if _git_value("status", "--porcelain"):
            raise EvidenceError("P10 evidence verification requires a clean source checkout", path="repository")
        source_commit = _git_value("rev-parse", "HEAD")
        lock_data = (ROOT / "toolchain.lock").read_bytes()
        result = validate_bundle_structure(
            bundle,
            expected_source_commit=source_commit,
            expected_toolchain_lock_sha256=sha256(lock_data),
        )
    except (EvidenceError, OSError, subprocess.SubprocessError) as error:
        result = {
            "ok": False,
            "code": "E_EVIDENCE_INCOMPLETE",
            "status": "BLOCKED",
            "diagnostics": [{"code": "E_EVIDENCE_INCOMPLETE", "message": str(error), "path": getattr(error, "path", None)}],
        }
    print(json.dumps(result, sort_keys=True, separators=(",", ":")))
    return 0 if result["ok"] else 1


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, required=True, help="directory containing manifest.json and immutable evidence files")
    options = parser.parse_args(argv)
    return run_cli(options.bundle)


if __name__ == "__main__":
    raise SystemExit(main())
