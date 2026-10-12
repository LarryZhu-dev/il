#!/usr/bin/env python3
"""
测试 il API 服务器

这个脚本测试所有 API 端点
"""

import requests
import time
import sys
import json

# API 基础 URL
BASE_URL = "http://localhost:8080"

def test_endpoint(name, path, expected_status=200):
    """测试单个端点"""
    try:
        print(f"🧪 测试 {name}...", end=" ")
        url = f"{BASE_URL}{path}"
        response = requests.get(url, timeout=5)

        if response.status_code != expected_status:
            print(f"❌ 失败")
            print(f"   预期状态码: {expected_status}")
            print(f"   实际状态码: {response.status_code}")
            return False

        # 尝试解析 JSON
        try:
            data = response.json()
            print(f"✅ 成功")
            print(f"   响应: {json.dumps(data, ensure_ascii=False, indent=2)}")
            return True
        except json.JSONDecodeError:
            print(f"❌ 失败")
            print(f"   无法解析 JSON: {response.text}")
            return False

    except requests.exceptions.RequestException as e:
        print(f"❌ 失败")
        print(f"   错误: {e}")
        return False
    except Exception as e:
        print(f"❌ 失败")
        print(f"   未知错误: {e}")
        return False

def wait_for_server(max_attempts=10, delay=1):
    """等待服务器启动"""
    print("⏳ 等待服务器启动...")

    for i in range(max_attempts):
        try:
            response = requests.get(f"{BASE_URL}/health", timeout=1)
            if response.status_code == 200:
                print(f"✅ 服务器已就绪 (尝试 {i+1}/{max_attempts})")
                return True
        except requests.exceptions.RequestException:
            pass

        time.sleep(delay)

    print(f"❌ 服务器未能在 {max_attempts * delay} 秒内启动")
    return False

def run_tests():
    """运行所有测试"""
    print("=" * 60)
    print("🧪 il API 服务器测试套件")
    print("=" * 60)
    print()

    # 等待服务器
    if not wait_for_server():
        print()
        print("💡 提示: 请确保服务器正在运行")
        print("   运行: ./target/debug/api_server")
        print("   或者: python examples/build_api_server.py")
        return False

    print()

    # 测试所有端点
    results = []

    print("📡 测试 API 端点")
    print("-" * 60)

    results.append(test_endpoint("健康检查", "/health"))
    print()

    results.append(test_endpoint("问候消息", "/hello"))
    print()

    results.append(test_endpoint("服务器时间", "/time"))
    print()

    results.append(test_endpoint("能力信息", "/capabilities"))
    print()

    results.append(test_endpoint("404 处理", "/invalid", expected_status=404))
    print()

    # 统计结果
    total = len(results)
    passed = sum(results)
    failed = total - passed

    print("=" * 60)
    print("📊 测试结果")
    print("=" * 60)
    print(f"总计: {total} 个测试")
    print(f"✅ 通过: {passed}")
    print(f"❌ 失败: {failed}")
    print()

    if failed == 0:
        print("🎉 所有测试通过！")
        return True
    else:
        print(f"⚠️  {failed} 个测试失败")
        return False

def benchmark():
    """简单的性能基准测试"""
    print()
    print("=" * 60)
    print("⚡ 性能基准测试")
    print("=" * 60)
    print()

    # 热身
    print("🔥 热身...")
    for _ in range(10):
        try:
            requests.get(f"{BASE_URL}/health", timeout=1)
        except:
            pass

    # 测试吞吐量
    print("📊 测试吞吐量 (100 请求)...")
    start = time.time()
    success = 0

    for _ in range(100):
        try:
            response = requests.get(f"{BASE_URL}/health", timeout=1)
            if response.status_code == 200:
                success += 1
        except:
            pass

    duration = time.time() - start

    print(f"✅ 成功: {success}/100")
    print(f"⏱️  耗时: {duration:.2f} 秒")
    print(f"⚡ 吞吐量: {success/duration:.1f} req/s")
    print()

    # 测试延迟
    print("📊 测试延迟 (50 请求)...")
    latencies = []

    for _ in range(50):
        start = time.time()
        try:
            response = requests.get(f"{BASE_URL}/health", timeout=1)
            if response.status_code == 200:
                latency = (time.time() - start) * 1000  # 转换为毫秒
                latencies.append(latency)
        except:
            pass

    if latencies:
        latencies.sort()
        p50 = latencies[len(latencies) // 2]
        p95 = latencies[int(len(latencies) * 0.95)]
        p99 = latencies[int(len(latencies) * 0.99)]

        print(f"📈 延迟统计:")
        print(f"   最小: {min(latencies):.2f} ms")
        print(f"   p50:  {p50:.2f} ms")
        print(f"   p95:  {p95:.2f} ms")
        print(f"   p99:  {p99:.2f} ms")
        print(f"   最大: {max(latencies):.2f} ms")

    print()

if __name__ == "__main__":
    try:
        # 运行测试
        success = run_tests()

        # 如果测试通过，运行基准测试
        if success:
            try:
                benchmark()
            except KeyboardInterrupt:
                print("\n⚠️  基准测试被中断")

        sys.exit(0 if success else 1)

    except KeyboardInterrupt:
        print("\n⚠️  测试被中断")
        sys.exit(1)
