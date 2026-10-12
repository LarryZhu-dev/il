# 故障排查指南 - il Greeting Server

本指南帮助你解决常见的构建和运行问题。

## 🔍 诊断工具

### 1. 检查环境

运行以下命令检查所有前置条件：

```bash
# 检查 Docker
docker --version
docker ps

# 检查 il-bootstrap 镜像
docker images | grep il-bootstrap

# 检查 Python
python --version

# 检查当前目录
pwd
ls -la
```

### 2. 查看详细日志

如果构建失败，查看详细输出：

```bash
# 运行时显示所有输出
python build_final.py --build-only --work-dir ./debug_build 2>&1 | tee build.log
```

## 🐛 常见问题

### 问题 1: "Docker daemon not running"

**症状：**
```
Cannot connect to the Docker daemon at unix:///var/run/docker.sock
```

**解决方法：**
1. 启动 Docker Desktop
2. 等待 Docker 完全启动（托盘图标不再动画）
3. 重新运行构建命令

**验证：**
```bash
docker ps
# 应该显示容器列表（可能为空）
```

---

### 问题 2: "il-bootstrap:local image not found"

**症状：**
```
Unable to find image 'il-bootstrap:local' locally
```

**解决方法：**
1. 构建 il-bootstrap 镜像：

```bash
cd D:/me/project/il
docker build -t il-bootstrap:local -f bootstrap/Dockerfile .
```

2. 等待构建完成（首次约 10-20 分钟）

**验证：**
```bash
docker images | grep il-bootstrap
# 应该看到 il-bootstrap:local
```

---

### 问题 3: "Module not found: core"

**症状：**
```
ERROR: Package not found: D:\me\project\il\packages\core\lib.il
```

**原因：** 不在正确的目录中运行

**解决方法：**
```bash
# 确保在正确的目录
cd D:/me/project/il/examples/greeting_server

# 验证
ls main.il  # 应该存在
ls ../../packages/core/lib.il  # 应该存在
```

---

### 问题 4: "Port 8080 already in use"

**症状：**
```
Error starting userland proxy: listen tcp4 0.0.0.0:8080: bind: address already in use
```

**解决方法 A - 使用不同端口：**
```bash
python build_final.py --port 3000
```

**解决方法 B - 找到并停止占用端口的进程：**

Windows (PowerShell):
```powershell
# 查找占用端口的进程
netstat -ano | findstr :8080

# 停止进程（替换 <PID> 为实际进程 ID）
taskkill /PID <PID> /F
```

Linux/Mac:
```bash
# 查找并停止
lsof -ti:8080 | xargs kill -9
```

---

### 问题 5: "Transaction failed" 或 "Build failed"

**症状：**
```
ERROR: Transaction failed
ERROR: Build failed
```

**诊断步骤：**

1. **检查源代码语法：**
```bash
# 验证 main.il 是有效的 UTF-8
file examples/greeting_server/main.il
```

2. **查看详细错误：**
```bash
# 保留构建目录
python build_final.py --build-only --work-dir ./debug --keep

# 查看事务结果
cat debug/transact_result.json | python -m json.tool

# 查看构建结果
cat debug/build_result.json | python -m json.tool
```

3. **手动测试：**
```bash
# 进入容器调试
docker run -it --rm -v $(pwd)/debug:/work il-bootstrap:local bash

# 在容器内
cd /work
cat transact_result.json
cat build_result.json
```

---

### 问题 6: "Permission denied" (Linux/Mac)

**症状：**
```bash
bash: /work/build.sh: Permission denied
```

**解决方法：**
```bash
chmod +x build_final.py
chmod +x tools/docker_build_v2.py
```

---

### 问题 7: 构建很慢或卡住

**可能原因：**
1. 首次构建需要下载和编译大量依赖
2. Docker 资源限制太低
3. 磁盘空间不足

**解决方法：**

1. **增加 Docker 资源（Docker Desktop）：**
   - 打开 Docker Desktop
   - Settings → Resources
   - 增加 CPU 和内存（推荐 4+ CPU, 8GB+ RAM）

2. **检查磁盘空间：**
```bash
df -h  # Linux/Mac
Get-PSDrive  # Windows PowerShell
```

3. **清理 Docker 缓存：**
```bash
docker system prune -a
```

4. **查看实时进度：**
```bash
# 在另一个终端
docker ps  # 找到容器 ID
docker logs -f <container-id>
```

---

### 问题 8: "Executable not found"

**症状：**
```
[ERROR] Executable not found at build_test/store/...
```

**诊断：**
```bash
# 检查构建结果
cat build_test/build_result.json | python -m json.tool

# 列出生成的文件
find build_test -type f -name "*.elf"
```

**解决方法：**
重新运行完整构建：
```bash
rm -rf build_test
python build_final.py --build-only --work-dir ./build_test
```

---

### 问题 9: 服务器启动但无响应

**症状：**
```bash
curl http://localhost:8080/health
# curl: (7) Failed to connect to localhost port 8080
```

**诊断：**

1. **检查服务器是否真的在运行：**
```bash
docker ps
# 应该看到一个运行中的容器
```

2. **检查端口映射：**
```bash
docker ps --format "{{.Ports}}"
# 应该看到 0.0.0.0:8080->8080/tcp
```

3. **查看容器日志：**
```bash
docker logs <container-id>
```

4. **测试容器内部：**
```bash
docker exec -it <container-id> bash
curl http://localhost:8080/health
```

---

### 问题 10: 测试失败

**症状：**
```bash
python test_server.py
[FAIL] Health Check
```

**诊断：**

1. **确保服务器正在运行：**
```bash
# 在一个终端
python build_final.py

# 在另一个终端
python test_server.py --wait
```

2. **手动测试端点：**
```bash
# 测试健康检查
curl -v http://localhost:8080/health

# 测试问候
curl -v -X POST http://localhost:8080/greet \
  -H "Content-Type: application/json" \
  -d '{"name": "Test"}'
```

3. **检查防火墙：**
   - Windows: 允许 Python/Docker 通过防火墙
   - Linux: 检查 iptables/firewalld
   - Mac: 系统偏好设置 → 安全性

---

## 🔧 高级调试

### 进入容器调试

```bash
# 启动交互式容器
docker run -it --rm \
  -v $(pwd)/build_test:/work \
  -w /work \
  il-bootstrap:local bash

# 在容器内手动执行步骤
cd /workspace
cargo build --release --bin il

cd /work
git init --initial-branch=main repo
cd repo
# ... 手动执行构建步骤
```

### 查看生成的中间文件

```bash
# 保留构建目录
python build_final.py --build-only --work-dir ./debug --keep

# 查看结构
tree debug/  # 或 find debug/

# 关键文件：
# - debug/source.il          - 完整源代码
# - debug/build.sh           - 构建脚本
# - debug/transact_result.json - 事务结果
# - debug/build_result.json  - 构建结果
# - debug/repo/              - Git 仓库
# - debug/store/             - 工件存储
```

### 直接使用 Docker 运行

```bash
# 编译后手动运行
docker run -it --rm \
  -p 8080:8080 \
  -v $(pwd)/build_test:/work \
  -w /work \
  il-bootstrap:local bash

# 在容器内
./store/.../greeting.elf 4<policy.json
```

---

## 📊 性能调优

### 如果构建太慢

1. **使用本地缓存：**
   - 重复构建会使用 Docker 层缓存
   - 不要每次都 `docker system prune`

2. **并行构建：**
   - 编辑 Dockerfile，增加 `MAKEFLAGS=-j$(nproc)`

3. **使用预构建镜像：**
   - 考虑推送 il-bootstrap:local 到 Docker Hub
   - 在其他机器上直接 pull

### 如果运行时响应慢

1. **增加容器资源**
2. **检查日志级别**
3. **使用 release 构建（默认已启用）**

---

## 🆘 获取帮助

如果以上都不能解决问题：

1. **收集信息：**
```bash
# 系统信息
uname -a  # Linux/Mac
systeminfo  # Windows

# Docker 信息
docker version
docker info

# 构建日志
python build_final.py --build-only --work-dir ./logs 2>&1 | tee full.log
```

2. **创建最小重现：**
   - 使用全新的构建目录
   - 记录每个命令和输出
   - 注意任何错误消息

3. **报告问题：**
   - 在 GitHub 上开 issue
   - 附上日志和系统信息
   - 描述期望行为和实际行为

---

## ✅ 验证清单

在报告问题前，确认已尝试：

- [ ] Docker Desktop 正在运行
- [ ] il-bootstrap:local 镜像存在
- [ ] 在正确的目录中（examples/greeting_server）
- [ ] 端口 8080 未被占用
- [ ] 有足够的磁盘空间（>10GB）
- [ ] Docker 有足够的资源（4+ CPU, 8GB+ RAM）
- [ ] 已查看 build_result.json 中的错误详情
- [ ] 已尝试清理并重新构建
- [ ] 已阅读本故障排查指南

---

**提示：** 大多数问题都是环境配置问题。确保 Docker 正常运行，镜像已构建，并且在正确的目录中。
