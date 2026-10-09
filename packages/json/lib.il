@id("json") module json visibility public imports [core] {
 @id("json.BytesResult") type BytesResult=Result<Bytes,core.IoError>;
 @id("json.hex_digit") fn hex_digit(n:U8)->U8 {if n<10{return n+48;}return n+87;}
 @id("json.quote") fn quote(value:String)->BytesResult effects [alloc] {
  let data:Bytes=runtime.string_to_bytes(value)?;
  let mut output:Bytes=runtime.bytes_from_u8(34)?;
  let length:Usize=runtime.bytes_len(data);let mut index:Usize=0;
  while index<length {
   let byte:U8=runtime.bytes_get(data,index);
   if byte==34 {let escaped:Bytes=runtime.string_to_bytes("\\\"")?;output=runtime.bytes_concat(output,escaped)?;}
   else if byte==92 {let escaped:Bytes=runtime.string_to_bytes("\\\\")?;output=runtime.bytes_concat(output,escaped)?;}
   else if byte<32 {let prefix:Bytes=runtime.string_to_bytes("\\u00")?;let high:Bytes=runtime.bytes_from_u8(hex_digit(byte/16))?;let low:Bytes=runtime.bytes_from_u8(hex_digit(byte%16))?;output=runtime.bytes_concat(output,prefix)?;output=runtime.bytes_concat(output,high)?;output=runtime.bytes_concat(output,low)?;}
   else {let literal:Bytes=runtime.bytes_from_u8(byte)?;output=runtime.bytes_concat(output,literal)?;}
   index=index+1;
  }
  let quote:Bytes=runtime.bytes_from_u8(34)?;return runtime.bytes_concat(output,quote);
 }
 @id("json.message") fn message(value:String)->BytesResult effects [alloc] {
  let quoted:Bytes=quote(value)?;let prefix:Bytes=runtime.string_to_bytes("{\"message\":")?;let suffix:Bytes=runtime.bytes_from_u8(125)?;
  let head:Bytes=runtime.bytes_concat(prefix,quoted)?;return runtime.bytes_concat(head,suffix);
 }
}
