@id("demo") module demo visibility public imports [core,alloc,io,time,net,json,http,tracing] {
 @id("demo.listen") capability listener:Listen scope "127.0.0.1:8080";
 @id("demo.clock") capability clock:ClockRead scope null;
 @id("demo.health") fn health(request:http.EmptyRequest)->http.ResponseResult effects [alloc] {
  let result:http.BytesResult=runtime.string_to_bytes("ok");match result{Ok(body)=>{return Ok(http.Response::Response(200,"text/plain; charset=utf-8",body));}Err(error)=>{return Err(http.HttpError::Io(error));}}
 }
 @id("demo.hello") fn hello(request:http.PathRequest)->http.ResponseResult effects [alloc] {
  match request{Path(name)=>{
   let greeting:alloc.StringResult=runtime.string_concat("Hello, ",name);match greeting{Err(error)=>{return Err(http.HttpError::Handler());}Ok(message)=>{
    let encoded:json.BytesResult=json.message(message);match encoded{Err(error)=>{return Err(http.HttpError::Io(error));}Ok(body)=>{return Ok(http.Response::Response(200,"application/json",body));}}
   }}
  }}
 }
 @id("demo.dispatch") fn dispatch(method:Bytes,path:Bytes)->http.ResponseResult effects [alloc] contracts [demo.server];
 @id("demo.server") contract subject demo.dispatch predicates [{"kind":"http_server","entry":"demo.serve","bind":"127.0.0.1:8080","capability":"demo.listen","routes":[{"entity_id":"demo.route.health","method":"GET","path":"/health","handler":"demo.health","parameters":[]},{"entity_id":"demo.route.hello","method":"GET","path":"/hello/{name}","handler":"demo.hello","parameters":[{"name":"name","type_ref":"String","source":"path","max_utf8_bytes":128}]}]}];
 @id("demo.response") fn response(parsed:http.ParseResult)->http.Response effects [alloc,process] {
  match parsed{Err(error)=>{tracing.event("E_HTTP_REQUEST_FAILED");return http.error_response(error);}Ok(request)=>{match request{Parsed(method,path)=>{let result:http.ResponseResult=dispatch(method,path);match result{Ok(value)=>{return value;}Err(error)=>{return http.error_response(error);}}}}}}
 }
 @id("demo.connection") fn connection(stream:net.Stream)->I32 effects [net,alloc,clock,process] capabilities [demo.clock] {
  let accepted:U64=runtime.net_opened_at(stream);let deadline:core.Deadline=time.deadline_after_ms(accepted,5000);
  let received:http.Read=http.read_request(stream,deadline);match received{Read(connection,parsed)=>{
   let answer:http.Response=response(parsed);let started:U64=runtime.clock_now[demo.clock]();let write_deadline:core.Deadline=time.deadline_after_ms(started,5000);
   let sent:http.UnitResult=http.write_response(connection,answer,write_deadline);match sent{Ok(unit)=>{return 0;}Err(error)=>{match error{Timeout=>{tracing.event("E_HTTP_WRITE_TIMEOUT");}Read=>{tracing.event("E_HTTP_PEER_DISCONNECT");}Write=>{tracing.event("E_HTTP_PEER_DISCONNECT");}Closed=>{tracing.event("E_HTTP_PEER_DISCONNECT");}PermissionDenied=>{tracing.event("E_HTTP_WRITE_FAILED");}InvalidData=>{tracing.event("E_HTTP_WRITE_FAILED");}Other=>{tracing.event("E_HTTP_WRITE_FAILED");}}return 1;}}
  }}
 }
 @id("demo.serve") fn serve(maximum:Usize)->I32 effects [net,alloc,clock,process] capabilities [demo.listen,demo.clock] {
  let opened:net.ListenerResult=runtime.net_listen[demo.listen]();match opened{Err(error)=>{tracing.event("E_HTTP_LISTEN_FAILED");return 1;}Ok(listener)=>{
   let mut count:Usize=0;while count<maximum {
    let accepted:net.StreamResult=runtime.net_accept(listener,core.Deadline::Infinite());match accepted{Err(error)=>{tracing.event("E_HTTP_ACCEPT_FAILED");return 1;}Ok(stream)=>{connection(stream);}}
    count=count+1;
   }
   return 0;
  }}
 }
 @id("demo.serve_once") fn serve_once()->I32 effects [net,alloc,clock,process] capabilities [demo.listen,demo.clock] {return serve(1);}
 @id("main") fn main()->I32 effects [net,alloc,clock,process] capabilities [demo.listen,demo.clock] {return serve(18446744073709551615);}
}
