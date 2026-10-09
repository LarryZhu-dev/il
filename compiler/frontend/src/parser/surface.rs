use super::*;

#[derive(Clone)]
pub(super) enum Expr {
    Literal(Literal), Name(String), Unary(Opcode, Box<Expr>), Negate(Box<Expr>),
    Binary(Opcode, Box<Expr>, Box<Expr>), Call(String, Option<String>, Vec<Expr>), Cast(String, Box<Expr>),
    Try(Box<Expr>), Template(Vec<Expr>), Record(String, Vec<(String, Expr)>),
}

#[derive(Clone)]
pub(super) struct Stmt { pub id: String, pub kind: Statement }

#[derive(Clone)]
pub(super) enum Statement {
    Core(Operation), Let { name: String, mutable: bool, output: String, ty: String, value: Expr },
    Assign(String, Expr), Eval(Expr), Return(Option<Expr>),
    If(Expr, Vec<Stmt>, Vec<Stmt>), While(Expr, Vec<Stmt>),
    Match(Expr, Vec<Arm>), Break, Continue,
    Branch(String, Vec<String>), CondBranch(String, String, Vec<String>, String, Vec<String>),
}

#[derive(Clone)]
pub(super) struct Arm { pub tag: String, pub names: Vec<String>, pub body: Vec<Stmt> }

pub(super) struct SurfaceBlock { pub id: String, pub name: String, pub args: Vec<Parameter>, pub statements: Vec<Stmt> }
pub(super) struct Body { pub blocks: Vec<SurfaceBlock>, pub implicit: bool }

pub(super) fn body(parser: &mut Parser, end: usize, scope: &str, owner: &str) -> Result<Body> {
    let start = parser.at;
    let annotation = parser.annotation()?;
    if parser.eat("block") {
        let mut blocks = vec![];
        let mut next_annotation = annotation;
        loop {
            let name = parser.name()?; let id = parser.id(next_annotation, owner);
            let mut args = if parser.is("(") { parser.parameters(&id, scope)? } else { vec![] };
            for arg in &mut args { arg.type_ref = parser.resolve_name(scope, &arg.type_ref); }
            let statements = statements(parser, scope, &id)?;
            blocks.push(SurfaceBlock { id, name, args, statements });
            if parser.at == end { break; }
            next_annotation = parser.annotation()?; parser.expect("block")?;
        }
        Ok(Body { blocks, implicit: false })
    } else {
        parser.at = start;
        let id = parser.allocate(owner);
        let statements = sequence(parser, end, scope, &id)?;
        Ok(Body { blocks: vec![SurfaceBlock { id, name: "entry".into(), args: vec![], statements }], implicit: true })
    }
}

fn statements(parser: &mut Parser, scope: &str, owner: &str) -> Result<Vec<Stmt>> {
    parser.expect("{")?; parser.enter()?;
    let mut result = vec![];
    while !parser.eat("}") { result.push(statement(parser, scope, owner)?); }
    parser.depth -= 1;
    Ok(result)
}

fn sequence(parser: &mut Parser, end: usize, scope: &str, owner: &str) -> Result<Vec<Stmt>> {
    let mut result = vec![];
    while parser.at < end { result.push(statement(parser, scope, owner)?); }
    Ok(result)
}

fn statement(parser: &mut Parser, scope: &str, owner: &str) -> Result<Stmt> {
    parser.enter()?;
    let result = statement_inner(parser, scope, owner);
    parser.depth -= 1;
    result
}

fn statement_inner(parser: &mut Parser, scope: &str, owner: &str) -> Result<Stmt> {
    let annotation = parser.annotation()?; let id = parser.id(annotation, owner);
    let kind = if parser.eat("op") { Statement::Core(parser.core_operation(id.clone())?) }
    else if parser.eat("let") {
        let mutable = parser.eat("mut"); let output = parser.annotation()?.unwrap_or_else(|| format!("{id}.value"));
        let name = parser.name()?; parser.expect(":")?;
        let raw = parser.type_name(scope)?; let ty = parser.resolve_name(scope, &raw);
        parser.expect("=")?; let value = expression(parser, scope, 0)?; parser.expect(";")?;
        Statement::Let { name, mutable, output, ty, value }
    } else if parser.eat("return") {
        let value = if parser.is(";") { None } else { Some(expression(parser, scope, 0)?) };
        parser.expect(";")?; Statement::Return(value)
    } else if parser.eat("if") {
        let condition = expression(parser, scope, 0)?;
        let yes = statements(parser, scope, &format!("{id}.then"))?;
        let no = if parser.eat("else") {
            if parser.is("if") { vec![statement(parser, scope, &format!("{id}.else"))?] }
            else { statements(parser, scope, &format!("{id}.else"))? }
        } else { vec![] };
        Statement::If(condition, yes, no)
    } else if parser.eat("while") {
        let condition = expression(parser, scope, 0)?;
        let body = statements(parser, scope, &format!("{id}.body"))?;
        Statement::While(condition, body)
    } else if parser.eat("match") {
        let value = expression(parser, scope, 0)?; parser.expect("{")?; parser.enter()?;
        let mut arms = vec![];
        while !parser.eat("}") {
            let tag = parser.name()?; let mut names = vec![];
            if parser.eat("(") && !parser.eat(")") { loop { names.push(parser.name()?); if parser.eat(")") { break; } parser.expect(",")?; }}
            parser.expect("=>")?;
            let body = statements(parser, scope, &format!("{id}.arm.{}", arms.len()))?;
            arms.push(Arm { tag, names, body }); parser.eat(",");
        }
        parser.depth -= 1; Statement::Match(value, arms)
    } else if parser.eat("break") { parser.expect(";")?; Statement::Break }
    else if parser.eat("continue") { parser.expect(";")?; Statement::Continue }
    else if parser.eat("branch") {
        let target = parser.name()?; let values = reference_arguments(parser)?; parser.expect(";")?;
        Statement::Branch(target, values)
    } else if parser.eat("cond_branch") {
        let condition = parser.name()?; parser.expect(",")?;
        let yes = parser.name()?; let yes_args = reference_arguments(parser)?; parser.expect(",")?;
        let no = parser.name()?; let no_args = reference_arguments(parser)?; parser.expect(";")?;
        Statement::CondBranch(condition, yes, yes_args, no, no_args)
    } else if matches!(parser.token().kind, Kind::Name(_)) {
        if parser.tokens.get(parser.at + 1).is_some_and(|token| token.kind == Kind::Symbol("=".into())) {
            let name = parser.name()?; parser.expect("=")?; let expr = expression(parser, scope, 0)?; parser.expect(";")?;
            Statement::Assign(name, expr)
        } else {
            let expr = expression(parser, scope, 0)?; parser.expect(";")?;
            if matches!(&expr, Expr::Call(name, _, _) if name == "eval") { return Err(parser.error("E_UNSUPPORTED_FEATURE", "runtime eval is forbidden")); }
            Statement::Eval(expr)
        }
    } else { return Err(parser.error("E_UNSUPPORTED_FEATURE", "unsupported statement")); };
    Ok(Stmt { id, kind })
}

fn reference_arguments(parser: &mut Parser) -> Result<Vec<String>> {
    parser.expect("(")?; let mut values = vec![];
    if !parser.eat(")") { loop { values.push(parser.name()?); if parser.eat(")") { break; } parser.expect(",")?; }}
    Ok(values)
}

fn expression(parser: &mut Parser, scope: &str, min_precedence: u8) -> Result<Expr> {
    parser.enter()?;
    let result = expression_inner(parser, scope, min_precedence);
    parser.depth -= 1; result
}

fn expression_inner(parser: &mut Parser, scope: &str, min_precedence: u8) -> Result<Expr> {
    let mut left = if parser.eat("(") {
        if parser.eat(")") { Expr::Literal(Literal::Unit(())) }
        else { let result = expression(parser, scope, 0)?; parser.expect(")")?; result }
    } else if parser.eat("!") || parser.eat("not") { Expr::Unary(Opcode::Not, Box::new(expression(parser, scope, 10)?)) }
    else if parser.is("-") && !matches!(parser.tokens.get(parser.at + 1).map(|token| &token.kind), Some(Kind::Number(_))) {
        parser.at += 1; Expr::Negate(Box::new(expression(parser, scope, 10)?))
    } else if parser.eat("cast") {
        parser.expect("<")?; let raw = parser.type_name(scope)?; let ty = parser.resolve_name(scope, &raw);
        parser.expect(">")?; parser.expect("(")?;
        let value = expression(parser, scope, 0)?; parser.expect(")")?; Expr::Cast(ty, Box::new(value))
    } else if parser.is("-") || parser.is("true") || parser.is("false") || matches!(parser.token().kind, Kind::String(_) | Kind::Number(_)) {
        let value = parser.json()?;
        Expr::Literal(serde_json::from_value(value).map_err(|error| parser.error("E_SCHEMA_INVALID", &error.to_string()))?)
    } else {
        let name = parser.name()?;
        if name == "f" && matches!(parser.token().kind, Kind::String(_)) {
            let content = parser.name()?; Expr::Template(template(parser, scope, &content)?)
        } else {
            let name = if parser.eat(":") { parser.expect(":")?; format!("{name}::{}", parser.name()?) } else { name };
            let capability = if parser.eat("[") {
                if !name.starts_with("runtime.") { return Err(parser.error("E_SCHEMA_INVALID", "only runtime calls accept a capability selector")); }
                let id = parser.name()?; parser.expect("]")?; Some(id)
            } else { None };
            if capability.is_some() && !parser.is("(") { return Err(parser.error("E_SCHEMA_INVALID", "capability selector requires a call")); }
            if parser.eat("(") {
                let mut args = vec![];
                if !parser.eat(")") { loop { args.push(expression(parser, scope, 0)?); if parser.eat(")") { break; } parser.expect(",")?; }}
                Expr::Call(name, capability, args)
            } else {
                let ty = parser.resolve_name(scope, &name);
                if parser.is("{") && parser.graph.types.iter().any(|t| t.entity_id == ty && t.kind == TypeKind::Record) {
                    parser.at += 1; let mut fields = vec![];
                    if !parser.eat("}") { loop {
                        let name = parser.name()?; parser.expect(":")?; let value = expression(parser, scope, 0)?;
                        fields.push((name, value)); if parser.eat("}") { break; } parser.expect(",")?;
                    }}
                    Expr::Record(ty, fields)
                } else { Expr::Name(name) }
            }
        }
    };
    check_expression_depth(parser, &left)?;
    while parser.eat("?") {
        left = Expr::Try(Box::new(left));
        check_expression_depth(parser, &left)?;
    }
    loop {
        let operator = match &parser.token().kind { Kind::Symbol(value) => value.as_str(), _ => break };
        let (level, opcode) = match operator {
            "|" => (1, Opcode::BitOr), "^" => (2, Opcode::BitXor), "&" => (3, Opcode::BitAnd),
            "==" => (4, Opcode::Eq), "!=" => (4, Opcode::Ne), "<" => (5, Opcode::Lt), "<=" => (5, Opcode::Le), ">" => (5, Opcode::Gt), ">=" => (5, Opcode::Ge),
            "<<" => (6, Opcode::Shl), ">>" => (6, Opcode::Shr), "+" => (7, Opcode::Add), "-" => (7, Opcode::Sub),
            "*" => (8, Opcode::Mul), "/" => (8, Opcode::Div), "%" => (8, Opcode::Rem), _ => break,
        };
        if level < min_precedence { break; } parser.at += 1;
        left = Expr::Binary(opcode, Box::new(left), Box::new(expression(parser, scope, level + 1)?));
        check_expression_depth(parser, &left)?;
    }
    Ok(left)
}

fn check_expression_depth(parser: &Parser, expression: &Expr) -> Result<()> {
    let mut pending = vec![(expression, 1)];
    while let Some((value, depth)) = pending.pop() {
        if depth > 32 { return Err(parser.error("E_RESOURCE_LIMIT", "expression depth exceeds 32")); }
        match value {
            Expr::Unary(_, child) | Expr::Negate(child) | Expr::Cast(_, child) | Expr::Try(child) => pending.push((child, depth + 1)),
            Expr::Binary(_, left, right) => { pending.push((left, depth + 1)); pending.push((right, depth + 1)); }
            Expr::Call(_, _, args) | Expr::Template(args) => pending.extend(args.iter().map(|arg| (arg, depth + 1))),
            Expr::Record(_, fields) => pending.extend(fields.iter().map(|(_, value)| (value, depth + 1))),
            Expr::Literal(_) | Expr::Name(_) => {}
        }
    }
    Ok(())
}

fn template(parser: &Parser, scope: &str, content: &str) -> Result<Vec<Expr>> {
    let mut parts = vec![]; let mut from = 0;
    while let Some(relative) = content[from..].find("${") {
        let start = from + relative;
        parts.push(Expr::Literal(Literal::String(content[from..start].into())));
        let bytes = content.as_bytes(); let mut at = start + 2; let mut depth = 1; let mut quoted = false;
        while at < bytes.len() && depth > 0 {
            if bytes[at] == b'\\' && quoted { at += 2; continue; }
            if bytes[at] == b'"' { quoted = !quoted; }
            if !quoted { if bytes[at] == b'{' { depth += 1; } if bytes[at] == b'}' { depth -= 1; } }
            if depth > 0 { at += 1; }
        }
        if depth != 0 { return Err(parser.error("E_SCHEMA_INVALID", "unterminated template interpolation")); }
        let tokens = lex(&content[start + 2..at]).map_err(|(_, message)| parser.error("E_SCHEMA_INVALID", &message))?;
        let mut nested = Parser { tokens, at: 0, revision: parser.revision, counter: parser.counter,
            graph: parser.graph.clone(), names: parser.names.clone(), pending: vec![], depth: parser.depth };
        parts.push(expression(&mut nested, scope, 0)?);
        if nested.token().kind != Kind::End { return Err(parser.error("E_SCHEMA_INVALID", "trailing template interpolation tokens")); }
        from = at + 1;
    }
    parts.push(Expr::Literal(Literal::String(content[from..].into())));
    Ok(parts)
}
