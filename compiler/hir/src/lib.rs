//! Checked high-level IR. Surface syntax is eliminated before this boundary.
use il_graph::{canonical_bytes, hash_bytes, Capability, Diagnostic, Graph};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Program {
    pub schema_version: String,
    pub compiler_version: String,
    pub input_hash: String,
    pub graph: Graph,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StageRecord {
    pub stage: String,
    pub input_hash: String,
    pub output_hash: String,
    pub compiler_version: String,
    pub diagnostics: Vec<Diagnostic>,
}

impl Program {
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, serde_json::Error> { canonical_bytes(self) }
    pub fn hash(&self) -> Result<String, serde_json::Error> { self.canonical_bytes().map(|bytes| hash_bytes(&bytes)) }
    pub fn stage_record(&self) -> Result<StageRecord, serde_json::Error> {
        Ok(StageRecord { stage: "lower_to_hir".into(), input_hash: self.input_hash.clone(), output_hash: self.hash()?, compiler_version: self.compiler_version.clone(), diagnostics: vec![] })
    }
}

pub fn lower(graph: &Graph) -> Result<Program, Vec<Diagnostic>> { lower_with_capabilities(graph, &[]) }

pub fn lower_with_capabilities(graph: &Graph, capabilities: &[Capability]) -> Result<Program, Vec<Diagnostic>> {
    let diagnostics = il_checker::check_with_capabilities(graph, capabilities);
    if !diagnostics.is_empty() { return Err(diagnostics); }
    let input_hash = graph.hash().map_err(|error| vec![Diagnostic::error("E_SCHEMA_INVALID", None, error.to_string(), graph.revision)])?;
    Ok(Program { schema_version: "1.0.0".into(), compiler_version: env!("CARGO_PKG_VERSION").into(), input_hash, graph: graph.clone() })
}
