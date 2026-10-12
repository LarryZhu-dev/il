# il Greeting Server - 使用指南

## 🎯 目标用户

本指南适用于想要：
- 快速启动 il HTTP 服务器
- 学习 il 语言特性
- 理解 Capability 安全模型
- 开发自己的 il 应用

## 📋 前置条件

### 必需
- ✅ **Windows 10/11** 
- ✅ **Docker Desktop** - [下载安装](https://www.docker.com/products/docker-desktop/)
- ✅ **PowerShell** - 系统自带

### 验证安装

```powershell
# 检查 Docker
docker --version
docker info

# 检查 PowerShell
$PSVersionTable.PSVersion
```

## 🚀 三种使用方式

### 方式 1：完全自动化（最简单）⭐

**适合**：第一次使用，快速体验

**步骤**：

```powershell
# 1. 打开 PowerShell，进入项目目录
cd D:\me\project\il

# 2. 运行完整工作流
powershell -ExecutionPolicy Bypass -File ./examples/greeting_server/complete_workflow.ps1
```

**它会做什么**：
1. ✅ 检查 Docker 状态
2. ✅ 构建 il CLI（约 1-2 分钟）
3. ✅ 构建 greeting server（约 30 秒）
4. ✅ 启动服务器
5. ✅ 运行所有测试
6. ✅ 自动清理

**预期输出**：

```
========================================
il Greeting Server - Complete Workflow
========================================

Step 1/4: Building il CLI...
----------------------------------------
[构建日志...]
✓ il CLI build successful

Step 2/4: Building greeting server...
----------------------------------------
[构建日志...]
✓ Server build successful

Step 3/4: Starting server...
----------------------------------------
Server starting (Job ID: 123)...
Waiting 3 seconds for server to be ready...
✓ Server is running

Step 4/4: Running tests...
----------------------------------------
[测试输出...]
✓ All tests passed!

Server is ready to use!
```

### 方式 2：分步执行（学习推荐）

**适合**：理解构建流程，调试问题

**步骤 1：构建**

```powershell
# 构建 il CLI 和服务器
powershell -ExecutionPolicy Bypass -File ./examples/greeting_server/simple_start.ps1
```

输出：
```
========================================
il Greeting Server - Simple Build
========================================

Step 1: Building il CLI...
[构建日志...]

Step 2: Building greeting server executable...
[构建日志...]

Build completed successfully!

Server executable:
  examples/greeting_server/build/greeting_server
```

**步骤 2：运行服务器**

```powershell
# 启动服务器（前台运行）
powershell -ExecutionPolicy Bypass -File ./examples/greeting_server/run_server.ps1
```

输出：
```
========================================
il Greeting Server - Run
========================================

Configuration:
  Host: 0.0.0.0
  Port: 8080

Starting server in container...
Press Ctrl+C to stop
----------------------------------------
[2026-10-12 10:30:15] Server listening on 0.0.0.0:8080
[2026-10-12 10:30:20] GET /greeting -> 200 OK
```

**自定义参数**：

```powershell
# 自定义主机和端口
powershell -ExecutionPolicy Bypass -File ./examples/greeting_server/run_server.ps1 -Host 127.0.0.1 -Port 9090
```

**步骤 3：测试**

在**另一个 PowerShell 终端**：

```powershell
# 运行完整测试套件
powershell -ExecutionPolicy Bypass -File ./examples/greeting_server/test_server.ps1
```

输出：
```
========================================
il Greeting Server - Test Suite
========================================

Running test suite...
----------------------------------------

Test 1: GET /greeting
✓ Status: 200
✓ Body: hi, this is a server based il

Test 2: GET /invalid (404)
✓ Status: 404

Test 3: POST /greeting (Method not allowed)
✓ Status: 405

Test 4: Performance (1000 requests)
✓ Completed in 1.23s
✓ Average: 1.23ms per request

========================================
✓ All tests passed!
========================================
```

**手动测试**：

```powershell
# 单个请求测试
./tools/dev.ps1 il -RequestLine "curl http://localhost:8080/greeting"

# 预期输出
hi, this is a server based il
```

**停止服务器**：

在运行服务器的终端按 `Ctrl+C`

### 方式 3：手动执行（完全控制）

**适合**：调试，开发，理解内部机制

**步骤 1：构建 il CLI**

```powershell
cd D:\me\project\il
./tools/dev.ps1 build
```

这将在 Docker 中构建：
- Rust 编译器版本的 il CLI
- LLVM 后端
- 运行时库

输出文件：`target/debug/il`（Linux ELF 格式）

**步骤 2：构建服务器**

```powershell
# 通过 dev.ps1 在容器中运行 Python 脚本
./tools/dev.ps1 il -RequestLine "python3 /workspace/examples/greeting_server/build_and_run.py build"
```

构建流程：
1. 合并所有包源码（8 个包）
2. 生成图事务 JSON
3. 调用 `il transact` 创建语义图
4. 调用 `il compile` 生成原生 ELF

输出文件：
- `examples/greeting_server/build/transact.json` - 图事务
- `examples/greeting_server/build/state.msgpack` - 语义图（更新）
- `examples/greeting_server/build/greeting_server` - 可执行文件

**步骤 3：运行服务器**

```powershell
# 在容器中运行
./tools/dev.ps1 il -RequestLine "python3 /workspace/examples/greeting_server/build_and_run.py run --host 0.0.0.0 --port 8080"
```

**步骤 4：测试**

```powershell
# 运行测试套件
./tools/dev.ps1 il -RequestLine "python3 /workspace/examples/greeting_server/test_client.py"

# 或单个请求
./tools/dev.ps1 il -RequestLine "curl http://localhost:8080/greeting"
```

## 📊 脚本对比

| 脚本 | 用途 | 时间 | 自动化程度 |
|------|------|------|------------|
| `complete_workflow.ps1` | 一键完成所有步骤 | ~3 分钟 | ⭐⭐⭐⭐⭐ |
| `simple_start.ps1` | 只构建 | ~2 分钟 | ⭐⭐⭐⭐ |
| `run_server.ps1` | 只运行 | 立即 | ⭐⭐⭐ |
| `test_server.ps1` | 只测试 | ~10 秒 | ⭐⭐⭐ |
| 手动命令 | 完全控制 | 变化 | ⭐ |

## 🔧 常见任务

### 修改服务器代码

1. 编辑 `examples/greeting_server/main.il`
2. 重新构建：
   ```powershell
   ./tools/dev.ps1 il -RequestLine "python3 /workspace/examples/greeting_server/build_and_run.py build"
   ```
3. 重启服务器

### 修改监听端口

**方法 1：运行时参数**

```powershell
powershell -ExecutionPolicy Bypass -File ./examples/greeting_server/run_server.ps1 -Port 9090
```

**方法 2：修改 policy.json**

```json
{
  "grants": [
    {
      "entity_id": "greeting.listen",
      "kind": "Listen",
      "scope": "0.0.0.0:9090"  // 修改这里
    }
  ]
}
```

**方法 3：修改源码**

编辑 `main.il`：
```il
@id("greeting.listen") 
capability listener:Listen 
  scope "0.0.0.0:9090";  // 修改这里
```

### 添加新路由

编辑 `main.il`，添加新的 contract：

```il
@id("greeting.health")
contract health_check {
  method = "GET";
  path = "/health";
  
  @implementation
  fn handle(req: HttpRequest) -> HttpResponse {
    HttpResponse {
      status: 200,
      headers: [("Content-Type", "text/plain")],
      body: b"OK"
    }
  }
}
```

然后重新构建。

### 查看服务器日志

服务器日志输出到标准错误（stderr），在运行服务器的终端可见：

```
[2026-10-12 10:30:15] Server listening on 0.0.0.0:8080
[2026-10-12 10:30:20] GET /greeting -> 200 OK (1.2ms)
[2026-10-12 10:30:21] GET /invalid -> 404 Not Found (0.5ms)
```

### 性能测试

使用内置测试：

```powershell
./tools/dev.ps1 il -RequestLine "python3 /workspace/examples/greeting_server/test_client.py"
```

或使用 curl 循环：

```powershell
./tools/dev.ps1 il -RequestLine "for i in {1..100}; do curl -s http://localhost:8080/greeting > /dev/null; done"
```

## ⚠️ 故障排查

### Docker 未运行

**症状**：
```
Error: Docker is not running
```

**解决**：
1. 启动 Docker Desktop
2. 等待 Docker 完全启动（右下角图标变绿）
3. 重试

### 构建失败

**症状**：
```
✗ il CLI build failed
```

**解决**：
1. 检查 Docker 资源配置（至少 2GB 内存）
2. 清理 Docker：
   ```powershell
   docker system prune -a
   ```
3. 重新构建

### 服务器启动失败

**症状**：
```
✗ Server failed to start
```

**可能原因**：
1. 端口被占用
2. 构建产物不存在
3. policy.json 配置错误

**解决**：
1. 检查端口占用：
   ```powershell
   netstat -ano | findstr :8080
   ```
2. 检查构建产物：
   ```powershell
   ls examples/greeting_server/build/greeting_server
   ```
3. 使用不同端口

### 测试失败

**症状**：
```
✗ Tests failed
```

**解决**：
1. 确认服务器正在运行
2. 检查端口和主机配置
3. 查看服务器日志

### 权限错误

**症状**：
```
无法运行脚本...ExecutionPolicy
```

**解决**：
使用 `-ExecutionPolicy Bypass` 参数：
```powershell
powershell -ExecutionPolicy Bypass -File <script.ps1>
```

## 📚 下一步

### 学习资源

- [DEPLOYMENT.md](./DEPLOYMENT.md) - 详细部署指南
- [SUMMARY.md](./SUMMARY.md) - 项目总结
- [main.il](./main.il) - 源码阅读
- [il 文档](../../README.md) - 语言文档

### 扩展功能

1. **添加更多路由** - 实现 REST API
2. **JSON 支持** - 解析和生成 JSON
3. **中间件** - 请求拦截和处理
4. **数据库** - 连接 PostgreSQL/MySQL
5. **认证** - 实现 JWT/OAuth

### 贡献

发现问题或有改进建议？欢迎提交 Issue 或 PR！

## 🎉 完成！

恭喜！你现在可以：
- ✅ 快速启动 il HTTP 服务器
- ✅ 修改和扩展功能
- ✅ 测试和调试
- ✅ 理解 Capability 模型

**开始构建你的 il 应用吧！** 🚀

---

**最后更新**：2026-10-12  
**版本**：1.0.0  
**维护者**：il 项目团队
