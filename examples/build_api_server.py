#!/usr/bin/env python3
"""
构建 il API 服务器

这个脚本将 api_server.il 编译为可执行文件
"""

import subprocess
import sys
import os
from pathlib import Path

def build_api_server():
    """编译 API 服务器"""

    # 确定项目根目录
    project_root = Path(__file__).parent.parent
    os.chdir(project_root)

    # 源文件和输出文件
    source = "examples/api_server.il"
    output = "target/debug/api_server"

    if sys.platform == "win32":
        output = "target/debug/api_server.exe"

    print("🔨 编译 il API 服务器...")
    print(f"   源文件: {source}")
    print(f"   输出: {output}")
    print()

    # 确保源文件存在
    if not Path(source).exists():
        print(f"❌ 错误: 找不到源文件 {source}")
        return False

    # 构建 il 编译器
    print("📦 构建 il 编译器...")
    result = subprocess.run(
        ["cargo", "build", "--bin", "il"],
        capture_output=True,
        text=True
    )

    if result.returncode != 0:
        print(f"❌ 编译器构建失败:")
        print(result.stderr)
        return False

    print("✅ 编译器构建成功")
    print()

    # 编译 il 代码
    print("🔨 编译 API 服务器代码...")

    il_binary = "target/debug/il"
    if sys.platform == "win32":
        il_binary = "target/debug/il.exe"

    result = subprocess.run(
        [il_binary, "compile", source, "-o", output],
        capture_output=True,
        text=True
    )

    if result.returncode != 0:
        print(f"❌ 编译失败:")
        print(result.stderr)
        print(result.stdout)
        return False

    print("✅ 编译成功!")
    print()
    print(f"🎉 服务器已生成: {output}")
    print()
    print("🚀 运行服务器:")
    if sys.platform == "win32":
        print(f"   {output}")
    else:
        print(f"   ./{output}")
    print()
    print("📡 服务器将在 http://0.0.0.0:8080 监听")
    print()
    print("🧪 测试 API:")
    print("   curl http://localhost:8080/health")
    print("   curl http://localhost:8080/hello")
    print("   curl http://localhost:8080/time")
    print("   curl http://localhost:8080/capabilities")

    return True

if __name__ == "__main__":
    success = build_api_server()
    sys.exit(0 if success else 1)
