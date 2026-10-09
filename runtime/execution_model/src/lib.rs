//! Shared execution values and reports; contains no evaluator.
use il_graph::Diagnostic;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Value {
    #[serde(rename = "type")]
    pub type_ref: String,
    pub data: ValueData,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case", deny_unknown_fields)]
pub enum ValueData {
    Unit,
    Bool(bool),
    Integer(String),
    String(String),
    Bytes(Vec<u8>),
    Record(Vec<Value>),
    Tuple(Vec<Value>),
    Variant(VariantValue),
    Resource(ResourceValue),
}

impl<'de> Deserialize<'de> for ValueData {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let mut object = serde_json::Map::<String,serde_json::Value>::deserialize(deserializer)?;
        let kind = object.remove("kind").and_then(|v| v.as_str().map(str::to_owned)).ok_or_else(|| D::Error::custom("value data requires a string kind"))?;
        if kind == "unit" {
            if !object.is_empty() { return Err(D::Error::custom("unit data accepts only kind")); }
            return Ok(Self::Unit);
        }
        let payload = object.remove("value").ok_or_else(|| D::Error::custom("value data requires value"))?;
        if !object.is_empty() { return Err(D::Error::custom("unknown value data field")); }
        fn decode<T: serde::de::DeserializeOwned,E: serde::de::Error>(value:serde_json::Value)->Result<T,E> { serde_json::from_value(value).map_err(E::custom) }
        match kind.as_str() {
            "bool" => decode(payload).map(Self::Bool), "integer" => decode(payload).map(Self::Integer),
            "string" => decode(payload).map(Self::String), "bytes" => decode(payload).map(Self::Bytes),
            "record" => decode(payload).map(Self::Record), "tuple" => decode(payload).map(Self::Tuple),
            "variant" => decode(payload).map(Self::Variant), "resource" => decode(payload).map(Self::Resource), _ => Err(D::Error::custom("unknown value data kind")),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceValue { pub kind: String, pub slot: u64, pub generation: u64 }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VariantValue { pub tag: String, pub fields: Vec<Value> }

impl Value {
    pub fn integer(type_ref: &str, value: i128) -> Self { Self { type_ref: type_ref.into(), data: ValueData::Integer(value.to_string()) } }
    pub fn unit() -> Self { Self { type_ref: "Unit".into(), data: ValueData::Unit } }
    pub fn boolean(value: bool) -> Self { Self { type_ref: "Bool".into(), data: ValueData::Bool(value) } }
    pub fn string(value: impl Into<String>) -> Self { Self { type_ref: "String".into(), data: ValueData::String(value.into()) } }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub max_steps: u64,
    pub max_call_depth: u32,
    pub max_heap_bytes: u64,
    pub max_output_bytes: u64,
}
impl Default for Limits {
    fn default() -> Self { Self { max_steps: 100_000, max_call_depth: 128, max_heap_bytes: 16_777_216, max_output_bytes: 1_048_576 } }
}
impl Limits {
    pub fn valid(self) -> bool {
        (1..=1_000_000).contains(&self.max_steps) && (1..=128).contains(&self.max_call_depth)
            && (1..=67_108_864).contains(&self.max_heap_bytes) && (1..=1_048_576).contains(&self.max_output_bytes)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus { Returned, Trapped, Rejected }

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleKind { Allocate, Move, Borrow, EndBorrow, Drop, Return }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LifecycleEvent { pub kind: LifecycleKind, pub entity_id: String, pub allocation_ids: Vec<u64> }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StackFrame {
    pub function_id: String,
    #[serde(deserialize_with = "required_nullable")]
    pub call_site: Option<String>,
    pub entity_id: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum HandleEventKind {
    #[serde(rename = "opened")] Open,
    #[serde(rename = "closed")] Close,
    #[serde(rename = "dropped")] Drop,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HandleEvent {
    pub kind: HandleEventKind,
    pub entity_id: String,
    pub slot: u64,
    pub generation: u64,
    #[serde(deserialize_with = "required_nullable")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Execution {
    pub status: ExecutionStatus,
    #[serde(deserialize_with = "required_nullable")]
    pub value: Option<Value>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub diagnostics: Vec<Diagnostic>,
    pub steps: u64,
    pub peak_heap_bytes: u64,
    pub live_allocations: u64,
    pub lifecycle: Vec<LifecycleEvent>,
    pub live_handles: u64,
    pub handle_events: Vec<HandleEvent>,
    pub stack_trace: Vec<StackFrame>,
}

fn required_nullable<'de,D:serde::Deserializer<'de>,T:Deserialize<'de>>(deserializer:D)->Result<Option<T>,D::Error> {
    Option::<T>::deserialize(deserializer)
}

impl Execution {
    pub fn rejected(diagnostics: Vec<Diagnostic>) -> Self {
        Self { status: ExecutionStatus::Rejected, value: None, stdout: vec![], stderr: vec![], diagnostics,
            steps: 0, peak_heap_bytes: 0, live_allocations: 0, lifecycle: vec![], live_handles: 0, handle_events: vec![], stack_trace: vec![] }
    }
}


/// Independent diagnostic reserve, measured as the compact JSON stack array.
pub const STACK_TRACE_MAX_BYTES: u64 = 1_048_576;
pub fn json_string_bytes(value: &str) -> u64 {
    value.bytes().fold(2u64, |size, byte| size.saturating_add(match byte {
        b'"' | b'\\' | b'\x08' | b'\x0c' | b'\n' | b'\r' | b'\t' => 2,
        0..=0x1f => 6, _ => 1,
    }))
}
pub fn stack_frame_json_bytes(function_id: &str, call_site: Option<&str>, entity_id: &str) -> u64 {
    let overhead = br#"{"function_id":,"call_site":,"entity_id":}"#.len() as u64;
    overhead.saturating_add(json_string_bytes(function_id)).saturating_add(call_site.map(json_string_bytes).unwrap_or(4)).saturating_add(json_string_bytes(entity_id))
}
pub fn stack_trace_push_bytes(active_bytes: u64, depth: usize, frame_bytes: u64) -> Option<u64> {
    let next = active_bytes.checked_add(frame_bytes)?.checked_add(u64::from(depth > 0))?;
    (active_bytes >= 2 && next <= STACK_TRACE_MAX_BYTES).then_some(next)
}
pub fn stack_trace_replace_bytes(active_bytes: u64, previous_frame_bytes: u64, new_frame_bytes: u64) -> Option<u64> {
    let next = active_bytes.checked_sub(previous_frame_bytes)?.checked_add(new_frame_bytes)?;
    (next >= 2 && next <= STACK_TRACE_MAX_BYTES).then_some(next)
}

#[cfg(test)]
mod stack_budget_tests {
    use super::*;
    #[test]
    fn borrowed_size_matches_json_with_escapes_and_utf8() {
        let samples = ["", "function.id", "\"\\\n\r\t\x08\x0c\x00\x1f", "智能语言😀"];
        for value in samples {
            assert_eq!(json_string_bytes(value), serde_json::to_vec(value).unwrap().len() as u64);
            for call_site in [None, Some(value)] {
                let frame = StackFrame { function_id: value.into(), call_site: call_site.map(str::to_owned), entity_id: value.into() };
                assert_eq!(stack_frame_json_bytes(value, call_site, value), serde_json::to_vec(&frame).unwrap().len() as u64);
            }
        }
    }
    #[test]
    fn stack_budget_boundaries_account_for_arrays_replacement_and_overflow() {
        assert_eq!(stack_trace_push_bytes(2, 0, STACK_TRACE_MAX_BYTES - 2), Some(STACK_TRACE_MAX_BYTES));
        assert_eq!(stack_trace_push_bytes(2, 0, STACK_TRACE_MAX_BYTES - 1), None);
        assert_eq!(stack_trace_push_bytes(STACK_TRACE_MAX_BYTES - 10, 1, 9), Some(STACK_TRACE_MAX_BYTES));
        assert_eq!(stack_trace_push_bytes(STACK_TRACE_MAX_BYTES - 10, 1, 10), None);
        assert_eq!(stack_trace_replace_bytes(STACK_TRACE_MAX_BYTES, 10, 10), Some(STACK_TRACE_MAX_BYTES));
        assert_eq!(stack_trace_replace_bytes(STACK_TRACE_MAX_BYTES, 10, 11), None);
        assert_eq!(stack_trace_replace_bytes(STACK_TRACE_MAX_BYTES, 10, 5), Some(STACK_TRACE_MAX_BYTES - 5));
        assert_eq!(stack_trace_push_bytes(u64::MAX, 1, 1), None);
        assert_eq!(stack_trace_replace_bytes(2, 10, 1), None);
    }
}
