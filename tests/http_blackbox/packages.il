@id("acceptance") module acceptance visibility public imports [core,http,json,test,time] {
 @id("acceptance.status") fn status(raw:String)->U16 effects [alloc] {
  let bytes:http.BytesResult=runtime.string_to_bytes(raw);match bytes{Err(error)=>{return 599;}Ok(value)=>{
   let parsed:http.ParseResult=http.parse(value);match parsed{Ok(request)=>{return 200;}Err(error)=>{match error{Protocol(code)=>{return code;}Io(failure)=>{return 598;}Handler=>{return 597;}}}}
  }}
 }
 @id("main") fn main()->I32 effects [alloc] {
  if status("GET /health HTTP/1.1\r\nHost: x\r\n\r\n")!=cast<U16>(200){return 1;}
  if status("GET /health HTTP/1.1\r\n\r\n")!=cast<U16>(400){return 2;}
  if status("GET /hello/%FF HTTP/1.1\r\nHost: x\r\n\r\n")!=cast<U16>(400){return 3;}
  if status("GET /health HTTP/1.0\r\nHost: x\r\n\r\n")!=cast<U16>(505){return 4;}
  if status("GET /health HTTP/1.1\r\nHost: x\r\nContent-Length: 1\r\n\r\n")!=cast<U16>(413){return 5;}
  let encoded:json.BytesResult=json.quote("中\"\\\n");match encoded{Err(error)=>{return 6;}Ok(bytes)=>{
   let decoded:http.StringResult=runtime.string_from_utf8(bytes);match decoded{Err(error)=>{return 7;}Ok(text)=>{if !runtime.string_equal(text,"\"中\\\"\\\\\\u000a\""){return 8;}}}
  }}
  let result:test.Result=test.assert_i64(42,42);match result{Err(error)=>{return 9;}Ok(unit)=>{}}
  let rejected:test.Result=test.assert_true(false);match rejected{Ok(unit)=>{return 10;}Err(error)=>{}}
  if !time.expired(time.deadline_after_ms(100,1),1000100){return 11;}
  return 0;
 }
}
