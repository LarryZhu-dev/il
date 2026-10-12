#!/usr/bin/env python3
"""Build and run the greeting server."""
import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
PROJECT_ROOT = ROOT.parent.parent

# Check if running in Docker container
IN_CONTAINER = os.path.exists("/.dockerenv") or os.environ.get("IL_CONTAINER") == "1"

# Find il executable (with or without .exe)
IL_CLI = None
for candidate in [
    PROJECT_ROOT / "target/release/il.exe",
    PROJECT_ROOT / "target/release/il",
    PROJECT_ROOT / "target/debug/il.exe",
    PROJECT_ROOT / "target/debug/il",
]:
    if candidate.exists():
        IL_CLI = candidate
        break

if IL_CLI is None:
    print("Error: il executable not found.", file=sys.stderr)
    print("Build it with: ./tools/dev.ps1 build", file=sys.stderr)
    sys.exit(1)

BUILD_DIR = ROOT / "build"
POLICY_FILE = ROOT / "policy.json"


def build():
    """Build the greeting server executable."""
    print("=" * 60)
    print("Building greeting server...")
    print("=" * 60)

    # Create build directory
    BUILD_DIR.mkdir(parents=True, exist_ok=True)

    # Read main.il
    print("\n1. Reading source files...")
    source = (ROOT / "main.il").read_text(encoding="utf-8")
    print(f"   ✓ main.il ({len(source)} bytes)")

    # Read package sources
    modules = ['core', 'alloc', 'io', 'time', 'net', 'json', 'http', 'tracing']
    package_sources = []

    for name in modules:
        pkg_file = PROJECT_ROOT / 'packages' / name / 'lib.il'
        if not pkg_file.exists():
            print(f"   ✗ Package {name} not found at {pkg_file}", file=sys.stderr)
            return False
        content = pkg_file.read_text(encoding='utf-8')
        package_sources.append(content)
        print(f"   ✓ {name} ({len(content)} bytes)")

    full_source = '\n'.join(package_sources) + '\n' + source
    print(f"\n   Total source: {len(full_source)} bytes")

    # Create transaction
    print("\n2. Creating graph transaction...")
    transact = {
        "task_id": "BUILD_GREETING_SERVER",
        "base_revision": 0,
        "scope": ["program"],
        "operations": [{
            "op": "import_text",
            "source": full_source
        }],
        "required_checks": []
    }

    transact_file = BUILD_DIR / "transact.json"
    transact_file.write_text(json.dumps(transact, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    print(f"   ✓ Wrote {transact_file}")

    # Run il CLI to compile
    print("\n3. Compiling to semantic graph...")
    cmd = [str(IL_CLI), "transact", str(transact_file), str(BUILD_DIR / "state.msgpack")]
    print(f"   $ {' '.join(cmd)}")
    result = subprocess.run(cmd, capture_output=True, text=True)

    if result.returncode != 0:
        print("   ✗ Compilation failed:", file=sys.stderr)
        print(result.stdout, file=sys.stderr)
        print(result.stderr, file=sys.stderr)
        return False

    print("   ✓ Graph compilation succeeded")
    if result.stdout:
        print(f"\n{result.stdout}")

    # Compile to native executable
    print("\n4. Compiling to native executable...")

    # Use .exe extension only on Windows outside container
    if os.name == 'nt' and not IN_CONTAINER:
        exe_name = "greeting_server.exe"
    else:
        exe_name = "greeting_server"

    cmd = [
        str(IL_CLI), "compile",
        str(BUILD_DIR / "state.msgpack"),
        str(BUILD_DIR / exe_name),
        "--entry", "main"
    ]
    print(f"   $ {' '.join(cmd)}")
    result = subprocess.run(cmd, capture_output=True, text=True)

    if result.returncode != 0:
        print("   ✗ Native compilation failed:", file=sys.stderr)
        print(result.stdout, file=sys.stderr)
        print(result.stderr, file=sys.stderr)
        return False

    exe_path = BUILD_DIR / exe_name
    if exe_path.exists():
        size = exe_path.stat().st_size
        print(f"   ✓ Built: {exe_path} ({size:,} bytes)")

    if result.stdout:
        print(f"\n{result.stdout}")

    print("\n" + "=" * 60)
    print("Build complete!")
    print("=" * 60)
    return True


def run(host="127.0.0.1", port=8080):
    """Run the greeting server."""
    # Find executable
    executable = None
    for name in ["greeting_server.exe", "greeting_server"]:
        candidate = BUILD_DIR / name
        if candidate.exists():
            executable = candidate
            break

    if executable is None:
        print("Executable not found. Building first...")
        if not build():
            return 1
        # Try again after build
        for name in ["greeting_server.exe", "greeting_server"]:
            candidate = BUILD_DIR / name
            if candidate.exists():
                executable = candidate
                break

    if executable is None:
        print("Error: Could not find executable after build", file=sys.stderr)
        return 1

    print("=" * 60)
    print(f"Starting greeting server on http://{host}:{port}")
    print("=" * 60)
    print("\nEndpoints:")
    print(f"  GET http://{host}:{port}/greeting")
    print("\nPress Ctrl+C to stop\n")

    # Update policy file with correct host/port
    policy_data = json.loads(POLICY_FILE.read_text())
    for grant in policy_data.get("grants", []):
        if grant.get("kind") == "Listen":
            grant["scope"] = f"{host}:{port}"

    updated_policy = BUILD_DIR / "policy_runtime.json"
    updated_policy.write_text(json.dumps(policy_data, indent=2) + '\n')

    # Run with policy file
    try:
        env = os.environ.copy()

        # Pass policy file path as environment variable
        env['IL_POLICY_FILE'] = str(updated_policy)

        # Start process
        process = subprocess.Popen(
            [str(executable)],
            stdin=subprocess.DEVNULL,
            env=env
        )

        return_code = process.wait()

        if return_code != 0:
            print(f"\nServer exited with code {return_code}", file=sys.stderr)

        return return_code

    except KeyboardInterrupt:
        print("\n\nShutting down gracefully...")
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            print("Force killing server...")
            process.kill()
            process.wait()
        print("Server stopped.")
        return 0
    except Exception as e:
        print(f"\nError: {e}", file=sys.stderr)
        if 'process' in locals():
            process.kill()
        return 1


def main():
    parser = argparse.ArgumentParser(description="Build and run il greeting server")
    parser.add_argument(
        "command",
        nargs="?",
        choices=["build", "run"],
        default="run",
        help="Command to execute (default: run)"
    )
    parser.add_argument(
        "--host",
        default="127.0.0.1",
        help="Host to bind to (default: 127.0.0.1, use 0.0.0.0 for all interfaces)"
    )
    parser.add_argument(
        "--port",
        type=int,
        default=8080,
        help="Port to bind to (default: 8080)"
    )

    args = parser.parse_args()

    if args.command == "build":
        sys.exit(0 if build() else 1)
    else:  # run
        sys.exit(run(host=args.host, port=args.port))


if __name__ == "__main__":
    main()
