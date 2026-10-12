#!/usr/bin/env python3
"""Monitor il build progress."""

import sys
import time
from pathlib import Path


def monitor_build(work_dir):
    """Monitor build progress by checking work directory."""
    work_dir = Path(work_dir)

    print("Monitoring build progress...\n")

    checks = [
        ("source.il", "Source file copied"),
        ("build.sh", "Build script generated"),
        ("repo/.git", "Git repository initialized"),
        ("repo/repository_state.json", "Repository state created"),
        ("transact_result.json", "Source transaction completed"),
        ("build_result.json", "Build completed"),
        ("store", "Store directory created"),
    ]

    completed = set()

    while True:
        changed = False
        for path, desc in checks:
            key = path
            if key in completed:
                continue

            full_path = work_dir / path
            if full_path.exists():
                print(f"✓ {desc}")
                completed.add(key)
                changed = True

        # Check for ELF files
        if "store" in completed and "elf_found" not in completed:
            store = work_dir / "store"
            elfs = list(store.glob("**/*.elf"))
            if elfs:
                print(f"✓ Executable generated: {elfs[0].name}")
                completed.add("elf_found")
                changed = True
                print("\n[SUCCESS] Build completed!")
                print(f"Executable: {elfs[0]}")
                break

        if changed:
            print()

        time.sleep(2)


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: python monitor_build.py <work_dir>")
        sys.exit(1)

    monitor_build(sys.argv[1])
