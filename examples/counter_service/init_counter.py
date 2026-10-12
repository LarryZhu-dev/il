#!/usr/bin/env python3
"""
Initialize counter in il repository.
"""
import json
import subprocess
import sys
from pathlib import Path


def initialize_counter(il_binary: Path, repository: Path, store: Path):
    """Initialize counter entity in repository."""

    # Read counter.il source
    counter_source = (Path(__file__).parent / "counter.il").read_text()

    print("Initializing counter...")
    print(f"Source:\n{counter_source}\n")

    # Create transaction to import the counter program
    transaction = {
        "base_revision": 0,
        "operations": [
            {
                "op": "import_text",
                "source": counter_source
            }
        ]
    }

    # Execute transaction
    try:
        result = subprocess.run(
            [str(il_binary), "--repository", str(repository), "--store", str(store), "transact"],
            input=json.dumps(transaction),
            capture_output=True,
            text=True,
            check=True
        )

        response = json.loads(result.stdout)

        if response.get("ok"):
            print(f"✓ Counter initialized at revision {response.get('result_revision')}")
            print(f"  Graph hash: {response.get('result', {}).get('graph_hash', 'N/A')}")
            return response.get('result_revision')
        else:
            print("✗ Failed to initialize counter:")
            for diag in response.get("diagnostics", []):
                print(f"  - {diag.get('message', str(diag))}")
            sys.exit(1)

    except subprocess.CalledProcessError as e:
        print(f"✗ Error executing transaction: {e}")
        print(f"STDERR: {e.stderr}")
        sys.exit(1)
    except json.JSONDecodeError as e:
        print(f"✗ Invalid JSON response: {e}")
        sys.exit(1)


def main():
    # Find il binary
    project_root = Path(__file__).parent.parent.parent

    candidates = [
        project_root / "target" / "release" / "il",
        project_root / "target" / "debug" / "il",
        project_root / "target" / "release" / "il.exe",
        project_root / "target" / "debug" / "il.exe",
    ]

    il_binary = None
    for candidate in candidates:
        if candidate.exists():
            il_binary = candidate
            break

    if not il_binary:
        print("ERROR: Could not find il binary. Please run 'python quick_start.py' first.")
        sys.exit(1)

    repository = project_root
    store = repository / ".il" / "project"

    print(f"Using il binary: {il_binary}")
    print(f"Repository: {repository}")
    print(f"Store: {store}\n")

    # Check if store is initialized
    if not (store / ".il" / "HEAD").exists():
        print("ERROR: Repository not initialized. Please run 'python quick_start.py' first.")
        sys.exit(1)

    initialize_counter(il_binary, repository, store)


if __name__ == "__main__":
    main()
