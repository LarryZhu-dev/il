//! E01 semantic building blocks.  This crate is deliberately pure: it has no
//! process, filesystem, network, clock, or environment access.

pub mod generics {
    use std::collections::BTreeMap;
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct Specializer { limit: usize, depth: usize, seen: BTreeMap<String, String> }
    impl Specializer {
        pub fn new() -> Self { Self { limit: 1024, depth: 64, seen: BTreeMap::new() } }
        pub fn specialize(&mut self, declaration: &str, args: &[&str], depth: usize) -> Result<String, &'static str> {
            if depth > self.depth { return Err("E_RESOURCE_LIMIT"); }
            let key = format!("{}<{}>", declaration, args.join(","));
            if let Some(id) = self.seen.get(&key) { return Ok(id.clone()); }
            if self.seen.len() >= self.limit { return Err("E_RESOURCE_LIMIT"); }
            let id = format!("{}${:016x}", declaration, stable_hash(&key));
            self.seen.insert(key, id.clone());
            Ok(id)
        }
        pub fn count(&self) -> usize { self.seen.len() }
    }
    impl Default for Specializer { fn default() -> Self { Self::new() } }
    fn stable_hash(value: &str) -> u64 { value.bytes().fold(14695981039346656037u64, |h, b| (h ^ b as u64).wrapping_mul(1099511628211)) }
}

pub mod traits {
    use std::collections::BTreeMap;
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct Method { pub name: String, pub signature: String, pub effects: Vec<String> }
    #[derive(Default)]
    pub struct Registry { traits: BTreeMap<String, Vec<Method>>, impls: BTreeMap<(String, String), Vec<Method>> }
    impl Registry {
        pub fn declare(&mut self, name: &str, methods: Vec<Method>) -> Result<(), &'static str> { if self.traits.insert(name.into(), methods).is_some() { Err("E_DUPLICATE_TRAIT") } else { Ok(()) } }
        pub fn implement(&mut self, trait_name: &str, ty: &str, methods: Vec<Method>) -> Result<(), &'static str> {
            let required = self.traits.get(trait_name).ok_or("E_NAME_NOT_FOUND")?;
            if required.iter().any(|r| methods.iter().find(|m| m.name == r.name).is_none()) { return Err("E_TRAIT_METHOD_MISSING"); }
            if required.iter().any(|r| methods.iter().find(|m| m.name == r.name).map(|m| (&m.signature, &m.effects)) != Some((&r.signature, &r.effects))) { return Err("E_TRAIT_SIGNATURE"); }
            if self.impls.insert((trait_name.into(), ty.into()), methods).is_some() { return Err("E_OVERLAPPING_IMPL"); }
            Ok(())
        }
        pub fn resolve(&self, trait_name: &str, ty: &str, method: &str) -> Result<&Method, &'static str> { self.impls.get(&(trait_name.into(), ty.into())).and_then(|v| v.iter().find(|m| m.name == method)).ok_or("E_TRAIT_NOT_IMPLEMENTED") }
    }
}

pub mod floats {
    #[derive(Clone, Copy, Debug, PartialEq)] pub enum Width { F32, F64 }
    pub fn add(width: Width, a: u64, b: u64) -> u64 { match width { Width::F32 => f32::to_bits(f32::from_bits(a as u32) + f32::from_bits(b as u32)) as u64, Width::F64 => f64::to_bits(f64::from_bits(a) + f64::from_bits(b)) } }
    pub fn div(width: Width, a: u64, b: u64) -> u64 { match width { Width::F32 => f32::to_bits(f32::from_bits(a as u32) / f32::from_bits(b as u32)) as u64, Width::F64 => f64::to_bits(f64::from_bits(a) / f64::from_bits(b)) } }
    pub fn not_equal(width: Width, a: u64, b: u64) -> bool { match width { Width::F32 => f32::from_bits(a as u32) != f32::from_bits(b as u32), Width::F64 => f64::from_bits(a) != f64::from_bits(b) } }
}

pub mod const_eval {
    #[derive(Clone, Copy)] pub struct Limits { pub operations: u64, pub depth: u32, pub storage: u64 }
    impl Default for Limits { fn default() -> Self { Self { operations: 1_000_000, depth: 128, storage: 16 * 1024 * 1024 } } }
    pub fn evaluate<F, T>(limits: Limits, depth: u32, storage: u64, f: F) -> Result<T, &'static str> where F: FnOnce() -> T {
        if depth > limits.depth || storage > limits.storage || limits.operations == 0 { return Err("E_RESOURCE_LIMIT"); }
        Ok(f())
    }
    pub fn reject_effect(effect: &str) -> Result<(), &'static str> { if effect == "" { Ok(()) } else { Err("E_CONST_EFFECT") } }
}

pub mod macros {
    #[derive(Clone, Debug, Eq, PartialEq)] pub struct Expansion { pub entities: Vec<String>, pub origin: String }
    pub fn expand(declaration: &str, invocation: &str, ordinal: u32, entities: &[&str]) -> Result<Expansion, &'static str> {
        if entities.len() > 10_000 { return Err("E_RESOURCE_LIMIT"); }
        let host_terms = ["process", "filesystem", "network", "shell", "command", "exec"];
        if host_terms.iter().any(|term| declaration.contains(term) || invocation.contains(term) || entities.iter().any(|entity| entity.contains(term))) { return Err("E_MACRO_HOST_ACCESS"); }
        let prefix = format!("{}${}${}", declaration, invocation, ordinal);
        Ok(Expansion { entities: entities.iter().enumerate().map(|(i, e)| format!("{}${}${}", prefix, i, e)).collect(), origin: format!("{}:{}", declaration, invocation) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn generic_ids_are_deterministic_and_shared() { let mut s = generics::Specializer::new(); assert_eq!(s.specialize("id", &["I64"], 0), s.specialize("id", &["I64"], 0)); assert_eq!(s.count(), 1); }
    #[test] fn trait_resolution_is_static_and_coherent() { let mut r = traits::Registry::default(); let m = traits::Method{name:"show".into(),signature:"fn()->String".into(),effects:vec![]}; r.declare("Display", vec![m.clone()]).unwrap(); r.implement("Display", "I64", vec![m]).unwrap(); assert!(r.resolve("Display", "I64", "show").is_ok()); }
    #[test] fn float_nan_and_zero_follow_ieee() { assert!(floats::not_equal(floats::Width::F64, f64::NAN.to_bits(), f64::NAN.to_bits())); assert!(f64::from_bits(floats::div(floats::Width::F64, 0f64.to_bits(), (-0f64).to_bits())).is_nan()); }
    #[test] fn const_eval_rejects_effects_and_limits() { assert!(const_eval::reject_effect("io").is_err()); assert!(const_eval::evaluate(const_eval::Limits::default(), 129, 0, || 1).is_err()); }
    #[test] fn macros_are_hygienic_and_host_closed() { let a = macros::expand("m", "call", 0, &["x"]).unwrap(); let b = macros::expand("m", "call", 0, &["x"]).unwrap(); assert_eq!(a,b); assert!(macros::expand("bad", "call", 0, &["process"]).is_err()); }
}
