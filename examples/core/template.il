module app {
    type Output = Result<I64, core.AllocError>;

    fn main() -> Output effects [alloc] {
        let name: String = "World";
        let greeting: String = f"Hello ${name}"?;
        let length: Usize = runtime.string_len(greeting);
        return Ok(cast<I64>(length));
    }
}
