use std::io::{self, Write};

/// C ABI bridge used by the native backend acceptance probe.
#[no_mangle]
pub extern "C" fn il_probe_write() -> i32 {
    let mut stdout = io::stdout().lock();
    match stdout.write_all(b"probe_ok\n").and_then(|()| stdout.flush()) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}
