@id("net") module net visibility public imports [core] {
 @id("net.Listener") type Listener=record {slot:U64,generation:U64} layout opaque;
 @id("net.Stream") type Stream=record {slot:U64,generation:U64} layout opaque;
 @id("net.ListenerResult") type ListenerResult=Result<Listener,core.IoError>;
 @id("net.StreamResult") type StreamResult=Result<Stream,core.IoError>;
 @id("net.UnitResult") type UnitResult=Result<Unit,core.IoError>;
 @id("net.CountResult") type CountResult=Result<Usize,core.IoError>;
 @id("net.write_all") fn write_all(stream:Stream,data:Bytes,deadline:core.Deadline)->CountResult effects [net] {
  let length:Usize=runtime.bytes_len(data);let mut offset:Usize=0;
  while offset<length {let count:Usize=runtime.net_write(stream,data,offset,deadline)?;if count==0{return Err(core.IoError::Write());}offset=offset+count;}
  return Ok(offset);
 }
 @id("net.close_stream") fn close_stream(stream:Stream)->UnitResult effects [net] {return runtime.net_close_stream(stream);}
 @id("net.close_listener") fn close_listener(listener:Listener)->UnitResult effects [net] {return runtime.net_close_listener(listener);}
}
