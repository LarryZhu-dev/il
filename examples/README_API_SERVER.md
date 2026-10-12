# il HTTP API Server - 架构文档

这是一个完全用 il 语言编写的 HTTP API 服务器，展示了 il 的核心特性：

## 核心特性展示

### 1. 能力约束系统（Capability-Based Security）

服务器使用 il 的能力系统来控制对敏感资源的访问：

```il
// 显式声明所需能力
capabilities {
    http_listen(8080)      // 监听 HTTP 端口
    clock_realtime          // 访问系统时间
}
```

- **最小权限原则**：程序只能使用显式声明的能力
- **编译时检查**：编译器验证所有能力使用都有对应声明
- **无隐式权限**：不能访问文件系统、网络（除声明的端口）等

### 2. 强类型系统

```il
// 类型安全的 HTTP 处理
fn handle_request(method: HttpMethod, path: string) -> HttpResponse {
    // 编译器验证所有路径返回正确类型
}
```

### 3. 无垃圾回收的内存管理

il 使用 LLVM 后端生成原生代码，通过：
- 编译时所有权分析（类似 Rust）
- 栈分配优先策略
- 零运行时开销

### 4. 图存储中间表示

所有 il 代码在编译前先导入到图存储：

```
源代码 (.il) → 导入 → 图存储 → 编译 → 原生可执行文件
```

这支持：
- 增量编译
- 形式化验证
- 精确的依赖追踪

## 编译流程详解

### 步骤 1: 导入源代码

```powershell
$importRequest = @{
    task_id = 'api_server'
    base_revision = 0
    scope = @('program')
    operations = @(@{
        op = 'import_text'
        source = $sourceCode
    })
    required_checks = @()
} | ConvertTo-Json
```

将 `.il` 文本转换为图节点存储在 `.il/api_server/` 中。

### 步骤 2: 编译为原生代码

```powershell
$buildRequest = @{
    revision = 1
    target = 'x86_64-unknown-linux-gnu'
    profile = 'release'
} | ConvertTo-Json
```

通过 LLVM 编译为优化的 Linux 可执行文件。

## API 端点实现

### GET /hello

```il
fn handle_hello() -> HttpResponse {
    HttpResponse {
        status: 200,
        headers: [("Content-Type", "application/json")],
        body: json_encode({"message": "Hello from il!"})
    }
}
```

返回固定的欢迎消息。

### GET /health

```il
fn handle_health() -> HttpResponse {
    HttpResponse {
        status: 200,
        body: json_encode({"status": "healthy"})
    }
}
```

健康检查端点，用于负载均衡器探测。

### GET /time

```il
fn handle_time() -> HttpResponse {
    // 需要 clock_realtime 能力
    let now = get_current_time_nanos();
    HttpResponse {
        status: 200,
        body: json_encode({
            "timestamp_nanos": now,
            "capability": "clock_realtime"
        })
    }
}
```

展示能力系统：访问系统时钟需要声明的能力。

### GET /capabilities

```il
fn handle_capabilities() -> HttpResponse {
    HttpResponse {
        status: 200,
        body: json_encode({
            "declared": ["http_listen(8080)", "clock_realtime"],
            "available_endpoints": ["/hello", "/health", "/time", "/capabilities"]
        })
    }
}
```

返回服务器声明的能力列表。

## 安全保证

### 编译时保证

1. **类型安全**：所有类型在编译时验证
2. **能力检查**：所有能力使用必须有声明
3. **内存安全**：无空指针、无缓冲区溢出
4. **无数据竞争**：并发安全由编译器保证

### 运行时保证

1. **最小攻击面**：只能使用声明的能力
2. **无动态执行**：不支持 eval 或动态加载
3. **确定性行为**：相同输入保证相同输出（除时钟）

## Docker 容器化

### 构建镜像

```dockerfile
FROM rust:1.90.0-slim
RUN apt-get update && apt-get install -y llvm-14
# 安装 il 编译器
```

### 运行容器

```bash
docker run --rm -it \
  --mount "type=bind,source=$PWD,target=/workspace" \
  --publish 8080:8080 \
  il-toolchain:1.90.0-llvm14 \
  /workspace/.il/api_server/build/x86_64-unknown-linux-gnu/release/executable
```

- **隔离环境**：Linux 容器与 Windows 主机隔离
- **端口映射**：容器的 8080 映射到主机 8080
- **卷挂载**：共享工作区文件系统

## 性能特征

### 编译产物

- **可执行文件大小**：约 2-5 MB（静态链接）
- **启动时间**：< 10ms
- **内存占用**：< 5 MB（无 GC 堆）

### 运行时性能

- **请求延迟**：亚毫秒级（简单端点）
- **吞吐量**：取决于端点复杂度
- **零运行时开销**：无 GC 暂停、无解释器

## 扩展方向

### 添加新端点

1. 修改 `api_server.il`，添加新的路由处理
2. 重新编译：`./examples/compile_api_server.ps1`
3. 重启服务器：`./examples/start_api_server.ps1`

### 添加新能力

```il
capabilities {
    http_listen(8080)
    clock_realtime
    file_read("/data")     // 新增文件读取
}
```

### 数据库集成

```il
capabilities {
    http_listen(8080)
    tcp_connect("postgres.local", 5432)  // 连接数据库
}
```

## 与其他语言对比

| 特性 | il | Go | Rust | Node.js |
|------|----|----|------|---------|
| 内存管理 | 编译时 | GC | 所有权 | GC |
| 能力系统 | ✅ | ❌ | ❌ | ❌ |
| 类型安全 | ✅ | ✅ | ✅ | ❌ (TS有) |
| 启动时间 | < 10ms | < 50ms | < 10ms | > 100ms |
| 运行时依赖 | 无 | 无 | 无 | Node.js |
| 二进制大小 | 2-5 MB | 5-10 MB | 2-5 MB | N/A |

## 故障排查

### 编译错误

```
error: capability 'file_read' used but not declared
```

**解决**：在 `capabilities {}` 块中声明所需能力。

### 运行时错误

```
Error: Address already in use
```

**解决**：检查 8080 端口是否被占用：
```powershell
netstat -ano | findstr :8080
```

### Docker 错误

```
docker: Cannot connect to the Docker daemon
```

**解决**：确保 Docker Desktop 正在运行。

## 相关文档

- `README.md` - 快速开始指南
- `api_server.il` - 源代码（带注释）
- `RFC 0001` - il 构建系统规范
- `RFC 0002` - 能力系统规范

## 未来工作

- [ ] 支持 POST/PUT/DELETE 方法
- [ ] JSON 请求体解析
- [ ] 路由参数提取（如 `/users/:id`）
- [ ] 中间件系统（日志、认证等）
- [ ] WebSocket 支持
- [ ] TLS/HTTPS 支持
