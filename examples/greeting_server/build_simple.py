#!/usr/bin/env python3
"""Simple build script for greeting_server using Docker wrapper."""

import sys
from pathlib import Path
import subprocess
import json
import tempfile
import shutil

ROOT = Path(__file__).resolve().parents[2]
DOCKER_BUILD = ROOT / "tools" / "docker_build.py"
PACKAGES_DIR = ROOT / "packages"

# Modules needed for greeting_server
MODULES = ('core', 'alloc', 'io', 'time', 'net', 'json', 'http', 'test', 'tracing')


def load_packages():
    """Load all package sources."""
    sources = []
    for name in MODULES:
        package_file = PACKAGES_DIR / name / "lib.il"
        if not package_file.exists():
            print(f"[ERROR] Package not found: {package_file}")
            sys.exit(1)
        sources.append(package_file.read_text(encoding='utf-8'))
    return '\n'.join(sources)


def create_full_source():
    """Create complete source with all packages and app code."""
    app_source = (Path(__file__).parent / "main.il").read_text(encoding='utf-8')
    package_source = load_packages()
    return package_source + '\n' + app_source


def create_policy(policy_path):
    """Create runtime policy file."""
    policy = {
        "schema_version": "1.0.0",
        "grants": [
            {
                "entity_id": "greeting.listen",
                "kind": "Listen",
                "scope": "127.0.0.1:8080"
            },
            {
                "entity_id": "greeting.clock",
                "kind": "ClockRead",
                "scope": None
            }
        ],
        "test_faults": None
    }

    policy_path.parent.mkdir(parents=True, exist_ok=True)
    policy_path.write_text(json.dumps(policy, indent=2) + "\n", encoding="utf-8")
    return policy_path


def build(work_dir=None, keep=False):
    """Build greeting_server.

    Args:
        work_dir: Directory to build in (None = temp)
        keep: Keep build directory after completion

    Returns:
        (executable_path, policy_path, work_dir)
    """
    if work_dir:
        work_path = Path(work_dir)
        work_path.mkdir(parents=True, exist_ok=True)
        temp_dir = None
    else:
        temp_dir = tempfile.TemporaryDirectory(prefix="greeting-server-")
        work_path = Path(temp_dir.name)

    try:
        print("=== Building Greeting Server ===")
        print(f"Work directory: {work_path}")
        print()

        # Check Docker
        print("[1/5] Checking Docker...")
        result = subprocess.run(
            [sys.executable, str(DOCKER_BUILD), "check"],
            capture_output=True
        )
        if result.returncode != 0:
            print("[ERROR] Docker is not available")
            print("Please install Docker and ensure it's running")
            sys.exit(1)
        print("[OK] Docker available")
        print()

        # Setup Docker image
        print("[2/5] Setting up Docker image...")
        result = subprocess.run(
            [sys.executable, str(DOCKER_BUILD), "setup"],
            capture_output=False
        )
        if result.returncode != 0:
            print("[ERROR] Failed to setup Docker image")
            sys.exit(1)
        print()

        # Create full source
        print("[3/5] Preparing source code...")
        full_source = create_full_source()
        source_file = work_path / "full_source.il"
        source_file.write_text(full_source, encoding="utf-8")
        print(f"[OK] Source prepared ({len(full_source)} bytes)")
        print()

        # Compile
        print("[4/5] Compiling (this may take a minute)...")
        result = subprocess.run(
            [sys.executable, str(DOCKER_BUILD), "compile",
             str(source_file), "--work-dir", str(work_path), "--profile", "release"],
            capture_output=True,
            text=True
        )

        if result.returncode != 0:
            print("[ERROR] Compilation failed")
            print(result.stderr)
            sys.exit(1)

        executable_path = Path(result.stdout.strip())
        print(f"[OK] Executable: {executable_path}")
        print()

        # Create policy
        print("[5/5] Creating policy file...")
        policy_path = work_path / "policy.json"
        create_policy(policy_path)
        print(f"[OK] Policy: {policy_path}")
        print()

        print("=== Build Complete ===")
        print(f"Executable: {executable_path}")
        print(f"Policy: {policy_path}")
        print(f"Work dir: {work_path}")
        print()

        return executable_path, policy_path, work_path, temp_dir

    except Exception as e:
        if temp_dir and not keep:
            temp_dir.cleanup()
        raise


def run(executable_path, policy_path, work_dir, port=8080):
    """Run greeting_server."""
    print("=== Starting Greeting Server ===")
    print(f"Port: {port}")
    print(f"Health: http://localhost:{port}/health")
    print(f"Greet: http://localhost:{port}/greet")
    print()
    print("Press Ctrl+C to stop")
    print()

    try:
        subprocess.run(
            [sys.executable, str(DOCKER_BUILD), "run",
             "--work-dir", str(work_dir),
             "--executable", str(executable_path),
             "--policy", str(policy_path),
             "--port", str(port)],
            check=False
        )
    except KeyboardInterrupt:
        print("\n[INFO] Server stopped")


def main():
    import argparse

    parser = argparse.ArgumentParser(description="Build and run greeting_server")
    parser.add_argument("--build-only", action="store_true", help="Build but don't run")
    parser.add_argument("--work-dir", help="Build directory (default: temp)")
    parser.add_argument("--keep", action="store_true", help="Keep temp directory")
    parser.add_argument("--port", type=int, default=8080, help="Server port")

    args = parser.parse_args()

    executable, policy, work_dir, temp_dir = build(args.work_dir, args.keep)

    try:
        if not args.build_only:
            run(executable, policy, work_dir, args.port)
    finally:
        if temp_dir and not args.keep:
            print()
            print("[INFO] Cleaning up...")
            temp_dir.cleanup()


if __name__ == "__main__":
    main()
