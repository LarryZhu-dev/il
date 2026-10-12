// il 语言 HTTP 服务器示例
// 展示如何使用 il 编写一个完整的 HTTP API 服务器

@id("api_server") module api_server visibility public imports [core,alloc,net,http,time,json,tracing] {
  // 声明网络监听能力
  @id("api_server.listen") capability listener:Listen scope "0.0.0.0:8080";

  // 声明时钟读取能力
  @id("api_server.clock") capability clock:ClockRead scope null;

  // ==================== API 端点处理器 ====================

  // GET /hello - 返回问候消息
  @id("api_server.hello") fn hello() -> http.ResponseResult effects [alloc, process] {
    let now: U64 = runtime.clock_now[api_server.clock]();
    let message_obj: json.Value = json.object([
      ("message", json.string("你好！欢迎使用 il HTTP 服务器")),
      ("timestamp", json.number(now)),
      ("language", json.string("il"))
    ])?;
    let serialized: Bytes = json.serialize(message_obj)?;
    return Ok(http.Response::Response(200, "application/json", serialized));
  }

  // GET /health - 健康检查端点
  @id("api_server.health") fn health() -> http.ResponseResult effects [alloc] {
    let health_obj: json.Value = json.object([
      ("status", json.string("healthy")),
      ("service", json.string("il_api_server")),
      ("version", json.string("1.0.0"))
    ])?;
    let serialized: Bytes = json.serialize(health_obj)?;
    return Ok(http.Response::Response(200, "application/json", serialized));
  }

  // GET /time - 返回当前服务器时间
  @id("api_server.time") fn get_time() -> http.ResponseResult effects [alloc, process] {
    let now: U64 = runtime.clock_now[api_server.clock]();
    let time_obj: json.Value = json.object([
      ("current_time", json.number(now)),
      ("unit", json.string("nanoseconds"))
    ])?;
    let serialized: Bytes = json.serialize(time_obj)?;
    return Ok(http.Response::Response(200, "application/json", serialized));
  }

  // GET /capabilities - 返回服务能力信息
  @id("api_server.capabilities") fn capabilities() -> http.ResponseResult effects [alloc] {
    let caps_obj: json.Value = json.object([
      ("capabilities", json.array([
        json.string("http_server"),
        json.string("json_api"),
        json.string("time_service")
      ])),
      ("endpoints", json.array([
        json.string("/hello"),
        json.string("/health"),
        json.string("/time"),
        json.string("/capabilities")
      ]))
    ])?;
    let serialized: Bytes = json.serialize(caps_obj)?;
    return Ok(http.Response::Response(200, "application/json", serialized));
  }

  // 404 处理 - 路径未找到
  @id("api_server.not_found") fn not_found(path: String) -> http.ResponseResult effects [alloc] {
    let error_obj: json.Value = json.object([
      ("error", json.string("Not Found")),
      ("path", json.string(path)),
      ("message", json.string("请求的端点不存在"))
    ])?;
    let serialized: Bytes = json.serialize(error_obj)?;
    return Ok(http.Response::Response(404, "application/json", serialized));
  }

  // ==================== 请求路由 ====================

  // 根据路径分发请求到相应的处理器
  @id("api_server.dispatch") fn dispatch(method: http.Method, path: String) -> http.ResponseResult effects [alloc, process] {
    // 只处理 GET 请求
    if path == "/hello" { return hello(); }
    if path == "/health" { return health(); }
    if path == "/time" { return get_time(); }
    if path == "/capabilities" { return capabilities(); }

    // 未找到路径
    return not_found(path);
  }

  // ==================== 连接处理 ====================

  // 处理单个 HTTP 连接
  @id("api_server.connection") fn connection(stream: net.Stream) -> Int effects [alloc, net, process] {
    // 获取连接建立时间
    let accepted: U64 = runtime.net_opened_at(stream);

    // 设置读取超时：5 秒
    let read_deadline: core.Deadline = time.deadline_after_ms(accepted, 5000);

    // 解析 HTTP 请求
    let parsed: http.ParseResult = http.parse_request(stream, read_deadline);

    match parsed {
      Err(error) => {
        tracing.event("E_HTTP_REQUEST_FAILED");
        return 1;
      }
      Ok(request) => {
        match request {
          Parsed(method, path) => {
            // 调用路由器处理请求
            let result: http.ResponseResult = dispatch(method, path);

            // 处理结果或错误
            let answer: http.Response = match result {
              Ok(value) => { value }
              Err(error) => { http.error_response(error) }
            };

            // 设置写入超时：5 秒
            let started: U64 = runtime.clock_now[api_server.clock]();
            let write_deadline: core.Deadline = time.deadline_after_ms(started, 5000);

            // 发送 HTTP 响应
            let sent: http.UnitResult = http.write_response(stream, answer, write_deadline);

            match sent {
              Ok(unit) => {
                return 0; // 成功
              }
              Err(error) => {
                // 记录写入错误
                match error {
                  Timeout => { tracing.event("E_HTTP_WRITE_TIMEOUT"); }
                  Read => { tracing.event("E_HTTP_PEER_DISCONNECT"); }
                  Write => { tracing.event("E_HTTP_PEER_DISCONNECT"); }
                  Closed => { tracing.event("E_HTTP_PEER_DISCONNECT"); }
                  PermissionDenied => { tracing.event("E_HTTP_WRITE_FAILED"); }
                  InvalidData => { tracing.event("E_HTTP_WRITE_FAILED"); }
                  Other => { tracing.event("E_HTTP_WRITE_FAILED"); }
                }
                return 1;
              }
            }
          }
        }
      }
    }
  }

  // ==================== 主函数 ====================

  // 服务器主循环
  @id("api_server.main") pub fn main() -> Int effects [alloc, net, process] {
    // 开始监听指定端口
    let opened: net.ListenerResult = runtime.net_listen[api_server.listen]();

    match opened {
      Err(error) => {
        tracing.event("E_HTTP_LISTEN_FAILED");
        return 1;
      }
      Ok(listener) => {
        // 无限循环接受连接
        loop {
          let accepted: net.StreamResult = runtime.net_accept(listener, core.Deadline::Infinite());

          match accepted {
            Err(error) => {
              tracing.event("E_HTTP_ACCEPT_FAILED");
              return 1;
            }
            Ok(stream) => {
              // 处理连接（同步模式）
              connection(stream);
            }
          }
        }
      }
    }
  }
}
