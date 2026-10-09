use il_graph::{CapabilityKind, Effect};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Passing { Copy, Shared, Owned }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeParameter { pub type_ref: &'static str, pub passing: Passing }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeResult { Exact(&'static str), Result { success: &'static str, error: &'static str } }

#[derive(Clone, Copy, Debug)]
pub struct RuntimeSignature {
    pub symbol: &'static str,
    pub parameters: &'static [RuntimeParameter],
    pub result: RuntimeResult,
    pub effects: &'static [Effect],
    pub capability: Option<CapabilityKind>,
}

const I64_COPY: RuntimeParameter = RuntimeParameter { type_ref: "I64", passing: Passing::Copy };
const STRING_SHARED: RuntimeParameter = RuntimeParameter { type_ref: "String", passing: Passing::Shared };
const BYTES_SHARED: RuntimeParameter = RuntimeParameter { type_ref: "Bytes", passing: Passing::Shared };

const FILE_SHARED: RuntimeParameter = RuntimeParameter { type_ref: "core.File", passing: Passing::Shared };
const FILE_OWNED: RuntimeParameter = RuntimeParameter { type_ref: "core.File", passing: Passing::Owned };
const USIZE_COPY: RuntimeParameter = RuntimeParameter { type_ref: "Usize", passing: Passing::Copy };
const DEADLINE_COPY: RuntimeParameter = RuntimeParameter { type_ref: "core.Deadline", passing: Passing::Copy };

const LISTENER_SHARED: RuntimeParameter = RuntimeParameter { type_ref: "net.Listener", passing: Passing::Shared };
const LISTENER_OWNED: RuntimeParameter = RuntimeParameter { type_ref: "net.Listener", passing: Passing::Owned };
const STREAM_SHARED: RuntimeParameter = RuntimeParameter { type_ref: "net.Stream", passing: Passing::Shared };
const STREAM_OWNED: RuntimeParameter = RuntimeParameter { type_ref: "net.Stream", passing: Passing::Owned };
const U8_COPY: RuntimeParameter = RuntimeParameter { type_ref: "U8", passing: Passing::Copy };

pub static RUNTIME_SIGNATURES: &[RuntimeSignature] = &[
    RuntimeSignature { symbol: "net_listen", parameters: &[], result: RuntimeResult::Result { success: "net.Listener", error: "core.IoError" }, effects: &[Effect::Net], capability: Some(CapabilityKind::Listen) },
    RuntimeSignature { symbol: "net_connect", parameters: &[DEADLINE_COPY], result: RuntimeResult::Result { success: "net.Stream", error: "core.IoError" }, effects: &[Effect::Net], capability: Some(CapabilityKind::Connect) },
    RuntimeSignature { symbol: "net_accept", parameters: &[LISTENER_SHARED, DEADLINE_COPY], result: RuntimeResult::Result { success: "net.Stream", error: "core.IoError" }, effects: &[Effect::Net], capability: None },
    RuntimeSignature { symbol: "net_read", parameters: &[STREAM_SHARED, USIZE_COPY, DEADLINE_COPY], result: RuntimeResult::Result { success: "Bytes", error: "core.IoError" }, effects: &[Effect::Net, Effect::Alloc], capability: None },
    RuntimeSignature { symbol: "net_write", parameters: &[STREAM_SHARED, BYTES_SHARED, USIZE_COPY, DEADLINE_COPY], result: RuntimeResult::Result { success: "Usize", error: "core.IoError" }, effects: &[Effect::Net], capability: None },
    RuntimeSignature { symbol: "net_close_listener", parameters: &[LISTENER_OWNED], result: RuntimeResult::Result { success: "Unit", error: "core.IoError" }, effects: &[Effect::Net], capability: None },
    RuntimeSignature { symbol: "net_close_stream", parameters: &[STREAM_OWNED], result: RuntimeResult::Result { success: "Unit", error: "core.IoError" }, effects: &[Effect::Net], capability: None },
    RuntimeSignature { symbol: "bytes_slice", parameters: &[BYTES_SHARED, USIZE_COPY, USIZE_COPY], result: RuntimeResult::Result { success: "Bytes", error: "core.IoError" }, effects: &[Effect::Alloc], capability: None },
    RuntimeSignature { symbol: "bytes_concat", parameters: &[BYTES_SHARED, BYTES_SHARED], result: RuntimeResult::Result { success: "Bytes", error: "core.IoError" }, effects: &[Effect::Alloc], capability: None },
    RuntimeSignature { symbol: "bytes_from_u8", parameters: &[U8_COPY], result: RuntimeResult::Result { success: "Bytes", error: "core.IoError" }, effects: &[Effect::Alloc], capability: None },
    RuntimeSignature { symbol: "string_to_bytes", parameters: &[STRING_SHARED], result: RuntimeResult::Result { success: "Bytes", error: "core.IoError" }, effects: &[Effect::Alloc], capability: None },
    RuntimeSignature { symbol: "string_from_utf8", parameters: &[BYTES_SHARED], result: RuntimeResult::Result { success: "String", error: "core.IoError" }, effects: &[Effect::Alloc], capability: None },
    RuntimeSignature { symbol: "bytes_get", parameters: &[BYTES_SHARED, USIZE_COPY], result: RuntimeResult::Exact("U8"), effects: &[], capability: None },
    RuntimeSignature { symbol: "bytes_equal", parameters: &[BYTES_SHARED, BYTES_SHARED], result: RuntimeResult::Exact("Bool"), effects: &[], capability: None },
    RuntimeSignature { symbol: "string_equal", parameters: &[STRING_SHARED, STRING_SHARED], result: RuntimeResult::Exact("Bool"), effects: &[], capability: None },
    RuntimeSignature { symbol: "net_opened_at", parameters: &[STREAM_SHARED], result: RuntimeResult::Exact("U64"), effects: &[Effect::Net], capability: None },
    RuntimeSignature { symbol: "file_open_read", parameters: &[STRING_SHARED], result: RuntimeResult::Result { success: "core.File", error: "core.IoError" }, effects: &[Effect::Fs], capability: Some(CapabilityKind::FileRead) },
    RuntimeSignature { symbol: "file_open_write", parameters: &[STRING_SHARED], result: RuntimeResult::Result { success: "core.File", error: "core.IoError" }, effects: &[Effect::Fs], capability: Some(CapabilityKind::FileWrite) },
    RuntimeSignature { symbol: "file_read_some", parameters: &[FILE_SHARED, USIZE_COPY, DEADLINE_COPY], result: RuntimeResult::Result { success: "Bytes", error: "core.IoError" }, effects: &[Effect::Fs, Effect::Alloc], capability: None },
    RuntimeSignature { symbol: "file_write_some", parameters: &[FILE_SHARED, BYTES_SHARED, USIZE_COPY, DEADLINE_COPY], result: RuntimeResult::Result { success: "Usize", error: "core.IoError" }, effects: &[Effect::Fs], capability: None },
    RuntimeSignature { symbol: "file_close", parameters: &[FILE_OWNED], result: RuntimeResult::Result { success: "Unit", error: "core.IoError" }, effects: &[Effect::Fs], capability: None },
    RuntimeSignature { symbol: "stdin_read", parameters: &[USIZE_COPY, DEADLINE_COPY], result: RuntimeResult::Result { success: "Bytes", error: "core.IoError" }, effects: &[Effect::Process, Effect::Alloc], capability: None },
    RuntimeSignature { symbol: "stdout_write", parameters: &[BYTES_SHARED, USIZE_COPY, DEADLINE_COPY], result: RuntimeResult::Result { success: "Usize", error: "core.IoError" }, effects: &[Effect::Process], capability: None },
    RuntimeSignature { symbol: "stderr_write", parameters: &[BYTES_SHARED, USIZE_COPY, DEADLINE_COPY], result: RuntimeResult::Result { success: "Usize", error: "core.IoError" }, effects: &[Effect::Process], capability: None },

    RuntimeSignature { symbol: "print_i64", parameters: &[I64_COPY], result: RuntimeResult::Result { success: "Unit", error: "core.IoError" }, effects: &[Effect::Process], capability: None },
    RuntimeSignature { symbol: "print_string", parameters: &[STRING_SHARED], result: RuntimeResult::Result { success: "Unit", error: "core.IoError" }, effects: &[Effect::Process], capability: None },
    RuntimeSignature { symbol: "string_len", parameters: &[STRING_SHARED], result: RuntimeResult::Exact("Usize"), effects: &[], capability: None },
    RuntimeSignature { symbol: "bytes_len", parameters: &[BYTES_SHARED], result: RuntimeResult::Exact("Usize"), effects: &[], capability: None },
    RuntimeSignature { symbol: "string_concat", parameters: &[STRING_SHARED, STRING_SHARED], result: RuntimeResult::Result { success: "String", error: "core.AllocError" }, effects: &[Effect::Alloc], capability: None },
    RuntimeSignature { symbol: "clock_now", parameters: &[], result: RuntimeResult::Exact("U64"), effects: &[Effect::Clock], capability: Some(CapabilityKind::ClockRead) },
    RuntimeSignature { symbol: "file_read", parameters: &[STRING_SHARED], result: RuntimeResult::Result { success: "Bytes", error: "core.IoError" }, effects: &[Effect::Fs, Effect::Alloc], capability: Some(CapabilityKind::FileRead) },
    RuntimeSignature { symbol: "file_write", parameters: &[STRING_SHARED, BYTES_SHARED], result: RuntimeResult::Result { success: "Usize", error: "core.IoError" }, effects: &[Effect::Fs], capability: Some(CapabilityKind::FileWrite) },
];

pub fn runtime_signature(symbol: &str) -> Option<&'static RuntimeSignature> { RUNTIME_SIGNATURES.iter().find(|signature| signature.symbol == symbol) }

pub fn runtime_error_variants(type_ref: &str) -> Option<&'static [&'static str]> {
    match type_ref {
        "core.IoError" => Some(&["Read", "Write", "Closed", "Timeout", "PermissionDenied", "InvalidData", "Other"]),
        "core.AllocError" => Some(&["OutOfMemory", "CapacityOverflow"]),
        _ => None,
    }
}
