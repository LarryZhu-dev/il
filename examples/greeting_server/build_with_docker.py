#!/usr/bin/env python3
"""Build and run greeting_server using il-bootstrap Docker container.

This script:
1. Uses the il CLI from the Docker container
2. Initializes a graph repository
3. Publishes the greeting_server source code
4. Compiles it to a native binary
5. Runs the server
"""
import json
import subprocess
import sys
from pathlib import Path
import tempfile
import shutil

PROJECT_ROOT = Path(__file__).parent.parent.parent
EXAMPLE_DIR = Path(__file__).parent
SOURCE_FILE = EXAMPLE_DIR / "main.il"

# Docker image with il CLI
DOCKER_IMAGE = "il-bootstrap:local"

def run_docker_il(command: str, request: dict, work_dir: Path, store_dir: Path) -> dict:
    """Run il CLI command inside Docker container."""
    request_json = json.dumps(request)

    # Mount both directories into container
    cmd = [
        "docker", "run", "--rm", "-i",
        "-v", f"{work_dir}:/work",
        "-v", f"{store_dir}:/store",
        DOCKER_IMAGE,
        "/app/target/release/il",
        "--repository", "/work",
        "--store", "/store",
        command
    ]

    print(f"Running: {command}")
    print(f"Request: {json.dumps(request, indent=2)}")

    result = subprocess.run(
        cmd,
        input=request_json,
        capture_output=True,
        text=True,
        encoding="utf-8",
        timeout=60
    )

    if result.returncode != 0:
        print(f"Error: {result.stderr}", file=sys.stderr)
        sys.exit(1)

    try:
        response = json.loads(result.stdout)
    except json.JSONDecodeError as e:
        print(f"Failed to parse response: {result.stdout}", file=sys.stderr)
        print(f"Stderr: {result.stderr}", file=sys.stderr)
        raise

    print(f"Response OK: {response.get('ok')}")
    if response.get('diagnostics'):
        print(f"Diagnostics: {json.dumps(response['diagnostics'], indent=2)}")

    if not response.get('ok'):
        print(f"Command failed: {json.dumps(response, indent=2)}", file=sys.stderr)
        sys.exit(1)

    return response

def init_repository(work_dir: Path):
    """Initialize a Git repository with il project structure."""
    # Initialize git
    subprocess.run(["git", "init", "--initial-branch=main"], cwd=work_dir, check=True)
    subprocess.run(
        ["git", "config", "user.name", "il test"],
        cwd=work_dir, check=True
    )
    subprocess.run(
        ["git", "config", "user.email", "test@example.com"],
        cwd=work_dir, check=True
    )

    # Create initial project structure
    state = {
        "project_id": "greeting_server",
        "schema_version": "1.0.0",
        "head_revision": 0,
        "last_verified_revision": 0,
        "compiler_version": None,
        "runtime_version": None,
        "target": "x86_64-unknown-linux-gnu",
        "open_tasks": [],
        "blocked_tasks": [],
        "failed_tasks": [],
        "locks": {}
    }

    graph = {
        "project_id": "greeting_server",
        "graph_version": "1.0.0",
        "revision": 0,
        "target": "x86_64-unknown-linux-gnu",
        "modules": [],
        "types": [],
        "functions": [],
        "capabilities": [],
        "packages": [],
        "contracts": []
    }

    toolchain = {
        "target": "x86_64-unknown-linux-gnu",
        "rust": "1.90.0",
        "llvm": "14.0.6"
    }

    # Write files
    (work_dir / "repository_state.json").write_text(json.dumps(state, indent=2) + "\n")
    (work_dir / "examples/bootstrap").mkdir(parents=True, exist_ok=True)
    (work_dir / "examples/bootstrap/graph.json").write_text(json.dumps(graph, indent=2) + "\n")
    (work_dir / "toolchain.lock").write_text(json.dumps(toolchain, indent=2) + "\n")

    # Commit
    subprocess.run(["git", "add", "."], cwd=work_dir, check=True)
    subprocess.run(
        ["git", "commit", "-m", "Initial commit"],
        cwd=work_dir, check=True
    )

    # Create revision bindings
    commit_hash = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=work_dir, capture_output=True, text=True, check=True
    ).stdout.strip()

    tree_hash = subprocess.run(
        ["git", "rev-parse", "HEAD^{tree}"],
        cwd=work_dir, capture_output=True, text=True, check=True
    ).stdout.strip()

    bindings = {
        "schema_version": "1.0.0",
        "bindings": [{
            "revision": 0,
            "git_commit": commit_hash,
            "tree_hash": tree_hash
        }]
    }

    bindings_dir = work_dir / ".git/il"
    bindings_dir.mkdir(parents=True, exist_ok=True)
    (bindings_dir / "revision_bindings.json").write_text(json.dumps(bindings, indent=2) + "\n")

def main():
    # Create temporary directories
    work_dir = Path(tempfile.mkdtemp(prefix="il-greeting-work-"))
    store_dir = Path(tempfile.mkdtemp(prefix="il-greeting-store-"))

    try:
        print(f"Work directory: {work_dir}")
        print(f"Store directory: {store_dir}")

        # Initialize repository
        print("\n=== Initializing repository ===")
        init_repository(work_dir)

        # Read source code
        source_code = SOURCE_FILE.read_text(encoding="utf-8")

        # Publish source code
        print("\n=== Publishing source code ===")
        transaction = {
            "task_id": "build-greeting-server",
            "base_revision": 0,
            "scope": ["greeting_server"],
            "operations": [{
                "op": "add_module",
                "module": {
                    "entity_id": "greeting_server",
                    "path": "greeting_server",
                    "imports": [],
                    "declarations": [],
                    "visibility": "private",
                    "source": source_code
                }
            }],
            "required_checks": ["schema", "names", "types"]
        }

        response = run_docker_il("transact", transaction, work_dir, store_dir)
        revision = response["result_revision"]
        print(f"Published revision: {revision}")

        # Build the program
        print("\n=== Building program ===")
        build_request = {
            "revision": revision,
            "entry_point": "greeting_server.main",
            "target": "x86_64-unknown-linux-gnu",
            "optimization_level": "release"
        }

        response = run_docker_il("build", build_request, work_dir, store_dir)
        print("Build successful!")

        # Extract binary path from artifacts
        artifacts = response.get("artifacts", {})
        binary_path = artifacts.get("executable")
        if binary_path:
            print(f"Binary location: {binary_path}")

        print("\n=== Build complete ===")
        print(f"Work directory: {work_dir}")
        print(f"Store directory: {store_dir}")
        print("\nTo run the server, execute the binary from the store directory.")
        print("Note: Keep these directories to preserve the build artifacts.")

    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        # Clean up on error
        shutil.rmtree(work_dir, ignore_errors=True)
        shutil.rmtree(store_dir, ignore_errors=True)
        sys.exit(1)

if __name__ == "__main__":
    main()
