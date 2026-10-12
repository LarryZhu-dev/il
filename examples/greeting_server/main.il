@id("greeting") module greeting visibility public imports [core,alloc,net,http,time,json,tracing] {
 @id("greeting.listen") capability listener:Listen scope "0.0.0.0:3000";
 @id("greeting.clock") capability clock:ClockRead scope null;

 @id("greeting.hello") fn hello()->http.ResponseResult effects [alloc,process] {
  let now:U64=runtime.clock_now[greeting.clock]();
  let message_obj:json.Value=json.object([("message",json.string("Hello from il!")),("timestamp",json.number(now))])?;
  let serialized:Bytes=json.serialize(message_obj)?;
  return Ok(http.Response::Response(200,"application/json",serialized));
 }

 @id("greeting.health") fn health()->http.ResponseResult effects [alloc] {
  let health_obj:json.Value=json.object([("status",json.string("healthy")),("service",json.string("greeting_server"))])?;
  let serialized:Bytes=json.serialize(health_obj)?;
  return Ok(http.Response::Response(200,"application/json",serialized));
 }

 @id("greeting.not_found") fn not_found(path:String)->http.ResponseResult effects [alloc] {
  let error_obj:json.Value=json.object([("error",json.string("Not Found")),("path",json.string(path))])?;
  let serialized:Bytes=json.serialize(error_obj)?;
  return Ok(http.Response::Response(404,"application/json",serialized));
 }

 @id("greeting.dispatch") fn dispatch(method:http.Method,path:String)->http.ResponseResult effects [alloc,process] {
  if path=="/hello"{return hello();}
  if path=="/health"{return health();}
  return not_found(path);
 }

 @id("greeting.connection") fn connection(stream:net.Stream)->Int effects [alloc,net,process] {
  let accepted:U64=runtime.net_opened_at(stream);let read_deadline:core.Deadline=time.deadline_after_ms(accepted,5000);
  let parsed:http.ParseResult=http.parse_request(stream,read_deadline);
  match parsed{Err(error)=>{tracing.event("E_HTTP_REQUEST_FAILED");return 1;}Ok(request)=>{match request{Parsed(method,path)=>{
   let result:http.ResponseResult=dispatch(method,path);let answer:http.Response=match result{Ok(value)=>{value}Err(error)=>{http.error_response(error)}};
   let started:U64=runtime.clock_now[greeting.clock]();let write_deadline:core.Deadline=time.deadline_after_ms(started,5000);
   let sent:http.UnitResult=http.write_response(stream,answer,write_deadline);match sent{Ok(unit)=>{return 0;}Err(error)=>{match error{Timeout=>{tracing.event("E_HTTP_WRITE_TIMEOUT");}Read=>{tracing.event("E_HTTP_PEER_DISCONNECT");}Write=>{tracing.event("E_HTTP_PEER_DISCONNECT");}Closed=>{tracing.event("E_HTTP_PEER_DISCONNECT");}PermissionDenied=>{tracing.event("E_HTTP_WRITE_FAILED");}InvalidData=>{tracing.event("E_HTTP_WRITE_FAILED");}Other=>{tracing.event("E_HTTP_WRITE_FAILED");}}return 1;}}
  }}}}
 }

 @id("greeting.main") pub fn main()->Int effects [alloc,net,process] {
  let opened:net.ListenerResult=runtime.net_listen[greeting.listen]();match opened{Err(error)=>{tracing.event("E_HTTP_LISTEN_FAILED");return 1;}Ok(listener)=>{loop{
    let accepted:net.StreamResult=runtime.net_accept(listener,core.Deadline::Infinite());match accepted{Err(error)=>{tracing.event("E_HTTP_ACCEPT_FAILED");return 1;}Ok(stream)=>{connection(stream);}}}}}
 }
}
