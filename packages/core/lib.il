@id("core") module core visibility public {
  @id("core.File") type File = record { slot: U64, generation: U64 } layout opaque;
  @id("core.Deadline") type Deadline = sum { Infinite, At(U64) };
  @id("core.IoError") type IoError = sum { Read, Write, Closed, Timeout, PermissionDenied, InvalidData, Other };
  @id("core.AllocError") type AllocError = sum { OutOfMemory, CapacityOverflow };

  @id("core.infinite") fn infinite() -> Deadline {
    return Deadline::Infinite();
  }
  @id("core.deadline_at") fn deadline_at(instant: U64) -> Deadline {
    return Deadline::At(instant);
  }
  @id("core.max_i64") fn max_i64(left: I64, right: I64) -> I64 {
    if left > right { return left; }
    return right;
  }
}
