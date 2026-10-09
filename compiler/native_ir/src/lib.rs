//! Verified fixed-width, memory-explicit native IR. No executable source IR is retained.
mod layout;
mod lower;
mod verify;
mod runtime;
pub use layout::{LayoutTable, TypeLayout, Shape, VariantLayout};
pub use lower::lower;
pub use verify::verify;
pub use runtime::runtime_externals;
use il_execution_model::{Value, Limits};
use serde::{Serialize, Deserialize};
fn required_nullable<'de,D:serde::Deserializer<'de>,T:Deserialize<'de>>(d:D)->Result<Option<T>,D::Error>{Option::<T>::deserialize(d)}

#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
pub enum BuildMode { Application { entry:String }, Captured { entry:String, arguments:Vec<Value>, limits:Limits } }

#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq,PartialOrd,Ord)]
#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
pub enum Type { Void, Int { bits:u16 }, Ptr }
impl Type { pub fn int(bits:u16)->Self { Self::Int{bits} } }
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
pub enum Operand { Register { name:String }, Integer { bits:u16, value:String }, Global { name:String }, Null }
impl Operand {
    pub fn reg(name:impl Into<String>)->Self { Self::Register{name:name.into()} }
    pub fn int(bits:u16,value:impl ToString)->Self { Self::Integer{bits,value:value.to_string()} }
}
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
pub struct Signature { pub result:Type, pub parameters:Vec<Type> }
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
pub struct External { pub symbol:String, pub signature:Signature, pub noreturn:bool }
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
pub struct Global { pub name:String, pub bytes:Vec<u8> }
#[derive(Clone,Copy,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="snake_case")]
pub enum Binary { Add, Sub, Mul, And, Or, Xor }
#[derive(Clone,Copy,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="snake_case")]
pub enum Checked { Add, Sub, Mul, Div, Rem, Shl, Shr }
#[derive(Clone,Copy,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="snake_case")]
pub enum Predicate { Eq, Ne, Ult, Ule, Ugt, Uge, Slt, Sle, Sgt, Sge }
#[derive(Clone,Copy,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="snake_case")]
pub enum Conversion { Truncate, SignExtend, ZeroExtend }

#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
pub enum InstructionKind {
    Alloca { bytes:u64, align:u32 },
    Offset { pointer:Operand, bytes:u64 },
    Load { pointer:Operand, ty:Type, align:u32 },
    Store { pointer:Operand, value:Operand, align:u32 },
    Copy { destination:Operand, source:Operand, bytes:u64 },
    Zero { pointer:Operand, bytes:u64 },
    Binary { operation:Binary, left:Operand, right:Operand },
    Compare { predicate:Predicate, left:Operand, right:Operand },
    Convert { operation:Conversion, value:Operand, bits:u16 },
    Checked { operation:Checked, signed:bool, left:Operand, right:Operand, context:Operand },
    CheckedCast { signed_source:bool, signed_target:bool, bits:u16, value:Operand, context:Operand },
    Call { symbol:String, arguments:Vec<Operand> },
}
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
pub struct Instruction { #[serde(deserialize_with="required_nullable")] pub result:Option<String>, pub entity_id:String, pub operation:InstructionKind }
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
pub enum Terminator { Return { #[serde(deserialize_with="required_nullable")] value:Option<Operand> }, Branch { target:String }, CondBranch { condition:Operand, yes:String, no:String }, Switch { value:Operand, cases:Vec<(u64,String)>, default:String }, Unreachable }
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
pub struct Block { pub name:String, pub instructions:Vec<Instruction>, pub terminator:Terminator }
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
pub struct Function { pub symbol:String, pub entity_id:String, pub signature:Signature, pub parameters:Vec<String>, pub blocks:Vec<Block> }
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
pub struct Program {
    pub schema_version:String, pub compiler_version:String, pub input_hash:String,
    pub source_revision:u64, pub target:String, pub mode:BuildMode,
    pub layouts:LayoutTable, pub globals:Vec<Global>, pub externs:Vec<External>, pub functions:Vec<Function>,
}
impl Program {
    pub fn canonical_bytes(&self)->Result<Vec<u8>,serde_json::Error>{il_graph::canonical_bytes(self)}
    pub fn hash(&self)->Result<String,serde_json::Error>{self.canonical_bytes().map(|b|il_graph::hash_bytes(&b))}
    pub fn stage_record(&self)->Result<il_hir::StageRecord,serde_json::Error>{Ok(il_hir::StageRecord{stage:"lower_to_native_ir".into(),input_hash:self.input_hash.clone(),output_hash:self.hash()?,compiler_version:self.compiler_version.clone(),diagnostics:vec![]})}
}
pub fn symbol(id:&str)->String { format!("il_{}",id.as_bytes().iter().map(|b|format!("{b:02x}")).collect::<String>()) }
