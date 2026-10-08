@id("app") module app {
  @id("app.main") fn main() -> I32 effects [] capabilities [] {
    @id("app.main.entry") block entry {
      @id("app.main.zero") let zero: I32 = 0;
      @id("app.main.return") return zero;
    }
  }
}
