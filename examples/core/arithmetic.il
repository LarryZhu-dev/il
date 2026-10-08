@id("math") module math {
  @id("math.add") fn add(@id("math.add.a") a: I64, @id("math.add.b") b: I64) -> I64 {
    @id("math.add.entry") block entry {
      @id("math.add.sum") let sum: I64 = a + b;
      @id("math.add.return") return sum;
    }
  }
  @id("math.main") fn main() -> I64 {
    @id("math.main.entry") block entry {
      @id("math.main.answer") let answer: I64 = add(20, 22);
      @id("math.main.return") return answer;
    }
  }
}
