#!/bin/bash
# Greeting Server 测试脚本

set -e

echo "==> 测试 Greeting Server"
echo ""

# 等待服务启动
echo "等待服务启动..."
for i in {1..30}; do
    if curl -s -f http://127.0.0.1:8080/greeting > /dev/null 2>&1; then
        echo "✓ 服务已就绪"
        break
    fi
    if [ $i -eq 30 ]; then
        echo "✗ 服务启动超时"
        exit 1
    fi
    sleep 1
done

echo ""

# 测试 /greeting 路由
echo "[测试 1] GET /greeting"
response=$(curl -s http://127.0.0.1:8080/greeting)
expected="hi, this is a server based il"

if [ "$response" = "$expected" ]; then
    echo "✓ 响应正确: $response"
else
    echo "✗ 响应错误"
    echo "  期望: $expected"
    echo "  实际: $response"
    exit 1
fi

echo ""

# 测试 404
echo "[测试 2] GET /notfound (应该返回 404)"
status_code=$(curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:8080/notfound)

if [ "$status_code" = "404" ]; then
    echo "✓ 正确返回 404"
else
    echo "✗ 状态码错误: $status_code"
    exit 1
fi

echo ""

# 测试超时
echo "[测试 3] 性能测试 (10 次请求)"
start=$(date +%s%3N)
for i in {1..10}; do
    curl -s http://127.0.0.1:8080/greeting > /dev/null
done
end=$(date +%s%3N)
elapsed=$((end - start))

echo "✓ 10 次请求完成，耗时: ${elapsed}ms"
echo "  平均响应时间: $((elapsed / 10))ms"

echo ""
echo "✓ 所有测试通过！"
