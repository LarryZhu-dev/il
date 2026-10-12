#!/usr/bin/env python3
"""Compile greeting_server example using the il CLI."""
import json
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
IL_BINARY = ROOT / "target" / "debug" / "il"

# Package sources (core modules)
MODULES = ('core', 'alloc', 'io', 'time', 'net', 'json', 'http', 'test', 'tracing')

def package_source():
    """Load all package sources."""
    return '\n'.join(
        (ROOT / 'packages' / name / 'lib.il').read_text(encoding='utf-8')
        for name in MODULES
    )

def compile_greeting_server(profile='debug'):
    """Compile greeting_server to native executable."""
    print(f"==> Compiling greeting_server ({profile})")

    # Create temporary directory for compilation
    with tempfile.TemporaryDirectory(prefix='il-greeting-') as tmpdir:
        tmp = Path(tmpdir)
        store = tmp / 'store'
        store.mkdir()

        # Load source
        app_source = (ROOT / 'examples' / 'greeting_server' / 'main.il').read_text(encoding='utf-8')
        full_source = package_source() + '\n' + app_source

        # Create build request
        request = {
            "source": full_source,
            "profile": profile,
            "entry": "greeting.serve",
            "output_path": str(ROOT / 'examples' / 'greeting_server' / f'greeting-server-{profile}')
        }

        # Write request to file
        request_file = tmp / 'build_request.json'
        request_file.write_text(json.dumps(request, indent=2), encoding='utf-8')

        # Run il build command
        cmd = [str(IL_BINARY), '--store', str(store), 'build-standalone']

        print(f"Running: {' '.join(cmd)}")
        print(f"Request: {request_file}")

        result = subprocess.run(
            cmd,
            stdin=request_file.open('r'),
            capture_output=True,
            text=True
        )

        print(result.stdout)
        if result.stderr:
            print(result.stderr, file=sys.stderr)

        if result.returncode != 0:
            print(f"✗ Build failed with exit code {result.returncode}", file=sys.stderr)
            return False

        # Parse result
        try:
            response = json.loads(result.stdout)
            if response.get('ok'):
                executable = response.get('result', {}).get('executable', {}).get('path')
                if executable:
                    print(f"✓ Built executable: {executable}")
                    return True
            else:
                print(f"✗ Build failed: {response.get('diagnostics', [])}", file=sys.stderr)
                return False
        except json.JSONDecodeError as e:
            print(f"✗ Failed to parse build output: {e}", file=sys.stderr)
            return False

def main():
    profile = sys.argv[1] if len(sys.argv) > 1 else 'debug'

    if not IL_BINARY.exists():
        print(f"✗ il binary not found: {IL_BINARY}", file=sys.stderr)
        print("Run: cargo build --release", file=sys.stderr)
        return 1

    if compile_greeting_server(profile):
        print("\n✓ Compilation successful!")
        return 0
    else:
        return 1

if __name__ == '__main__':
    sys.exit(main())
