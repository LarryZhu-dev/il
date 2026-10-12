#!/usr/bin/env python3
"""
Production build script for il greeting_server.

This script:
1. Merges stdlib + application into source.il
2. Runs the full Docker-based compilation
3. Extracts the resulting ELF executable
4. Provides a ready-to-run binary

Usage:
    python build.py                    # Build to ./build/
    python build.py --output ./dist/   # Custom output directory
    python build.py --clean            # Clean and rebuild
"""

import argparse
import json
import shutil
import subprocess
import sys
from pathlib import Path


def run_command(cmd, cwd=None, check=True):
    """Run a shell command and return the result."""
    print(f"Running: {' '.join(cmd)}")
    result = subprocess.run(
        cmd,
        cwd=cwd,
        capture_output=True,
        text=True,
        check=False,
    )

    if check and result.returncode != 0:
        print(f"Error: Command failed with exit code {result.returncode}")
        print(f"STDOUT:\n{result.stdout}")
        print(f"STDERR:\n{result.stderr}")
        sys.exit(1)

    return result


def merge_source(main_file, output_file, repo_root):
    """Merge stdlib modules with application code."""
    print(f"\n[1/4] Merging standard library with {main_file}...")

    merge_tool = repo_root / "tools" / "merge_stdlib.py"
    result = run_command([
        sys.executable,
        str(merge_tool),
        str(main_file),
        "-o",
        str(output_file),
    ])

    if not output_file.exists():
        print(f"Error: Failed to create {output_file}")
        sys.exit(1)

    size = output_file.stat().st_size
    print(f"✓ Created {output_file} ({size:,} bytes)")
    return output_file


def docker_compile(source_file, work_dir, repo_root):
    """Run Docker-based compilation."""
    print(f"\n[2/4] Compiling {source_file.name}...")

    build_tool = repo_root / "tools" / "docker_build_v2.py"
    result = run_command([
        sys.executable,
        str(build_tool),
        "compile",
        str(source_file),
        "--work-dir",
        str(work_dir),
    ])

    # Check for build_result.json
    result_file = work_dir / "build_result.json"
    if not result_file.exists():
        print("Error: Build did not complete")
        sys.exit(1)

    with open(result_file) as f:
        build_result = json.load(f)

    if not build_result.get("ok"):
        print("Error: Build failed")
        print(json.dumps(build_result, indent=2))
        sys.exit(1)

    print("✓ Compilation successful")
    return build_result


def extract_executable(work_dir, output_dir, app_name):
    """Extract the compiled ELF executable."""
    print(f"\n[3/4] Extracting executable...")

    store_dir = work_dir / "store"
    if not store_dir.exists():
        print(f"Error: Store directory not found: {store_dir}")
        sys.exit(1)

    # Find ELF files
    elf_files = list(store_dir.glob("**/*.elf"))
    if not elf_files:
        print(f"Error: No ELF files found in {store_dir}")
        sys.exit(1)

    # Copy the first ELF to output directory
    src_elf = elf_files[0]
    output_dir.mkdir(parents=True, exist_ok=True)
    dst_elf = output_dir / f"{app_name}"

    shutil.copy2(src_elf, dst_elf)
    dst_elf.chmod(0o755)

    size = dst_elf.stat().st_size
    print(f"✓ Extracted {dst_elf} ({size:,} bytes)")

    return dst_elf


def print_usage(executable):
    """Print usage instructions."""
    print(f"\n[4/4] Build complete!")
    print(f"\nTo run the server:")
    print(f"  {executable}")
    print(f"\nTo test:")
    print(f"  curl http://localhost:3000/hello")
    print(f"  curl http://localhost:3000/health")


def main():
    parser = argparse.ArgumentParser(
        description="Build il greeting_server executable"
    )
    parser.add_argument(
        "--output",
        default="./build",
        help="Output directory for executable (default: ./build)",
    )
    parser.add_argument(
        "--work-dir",
        default="./build_work",
        help="Temporary work directory (default: ./build_work)",
    )
    parser.add_argument(
        "--clean",
        action="store_true",
        help="Clean work directory before building",
    )
    parser.add_argument(
        "--app-name",
        default="greeting_server",
        help="Application name (default: greeting_server)",
    )

    args = parser.parse_args()

    # Resolve paths
    script_dir = Path(__file__).parent
    repo_root = script_dir.parent.parent
    main_file = script_dir / "main.il"
    source_file = script_dir / "source.il"
    work_dir = script_dir / args.work_dir
    output_dir = script_dir / args.output

    # Validate
    if not main_file.exists():
        print(f"Error: {main_file} not found")
        sys.exit(1)

    # Clean if requested
    if args.clean and work_dir.exists():
        print(f"Cleaning {work_dir}...")
        shutil.rmtree(work_dir)

    try:
        # Step 1: Merge source
        merge_source(main_file, source_file, repo_root)

        # Step 2: Docker compile
        build_result = docker_compile(source_file, work_dir, repo_root)

        # Step 3: Extract executable
        executable = extract_executable(work_dir, output_dir, args.app_name)

        # Step 4: Print usage
        print_usage(executable)

        print("\n✅ Build successful!")
        return 0

    except KeyboardInterrupt:
        print("\n\nBuild interrupted by user")
        return 130
    except Exception as e:
        print(f"\n❌ Build failed: {e}")
        import traceback
        traceback.print_exc()
        return 1


if __name__ == "__main__":
    sys.exit(main())
