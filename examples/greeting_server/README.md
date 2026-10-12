# Greeting Server - il HTTP 后端示例

这是一个使用 il 语言编写的完整 HTTP 服务器示例，展示了 il 的 capability 安全模型和网络编程能力。

## 功能特性

- ✅ HTTP/1.1 协议支持
- ✅ 路由系统（contract 声明）
- ✅ Capability 安全模型
- ✅ 结构化日志输出
- ✅ 纯 il 实现（无外部依赖）

## 快速开始

### 前置条件

- **Docker Desktop** - 已安装并运行
- **PowerShell** - Windows 自带

### 方法 1：完整工作流（推荐）⭐

一键完成构建、运行和测试：

```powershell
# 从项目根目录运行
cd D:\me\project\il
powershell -ExecutionPolicy Bypass -File ./examples/greeting_server/complete_workflow.ps1
```

这将自动：
1. ✅ 构建 il CLI
2. ✅ 构建 greeting server
3. ✅ 启动服务器
4. ✅ 运行测试套件
5. ✅ 停止服务器

### 方法 2：分步执行

**步骤 1：构建**

```powershell
# 构建 il CLI 和服务器
powershell -ExecutionPolicy Bypass -File ./examples/greeting_server/simple_start.ps1
```

**步骤 2：运行**

```powershell
# 启动服务器
powershell -ExecutionPolicy Bypass -File ./examples/greeting_server/run_server.ps1

# 或指定参数
powershell -ExecutionPolicy Bypass -File ./examples/greeting_server/run_server.ps1 -Host 0.0.0.0 -Port 9090
```

**步骤 3：测试**

```powershell
# 在另一个终端运行测试
powershell -ExecutionPolicy Bypass -File ./examples/greeting_server/test_server.ps1

# 或手动测试
./tools/dev.ps1 il -RequestLine "curl http://localhost:8080/greeting"
```

### 方法 3：手动构建（理解细节）

### 1. 构建 il CLI

首先需要构建 il CLI 和运行时：

```powershell
# 在项目根目录
./tools/dev.ps1 build
```

这将在 Docker 容器中构建：
- il CLI（图编译器）
- LLVM 原生后端
- 运行时库

### 2. 构建服务器

```bash
cd examples/greeting_server
python build_and_run.py build
```

构建流程：
1. 合并包源码（core, alloc, io, time, net, json, http, tracing）
2. 创建图事务（transact.json）
3. 编译到原生 ELF 可执行文件

### 3. 运行服务器

```bash
python build_and_run.py
```

服务器将监听 `http://127.0.0.1:8080`

### 4. 测试服务器

```bash
curl http://127.0.0.1:8080/greeting
# 响应: hi, this is a server based il
```

## 架构说明

### Capability 系统

il 使用声明式 capability 保证安全：

```il
// 1. 编译时声明需要的权限
@id("greeting.listen") 
capability listener:Listen 
  scope "127.0.0.1:8080";

@id("greeting.clock") 
capability clock:ClockRead;

// 2. 使用声明的 capability
let listener = runtime.net_listen[greeting.listen]();
let now = runtime.clock_now[greeting.clock]();
```

**运行时注入**（policy.json）：
```json
{
  "schema_version": "1.0.0",
  "grants": [
    {
      "entity_id": "greeting.listen",
      "kind": "Listen",
      "scope": "127.0.0.1:8080"
    },
    {
      "entity_id": "greeting.clock",
      "kind": "ClockRead"
    }
  ]
}
```

### HTTP 服务器流程

```
监听 127.0.0.1:8080
  ↓
接受客户端连接
  ↓
解析 HTTP 请求（方法、路径、headers）
  ↓
路由分发（通过 contract）
  ↓
生成 HTTP 响应
  ↓
关闭连接
```

### Contract 路由系统

```il
@id("greeting.root")
contract root {
  method = "GET";
  path = "/greeting";
  
  @implementation
  fn handle(req: HttpRequest) -> HttpResponse {
    HttpResponse {
      status: 200,
      headers: [],
      body: b"hi, this is a server based il"
    }
  }
}
```

Contract 在编译时验证：
- 方法和路径的唯一性
- 处理函数签名正确
- 所有路由都有实现

## 文件说明

- **main.il** - 服务器源码
  - `main()` - 入口函数
  - `root` contract - /greeting 路由实现
  - capability 声明

- **policy.json** - 运行时权限策略
  - Listen capability (127.0.0.1:8080)
  - ClockRead capability

- **build_and_run.py** - 构建和运行脚本
  - 自动合并包源码
  - 调用 il CLI 编译
  - 注入 policy 运行

- **build/** - 构建输出目录
  - transact.json - 图事务
  - state.msgpack - 编译后的图
  - greeting_server - 原生可执行文件

## 扩展开发

### 添加新路由

```il
@id("greeting.user")
contract user_greeting {
  method = "GET";
  path = "/greeting/{name}";
  
  @implementation
  fn handle(req: HttpRequest) -> HttpResponse {
    let name = req.path_params.get("name");
    let body = format("Hello, {}!", name);
    HttpResponse {
      status: 200,
      headers: [],
      body: body.as_bytes()
    }
  }
}
```

### JSON 响应示例

```il
@id("greeting.json")
contract json_response {
  method = "GET";
  path = "/api/status";
  
  @implementation
  fn handle(req: HttpRequest) -> HttpResponse {
    let json = json::object([
      ("status", json::string("ok")),
      ("timestamp", json::number(runtime.clock_now[greeting.clock]())),
    ]);
    
    HttpResponse {
      status: 200,
      headers: [("Content-Type", "application/json")],
      body: json.to_bytes()
    }
  }
}
```

### 添加日志记录

```il
tracing.info("Client connected", [
  ("remote_addr", client_addr.to_string()),
  ("method", request.method),
  ("path", request.path),
]);
```

## 性能特点

- **零拷贝解析** - HTTP 解析直接在接收缓冲区上操作
- **静态分发** - Contract 路由在编译时解析
- **原生代码** - 通过 LLVM 编译到机器码
- **最小运行时** - 无 GC，无解释器开销

## 安全保证

1. **编译时隔离** - 未声明的 capability 无法编译
2. **运行时验证** - Policy 在进程启动时验证
3. **类型安全** - 静态类型系统防止内存错误
4. **资源边界** - Capability scope 限制访问范围

## 故障排查

### 构建失败

```bash
# 检查 il CLI 是否存在
ls -lh ../../target/debug/il

# 重新构建 il CLI
cd ../..
./tools/dev.ps1 build
```

### 运行时错误

```bash
# 检查 policy 文件
cat policy.json

# 查看详细日志（stderr）
python build_and_run.py 2>&1 | tee server.log
```

### 端口被占用

修改 policy.json 和 main.il 中的端口：
```json
"scope": "127.0.0.1:9090"
```

## 参考资料

- [RFC 0005](../../docs/rfcs/0005-text-syntax.md) - il 文本语法
- [RFC 0008](../../docs/rfcs/0008-transactions.md) - 图事务系统
- [RFC 0013](../../docs/rfcs/0013-interpreter.md) - 解释器接口
- [HTTP 包文档](../../packages/http/README.md) - HTTP 协议实现
- [Capability 设计](../../docs/capabilities.md) - 安全模型

## 许可证

与 il 项目相同的许可证。
