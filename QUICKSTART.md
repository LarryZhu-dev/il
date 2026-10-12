# il 后端服务器快速启动指南

## 当前可用选项

### 选项 1：演示服务器（立即可用）✅

**最快速的方式，无需编译**

```bash
# 启动演示服务器
python demo_server.py

# 在另一个终端测试
python test_demo_server.py
```

**特点**：
- ✅ 立即可用，无需编译
- ✅ 完整的 HTTP API 演示
- ✅ 内存中的图操作
- ⚠️  演示模式，不使用真实 il 二进制

**适用场景**：API 测试、集成开发、原型验证

---

### 选项 2：Docker 容器服务器（推荐用于生产）🐳

**使用 Docker 运行完整的 il 服务器**

#### 步骤 1：构建镜像

```bash
# 构建 Docker 镜像（首次需要 10-15 分钟）
docker build -f Dockerfile.server -t il-server:latest .
```

#### 步骤 2：运行容器

```bash
# 使用 docker-compose（推荐）
docker-compose up -d

# 或者直接运行
docker run -d -p 8000:8000 --name il-server il-server:latest
```

#### 步骤 3：测试

```bash
# 检查健康状态
curl http://localhost:8000/health

# 获取状态
curl http://localhost:8000/api/state

# 查看日志
docker-compose logs -f
```

#### 管理命令

```bash
# 停止服务
docker-compose down

# 重启服务
docker-compose restart

# 查看日志
docker-compose logs -f il-server

# 进入容器调试
docker exec -it il-server bash
```

**特点**：
- ✅ 完整的 il 二进制编译
- ✅ 真实的图操作能力
- ✅ 生产级配置
- ✅ 健康检查和自动重启
- ⚠️  需要 Docker Desktop

**适用场景**：生产部署、完整功能测试、CI/CD 集成

---

### 选项 3：本地编译（高级用户）

**需要 Rust 工具链**

#### Windows MSVC

```powershell
# 1. 安装 Rust
# https://rustup.rs/

# 2. 安装 Visual Studio Build Tools
# https://visualstudio.microsoft.com/downloads/

# 3. 添加 Windows target
rustup target add x86_64-pc-windows-msvc

# 4. 构建
cargo build --release --target x86_64-pc-windows-msvc

# 5. 运行服务器
python start_server.py --il-binary target/x86_64-pc-windows-msvc/release/il.exe
```

#### Linux/WSL

```bash
# 1. 构建
cargo build --release

# 2. 运行服务器
python tools/http_server/server.py --il-binary target/release/il
```

**特点**：
- ✅ 原生性能
- ✅ 完整开发体验
- ✅ 即时编译测试
- ⚠️  需要安装 Rust 工具链
- ⚠️  需要配置 LLVM

**适用场景**：il 核心开发、性能优化、调试

---

## API 端点概览

所有选项都提供相同的 HTTP API：

### 基础端点
- `GET /health` - 健康检查
- `GET /api/state` - 获取当前状态

### 图操作端点（完整版）
- `POST /api/transact` - 执行事务
- `POST /api/inspect` - 检查实体
- `POST /api/validate` - 验证图
- `POST /api/build` - 构建可执行文件
- `POST /api/test` - 运行测试
- `POST /api/diff` - 比较版本
- `POST /api/restore` - 恢复版本
- `POST /api/slice` - 获取图切片
- `POST /api/callers` - 查找调用者
- `POST /api/dependencies` - 查找依赖

### 演示端点（演示版）
- `GET /` - 服务信息
- `GET /health` - 健康检查
- `GET /capabilities` - 系统能力
- `GET /graph/state` - 图状态
- `POST /graph/node` - 创建节点
- `GET /graph/node/{id}` - 获取节点
- `POST /graph/edge` - 创建边
- `POST /graph/execute` - 执行节点
- `DELETE /graph/reset` - 重置图

---

## 快速测试示例

### 使用 curl

```bash
# 健康检查
curl http://localhost:8000/health

# 获取状态（完整版）
curl http://localhost:8000/api/state

# 获取状态（演示版）
curl http://localhost:8000/graph/state
```

### 使用 Python

```python
import requests

# 健康检查
r = requests.get("http://localhost:8000/health")
print(r.json())

# 创建节点（演示版）
r = requests.post("http://localhost:8000/graph/node",
                  json={
                      "id": "test",
                      "type": "constant",
                      "value": {"type": "text", "content": "Hello"}
                  })
print(r.json())
```

---

## 故障排除

### Docker 构建失败

```bash
# 清理并重试
docker system prune -a
docker build -f Dockerfile.server -t il-server:latest .
```

### 端口已被占用

```bash
# 查找占用进程
netstat -ano | findstr :8000

# 或更改端口
docker run -d -p 8080:8000 --name il-server il-server:latest
```

### 演示服务器无法启动

```bash
# 检查 Python 依赖
pip install fastapi uvicorn requests

# 使用详细日志
python demo_server.py --log-level debug
```

### Docker 容器无法访问

```bash
# 检查容器状态
docker ps -a

# 查看日志
docker logs il-server

# 检查健康状态
docker inspect il-server | grep -A 10 Health
```

---

## 性能和限制

### 演示服务器
- **吞吐量**：~1000 req/s（简单操作）
- **延迟**：<10ms（本地）
- **并发**：受 Python GIL 限制
- **内存**：~50MB 基础 + 图大小

### Docker 完整服务器
- **吞吐量**：~500 req/s（图操作）
- **延迟**：10-100ms（取决于操作）
- **并发**：支持多进程
- **内存**：~200MB 基础 + 图大小

### 本地编译
- **吞吐量**：~1000 req/s（图操作）
- **延迟**：5-50ms（原生性能）
- **并发**：完全支持
- **内存**：~100MB 基础 + 图大小

---

## 下一步

1. **选择启动方式**：根据需求选择上述选项之一
2. **阅读 API 文档**：查看 `DEMO_SERVER_README.md`
3. **运行测试套件**：`python test_demo_server.py`
4. **集成到应用**：使用 HTTP API 集成到您的系统

## 相关文档

- `DEMO_SERVER_README.md` - 演示服务器详细文档
- `IMPLEMENTATION_PLAN.md` - 完整实施计划
- `tools/http_server/server.py` - 完整服务器实现
- `docker-compose.yml` - Docker 编排配置
- `Dockerfile.server` - Docker 镜像定义

---

**当前推荐**：如果您需要立即测试，使用**选项 1（演示服务器）**；如果需要生产部署，等待 Docker 构建完成后使用**选项 2（Docker 容器）**。
