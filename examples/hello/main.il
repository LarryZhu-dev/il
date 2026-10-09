@id("hello") module hello imports [core, alloc, io] {
  @id("hello.main") fn main() -> I32 effects [alloc, process] {
    let greeting: alloc.StringResult = alloc.concat("Hello from ", "Intelligent language!\n");
    match greeting {
      Ok(text) => {
        let written: io.UnitResult = io.print_string(text);
        match written {
          Ok(unit) => { return 0; }
          Err(error) => { return 2; }
        }
      }
      Err(error) => { return 1; }
    }
  }
}
