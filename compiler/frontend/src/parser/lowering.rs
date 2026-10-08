use super::*;
use super::surface::{Arm, Body, Expr, Statement, Stmt};
use il_checker::{is_owned_type, runtime_signature, runtime_error_variants, Passing, RuntimeResult};

#[derive(Clone)]
struct Binding { value: ValueDef, alive: bool, loan: bool, order: u64 }
type Environment = BTreeMap<String, Binding>;
#[derive(Clone)]
struct Local { key: String, mutable: bool }
type Locals = BTreeMap<String, Local>;
#[derive(Clone)]
struct Target { id: String, args: Vec<ValueDef>, keys: Vec<String>, env: Environment }
struct Draft { id: String, args: Vec<ValueDef>, operations: Vec<Operation> }
struct ArmExit { draft: Draft, env: Environment }
#[derive(Clone)]
struct Loop { header: Target, exit: Target }

pub(super) fn lower(graph: &mut Graph, names: &BTreeMap<String, String>, scope: &str, function: &Function, body: Body) -> Result<Vec<Block>> {
    let block_names = body.blocks.iter().map(|block| (block.name.clone(), block.id.clone())).collect();
    let mut lower = Lower { graph, declarations: names, scope, function, blocks: vec![], current: None,
        env: BTreeMap::new(), locals: BTreeMap::new(), sequence: 0, loops: vec![], block_names };
    for (index, block) in body.blocks.into_iter().enumerate() {
        lower.env.clear(); lower.locals.clear();
        for parameter in &function.parameters {
            lower.insert(parameter.entity_id.clone(), ValueDef { entity_id: parameter.entity_id.clone(), type_ref: parameter.type_ref.clone() }, false);
            lower.locals.insert(parameter.name.clone(), Local { key: parameter.entity_id.clone(), mutable: false });
        }
        for argument in &block.args {
            lower.insert(argument.entity_id.clone(), ValueDef { entity_id: argument.entity_id.clone(), type_ref: argument.type_ref.clone() }, false);
            lower.locals.insert(argument.name.clone(), Local { key: argument.entity_id.clone(), mutable: false });
        }
        let args = block.args.into_iter().map(|arg| ValueDef { entity_id: arg.entity_id, type_ref: arg.type_ref }).collect();
        lower.current = Some(Draft { id: block.id.clone(), args, operations: vec![] });
        if body.implicit && index == 0 && !function.parameters.is_empty() {
            lower.current.as_mut().unwrap().id = format!("{}.prologue", function.entity_id);
            let target = lower.target(block.id, &lower.env.clone(), None)?;
            lower.branch(&target, format!("{}.prologue.transfer", function.entity_id))?;
            lower.start(&target);
        }
        lower.statements(&block.statements)?;
        if lower.current.is_some() { return Err(lower.error("E_MISSING_RETURN", "function or explicit block requires an explicit terminator")); }
    }
    Ok(lower.blocks)
}

struct Lower<'a> {
    graph: &'a mut Graph, declarations: &'a BTreeMap<String, String>, scope: &'a str, function: &'a Function,
    blocks: Vec<Block>, current: Option<Draft>, env: Environment, locals: Locals,
    sequence: u64, loops: Vec<Loop>, block_names: BTreeMap<String, String>,
}

impl Lower<'_> {
    fn error(&self, code: &str, message: &str) -> Diagnostic {
        let mut error = Diagnostic::error(code, Some(&self.function.entity_id), message, self.graph.revision);
        error.stage = "text_parse".into(); error
    }
    fn fresh(&mut self, owner: &str) -> String { let id = format!("{owner}.expr.{}", self.sequence); self.sequence += 1; id }
    fn owns(&self, ty: &str) -> bool { is_owned_type(self.graph, ty) }
    fn insert(&mut self, key: String, value: ValueDef, loan: bool) {
        let order = self.sequence; self.sequence += 1;
        self.env.insert(key, Binding { value, alive: true, loan, order });
    }
    fn value(&self, key: &str) -> Result<ValueDef> {
        let binding = self.env.get(key).ok_or_else(|| self.error("E_NAME_NOT_FOUND", &format!("value is not defined: {key}")))?;
        if !binding.alive { return Err(self.error("E_USE_AFTER_MOVE", &format!("value is no longer live: {key}"))); }
        Ok(binding.value.clone())
    }
    fn name(&self, name: &str) -> Result<String> {
        let key = self.locals.get(name).map(|local| local.key.clone()).unwrap_or_else(|| name.into());
        self.value(&key)?; Ok(key)
    }
    fn target(&self, id: String, source: &Environment, selected: Option<&[String]>) -> Result<Target> {
        let mut bindings: Vec<_> = source.iter().filter(|(key, value)| value.alive && selected.is_none_or(|keys| keys.contains(key))).collect();
        bindings.sort_by_key(|(_, value)| value.order);
        let mut env = Environment::new(); let mut keys = vec![]; let mut args = vec![];
        for (index, (key, binding)) in bindings.into_iter().enumerate() {
            if binding.loan { return Err(self.error("E_BORROW_ESCAPE", "lexical borrow cannot cross a control-flow edge")); }
            let value = ValueDef { entity_id: format!("{id}.arg.{index}"), type_ref: binding.value.type_ref.clone() };
            env.insert(key.clone(), Binding { value: value.clone(), ..binding.clone() }); keys.push(key.clone()); args.push(value);
        }
        Ok(Target { id, args, keys, env })
    }
    fn edge(&self, target: &Target) -> Result<Vec<String>> {
        target.keys.iter().zip(&target.args).map(|(key, arg)| {
            let value = self.value(key).map_err(|_| self.error("E_OWNERSHIP_JOIN", "control-flow edge loses a required live binding"))?;
            if value.type_ref != arg.type_ref { return Err(self.error("E_TYPE_MISMATCH", "join binding types disagree")); }
            Ok(value.entity_id)
        }).collect()
    }
    fn start(&mut self, target: &Target) {
        self.current = Some(Draft { id: target.id.clone(), args: target.args.clone(), operations: vec![] }); self.env = target.env.clone();
    }
    fn make(&self, id: String, opcode: Opcode, inputs: Vec<String>, outputs: Vec<ValueDef>, attributes: Attributes) -> Result<Operation> {
        let mut operation = op(id, opcode, inputs, outputs, attributes);
        let type_of = |id: &str| self.env.values().find(|binding| binding.value.entity_id == id).map(|binding| binding.value.type_ref.as_str());
        let mut moved = BTreeSet::new();
        if matches!(opcode, Opcode::Move | Opcode::Return | Opcode::Record | Opcode::Tuple | Opcode::Variant | Opcode::Payload | Opcode::Drop | Opcode::Call | Opcode::Branch | Opcode::CondBranch | Opcode::Switch) {
            for input in operation.inputs.iter().chain(operation.branch_arguments()) {
                if type_of(input).is_some_and(|ty| self.owns(ty)) { moved.insert(input.clone()); }
            }
            if matches!(opcode, Opcode::CondBranch | Opcode::Switch) {
                moved = operation.branch_arguments().into_iter().filter(|id| type_of(id).is_some_and(|ty| self.owns(ty))).cloned().collect();
            }
        }
        match &operation.attributes {
            Attributes::Call { callee } => {
                let signature = self.graph.functions.iter().find(|f| f.entity_id == *callee).ok_or_else(|| self.error("E_NAME_NOT_FOUND", "call target is not declared"))?;
                operation.effects = signature.effects.clone();
            }
            Attributes::RuntimeCall { symbol } => {
                let signature = runtime_signature(symbol).ok_or_else(|| self.error("E_UNSUPPORTED_FEATURE", "unknown runtime symbol"))?;
                operation.effects = signature.effects.to_vec();
                for (parameter, input) in signature.parameters.iter().zip(&operation.inputs) {
                    if parameter.passing == Passing::Owned && type_of(input).is_some_and(|ty| self.owns(ty)) { moved.insert(input.clone()); }
                }
            }
            _ => {}
        }
        if opcode == Opcode::Const && operation.outputs.iter().any(|value| self.owns(&value.type_ref)) { operation.effects = vec![Effect::Alloc]; }
        if opcode == Opcode::Clone && operation.inputs.first().and_then(|id| type_of(id)).is_some_and(|ty| self.owns(ty)) { operation.effects = vec![Effect::Alloc]; }
        operation.consumes = moved.into_iter().collect();
        if !matches!(opcode, Opcode::Borrow | Opcode::BorrowMut) { operation.produces = operation.outputs.iter().filter(|value| self.owns(&value.type_ref)).cloned().collect(); }
        Ok(operation)
    }
    fn emit(&mut self, operation: Operation) -> Result<()> {
        if self.blocks.len() > 10_000 || self.sequence > 100_000 { return Err(self.error("E_RESOURCE_LIMIT", "surface lowering budget exceeded")); }
        for consumed in &operation.consumes {
            for binding in self.env.values_mut().filter(|binding| binding.value.entity_id == *consumed) { binding.alive = false; }
        }
        if operation.opcode == Opcode::EndBorrow {
            for input in &operation.inputs { for binding in self.env.values_mut().filter(|binding| binding.value.entity_id == *input) { binding.alive = false; } }
        }
        for value in &operation.outputs { self.insert(value.entity_id.clone(), value.clone(), matches!(operation.opcode, Opcode::Borrow | Opcode::BorrowMut)); }
        if operation.opcode.is_terminator() {
            let draft = self.current.take().ok_or_else(|| self.error("E_SCHEMA_INVALID", "operation after terminator"))?;
            self.blocks.push(Block { entity_id: draft.id, arguments: draft.args, operations: draft.operations, terminator: operation });
        } else {
            let error = self.error("E_SCHEMA_INVALID", "operation after terminator");
            self.current.as_mut().ok_or(error)?.operations.push(operation);
        }
        Ok(())
    }
    fn branch(&mut self, target: &Target, id: String) -> Result<()> {
        let inputs = self.edge(target)?;
        let operation = self.make(id, Opcode::Branch, inputs, vec![], Attributes::Branch { target: target.id.clone() })?;
        self.emit(operation)
    }
    fn save_exit(&mut self) -> Option<ArmExit> { self.current.take().map(|draft| ArmExit { draft, env: self.env.clone() }) }
    fn join(&mut self, id: String, baseline: &Environment, exits: Vec<ArmExit>) -> Result<()> {
        if exits.is_empty() { self.current = None; return Ok(()); }
        let mut selected = vec![];
        for (key, initial) in baseline.iter().filter(|(_, binding)| binding.alive) {
            let alive: Vec<_> = exits.iter().map(|exit| exit.env.get(key).is_some_and(|binding| binding.alive)).collect();
            if alive.iter().any(|alive| *alive) && alive.iter().any(|alive| !*alive) && self.owns(&initial.value.type_ref) {
                return Err(self.error("E_OWNERSHIP_JOIN", "owned binding has inconsistent liveness at control-flow join"));
            }
            if alive.iter().all(|alive| *alive) { selected.push(key.clone()); }
        }
        let target = self.target(id, &exits[0].env, Some(&selected))?;
        for (index, exit) in exits.into_iter().enumerate() {
            self.current = Some(exit.draft); self.env = exit.env;
            self.branch(&target, format!("{}.incoming.{index}", target.id))?;
        }
        self.start(&target); Ok(())
    }
    fn statements(&mut self, statements: &[Stmt]) -> Result<()> {
        for statement in statements {
            if self.current.is_none() { return Err(self.error("E_SCHEMA_INVALID", "statement after unconditional terminator")); }
            self.statement(statement)?;
        }
        Ok(())
    }
    fn statement(&mut self, statement: &Stmt) -> Result<()> {
        let id = &statement.id;
        match &statement.kind {
            Statement::Core(operation) => self.emit(operation.clone())?,
            Statement::Let { name, mutable, output, ty, value } => {
                if self.locals.contains_key(name) { return Err(self.error("E_DUPLICATE_NAME", "local name is already declared")); }
                let key = self.expression(value, Some(ty), Some((id.clone(), output.clone())))?;
                self.locals.insert(name.clone(), Local { key, mutable: *mutable });
            }
            Statement::Assign(name, value) => {
                let local = self.locals.get(name).cloned().ok_or_else(|| self.error("E_NAME_NOT_FOUND", "assignment target is not declared"))?;
                if !local.mutable { return Err(self.error("E_SCHEMA_INVALID", "assignment requires let mut")); }
                let ty = self.env.get(&local.key).ok_or_else(|| self.error("E_NAME_NOT_FOUND", "assignment binding is unavailable"))?.value.type_ref.clone();
                let key = self.expression(value, Some(&ty), Some((id.clone(), format!("{id}.value"))))?;
                if self.env.get(&local.key).is_some_and(|binding| binding.alive) && self.owns(&ty) {
                    let input = self.value(&local.key)?.entity_id;
                    self.emit(self.make(format!("{id}.replace.drop"), Opcode::Drop, vec![input], vec![], Attributes::Empty {})?)?;
                }
                let binding = self.env.remove(&key).ok_or_else(|| self.error("E_NAME_NOT_FOUND", "assignment value missing"))?;
                self.env.insert(local.key, binding);
            }
            Statement::Eval(value) => { self.expression(value, None, None)?; }
            Statement::Return(value) => {
                let mut inputs = vec![];
                if let Some(value) = value {
                    let result = self.function.result.clone(); let key = self.expression(value, Some(&result), None)?;
                    if result != "Unit" { inputs.push(self.value(&key)?.entity_id); }
                }
                self.emit(self.make(id.clone(), Opcode::Return, inputs, vec![], Attributes::Empty {})?)?;
            }
            Statement::If(condition, yes, no) => {
                let condition = self.expression(condition, Some("Bool"), None)?;
                let baseline = self.env.clone(); let locals = self.locals.clone();
                let then_target = self.target(format!("{id}.then"), &baseline, None)?;
                let else_target = self.target(format!("{id}.else"), &baseline, None)?;
                let attributes = Attributes::CondBranch { then_block: then_target.id.clone(), else_block: else_target.id.clone(), then_arguments: self.edge(&then_target)?, else_arguments: self.edge(&else_target)? };
                self.emit(self.make(id.clone(), Opcode::CondBranch, vec![self.value(&condition)?.entity_id], vec![], attributes)?)?;
                self.start(&then_target); self.locals = locals.clone(); self.statements(yes)?; let mut exits = vec![];
                if let Some(exit) = self.save_exit() { exits.push(exit); }
                self.start(&else_target); self.locals = locals.clone(); self.statements(no)?;
                if let Some(exit) = self.save_exit() { exits.push(exit); }
                self.join(format!("{id}.join"), &baseline, exits)?; self.locals = locals;
            }
            Statement::While(condition, body) => {
                let locals = self.locals.clone(); let baseline = self.env.clone();
                let header = self.target(format!("{id}.header"), &baseline, None)?;
                self.branch(&header, format!("{id}.enter"))?; self.start(&header);
                let condition = self.expression(condition, Some("Bool"), None)?;
                let state = self.env.clone(); let body_target = self.target(format!("{id}.body"), &state, None)?;
                let exit_target = self.target(format!("{id}.exit"), &state, None)?;
                let attributes = Attributes::CondBranch { then_block: body_target.id.clone(), else_block: exit_target.id.clone(), then_arguments: self.edge(&body_target)?, else_arguments: self.edge(&exit_target)? };
                self.emit(self.make(id.clone(), Opcode::CondBranch, vec![self.value(&condition)?.entity_id], vec![], attributes)?)?;
                self.loops.push(Loop { header: header.clone(), exit: exit_target.clone() });
                self.start(&body_target); self.statements(body)?;
                if self.current.is_some() { self.branch(&header, format!("{id}.back"))?; }
                self.loops.pop(); self.start(&exit_target); self.locals = locals;
            }
            Statement::Break | Statement::Continue => {
                let loop_ = self.loops.last().cloned().ok_or_else(|| self.error("E_SCHEMA_INVALID", "break/continue requires a loop"))?;
                let target = if matches!(statement.kind, Statement::Break) { loop_.exit } else { loop_.header };
                self.branch(&target, id.clone())?;
            }
            Statement::Match(value, arms) => self.match_statement(id, value, arms)?,
            Statement::Branch(target, values) => {
                let inputs = values.iter().map(|name| self.name(name).and_then(|key| self.value(&key)).map(|value| value.entity_id)).collect::<Result<Vec<_>>>()?;
                let target = self.block_names.get(target).cloned().unwrap_or_else(|| target.clone());
                self.emit(self.make(id.clone(), Opcode::Branch, inputs, vec![], Attributes::Branch { target })?)?;
            }
            Statement::CondBranch(condition, yes, yes_values, no, no_values) => {
                let inputs = vec![self.value(&self.name(condition)?)?.entity_id];
                let resolve = |values: &[String]| values.iter().map(|name| self.name(name).and_then(|key| self.value(&key)).map(|value| value.entity_id)).collect::<Result<Vec<_>>>();
                let attributes = Attributes::CondBranch { then_block: self.block_names.get(yes).cloned().unwrap_or_else(|| yes.clone()), else_block: self.block_names.get(no).cloned().unwrap_or_else(|| no.clone()), then_arguments: resolve(yes_values)?, else_arguments: resolve(no_values)? };
                self.emit(self.make(id.clone(), Opcode::CondBranch, inputs, vec![], attributes)?)?;
            }
        }
        Ok(())
    }
    fn output(&mut self, opcode: Opcode, keys: Vec<String>, ty: &str, attributes: Attributes, preferred: Option<(String, String)>) -> Result<String> {
        let (id, key) = preferred.unwrap_or_else(|| { let id = self.fresh(&self.function.entity_id.clone()); let key = format!("{id}.value"); (id, key) });
        let inputs = keys.iter().map(|key| self.value(key).map(|value| value.entity_id)).collect::<Result<Vec<_>>>()?;
        let outputs = vec![ValueDef { entity_id: key.clone(), type_ref: ty.into() }];
        self.emit(self.make(id, opcode, inputs, outputs, attributes)?)?; Ok(key)
    }
    fn unit(&mut self) -> Result<String> { self.output(Opcode::Const, vec![], "Unit", Attributes::Constant { value: Literal::Unit(()) }, None) }
    fn type_of_expression(&self, expression: &Expr) -> Option<String> {
        match expression {
            Expr::Name(name) => self.name(name).ok().and_then(|key| self.value(&key).ok()).map(|value| value.type_ref),
            Expr::Literal(Literal::Bool(_)) => Some("Bool".into()), Expr::Literal(Literal::String(_)) => Some("String".into()),
            Expr::Literal(Literal::Bytes(_)) => Some("Bytes".into()), Expr::Literal(Literal::Unit(_)) => Some("Unit".into()),
            Expr::Literal(_) => None, Expr::Cast(ty, _) | Expr::Record(ty, _) => Some(ty.clone()),
            Expr::Unary(_, value) | Expr::Negate(value) => self.type_of_expression(value),
            Expr::Binary(opcode, left, right) => if matches!(opcode, Opcode::Eq | Opcode::Ne | Opcode::Lt | Opcode::Le | Opcode::Gt | Opcode::Ge) { Some("Bool".into()) } else { self.type_of_expression(left).or_else(|| self.type_of_expression(right)) },
            Expr::Call(name, _) => {
                let callee = resolve_name(self.graph, self.declarations, self.scope, name);
                self.graph.functions.iter().find(|f| f.entity_id == callee).map(|f| f.result.clone())
                    .or_else(|| name.split_once("::").map(|(ty, _)| resolve_name(self.graph, self.declarations, self.scope, ty)))
            }
            Expr::Try(value) => self.type_of_expression(value).and_then(|name| self.graph.types.iter().find(|ty| ty.entity_id == name && ty.kind == TypeKind::Result)).and_then(|ty| ty.parameters.first()).cloned(),
            Expr::Template(_) => None,
        }
    }
    fn expression(&mut self, expression: &Expr, expected: Option<&str>, preferred: Option<(String, String)>) -> Result<String> {
        match expression {
            Expr::Name(name) => {
                let key = self.name(name)?;
                if let Some(preferred) = preferred { let ty = expected.map(str::to_owned).unwrap_or(self.value(&key)?.type_ref); self.output(Opcode::Move, vec![key], &ty, Attributes::Empty {}, Some(preferred)) }
                else { Ok(key) }
            }
            Expr::Literal(value) => {
                let inferred = self.type_of_expression(expression).unwrap_or_else(|| "I64".into()); let ty = expected.unwrap_or(&inferred);
                self.output(Opcode::Const, vec![], ty, Attributes::Constant { value: value.clone() }, preferred)
            }
            Expr::Unary(opcode, value) => {
                let key = self.expression(value, expected, None)?; let ty = expected.map(str::to_owned).unwrap_or(self.value(&key)?.type_ref);
                self.output(*opcode, vec![key], &ty, Attributes::Empty {}, preferred)
            }
            Expr::Negate(value) => {
                let ty = self.type_of_expression(value).or_else(|| expected.map(str::to_owned)).unwrap_or_else(|| "I64".into());
                let right = self.expression(value, Some(&ty), None)?; let left = self.expression(&Expr::Literal(Literal::Integer(0)), Some(&ty), None)?;
                self.output(Opcode::Sub, vec![left, right], &ty, Attributes::Empty {}, preferred)
            }
            Expr::Binary(opcode, left, right) => {
                let comparison = matches!(opcode, Opcode::Eq | Opcode::Ne | Opcode::Lt | Opcode::Le | Opcode::Gt | Opcode::Ge);
                let ty = self.type_of_expression(left).or_else(|| self.type_of_expression(right)).or_else(|| if comparison { None } else { expected.map(str::to_owned) }).unwrap_or_else(|| "I64".into());
                let left = self.expression(left, Some(&ty), None)?; let right = self.expression(right, Some(&ty), None)?;
                let result = if comparison { "Bool" } else { &ty };
                self.output(*opcode, vec![left, right], expected.unwrap_or(result), Attributes::Empty {}, preferred)
            }
            Expr::Cast(target_type, value) => {
                let input = self.expression(value, None, None)?;
                self.output(Opcode::Cast, vec![input], expected.unwrap_or(target_type), Attributes::Cast { target_type: target_type.clone() }, preferred)
            }
            Expr::Call(name, arguments) => self.call(name, arguments, expected, preferred),
            Expr::Record(ty, fields) => {
                let declaration = self.graph.types.iter().find(|t| &t.entity_id == ty).cloned().ok_or_else(|| self.error("E_NAME_NOT_FOUND", "record type not found"))?;
                if fields.len() != declaration.fields.len() || fields.iter().map(|(name, _)| name).collect::<BTreeSet<_>>().len() != fields.len() { return Err(self.error("E_TYPE_MISMATCH", "record fields must be supplied exactly once")); }
                let mut evaluated = BTreeMap::new();
                for (name, value) in fields {
                    let field = declaration.fields.iter().find(|field| &field.name == name).ok_or_else(|| self.error("E_TYPE_MISMATCH", "unknown record field"))?;
                    evaluated.insert(name.clone(), self.expression(value, Some(&field.type_ref), None)?);
                }
                let keys = declaration.fields.iter().map(|field| evaluated.get(&field.name).cloned().ok_or_else(|| self.error("E_TYPE_MISMATCH", "record field missing"))).collect::<Result<Vec<_>>>()?;
                self.output(Opcode::Record, keys, expected.unwrap_or(ty), Attributes::Record { type_id: ty.clone() }, preferred)
            }
            Expr::Try(value) => self.question(value, preferred),
            Expr::Template(parts) => self.template(parts, expected, preferred),
        }
    }
    fn variants(&self, name: &str) -> Result<Vec<(String, Vec<String>)>> {
        let ty = self.graph.types.iter().find(|ty| ty.entity_id == name).ok_or_else(|| self.error("E_TYPE_MISMATCH", "sum type is not declared"))?;
        match ty.kind {
            TypeKind::Sum => Ok(ty.variants.iter().map(|variant| (variant.name.clone(), variant.fields.clone())).collect()),
            TypeKind::Option if ty.parameters.len() == 1 => Ok(vec![("None".into(), vec![]), ("Some".into(), ty.parameters.clone())]),
            TypeKind::Result if ty.parameters.len() == 2 => Ok(vec![("Ok".into(), vec![ty.parameters[0].clone()]), ("Err".into(), vec![ty.parameters[1].clone()])]),
            _ => Err(self.error("E_TYPE_MISMATCH", "operation requires Sum, Option, or Result")),
        }
    }
    fn result_parts(&self, name: &str) -> Result<(String, String)> {
        self.graph.types.iter().find(|ty| ty.entity_id == name && ty.kind == TypeKind::Result && ty.parameters.len() == 2)
            .map(|ty| (ty.parameters[0].clone(), ty.parameters[1].clone())).ok_or_else(|| self.error("E_TYPE_MISMATCH", "error propagation requires Result<T,E>"))
    }
    fn ensure_result(&mut self, success: &str, error: &str, expected: Option<&str>) -> Result<String> {
        let contextual = expected.filter(|expected| self.result_parts(expected).is_ok_and(|parts| parts == (success.into(), error.into()))).map(str::to_owned);
        if !self.graph.types.iter().any(|ty| ty.entity_id == error) {
            let variants = runtime_error_variants(error).ok_or_else(|| self.error("E_TYPE_MISMATCH", "runtime error type is undeclared"))?;
            let core = if let Some(module) = self.graph.modules.iter().find(|module| module.path == "core") {
                if module.visibility != Visibility::Public { return Err(self.error("E_NAME_NOT_FOUND", "runtime core module must be public")); }
                module.entity_id.clone()
            } else {
                if self.graph.modules.iter().any(|module| module.entity_id == "core") { return Err(self.error("E_DUPLICATE_NAME", "reserved core module ID already belongs to another module")); }
                self.graph.modules.push(Module { entity_id: "core".into(), path: "core".into(), imports: vec![], declarations: vec![], visibility: Visibility::Public });
                "core".into()
            };
            self.graph.types.push(TypeDef { entity_id: error.into(), kind: TypeKind::Sum, parameters: vec![], layout: Layout::Inferred, integer: None, fields: vec![], variants: variants.iter().map(|name| Variant { name: (*name).into(), fields: vec![] }).collect() });
            self.graph.modules.iter_mut().find(|module| module.entity_id == core).unwrap().declarations.push(error.into());
        }
        let error_owner = self.graph.modules.iter().find(|module| module.declarations.iter().any(|id| id == error)).map(|module| (module.entity_id.clone(), module.visibility));
        if let Some((owner, visibility)) = error_owner {
            if owner != self.scope {
                if visibility != Visibility::Public { return Err(self.error("E_NAME_NOT_FOUND", "runtime error module must be explicitly public")); }
                if let Some(module) = self.graph.modules.iter_mut().find(|module| module.entity_id == self.scope) {
                    if !module.imports.contains(&owner) { module.imports.push(owner); }
                }
            }
        }
        if let Some(contextual) = contextual { return Ok(contextual); }
        if let Some(ty) = self.graph.types.iter().find(|ty| ty.kind == TypeKind::Result && ty.parameters == [success, error] && self.graph.modules.iter().any(|module| module.entity_id == self.scope && module.declarations.contains(&ty.entity_id))) {
            return Ok(ty.entity_id.clone());
        }
        let id = self.fresh(&format!("{}.result", self.function.entity_id));
        self.graph.types.push(TypeDef { entity_id: id.clone(), kind: TypeKind::Result, parameters: vec![success.into(), error.into()], layout: Layout::Inferred, integer: None, fields: vec![], variants: vec![] });
        let module = self.graph.modules.iter_mut().find(|module| module.entity_id == self.scope).ok_or_else(|| Diagnostic::error("E_NAME_NOT_FOUND", None, "generated type requires an owning module", self.graph.revision))?;
        module.declarations.push(id.clone()); Ok(id)
    }
    fn call(&mut self, name: &str, arguments: &[Expr], expected: Option<&str>, preferred: Option<(String, String)>) -> Result<String> {
        if let Some(opcode) = match name { "move" => Some(Opcode::Move), "clone" => Some(Opcode::Clone), "borrow" => Some(Opcode::Borrow), "borrow_mut" => Some(Opcode::BorrowMut), "drop" => Some(Opcode::Drop), "end_borrow" => Some(Opcode::EndBorrow), _ => None } {
            if arguments.len() != 1 { return Err(self.error("E_TYPE_MISMATCH", "ownership operation requires one argument")); }
            let key = self.expression(&arguments[0], None, None)?;
            let ty = self.value(&key)?.type_ref;
            if matches!(opcode, Opcode::Drop | Opcode::EndBorrow) {
                let id = preferred.as_ref().map(|pair| pair.0.clone()).unwrap_or_else(|| self.fresh(&self.function.entity_id.clone()));
                let input = self.value(&key)?.entity_id;
                self.emit(self.make(id, opcode, vec![input], vec![], Attributes::Empty {})?)?; return self.unit();
            }
            return self.output(opcode, vec![key], expected.unwrap_or(&ty), Attributes::Empty {}, preferred);
        }
        if let Some(symbol) = name.strip_prefix("runtime.") {
            let signature = runtime_signature(symbol).ok_or_else(|| self.error("E_UNSUPPORTED_FEATURE", "unknown runtime function"))?;
            if arguments.len() != signature.parameters.len() { return Err(self.error("E_TYPE_MISMATCH", "runtime argument count mismatch")); }
            let mut inputs = vec![];
            for (argument, parameter) in arguments.iter().zip(signature.parameters) { inputs.push(self.expression(argument, Some(parameter.type_ref), None)?); }
            let ty = match signature.result { RuntimeResult::Exact(ty) => ty.into(), RuntimeResult::Result { success, error } => self.ensure_result(success, error, expected)? };
            return self.output(Opcode::RuntimeCall, inputs, &ty, Attributes::RuntimeCall { symbol: symbol.into() }, preferred);
        }
        let callee = resolve_name(self.graph, self.declarations, self.scope, name);
        if let Some(function) = self.graph.functions.iter().find(|function| function.entity_id == callee).cloned() {
            if arguments.len() != function.parameters.len() { return Err(self.error("E_TYPE_MISMATCH", "function argument count mismatch")); }
            let mut inputs = vec![];
            for (argument, parameter) in arguments.iter().zip(&function.parameters) { inputs.push(self.expression(argument, Some(&parameter.type_ref), None)?); }
            if function.result == "Unit" {
                let id = preferred.as_ref().map(|pair| format!("{}.call", pair.0)).unwrap_or_else(|| self.fresh(&self.function.entity_id.clone()));
                let input_ids = inputs.iter().map(|key| self.value(key).map(|value| value.entity_id)).collect::<Result<Vec<_>>>()?;
                self.emit(self.make(id, Opcode::Call, input_ids, vec![], Attributes::Call { callee })?)?;
                return self.output(Opcode::Const, vec![], "Unit", Attributes::Constant { value: Literal::Unit(()) }, preferred);
            }
            return self.output(Opcode::Call, inputs, expected.unwrap_or(&function.result), Attributes::Call { callee }, preferred);
        }
        let (ty, variant) = if let Some((ty, variant)) = name.split_once("::") {
            (resolve_name(self.graph, self.declarations, self.scope, ty), variant)
        } else if let Some(expected) = expected { (expected.into(), name) }
        else { return Err(self.error("E_NAME_NOT_FOUND", &format!("function or contextual constructor is undeclared: {name}"))); };
        if name == "tuple" {
            let fields = self.graph.types.iter().find(|declaration| declaration.entity_id == ty && declaration.kind == TypeKind::Tuple).map(|declaration| declaration.parameters.clone()).ok_or_else(|| self.error("E_TYPE_MISMATCH", "tuple constructor requires contextual Tuple type"))?;
            if fields.len() != arguments.len() { return Err(self.error("E_TYPE_MISMATCH", "tuple arity mismatch")); }
            let mut keys = vec![]; for (argument, field) in arguments.iter().zip(fields) { keys.push(self.expression(argument, Some(&field), None)?); }
            return self.output(Opcode::Tuple, keys, &ty, Attributes::Empty {}, preferred);
        }
        let fields = self.variants(&ty)?.into_iter().find(|(tag, _)| tag == variant).map(|(_, fields)| fields).ok_or_else(|| self.error("E_TYPE_MISMATCH", "unknown variant constructor"))?;
        if fields.len() != arguments.len() { return Err(self.error("E_TYPE_MISMATCH", "variant payload arity mismatch")); }
        let mut keys = vec![]; for (argument, field) in arguments.iter().zip(fields) { keys.push(self.expression(argument, Some(&field), None)?); }
        self.output(Opcode::Variant, keys, &ty, Attributes::Variant { type_id: ty.clone(), variant: variant.into() }, preferred)
    }
    fn switch(&mut self, id: &str, value: &str, cases: &[(String, Target)]) -> Result<()> {
        let mut arms = vec![];
        for (tag, target) in cases { arms.push(SwitchCase { tag: tag.clone(), target: target.id.clone(), arguments: self.edge(target)? }); }
        let default = format!("{id}.invalid_tag");
        let attributes = Attributes::Switch { cases: arms, default: default.clone(), default_arguments: vec![] };
        self.emit(self.make(id.into(), Opcode::Switch, vec![self.value(value)?.entity_id], vec![], attributes)?)?;
        self.current = Some(Draft { id: default.clone(), args: vec![], operations: vec![] }); self.env.clear();
        self.emit(op(format!("{default}.trap"), Opcode::Trap, vec![], vec![], Attributes::Trap { code: "E_INVALID_DISCRIMINANT".into() }))?;
        Ok(())
    }
    fn payload(&mut self, owner: &str, key: &str, fields: &[String], preferred: Option<(String, String)>) -> Result<Vec<String>> {
        let id = preferred.as_ref().map(|pair| pair.0.clone()).unwrap_or_else(|| format!("{owner}.payload"));
        let outputs: Vec<_> = fields.iter().enumerate().map(|(index, ty)| ValueDef {
            entity_id: if fields.len() == 1 { preferred.as_ref().map(|pair| pair.1.clone()).unwrap_or_else(|| format!("{id}.value")) } else { format!("{id}.value.{index}") }, type_ref: ty.clone()
        }).collect();
        let result = outputs.iter().map(|value| value.entity_id.clone()).collect();
        self.emit(self.make(id, Opcode::Payload, vec![self.value(key)?.entity_id], outputs, Attributes::Empty {})?)?;
        Ok(result)
    }
    fn question(&mut self, expression: &Expr, preferred: Option<(String, String)>) -> Result<String> {
        let key = self.expression(expression, None, None)?;
        let (success, error) = self.result_parts(&self.value(&key)?.type_ref)?;
        let (_, function_error) = self.result_parts(&self.function.result)?;
        if error != function_error { return Err(self.error("E_TYPE_MISMATCH", "? requires an identical enclosing Result error type")); }
        let id = preferred.as_ref().map(|pair| format!("{}.try", pair.0)).unwrap_or_else(|| self.fresh(&format!("{}.try", self.function.entity_id)));
        let baseline = self.env.clone();
        let ok = self.target(format!("{id}.ok"), &baseline, None)?;
        let err = self.target(format!("{id}.err"), &baseline, None)?;
        self.switch(&id, &key, &[("Ok".into(), ok.clone()), ("Err".into(), err.clone())])?;
        self.start(&err);
        let error_value = self.payload(&err.id, &key, &[error], None)?.remove(0);
        let function_result = self.function.result.clone();
        let returned = self.output(Opcode::Variant, vec![error_value], &function_result, Attributes::Variant { type_id: function_result.clone(), variant: "Err".into() }, None)?;
        self.emit(self.make(format!("{id}.propagate"), Opcode::Return, vec![self.value(&returned)?.entity_id], vec![], Attributes::Empty {})?)?;
        self.start(&ok);
        Ok(self.payload(&ok.id, &key, &[success], preferred)?.remove(0))
    }
    fn match_statement(&mut self, id: &str, expression: &Expr, arms: &[Arm]) -> Result<()> {
        let key = self.expression(expression, None, None)?;
        let variants = self.variants(&self.value(&key)?.type_ref)?;
        let mut seen = BTreeSet::new();
        for arm in arms {
            if !seen.insert(&arm.tag) { return Err(self.error("E_DUPLICATE_NAME", "match tag appears more than once")); }
            if arm.tag != "_" && !variants.iter().any(|(tag, _)| tag == &arm.tag) { return Err(self.error("E_TYPE_MISMATCH", "match tag is not part of the sum")); }
        }
        let baseline = self.env.clone(); let locals = self.locals.clone();
        let targets = arms.iter().enumerate().map(|(index, _)| self.target(format!("{id}.arm.{index}"), &baseline, None)).collect::<Result<Vec<_>>>()?;
        let mut cases = vec![];
        for (tag, _) in &variants {
            let index = arms.iter().position(|arm| &arm.tag == tag).or_else(|| arms.iter().position(|arm| arm.tag == "_")).ok_or_else(|| self.error("E_NON_EXHAUSTIVE_MATCH", "match must cover every sum variant"))?;
            cases.push((tag.clone(), targets[index].clone()));
        }
        if targets.iter().any(|target| !cases.iter().any(|(_, used)| used.id == target.id)) { return Err(self.error("E_SCHEMA_INVALID", "unreachable match arm")); }
        self.switch(id, &key, &cases)?;
        let mut exits = vec![];
        for (arm, target) in arms.iter().zip(targets) {
            self.start(&target); self.locals = locals.clone();
            if arm.tag == "_" {
                if !arm.names.is_empty() { return Err(self.error("E_SCHEMA_INVALID", "wildcard arm cannot bind a variant payload")); }
                if self.owns(&self.value(&key)?.type_ref) { self.emit(self.make(format!("{}.discard", target.id), Opcode::Drop, vec![self.value(&key)?.entity_id], vec![], Attributes::Empty {})?)?; }
            } else {
                let fields = variants.iter().find(|(tag, _)| tag == &arm.tag).unwrap().1.clone();
                if fields.len() != arm.names.len() { return Err(self.error("E_TYPE_MISMATCH", "match pattern payload arity mismatch")); }
                let values = self.payload(&target.id, &key, &fields, None)?;
                for (name, value) in arm.names.iter().zip(values) {
                    if name == "_" {
                        if self.owns(&self.value(&value)?.type_ref) { let discard = self.fresh(&target.id); self.emit(self.make(discard, Opcode::Drop, vec![self.value(&value)?.entity_id], vec![], Attributes::Empty {})?)?; }
                    } else {
                        if self.locals.contains_key(name) { return Err(self.error("E_DUPLICATE_NAME", "match binding shadows a live local")); }
                        self.locals.insert(name.clone(), Local { key: value, mutable: false });
                    }
                }
            }
            self.statements(&arm.body)?;
            if let Some(exit) = self.save_exit() { exits.push(exit); }
        }
        self.join(format!("{id}.join"), &baseline, exits)?; self.locals = locals; Ok(())
    }
    fn template(&mut self, parts: &[Expr], expected: Option<&str>, preferred: Option<(String, String)>) -> Result<String> {
        let result_type = self.ensure_result("String", "core.AllocError", expected)?;
        if expected.is_some_and(|expected| expected != result_type) { return Err(self.error("E_TYPE_MISMATCH", "template type is Result<String,core.AllocError>")); }
        let outer_keys: Vec<_> = self.env.keys().cloned().collect();
        let mut values = vec![];
        for part in parts {
            let value = self.expression(part, None, None)?;
            if self.value(&value)?.type_ref != "String" { return Err(self.error("E_TYPE_MISMATCH", "template interpolation requires String; conversions must be explicit")); }
            values.push(value);
        }
        let id = preferred.as_ref().map(|pair| pair.0.clone()).unwrap_or_else(|| self.fresh(&format!("{}.template", self.function.entity_id)));
        let result_key = format!("{id}.result.binding");
        let mut baseline: Environment = self.env.iter().filter(|(key, _)| outer_keys.contains(key)).map(|(key, value)| (key.clone(), value.clone())).collect();
        let mut exits = vec![];
        let mut accumulator = values.remove(0);
        for (index, next) in values.into_iter().enumerate() {
            let concat = self.output(Opcode::RuntimeCall, vec![accumulator, next], &result_type, Attributes::RuntimeCall { symbol: "string_concat".into() }, None)?;
            let state = self.env.clone();
            let ok = self.target(format!("{id}.part.{index}.ok"), &state, None)?;
            let err = self.target(format!("{id}.part.{index}.err"), &state, None)?;
            let switch_id = if index == 0 { id.clone() } else { format!("{id}.part.{index}.switch") };
            self.switch(&switch_id, &concat, &[("Ok".into(), ok.clone()), ("Err".into(), err.clone())])?;
            self.start(&err);
            let error = self.payload(&err.id, &concat, &["core.AllocError".into()], None)?.remove(0);
            let returned = self.output(Opcode::Variant, vec![error], &result_type, Attributes::Variant { type_id: result_type.clone(), variant: "Err".into() }, None)?;
            let binding = self.env.remove(&returned).unwrap(); self.env.insert(result_key.clone(), binding);
            exits.push(self.save_exit().unwrap());
            self.start(&ok); accumulator = self.payload(&ok.id, &concat, &["String".into()], None)?.remove(0);
        }
        let returned = self.output(Opcode::Variant, vec![accumulator], &result_type, Attributes::Variant { type_id: result_type.clone(), variant: "Ok".into() }, None)?;
        let binding = self.env.remove(&returned).unwrap();
        baseline.insert(result_key.clone(), binding.clone()); self.env.insert(result_key.clone(), binding);
        exits.push(self.save_exit().unwrap());
        self.join(format!("{id}.join"), &baseline, exits)?;
        if let Some((_, output)) = preferred {
            let old = self.env.get(&result_key).unwrap().value.entity_id.clone();
            self.env.get_mut(&result_key).unwrap().value.entity_id = output.clone();
            for argument in &mut self.current.as_mut().unwrap().args { if argument.entity_id == old { argument.entity_id = output.clone(); } }
        }
        Ok(result_key)
    }
}
