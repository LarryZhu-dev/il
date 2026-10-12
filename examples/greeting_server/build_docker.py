#!/usr/bin/env python3
"""使用 Docker 构建和运行 greeting_server。

这个脚本在 Windows 上使用 Docker 来运行 Linux il CLI。
"""

import argparse
import json
import hashlib
import subprocess
import sys
import tempfile
import time
from pathlib import Path

# 项目根目录
ROOT = Path(__file__).resolve().parents[2]

# Docker 镜像
DOCKER_IMAGE = "il-bootstrap:local"

# 模块列表
MODULES = ('core', 'alloc', 'io', 'time', 'net', 'json', 'http', 'test', 'tracing')

TARGET = "x86_64-unknown-linux-gnu"


def sha256(path):
    """计算文件的 SHA256 哈希。"""
    return "sha256:" + hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write_json(path, value):
    """写入 JSON 文件。"""
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def docker_run(cmd, volumes=None, capture_output=True):
    """在 Docker 容器中运行命令。"""
    docker_cmd = ["docker", "run", "--rm"]

    if volumes:
        for host_path, container_path in volumes.items():
            # 转换 Windows 路径为 Docker 格式
            host_str = str(host_path).replace('\\', '/')
            if ':' in host_str:  # Windows 绝对路径
                host_str = '/' + host_str.replace(':', '')
            docker_cmd.extend(["-v", f"{host_str}:{container_path}"])

    docker_cmd.append(DOCKER_IMAGE)
    docker_cmd.extend(cmd)

    result = subprocess.run(
        docker_cmd,
        capture_output=capture_output,
        text=True,
        encoding="utf-8",
        timeout=120
    )

    return result


def package_source():
    """加载所有依赖包的源代码。"""
    return '\n'.join(
        (ROOT / 'packages' / name / 'lib.il').read_text(encoding='utf-8')
        for name in MODULES
    )


def setup_repository(repo_path):
    """创建并初始化 Git 仓库。"""
    repo_path.mkdir(parents=True, exist_ok=True)

    # 初始化 Git
    subprocess.run(
        ["git", "init", "--initial-branch=main"],
        cwd=repo_path,
        check=True,
        capture_output=True
    )

    # 创建初始状态
    state = {
        "project_id": "greeting_server",
        "schema_version": "1.0.0",
        "head_revision": 0,
        "last_verified_revision": 0,
        "compiler_version": None,
        "runtime_version": None,
        "target": TARGET,
        "open_tasks": ["build"],
        "blocked_tasks": [],
        "failed_tasks": [],
        "locks": {},
    }

    graph = {
        "project_id": "greeting_server",
        "graph_version": "1.0.0",
        "revision": 0,
        "target": TARGET,
        "modules": [],
        "types": [],
        "functions": [],
        "capabilities": [],
        "packages": [],
        "contracts": [],
    }

    toolchain = {
        "target": TARGET,
        "rust": "1.90.0",
        "llvm": "14.0.6",
    }

    write_json(repo_path / "repository_state.json", state)
    write_json(repo_path / "examples/bootstrap/graph.json", graph)
    write_json(repo_path / "toolchain.lock", toolchain)

    # 提交初始状态
    subprocess.run(
        ["git", "add", "repository_state.json", "examples/bootstrap/graph.json", "toolchain.lock"],
        cwd=repo_path,
        check=True,
        capture_output=True
    )

    subprocess.run(
        ["git", "-c", "user.name=Builder", "-c", "user.email=build@local",
         "-c", "commit.gpgsign=false", "commit", "-m", "init: bootstrap greeting_server"],
        cwd=repo_path,
        check=True,
        capture_output=True
    )

    # 获取提交哈希
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=repo_path,
        check=True,
        capture_output=True,
        text=True
    )
    commit_hash = result.stdout.strip()

    result = subprocess.run(
        ["git", "rev-parse", "HEAD^{tree}"],
        cwd=repo_path,
        check=True,
        capture_output=True,
        text=True
    )
    tree_hash = result.stdout.strip()

    # 创建修订绑定
    bindings = {
        "schema_version": "1.0.0",
        "bindings": [{
            "revision": 0,
            "git_commit": commit_hash,
            "tree_hash": tree_hash,
        }],
    }

    write_json(repo_path / ".git/il/revision_bindings.json", bindings)

    print(f"[OK] Repository initialized at {repo_path}")
    print(f"  Commit: {commit_hash}")
    return repo_path


def build_il_in_docker(work_root):
    """在 Docker 中构建 il CLI。"""
    print("\n[INFO] Building il CLI in Docker...")

    result = docker_run(
        ["sh", "-c", "cd /workspace && cargo build --release --bin il"],
        capture_output=True
    )

    if result.returncode != 0:
        print(f"[ERROR] Failed to build il CLI")
        print(f"stderr: {result.stderr}")
        sys.exit(1)

    print("[OK] il CLI built successfully")


def invoke_il_docker(work_root, command, request):
    """在 Docker 中调用 il CLI。"""
    request_text = json.dumps(request)

    # 创建临时请求文件
    request_file = work_root / "request.json"
    request_file.write_text(request_text, encoding="utf-8")

    # 挂载工作目录到容器
    volumes = {
        work_root: "/work"
    }

    result = docker_run(
        ["sh", "-c",
         f"cd /workspace && cargo build --release --bin il && "
         f"/workspace/target/release/il --repository /work/repository --store /work/store {command} < /work/request.json"],
        volumes=volumes,
        capture_output=True
    )

    if result.returncode != 0:
        print(f"[ERROR] il CLI failed")
        print(f"stdout: {result.stdout}")
        print(f"stderr: {result.stderr}")
        sys.exit(1)

    try:
        response = json.loads(result.stdout)
    except json.JSONDecodeError as e:
        print(f"[ERROR] il CLI returned invalid JSON")
        print(f"stdout: {result.stdout}")
        sys.exit(1)

    if not response.get("ok"):
        print(f"[ERROR] il CLI returned ok=false")
        print(json.dumps(response, indent=2))
        sys.exit(1)

    return response


def publish_source(work_root):
    """发布源代码到图。"""
    print("\n[INFO] Publishing source code...")

    # 读取 greeting_server 源代码
    greeting_source = (Path(__file__).parent / "main.il").read_text(encoding='utf-8')

    # 组合所有包和应用源代码
    full_source = package_source() + '\n' + greeting_source

    request = {
        "task_id": "build-greeting-server",
        "base_revision": 0,
        "scope": ["program"],
        "operations": [{"op": "import_text", "source": full_source}],
        "required_checks": []
    }

    response = invoke_il_docker(work_root, "transact", request)

    print(f"[OK] Source published (revision {response['result_revision']})")
    return response


def build_executable(work_root, profile="release"):
    """编译可执行文件。"""
    print(f"\n[INFO] Compiling {profile} version...")

    request = {
        "revision": 1,
        "target": TARGET,
        "profile": profile,
        "runtime_profile": "full"
    }

    response = invoke_il_docker(work_root, "build", request)

    native = response["result"]["native"]
    # 路径在容器内，需要转换回主机路径
    container_path = native["artifacts"]["executable"]["path"]
    # /work/store/... -> work_root/store/...
    relative_path = container_path.replace("/work/", "")
    executable_path = work_root / relative_path

    print(f"[OK] Compilation successful")
    print(f"  Executable: {executable_path}")

    return executable_path


def create_policy(policy_path):
    """创建运行时策略文件。"""
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

    write_json(policy_path, policy)
    print(f"[OK] Policy created at {policy_path}")
    return policy_path


def run_server_docker(work_root, executable_path, policy_path):
    """在 Docker 中运行 HTTP 服务器。"""
    print(f"\n[INFO] Starting server...")
    print(f"  Listening on: http://127.0.0.1:8080")
    print(f"  Press Ctrl+C to stop\n")

    try:
        # 获取相对路径
        rel_executable = executable_path.relative_to(work_root)
        rel_policy = policy_path.relative_to(work_root)

        volumes = {
            work_root: "/work"
        }

        result = docker_run(
            ["sh", "-c", f"/work/{rel_executable} 4</work/{rel_policy}"],
            volumes=volumes,
            capture_output=False  # 让输出直接显示
        )

    except KeyboardInterrupt:
        print("\n\n[INFO] Server stopped")


def main():
    parser = argparse.ArgumentParser(description="Build and run greeting_server using Docker")
    parser.add_argument("--build", action="store_true", help="Build only, don't run")
    parser.add_argument("--run", action="store_true", help="Build and run")
    parser.add_argument("--profile", default="release", choices=["debug", "release"], help="Build profile")
    parser.add_argument("--work-dir", type=Path, help="Work directory (default: temp)")

    args = parser.parse_args()

    # 检查 Docker
    try:
        subprocess.run(["docker", "version"], capture_output=True, check=True)
    except:
        print("[ERROR] Docker is not available")
        sys.exit(1)

    # 检查镜像
    result = subprocess.run(
        ["docker", "images", "-q", DOCKER_IMAGE],
        capture_output=True,
        text=True
    )
    if not result.stdout.strip():
        print(f"[ERROR] Docker image {DOCKER_IMAGE} not found")
        print("Please build it first with: docker build -t il-bootstrap:local tools/")
        sys.exit(1)

    # 确定工作目录
    if args.work_dir:
        work_root = args.work_dir
        work_root.mkdir(parents=True, exist_ok=True)
        cleanup = False
    else:
        temp_dir = tempfile.TemporaryDirectory(prefix="greeting-server-")
        work_root = Path(temp_dir.name)
        cleanup = True

    try:
        repo_path = work_root / "repository"
        store_path = work_root / "store"
        policy_path = work_root / "policy.json"

        print("=== Building Greeting Server ===")
        print(f"Work directory: {work_root}")
        print(f"Docker image: {DOCKER_IMAGE}")

        # 设置仓库
        setup_repository(repo_path)

        # 发布源代码
        publish_source(work_root)

        # 编译
        executable = build_executable(work_root, args.profile)

        # 创建策略
        create_policy(policy_path)

        print("\n=== Build Complete ===")
        print(f"Executable: {executable}")
        print(f"Policy: {policy_path}")

        if args.run or not args.build:
            run_server_docker(work_root, executable, policy_path)

    finally:
        if cleanup:
            temp_dir.cleanup()


if __name__ == "__main__":
    main()
