"""Derive the P10 reproducible-hash gate from a retained rebuild report.

The adapter does not run builds or copy artifact bytes.  It validates the
report's source identity, independent clean-store claims, artifact hashes,
cross-root equality, and native application execution receipts, then emits
references to the actual artifact files in the trusted run.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
from typing import Any


class ReproductionRecordError(ValueError):
    """The rebuild report cannot attest reproducibility."""


def _digest(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


def _canonical(value: Any) -> bytes:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")


def _read_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ReproductionRecordError(f"cannot read reproduction report: {error}") from error
    if not isinstance(value, dict):
        raise ReproductionRecordError("reproduction report is not an object")
    return value


def _under(root: Path, text: Any, field: str, *, file: bool = True) -> Path:
    if not isinstance(text, str) or not text:
        raise ReproductionRecordError(f"{field} path is missing")
    candidate = Path(text)
    if candidate.is_absolute():
        lexical = candidate
    else:
        if "\\" in text or ":" in text:
            raise ReproductionRecordError(f"{field} path is not a canonical relative POSIX path")
        pure = PurePosixPath(text)
        if pure.is_absolute() or any(part in {"", ".", ".."} for part in text.split("/")):
            raise ReproductionRecordError(f"{field} path is not canonical")
        lexical = root.joinpath(*pure.parts)
    try:
        lexical.absolute().relative_to(root)
        resolved = lexical.resolve(strict=True)
        resolved.relative_to(root)
    except (OSError, ValueError) as error:
        raise ReproductionRecordError(f"{field} is missing or escapes the trusted report root") from error
    if file and not resolved.is_file():
        raise ReproductionRecordError(f"{field} is not a regular file")
    if not file and not resolved.is_dir():
        raise ReproductionRecordError(f"{field} is not a directory")
    cursor = root
    for part in lexical.absolute().relative_to(root).parts:
        cursor = cursor / part
        if cursor.is_symlink():
            raise ReproductionRecordError(f"{field} traverses a symbolic link")
    return resolved


def _artifact(root: Path, row: Any, field: str, role: str) -> dict[str, str]:
    if not isinstance(row, dict):
        raise ReproductionRecordError(f"{field} artifact metadata is missing")
    path = _under(root, row.get("path"), field)
    data = path.read_bytes()
    actual = _digest(data)
    if row.get("sha256") != actual:
        raise ReproductionRecordError(f"{field} artifact hash does not match its bytes")
    if type(row.get("size_bytes")) is not int or row["size_bytes"] != len(data):
        raise ReproductionRecordError(f"{field} artifact size does not match its bytes")
    return {"role": role, "path": path.relative_to(root).as_posix(), "sha256": actual}


def _build_seconds(build: dict[str, Any], label: str) -> int:
    started, ended = build.get("started_monotonic_ns"), build.get("ended_monotonic_ns")
    if type(started) is not int or type(ended) is not int or started <= 0 or ended <= started:
        raise ReproductionRecordError(f"{label} build has no valid measured duration")
    return ended - started


def _root_record(root: Path, report_root: Path, row: Any, label: str, expected_tree: str,
                 expected_commit: str) -> tuple[dict[str, Any], dict[str, str], int]:
    if not isinstance(row, dict) or row.get("label") != label:
        raise ReproductionRecordError(f"expected exactly the {label} independent build")
    materialized = row.get("materialized")
    if not isinstance(materialized, dict) or materialized.get("tree") != expected_tree:
        raise ReproductionRecordError(f"{label} source tree differs from the committed source")
    archive_hash = materialized.get("archive_sha256")
    if not isinstance(archive_hash, str) or not re.fullmatch(r"sha256:[0-9a-f]{64}", archive_hash):
        raise ReproductionRecordError(f"{label} source archive hash is invalid")
    build = row.get("build")
    if not isinstance(build, dict) or build.get("exit_code") != 0 or build.get("error") is not None:
        raise ReproductionRecordError(f"{label} bootstrap build did not succeed")
    argv = build.get("argv")
    if (not isinstance(argv, list) or not argv or not all(isinstance(part, str) for part in argv)
            or argv[-4:] != ["build", "--workspace", "--release", "--locked"]):
        raise ReproductionRecordError(f"{label} was not built with the locked release workspace command")
    env = build.get("env")
    target_dir = env.get("CARGO_TARGET_DIR") if isinstance(env, dict) else None
    if not isinstance(target_dir, str) or not target_dir:
        raise ReproductionRecordError(f"{label} build does not identify its isolated Cargo target store")
    _under(report_root, target_dir, f"{label} CARGO_TARGET_DIR", file=False)
    target_path = Path(target_dir).resolve()
    source_build = Path(build.get("cwd", "")).resolve()
    if source_build != _under(report_root, f"{label}/source", f"{label} source checkout", file=False):
        raise ReproductionRecordError(f"{label} build did not use its own materialized source checkout")
    if target_path != _under(report_root, f"{label}/target", f"{label} target store", file=False):
        raise ReproductionRecordError(f"{label} target store is outside its independent root")
    # Current reproduction.json does not record this precondition.  A unique
    # path is not evidence that it was empty before cargo started.
    if build.get("target_empty_before_build") is not True:
        raise ReproductionRecordError(f"{label} report lacks target_empty_before_build=true")
    if build.get("source_commit") not in (None, expected_commit):
        raise ReproductionRecordError(f"{label} build source commit differs from the report source")
    if build.get("log") is not None:
        log = build["log"]
        log_path = _under(report_root, log.get("path") if isinstance(log, dict) else None, f"{label} build log")
        if _digest(log_path.read_bytes()) != log.get("sha256"):
            raise ReproductionRecordError(f"{label} build log hash mismatch")
    elif not isinstance(build.get("stdout_sha256"), str) or not isinstance(build.get("stderr_sha256"), str):
        raise ReproductionRecordError(f"{label} build has no retained output hashes")

    artifacts = row.get("artifacts")
    if not isinstance(artifacts, dict) or not {"compiler", "runtime"}.issubset(artifacts):
        raise ReproductionRecordError(f"{label} compiler/runtime artifacts are incomplete")
    app = row.get("application")
    if not isinstance(app, dict) or not isinstance(app.get("artifacts"), dict):
        raise ReproductionRecordError(f"{label} application build artifacts are missing")
    execution = app.get("execution")
    if (not isinstance(execution, dict) or execution.get("clean_environment") is not True
            or execution.get("health_check_passed") is not True or execution.get("error") is not None
            or execution.get("termination_signal") != "SIGTERM"
            or execution.get("cleanup_intent") != "SIGTERM_after_health"
            or execution.get("cleanup_reaped") is not True):
        raise ReproductionRecordError(f"{label} native application did not pass its clean-environment health execution")
    exe = app["artifacts"].get("executable")
    if not isinstance(exe, dict) or execution.get("executable_sha256") != exe.get("sha256"):
        raise ReproductionRecordError(f"{label} execution receipt is not bound to its built executable")
    app_artifacts = app["artifacts"]
    if not app_artifacts:
        raise ReproductionRecordError(f"{label} application has no hashed native artifacts")
    refs: dict[str, dict[str, str]] = {}
    hashes: dict[str, str] = {}
    for name, item in sorted({**artifacts, **app_artifacts}.items()):
        role = "native_elf" if name == "executable" else name
        ref = _artifact(report_root, item, f"{label} {name}", role)
        if role in refs:
            raise ReproductionRecordError(f"{label} contains duplicate reproducibility artifact role {role}")
        refs[role] = ref
        hashes[role] = ref["sha256"]
    if execution.get("executable_sha256") != hashes.get("native_elf"):
        raise ReproductionRecordError(f"{label} health execution hash differs from retained native ELF bytes")
    return ({"store_id": label, "clean": True, "empty_environment": True,
             "exit_code": 0, "execution_exit_code": execution["exit_code"],
             "execution_termination_signal": execution["termination_signal"],
             "execution_cleanup_intent": execution["cleanup_intent"],
             "execution_cleanup_reaped": execution["cleanup_reaped"],
             "artifacts": list(refs.values())}, hashes, _build_seconds(build, label))


def derive_reproducible_hash(report: str | Path, *, provenance: dict[str, str],
                             output: str | Path | None = None) -> dict[str, Any]:
    """Return a reproducible_hash record only when the report proves every input."""
    blocked: dict[str, str] = {}
    records: dict[str, dict[str, Any]] = {}
    try:
        report_path = Path(report).resolve(strict=True)
        report_root = report_path.parent.resolve(strict=True)
        value = _read_json(report_path)
        commit = provenance.get("source_commit")
        target = provenance.get("target")
        lock_hash = provenance.get("toolchain_lock_sha256")
        if not isinstance(commit, str) or not re.fullmatch(r"[0-9a-f]{40}", commit):
            raise ReproductionRecordError("source_commit provenance is missing or invalid")
        if not isinstance(target, str) or not target:
            raise ReproductionRecordError("target provenance is missing")
        if not isinstance(lock_hash, str) or not re.fullmatch(r"sha256:[0-9a-f]{64}", lock_hash):
            raise ReproductionRecordError("toolchain lock hash provenance is missing or invalid")
        source = value.get("source")
        if value.get("kind") != "independent_rebuild" or value.get("status") != "PASSED":
            raise ReproductionRecordError("reproduction report is not a passed independent_rebuild")
        if not isinstance(source, dict) or source.get("commit") != commit:
            raise ReproductionRecordError("reproduction source commit differs from the exact bundle source")
        tree = source.get("tree")
        if not isinstance(tree, str) or not re.fullmatch(r"[0-9a-f]{40}", tree):
            raise ReproductionRecordError("reproduction source tree identity is missing or invalid")
        rows = value.get("roots")
        if not isinstance(rows, list) or len(rows) != 2:
            raise ReproductionRecordError("reproduction must contain exactly two independent roots")
        if target != "x86_64-unknown-linux-gnu":
            raise ReproductionRecordError("reproduction report only supports x86_64-unknown-linux-gnu")
        first, first_hashes, first_duration = _root_record(report_root, report_root, rows[0], "root_a", tree, commit)
        second, second_hashes, second_duration = _root_record(report_root, report_root, rows[1], "root_b", tree, commit)
        if first_hashes != second_hashes:
            raise ReproductionRecordError("independent roots produced different artifact hash sets")
        if rows[0]["materialized"]["archive_sha256"] != rows[1]["materialized"]["archive_sha256"]:
            raise ReproductionRecordError("independent roots were not materialized from identical source bytes")
        comparisons = value.get("comparisons")
        if not isinstance(comparisons, list):
            raise ReproductionRecordError("reproduction report has no artifact comparisons")
        observed = {row.get("artifact"): row for row in comparisons if isinstance(row, dict)}
        for role, identity in first_hashes.items():
            name = "executable" if role == "native_elf" else role
            comparison = observed.get(name)
            if not isinstance(comparison, dict) or comparison.get("left") != identity or comparison.get("right") != identity or comparison.get("equal") is not True:
                raise ReproductionRecordError(f"reproduction report lacks an equal comparison for artifact {name}")
        argv = rows[0]["build"]["argv"]
        duration = first_duration + second_duration
        record = {
            "schema_version": "1.0.0", "kind": "p10_gate_record", "predicate": "reproducible_hash",
            "source_commit": commit, "target": target, "toolchain_lock_sha256": lock_hash,
            "status": "PASSED", "command_id": "reproducibility",
            "argv_sha256": _digest(_canonical(argv)), "exit_code": 0, "duration_ns": duration,
            "test_suite": {"id": "independent_clean_rebuilds", "exit_code": 0, "count": 2},
            "artifacts": [],
            "assertions": [{"id": "reproducible_hash", "passed": True}],
            "clean_rebuilds": [first, second],
        }
        records["reproducible_hash"] = record
    except (OSError, ValueError, KeyError, TypeError) as error:
        blocked["reproducible_hash"] = str(error)
    result = {"schema_version": "1.0.0", "kind": "p10_reproducibility_derivation",
              "status": "COMPLETE" if not blocked else "BLOCKED", "records": records,
              "blocked_predicates": blocked}
    if output is not None:
        destination = Path(output)
        destination.mkdir(parents=True, exist_ok=True)
        for predicate, record in records.items():
            path = destination / f"{predicate}.json"
            data = (json.dumps(record, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode("utf-8")
            if path.exists() and path.read_bytes() != data:
                raise ReproductionRecordError(f"refusing to replace differing candidate record: {path}")
            if not path.exists():
                path.write_bytes(data)
        status_path = destination / "reproducibility_derivation.json"
        status = {key: item for key, item in result.items() if key != "records"}
        data = (json.dumps(status, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode("utf-8")
        if status_path.exists() and status_path.read_bytes() != data:
            raise ReproductionRecordError(f"refusing to replace differing derivation status: {status_path}")
        if not status_path.exists():
            status_path.write_bytes(data)
    return result


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--toolchain-lock-sha256", required=True)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args(argv)
    result = derive_reproducible_hash(args.report, provenance={
        "source_commit": args.source_commit, "target": args.target,
        "toolchain_lock_sha256": args.toolchain_lock_sha256,
    }, output=args.output)
    print(json.dumps({key: value for key, value in result.items() if key != "records"}, ensure_ascii=False))
    return 0 if result["status"] == "COMPLETE" else 2


if __name__ == "__main__":
    raise SystemExit(main())
