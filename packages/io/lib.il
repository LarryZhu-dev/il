@id("io") module io visibility public imports [core] {
  @id("io.UnitResult") type UnitResult = Result<Unit, core.IoError>;
  @id("io.BytesResult") type BytesResult = Result<Bytes, core.IoError>;
  @id("io.CountResult") type CountResult = Result<Usize, core.IoError>;

  @id("io.print_string") fn print_string(value: String) -> UnitResult effects [process] {
    return runtime.print_string(value);
  }
  @id("io.print_i64") fn print_i64(value: I64) -> UnitResult effects [process] {
    return runtime.print_i64(value);
  }
  @id("io.read_stdin") fn read_stdin(maximum: Usize, deadline: core.Deadline) -> BytesResult effects [process, alloc] {
    return runtime.stdin_read(maximum, deadline);
  }
  @id("io.write_stdout") fn write_stdout(value: Bytes, deadline: core.Deadline) -> CountResult effects [process] {
    let length: Usize = runtime.bytes_len(value);
    let mut offset: Usize = 0;
    while offset < length {
      let count: Usize = runtime.stdout_write(value, offset, deadline)?;
      if count == 0 { return Err(core.IoError::Write()); }
      offset = offset + count;
    }
    return Ok(offset);
  }
  @id("io.write_stderr") fn write_stderr(value: Bytes, deadline: core.Deadline) -> CountResult effects [process] {
    let length: Usize = runtime.bytes_len(value);
    let mut offset: Usize = 0;
    while offset < length {
      let count: Usize = runtime.stderr_write(value, offset, deadline)?;
      if count == 0 { return Err(core.IoError::Write()); }
      offset = offset + count;
    }
    return Ok(offset);
  }
  @id("io.close_file") fn close_file(file: core.File) -> UnitResult effects [fs] {
    return runtime.file_close(file);
  }
}
