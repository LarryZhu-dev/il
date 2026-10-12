# il HTTP 演示服务器

这是一个功能齐全的 il HTTP 服务器演示版本，展示了 il 的核心图操作 API。

## 快速开始

### 启动服务器

```bash
python demo_server.py
```

服务器将在 `http://127.0.0.1:8000` 上运行。

### 运行测试

```bash
python test_demo_server.py
```

这将运行完整的测试套件，验证所有端点功能。

## API 端点

### 基础端点

#### GET /
获取服务信息

**响应示例：**
```json
{
  "service": "il",
  "version": "0.1.0-demo",
  "mode": "demo",
  "description": "Demo server using test fixtures"
}
```

#### GET /health
健康检查

**响应示例：**
```json
{
  "status": "healthy",
  "mode": "demo",
  "message": "Demo server running with test fixtures"
}
```

#### GET /capabilities
获取系统能力

**响应示例：**
```json
{
  "http": {
    "enabled": true,
    "version": "1.1"
  },
  "graph": {
    "enabled": true,
    "max_nodes": 1000
  },
  "persistence": {
    "enabled": false
  }
}
```

### 图操作端点

#### GET /graph/state
获取当前图状态

**响应示例：**
```json
{
  "nodes": {
    "greeting": {
      "type": "constant",
      "value": {
        "type": "text",
        "content": "Hello, World!"
      }
    }
  },
  "edges": []
}
```

#### POST /graph/node
创建新节点

**请求体：**
```json
{
  "id": "user_name",
  "type": "constant",
  "value": {
    "type": "text",
    "content": "Alice"
  }
}
```

**响应示例：**
```json
{
  "id": "user_name",
  "status": "created"
}
```

#### GET /graph/node/{node_id}
获取特定节点

**响应示例：**
```json
{
  "id": "user_name",
  "type": "constant",
  "value": {
    "type": "text",
    "content": "Alice"
  }
}
```

#### POST /graph/edge
创建新边

**请求体：**
```json
{
  "source": "greeting",
  "target": "user_name",
  "label": "uses"
}
```

**响应示例：**
```json
{
  "status": "created",
  "edge": {
    "source": "greeting",
    "target": "user_name",
    "label": "uses"
  }
}
```

#### POST /graph/execute
执行图

**请求体：**
```json
{
  "node_id": "greeting"
}
```

**响应示例：**
```json
{
  "status": "completed",
  "node_id": "greeting",
  "result": {
    "type": "text",
    "content": "Hello, World!"
  }
}
```

#### DELETE /graph/reset
重置图到初始状态

**响应示例：**
```json
{
  "status": "reset",
  "message": "Graph reset to initial state"
}
```

## 使用示例

### 使用 curl

```bash
# 检查健康状态
curl http://127.0.0.1:8000/health

# 获取图状态
curl http://127.0.0.1:8000/graph/state

# 创建节点
curl -X POST http://127.0.0.1:8000/graph/node \
  -H "Content-Type: application/json" \
  -d '{"id":"my_node","type":"constant","value":{"type":"text","content":"Hello"}}'

# 执行节点
curl -X POST http://127.0.0.1:8000/graph/execute \
  -H "Content-Type: application/json" \
  -d '{"node_id":"greeting"}'
```

### 使用 Python

```python
import requests

# 检查健康状态
r = requests.get("http://127.0.0.1:8000/health")
print(r.json())

# 创建节点
r = requests.post("http://127.0.0.1:8000/graph/node",
                  json={
                      "id": "my_node",
                      "type": "constant",
                      "value": {"type": "text", "content": "Hello"}
                  })
print(r.json())

# 执行图
r = requests.post("http://127.0.0.1:8000/graph/execute",
                  json={"node_id": "my_node"})
print(r.json())
```

## 架构说明

### 演示模式

当前版本是演示模式，使用内存中的模拟状态而不是实际的 il 二进制文件。这允许：

- 无需编译即可测试 HTTP API
- 快速原型开发和集成测试
- 理解 il 的核心概念和数据模型

### 完整版本

完整的 il 服务器（见 `tools/http_server/server.py`）将：

- 使用真实的 il 二进制文件执行图操作
- 支持持久化存储
- 提供完整的图执行语义
- 包含更多高级功能（HTTP 能力、事务等）

## 文件说明

- `demo_server.py` - 演示服务器实现
- `test_demo_server.py` - 完整的测试套件
- `start_server.py` - 生产服务器启动脚本（需要编译的 il 二进制）
- `tools/http_server/server.py` - 完整的服务器实现
- `tools/http_server/il_client.py` - il 二进制客户端封装

## 下一步

1. **编译 il 二进制文件**：
   ```bash
   cargo build --release
   ```

2. **运行完整服务器**：
   ```bash
   python start_server.py
   ```

3. **集成到您的应用**：使用上述 API 端点集成 il 到您的应用程序中

## 技术栈

- **FastAPI** - 现代、快速的 Python Web 框架
- **Uvicorn** - ASGI 服务器
- **il** - Intelligent Language 图执行引擎

## 支持

有关 il 的更多信息，请参阅：
- RFC 文档：`accepted_rfcs/`
- 测试示例：`tests/`
- 示例代码：`examples/`
