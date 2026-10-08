//! Shared static checking for graph and text frontends.
pub mod runtime;
mod types;
mod ownership;

pub use runtime::*;
pub use types::is_owned_type;

use il_graph::*;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use types::Types;

pub fn check(graph: &Graph) -> Vec<Diagnostic> { check_with_capabilities(graph, &[]) }

pub fn check_with_capabilities(graph: &Graph, injected: &[Capability]) -> Vec<Diagnostic> {
    let node_count = graph.types.len() + graph.functions.iter().map(|function| function.blocks.iter().map(|block| block.operations.len() + block.arguments.len() + 2).sum::<usize>()).sum::<usize>();
    if graph.types.len() > 256 || node_count > 100_000 {
        return vec![Diagnostic::error("E_RESOURCE_LIMIT", None, "static checking budget exceeded", graph.revision)];
    }
    let structural = graph.validate_structural();
    if !structural.is_empty() { return structural; }
    let owners = graph.modules.iter().flat_map(|module| module.declarations.iter().map(move |declaration| (declaration.as_str(), module))).collect();
    let context = Context { graph, types: Types::new(graph), owners, injected, diagnostics: RefCell::new(vec![]) };
    context.declarations();
    for function in &graph.functions { context.function(function); }
    let mut diagnostics = context.diagnostics.into_inner();
    let mut seen = BTreeSet::new();
    diagnostics.retain(|diagnostic| seen.insert(diagnostic.diagnostic_id.clone()));
    diagnostics
}

struct Context<'a> {
    graph: &'a Graph,
    types: Types<'a>,
    owners: BTreeMap<&'a str, &'a Module>,
    injected: &'a [Capability],
    diagnostics: RefCell<Vec<Diagnostic>>,
}

#[derive(Clone, Debug)]
struct Facts {
    moved: BTreeSet<String>,
    effects: BTreeSet<Effect>,
    borrowed_output: bool,
    shared_inputs: BTreeSet<String>,
}

impl Facts {
    fn new() -> Self { Self { moved: BTreeSet::new(), effects: BTreeSet::new(), borrowed_output: false, shared_inputs: BTreeSet::new() } }
}

impl Context<'_> {
    fn error(&self, code: &str, entity: &str, message: impl Into<String>) {
        let mut diagnostic = Diagnostic::error(code, Some(entity), message, self.graph.revision);
        diagnostic.stage = match code {
            "E_EFFECT_UNDECLARED" => "effect_check", "E_CAPABILITY_MISSING" => "capability_check",
            "E_USE_AFTER_MOVE" | "E_BORROW_ESCAPE" | "E_DOUBLE_DROP" | "E_BORROW_CONFLICT" | "E_OWNERSHIP_JOIN" => "ownership_check",
            _ => "type_check",
        }.into();
        self.diagnostics.borrow_mut().push(diagnostic);
    }

    fn declarations(&self) {
        for module in &self.graph.modules {
            for imported in &module.imports {
                if imported != &module.entity_id && self.graph.modules.iter().find(|candidate| &candidate.entity_id == imported).is_some_and(|candidate| candidate.visibility != Visibility::Public) {
                    self.error("E_NAME_NOT_FOUND", &module.entity_id, format!("imported module is private: {imported}"));
                }
            }
            let mut names = BTreeSet::new();
            for declaration in &module.declarations {
                if let Some(function) = self.graph.functions.iter().find(|function| &function.entity_id == declaration) {
                    if !names.insert(function.name.as_str()) { self.error("E_DUPLICATE_NAME", &function.entity_id, "duplicate function name in module"); }
                }
            }
        }
        for ty in &self.graph.types {
            if !self.owners.contains_key(ty.entity_id.as_str()) { self.error("E_NAME_NOT_FOUND", &ty.entity_id, "type declaration has no owning module"); }
            for reference in ty.parameters.iter().chain(ty.fields.iter().map(|field| &field.type_ref)).chain(ty.variants.iter().flat_map(|variant| &variant.fields)) {
                self.resolve_reference(&ty.entity_id, reference, true);
            }
            if is_builtin_type(&ty.entity_id) { self.error("E_DUPLICATE_NAME", &ty.entity_id, "builtin type names cannot be redefined"); }
            if ty.kind == TypeKind::Sum && ty.variants.is_empty() { self.error("E_SCHEMA_INVALID", &ty.entity_id, "sum requires at least one variant"); }
            if !matches!(ty.kind, TypeKind::Option | TypeKind::Result | TypeKind::Tuple) && !ty.parameters.is_empty() {
                self.error("E_UNSUPPORTED_FEATURE", &ty.entity_id, "user type parameters require E01 generics");
            }
            if ty.kind != TypeKind::Record && !ty.fields.is_empty() || ty.kind != TypeKind::Sum && !ty.variants.is_empty() {
                self.error("E_SCHEMA_INVALID", &ty.entity_id, "type members do not match its kind");
            }
        }
        for capability in &self.graph.capabilities {
            if !self.injected.iter().any(|injected| injected == capability) {
                self.error("E_CAPABILITY_MISSING", &capability.entity_id, "capability declaration has no identical trusted host injection");
            }
        }
        let mut injection_ids = BTreeSet::new();
        for capability in self.injected {
            if !injection_ids.insert(&capability.entity_id) { self.error("E_CAPABILITY_MISSING", &capability.entity_id, "ambiguous duplicated host injection"); }
        }
        for contract in &self.graph.contracts {
            let Some(function) = self.graph.functions.iter().find(|function| function.entity_id == contract.subject) else {
                self.error("E_TYPE_MISMATCH", &contract.entity_id, "function contract attached to a nonfunction subject"); continue;
            };
            if !function.contracts.contains(&contract.entity_id) { self.error("E_SCHEMA_INVALID", &contract.entity_id, "contract must be attached by its subject function"); }
            for predicate in &contract.predicates {
                match predicate {
                    ContractPredicate::Returns { type_ref } if type_ref != &function.result => self.error("E_TYPE_MISMATCH", &contract.entity_id, "return contract differs from function signature"),
                    ContractPredicate::RequiresEffect { effect } if !function.effects.contains(effect) => self.error("E_EFFECT_UNDECLARED", &contract.entity_id, "contract effect absent from function declaration"),
                    ContractPredicate::RequiresCapability { capability } if !function.capabilities.contains(capability) => self.error("E_CAPABILITY_MISSING", &contract.entity_id, "contract capability absent from function declaration"),
                    _ => {}
                }
            }
        }
    }

    fn function(&self, function: &Function) {
        if !self.owners.contains_key(function.entity_id.as_str()) { self.error("E_NAME_NOT_FOUND", &function.entity_id, "function declaration has no owning module"); }
        self.resolve_reference(&function.entity_id, &function.result, true);
        let mut names = BTreeSet::new();
        for parameter in &function.parameters {
            self.resolve_reference(&function.entity_id, &parameter.type_ref, true);
            if !names.insert(&parameter.name) { self.error("E_DUPLICATE_NAME", &parameter.entity_id, "duplicate parameter name"); }
            if parameter.type_ref == "Never" { self.error("E_TYPE_MISMATCH", &parameter.entity_id, "Never cannot be a parameter value"); }
        }
        self.unique(&function.effects, &function.entity_id, "duplicate declared effect");
        self.unique(&function.capabilities, &function.entity_id, "duplicate required capability");
        self.unique(&function.contracts, &function.entity_id, "duplicate function contract");
        for contract_id in &function.contracts {
            if self.graph.contracts.iter().find(|contract| &contract.entity_id == contract_id).is_some_and(|contract| contract.subject != function.entity_id) {
                self.error("E_TYPE_MISMATCH", &function.entity_id, "function cannot attach a contract belonging to another subject");
            }
        }
        if let Some(entry) = function.blocks.first() {
            if !entry.arguments.is_empty() { self.error("E_TYPE_MISMATCH", &entry.entity_id, "entry block takes parameters from the function, not block arguments"); }
        }
        let mut facts = BTreeMap::new();
        for block in &function.blocks {
            for argument in &block.arguments { self.resolve_reference(&function.entity_id, &argument.type_ref, true); }
            let mut values: BTreeMap<String, String> = function.parameters.iter().map(|parameter| (parameter.entity_id.clone(), parameter.type_ref.clone())).chain(block.arguments.iter().map(|value| (value.entity_id.clone(), value.type_ref.clone()))).collect();
            for operation in block.operations.iter().chain(std::iter::once(&block.terminator)) {
                let operation_facts = self.operation(function, operation, &values);
                for output in &operation.outputs {
                    self.resolve_reference(&function.entity_id, &output.type_ref, true);
                    if output.type_ref == "Never" { self.error("E_TYPE_MISMATCH", &output.entity_id, "Never cannot be materialized as a value"); }
                    values.insert(output.entity_id.clone(), output.type_ref.clone());
                }
                facts.insert(operation.entity_id.clone(), operation_facts);
            }
        }
        self.control_flow(function);
        ownership::check_function(self, function, &facts);
    }

    fn unique<T: PartialEq>(&self, values: &[T], entity: &str, message: &str) {
        if values.iter().enumerate().any(|(index, value)| values[..index].contains(value)) { self.error("E_SCHEMA_INVALID", entity, message); }
    }

    fn resolve_reference(&self, owner: &str, reference: &str, type_reference: bool) {
        if type_reference && is_builtin_type(reference) { return; }
        let Some(source) = self.owners.get(owner) else { return; };
        let Some(target) = self.owners.get(reference) else {
            self.error("E_NAME_NOT_FOUND", owner, format!("referenced declaration has no module: {reference}")); return;
        };
        if source.entity_id != target.entity_id && (target.visibility != Visibility::Public || !source.imports.contains(&target.entity_id)) {
            self.error("E_NAME_NOT_FOUND", owner, format!("cross-module reference requires a direct import of public module {}: {reference}", target.entity_id));
        }
    }

    fn signature(&self, operation: &Operation, values: &BTreeMap<String, String>, expected_inputs: &[String], expected_outputs: &[String]) {
        let actual_inputs: Vec<_> = operation.inputs.iter().filter_map(|id| values.get(id)).cloned().collect();
        let actual_outputs: Vec<_> = operation.outputs.iter().map(|value| value.type_ref.clone()).collect();
        if actual_inputs != expected_inputs || actual_outputs != expected_outputs {
            self.error("E_TYPE_MISMATCH", &operation.entity_id, format!("expected inputs {expected_inputs:?}, outputs {expected_outputs:?}; got {actual_inputs:?}, {actual_outputs:?}"));
        }
    }

    fn move_owned(&self, facts: &mut Facts, inputs: &[String], values: &BTreeMap<String, String>) {
        for input in inputs { if values.get(input).is_some_and(|ty| self.types.is_owned(ty)) { facts.moved.insert(input.clone()); } }
    }

    fn operation(&self, function: &Function, operation: &Operation, values: &BTreeMap<String, String>) -> Facts {
        let mut facts = Facts::new();
        let inputs: Vec<String> = operation.inputs.iter().filter_map(|id| values.get(id)).cloned().collect();
        let outputs: Vec<String> = operation.outputs.iter().map(|value| value.type_ref.clone()).collect();
        let one_in = inputs.first().map(String::as_str).unwrap_or("");
        let one_out = outputs.first().map(String::as_str).unwrap_or("");
        let empty: Vec<String> = vec![];
        match operation.opcode {
            Opcode::Const => {
                if inputs.len() != 0 || outputs.len() != 1 { self.error("E_TYPE_MISMATCH", &operation.entity_id, "const takes no inputs and produces one value"); }
                if let Attributes::Constant { value } = &operation.attributes {
                    if let Err(code) = self.types.literal_matches(value, one_out) { self.error(code, &operation.entity_id, "constant does not fit its explicitly declared type"); }
                }
                if self.types.definition(one_out).is_some_and(|ty| ty.layout == Layout::Opaque) { self.error("E_UNSUPPORTED_FEATURE", &operation.entity_id, "constant cannot construct an opaque resource"); }
                if self.types.is_owned(one_out) { facts.effects.insert(Effect::Alloc); }
            }
            Opcode::Add | Opcode::Sub | Opcode::Mul | Opcode::Div | Opcode::Rem | Opcode::Shl | Opcode::Shr | Opcode::BitAnd | Opcode::BitOr | Opcode::BitXor => {
                self.signature(operation, values, &[one_in.into(), one_in.into()], &[one_in.into()]);
                if self.types.integer(one_in).is_none() { self.error("E_TYPE_MISMATCH", &operation.entity_id, "integer operation requires a fixed-width integer"); }
            }
            Opcode::Eq | Opcode::Ne | Opcode::Lt | Opcode::Le | Opcode::Gt | Opcode::Ge => {
                self.signature(operation, values, &[one_in.into(), one_in.into()], &["Bool".into()]);
                if self.types.integer(one_in).is_none() && !(one_in == "Bool" && matches!(operation.opcode, Opcode::Eq | Opcode::Ne)) {
                    self.error("E_TYPE_MISMATCH", &operation.entity_id, "comparison requires integers, or Bool equality");
                }
            }
            Opcode::Not => {
                self.signature(operation, values, &[one_in.into()], &[one_in.into()]);
                if one_in != "Bool" && self.types.integer(one_in).is_none() { self.error("E_TYPE_MISMATCH", &operation.entity_id, "not requires Bool or an integer"); }
            }
            Opcode::Cast => {
                if let Attributes::Cast { target_type } = &operation.attributes {
                    self.signature(operation, values, &[one_in.into()], std::slice::from_ref(target_type));
                    if self.types.integer(one_in).is_none() || self.types.integer(target_type).is_none() { self.error("E_TYPE_MISMATCH", &operation.entity_id, "cast only converts explicit integer types"); }
                    if self.types.definition(target_type).is_some_and(|ty| ty.layout == Layout::Opaque) { self.error("E_UNSUPPORTED_FEATURE", &operation.entity_id, "cast cannot construct an opaque resource"); }
                }
            }
            Opcode::Call => {
                if let Attributes::Call { callee } = &operation.attributes {
                    self.resolve_reference(&function.entity_id, callee, false);
                    if let Some(callee) = self.graph.functions.iter().find(|item| &item.entity_id == callee) {
                        let expected_outputs = if callee.result == "Unit" { vec![] } else { vec![callee.result.clone()] };
                        self.signature(operation, values, &callee.parameters.iter().map(|parameter| parameter.type_ref.clone()).collect::<Vec<_>>(), &expected_outputs);
                        facts.effects.extend(&callee.effects);
                        self.move_owned(&mut facts, &operation.inputs, values);
                        for capability in &callee.capabilities {
                            if !function.capabilities.contains(capability) { self.error("E_CAPABILITY_MISSING", &operation.entity_id, "caller does not inject callee's required capability"); }
                        }
                    }
                }
            }
            Opcode::RuntimeCall => self.runtime_call(function, operation, values, &mut facts),
            Opcode::Record => {
                if let Attributes::Record { type_id } = &operation.attributes {
                    match self.types.definition(type_id) {
                        Some(ty) if ty.kind == TypeKind::Record && ty.layout != Layout::Opaque => {
                            self.signature(operation, values, &ty.fields.iter().map(|field| field.type_ref.clone()).collect::<Vec<_>>(), std::slice::from_ref(type_id));
                            self.move_owned(&mut facts, &operation.inputs, values);
                        }
                        _ => self.error("E_TYPE_MISMATCH", &operation.entity_id, "record constructor requires a concrete nominal record"),
                    }
                }
            }
            Opcode::Tuple => {
                match self.types.definition(one_out) {
                    Some(ty) if ty.kind == TypeKind::Tuple && ty.layout != Layout::Opaque => self.signature(operation, values, &ty.parameters, &[one_out.into()]),
                    _ => self.error("E_TYPE_MISMATCH", &operation.entity_id, "tuple constructor output must name a tuple type"),
                }
                self.move_owned(&mut facts, &operation.inputs, values);
            }
            Opcode::Field | Opcode::TupleGet => {
                if self.types.definition(one_in).is_some_and(|ty| ty.layout == Layout::Opaque) { self.error("E_UNSUPPORTED_FEATURE", &operation.entity_id, "opaque resource fields cannot be projected"); }
                let projection = match (&operation.attributes, self.types.definition(one_in)) {
                    (Attributes::Field { field }, Some(ty)) if ty.kind == TypeKind::Record => ty.fields.iter().find(|item| &item.name == field).map(|item| item.type_ref.clone()),
                    (Attributes::TupleGet { index }, Some(ty)) if ty.kind == TypeKind::Tuple => ty.parameters.get(*index as usize).cloned(),
                    _ => None,
                };
                if let Some(result) = projection {
                    self.signature(operation, values, &[one_in.into()], std::slice::from_ref(&result));
                    if self.types.is_owned(&result) { self.error("E_UNSUPPORTED_FEATURE", &operation.entity_id, "owned field partial moves are forbidden; use an explicit whole-value operation"); }
                } else { self.error("E_TYPE_MISMATCH", &operation.entity_id, "unknown field or invalid tuple index"); }
                facts.shared_inputs.extend(operation.inputs.iter().cloned());
            }
            Opcode::Variant => {
                if let Attributes::Variant { type_id, variant } = &operation.attributes {
                    if self.types.definition(type_id).is_some_and(|ty| ty.layout == Layout::Opaque) { self.error("E_UNSUPPORTED_FEATURE", &operation.entity_id, "variant construction cannot forge an opaque resource"); }
                    match self.types.variants(type_id).and_then(|variants| variants.into_iter().find(|(name, _)| name == variant)) {
                        Some((_, fields)) => self.signature(operation, values, &fields, std::slice::from_ref(type_id)),
                        None => self.error("E_TYPE_MISMATCH", &operation.entity_id, "variant constructor names no declared variant"),
                    }
                    self.move_owned(&mut facts, &operation.inputs, values);
                }
            }
            Opcode::Tag => {
                if self.types.definition(one_in).is_some_and(|ty| ty.layout == Layout::Opaque) { self.error("E_UNSUPPORTED_FEATURE", &operation.entity_id, "opaque resource discriminant cannot be inspected"); }
                self.signature(operation, values, &[one_in.into()], &["U32".into()]);
                if self.types.variants(one_in).is_none() { self.error("E_TYPE_MISMATCH", &operation.entity_id, "tag requires Sum, Option or Result"); }
                facts.shared_inputs.extend(operation.inputs.iter().cloned());
            }
            Opcode::Payload => {
                if self.types.definition(one_in).is_some_and(|ty| ty.layout == Layout::Opaque) { self.error("E_UNSUPPORTED_FEATURE", &operation.entity_id, "opaque resource payload cannot be extracted"); }
                if inputs.len() != 1 || self.types.variants(one_in).is_none() { self.error("E_TYPE_MISMATCH", &operation.entity_id, "payload requires one discriminant-refined sum value"); }
                self.move_owned(&mut facts, &operation.inputs, values);
                // The exact payload signature is validated using each CFG path's refinement.
            }
            Opcode::Move | Opcode::Clone | Opcode::Borrow | Opcode::BorrowMut => {
                self.signature(operation, values, &[one_in.into()], &[one_in.into()]);
                match operation.opcode {
                    Opcode::Move => self.move_owned(&mut facts, &operation.inputs, values),
                    Opcode::Clone => { if self.types.is_owned(one_in) { facts.effects.insert(Effect::Alloc); } facts.shared_inputs.extend(operation.inputs.iter().cloned()); }
                    _ => { facts.borrowed_output = true; facts.shared_inputs.extend(operation.inputs.iter().cloned()); }
                }
                if operation.opcode == Opcode::Clone && self.types.contains_opaque(one_in) {
                    self.error("E_UNSUPPORTED_FEATURE", &operation.entity_id, "opaque resources need an explicit declared duplication operation");
                }
            }
            Opcode::EndBorrow | Opcode::Drop => {
                self.signature(operation, values, &[one_in.into()], &empty);
                if operation.opcode == Opcode::Drop { self.move_owned(&mut facts, &operation.inputs, values); }
            }
            Opcode::Branch => {
                if !outputs.is_empty() { self.error("E_TYPE_MISMATCH", &operation.entity_id, "branch cannot produce values"); }
                if let Attributes::Branch { target } = &operation.attributes { self.edge_signature(function, operation, target, &operation.inputs, values); }
                self.move_owned(&mut facts, &operation.inputs, values);
            }
            Opcode::CondBranch => {
                self.signature(operation, values, &["Bool".into()], &empty);
                if let Attributes::CondBranch { then_block, else_block, then_arguments, else_arguments } = &operation.attributes {
                    self.edge_signature(function, operation, then_block, then_arguments, values);
                    self.edge_signature(function, operation, else_block, else_arguments, values);
                    self.move_owned(&mut facts, then_arguments, values);
                    self.move_owned(&mut facts, else_arguments, values);
                }
            }
            Opcode::Switch => {
                if self.types.definition(one_in).is_some_and(|ty| ty.layout == Layout::Opaque) { self.error("E_UNSUPPORTED_FEATURE", &operation.entity_id, "opaque resource discriminant cannot be switched"); }
                self.signature(operation, values, &[one_in.into()], &empty);
                if let Attributes::Switch { cases, default, default_arguments } = &operation.attributes {
                    let tags: Vec<_> = cases.iter().map(|case| case.tag.clone()).collect();
                    self.unique(&tags, &operation.entity_id, "duplicate switch tag");
                    match self.types.variants(one_in) {
                        Some(variants) => {
                            let expected: BTreeSet<_> = variants.iter().map(|(tag, _)| tag).collect();
                            let actual: BTreeSet<_> = tags.iter().collect();
                            if actual != expected { self.error("E_NON_EXHAUSTIVE_MATCH", &operation.entity_id, "switch must enumerate every variant exactly"); }
                        }
                        None => self.error("E_TYPE_MISMATCH", &operation.entity_id, "switch requires Sum, Option or Result"),
                    }
                    for case in cases {
                        self.edge_signature(function, operation, &case.target, &case.arguments, values);
                        self.move_owned(&mut facts, &case.arguments, values);
                    }
                    self.edge_signature(function, operation, default, default_arguments, values);
                    self.move_owned(&mut facts, default_arguments, values);
                    if !function.blocks.iter().find(|block| &block.entity_id == default).is_some_and(|block| block.terminator.opcode == Opcode::Trap) {
                        self.error("E_NON_EXHAUSTIVE_MATCH", &operation.entity_id, "invalid-discriminant default must terminate with trap");
                    }
                }
                facts.shared_inputs.extend(operation.inputs.iter().cloned());
            }
            Opcode::Return => {
                let expected = if function.result == "Unit" { vec![] } else { vec![function.result.clone()] };
                self.signature(operation, values, &expected, &empty);
                if function.result == "Never" { self.error("E_TYPE_MISMATCH", &operation.entity_id, "Never function cannot return"); }
                self.move_owned(&mut facts, &operation.inputs, values);
            }
            Opcode::Trap => self.signature(operation, values, &empty, &empty),
        }
        self.unique(&operation.effects, &operation.entity_id, "duplicate operation effect");
        self.unique(&operation.consumes, &operation.entity_id, "duplicate consumed value");
        let declared_effects: BTreeSet<_> = operation.effects.iter().copied().collect();
        if declared_effects != facts.effects { self.error("E_EFFECT_UNDECLARED", &operation.entity_id, "operation effects differ from inferred requirements"); }
        if facts.effects.iter().any(|effect| !function.effects.contains(effect)) { self.error("E_EFFECT_UNDECLARED", &operation.entity_id, "function does not declare all required operation effects"); }
        let declared_consumes: BTreeSet<_> = operation.consumes.iter().cloned().collect();
        if declared_consumes != facts.moved { self.error("E_OWNERSHIP_METADATA", &operation.entity_id, "consumes metadata differs from inferred owned transfers"); }
        let expected_produces: Vec<_> = operation.outputs.iter().filter(|output| !facts.borrowed_output && self.types.is_owned(&output.type_ref)).cloned().collect();
        if operation.produces != expected_produces { self.error("E_OWNERSHIP_METADATA", &operation.entity_id, "produces metadata differs from inferred owned outputs"); }
        facts
    }

    fn edge_signature(&self, function: &Function, operation: &Operation, target: &str, arguments: &[String], values: &BTreeMap<String, String>) {
        if let Some(block) = function.blocks.iter().find(|block| block.entity_id == target) {
            let expected: Vec<_> = block.arguments.iter().map(|argument| argument.type_ref.as_str()).collect();
            let actual: Vec<_> = arguments.iter().filter_map(|id| values.get(id).map(String::as_str)).collect();
            if expected != actual { self.error("E_TYPE_MISMATCH", &operation.entity_id, "branch arguments differ from target block parameters"); }
        }
    }

    fn runtime_call(&self, function: &Function, operation: &Operation, values: &BTreeMap<String, String>, facts: &mut Facts) {
        let Attributes::RuntimeCall { symbol } = &operation.attributes else { return; };
        let Some(signature) = runtime_signature(symbol) else { self.error("E_UNSUPPORTED_FEATURE", &operation.entity_id, "runtime symbol has no implemented static ABI contract"); return; };
        let output = match signature.result {
            RuntimeResult::Exact(name) => if name == "Unit" { vec![] } else { vec![name.into()] },
            RuntimeResult::Result { success, error } => {
                self.runtime_error_shape(error, &operation.entity_id);
                let actual = operation.outputs.first().map(|output| output.type_ref.as_str()).unwrap_or("");
                if !self.types.definition(actual).is_some_and(|ty| ty.kind == TypeKind::Result && ty.layout == Layout::Inferred && ty.parameters == [success, error]) {
                    self.error("E_TYPE_MISMATCH", &operation.entity_id, format!("runtime requires Result<{success},{error}> output"));
                }
                vec![actual.into()]
            }
        };
        self.signature(operation, values, &signature.parameters.iter().map(|parameter| parameter.type_ref.into()).collect::<Vec<_>>(), &output);
        facts.effects.extend(signature.effects);
        for (input, parameter) in operation.inputs.iter().zip(signature.parameters) {
            if parameter.passing == Passing::Owned { self.move_owned(facts, std::slice::from_ref(input), values); }
            if parameter.passing == Passing::Shared { facts.shared_inputs.insert(input.clone()); }
        }
        if let Some(kind) = signature.capability {
            let granted = function.capabilities.iter().any(|id| self.graph.capabilities.iter().any(|capability| &capability.entity_id == id && capability.kind == kind && self.injected.contains(capability)));
            if !granted { self.error("E_CAPABILITY_MISSING", &operation.entity_id, "runtime call lacks required trusted host capability"); }
        }
    }

    fn runtime_error_shape(&self, name: &str, entity: &str) {
        let Some(tags) = runtime_error_variants(name) else { return; };
        let valid = self.types.definition(name).is_some_and(|ty| ty.kind == TypeKind::Sum && ty.layout == Layout::Inferred && ty.parameters.is_empty() && ty.fields.is_empty() && ty.variants.len() == tags.len() && ty.variants.iter().zip(tags).all(|(variant, name)| variant.name == *name && variant.fields.is_empty()));
        if !valid { self.error("E_TYPE_MISMATCH", entity, format!("runtime error type {name} does not match its locked ABI variants")); }
    }

    fn control_flow(&self, function: &Function) {
        let Some(entry) = function.blocks.first() else { return; };
        let blocks: BTreeMap<_, _> = function.blocks.iter().map(|block| (block.entity_id.as_str(), block)).collect();
        let mut reachable = BTreeSet::new();
        let mut pending = VecDeque::from([entry.entity_id.as_str()]);
        while let Some(id) = pending.pop_front() {
            if !reachable.insert(id) { continue; }
            if let Some(block) = blocks.get(id) { pending.extend(block.terminator.branch_targets().into_iter().map(String::as_str)); }
        }
        let mut can_exit: BTreeSet<_> = function.blocks.iter().filter(|block| matches!(block.terminator.opcode, Opcode::Return | Opcode::Trap)).map(|block| block.entity_id.as_str()).collect();
        loop {
            let previous = can_exit.len();
            for block in &function.blocks {
                if block.terminator.branch_targets().iter().any(|target| can_exit.contains(target.as_str())) { can_exit.insert(block.entity_id.as_str()); }
            }
            if can_exit.len() == previous { break; }
        }
        if function.result != "Never" && reachable.iter().any(|id| !can_exit.contains(id)) {
            self.error("E_MISSING_RETURN", &function.entity_id, "reachable control flow has no path to return or trap");
        }
        for block in &function.blocks {
            if !reachable.contains(block.entity_id.as_str()) { self.error("E_UNREACHABLE_BLOCK", &block.entity_id, "block is not reachable from function entry"); }
        }
    }
}
