use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[derive(Debug)]
pub struct Failure { pub code: String, pub message: String, pub committed_revision: Option<u64>, pub diagnostics: Vec<il_graph::Diagnostic> }

impl Failure {
    pub fn new(code: &str, message: &str) -> Self { Self { code: code.into(), message: message.into(), committed_revision: None, diagnostics: vec![] } }
    pub fn input(message: &str) -> Self { Self::new("E_SCHEMA_INVALID", message) }
    pub fn io(error: std::io::Error) -> Self { Self::new("E_TOOLCHAIN_FAILURE", &error.to_string()) }
    pub fn json(error: serde_json::Error) -> Self { Self::input(&error.to_string()) }
    pub fn diagnostics(diagnostics: Vec<il_graph::Diagnostic>) -> Self {
        Self { code: diagnostics.first().map(|d| d.code.clone()).unwrap_or_else(|| "E_SCHEMA_INVALID".into()),
            message: "language validation failed".into(), committed_revision: None, diagnostics }
    }
}

impl From<il_graph::StoreError> for Failure {
    fn from(error: il_graph::StoreError) -> Self { Self { code: error.code, message: error.message, committed_revision: error.committed_revision, diagnostics: vec![] } }
}

pub fn envelope(tool: &str, base: u64, revision: u64, result: Value) -> Value {
    json!({"ok": true, "tool": tool, "tool_version": "1.0.0", "base_revision": base,
        "result_revision": revision, "diagnostics": [], "artifacts": [], "evidence_id": null, "result": result})
}

pub fn failed(tool: &str, base: u64, error: &Failure) -> Value {
    let identity = format!("{}:{}:{}", error.code, base, error.message);
    let mut value = envelope(tool, base, error.committed_revision.unwrap_or(base),
        error.committed_revision.map(|revision| json!({"committed": true, "revision": revision})).unwrap_or(Value::Null));
    value["ok"] = json!(false);
    value["diagnostics"] = json!([{
        "diagnostic_id": format!("diag_{:x}", Sha256::digest(identity.as_bytes())),
        "code": error.code, "stage": "tool_protocol", "severity": "error", "entity_id": null,
        "related_entities": [], "expected": null, "actual": null, "cause": error.message,
        "suggested_operations": [], "retryable": error.code == "E_STALE_REVISION", "base_revision": base
    }]);
    if !error.diagnostics.is_empty() { value["diagnostics"] = json!(error.diagnostics); }
    value
}
