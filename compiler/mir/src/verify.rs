use crate::*;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub fn verify(program: &Program) -> Vec<Diagnostic> { verify_with_capabilities(program, &[]) }

pub fn verify_with_capabilities(program: &Program, injected: &[Capability]) -> Vec<Diagnostic> {
    let mut diagnostics = vec![];
    let mut report = |code: &str, entity: &str, message: &str| {
        let mut diagnostic = Diagnostic::error(code, Some(entity), message, program.source_revision);
        diagnostic.stage = "verify_mir".into();
        diagnostics.push(diagnostic);
    };
    let nodes = program.functions.iter().map(|function| function.blocks.iter().map(|block| block.operations.len() + block.arguments.len() + 2).sum::<usize>()).sum::<usize>();
    if program.types.len() > 256 || nodes > 100_000 {
        report("E_RESOURCE_LIMIT", "mir", "MIR verification budget exceeded"); return diagnostics;
    }
    if program.schema_version != "1.0.0" || program.compiler_version != env!("CARGO_PKG_VERSION") || program.target != TARGET ||
        program.input_hash.len() != 71 || !program.input_hash.starts_with("sha256:") || !program.input_hash[7..].bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()) {
        report("E_SCHEMA_INVALID", "mir", "invalid MIR version, target or input hash"); return diagnostics;
    }
    let mut public=BTreeSet::new();for id in &program.public_functions{if !public.insert(id)||!program.functions.iter().any(|function|&function.entity_id==id){report("E_SCHEMA_INVALID",id,"public function must be a unique declared function");}}
    for function in &program.functions {
        if !valid_id(&function.name) { report("E_SCHEMA_INVALID", &function.entity_id, "MIR function name must be a valid logical identifier"); }
        for block in &function.blocks {
            for operation in &block.operations {
                if operation.opcode.is_terminator() { report("E_SCHEMA_INVALID", &operation.entity_id, "MIR terminators cannot appear in operation lists"); }
            }
        }
    }
    if !diagnostics.is_empty() { return diagnostics; }
    // Construct a type-checking view exclusively from this MIR payload. This is
    // not the original graph: cleanup is verified independently below, and no
    // declaration in the payload is treated as a trusted capability injection.
    let graph = type_view(program);
    diagnostics.extend(il_checker::check_with_capabilities(&graph, injected));
    if !diagnostics.is_empty() {
        for diagnostic in &mut diagnostics { diagnostic.stage = "verify_mir".into(); }
        return diagnostics;
    }
    for function in &program.functions { verify_lifetimes(program, function, &graph, &mut diagnostics); }
    let mut seen = BTreeSet::new();
    diagnostics.retain(|diagnostic| seen.insert(diagnostic.diagnostic_id.clone()));
    diagnostics
}

fn owned(graph: &Graph, ty: &str) -> bool { il_checker::is_owned_type(graph, ty) }

fn type_view(program: &Program) -> Graph {
    let mut graph = Graph::empty();
    graph.revision = program.source_revision;
    graph.target = program.target.clone();
    graph.types = program.types.clone();
    graph.capabilities = program.capabilities.clone();
    for function in &program.functions {
        let blocks = function.blocks.iter().map(|block| {
            let types: BTreeMap<_, _> = function.parameters.iter().map(|value| (value.entity_id.clone(), value.type_ref.clone()))
                .chain(block.arguments.iter().map(|value| (value.entity_id.clone(), value.type_ref.clone())))
                .chain(block.operations.iter().flat_map(|operation| operation.outputs.iter().map(|value| (value.entity_id.clone(), value.type_ref.clone())))).collect();
            let (opcode, inputs, attributes, transfers) = match &block.terminator {
                Terminator::Return { value, .. } => (Opcode::Return, value.iter().cloned().collect::<Vec<_>>(), Attributes::Empty {}, value.iter().cloned().collect::<Vec<_>>()),
                Terminator::Trap { code, .. } => (Opcode::Trap, vec![], Attributes::Trap { code: code.clone() }, vec![]),
                Terminator::Branch { edge, .. } => (Opcode::Branch, edge.arguments.clone(), Attributes::Branch { target: edge.target.clone() }, edge.arguments.clone()),
                Terminator::CondBranch { condition, then_edge, else_edge, .. } => (Opcode::CondBranch, vec![condition.clone()], Attributes::CondBranch {
                    then_block: then_edge.target.clone(), else_block: else_edge.target.clone(), then_arguments: then_edge.arguments.clone(), else_arguments: else_edge.arguments.clone(),
                }, then_edge.arguments.iter().chain(&else_edge.arguments).cloned().collect()),
                Terminator::Switch { value, cases, default, .. } => (Opcode::Switch, vec![value.clone()], Attributes::Switch {
                    cases: cases.iter().map(|case| SwitchCase { tag: case.tag.clone(), target: case.edge.target.clone(), arguments: case.edge.arguments.clone() }).collect(),
                    default: default.target.clone(), default_arguments: default.arguments.clone(),
                }, cases.iter().flat_map(|case| case.edge.arguments.iter()).chain(&default.arguments).cloned().collect()),
            };
            let consumes: BTreeSet<_> = transfers.into_iter().filter(|id| types.get(id).is_some_and(|ty| owned(&graph, ty))).collect();
            il_graph::Block { entity_id: block.entity_id.clone(), arguments: block.arguments.clone(), operations: block.operations.clone(), terminator: Operation {
                entity_id: block.terminator.entity_id().into(), opcode, inputs, outputs: vec![], attributes, effects: vec![], consumes: consumes.into_iter().collect(), produces: vec![],
            } }
        }).collect();
        graph.functions.push(il_graph::Function { entity_id: function.entity_id.clone(), name: function.name.clone(), parameters: function.parameters.clone(), result: function.result.clone(),
            effects: function.effects.clone(), capabilities: function.capabilities.clone(), blocks, contracts: vec![] });
    }
    let mut occupied = BTreeSet::new();
    for ty in &graph.types { occupied.insert(ty.entity_id.clone()); }
    for capability in &graph.capabilities { occupied.insert(capability.entity_id.clone()); }
    for function in &graph.functions {
        occupied.insert(function.entity_id.clone());
        for parameter in &function.parameters { occupied.insert(parameter.entity_id.clone()); }
        for block in &function.blocks {
            occupied.insert(block.entity_id.clone());
            for argument in &block.arguments { occupied.insert(argument.entity_id.clone()); }
            for operation in block.operations.iter().chain(std::iter::once(&block.terminator)) {
                occupied.insert(operation.entity_id.clone());
                for output in &operation.outputs { occupied.insert(output.entity_id.clone()); }
            }
        }
    }
    let mut module_id = "__mir_module".to_owned();
    while occupied.contains(&module_id) { module_id.push('_'); }
    graph.modules.push(Module { entity_id: module_id.clone(), path: module_id, imports: vec![],
        declarations: graph.types.iter().map(|ty| ty.entity_id.clone()).chain(graph.functions.iter().map(|function| function.entity_id.clone())).collect(), visibility: Visibility::Public });
    // MIR calls are already resolved IDs. Display names from distinct source
    // modules may coincide; the synthetic view gives each its stable identity.
    for function in &mut graph.functions { function.name = function.entity_id.clone(); }
    graph
}

fn error(program: &Program, diagnostics: &mut Vec<Diagnostic>, code: &str, entity: &str, message: impl Into<String>) {
    let mut diagnostic = Diagnostic::error(code, Some(entity), message, program.source_revision);
    diagnostic.stage = "verify_mir".into();
    diagnostics.push(diagnostic);
}

#[derive(Clone)]
struct State {
    types: BTreeMap<String, String>,
    live: BTreeSet<String>,
    loans: BTreeMap<String, String>,
    order: Vec<String>,
    parameter_count: usize,
}

impl State {
    fn transfer(&mut self, program: &Program, diagnostics: &mut Vec<Diagnostic>, entity: &str, id: &str, graph: &Graph) {
        if self.loans.contains_key(id) { error(program, diagnostics, "E_BORROW_ESCAPE", entity, "MIR cannot transfer a borrowed view"); return; }
        if self.types.get(id).is_some_and(|ty| owned(graph, ty)) {
            if self.loans.values().any(|owner| owner == id) { error(program, diagnostics, "E_BORROW_ESCAPE", entity, "MIR transfers owner with an active loan"); }
            if !self.live.remove(id) { error(program, diagnostics, "E_USE_AFTER_MOVE", entity, format!("MIR owner is no longer live: {id}")); }
        }
    }

    fn expected_cleanup(&self, all: bool) -> Vec<DropAction> {
        self.order.iter().skip(if all { 0 } else { self.parameter_count }).rev().filter(|id| self.live.contains(*id)).filter_map(|id| self.types.get(id).map(|ty| DropAction { value_id: id.clone(), type_ref: ty.clone() })).collect()
    }

    fn cleanup(&mut self, program: &Program, diagnostics: &mut Vec<Diagnostic>, entity: &str, actions: &[DropAction], all: bool, function: &Function, graph: &Graph) {
        let expected = self.expected_cleanup(all);
        for action in expected.iter().chain(actions) { for effect in il_checker::resource_cleanup_effects(graph, &action.type_ref) { if !function.effects.contains(&effect) { error(program, diagnostics, "E_EFFECT_UNDECLARED", entity, format!("implicit resource cleanup requires {effect:?} effect")); } } }
        let expected_ids: BTreeSet<_> = expected.iter().map(|action| &action.value_id).collect();
        let actual_ids: BTreeSet<_> = actions.iter().map(|action| &action.value_id).collect();
        if expected_ids.difference(&actual_ids).next().is_some() { error(program, diagnostics, "E_MIR_MISSING_DROP", entity, "MIR omits cleanup for a live owned value"); }
        if actual_ids.difference(&expected_ids).next().is_some() { error(program, diagnostics, "E_MIR_INVALID_CLEANUP", entity, "MIR cleanup includes a transferred, borrowed, scalar or out-of-scope value"); }
        if expected_ids == actual_ids && expected != actions { error(program, diagnostics, "E_MIR_CLEANUP_ORDER", entity, "MIR cleanup must use exact types and reverse initialization order"); }
        let mut seen = BTreeSet::new();
        for action in actions {
            if !seen.insert(&action.value_id) || !self.live.remove(&action.value_id) { error(program, diagnostics, "E_DOUBLE_DROP", entity, format!("MIR cleanup repeats or invalidates a dead owner: {}", action.value_id)); }
            if self.types.get(&action.value_id) != Some(&action.type_ref) { error(program, diagnostics, "E_TYPE_MISMATCH", entity, "MIR cleanup type differs from its value definition"); }
            if self.loans.contains_key(&action.value_id) || self.loans.values().any(|owner| owner == &action.value_id) { error(program, diagnostics, "E_BORROW_ESCAPE", entity, "MIR cleanup releases a borrowed owner or borrowed view"); }
        }
    }
}

fn inferred_moves(operation: &Operation, state: &State, graph: &Graph) -> Vec<String> {
    let candidates: Vec<_> = match operation.opcode {
        Opcode::Move | Opcode::Drop | Opcode::Call | Opcode::Record | Opcode::Tuple | Opcode::Variant | Opcode::Payload => operation.inputs.clone(),
        Opcode::RuntimeCall => match &operation.attributes {
            Attributes::RuntimeCall { symbol, .. } => il_checker::runtime_signature(symbol).map(|signature| operation.inputs.iter().zip(signature.parameters).filter(|(_, parameter)| parameter.passing == il_checker::Passing::Owned).map(|(id, _)| id.clone()).collect()).unwrap_or_default(),
            _ => vec![],
        },
        _ => vec![],
    };
    candidates.into_iter().filter(|id| state.types.get(id).is_some_and(|ty| owned(graph, ty))).collect()
}

fn verify_lifetimes(program: &Program, function: &Function, graph: &Graph, diagnostics: &mut Vec<Diagnostic>) {
    let Some(entry) = function.blocks.first() else { return; };
    let parameters: BTreeMap<_, _> = function.parameters.iter().map(|parameter| (parameter.entity_id.clone(), parameter.type_ref.clone())).collect();
    let initial: BTreeSet<_> = function.parameters.iter().filter(|parameter| owned(graph, &parameter.type_ref)).map(|parameter| parameter.entity_id.clone()).collect();
    let mut incoming = BTreeMap::from([(entry.entity_id.clone(), initial)]);
    let blocks: BTreeMap<_, _> = function.blocks.iter().map(|block| (block.entity_id.as_str(), block)).collect();
    let mut queue = VecDeque::from([entry.entity_id.clone()]);
    let mut visited = BTreeSet::new();
    while let Some(id) = queue.pop_front() {
        if !visited.insert(id.clone()) { continue; }
        let Some(block) = blocks.get(id.as_str()) else { continue; };
        let mut state = State { types: parameters.clone(), live: incoming.get(&id).cloned().unwrap_or_default(), loans: BTreeMap::new(),
            order: function.parameters.iter().filter(|parameter| owned(graph, &parameter.type_ref)).map(|parameter| parameter.entity_id.clone()).collect(), parameter_count: 0 };
        state.parameter_count = state.order.len();
        for argument in &block.arguments {
            state.types.insert(argument.entity_id.clone(), argument.type_ref.clone());
            if owned(graph, &argument.type_ref) { state.live.insert(argument.entity_id.clone()); state.order.push(argument.entity_id.clone()); }
        }
        for operation in &block.operations {
            for input in &operation.inputs {
                if state.types.get(input).is_some_and(|ty| owned(graph, ty)) && !state.live.contains(input) && !state.loans.contains_key(input) {
                    error(program, diagnostics, "E_USE_AFTER_MOVE", &operation.entity_id, "MIR uses an owner after move or cleanup");
                }
            }
            for input in inferred_moves(operation, &state, graph) { state.transfer(program, diagnostics, &operation.entity_id, &input, graph); }
            if operation.opcode == Opcode::EndBorrow {
                if let Some(input) = operation.inputs.first() {
                    if state.loans.remove(input).is_none() { error(program, diagnostics, "E_BORROW_ESCAPE", &operation.entity_id, "MIR ends a loan that is not active"); }
                }
            }
            for output in &operation.outputs {
                state.types.insert(output.entity_id.clone(), output.type_ref.clone());
                if matches!(operation.opcode, Opcode::Borrow | Opcode::BorrowMut) {
                    if let Some(input) = operation.inputs.first() { state.loans.insert(output.entity_id.clone(), input.clone()); }
                } else if owned(graph, &output.type_ref) { state.live.insert(output.entity_id.clone()); state.order.push(output.entity_id.clone()); }
            }
        }
        let entity = block.terminator.entity_id();
        match &block.terminator {
            Terminator::Return { value, cleanup, .. } => {
                if let Some(value) = value { state.transfer(program, diagnostics, entity, value, graph); }
                if !state.loans.is_empty() { error(program, diagnostics, "E_BORROW_ESCAPE", entity, "MIR returns with live lexical loans"); }
                state.cleanup(program, diagnostics, entity, cleanup, true, function, graph);
            }
            Terminator::Trap { .. } => {},
            terminator => {
                if !state.loans.is_empty() { error(program, diagnostics, "E_BORROW_ESCAPE", entity, "MIR control flow transfers live lexical loans"); }
                for edge in terminator.edges() {
                    let mut outgoing = state.clone();
                    for argument in &edge.arguments { outgoing.transfer(program, diagnostics, entity, argument, graph); }
                    outgoing.cleanup(program, diagnostics, entity, &edge.cleanup, false, function, graph);
                    let next: BTreeSet<_> = outgoing.live.into_iter().filter(|id| parameters.contains_key(id)).collect();
                    match incoming.get(&edge.target) {
                        Some(previous) if previous != &next => error(program, diagnostics, "E_OWNERSHIP_JOIN", &edge.target, "MIR parameter lifetime differs across incoming edges"),
                        None => { incoming.insert(edge.target.clone(), next); queue.push_back(edge.target.clone()); }
                        _ => {}
                    }
                }
            }
        }
    }
}
