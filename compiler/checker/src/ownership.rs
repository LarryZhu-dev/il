use super::{Context, Facts};
use il_graph::*;
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Life { Live, Moved, Dropped, Conflict }

#[derive(Clone, Debug, PartialEq, Eq)]
struct Incoming {
    parameters: BTreeMap<String, Life>,
    refinements: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
struct Loan { owner: String, mutable: bool, active: bool }

#[derive(Clone)]
struct State {
    life: BTreeMap<String, Life>,
    types: BTreeMap<String, String>,
    loans: BTreeMap<String, Loan>,
    refinements: BTreeMap<String, String>,
}

impl State {
    fn read(&self, context: &Context<'_>, operation: &Operation, value: &str) {
        match self.life.get(value) {
            Some(Life::Live) => {}
            Some(Life::Dropped) if operation.opcode == Opcode::Drop => context.error("E_DOUBLE_DROP", &operation.entity_id, format!("value was already dropped: {value}")),
            _ => context.error("E_USE_AFTER_MOVE", &operation.entity_id, format!("value is not live on every path: {value}")),
        }
        if let Some(loan) = self.loans.get(value) {
            if !loan.active { context.error("E_BORROW_ESCAPE", &operation.entity_id, "borrow is used after end_borrow"); }
        } else if self.loans.values().any(|loan| loan.active && loan.owner == value && loan.mutable) {
            context.error("E_BORROW_CONFLICT", &operation.entity_id, "owner cannot be read while uniquely borrowed");
        }
    }

    fn move_value(&mut self, context: &Context<'_>, operation: &Operation, value: &str, drop: bool) {
        self.read(context, operation, value);
        if self.loans.contains_key(value) { context.error("E_BORROW_ESCAPE", &operation.entity_id, "borrowed view cannot be moved, stored, returned or dropped as an owner"); return; }
        if self.loans.values().any(|loan| loan.active && loan.owner == value) {
            context.error("E_BORROW_CONFLICT", &operation.entity_id, "owner cannot move or drop while borrowed");
        }
        self.life.insert(value.into(), if drop { Life::Dropped } else { Life::Moved });
    }
}

pub(super) fn check_function(context: &Context<'_>, function: &Function, facts: &BTreeMap<String, Facts>) {
    let Some(entry) = function.blocks.first() else { return; };
    let initial = Incoming { parameters: function.parameters.iter().map(|parameter| (parameter.entity_id.clone(), Life::Live)).collect(), refinements: BTreeMap::new() };
    let blocks: BTreeMap<_, _> = function.blocks.iter().map(|block| (block.entity_id.as_str(), block)).collect();
    let mut inputs: BTreeMap<String, Incoming> = [(entry.entity_id.clone(), initial)].into_iter().collect();
    let mut queue = VecDeque::from([entry.entity_id.clone()]);
    let mut visits = 0usize;
    while let Some(id) = queue.pop_front() {
        visits += 1;
        if visits > function.blocks.len().saturating_mul(function.parameters.len() + 8).max(32) {
            context.error("E_RESOURCE_LIMIT", &function.entity_id, "ownership dataflow convergence budget exceeded"); break;
        }
        let Some(block) = blocks.get(id.as_str()) else { continue; };
        let Some(incoming) = inputs.get(&id).cloned() else { continue; };
        let mut state = State {
            life: incoming.parameters.clone(),
            types: function.parameters.iter().map(|parameter| (parameter.entity_id.clone(), parameter.type_ref.clone())).collect(),
            loans: BTreeMap::new(), refinements: incoming.refinements,
        };
        for argument in &block.arguments {
            state.life.insert(argument.entity_id.clone(), Life::Live);
            state.types.insert(argument.entity_id.clone(), argument.type_ref.clone());
        }
        for operation in &block.operations {
            apply_operation(context, operation, facts.get(&operation.entity_id), &mut state);
        }
        let operation = &block.terminator;
        let edges = edges(operation);
        if edges.is_empty() {
            apply_operation(context, operation, facts.get(&operation.entity_id), &mut state);
            if operation.opcode == Opcode::Return { check_cleanup_effect(context, function, operation, &state, true); }
            for loan in state.loans.values().filter(|loan| loan.active) {
                context.error("E_BORROW_ESCAPE", &operation.entity_id, format!("live loan of {} crosses lexical block end", loan.owner));
            }
            continue;
        }
        for input in &operation.inputs { state.read(context, operation, input); }
        for loan in state.loans.values().filter(|loan| loan.active) {
            context.error("E_BORROW_ESCAPE", &operation.entity_id, format!("live loan of {} crosses control flow edge", loan.owner));
        }
        for (target, arguments, tag) in edges {
            let Some(target_block) = blocks.get(target.as_str()) else { continue; };
            let mut edge_state = state.clone();
            let mut refined = edge_state.refinements.clone();
            if let (Some(tag), Some(value)) = (&tag, operation.inputs.first()) { refined.insert(value.clone(), tag.clone()); }
            for (index, argument) in arguments.iter().enumerate() {
                edge_state.read(context, operation, argument);
                if edge_state.loans.contains_key(argument) { context.error("E_BORROW_ESCAPE", &operation.entity_id, "borrowed view cannot transfer to another block"); }
                if edge_state.types.get(argument).is_some_and(|ty| context.types.is_owned(ty)) {
                    edge_state.move_value(context, operation, argument, false);
                }
                if let (Some(destination), Some(variant)) = (target_block.arguments.get(index), refined.get(argument).cloned()) { refined.insert(destination.entity_id.clone(), variant); }
            }
            check_cleanup_effect(context, function, operation, &edge_state, false);
            let parameters = function.parameters.iter().map(|parameter| (parameter.entity_id.clone(), *edge_state.life.get(&parameter.entity_id).unwrap_or(&Life::Conflict))).collect();
            refined.retain(|id, _| function.parameters.iter().any(|parameter| &parameter.entity_id == id) || target_block.arguments.iter().any(|argument| &argument.entity_id == id));
            let next = Incoming { parameters, refinements: refined };
            let changed = if let Some(previous) = inputs.get_mut(&target) {
                let original = previous.clone();
                for (id, life) in &next.parameters {
                    let previous_life = previous.parameters.get(id).copied().unwrap_or(Life::Conflict);
                    if previous_life != *life {
                        if context.types.is_owned(state.types.get(id).map(String::as_str).unwrap_or("")) {
                            context.error("E_OWNERSHIP_JOIN", &target, format!("owned parameter has inconsistent path state: {id}"));
                        }
                        previous.parameters.insert(id.clone(), Life::Conflict);
                    }
                }
                previous.refinements.retain(|id, tag| next.refinements.get(id) == Some(tag));
                *previous != original
            } else { inputs.insert(target.clone(), next); true };
            if changed { queue.push_back(target); }
        }
    }
}

fn check_cleanup_effect(context: &Context<'_>, function: &Function, operation: &Operation, state: &State, include_parameters: bool) {
    for (id, _) in state.life.iter().filter(|(id, life)| **life == Life::Live
        && !state.loans.contains_key(*id)
        && (include_parameters || !function.parameters.iter().any(|parameter| &parameter.entity_id == *id))) {
        if let Some(ty) = state.types.get(id) {
            for effect in super::resource_cleanup_effects(context.graph, ty) {
                if !function.effects.contains(&effect) { context.error("E_EFFECT_UNDECLARED", &operation.entity_id, format!("implicit resource cleanup requires the {effect:?} effect")); }
            }
        }
    }
}

fn apply_operation(context: &Context<'_>, operation: &Operation, facts: Option<&Facts>, state: &mut State) {
    for input in &operation.inputs { state.read(context, operation, input); }
    let Some(facts) = facts else { return; };
    if operation.opcode == Opcode::Payload {
        if let Some(input) = operation.inputs.first() {
            let actual: Vec<_> = operation.outputs.iter().map(|output| output.type_ref.clone()).collect();
            let fields = state.types.get(input).and_then(|ty| context.types.variants(ty)).and_then(|variants| state.refinements.get(input).and_then(|tag| variants.into_iter().find(|(name, _)| name == tag))).map(|(_, fields)| fields);
            if fields.as_ref() != Some(&actual) { context.error("E_TYPE_MISMATCH", &operation.entity_id, "payload fields require a matching variant proven on every incoming path"); }
        }
    }
    match operation.opcode {
        Opcode::Borrow | Opcode::BorrowMut => {
            if let (Some(input), Some(output)) = (operation.inputs.first(), operation.outputs.first()) {
                if state.loans.contains_key(input) { context.error("E_BORROW_CONFLICT", &operation.entity_id, "reborrow requires an explicit nested-loan contract"); }
                let mutable = operation.opcode == Opcode::BorrowMut;
                if state.loans.values().any(|loan| loan.active && loan.owner == *input && (mutable || loan.mutable)) { context.error("E_BORROW_CONFLICT", &operation.entity_id, "shared and unique loan lifetimes overlap"); }
                state.loans.insert(output.entity_id.clone(), Loan { owner: input.clone(), mutable, active: true });
            }
        }
        Opcode::EndBorrow => {
            if let Some(input) = operation.inputs.first() {
                match state.loans.get_mut(input) {
                    Some(loan) if loan.active => { loan.active = false; state.life.insert(input.clone(), Life::Moved); }
                    _ => context.error("E_BORROW_ESCAPE", &operation.entity_id, "end_borrow requires one active loan"),
                }
            }
        }
        _ => {
            let escaping = matches!(operation.opcode, Opcode::Move | Opcode::Return | Opcode::Record | Opcode::Tuple | Opcode::Variant | Opcode::Payload | Opcode::Drop | Opcode::Call);
            for input in &operation.inputs {
                if state.loans.contains_key(input) && escaping { context.error("E_BORROW_ESCAPE", &operation.entity_id, "borrow cannot escape through an owning operation"); }
                if facts.moved.contains(input) { state.move_value(context, operation, input, operation.opcode == Opcode::Drop); }
                else if state.loans.contains_key(input) && operation.opcode == Opcode::RuntimeCall && !facts.shared_inputs.contains(input) {
                    context.error("E_BORROW_ESCAPE", &operation.entity_id, "runtime parameter does not accept a borrowed view");
                }
            }
        }
    }
    for output in &operation.outputs {
        state.life.insert(output.entity_id.clone(), Life::Live);
        state.types.insert(output.entity_id.clone(), output.type_ref.clone());
        if matches!(operation.opcode, Opcode::Move | Opcode::Clone) {
            if let Some(tag) = operation.inputs.first().and_then(|input| state.refinements.get(input)).cloned() { state.refinements.insert(output.entity_id.clone(), tag); }
        }
        if let Attributes::Variant { variant, .. } = &operation.attributes { state.refinements.insert(output.entity_id.clone(), variant.clone()); }
    }
}

fn edges(operation: &Operation) -> Vec<(String, Vec<String>, Option<String>)> {
    match &operation.attributes {
        Attributes::Branch { target } => vec![(target.clone(), operation.inputs.clone(), None)],
        Attributes::CondBranch { then_block, else_block, then_arguments, else_arguments } => vec![(then_block.clone(), then_arguments.clone(), None), (else_block.clone(), else_arguments.clone(), None)],
        Attributes::Switch { cases, default, default_arguments } => cases.iter().map(|case| (case.target.clone(), case.arguments.clone(), Some(case.tag.clone()))).chain(std::iter::once((default.clone(), default_arguments.clone(), None))).collect(),
        _ => vec![],
    }
}
