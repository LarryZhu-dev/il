use il_graph::*;
use std::collections::{BTreeMap, BTreeSet};

pub struct Types<'a> {
    pub definitions: BTreeMap<&'a str, &'a TypeDef>,
    owned: BTreeSet<String>,
}

impl<'a> Types<'a> {
    pub fn new(graph: &'a Graph) -> Self {
        let definitions = graph.types.iter().map(|ty| (ty.entity_id.as_str(), ty)).collect();
        let mut result = Self { definitions, owned: ["String".to_owned(), "Bytes".to_owned()].into_iter().collect() };
        // Monotone propagation is bounded by the number of types and avoids recursive
        // traversal on adversarial type graphs. Structural checking rejects cycles.
        for _ in 0..=graph.types.len() {
            let mut changed = false;
            for ty in &graph.types {
                let owns = matches!(ty.kind, TypeKind::String | TypeKind::Bytes) || ty.layout == Layout::Opaque ||
                    ty.parameters.iter().chain(ty.fields.iter().map(|field| &field.type_ref)).chain(ty.variants.iter().flat_map(|variant| &variant.fields)).any(|name| result.owned.contains(name));
                if owns { changed |= result.owned.insert(ty.entity_id.clone()); }
            }
            if !changed { break; }
        }
        result
    }

    pub fn is_owned(&self, name: &str) -> bool { self.owned.contains(name) }
    pub fn definition(&self, name: &str) -> Option<&'a TypeDef> { self.definitions.get(name).copied() }

    pub fn integer(&self, name: &str) -> Option<(bool, u8)> {
        match name {
            "I8" => Some((true, 8)), "I16" => Some((true, 16)), "I32" => Some((true, 32)), "I64" => Some((true, 64)),
            "U8" => Some((false, 8)), "U16" => Some((false, 16)), "U32" => Some((false, 32)), "U64" | "Usize" => Some((false, 64)),
            _ => self.definition(name).filter(|ty| ty.layout != Layout::Opaque).and_then(|ty| ty.integer.as_ref()).map(|layout| (layout.signed, layout.bits)),
        }
    }

    pub fn contains_opaque(&self, name: &str) -> bool {
        let mut pending = vec![name.to_owned()];
        let mut visited = BTreeSet::new();
        while let Some(name) = pending.pop() {
            if !visited.insert(name.clone()) { continue; }
            if let Some(ty) = self.definition(&name) {
                if ty.layout == Layout::Opaque { return true; }
                pending.extend(ty.parameters.iter().chain(ty.fields.iter().map(|field| &field.type_ref)).chain(ty.variants.iter().flat_map(|variant| &variant.fields)).cloned());
            }
        }
        false
    }

    pub fn variants(&self, name: &str) -> Option<Vec<(String, Vec<String>)>> {
        let ty = self.definition(name)?;
        match ty.kind {
            TypeKind::Sum => Some(ty.variants.iter().map(|variant| (variant.name.clone(), variant.fields.clone())).collect()),
            TypeKind::Option if ty.parameters.len() == 1 => Some(vec![("None".into(), vec![]), ("Some".into(), ty.parameters.clone())]),
            TypeKind::Result if ty.parameters.len() == 2 => Some(vec![("Ok".into(), vec![ty.parameters[0].clone()]), ("Err".into(), vec![ty.parameters[1].clone()])]),
            _ => None,
        }
    }

    pub fn literal_matches(&self, literal: &Literal, name: &str) -> Result<(), &'static str> {
        match literal {
            Literal::Integer(number) => self.integer_literal(*number as i128, name),
            Literal::Unsigned(number) => self.integer_literal(*number as i128, name),
            Literal::Bool(_) if name == "Bool" => Ok(()),
            Literal::String(_) if name == "String" => Ok(()),
            Literal::Bytes(_) if name == "Bytes" => Ok(()),
            Literal::Unit(()) if name == "Unit" => Ok(()),
            _ => Err("E_TYPE_MISMATCH"),
        }
    }

    fn integer_literal(&self, number: i128, name: &str) -> Result<(), &'static str> {
        let Some((signed, bits)) = self.integer(name) else { return Err("E_TYPE_MISMATCH"); };
        if !matches!(bits, 8 | 16 | 32 | 64) { return Err("E_SCHEMA_INVALID"); }
        let (minimum, maximum) = if signed { (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1) } else { (0, (1i128 << bits) - 1) };
        if number < minimum || number > maximum { Err("E_INTEGER_OVERFLOW") } else { Ok(()) }
    }
}

pub fn is_owned_type(graph: &Graph, name: &str) -> bool { Types::new(graph).is_owned(name) }

/// Whether recursive destruction may close an owned host file.
pub fn contains_file_type(graph: &Graph, name: &str) -> bool {
    let mut pending = vec![name];
    let mut seen = BTreeSet::new();
    while let Some(name) = pending.pop() {
        if name == "core.File" { return true; }
        if !seen.insert(name) { continue; }
        if let Some(ty) = graph.types.iter().find(|ty| ty.entity_id == name) {
            pending.extend(ty.parameters.iter().chain(ty.fields.iter().map(|field| &field.type_ref)).chain(ty.variants.iter().flat_map(|variant| &variant.fields)).map(String::as_str));
        }
    }
    false
}
