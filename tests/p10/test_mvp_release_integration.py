"""Integration test for P10 MVP release manifest generation."""

from __future__ import annotations

import json
from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest

from tools.p10.produce import emit_release_manifests


ROOT = Path(__file__).resolve().parents[2]


@pytest.fixture
def mock_p10_bundle(tmp_path):
    """Create a mock P10 acceptance bundle for testing."""
    # Create contract.json first
    contract = {
        "target": "x86_64-unknown-linux-gnu",
        "release_executable_role": "application_release",
        "required_artifact_roles": ["graph", "compiler", "runtime", "toolchain_lock"],
        "gates": {
            "G0": ["graph_schema_valid", "graph_loader_deterministic", "graph_semantics_accepted", "core_types_consistent"],
            "G1": ["execution_accepted", "native_backend_accepted", "runtime_accepted", "network_protocol_accepted",
                   "http_protocol_accepted", "ai_protocol_accepted", "interpreter_profiles_match"],
            "G2": ["no_high_level_target_source", "http_blackbox_tests_pass"],
            "G3": ["ai_transaction_repair_passes", "rollback_verified"],
            "G4": ["reproducible_from_source", "all_artifacts_hashed"],
        },
        "g4": {
            "tasks": {"create_todo": {}, "list_todos": {}},
            "implementations": ["graph_mutation", "reflection_based_repair", "guided_transaction"],
            "trial_count": 3,
        },
    }

    tools_dir = tmp_path / "tools" / "p10"
    tools_dir.mkdir(parents=True)
    (tools_dir / "contract.json").write_text(json.dumps(contract, ensure_ascii=False, indent=2))

    # Copy release.py to the test environment
    import shutil
    release_src = ROOT / "tools" / "p10" / "release.py"
    if release_src.is_file():
        shutil.copy2(release_src, tools_dir / "release.py")

    all_predicates = {
        "graph_schema_valid": {"path": "gates/g0_graph_schema.json", "sha256": "sha256:" + "a" * 64},
        "graph_loader_deterministic": {"path": "gates/g0_loader.json", "sha256": "sha256:" + "b" * 64},
        "graph_semantics_accepted": {"path": "gates/g0_semantics.json", "sha256": "sha256:" + "c" * 64},
        "core_types_consistent": {"path": "gates/g0_types.json", "sha256": "sha256:" + "d" * 64},
        "execution_accepted": {"path": "gates/g1_execution.json", "sha256": "sha256:" + "e" * 64},
        "native_backend_accepted": {"path": "gates/g1_native.json", "sha256": "sha256:" + "f" * 64},
        "runtime_accepted": {"path": "gates/g1_runtime.json", "sha256": "sha256:" + "1" * 64},
        "network_protocol_accepted": {"path": "gates/g1_network.json", "sha256": "sha256:" + "2" * 64},
        "http_protocol_accepted": {"path": "gates/g1_http.json", "sha256": "sha256:" + "3" * 64},
        "ai_protocol_accepted": {"path": "gates/g1_ai.json", "sha256": "sha256:" + "4" * 64},
        "interpreter_profiles_match": {"path": "gates/g1_profiles.json", "sha256": "sha256:" + "5" * 64},
        "no_high_level_target_source": {"path": "gates/g2_native_cli.json", "sha256": "sha256:" + "6" * 64},
        "http_blackbox_tests_pass": {"path": "gates/g2_http.json", "sha256": "sha256:" + "7" * 64},
        "ai_transaction_repair_passes": {"path": "gates/g3_repair.json", "sha256": "sha256:" + "8" * 64},
        "rollback_verified": {"path": "gates/g3_rollback.json", "sha256": "sha256:" + "9" * 64},
        "reproducible_from_source": {"path": "gates/g4_repro.json", "sha256": "sha256:" + "0" * 64},
        "all_artifacts_hashed": {"path": "gates/g4_inventory.json", "sha256": "sha256:" + "aa" * 32},
    }

    bundle = {
        "schema_version": "1.0.0",
        "kind": "p10_acceptance_bundle",
        "source_commit": "abc123" + "0" * 34,
        "target": "x86_64-unknown-linux-gnu",
        "toolchain_lock_sha256": "sha256:" + "bb" * 32,
        "executables": [{
            "role": "application_release",
            "kind": "application",
            "profile": "release",
            "target": "x86_64-unknown-linux-gnu",
            "entry_point": "demo.serve",
            "graph_revision": 100,
            "graph_sha256": "sha256:" + "cc" * 32,
            "runtime_profile": "full",
        }],
        "predicates": all_predicates,
        "artifacts": [
            {"role": "graph", "path": "build/graph.json", "sha256": "sha256:" + "dd" * 32},
            {"role": "compiler", "path": "target/release/il", "sha256": "sha256:" + "ee" * 32},
            {"role": "runtime", "path": "target/release/libil_native_runtime.a", "sha256": "sha256:" + "ff" * 32},
            {"role": "native_elf", "path": "build/app.elf", "sha256": "sha256:" + "11" * 32},
            {"role": "toolchain_lock", "path": "toolchain.lock", "sha256": "sha256:" + "bb" * 32},
            {"role": "application_release", "path": "http/app_release", "sha256": "sha256:" + "22" * 32},
        ],
        "comparison": {
            "trials": [
                {"path": f"g4/trial_{i}.json", "sha256": f"sha256:{'33' * 32}"}
                for i in range(18)
            ]
        },
    }

    # Create run directory structure
    run_dir = tmp_path / "p10_run"
    p10_dir = run_dir / "p10"
    p10_dir.mkdir(parents=True)

    # Write acceptance bundle
    bundle_path = p10_dir / "acceptance_bundle.json"
    bundle_path.write_text(json.dumps(bundle, ensure_ascii=False, indent=2))

    # Create G4 comparison report
    g4_dir = run_dir / "g4"
    g4_dir.mkdir(parents=True)
    comparison = {
        "schema_version": "1.0.0",
        "kind": "g4_comparison",
        "trials_count": 18,
        "all_trials_success": True,
    }
    (g4_dir / "comparison.json").write_text(json.dumps(comparison))

    return run_dir


def test_emit_release_manifests_success(mock_p10_bundle, tmp_path):
    """Test successful release manifest generation from complete P10 run."""
    output_dir = tmp_path / "release"
    eval_dir = tmp_path / "eval"

    result = emit_release_manifests(
        mock_p10_bundle,
        output_dir,
        root=tmp_path,
    )

    # Print result for debugging
    if not result["ok"]:
        print(f"Error: {result.get('error')}")

    assert result["ok"] is True, f"Release generation failed: {result.get('error')}"
    assert result["status"] == "COMPLETE"
    assert "outputs" in result

    # Verify release manifests were created
    assert (output_dir / "mvp_manifest.json").is_file()
    assert (output_dir / "mvp_binary_hashes.json").is_file()
    assert (output_dir / "known_limits.json").is_file()
    assert (eval_dir / "mvp_comparison.json").is_file()

    # Verify MVP manifest content
    mvp_manifest = json.loads((output_dir / "mvp_manifest.json").read_text())
    assert mvp_manifest["release"] == "il-mvp"
    assert mvp_manifest["language_spec_version"] == "1.0.0"
    assert mvp_manifest["verified_revision"] == 100
    assert all(gate == "PASSED" for gate in mvp_manifest["gates"].values())
    assert mvp_manifest["reproducibility"]["clean_rebuild"] is True

    # Verify binary hashes content
    binary_hashes = json.loads((output_dir / "mvp_binary_hashes.json").read_text())
    assert binary_hashes["schema_version"] == "1.0.0"
    assert binary_hashes["target"] == "x86_64-unknown-linux-gnu"
    assert "compiler" in binary_hashes["binaries"]
    assert "runtime" in binary_hashes["binaries"]

    # Verify known limits content
    known_limits = json.loads((output_dir / "known_limits.json").read_text())
    assert known_limits["schema_version"] == "1.0.0"
    assert known_limits["limits"]["http"]["protocol"] == "HTTP/1.1_subset"
    assert known_limits["limits"]["runtime"]["concurrency"] == "none"


def test_emit_release_manifests_missing_bundle(tmp_path):
    """Test release manifest generation fails when bundle is missing."""
    run_dir = tmp_path / "empty_run"
    run_dir.mkdir()
    output_dir = tmp_path / "release"

    result = emit_release_manifests(run_dir, output_dir, root=tmp_path)

    assert result["ok"] is False
    assert result["status"] == "BLOCKED"
    assert "acceptance bundle not found" in result["error"]


def test_emit_release_manifests_missing_g4_comparison(mock_p10_bundle, tmp_path):
    """Test release manifest generation fails when G4 comparison is missing."""
    # Remove G4 comparison
    (mock_p10_bundle / "g4" / "comparison.json").unlink()

    output_dir = tmp_path / "release"
    result = emit_release_manifests(mock_p10_bundle, output_dir, root=tmp_path)

    assert result["ok"] is False
    assert result["status"] == "BLOCKED"
    assert "G4 comparison report not found" in result["error"]
