module app {
    fn main() -> I32 {
        let mut n: I32 = 0;
        let mut total: I32 = 0;
        while n < 10 {
            n = n + 1;
            if n == 3 { continue; }
            if n == 7 { break; }
            total = total + n;
        }
        return total;
    }
}
