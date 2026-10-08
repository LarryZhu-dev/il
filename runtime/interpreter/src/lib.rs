//! Deterministic execution of verified il MIR with bounded captured host effects.
mod machine;
mod numbers;
mod value;

pub use machine::execute;
pub use value::{Execution, ExecutionStatus, LifecycleEvent, LifecycleKind, Limits, Value, ValueData, VariantValue};
