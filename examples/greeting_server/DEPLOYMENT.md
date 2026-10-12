# Greeting Server - 部署指南

## 🎯 快速开始（推荐）

### 一键启动

在项目根目录运行：

```powershell
cd examples/greeting_server
powershell -ExecutionPolicy Bypass -File quick_start.ps1
```

这将：
1. ✅ 在 Docker 中构建 il CLI
2. ✅ 构建 greeting_server 可执行文件
3. ✅ 启动服务器
4. ✅ 自动测试

## 📋 前置条件

### 必需
- **Docker Desktop** - 已安装并运行
- **PowerShell** - Windows 自带

### 可选
- **WSL2** - 用于原生 Linux 环境
- **curl** - 用于测试（容器内自带）

## 🚀 部署方案

### 方案 1：Docker 容器（推荐）⭐

最简单且官方支持的方式。

#### 自动化部署

```powershell
# 从项目根目录
cd examples/greeting_server
powershell -ExecutionPolicy Bypass -File quick_start.ps1
```

#### 手动部署

```powershell
# 1. 构建 il CLI
./tools/dev.ps1 build

# 2. 进入容器
./tools/dev.ps1 shell

# 3. 在容器内构建服务器
cd examples/greeting_server
python3 build_and_run.py build

# 4. 运行服务器
python3 build_and_run.py run --host 0.0.0.0
```

#### 测试

```powershell
# 在另一个终端
docker exec -it il-dev curl http://localhost:8080/greeting

# 或使用测试套件
docker exec -it il-dev bash
cd examples/greeting_server
python3 test_client.py
```

### 方案 2：WSL2 环境

如果安装了 WSL2，可以直接运行：

```bash
# 在 WSL2 终端
cd /mnt/d/me/project/il/examples/greeting_server

# 构建和运行
python3 build_and_run.py

# 测试（从 Windows）
curl http://localhost:8080/greeting
```

### 方案 3：后台服务

在容器中作为后台服务运行：

```powershell
# 启动容器（如果未运行）
./tools/dev.ps1 shell

# 在容器内后台运行
cd examples/greeting_server
nohup python3 build_and_run.py run --host 0.0.0.0 > server.log 2>&1 &

# 查看日志
tail -f server.log

# 停止服务
pkill -f greeting_server
```

## 🔧 配置

### 修改监听地址和端口

**方法 1：命令行参数**
```bash
python3 build_and_run.py run --host 0.0.0.0 --port 9090
```

**方法 2：修改 policy.json**
```json
{
  "schema_version": "1.0.0",
  "grants": [
    {
      "entity_id": "greeting.listen",
      "kind": "Listen",
      "scope": "0.0.0.0:9090"  // 修改这里
    }
  ]
}
```

**方法 3：修改 main.il**
```il
@id("greeting.listen") 
capability listener:Listen 
  scope "0.0.0.0:9090";  // 修改这里
```

### 添加新路由

编辑 `main.il`，添加 contract：

```il
@id("greeting.status")
contract status {
  method = "GET";
  path = "/status";
  
  @implementation
  fn handle(req: HttpRequest) -> HttpResponse {
    HttpResponse {
      status: 200,
      headers: [("Content-Type", "application/json")],
      body: b"{\"status\":\"ok\"}"
    }
  }
}
```

重新构建：
```bash
python3 build_and_run.py build
```

## 🧪 测试

### 基本测试

```bash
# 健康检查
curl http://localhost:8080/greeting

# 预期响应
hi, this is a server based il
```

### 完整测试套件

```bash
python3 test_client.py
```

测试包括：
- ✅ GET /greeting - 正常响应
- ✅ GET /invalid - 404 错误
- ✅ POST /greeting - 方法不允许
- ✅ 性能测试 - 1000 请求

### 手动测试

```bash
# 详细响应
curl -v http://localhost:8080/greeting

# 测试 404
curl -v http://localhost:8080/notfound

# 测试错误方法
curl -v -X POST http://localhost:8080/greeting
```

## 📊 监控和日志

### 查看服务器日志

```bash
# 如果使用后台运行
tail -f server.log

# 如果前台运行，日志直接输出到 stderr
```

### 检查进程

```bash
# 在容器内
ps aux | grep greeting_server

# 检查端口
netstat -tlnp | grep 8080
```

### 性能监控

```bash
# 使用 test_client.py 的基准测试
python3 test_client.py

# 或使用 ab (Apache Bench)
ab -n 1000 -c 10 http://localhost:8080/greeting
```

## 🐛 故障排查

### 构建失败

**问题**：`il executable not found`

**解决**：
```powershell
# 构建 il CLI
./tools/dev.ps1 build

# 验证
ls target/debug/il
```

**问题**：`Package not found`

**解决**：
```bash
# 检查包是否存在
ls packages/*/lib.il

# 应该看到：core, alloc, io, time, net, json, http, tracing
```

### 运行失败

**问题**：`Address already in use`

**解决**：
```bash
# 查找占用端口的进程
lsof -i :8080  # Linux/Mac
netstat -ano | findstr :8080  # Windows

# 杀死进程或使用不同端口
python3 build_and_run.py run --port 9090
```

**问题**：`Permission denied`

**解决**：
```bash
# 使用非特权端口（>1024）
python3 build_and_run.py run --port 8080

# 或添加执行权限
chmod +x build/greeting_server
```

### 连接失败

**问题**：从 Windows 无法连接到容器

**解决**：
```powershell
# 1. 确保使用 0.0.0.0 而不是 127.0.0.1
python3 build_and_run.py run --host 0.0.0.0

# 2. 检查容器 IP
docker inspect il-dev | findstr IPAddress

# 3. 使用容器 IP 测试
curl http://<container-ip>:8080/greeting

# 4. 或配置端口映射（修改 dev.ps1）
docker run -p 8080:8080 ...
```

### Docker 问题

**问题**：`Cannot connect to Docker daemon`

**解决**：
```powershell
# 启动 Docker Desktop
# 等待启动完成，然后重试
```

**问题**：`Container not found`

**解决**：
```powershell
# 启动容器
./tools/dev.ps1 shell

# 或直接运行命令
./tools/dev.ps1 il state
```

## 📈 生产部署

### 性能优化

1. **使用 release 构建**
```powershell
# 修改 dev.ps1 使用 release 模式
./tools/dev.ps1 build --release
```

2. **调整并发处理**
```il
// 在 main.il 中增加并发限制
// (当前是单线程同步处理)
```

3. **启用日志缓冲**
```il
// 批量写入日志而不是每次请求
```

### 安全加固

1. **限制 Capability scope**
```json
{
  "scope": "127.0.0.1:8080"  // 只监听本地
}
```

2. **添加请求验证**
```il
// 在 contract handler 中验证输入
```

3. **设置资源限制**
```bash
# Docker 资源限制
docker run --memory=512m --cpus=1 ...
```

### 高可用部署

1. **负载均衡**
```nginx
upstream il_backend {
    server container1:8080;
    server container2:8080;
    server container3:8080;
}
```

2. **健康检查**
```bash
# 添加 /health 端点
# 使用 Docker healthcheck
```

3. **自动重启**
```bash
# 使用 systemd 或 Docker restart policy
docker run --restart=unless-stopped ...
```

## 📚 参考资料

- [README.md](./README.md) - 功能特性和架构
- [PLAN.md](./PLAN.md) - 完整实现计划
- [main.il](./main.il) - 服务器源码
- [policy.json](./policy.json) - 运行时策略
- [il 项目文档](../../README.md)

## 🆘 获取帮助

### 命令帮助

```bash
python3 build_and_run.py --help
python3 test_client.py --help
```

### 常见问题

1. **Q: 可以在 Windows 上原生运行吗？**
   
   A: 目前不支持。il 运行时需要 Linux 环境。使用 Docker 或 WSL2。

2. **Q: 如何添加 HTTPS 支持？**
   
   A: 在前面放置反向代理（nginx/caddy），或等待 TLS capability 实现。

3. **Q: 性能如何？**
   
   A: 原生编译到机器码，无 GC 开销。简单测试约 1000+ req/s（单核）。

4. **Q: 可以连接数据库吗？**
   
   A: 需要添加数据库 capability 和相应的包支持。

## ✅ 验收标准

部署成功的标志：

- [ ] Docker 容器正常运行
- [ ] 服务器监听 8080 端口
- [ ] `curl http://localhost:8080/greeting` 返回正确响应
- [ ] `python3 test_client.py` 所有测试通过
- [ ] 日志正常输出
- [ ] 可以优雅关闭（Ctrl+C）

## 🎉 下一步

1. **添加更多路由** - 参考 README.md
2. **集成数据库** - 等待 DB capability
3. **添加认证** - 实现 Auth capability
4. **WebSocket 支持** - 扩展 HTTP 包
5. **自定义中间件** - 请求/响应拦截器

---

**祝使用愉快！** 🚀

如有问题，请查看 [PLAN.md](./PLAN.md) 获取详细技术信息。
