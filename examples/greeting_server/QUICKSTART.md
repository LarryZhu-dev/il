# Quick Start Guide - il Greeting Server

这是一个完整的指南，帮助你在 5 分钟内运行一个 il HTTP 服务器。

## 前置要求

1. **Docker Desktop** - 必须安装并运行
   - Windows: https://www.docker.com/products/docker-desktop/
   - 安装后，确保 Docker 正在运行

2. **Python 3.7+** - 通常 Windows 已经安装

3. **Git** - 用于克隆仓库

## 第一步：检查环境

打开命令行（PowerShell 或 Git Bash），运行：

```bash
# 检查 Docker
docker --version

# 检查 Python
python --version

# 检查 Git
git --version
```

如果所有命令都成功，说明环境准备好了。

## 第二步：获取代码

```bash
# 进入项目目录
cd D:/me/project/il/examples/greeting_server

# 或者如果你克隆了仓库
git clone <repository-url>
cd il/examples/greeting_server
```

## 第三步：构建和运行

### 方法 1：一键运行（推荐）

```bash
python build_final.py
```

这个命令会：
1. 加载所有依赖包
2. 编译 il 程序（首次需要 1-2 分钟）
3. 创建安全策略
4. 启动服务器

### 方法 2：分步执行

```bash
# 只构建，不运行
python build_final.py --build-only --work-dir ./build

# 稍后运行
python build_final.py --work-dir ./build
```

### 方法 3：指定端口

```bash
python build_final.py --port 3000
```

## 第四步：测试服务器

服务器启动后，打开新的命令行窗口：

### 测试 1：健康检查

```bash
curl http://localhost:8080/health
```

期望输出：
```json
{
  "status": "ok",
  "timestamp": 1697040000
}
```

### 测试 2：问候接口

```bash
curl -X POST http://localhost:8080/greet ^
  -H "Content-Type: application/json" ^
  -d "{\"name\": \"World\"}"
```

期望输出：
```json
{
  "message": "Hello, World!",
  "timestamp": 1697040000
}
```

### 测试 3：运行完整测试套件

```bash
python test_server.py
```

期望看到所有测试通过。

## 第五步：停止服务器

在运行服务器的窗口中按 `Ctrl+C`。

## 常见问题

### Q: Docker 错误 "Cannot connect to Docker daemon"

**A:** Docker Desktop 没有运行。启动 Docker Desktop 并等待它完全启动。

### Q: 端口被占用

**A:** 使用不同的端口：

```bash
python build_final.py --port 3000
```

### Q: 构建很慢

**A:** 首次构建需要：
- 下载 Rust 工具链
- 编译 il CLI
- 编译 LLVM 后端

这是正常的。后续构建会快很多（因为 Docker 缓存）。

### Q: "Module not found" 错误

**A:** 确保在正确的目录：

```bash
# 应该看到这些文件
ls
# main.il  build_final.py  test_server.py  README.md
```

### Q: 想看构建的详细过程

**A:** 查看构建目录：

```bash
python build_final.py --build-only --work-dir ./mybuild --keep
# 构建产物保存在 ./mybuild
```

## 下一步

恭喜！你已经运行了第一个 il HTTP 服务器。

### 学习更多

1. **阅读源代码** - 查看 `main.il` 了解如何用 il 编写 HTTP 服务器
2. **修改端点** - 尝试添加新的 HTTP 路由
3. **查看文档** - 阅读 `README.md` 了解完整功能
4. **运行测试** - 学习如何测试 il 程序

### 实验想法

1. **添加新端点**
   - 例如：`GET /time` 返回当前时间
   - 例如：`POST /calculate` 做一些计算

2. **修改响应格式**
   - 添加更多字段
   - 改变消息格式

3. **添加错误处理**
   - 处理无效的 JSON
   - 添加输入验证

4. **性能测试**
   - 使用 `ab` 或 `wrk` 进行负载测试
   - 看看能处理多少请求/秒

## 获取帮助

- **文档**: 查看 `README.md`
- **示例**: 查看 `examples/` 目录
- **问题**: 在 GitHub 上开 issue

## 总结

你已经学会了：

✅ 设置 il 开发环境  
✅ 构建 il 程序  
✅ 运行 HTTP 服务器  
✅ 测试 API 端点  

现在你可以开始构建自己的 il 应用了！
