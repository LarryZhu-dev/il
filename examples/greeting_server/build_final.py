#!/usr/bin/env python3
"""
Final working build script for il Greeting Server.

Usage:
    python build_final.py build    # Build the server
    python build_final.py run      # Run the server
    python build_final.py test     # Test the server
"""

import json
import subprocess
import sys
import time
from pathlib import Path

# Add tools to path
TOOLS_DIR = Path(__file__).parent.parent.parent / "tools"
sys.path.insert(0, str(TOOLS_DIR))

from docker_build_v2 import compile_program, run_program


def build():
    """Build the greeting server."""
    print("=== Building Greeting Server ===")

    main_file = Path(__file__).parent / "main.il"
    source = Path(__file__).parent / "source.il"
    work_dir = Path(__file__).parent / "build_test"

    # Merge stdlib if source.il doesn't exist or is outdated
    if not source.exists() or main_file.stat().st_mtime > source.stat().st_mtime:
        print("Merging standard library modules...")
        merge_result = subprocess.run([
            sys.executable,
            str(TOOLS_DIR / "merge_stdlib.py"),
            str(main_file),
            "-o", str(source)
        ], capture_output=True, text=True)

        if merge_result.returncode != 0:
            print(merge_result.stderr)
            print("[ERROR] Failed to merge stdlib")
            return False
        print(merge_result.stdout)

    print(f"Work directory: {work_dir.relative_to(Path.cwd())}")
    print()

    # Compile
    exe = compile_program(work_dir, source)

    if not exe:
        print()
        print("[ERROR] Build failed")
        return False

    print()
    print("[SUCCESS] Server built successfully!")
    print(f"Executable: {exe.relative_to(Path.cwd())}")
    return True


def run_server():
    """Run the greeting server."""
    work_dir = Path(__file__).parent / "build_test"

    # Find executable
    store = work_dir / "store"
    if not store.exists():
        print("[ERROR] Build directory not found. Run 'build' first.")
        return False

    exes = list(store.glob("**/greeting.elf"))
    if not exes:
        print("[ERROR] Executable not found. Run 'build' first.")
        return False

    exe = exes[0]
    policy = work_dir / "policy.json"

    # Create policy if missing
    if not policy.exists():
        policy.write_text(json.dumps({
            "version": "1.0.0",
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
            ]
        }, indent=2))

    print("=== Starting Greeting Server ===")
    print()

    run_program(work_dir, exe, policy, port=8080)


def test_server():
    """Test the greeting server."""
    import socket
    import requests

    print("=== Testing Greeting Server ===")
    print()

    # Check if server is running
    try:
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        result = sock.connect_ex(('127.0.0.1', 8080))
        sock.close()

        if result != 0:
            print("[ERROR] Server not running on port 8080")
            print("Start it with: python build_final.py run")
            return False
    except Exception as e:
        print(f"[ERROR] Cannot connect to server: {e}")
        return False

    print("[1/3] Testing /health endpoint...")
    try:
        resp = requests.get("http://localhost:8080/health", timeout=5)
        if resp.status_code == 200:
            data = resp.json()
            print(f"  ✓ Status: {data.get('status')}")
            print(f"  ✓ Timestamp: {data.get('timestamp')}")
        else:
            print(f"  ✗ Unexpected status: {resp.status_code}")
            return False
    except Exception as e:
        print(f"  ✗ Request failed: {e}")
        return False

    print()
    print("[2/3] Testing /greet endpoint (valid)...")
    try:
        resp = requests.post(
            "http://localhost:8080/greet",
            json={"name": "Alice"},
            timeout=5
        )
        if resp.status_code == 200:
            data = resp.json()
            print(f"  ✓ Message: {data.get('message')}")
            print(f"  ✓ Timestamp: {data.get('timestamp')}")
        else:
            print(f"  ✗ Unexpected status: {resp.status_code}")
            return False
    except Exception as e:
        print(f"  ✗ Request failed: {e}")
        return False

    print()
    print("[3/3] Testing /greet endpoint (missing name)...")
    try:
        resp = requests.post(
            "http://localhost:8080/greet",
            json={},
            timeout=5
        )
        if resp.status_code == 400:
            data = resp.json()
            print(f"  ✓ Error: {data.get('error')}")
        else:
            print(f"  ✗ Expected 400, got: {resp.status_code}")
            return False
    except Exception as e:
        print(f"  ✗ Request failed: {e}")
        return False

    print()
    print("[SUCCESS] All tests passed!")
    return True


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(1)

    cmd = sys.argv[1]

    if cmd == "build":
        success = build()
        sys.exit(0 if success else 1)

    elif cmd == "run":
        run_server()

    elif cmd == "test":
        success = test_server()
        sys.exit(0 if success else 1)

    else:
        print(f"Unknown command: {cmd}")
        print(__doc__)
        sys.exit(1)


if __name__ == "__main__":
    main()
