@id("tracing") module tracing visibility public imports [core,json,io] {
 @id("tracing.CountResult") type CountResult=Result<Usize,core.IoError>;
 @id("tracing.event") fn event(code:String)->CountResult effects [alloc,process] {
  let encoded:Bytes=json.quote(code)?;let prefix:Bytes=runtime.string_to_bytes("{\"code\":")?;let suffix:Bytes=runtime.string_to_bytes("}\n")?;
  let head:Bytes=runtime.bytes_concat(prefix,encoded)?;let line:Bytes=runtime.bytes_concat(head,suffix)?;let count:Usize=io.write_stderr(line,core.Deadline::Infinite())?;return Ok(count);
 }
}
