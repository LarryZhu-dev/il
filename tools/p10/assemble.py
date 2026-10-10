#!/usr/bin/env python3
"""Assemble a successful trusted P10 run into a portable evidence bundle."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import tempfile
from typing import Any


HASH_PREFIX = "sha256:"
TARGET = "x86_64-unknown-linux-gnu"
REQUIRED_COMMANDS = {"ci", "http", "g4", "reproducibility"}


class AssembleError(RuntimeError):
    pass


def digest(data: bytes) -> str:
    return HASH_PREFIX + hashlib.sha256(data).hexdigest()


def is_hash(value: Any) -> bool:
    return (isinstance(value, str) and len(value) == 71 and value.startswith(HASH_PREFIX)
            and all(char in "0123456789abcdef" for char in value[7:]))


def is_commit(value: Any) -> bool:
    return isinstance(value, str) and len(value) == 40 and all(char in "0123456789abcdef" for char in value)


def load_json(path: Path) -> tuple[Any, bytes]:
    try:
        raw = path.read_bytes()

        def pairs(items: list[tuple[str, Any]]) -> dict[str, Any]:
            result: dict[str, Any] = {}
            for key, value in items:
                if key in result:
                    raise AssembleError(f"duplicate JSON key in {path}: {key}")
                result[key] = value
            return result

        def reject_constant(value: str) -> None:
            raise AssembleError(f"invalid JSON constant in {path}: {value}")

        return json.loads(raw.decode("utf-8"), object_pairs_hook=pairs, parse_constant=reject_constant), raw
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise AssembleError(f"invalid JSON {path}: {error}") from error


def canonical_relative(value: Any, field: str) -> str:
    if not isinstance(value, str) or not value or "\\" in value or ":" in value:
        raise AssembleError(f"{field} must be a canonical relative POSIX path")
    pure = PurePosixPath(value)
    if pure.is_absolute() or any(part in {"", ".", ".."} for part in value.split("/")):
        raise AssembleError(f"{field} escapes its root: {value!r}")
    return pure.as_posix()


def regular_file(root: Path, candidate: Path, label: str) -> Path:
    try:
        root = root.resolve(strict=True)
        if candidate.is_symlink():
            raise AssembleError(f"{label} is a symbolic link: {candidate}")
        resolved = candidate.resolve(strict=True)
        resolved.relative_to(root)
        if resolved.is_symlink() or not resolved.is_file():
            raise AssembleError(f"{label} is not a regular file: {candidate}")
        return resolved
    except (OSError, ValueError) as error:
        raise AssembleError(f"{label} escapes trusted run: {candidate}") from error


def atomic_write(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".pending", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass


def read_journal(path: Path) -> tuple[Path, dict[str, Any], bytes]:
    journal_path = path.resolve()
    if journal_path.name != "journal.json" or journal_path.is_symlink() or not journal_path.is_file():
        raise AssembleError("trusted run journal is missing or is a symbolic link")
    run = journal_path.parent.resolve()
    journal, raw = load_json(journal_path)
    if not isinstance(journal, dict) or journal.get("schema_version") != "1.0.0" or journal.get("kind") != "p10_trusted_run":
        raise AssembleError("unsupported trusted run journal")
    if journal.get("status") != "COMPLETE":
        raise AssembleError("only a COMPLETE trusted run may be assembled")
    source = journal.get("source")
    if (not isinstance(source, dict) or not is_commit(source.get("commit"))
            or not is_commit(source.get("tree")) or source.get("dirty") is True):
        raise AssembleError("journal source identity is incomplete or dirty")
    lock = (journal.get("toolchain") or {}).get("lock")
    if not isinstance(lock, dict) or not is_hash(lock.get("sha256")):
        raise AssembleError("journal toolchain lock identity is missing")
    commands = journal.get("commands")
    if (not isinstance(commands, list) or len(commands) != len(REQUIRED_COMMANDS)
            or {row.get("command_id") for row in commands if isinstance(row, dict)} != REQUIRED_COMMANDS):
        raise AssembleError("trusted journal must contain exactly ci, http, g4 and reproducibility commands")
    for row in commands:
        if not isinstance(row, dict) or set(row) != {"command_id", "path", "sha256"}:
            raise AssembleError("invalid command observation reference")
        canonical_relative(row["path"], "command observation path")
        if not is_hash(row["sha256"]):
            raise AssembleError("invalid command observation hash")
        observation_path = regular_file(run, run / row["path"], "command observation")
        if digest(observation_path.read_bytes()) != row["sha256"]:
            raise AssembleError(f"command observation hash mismatch: {row['command_id']}")
        observation, _ = load_json(observation_path)
        if (not isinstance(observation, dict) or observation.get("kind") != "command_observation"
                or observation.get("command_id") != row["command_id"] or observation.get("status") != "SUCCESS"):
            raise AssembleError(f"command {row['command_id']} is not a successful trusted observation")
    plan = journal.get("plan")
    planned = {row.get("command_id") for row in plan.get("commands", []) if isinstance(row, dict)} if isinstance(plan, dict) else set()
    if planned != REQUIRED_COMMANDS:
        raise AssembleError("journal command plan does not match the required P10 commands")
    return run, journal, raw


def find_report(run: Path) -> Path:
    """Find the acceptance report emitted by the reviewed CI command.

    The producer never accepts a caller supplied bundle/manifest.  A report is
    an ordinary command output and is only eligible when it is below the
    command output directory (``ci``), keeping the trust boundary explicit.
    """
    candidates: list[Path] = []
    for root in (run / "ci", run / "build" / "ci"):
        if root.is_dir():
            candidates.extend(sorted(root.glob("**/manifest.json")))
            candidates.extend(sorted(root.glob("**/p10_manifest.json")))
    for candidate in candidates:
        if candidate.is_symlink() or not candidate.is_file():
            continue
        value, _ = load_json(candidate)
        if isinstance(value, dict) and value.get("kind") == "p10_acceptance_bundle":
            return candidate.resolve()
    raise AssembleError("trusted CI output contains no p10 acceptance report")


def validate_seed(manifest: Any, journal: dict[str, Any]) -> dict[str, Any]:
    required = {"schema_version", "kind", "source_commit", "target", "toolchain_lock_sha256",
                "executables", "predicates", "artifacts", "comparison"}
    if (not isinstance(manifest, dict) or set(manifest) != required
            or manifest.get("schema_version") != "1.0.0"
            or manifest.get("kind") != "p10_acceptance_bundle" or manifest.get("target") != TARGET):
        raise AssembleError("unsupported P10 acceptance manifest")
    if manifest.get("source_commit") != journal["source"]["commit"]:
        raise AssembleError("P10 manifest source commit differs from trusted journal")
    if manifest.get("toolchain_lock_sha256") != journal["toolchain"]["lock"]["sha256"]:
        raise AssembleError("P10 manifest toolchain lock differs from trusted journal")
    comparison = manifest.get("comparison")
    if not isinstance(comparison, dict) or len(comparison.get("trials", [])) != 18:
        raise AssembleError("P10 manifest must contain exactly 18 G4 trial references")
    return manifest


class Rebasing:
    """Maintain one authoritative mapping from run files to bundle files."""

    def __init__(self, run: Path, destination: Path, report: Path):
        self.run = run
        self.destination = destination
        self.mapping: dict[Path, Path] = {}
        self.cache: dict[Path, bytes] = {}
        for item in run.rglob("*"):
            if item.is_symlink():
                raise AssembleError(f"trusted output contains a symbolic link: {item.relative_to(run)}")
            if item.is_file():
                source = item.resolve()
                target = destination / ("manifest.json" if source == report else "records" / source.relative_to(run))
                self.mapping[source] = target

    def resolve_reference(self, base: Path, text: str) -> Path:
        if os.path.isabs(text):
            candidate = Path(text)
        else:
            canonical_relative(text, "evidence reference path")
            candidate = base / Path(*text.split("/"))
            if not candidate.exists():
                candidate = self.run / Path(*text.split("/"))
        source = regular_file(self.run, candidate, "evidence reference")
        if source not in self.mapping:
            raise AssembleError(f"referenced evidence was not archived: {text}")
        return source

    def write_source(self, source: Path) -> bytes:
        source = regular_file(self.run, source, "evidence file")
        if source in self.cache:
            return self.cache[source]
        target = self.mapping[source]
        if source.suffix.lower() == ".json":
            value, _ = load_json(source)
            value = self.rewrite(value, source.parent)
            data = (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode("utf-8")
        else:
            data = source.read_bytes()
        self.cache[source] = data
        atomic_write(target, data)
        return data

    def rewrite(self, value: Any, base: Path) -> Any:
        if isinstance(value, list):
            return [self.rewrite(item, base) for item in value]
        if not isinstance(value, dict):
            return value
        result = {key: self.rewrite(item, base) for key, item in value.items()}
        if isinstance(value.get("path"), str) and is_hash(value.get("sha256")):
            source = self.resolve_reference(base, value["path"])
            original = source.read_bytes()
            if digest(original) != value["sha256"]:
                raise AssembleError(f"referenced evidence hash mismatch: {value['path']}")
            data = self.write_source(source)
            result["path"] = self.mapping[source].relative_to(self.destination).as_posix()
            result["sha256"] = digest(data)
        return result


def assemble(journal: str | Path, output: str | Path) -> dict[str, Any]:
    run, journal_value, journal_bytes = read_journal(Path(journal))
    destination = Path(output).resolve()
    marker = digest(journal_bytes)
    if destination.exists():
        marker_path = destination / ".assemble-source"
        if (marker_path.is_file() and marker_path.read_text(encoding="ascii").strip() == marker
                and (destination / "manifest.json").is_file()):
            value, _ = load_json(destination / "manifest.json")
            return value
        raise AssembleError(f"refusing to overwrite existing bundle: {destination}")
    report = find_report(run)
    report_value, _ = load_json(report)
    validate_seed(report_value, journal_value)
    destination.mkdir(parents=True)
    try:
        rebasing = Rebasing(run, destination, report)
        rebasing.write_source(report)
        final, _ = load_json(destination / "manifest.json")
        atomic_write(destination / ".assemble-source", (marker + "\n").encode("ascii"))
        return final
    except BaseException:
        shutil.rmtree(destination, ignore_errors=True)
        raise


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--journal", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        result = assemble(args.journal, args.output)
    except AssembleError as error:
        print(json.dumps({"ok": False, "status": "BLOCKED", "error": str(error)}, ensure_ascii=False))
        return 2
    print(json.dumps({"ok": True, "status": "ASSEMBLED", "manifest": str(args.output / "manifest.json"), "source_commit": result["source_commit"]}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
