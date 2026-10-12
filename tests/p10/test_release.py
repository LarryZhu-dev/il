"""Test P10 MVP release manifest generation."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

from tools.p10.release import (
    generate_mvp_manifest,
    generate_binary_hashes,
    generate_known_limits,
    ReleaseError,
)


ROOT = Path(__file__).resolve().parents[2]
CONTRACT = json.loads((ROOT / "tools" / "p10" / "contract.json").read_text(encoding="utf-8"))


def test_generate_mvp_manifest_requires_all_gates():
    """MVP manifest generation fails when any gate predicate is missing."""
    incomplete_bundle = {
        "schema_version": "1.0.0",
        "kind": "p10_acceptance_bundle",
        "source_commit": "a" * 40,
        "target": "x86_64-unknown-linux-gnu",
        "toolchain_lock_sha256": "sha256:" + "0" * 64,
        "executables": [{
            "role": "application_release",
            "profile": "release",
            "graph_revision": 0,
            "graph_sha256": "sha256:" + "1" * 64,
        }],
        "predicates": {
            "graph_schema_valid": {"path": "gates/g0.json", "sha256": "sha256:" + "2" * 64},
        },
        "artifacts": [],
        "comparison": {"trials": []},
    }

    with pytest.raises(ReleaseError, match="gates not satisfied"):
        generate_mvp_manifest(incomplete_bundle, CONTRACT)


def test_generate_mvp_manifest_complete():
    """MVP manifest generation succeeds with all gates present."""
    all_predicates = {
        name: {"path": f"gates/{name}.json", "sha256": f"sha256:{'0' * 64}"}
        for gate_preds in CONTRACT["gates"].values()
        for name in gate_preds
    }

    bundle = {
        "schema_version": "1.0.0",
        "kind": "p10_acceptance_bundle",
        "source_commit": "a" * 40,
        "target": "x86_64-unknown-linux-gnu",
        "toolchain_lock_sha256": "sha256:" + "b" * 64,
        "executables": [{
            "role": "application_release",
            "kind": "application",
            "profile": "release",
            "target": "x86_64-unknown-linux-gnu",
            "entry_point": "demo.serve",
            "graph_revision": 42,
            "graph_sha256": "sha256:" + "c" * 64,
            "runtime_profile": "full",
        }],
        "predicates": all_predicates,
        "artifacts": [
            {"role": "graph", "path": "repro/graph.json", "sha256": "sha256:" + "d" * 64},
            {"role": "compiler", "path": "repro/ilc", "sha256": "sha256:" + "e" * 64},
            {"role": "runtime", "path": "repro/ilrt", "sha256": "sha256:" + "f" * 64},
            {"role": "native_elf", "path": "repro/app", "sha256": "sha256:" + "1" * 64},
            {"role": "toolchain_lock", "path": "toolchain.lock", "sha256": "sha256:" + "b" * 64},
            {"role": "application_release", "path": "http/app_release", "sha256": "sha256:" + "2" * 64},
        ],
        "comparison": {"trials": []},
    }

    manifest = generate_mvp_manifest(bundle, CONTRACT)

    assert manifest["release"] == "il-mvp"
    assert manifest["language_spec_version"] == "1.0.0"
    assert manifest["target"] == "x86_64-unknown-linux-gnu"
    assert manifest["verified_revision"] == 42
    assert manifest["source_commit"] == "a" * 40
    assert all(gate == "PASSED" for gate in manifest["gates"].values())
    assert len(manifest["gates"]) == len(CONTRACT["gates"])
    assert manifest["reproducibility"]["clean_rebuild"] is True
    assert "single_request_then_close" in manifest["known_limits"]


def test_generate_mvp_manifest_missing_release_executable():
    """MVP manifest generation fails without release executable."""
    all_predicates = {
        name: {"path": f"gates/{name}.json", "sha256": f"sha256:{'0' * 64}"}
        for gate_preds in CONTRACT["gates"].values()
        for name in gate_preds
    }

    bundle = {
        "schema_version": "1.0.0",
        "kind": "p10_acceptance_bundle",
        "source_commit": "a" * 40,
        "target": "x86_64-unknown-linux-gnu",
        "toolchain_lock_sha256": "sha256:" + "b" * 64,
        "executables": [{
            "role": "application_debug",
            "profile": "debug",
        }],
        "predicates": all_predicates,
        "artifacts": [],
        "comparison": {"trials": []},
    }

    with pytest.raises(ReleaseError, match="release executable role .* not found"):
        generate_mvp_manifest(bundle, CONTRACT)


def test_generate_binary_hashes():
    """Binary hashes manifest contains all application and core artifacts."""
    bundle = {
        "source_commit": "a" * 40,
        "target": "x86_64-unknown-linux-gnu",
        "toolchain_lock_sha256": "sha256:" + "b" * 64,
        "artifacts": [
            {"role": "compiler", "path": "build/ilc", "sha256": "sha256:" + "c" * 64},
            {"role": "runtime", "path": "build/ilrt", "sha256": "sha256:" + "d" * 64},
            {"role": "native_elf", "path": "build/app", "sha256": "sha256:" + "e" * 64},
            {"role": "application_debug", "path": "http/debug", "sha256": "sha256:" + "f" * 64},
            {"role": "application_release", "path": "http/release", "sha256": "sha256:" + "1" * 64},
            {"role": "graph", "path": "graph.json", "sha256": "sha256:" + "2" * 64},
        ],
    }

    hashes = generate_binary_hashes(bundle, CONTRACT)

    assert hashes["schema_version"] == "1.0.0"
    assert hashes["target"] == "x86_64-unknown-linux-gnu"
    assert hashes["source_commit"] == "a" * 40
    assert "compiler" in hashes["binaries"]
    assert "runtime" in hashes["binaries"]
    assert "native_elf" in hashes["binaries"]
    assert "application_debug" in hashes["binaries"]
    assert "application_release" in hashes["binaries"]
    assert "graph" not in hashes["binaries"]  # Not a binary


def test_generate_known_limits():
    """Known limits manifest matches specification requirements."""
    limits = generate_known_limits(CONTRACT)

    assert limits["schema_version"] == "1.0.0"
    assert limits["target"] == CONTRACT["target"]
    assert limits["limits"]["http"]["protocol"] == "HTTP/1.1_subset"
    assert limits["limits"]["http"]["connection"] == "one_request_then_close"
    assert limits["limits"]["http"]["request_line_max_bytes"] == 4096
    assert limits["limits"]["runtime"]["concurrency"] == "none"
    assert limits["limits"]["runtime"]["async"] == "not_implemented"
    assert limits["limits"]["language"]["generics"] == "not_implemented"
