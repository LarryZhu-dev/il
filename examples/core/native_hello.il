module app {
    @id("main") fn main() -> I32 effects [alloc, process] {
        runtime.print_string("Hello from Intelligent language!\n");
        return 0;
    }
}
