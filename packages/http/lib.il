@id("http") module http visibility public imports [core,net] {
 @id("http.HttpError") type HttpError=sum {Protocol(U16),Io(core.IoError),Handler};
 @id("http.Request") type Request=sum {Parsed(Bytes,Bytes)};
 @id("http.EmptyRequest") type EmptyRequest=sum {Empty};
 @id("http.PathRequest") type PathRequest=sum {Path(String)};
 @id("http.ParseResult") type ParseResult=Result<Request,HttpError>;
 @id("http.Read") type Read=sum {Read(net.Stream,ParseResult)};
 @id("http.Response") type Response=sum {Response(U16,String,Bytes)};
 @id("http.ResponseResult") type ResponseResult=Result<Response,HttpError>;
 @id("http.PathMatch") type PathMatch=sum {Miss,Match(String)};
 @id("http.MatchResult") type MatchResult=Result<PathMatch,HttpError>;
 @id("http.BytesResult") type BytesResult=Result<Bytes,core.IoError>;
 @id("http.StringResult") type StringResult=Result<String,core.IoError>;
 @id("http.UnitResult") type UnitResult=Result<Unit,core.IoError>;
 @id("http.Scan") type Scan=sum {Parsed(Bytes,Bytes),Rejected(U16)};
 @id("http.StatusResult") type StatusResult=Result<U16,core.IoError>;
 @id("http.ScanResult") type ScanResult=Result<Scan,core.IoError>;
 @id("http.PathScan") type PathScan=sum {Decoded(Bytes),Rejected(U16)};
 @id("http.PathScanResult") type PathScanResult=Result<PathScan,core.IoError>;
 @id("http.lower_ascii") fn lower_ascii(byte:U8)->U8 {if byte>=65 {if byte<=90{return byte+32;}}return byte;}
 @id("http.token") fn token(byte:U8)->Bool {
  if byte>=65{if byte<=90{return true;}}
  if byte>=97{if byte<=122{return true;}}
  if byte>=48{if byte<=57{return true;}}
  if byte==33{return true;}if byte==35{return true;}if byte==36{return true;}if byte==37{return true;}if byte==38{return true;}if byte==39{return true;}if byte==42{return true;}if byte==43{return true;}if byte==45{return true;}if byte==46{return true;}if byte==94{return true;}if byte==95{return true;}if byte==96{return true;}if byte==124{return true;}if byte==126{return true;}return false;
 }
 @id("http.hex_value") fn hex_value(byte:U8)->U16 {
  if byte>=48{if byte<=57{return cast<U16>(byte-48);}}
  if byte>=65{if byte<=70{return cast<U16>(byte-55);}}
  if byte>=97{if byte<=102{return cast<U16>(byte-87);}}
  return 256;
 }
 @id("http.decode_path") fn decode_path(raw:Bytes)->PathScanResult effects [alloc] {
  let length:Usize=runtime.bytes_len(raw);let mut index:Usize=0;let mut output:Bytes=runtime.string_to_bytes("")?;
  while index<length {
   let mut byte:U8=runtime.bytes_get(raw,index);
   if byte==37 {
    if length-index<3{return Ok(PathScan::Rejected(400));}
    let high:U16=hex_value(runtime.bytes_get(raw,index+1));let low:U16=hex_value(runtime.bytes_get(raw,index+2));
    if high>15{return Ok(PathScan::Rejected(400));}if low>15{return Ok(PathScan::Rejected(400));}
    byte=cast<U8>(high*16+low);if byte==47{return Ok(PathScan::Rejected(400));}if byte==0{return Ok(PathScan::Rejected(400));}index=index+2;
   }
   if byte==0{return Ok(PathScan::Rejected(400));}
   let one:Bytes=runtime.bytes_from_u8(byte)?;output=runtime.bytes_concat(output,one)?;index=index+1;
  }
  let utf8:StringResult=runtime.string_from_utf8(output);
  match utf8 {Ok(text)=>{return Ok(PathScan::Decoded(output));}Err(error)=>{match error {InvalidData=>{return Ok(PathScan::Rejected(400));}Read=>{return Err(core.IoError::Read());}Write=>{return Err(core.IoError::Write());}Closed=>{return Err(core.IoError::Closed());}Timeout=>{return Err(core.IoError::Timeout());}PermissionDenied=>{return Err(core.IoError::PermissionDenied());}Other=>{return Err(core.IoError::Other());}}}}
 }
 @id("http.header_kind") fn header_kind(name:Bytes)->U8 {
  let length:Usize=runtime.bytes_len(name);
  if length==4 {
   let c0:U8=lower_ascii(runtime.bytes_get(name,0));if c0!=104{return 0;}
   let c1:U8=lower_ascii(runtime.bytes_get(name,1));if c1!=111{return 0;}
   let c2:U8=lower_ascii(runtime.bytes_get(name,2));if c2!=115{return 0;}
   let c3:U8=lower_ascii(runtime.bytes_get(name,3));if c3!=116{return 0;}
   return 1;
  }
  if length==14 {
   let c0:U8=lower_ascii(runtime.bytes_get(name,0));if c0!=99{return 0;}
   let c1:U8=lower_ascii(runtime.bytes_get(name,1));if c1!=111{return 0;}
   let c2:U8=lower_ascii(runtime.bytes_get(name,2));if c2!=110{return 0;}
   let c3:U8=lower_ascii(runtime.bytes_get(name,3));if c3!=116{return 0;}
   let c4:U8=lower_ascii(runtime.bytes_get(name,4));if c4!=101{return 0;}
   let c5:U8=lower_ascii(runtime.bytes_get(name,5));if c5!=110{return 0;}
   let c6:U8=lower_ascii(runtime.bytes_get(name,6));if c6!=116{return 0;}
   let c7:U8=lower_ascii(runtime.bytes_get(name,7));if c7!=45{return 0;}
   let c8:U8=lower_ascii(runtime.bytes_get(name,8));if c8!=108{return 0;}
   let c9:U8=lower_ascii(runtime.bytes_get(name,9));if c9!=101{return 0;}
   let c10:U8=lower_ascii(runtime.bytes_get(name,10));if c10!=110{return 0;}
   let c11:U8=lower_ascii(runtime.bytes_get(name,11));if c11!=103{return 0;}
   let c12:U8=lower_ascii(runtime.bytes_get(name,12));if c12!=116{return 0;}
   let c13:U8=lower_ascii(runtime.bytes_get(name,13));if c13!=104{return 0;}
   return 2;
  }
  if length==17 {
   let c0:U8=lower_ascii(runtime.bytes_get(name,0));if c0!=116{return 0;}
   let c1:U8=lower_ascii(runtime.bytes_get(name,1));if c1!=114{return 0;}
   let c2:U8=lower_ascii(runtime.bytes_get(name,2));if c2!=97{return 0;}
   let c3:U8=lower_ascii(runtime.bytes_get(name,3));if c3!=110{return 0;}
   let c4:U8=lower_ascii(runtime.bytes_get(name,4));if c4!=115{return 0;}
   let c5:U8=lower_ascii(runtime.bytes_get(name,5));if c5!=102{return 0;}
   let c6:U8=lower_ascii(runtime.bytes_get(name,6));if c6!=101{return 0;}
   let c7:U8=lower_ascii(runtime.bytes_get(name,7));if c7!=114{return 0;}
   let c8:U8=lower_ascii(runtime.bytes_get(name,8));if c8!=45{return 0;}
   let c9:U8=lower_ascii(runtime.bytes_get(name,9));if c9!=101{return 0;}
   let c10:U8=lower_ascii(runtime.bytes_get(name,10));if c10!=110{return 0;}
   let c11:U8=lower_ascii(runtime.bytes_get(name,11));if c11!=99{return 0;}
   let c12:U8=lower_ascii(runtime.bytes_get(name,12));if c12!=111{return 0;}
   let c13:U8=lower_ascii(runtime.bytes_get(name,13));if c13!=100{return 0;}
   let c14:U8=lower_ascii(runtime.bytes_get(name,14));if c14!=105{return 0;}
   let c15:U8=lower_ascii(runtime.bytes_get(name,15));if c15!=110{return 0;}
   let c16:U8=lower_ascii(runtime.bytes_get(name,16));if c16!=103{return 0;}
   return 3;
  }
  if length==7 {
   let c0:U8=lower_ascii(runtime.bytes_get(name,0));if c0!=117{return 0;}
   let c1:U8=lower_ascii(runtime.bytes_get(name,1));if c1!=112{return 0;}
   let c2:U8=lower_ascii(runtime.bytes_get(name,2));if c2!=103{return 0;}
   let c3:U8=lower_ascii(runtime.bytes_get(name,3));if c3!=114{return 0;}
   let c4:U8=lower_ascii(runtime.bytes_get(name,4));if c4!=97{return 0;}
   let c5:U8=lower_ascii(runtime.bytes_get(name,5));if c5!=100{return 0;}
   let c6:U8=lower_ascii(runtime.bytes_get(name,6));if c6!=101{return 0;}
   return 4;
  }
  return 0;
 }
 @id("http.scan_headers") fn scan_headers(raw:Bytes)->StatusResult effects [alloc] {
  let length:Usize=runtime.bytes_len(raw);let mut index:Usize=0;let headers_start:Usize=index;let mut host_count:Usize=0;let mut content_count:Usize=0;let mut body:Bool=false;let mut header_done:Bool=false;
  while index<length {
   let start:Usize=index;let mut end:Usize=start;
   while end<length {let byte:U8=runtime.bytes_get(raw,end);if byte==13{break;}if byte==10{return Ok(400);}end=end+1;if end-headers_start>16384{return Ok(431);}}
   if length-end<2{return Ok(400);}if runtime.bytes_get(raw,end+1)!=cast<U8>(10){return Ok(400);}
   if end+2-headers_start>16384{return Ok(431);}
   if end==start{header_done=true;break;}
   let mut colon:Usize=start;while colon<end {let byte:U8=runtime.bytes_get(raw,colon);if byte==58{break;}if !token(byte){return Ok(400);}colon=colon+1;}
   if colon==start{return Ok(400);}if colon==end{return Ok(400);}
   let name:Bytes=runtime.bytes_slice(raw,start,colon-start)?;let header:U8=header_kind(name);
   let mut value_start:Usize=colon+1;while value_start<end {let byte:U8=runtime.bytes_get(raw,value_start);if byte==32{value_start=value_start+1;}else if byte==9{value_start=value_start+1;}else{break;}}
   let mut value_end:Usize=end;while value_end>value_start {let byte:U8=runtime.bytes_get(raw,value_end-1);if byte==32{value_end=value_end-1;}else if byte==9{value_end=value_end-1;}else{break;}}
   let mut value_index:Usize=value_start;while value_index<value_end {let byte:U8=runtime.bytes_get(raw,value_index);if byte<32{if byte!=9{return Ok(400);}}if byte==127{return Ok(400);}value_index=value_index+1;}
   if header==1{host_count=host_count+1;if host_count>1{return Ok(400);}if value_start==value_end{return Ok(400);}}
   if header==2{content_count=content_count+1;if content_count>1{return Ok(400);}if value_start==value_end{return Ok(400);}let mut digit_index:Usize=value_start;while digit_index<value_end {let digit:U8=runtime.bytes_get(raw,digit_index);if digit<48{return Ok(400);}if digit>57{return Ok(400);}if digit!=48{body=true;}digit_index=digit_index+1;}}
   if header==3{return Ok(400);}if header==4{return Ok(400);}
   index=end+2;
  }
  if !header_done{return Ok(400);}if host_count!=1{return Ok(400);}if body{return Ok(413);}
  return Ok(0);
 }
 @id("http.check_version") fn check_version(version:Bytes)->StatusResult effects [alloc] {
  if runtime.bytes_len(version)!=cast<Usize>(8){return Ok(400);}
  let prefix:Bytes=runtime.bytes_slice(version,0,5)?;let expected_prefix:Bytes=runtime.string_to_bytes("HTTP/")?;
  if !runtime.bytes_equal(prefix,expected_prefix){return Ok(400);}drop(prefix);drop(expected_prefix);
  if runtime.bytes_get(version,6)!=cast<U8>(46){return Ok(400);}
  let major:U8=runtime.bytes_get(version,5);let minor:U8=runtime.bytes_get(version,7);
  if major<48{return Ok(400);}if major>57{return Ok(400);}if minor<48{return Ok(400);}if minor>57{return Ok(400);}
  let expected_version:Bytes=runtime.string_to_bytes("HTTP/1.1")?;
  if !runtime.bytes_equal(version,expected_version){return Ok(505);}
  return Ok(0);
 }
 @id("http.scan") fn scan(raw:Bytes)->ScanResult effects [alloc] {
  let length:Usize=runtime.bytes_len(raw);let mut line_end:Usize=0;
  while line_end<length {
   let byte:U8=runtime.bytes_get(raw,line_end);
   if byte==10{return Ok(Scan::Rejected(400));}
   if byte==13{break;}
   line_end=line_end+1;if line_end>4096{return Ok(Scan::Rejected(414));}
  }
  if line_end>=length{return Ok(Scan::Rejected(400));}
  if length-line_end<2{return Ok(Scan::Rejected(400));}
  if runtime.bytes_get(raw,line_end+1)!=cast<U8>(10){return Ok(Scan::Rejected(400));}
  let mut first_space:Usize=0;
  while first_space<line_end {let byte:U8=runtime.bytes_get(raw,first_space);if byte==32{break;}if !token(byte){return Ok(Scan::Rejected(400));}first_space=first_space+1;}
  if first_space==0{return Ok(Scan::Rejected(400));}if first_space>=line_end{return Ok(Scan::Rejected(400));}
  let target_start:Usize=first_space+1;let mut second_space:Usize=target_start;
  while second_space<line_end {let byte:U8=runtime.bytes_get(raw,second_space);if byte==32{break;}if byte<33{return Ok(Scan::Rejected(400));}if byte==127{return Ok(Scan::Rejected(400));}if byte==35{return Ok(Scan::Rejected(400));}second_space=second_space+1;}
  if second_space==target_start{return Ok(Scan::Rejected(400));}if second_space>=line_end{return Ok(Scan::Rejected(400));}
  let version:Bytes=runtime.bytes_slice(raw,second_space+1,line_end-second_space-1)?;
  let version_status:U16=check_version(version)?;if version_status!=0{return Ok(Scan::Rejected(version_status));}
  let method:Bytes=runtime.bytes_slice(raw,0,first_space)?;
  let connect:Bytes=runtime.string_to_bytes("CONNECT")?;if runtime.bytes_equal(method,connect){return Ok(Scan::Rejected(405));}drop(connect);
  if runtime.bytes_get(raw,target_start)!=cast<U8>(47){return Ok(Scan::Rejected(400));}
  let mut target_end:Usize=target_start;while target_end<second_space{let byte:U8=runtime.bytes_get(raw,target_end);if byte==63{break;}if byte==35{return Ok(Scan::Rejected(400));}target_end=target_end+1;}
  let header_bytes:Bytes=runtime.bytes_slice(raw,line_end+2,length-line_end-2)?;let header_status:U16=scan_headers(header_bytes)?;if header_status!=0{return Ok(Scan::Rejected(header_status));}
  let encoded_path:Bytes=runtime.bytes_slice(raw,target_start,target_end-target_start)?;
  let path:PathScan=decode_path(encoded_path)?;
  match path {Decoded(bytes)=>{return Ok(Scan::Parsed(method,bytes));}Rejected(status)=>{return Ok(Scan::Rejected(status));}}
 }
 @id("http.parse") fn parse(raw:Bytes)->ParseResult effects [alloc] {
  let result:ScanResult=scan(raw);match result {Err(error)=>{return Err(HttpError::Io(error));}Ok(value)=>{match value {Parsed(method,path)=>{return Ok(Request::Parsed(method,path));}Rejected(status)=>{return Err(HttpError::Protocol(status));}}}}
 }

 @id("http.empty_bytes") fn empty_bytes()->Bytes effects [alloc] {
  @id("http.empty_bytes.const") op const inputs [] outputs [@id("http.empty_bytes.value") Bytes] attributes {"value":[]} effects [alloc] consumes [] produces [@id("http.empty_bytes.value") Bytes];
  return http.empty_bytes.value;
 }
 @id("http.match_path") fn match_path(path:Bytes,pattern:Bytes)->MatchResult effects [alloc] {
  let path_length:Usize=runtime.bytes_len(path);let pattern_length:Usize=runtime.bytes_len(pattern);
  let mut i:Usize=0;let mut j:Usize=0;let mut capture_start:Usize=0;let mut capture_length:Usize=0;let mut captured:Bool=false;
  while j<pattern_length {
   let current:U8=runtime.bytes_get(pattern,j);
   if current==123 {
    if captured{return Err(HttpError::Handler());}if j>0{if runtime.bytes_get(pattern,j-1)!=cast<U8>(47){return Err(HttpError::Handler());}}
    captured=true;j=j+1;let name_start:Usize=j;
    while j<pattern_length {if runtime.bytes_get(pattern,j)==cast<U8>(125){break;}if runtime.bytes_get(pattern,j)==cast<U8>(47){return Err(HttpError::Handler());}j=j+1;}
    if j==name_start{return Err(HttpError::Handler());}if j==pattern_length{return Err(HttpError::Handler());}j=j+1;
    if j<pattern_length{if runtime.bytes_get(pattern,j)!=cast<U8>(47){return Err(HttpError::Handler());}}
    capture_start=i;while i<path_length{if runtime.bytes_get(path,i)==cast<U8>(47){break;}i=i+1;}
    capture_length=i-capture_start;if capture_length==0{return Ok(PathMatch::Miss());}
   }else{if i>=path_length{return Ok(PathMatch::Miss());}if runtime.bytes_get(path,i)!=current{return Ok(PathMatch::Miss());}i=i+1;j=j+1;}
  }
  if i!=path_length{return Ok(PathMatch::Miss());}
  if capture_length>128{return Err(HttpError::Protocol(414));}
  let slice:BytesResult=runtime.bytes_slice(path,capture_start,capture_length);
  match slice {Err(error)=>{return Err(HttpError::Io(error));}Ok(bytes)=>{let text:StringResult=runtime.string_from_utf8(bytes);match text{Err(error)=>{return Err(HttpError::Io(error));}Ok(value)=>{return Ok(PathMatch::Match(value));}}}}
 }
 @id("http.decimal") fn decimal(value:Usize)->BytesResult effects [alloc] {
  let mut remaining:Usize=value;let mut output:Bytes=runtime.string_to_bytes("")?;
  if remaining==0{return runtime.bytes_from_u8(48);}
  while remaining>0 {let digit:U8=cast<U8>(remaining%10)+48;let byte:Bytes=runtime.bytes_from_u8(digit)?;output=runtime.bytes_concat(byte,output)?;remaining=remaining/10;}
  return Ok(output);
 }
 @id("http.append_text") fn append_text(output:Bytes,text:String)->BytesResult effects [alloc] {let bytes:Bytes=runtime.string_to_bytes(text)?;return runtime.bytes_concat(output,bytes);}
 @id("http.content_header") fn content_header(content_type:String)->BytesResult effects [alloc] {
  let content:Bytes=runtime.string_to_bytes(content_type)?;let mut index:Usize=0;let length:Usize=runtime.bytes_len(content);
  while index<length {let byte:U8=runtime.bytes_get(content,index);if byte==13{return Err(core.IoError::InvalidData());}if byte==10{return Err(core.IoError::InvalidData());}index=index+1;}
  if length==0{return Ok(empty_bytes());}
  let prefix:Bytes=runtime.string_to_bytes("Content-Type: ")?;let joined:Bytes=runtime.bytes_concat(prefix,content)?;return append_text(joined,"\r\n");
 }
 @id("http.response_headers") fn response_headers(status:U16,content_type:String,size:Usize)->BytesResult effects [alloc] {
  if status<100{return Err(core.IoError::InvalidData());}if status>599{return Err(core.IoError::InvalidData());}
  let code:Bytes=decimal(cast<Usize>(status))?;let prefix:Bytes=runtime.string_to_bytes("HTTP/1.1 ")?;
  let mut output:Bytes=runtime.bytes_concat(prefix,code)?;drop(prefix);drop(code);output=append_text(output," \r\n")?;
  let content:Bytes=content_header(content_type)?;output=runtime.bytes_concat(output,content)?;drop(content);
  output=append_text(output,"Content-Length: ")?;let length:Bytes=decimal(size)?;output=runtime.bytes_concat(output,length)?;drop(length);
  output=append_text(output,"\r\nConnection: close\r\n")?;if status==405{output=append_text(output,"Allow: GET\r\n")?;}
  return append_text(output,"\r\n");
 }
 @id("http.encode") fn encode(response:Response)->BytesResult effects [alloc] {
  match response {Response(status,content_type,body)=>{let headers:Bytes=response_headers(status,content_type,runtime.bytes_len(body))?;return runtime.bytes_concat(headers,body);}}
 }
 @id("http.error_response") fn error_response(error:HttpError)->Response effects [alloc] {
  let mut status:U16=500;match error{Protocol(code)=>{status=code;}Io(io_error)=>{match io_error{Timeout=>{status=408;}Read=>{}Write=>{}Closed=>{}PermissionDenied=>{}InvalidData=>{}Other=>{}}}Handler=>{}}
  return Response::Response(status,"",empty_bytes());
 }
 @id("http.write_response") fn write_response(stream:net.Stream,response:Response,deadline:core.Deadline)->UnitResult effects [net,alloc] {
  let data:Bytes=encode(response)?;let length:Usize=runtime.bytes_len(data);let mut offset:Usize=0;
  while offset<length {let count:Usize=runtime.net_write(stream,data,offset,deadline)?;if count==0{return Err(core.IoError::Write());}offset=offset+count;}
  return Ok(());
 }
 @id("http.read_request") fn read_request(stream:net.Stream,deadline:core.Deadline)->Read effects [net,alloc] {
  let mut raw:Bytes=empty_bytes();let mut scanned:Usize=0;let mut line_finished:Bool=false;let mut header_start:Usize=0;
  while true {
   let incoming:BytesResult=runtime.net_read(stream,4096,deadline);
   match incoming {Err(error)=>{match error{Timeout=>{return Read::Read(stream,Err(HttpError::Protocol(408)));}Read=>{return Read::Read(stream,Err(HttpError::Io(core.IoError::Read())));}Write=>{return Read::Read(stream,Err(HttpError::Io(core.IoError::Write())));}Closed=>{return Read::Read(stream,Err(HttpError::Io(core.IoError::Closed())));}PermissionDenied=>{return Read::Read(stream,Err(HttpError::Io(core.IoError::PermissionDenied())));}InvalidData=>{return Read::Read(stream,Err(HttpError::Io(core.IoError::InvalidData())));}Other=>{return Read::Read(stream,Err(HttpError::Io(core.IoError::Other())));}}}
    Ok(chunk)=>{
     if runtime.bytes_len(chunk)==cast<Usize>(0){return Read::Read(stream,Err(HttpError::Protocol(400)));}
     let joined:BytesResult=runtime.bytes_concat(raw,chunk);match joined{Err(error)=>{return Read::Read(stream,Err(HttpError::Io(error)));}Ok(bytes)=>{raw=bytes;}}
    }
   }
   let length:Usize=runtime.bytes_len(raw);
   while scanned<length {
    let byte:U8=runtime.bytes_get(raw,scanned);
    if !line_finished {
     if scanned>=4096{if byte!=13{return Read::Read(stream,Err(HttpError::Protocol(414)));}}
     if byte==10{return Read::Read(stream,Err(HttpError::Protocol(400)));}
     if byte==13 {if length-scanned<2{break;}if runtime.bytes_get(raw,scanned+1)!=cast<U8>(10){return Read::Read(stream,Err(HttpError::Protocol(400)));}line_finished=true;header_start=scanned+2;scanned=scanned+1;}
    }else{
     if scanned+1-header_start>16384{return Read::Read(stream,Err(HttpError::Protocol(431)));}
     if byte==10{if runtime.bytes_get(raw,scanned-1)!=cast<U8>(13){return Read::Read(stream,Err(HttpError::Protocol(400)));}}
     if byte==13{if length-scanned<2{break;}if runtime.bytes_get(raw,scanned+1)!=cast<U8>(10){return Read::Read(stream,Err(HttpError::Protocol(400)));}}
     if scanned>=3 {if byte==10{if runtime.bytes_get(raw,scanned-1)==cast<U8>(13){if runtime.bytes_get(raw,scanned-2)==cast<U8>(10){if runtime.bytes_get(raw,scanned-3)==cast<U8>(13){return Read::Read(stream,parse(raw));}}}}}
    }
    scanned=scanned+1;
   }
  }
  return Read::Read(stream,Err(HttpError::Handler()));
 }
}
