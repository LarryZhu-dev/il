# il 语言 HTTP 服务器示例

这是一个完整的 il 语言 HTTP API 服务器示例，展示了如何使用 il 编写真实的网络服务。

## 📁 文件

- **api_server.il** - 完整的 HTTP 服务器实现（~180 行）

## ✨ 特性

### API 端点

1. **GET /hello** - 问候消息
   - 返回欢迎消息和时间戳
   - 展示 JSON 响应

2. **GET /health** - 健康检查
   - 返回服务状态
   - 用于监控和负载均衡

3. **GET /time** - 服务器时间
   - 返回当前时间（纳秒）
   - 展示时钟能力使用

4. **GET /capabilities** - 能力信息
   - 返回服务支持的功能
   - 列出所有可用端点

### il 语言特性展示

- ✅ **能力系统** - Listen 和 ClockRead 能力
- ✅ **效应跟踪** - alloc, net, process 效应
- ✅ **错误处理** - Result 类型和 match 表达式
- ✅ **JSON 处理** - 对象、数组、序列化
- ✅ **HTTP 协议** - 请求解析、响应生成
- ✅ **网络编程** - 监听、接受连接、读写
- ✅ **时间管理** - 超时、截止时间
- ✅ **事件跟踪** - tracing 系统

## 🚀 使用方法

### 编译服务器

```bash
# 编译 il 代码
cargo run --bin il -- compile examples/api_server.il -o api_server

# 或使用构建脚本
python build.py --source examples/api_server.il --output api_server
```

### 运行服务器

```bash
# 运行编译后的二进制
./api_server

# 服务器将在 0.0.0.0:8080 监听
```

### 测试 API

```bash
# 健康检查
curl http://localhost:8080/health

# 问候消息
curl http://localhost:8080/hello

# 服务器时间
curl http://localhost:8080/time

# 能力信息
curl http://localhost:8080/capabilities
```

## 📊 示例响应

### GET /hello

```json
{
  "message": "你好！欢迎使用 il HTTP 服务器",
  "timestamp": 1728734567000000000,
  "language": "il"
}
```

### GET /health

```json
{
  "status": "healthy",
  "service": "il_api_server",
  "version": "1.0.0"
}
```

### GET /time

```json
{
  "current_time": 1728734567123456789,
  "unit": "nanoseconds"
}
```

### GET /capabilities

```json
{
  "capabilities": [
    "http_server",
    "json_api",
    "time_service"
  ],
  "endpoints": [
    "/hello",
    "/health",
    "/time",
    "/capabilities"
  ]
}
```

### 404 Not Found

```json
{
  "error": "Not Found",
  "path": "/invalid",
  "message": "请求的端点不存在"
}
```

## 🏗️ 架构说明

### 模块结构

```
api_server (模块)
├── 能力声明
│   ├── listener:Listen - 网络监听
│   └── clock:ClockRead - 时钟读取
├── 端点处理器
│   ├── hello() - 问候端点
│   ├── health() - 健康检查
│   ├── get_time() - 时间端点
│   ├── capabilities() - 能力端点
│   └── not_found() - 404 处理
├── 请求路由
│   └── dispatch() - 路由分发
├── 连接处理
│   └── connection() - 处理单个连接
└── 主函数
    └── main() - 服务器主循环
```

### 请求处理流程

1. **监听** - `net_listen` 在 8080 端口监听
2. **接受** - `net_accept` 接受新连接
3. **解析** - `http.parse_request` 解析 HTTP 请求
4. **路由** - `dispatch` 根据路径分发请求
5. **处理** - 调用相应的端点处理器
6. **响应** - `http.write_response` 发送 HTTP 响应
7. **循环** - 返回步骤 2，处理下一个连接

### 超时管理

- **读取超时**: 5 秒
- **写入超时**: 5 秒
- 防止连接挂起

### 错误处理

所有错误都会记录到 tracing 系统：
- `E_HTTP_LISTEN_FAILED` - 监听失败
- `E_HTTP_ACCEPT_FAILED` - 接受连接失败
- `E_HTTP_REQUEST_FAILED` - 请求解析失败
- `E_HTTP_WRITE_TIMEOUT` - 写入超时
- `E_HTTP_PEER_DISCONNECT` - 对端断开
- `E_HTTP_WRITE_FAILED` - 写入失败

## 🎓 学习要点

### 1. 能力系统

```il
// 声明需要的能力
@id("api_server.listen") capability listener:Listen scope "0.0.0.0:8080";
@id("api_server.clock") capability clock:ClockRead scope null;
```

### 2. 效应跟踪

```il
// 函数声明其效应
fn hello() -> http.ResponseResult effects [alloc, process] {
  // ...
}
```

### 3. Result 类型

```il
// 使用 Result 处理可能的错误
let parsed: http.ParseResult = http.parse_request(stream, read_deadline);

match parsed {
  Ok(request) => { /* 成功 */ }
  Err(error) => { /* 失败 */ }
}
```

### 4. JSON 构建

```il
// 构建 JSON 对象
let obj: json.Value = json.object([
  ("key1", json.string("value")),
  ("key2", json.number(123))
])?;

// 序列化为字节
let bytes: Bytes = json.serialize(obj)?;
```

### 5. HTTP 响应

```il
// 创建 HTTP 响应
return Ok(http.Response::Response(
  200,                    // 状态码
  "application/json",     // Content-Type
  serialized              // 响应体
));
```

## 🔧 扩展建议

### 添加新端点

```il
// 1. 实现处理器函数
@id("api_server.new_endpoint") fn new_endpoint() -> http.ResponseResult effects [alloc] {
  let response: json.Value = json.object([
    ("data", json.string("your data"))
  ])?;
  let serialized: Bytes = json.serialize(response)?;
  return Ok(http.Response::Response(200, "application/json", serialized));
}

// 2. 在 dispatch 中添加路由
if path == "/new" { return new_endpoint(); }
```

### 支持 POST 请求

```il
@id("api_server.dispatch") fn dispatch(method: http.Method, path: String) -> http.ResponseResult {
  match method {
    Get => {
      // GET 路由
    }
    Post => {
      // POST 路由
      if path == "/data" { return handle_post(); }
    }
  }
}
```

### 添加状态管理

```il
// 使用可变状态（需要额外的能力和效应）
@id("api_server.state") capability state:MemoryAccess scope null;

// 在函数中读写状态
fn with_state() -> http.ResponseResult effects [alloc, memory] {
  // 实现状态逻辑
}
```

## 📝 注意事项

### 当前实现

- ✅ 同步单线程模型
- ✅ 一次处理一个连接
- ✅ 简单的路由系统
- ✅ 基础的错误处理

### 生产环境建议

- 🔄 实现并发连接处理
- 🔄 添加连接池
- 🔄 实现请求体解析
- 🔄 添加认证和授权
- 🔄 实现日志记录
- 🔄 添加性能监控

## 🎯 与其他示例对比

| 特性 | api_server.il | demo_server.py |
|------|---------------|----------------|
| 语言 | il | Python |
| 类型 | 静态类型 | 动态类型 |
| 能力 | 显式声明 | 隐式使用 |
| 效应 | 跟踪 | 未跟踪 |
| 编译 | 提前编译 | 解释执行 |
| 性能 | 高 | 中 |
| 安全 | 能力安全 | 标准安全 |

## 🚀 性能预期

基于 il 的设计目标：

- **吞吐量**: 预期 > 10,000 req/s
- **延迟**: 预期 < 1ms (p50)
- **内存**: 最小占用，无 GC 暂停
- **启动**: 极快（无运行时初始化）

## 📚 相关资源

- **il 语言规范**: 查看 `docs/language.md`
- **HTTP 能力**: 查看 `docs/capabilities/http.md`
- **示例代码**: 查看 `examples/` 目录
- **测试**: 查看 `tests/http_blackbox/` 目录

## 🎉 总结

这个示例展示了：

1. ✅ 如何用 il 编写 HTTP 服务器
2. ✅ 能力系统的实际应用
3. ✅ 效应跟踪的价值
4. ✅ 类型安全的网络编程
5. ✅ 完整的错误处理
6. ✅ JSON API 实现

**现在你可以用 il 编写真正的网络服务了！** 🎊
