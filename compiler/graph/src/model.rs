use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

use crate::{GRAPH_VERSION, TARGET};

pub type EntityId = String;
pub type TypeRef = String;

fn optional_non_null<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

fn required_nullable<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}

fn has_duplicates<T: Ord>(values: &[T]) -> bool {
    values.iter().collect::<BTreeSet<_>>().len() != values.len()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Graph {
    pub project_id: String,
    pub graph_version: String,
    pub revision: u64,
    pub target: String,
    pub modules: Vec<Module>,
    pub types: Vec<TypeDef>,
    pub functions: Vec<Function>,
    pub capabilities: Vec<Capability>,
    pub packages: Vec<Package>,
    pub contracts: Vec<Contract>,
}

pub type ProgramGraph = Graph;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Module {
    pub entity_id: EntityId,
    pub path: String,
    pub imports: Vec<EntityId>,
    pub declarations: Vec<EntityId>,
    pub visibility: Visibility,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Visibility { Private, Public }

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TypeKind { Unit, Never, Bool, Int, Usize, String, Bytes, Record, Sum, Option, Result, Tuple }

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Layout { Inferred, Ffi, Opaque }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TypeDef {
    pub entity_id: EntityId,
    pub kind: TypeKind,
    pub parameters: Vec<TypeRef>,
    pub layout: Layout,
    #[serde(default, deserialize_with = "optional_non_null", skip_serializing_if = "Option::is_none")]
    pub integer: Option<IntegerLayout>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<Field>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<Variant>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IntegerLayout { pub signed: bool, pub bits: u8 }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Field { pub name: String, #[serde(rename = "type")] pub type_ref: TypeRef }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Variant { pub name: String, pub fields: Vec<TypeRef> }

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Effect { Alloc, Fs, Net, Clock, Process, Unsafe }

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
    pub contracts: Vec<EntityId>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub entity_id: EntityId,
    pub name: String,
    #[serde(rename = "type")]
    pub type_ref: TypeRef,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ValueDef { pub entity_id: EntityId, #[serde(rename = "type")] pub type_ref: TypeRef }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Block {
    pub entity_id: EntityId,
    pub arguments: Vec<ValueDef>,
    pub operations: Vec<Operation>,
    pub terminator: Operation,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Opcode {
    Const, Add, Sub, Mul, Div, Rem, Shl, Shr, BitAnd, BitOr, BitXor,
    Eq, Ne, Lt, Le, Gt, Ge, Not, Cast, Call, Record, Field, Tuple, TupleGet,
    Variant, Tag, Payload, Move, Clone, Borrow, BorrowMut, EndBorrow, Drop,
    Branch, CondBranch, Switch, Return, Trap, RuntimeCall,
}

impl Opcode {
    pub fn is_terminator(self) -> bool {
        matches!(self, Self::Branch | Self::CondBranch | Self::Switch | Self::Return | Self::Trap)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Literal { Bool(bool), Integer(i64), Unsigned(u64), String(String), Bytes(Vec<u8>), Unit(()) }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged, deny_unknown_fields)]
pub enum Attributes {
    Constant { value: Literal },
    Call { callee: EntityId },
    RuntimeCall {
        symbol: String,
        #[serde(default, deserialize_with = "optional_non_null", skip_serializing_if = "Option::is_none")]
        capability: Option<EntityId>,
    },
    Cast { target_type: TypeRef },
    Record { type_id: TypeRef },
    Field { field: String },
    TupleGet { index: u32 },
    Variant { type_id: TypeRef, variant: String },
    Branch { target: EntityId },
    CondBranch { then_block: EntityId, else_block: EntityId, then_arguments: Vec<EntityId>, else_arguments: Vec<EntityId> },
    Switch { cases: Vec<SwitchCase>, default: EntityId, default_arguments: Vec<EntityId> },
    Trap { code: String },
    Empty {},
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SwitchCase { pub tag: String, pub target: EntityId, pub arguments: Vec<EntityId> }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub entity_id: EntityId,
    pub opcode: Opcode,
    pub inputs: Vec<EntityId>,
    pub outputs: Vec<ValueDef>,
    pub attributes: Attributes,
    pub effects: Vec<Effect>,
    pub consumes: Vec<EntityId>,
    pub produces: Vec<ValueDef>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CapabilityKind { FileRead, FileWrite, Listen, Connect, SpawnProcess, ClockRead }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    pub entity_id: EntityId,
    pub kind: CapabilityKind,
    #[serde(deserialize_with = "required_nullable")]
    pub scope: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub entity_id: EntityId,
    pub name: String,
    pub version: String,
    pub modules: Vec<EntityId>,
    pub effects: Vec<Effect>,
    pub capabilities: Vec<EntityId>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Contract {
    pub entity_id: EntityId,
    pub subject: EntityId,
    pub predicates: Vec<ContractPredicate>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContractPredicate {
    Returns { type_ref: TypeRef },
    RequiresEffect { effect: Effect },
    RequiresCapability { capability: EntityId },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub diagnostic_id: String,
    pub code: String,
    pub stage: String,
    pub severity: String,
    pub entity_id: Option<EntityId>,
    pub related_entities: Vec<EntityId>,
    pub expected: Option<String>,
    pub actual: Option<String>,
    pub cause: String,
    pub suggested_operations: Vec<crate::transaction::TransactionOperation>,
    pub retryable: bool,
    pub base_revision: u64,
}

impl Diagnostic {
    pub fn error(code: &str, entity_id: Option<&str>, cause: impl Into<String>, revision: u64) -> Self {
        let cause = cause.into();
        let identity = format!("{code}\0{}\0{cause}\0{revision}", entity_id.unwrap_or(""));
        Self { diagnostic_id: format!("diag_{}", &hash_bytes(identity.as_bytes())[7..23]), code: code.into(),
            stage: "graph_validate".into(), severity: "error".into(), entity_id: entity_id.map(str::to_owned),
            related_entities: vec![], expected: None, actual: None, cause,
            suggested_operations: vec![], retryable: !matches!(code, "E_UNSUPPORTED_FEATURE" | "E_STATE_INCONSISTENT" | "E_COMMIT_DURABILITY_UNCERTAIN"), base_revision: revision }
    }
}

pub fn hash_bytes(bytes: &[u8]) -> String { format!("sha256:{:x}", Sha256::digest(bytes)) }

pub fn canonical_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub fn valid_id(id: &str) -> bool {
    let mut bytes = id.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_')) &&
        bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-'))
}

pub fn is_builtin_type(name: &str) -> bool {
    matches!(name, "Unit" | "Never" | "Bool" | "I8" | "I16" | "I32" | "I64" | "U8" | "U16" | "U32" | "U64" | "Usize" | "String" | "Bytes")
}

impl Graph {
    pub fn empty() -> Self {
        Self { project_id: "il".into(), graph_version: GRAPH_VERSION.into(), revision: 0, target: TARGET.into(),
            modules: vec![], types: vec![], functions: vec![], capabilities: vec![], packages: vec![], contracts: vec![] }
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, serde_json::Error> { canonical_bytes(self) }
    pub fn hash(&self) -> Result<String, serde_json::Error> { self.canonical_bytes().map(|bytes| hash_bytes(&bytes)) }
    pub fn parse(bytes: &[u8]) -> Result<Self, serde_json::Error> { serde_json::from_slice(bytes) }

    pub fn has_semantics(&self) -> bool {
        !self.types.is_empty() || !self.functions.is_empty() || !self.capabilities.is_empty() || !self.packages.is_empty() || !self.contracts.is_empty() || self.modules.iter().any(|module| !module.declarations.is_empty())
    }

    pub fn validate_structural(&self) -> Vec<Diagnostic> {
        let mut diagnostics = vec![];
        let mut ids = BTreeSet::new();
        let mut add_id = |id: &str| {
            if id == "program" { diagnostics.push(Diagnostic::error("E_SCHEMA_INVALID", Some(id), "program is the reserved whole-program scope", self.revision)); }
            if !valid_id(id) { diagnostics.push(Diagnostic::error("E_SCHEMA_INVALID", Some(id), "invalid stable logical entity identifier", self.revision)); }
            if !ids.insert(id.to_owned()) { diagnostics.push(Diagnostic::error("E_DUPLICATE_NAME", Some(id), "duplicate entity identifier", self.revision)); }
        };
        for module in &self.modules { add_id(&module.entity_id); }
        for ty in &self.types { add_id(&ty.entity_id); }
        for function in &self.functions {
            add_id(&function.entity_id);
            for parameter in &function.parameters { add_id(&parameter.entity_id); }
            for block in &function.blocks {
                add_id(&block.entity_id);
                for argument in &block.arguments { add_id(&argument.entity_id); }
                for operation in block.operations.iter().chain(std::iter::once(&block.terminator)) {
                    add_id(&operation.entity_id);
                    for value in &operation.outputs { add_id(&value.entity_id); }
                }
            }
        }
        for capability in &self.capabilities { add_id(&capability.entity_id); }
        for package in &self.packages { add_id(&package.entity_id); }
        for contract in &self.contracts { add_id(&contract.entity_id); }
        drop(add_id);
        let mut error = |code: &str, entity: &str, cause: String| diagnostics.push(Diagnostic::error(code, Some(entity), cause, self.revision));
        if self.project_id != "il" || self.graph_version != GRAPH_VERSION || self.target != TARGET {
            error("E_SCHEMA_INVALID", "graph", "unsupported project, graph version or target".into());
        }
        let module_ids: BTreeSet<_> = self.modules.iter().map(|m| m.entity_id.as_str()).collect();
        let declaration_ids: BTreeSet<_> = self.types.iter().map(|t| t.entity_id.as_str()).chain(self.functions.iter().map(|f| f.entity_id.as_str())).collect();
        let type_ids: BTreeSet<_> = self.types.iter().map(|t| t.entity_id.as_str()).collect();
        let function_ids: BTreeSet<_> = self.functions.iter().map(|f| f.entity_id.as_str()).collect();
        let capability_ids: BTreeSet<_> = self.capabilities.iter().map(|c| c.entity_id.as_str()).collect();
        let contract_ids: BTreeSet<_> = self.contracts.iter().map(|c| c.entity_id.as_str()).collect();
        let type_exists = |name: &str| is_builtin_type(name) || type_ids.contains(name);
        let mut paths = BTreeSet::new();
        let mut declared = BTreeSet::new();
        for module in &self.modules {
            if !valid_id(&module.path) || !paths.insert(&module.path) { error("E_DUPLICATE_NAME", &module.entity_id, "invalid or duplicate module path".into()); }
            for (references, expected, label) in [(&module.imports, &module_ids, "module import"), (&module.declarations, &declaration_ids, "declaration")] {
                let mut seen = BTreeSet::new();
                for reference in references {
                    if !expected.contains(reference.as_str()) { error("E_NAME_NOT_FOUND", &module.entity_id, format!("dangling {label}: {reference}")); }
                    if !seen.insert(reference) { error("E_DUPLICATE_NAME", &module.entity_id, format!("duplicate {label}: {reference}")); }
                }
            }
            for declaration in &module.declarations {
                if !declared.insert(declaration) { error("E_DUPLICATE_NAME", &module.entity_id, format!("declaration belongs to multiple modules: {declaration}")); }
            }
        }
        for ty in &self.types {
            for reference in ty.parameters.iter().chain(ty.fields.iter().map(|field| &field.type_ref)).chain(ty.variants.iter().flat_map(|variant| &variant.fields)) {
                if !type_exists(reference) { error("E_NAME_NOT_FOUND", &ty.entity_id, format!("dangling type: {reference}")); }
            }
            if let Some(integer) = &ty.integer {
                if ty.kind != TypeKind::Int || !matches!(integer.bits, 8 | 16 | 32 | 64) { error("E_SCHEMA_INVALID", &ty.entity_id, "invalid integer layout".into()); }
            }
            if (ty.kind == TypeKind::Int) != ty.integer.is_some() || (!ty.fields.is_empty() && ty.kind != TypeKind::Record) || (!ty.variants.is_empty() && ty.kind != TypeKind::Sum) {
                error("E_SCHEMA_INVALID", &ty.entity_id, "type payload does not match kind".into());
            }
            if (ty.kind == TypeKind::Option && ty.parameters.len() != 1) || (ty.kind == TypeKind::Result && ty.parameters.len() != 2) {
                error("E_SCHEMA_INVALID", &ty.entity_id, "incorrect parameterized type arity".into());
            }
            let mut names = BTreeSet::new();
            for name in ty.fields.iter().map(|f| &f.name).chain(ty.variants.iter().map(|v| &v.name)) {
                if !valid_id(name) || !names.insert(name) { error("E_DUPLICATE_NAME", &ty.entity_id, "invalid or duplicate type member name".into()); }
            }
        }
        for capability in &self.capabilities {
            if (capability.kind == CapabilityKind::ClockRead) != capability.scope.is_none() || capability.scope.as_ref().is_some_and(|s| s.is_empty()) {
                error("E_SCHEMA_INVALID", &capability.entity_id, "capability scope does not match kind".into());
            }
        }
        for package in &self.packages {
            if package.name.is_empty() || package.version.is_empty() {
                error("E_SCHEMA_INVALID", &package.entity_id, "package name and version must be nonempty".into());
            }
            if has_duplicates(&package.modules) || has_duplicates(&package.effects) || has_duplicates(&package.capabilities) {
                error("E_SCHEMA_INVALID", &package.entity_id, "package modules, effects and capabilities must contain unique entries".into());
            }
            for reference in &package.modules { if !module_ids.contains(reference.as_str()) { error("E_NAME_NOT_FOUND", &package.entity_id, format!("dangling package module: {reference}")); } }
            for reference in &package.capabilities { if !capability_ids.contains(reference.as_str()) { error("E_NAME_NOT_FOUND", &package.entity_id, format!("dangling capability: {reference}")); } }
        }
        for contract in &self.contracts {
            if !declaration_ids.contains(contract.subject.as_str()) { error("E_NAME_NOT_FOUND", &contract.entity_id, "dangling contract subject".into()); }
            for predicate in &contract.predicates {
                match predicate {
                    ContractPredicate::Returns { type_ref } if !type_exists(type_ref) => error("E_NAME_NOT_FOUND", &contract.entity_id, "dangling contract type".into()),
                    ContractPredicate::RequiresCapability { capability } if !capability_ids.contains(capability.as_str()) => error("E_NAME_NOT_FOUND", &contract.entity_id, "dangling contract capability".into()),
                    _ => {}
                }
            }
        }
        for function in &self.functions {
            if !valid_id(&function.name) { error("E_SCHEMA_INVALID", &function.entity_id, "invalid function name".into()); }
            for parameter in &function.parameters {
                if !valid_id(&parameter.name) { error("E_SCHEMA_INVALID", &parameter.entity_id, "invalid parameter name".into()); }
            }
            if has_duplicates(&function.effects) || has_duplicates(&function.capabilities) || has_duplicates(&function.contracts) {
                error("E_SCHEMA_INVALID", &function.entity_id, "function effects, capabilities and contracts must contain unique entries".into());
            }
            for reference in function.parameters.iter().map(|p| &p.type_ref).chain(std::iter::once(&function.result)) {
                if !type_exists(reference) { error("E_NAME_NOT_FOUND", &function.entity_id, format!("dangling signature type: {reference}")); }
            }
            for reference in &function.capabilities { if !capability_ids.contains(reference.as_str()) { error("E_NAME_NOT_FOUND", &function.entity_id, format!("dangling capability: {reference}")); } }
            for reference in &function.contracts { if !contract_ids.contains(reference.as_str()) { error("E_NAME_NOT_FOUND", &function.entity_id, format!("dangling contract: {reference}")); } }
            if function.blocks.is_empty() { error("E_MISSING_RETURN", &function.entity_id, "function has no entry block".into()); }
            let blocks: BTreeSet<_> = function.blocks.iter().map(|b| b.entity_id.as_str()).collect();
            for block in &function.blocks {
                let mut values: BTreeSet<_> = function.parameters.iter().map(|p| p.entity_id.as_str()).chain(block.arguments.iter().map(|v| v.entity_id.as_str())).collect();
                for argument in &block.arguments { if !type_exists(&argument.type_ref) { error("E_NAME_NOT_FOUND", &argument.entity_id, "dangling block argument type".into()); } }
                for (operation, is_final) in block.operations.iter().map(|op| (op, false)).chain(std::iter::once((&block.terminator, true))) {
                    if operation.opcode.is_terminator() != is_final { error("E_SCHEMA_INVALID", &operation.entity_id, "terminator must appear exactly once at block end".into()); }
                    if !operation.attributes_match() { error("E_SCHEMA_INVALID", &operation.entity_id, "opcode attributes do not match closed schema".into()); }
                    if has_duplicates(&operation.effects) || has_duplicates(&operation.consumes) {
                        error("E_SCHEMA_INVALID", &operation.entity_id, "operation effects and consumed values must contain unique entries".into());
                    }
                    for reference in operation.inputs.iter().chain(&operation.consumes).chain(operation.branch_arguments()) {
                        if !values.contains(reference.as_str()) { error("E_NAME_NOT_FOUND", &operation.entity_id, format!("value used before local definition: {reference}")); }
                    }
                    for consumed in &operation.consumes {
                        if !operation.inputs.contains(consumed) && !operation.branch_arguments().contains(&consumed) {
                            error("E_SCHEMA_INVALID", &operation.entity_id, "consumed value must be an input or an explicit branch argument".into());
                        }
                    }
                    for produced in &operation.produces { if !operation.outputs.contains(produced) { error("E_SCHEMA_INVALID", &operation.entity_id, "produced owned value must be an output".into()); } }
                    for target in operation.branch_targets() { if !blocks.contains(target.as_str()) { error("E_NAME_NOT_FOUND", &operation.entity_id, format!("dangling block target: {target}")); } }
                    match &operation.attributes {
                        Attributes::Call { callee } if !function_ids.contains(callee.as_str()) => error("E_NAME_NOT_FOUND", &operation.entity_id, format!("dangling callee: {callee}")),
                        Attributes::Cast { target_type } if !type_exists(target_type) => error("E_NAME_NOT_FOUND", &operation.entity_id, "dangling cast target type".into()),
                        Attributes::Record { type_id } | Attributes::Variant { type_id, .. } if !type_exists(type_id) => error("E_NAME_NOT_FOUND", &operation.entity_id, "dangling constructed type".into()),
                        _ => {}
                    }
                    for output in &operation.outputs {
                        if !type_exists(&output.type_ref) { error("E_NAME_NOT_FOUND", &output.entity_id, "dangling value type".into()); }
                        values.insert(&output.entity_id);
                    }
                }
            }
        }
        drop(error);
        // MVP has no pointer-bearing user type; all cycles through value fields are illegal.
        let edges: BTreeMap<&str, Vec<&str>> = self.types.iter().map(|ty| (ty.entity_id.as_str(), ty.parameters.iter().map(String::as_str).chain(ty.fields.iter().map(|f| f.type_ref.as_str())).chain(ty.variants.iter().flat_map(|v| v.fields.iter().map(String::as_str))).filter(|name| type_ids.contains(name)).collect())).collect();
        let mut colors = BTreeMap::<&str, u8>::new();
        for id in edges.keys() {
            if colors.contains_key(id) { continue; }
            let mut stack = vec![(*id, false)];
            while let Some((node, leaving)) = stack.pop() {
                if leaving { colors.insert(node, 2); continue; }
                match colors.get(node) {
                    Some(1) => {
                        diagnostics.push(Diagnostic::error("E_SCHEMA_INVALID", Some(node), "type cycle without indirection", self.revision));
                        continue;
                    }
                    Some(2) => continue,
                    _ => {}
                }
                colors.insert(node, 1);
                stack.push((node, true));
                if let Some(children) = edges.get(node) {
                    for child in children.iter().rev() { stack.push((child, false)); }
                }
            }
        }
        diagnostics
    }
}

impl Operation {
    pub fn attributes_match(&self) -> bool {
        let fields_valid = match &self.attributes {
            Attributes::Call { callee } => valid_id(callee),
            Attributes::RuntimeCall { symbol, capability } => !symbol.is_empty() && capability.as_ref().is_none_or(|id| valid_id(id)),
            Attributes::Cast { target_type } => !target_type.is_empty(),
            Attributes::Record { type_id } => !type_id.is_empty(),
            Attributes::Field { field } => valid_id(field),
            Attributes::Variant { type_id, variant } => !type_id.is_empty() && valid_id(variant),
            Attributes::Branch { target } => valid_id(target),
            Attributes::CondBranch { then_block, else_block, then_arguments, else_arguments } =>
                valid_id(then_block) && valid_id(else_block) && then_arguments.iter().chain(else_arguments).all(|id| valid_id(id)),
            Attributes::Switch { cases, default, default_arguments } => valid_id(default)
                && default_arguments.iter().all(|id| valid_id(id))
                && cases.iter().all(|case| !case.tag.is_empty() && valid_id(&case.target) && case.arguments.iter().all(|id| valid_id(id))),
            Attributes::Trap { code } => !code.is_empty(),
            Attributes::Constant { .. } | Attributes::TupleGet { .. } | Attributes::Empty {} => true,
        };
        if !fields_valid { return false; }
        match (&self.opcode, &self.attributes) {
            (Opcode::Const, Attributes::Constant { .. }) | (Opcode::Call, Attributes::Call { .. }) |
            (Opcode::RuntimeCall, Attributes::RuntimeCall { .. }) | (Opcode::Cast, Attributes::Cast { .. }) |
            (Opcode::Record, Attributes::Record { .. }) | (Opcode::Field, Attributes::Field { .. }) |
            (Opcode::TupleGet, Attributes::TupleGet { .. }) | (Opcode::Variant, Attributes::Variant { .. }) |
            (Opcode::Branch, Attributes::Branch { .. }) | (Opcode::CondBranch, Attributes::CondBranch { .. }) |
            (Opcode::Switch, Attributes::Switch { .. }) | (Opcode::Trap, Attributes::Trap { .. }) => true,
            (opcode, Attributes::Empty {}) => !matches!(opcode, Opcode::Const | Opcode::Call | Opcode::RuntimeCall | Opcode::Cast | Opcode::Record | Opcode::Field | Opcode::TupleGet | Opcode::Variant | Opcode::Branch | Opcode::CondBranch | Opcode::Switch | Opcode::Trap),
            _ => false,
        }
    }

    pub fn branch_targets(&self) -> Vec<&String> {
        match &self.attributes {
            Attributes::Branch { target } => vec![target],
            Attributes::CondBranch { then_block, else_block, .. } => vec![then_block, else_block],
            Attributes::Switch { cases, default, .. } => cases.iter().map(|case| &case.target).chain(std::iter::once(default)).collect(),
            _ => vec![],
        }
    }

    pub fn branch_arguments(&self) -> Vec<&String> {
        match &self.attributes {
            Attributes::CondBranch { then_arguments, else_arguments, .. } => then_arguments.iter().chain(else_arguments).collect(),
            Attributes::Switch { cases, default_arguments, .. } => cases.iter().flat_map(|case| &case.arguments).chain(default_arguments).collect(),
            _ => vec![],
        }
    }
}
