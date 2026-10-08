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
            "variant" => decode(payload).map(Self::Variant), _ => Err(D::Error::custom("unknown value data kind")),
        }
    }
}

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
}

fn required_nullable<'de,D:serde::Deserializer<'de>>(deserializer:D)->Result<Option<Value>,D::Error> {
    Option::<Value>::deserialize(deserializer)
}

impl Execution {
    pub fn rejected(diagnostics: Vec<Diagnostic>) -> Self {
        Self { status: ExecutionStatus::Rejected, value: None, stdout: vec![], stderr: vec![], diagnostics,
            steps: 0, peak_heap_bytes: 0, live_allocations: 0, lifecycle: vec![] }
    }
}
