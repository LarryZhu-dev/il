//! Deterministic execution of verified il MIR with bounded captured host effects.
mod machine;
mod numbers;
pub use machine::execute;
