#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
il 后端服务简单使用示例
演示如何在应用中集成 il HTTP API
"""
import requests
import json
import sys
import io

# 修复 Windows 控制台编码问题
if sys.platform == 'win32':
    sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding='utf-8')
    sys.stderr = io.TextIOWrapper(sys.stderr.buffer, encoding='utf-8')

# 服务器地址
BASE_URL = "http://127.0.0.1:8000"


def print_section(title):
    """打印章节标题"""
    print(f"\n{'=' * 60}")
    print(f"  {title}")
    print('=' * 60)


def example_1_basic_operations():
    """示例 1: 基础操作"""
    print_section("示例 1: 基础操作")

    # 1. 检查服务健康状态
    print("\n1. 检查服务健康状态")
    r = requests.get(f"{BASE_URL}/health")
    print(f"状态: {r.status_code}")
    print(json.dumps(r.json(), indent=2, ensure_ascii=False))

    # 2. 获取服务能力
    print("\n2. 获取服务能力")
    r = requests.get(f"{BASE_URL}/capabilities")
    print(json.dumps(r.json(), indent=2, ensure_ascii=False))

    # 3. 获取当前图状态
    print("\n3. 获取当前图状态")
    r = requests.get(f"{BASE_URL}/graph/state")
    state = r.json()
    print(f"节点数量: {len(state['nodes'])}")
    print(f"边数量: {len(state['edges'])}")


def example_2_create_nodes():
    """示例 2: 创建节点"""
    print_section("示例 2: 创建节点")

    # 重置图到初始状态
    print("\n1. 重置图")
    r = requests.delete(f"{BASE_URL}/graph/reset")
    print(f"重置结果: {r.json()['status']}")

    # 创建多个节点
    nodes = [
        {
            "id": "name",
            "type": "constant",
            "value": {"type": "text", "content": "Alice"}
        },
        {
            "id": "age",
            "type": "constant",
            "value": {"type": "number", "content": 30}
        },
        {
            "id": "greeting",
            "type": "constant",
            "value": {"type": "text", "content": "你好"}
        }
    ]

    print("\n2. 创建节点")
    for node in nodes:
        r = requests.post(f"{BASE_URL}/graph/node", json=node)
        if r.status_code == 200:
            print(f"✓ 创建节点: {node['id']}")
        else:
            print(f"✗ 创建失败: {node['id']} - {r.json()}")


def example_3_create_relationships():
    """示例 3: 创建关系"""
    print_section("示例 3: 创建关系")

    edges = [
        {"source": "greeting", "target": "name", "label": "greets"},
        {"source": "name", "target": "age", "label": "has_age"}
    ]

    print("\n创建边")
    for edge in edges:
        r = requests.post(f"{BASE_URL}/graph/edge", json=edge)
        if r.status_code == 200:
            print(f"✓ {edge['source']} --[{edge['label']}]--> {edge['target']}")
        else:
            print(f"✗ 创建失败: {r.json()}")

    # 查看最终状态
    print("\n最终图状态")
    r = requests.get(f"{BASE_URL}/graph/state")
    state = r.json()
    print(f"节点: {list(state['nodes'].keys())}")
    print(f"边数量: {len(state['edges'])}")


def example_4_execute_nodes():
    """示例 4: 执行节点"""
    print_section("示例 4: 执行节点")

    nodes_to_execute = ["greeting", "name", "age"]

    for node_id in nodes_to_execute:
        r = requests.post(f"{BASE_URL}/graph/execute",
                         json={"node_id": node_id})
        if r.status_code == 200:
            result = r.json()
            print(f"\n执行节点: {node_id}")
            print(f"状态: {result['status']}")
            print(f"结果: {result['result']}")
        else:
            print(f"✗ 执行失败: {node_id}")


def example_5_query_nodes():
    """示例 5: 查询节点"""
    print_section("示例 5: 查询节点")

    print("\n查询所有节点详情")
    r = requests.get(f"{BASE_URL}/graph/state")
    state = r.json()

    for node_id, node_data in state['nodes'].items():
        print(f"\n节点 ID: {node_id}")
        print(f"  类型: {node_data['type']}")
        print(f"  值: {node_data['value']}")


def example_6_error_handling():
    """示例 6: 错误处理"""
    print_section("示例 6: 错误处理")

    # 1. 尝试获取不存在的节点
    print("\n1. 获取不存在的节点")
    r = requests.get(f"{BASE_URL}/graph/node/nonexistent")
    print(f"状态码: {r.status_code}")
    print(f"错误信息: {r.json()['detail']}")

    # 2. 尝试创建重复节点
    print("\n2. 创建重复节点")
    r = requests.post(f"{BASE_URL}/graph/node",
                     json={
                         "id": "greeting",
                         "type": "constant",
                         "value": {"type": "text", "content": "重复"}
                     })
    print(f"状态码: {r.status_code}")
    print(f"错误信息: {r.json()['detail']}")

    # 3. 尝试创建无效的边
    print("\n3. 创建无效的边")
    r = requests.post(f"{BASE_URL}/graph/edge",
                     json={
                         "source": "nonexistent",
                         "target": "greeting",
                         "label": "invalid"
                     })
    print(f"状态码: {r.status_code}")
    print(f"错误信息: {r.json()['detail']}")


def example_7_integration_pattern():
    """示例 7: 集成模式"""
    print_section("示例 7: 应用集成模式")

    print("\n模拟一个简单的工作流：")
    print("1. 初始化环境")
    print("2. 创建数据节点")
    print("3. 建立关系")
    print("4. 执行计算")
    print("5. 获取结果")

    # 1. 初始化
    requests.delete(f"{BASE_URL}/graph/reset")
    print("\n✓ 环境已重置")

    # 2. 创建数据节点
    data = {
        "user": {"id": "user", "type": "constant",
                "value": {"type": "text", "content": "张三"}},
        "product": {"id": "product", "type": "constant",
                   "value": {"type": "text", "content": "商品A"}},
        "quantity": {"id": "quantity", "type": "constant",
                    "value": {"type": "number", "content": 2}}
    }

    for node_data in data.values():
        requests.post(f"{BASE_URL}/graph/node", json=node_data)
    print("✓ 数据节点已创建")

    # 3. 建立关系
    edges = [
        {"source": "user", "target": "product", "label": "购买"},
        {"source": "product", "target": "quantity", "label": "数量"}
    ]
    for edge in edges:
        requests.post(f"{BASE_URL}/graph/edge", json=edge)
    print("✓ 关系已建立")

    # 4. 执行并获取结果
    print("\n执行结果：")
    for node_id in ["user", "product", "quantity"]:
        r = requests.post(f"{BASE_URL}/graph/execute",
                         json={"node_id": node_id})
        result = r.json()['result']
        print(f"  {node_id}: {result['content']}")

    print("\n✓ 工作流完成")


def main():
    """运行所有示例"""
    print("=" * 60)
    print("  il 后端服务使用示例")
    print("=" * 60)
    print(f"\n服务地址: {BASE_URL}")

    # 检查服务是否可用
    try:
        r = requests.get(f"{BASE_URL}/health", timeout=2)
        if r.status_code != 200:
            print(f"\n错误: 服务返回状态 {r.status_code}")
            print("请确保服务器正在运行: python demo_server.py")
            return
    except requests.exceptions.ConnectionError:
        print("\n错误: 无法连接到服务器")
        print("请先启动服务器: python demo_server.py")
        return

    try:
        # 运行所有示例
        example_1_basic_operations()
        example_2_create_nodes()
        example_3_create_relationships()
        example_4_execute_nodes()
        example_5_query_nodes()
        example_6_error_handling()
        example_7_integration_pattern()

        print("\n" + "=" * 60)
        print("  所有示例运行完成！")
        print("=" * 60)
        print("\n下一步:")
        print("1. 查看 DEMO_SERVER_README.md 了解更多 API")
        print("2. 查看 QUICKSTART.md 了解部署选项")
        print("3. 将 API 集成到您的应用中")

    except Exception as e:
        print(f"\n错误: {e}")
        import traceback
        traceback.print_exc()


if __name__ == "__main__":
    main()
