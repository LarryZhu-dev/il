# Greeting Server 完整实现计划

## 当前状态 ✅

### 已完成
1. ✅ **P10 验收包完整实现** (commit 2c4a82f)
   - 工件清单记录衍生
   - 回滚验证记录衍生
   - 完整清单组装
   - 所有 71 个测试通过

2. ✅ **il CLI 和运行时构建**
   - 使用 `./tools/dev.ps1 build` 在 Docker 中构建
   - 输出：`target/debug/il` (Linux ELF)
   - LLVM 原生后端就绪

3. ✅ **Greeting Server 源码**
   - `main.il` - HTTP 服务器实现
   - `policy.json` - Capability 运行时策略
   - Contract 路由系统
   - 结构化日志

4. ✅ **构建和测试工具**
   - `build_and_run.py` - 自动构建和运行脚本
   - `test_client.py` - HTTP 客户端测试套件
   - `README.md` - 完整文档

## 环境约束

### Docker 容器架构
il 项目设计为在 **Linux 容器** 中构建和运行：
- **构建环境**：Ubuntu 22.04 + Rust + LLVM
- **目标平台**：Linux x86-64 ELF
- **Windows 主机**：通过 Docker Desktop 运行

### 文件位置
```
il/
├── tools/dev.ps1          # Docker 封装脚本
├── target/debug/il        # Linux ELF 可执行文件
├── packages/              # 标准库包源码
│   ├── core/
│   ├── http/
│   ├── net/
│   └── ...
└── examples/greeting_server/
    ├── main.il           # 服务器源码
    ├── policy.json       # 运行时策略
    ├── build_and_run.py  # 构建脚本
    └── build/            # 构建输出
```

## 运行方案

### 方案 1：Docker 容器运行 ⭐ 推荐

**优势**：
- ✅ 官方支持的运行环境
- ✅ 无需 WSL 或其他依赖
- ✅ 完整的 Linux 环境
- ✅ 可以映射端口到 Windows 主机

**步骤**：

1. **启动容器并构建服务器**
```powershell
# 进入交互式容器
./tools/dev.ps1 shell

# 在容器内构建服务器
cd examples/greeting_server
python3 build_and_run.py build

# 运行服务器（监听 0.0.0.0:8080）
python3 build_and_run.py run --host 0.0.0.0
```

2. **从 Windows 主机测试**
```powershell
# 获取容器 IP
docker inspect il-dev | findstr IPAddress

# 测试服务器
curl http://<container-ip>:8080/greeting
```

3. **或者映射端口**
修改 `dev.ps1` 添加端口映射：
```powershell
docker run -p 8080:8080 ...
```

### 方案 2：WSL2 运行

**优势**：
- ✅ 原生 Linux 环境
- ✅ 可以直接访问 Windows 文件系统
- ✅ 端口自动映射到 Windows

**前提条件**：
- 安装 WSL2
- 安装 Python 3

**步骤**：

1. **复制 il CLI 到 WSL**
```bash
# 在 WSL 中
cd /mnt/d/me/project/il/examples/greeting_server
```

2. **构建和运行**
```bash
python3 build_and_run.py
```

3. **从 Windows 测试**
```powershell
curl http://localhost:8080/greeting
```

### 方案 3：构建 Windows 原生版本

**挑战**：
- ❌ 需要 Windows 上的 Rust 工具链
- ❌ 需要 LLVM Windows 版本
- ❌ il 运行时可能需要适配
- ⚠️  未在 Windows 上验证

**不推荐**：项目设计目标是 Linux，Windows 支持未验证。

## 下一步操作

### 立即可用的操作

#### 选项 A：Docker 容器运行 ⭐

```powershell
# 1. 启动开发容器
./tools/dev.ps1 shell

# 2. 在容器内运行以下命令：
cd examples/greeting_server

# 构建服务器
python3 build_and_run.py build

# 运行服务器
python3 build_and_run.py run --host 0.0.0.0

# 在另一个终端测试
docker exec -it il-dev curl http://localhost:8080/greeting
```

#### 选项 B：WSL2 运行

```bash
# 在 WSL2 中
cd /mnt/d/me/project/il/examples/greeting_server
python3 build_and_run.py

# 测试
curl http://localhost:8080/greeting
```

### 验证清单

运行以下测试确认服务器工作：

```bash
# 1. 基本功能测试
curl -v http://localhost:8080/greeting
# 预期：200 OK, "hi, this is a server based il"

# 2. 404 测试
curl -v http://localhost:8080/invalid
# 预期：404 Not Found

# 3. 自动化测试套件
python3 test_client.py
# 预期：所有测试通过

# 4. 性能基准测试
python3 test_client.py  # 包含 1000 req benchmark
```

## 技术细节

### 构建流程

```
源码 (main.il)
  ↓
合并包源码 (core, http, net, ...)
  ↓
生成图事务 (transact.json)
  ↓
il CLI 编译 (JSON → Graph → HIR → MIR → LLVM IR)
  ↓
LLVM 原生编译 (IR → ELF)
  ↓
可执行文件 (greeting_server)
```

### Capability 注入流程

```
编译时：
  main.il 声明 capability
    @id("greeting.listen") capability listener:Listen

运行时：
  policy.json 授予权限
    {"entity_id": "greeting.listen", "kind": "Listen", ...}
    
  il runtime 验证并注入
    runtime.net_listen[greeting.listen]() ✅
```

### HTTP 处理流程

```
TCP Listen (127.0.0.1:8080)
  ↓
Accept Connection
  ↓
Read HTTP Request
  ↓
Parse (method, path, headers)
  ↓
Route via Contract
  ↓
Execute Handler
  ↓
Send HTTP Response
  ↓
Close Connection
```

## 已知问题和限制

1. **Windows 原生不支持**
   - il CLI 是 Linux ELF
   - 必须在容器或 WSL 中运行

2. **端口绑定**
   - Docker 容器需要端口映射
   - WSL2 自动映射端口

3. **文件路径**
   - build_and_run.py 假设相对路径
   - 在容器中运行需要正确的工作目录

## 成功标准

✅ **最小可行服务器**：
- [ ] 在 Linux 环境中成功构建
- [ ] 启动并监听 8080 端口
- [ ] 响应 GET /greeting 请求
- [ ] 返回正确的响应体

✅ **生产就绪**：
- [ ] 所有测试通过
- [ ] 日志输出正常
- [ ] 可以处理并发请求
- [ ] 优雅关闭

## 参考文档

- [il README](../../README.md) - 项目概述
- [RFC 0005](../../docs/rfcs/0005-text-syntax.md) - 文本语法
- [RFC 0008](../../docs/rfcs/0008-transactions.md) - 图事务
- [HTTP 包](../../packages/http/) - HTTP 实现
- [Runtime 包](../../packages/runtime/) - 运行时接口

## 结论

**当前最佳路径**：使用 Docker 容器运行，这是项目设计的标准方式。

```powershell
# 一键启动服务器
./tools/dev.ps1 shell
cd examples/greeting_server && python3 build_and_run.py
```

Windows 原生支持需要额外的移植工作，不在当前范围内。
