//! Explicit control flow and cleanup IR with an independent verifier.
mod lower;
mod verify;
pub use lower::{lower, lower_with_capabilities};
pub use verify::{verify, verify_with_capabilities};

use il_graph::*;
use serde::{Deserialize, Serialize};

fn required_nullable<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Program {
    pub schema_version: String,
    pub compiler_version: String,
    pub input_hash: String,
    pub source_revision: u64,
    pub target: String,
    pub types: Vec<TypeDef>,
    pub capabilities: Vec<Capability>,
    pub public_functions: Vec<EntityId>,
    pub functions: Vec<Function>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Function {
    pub entity_id: EntityId,
    pub name: String,
    pub parameters: Vec<Parameter>,
    pub result: TypeRef,
    pub effects: Vec<Effect>,
    pub capabilities: Vec<EntityId>,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Block {
    pub entity_id: EntityId,
    pub arguments: Vec<ValueDef>,
    pub operations: Vec<Operation>,
    pub terminator: Terminator,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DropAction { pub value_id: EntityId, pub type_ref: TypeRef }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Edge { pub target: EntityId, pub arguments: Vec<EntityId>, pub cleanup: Vec<DropAction> }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SwitchEdge { pub tag: String, pub edge: Edge }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Terminator {
    Return { entity_id: EntityId, #[serde(deserialize_with = "required_nullable")] value: Option<EntityId>, cleanup: Vec<DropAction> },
    Trap { entity_id: EntityId, code: String },
    Branch { entity_id: EntityId, edge: Edge },
    CondBranch { entity_id: EntityId, condition: EntityId, then_edge: Edge, else_edge: Edge },
    Switch { entity_id: EntityId, value: EntityId, cases: Vec<SwitchEdge>, default: Edge },
}

impl Terminator {
    pub fn entity_id(&self) -> &str {
        match self { Self::Return { entity_id, .. } | Self::Trap { entity_id, .. } | Self::Branch { entity_id, .. } | Self::CondBranch { entity_id, .. } | Self::Switch { entity_id, .. } => entity_id }
    }
    pub fn edges(&self) -> Vec<&Edge> {
        match self {
            Self::Branch { edge, .. } => vec![edge], Self::CondBranch { then_edge, else_edge, .. } => vec![then_edge, else_edge],
            Self::Switch { cases, default, .. } => cases.iter().map(|case| &case.edge).chain(std::iter::once(default)).collect(), _ => vec![],
        }
    }
}

impl Program {
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, serde_json::Error> { canonical_bytes(self) }
    pub fn hash(&self) -> Result<String, serde_json::Error> { self.canonical_bytes().map(|bytes| hash_bytes(&bytes)) }
    pub fn stage_record(&self) -> Result<il_hir::StageRecord, serde_json::Error> {
        Ok(il_hir::StageRecord { stage: "lower_to_mir".into(), input_hash: self.input_hash.clone(), output_hash: self.hash()?, compiler_version: self.compiler_version.clone(), diagnostics: vec![] })
    }
}
