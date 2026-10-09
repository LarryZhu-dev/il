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

pub static RUNTIME_SIGNATURES: &[RuntimeSignature] = &[
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
