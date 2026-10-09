@id("alloc") module alloc visibility public imports [core] {
  @id("alloc.StringResult") type StringResult = Result<String, core.AllocError>;

  @id("alloc.concat") fn concat(left: String, right: String) -> StringResult effects [alloc] {
    return runtime.string_concat(left, right);
  }
  @id("alloc.clone_string") fn clone_string(value: String) -> String effects [alloc] {
    return clone(value);
  }
  @id("alloc.clone_bytes") fn clone_bytes(value: Bytes) -> Bytes effects [alloc] {
    return clone(value);
  }
  @id("alloc.string_size") fn string_size(value: String) -> Usize {
    return runtime.string_len(value);
  }
  @id("alloc.bytes_size") fn bytes_size(value: Bytes) -> Usize {
    return runtime.bytes_len(value);
  }
}
