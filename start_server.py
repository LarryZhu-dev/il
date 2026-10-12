#!/usr/bin/env python3
"""
一键启动脚本 - 选择并启动 HTTP 服务器

这个脚本提供交互式菜单来选择启动哪个服务器
"""

import subprocess
import sys
import os
from pathlib import Path

def print_banner():
    """打印欢迎横幅"""
    print()
    print("╔══════════════════════════════════════════════════════════════════╗")
    print("║                                                                  ║")
    print("║              🚀 il HTTP 服务器启动器 🚀                          ║")
    print("║                                                                  ║")
    print("╚══════════════════════════════════════════════════════════════════╝")
    print()

def print_menu():
    """打印菜单选项"""
    print("请选择要启动的服务器：")
    print()
    print("  1. Python 演示服务器 (推荐)")
    print("     - 即时启动，无需编译")
    print("     - 完整 REST API (8 个端点)")
    print("     - 内存图数据库")
    print("     - 地址: http://127.0.0.1:8000")
    print()
    print("  2. Docker 容器服务器")
    print("     - 生产级部署")
    print("     - 完整 il 编译")
    print("     - 自动健康检查")
    print("     - 地址: http://localhost:8000")
    print()
    print("  3. il 原生编译服务器")
    print("     - 需要先编译")
    print("     - 最佳性能")
    print("     - il 语言示例")
    print("     - 地址: http://localhost:8080")
    print()
    print("  4. 运行测试套件")
    print("     - 验证所有功能")
    print("     - 71 个测试")
    print("     - 性能基准")
    print()
    print("  0. 退出")
    print()

def start_demo_server():
    """启动演示服务器"""
    print()
    print("🚀 启动 Python 演示服务器...")
    print()
    print("服务器将在 http://127.0.0.1:8000 运行")
    print("按 Ctrl+C 停止服务器")
    print()

    try:
        subprocess.run([sys.executable, "demo_server.py"])
    except KeyboardInterrupt:
        print()
        print("✅ 服务器已停止")

def start_docker_server():
    """启动 Docker 服务器"""
    print()
    print("🐳 启动 Docker 容器服务器...")
    print()

    # 检查 Docker 是否可用
    try:
        subprocess.run(["docker", "--version"], capture_output=True, check=True)
    except (subprocess.CalledProcessError, FileNotFoundError):
        print("❌ 错误: Docker 未安装或未运行")
        print()
        print("请先安装并启动 Docker Desktop")
        print("下载地址: https://www.docker.com/products/docker-desktop")
        return

    print("启动容器...")
    result = subprocess.run(["docker-compose", "up", "-d"])

    if result.returncode == 0:
        print()
        print("✅ Docker 服务器已启动")
        print()
        print("服务地址: http://localhost:8000")
        print("查看日志: docker-compose logs -f")
        print("停止服务: docker-compose down")
    else:
        print()
        print("❌ 启动失败")
        print()
        print("可能的原因:")
        print("  - Docker 镜像未构建")
        print("  - 端口 8000 已被占用")
        print("  - docker-compose.yml 配置问题")
        print()
        print("请运行: docker-compose build")

def start_native_server():
    """启动原生编译服务器"""
    print()
    print("⚡ 启动 il 原生编译服务器...")
    print()

    # 检查二进制文件是否存在
    binary = "target/debug/api_server"
    if sys.platform == "win32":
        binary = "target/debug/api_server.exe"

    if not Path(binary).exists():
        print("❌ 服务器未编译")
        print()
        print("请先运行编译脚本:")
        print("  python examples/build_api_server.py")
        print()

        response = input("是否现在编译? (y/N): ").strip().lower()
        if response == 'y':
            print()
            print("🔨 开始编译...")
            result = subprocess.run([sys.executable, "examples/build_api_server.py"])
            if result.returncode != 0:
                print()
                print("❌ 编译失败")
                return
        else:
            return

    print(f"运行 {binary}...")
    print()
    print("服务器将在 http://localhost:8080 运行")
    print("按 Ctrl+C 停止服务器")
    print()

    try:
        subprocess.run([binary])
    except KeyboardInterrupt:
        print()
        print("✅ 服务器已停止")

def run_tests():
    """运行测试套件"""
    print()
    print("🧪 运行测试套件...")
    print()

    # 运行 Python 演示服务器测试
    print("【1/2】运行 Python 演示服务器测试")
    print("=" * 60)
    result1 = subprocess.run([sys.executable, "test_demo_server.py"])

    print()
    print("【2/2】运行使用示例")
    print("=" * 60)
    result2 = subprocess.run([sys.executable, "example_usage.py"])

    print()
    if result1.returncode == 0 and result2.returncode == 0:
        print("✅ 所有测试通过！")
    else:
        print("⚠️  部分测试失败")

def main():
    """主函数"""
    # 确保在项目根目录
    if not Path("demo_server.py").exists():
        print("❌ 错误: 请在项目根目录运行此脚本")
        sys.exit(1)

    while True:
        print_banner()
        print_menu()

        try:
            choice = input("请选择 (0-4): ").strip()

            if choice == "1":
                start_demo_server()
            elif choice == "2":
                start_docker_server()
            elif choice == "3":
                start_native_server()
            elif choice == "4":
                run_tests()
            elif choice == "0":
                print()
                print("👋 再见！")
                print()
                break
            else:
                print()
                print("❌ 无效选择，请输入 0-4")

            if choice != "0":
                print()
                input("按 Enter 继续...")

        except KeyboardInterrupt:
            print()
            print()
            print("👋 再见！")
            print()
            break
        except EOFError:
            break

if __name__ == "__main__":
    main()
