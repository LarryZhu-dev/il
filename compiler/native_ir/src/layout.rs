use serde::{Serialize,Deserialize};
use std::collections::BTreeMap;
use il_graph::{TypeDef,TypeKind,Layout};

pub type LayoutTable=BTreeMap<String,TypeLayout>;
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
pub struct TypeLayout { pub size:u64,pub align:u32,pub owned:bool,pub shape:Shape }
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
pub struct VariantLayout { pub name:String,pub fields:Vec<String>,pub offsets:Vec<u64> }
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
pub enum Shape { Unit,Never,Bool,Integer{signed:bool,bits:u16},Buffer{utf8:bool},Resource{resource:il_graph::ResourceKind},Record{names:Vec<String>,fields:Vec<String>,offsets:Vec<u64>},Tuple{fields:Vec<String>,offsets:Vec<u64>},Sum{payload:u64,variants:Vec<VariantLayout>} }
pub(crate) fn align(value:u64,alignment:u32)->u64 { (value+alignment as u64-1)&!(alignment as u64-1) }
pub(crate) fn build(definitions:&[TypeDef])->Result<LayoutTable,String>{
    let mut table=LayoutTable::new();
    for (name,shape,size,alignment) in [("Unit",Shape::Unit,0,1),("Never",Shape::Never,0,1),("Bool",Shape::Bool,1,1),("String",Shape::Buffer{utf8:true},24,8),("Bytes",Shape::Buffer{utf8:false},24,8)] {
        table.insert(name.into(),TypeLayout{size,align:alignment,owned:matches!(shape,Shape::Buffer{..}),shape});
    }
    for (name,signed,bits) in [("I8",true,8),("I16",true,16),("I32",true,32),("I64",true,64),("U8",false,8),("U16",false,16),("U32",false,32),("U64",false,64),("Usize",false,64)] {
        table.insert(name.into(),TypeLayout{size:bits/8,align:(bits/8)as u32,owned:false,shape:Shape::Integer{signed,bits:bits as u16}});
    }
    let mut pending:Vec<_>=definitions.iter().collect();
    while !pending.is_empty(){
        let mut progress=false;
        pending.retain(|definition|{
            if definition.layout==Layout::Opaque{
                if il_graph::ResourceKind::from_nominal(&definition.entity_id).is_some()&&definition.kind==TypeKind::Record&&definition.parameters.is_empty()&&definition.variants.is_empty()&&definition.integer.is_none()&&definition.fields.iter().map(|f|(f.name.as_str(),f.type_ref.as_str())).eq([("slot","U64"),("generation","U64")]){table.insert(definition.entity_id.clone(),TypeLayout{size:16,align:8,owned:true,shape:Shape::Resource{resource:il_graph::ResourceKind::from_nominal(&definition.entity_id).unwrap()}});progress=true;return false;}
                return true;
            }
            let shape=match definition.kind{
                TypeKind::Unit=>Shape::Unit,TypeKind::Never=>Shape::Never,TypeKind::Bool=>Shape::Bool,
                TypeKind::Int=>{let Some(integer)=&definition.integer else{return true;};Shape::Integer{signed:integer.signed,bits:integer.bits as u16}},
                TypeKind::Usize=>Shape::Integer{signed:false,bits:64},TypeKind::String=>Shape::Buffer{utf8:true},TypeKind::Bytes=>Shape::Buffer{utf8:false},
                TypeKind::Record=>Shape::Record{names:definition.fields.iter().map(|f|f.name.clone()).collect(),fields:definition.fields.iter().map(|f|f.type_ref.clone()).collect(),offsets:vec![]},
                TypeKind::Tuple=>Shape::Tuple{fields:definition.parameters.clone(),offsets:vec![]},
                TypeKind::Sum=>Shape::Sum{payload:0,variants:definition.variants.iter().map(|v|VariantLayout{name:v.name.clone(),fields:v.fields.clone(),offsets:vec![]}).collect()},
                TypeKind::Option=>Shape::Sum{payload:0,variants:vec![VariantLayout{name:"None".into(),fields:vec![],offsets:vec![]},VariantLayout{name:"Some".into(),fields:definition.parameters.clone(),offsets:vec![]}]},
                TypeKind::Result=>Shape::Sum{payload:0,variants:vec![VariantLayout{name:"Ok".into(),fields:vec![definition.parameters[0].clone()],offsets:vec![]},VariantLayout{name:"Err".into(),fields:vec![definition.parameters[1].clone()],offsets:vec![]}]},
            };
            if let Some(layout)=compute(shape,&table){table.insert(definition.entity_id.clone(),layout);progress=true;false}else{true}
        });
        if !progress{return Err("opaque or recursively unsized native type is unsupported".into());}
    }
    Ok(table)
}
fn fields_layout(fields:&[String],table:&LayoutTable)->Option<(u64,u32,bool,Vec<u64>)>{
    let(mut size,mut alignment,mut owned,mut offsets)=(0,1,false,vec![]);
    for field in fields{let layout=table.get(field)?;alignment=alignment.max(layout.align);owned|=layout.owned;size=align(size,layout.align);offsets.push(size);size=size.checked_add(layout.size)?;if size>67_108_864{return None;}}
    Some((align(size,alignment),alignment,owned,offsets))
}
pub(crate) fn compute(mut shape:Shape,table:&LayoutTable)->Option<TypeLayout>{
    let(size,alignment,owned)=match &mut shape{
        Shape::Unit|Shape::Never=>(0,1,false),Shape::Bool=>(1,1,false),Shape::Integer{bits,..} if matches!(bits,8|16|32|64)=>(*bits as u64/8,*bits as u32/8,false),Shape::Integer{..}=>return None,
        Shape::Buffer{..}=>(24,8,true),Shape::Resource{..}=>(16,8,true),
        Shape::Record{fields,offsets,..}|Shape::Tuple{fields,offsets}=>{let(s,a,o,f)=fields_layout(fields,table)?;*offsets=f;(s,a,o)},
        Shape::Sum{payload,variants}=>{
            let(mut max_size,mut max_align,mut owned)=(0,1,false);
            for variant in variants{let(s,a,o,offsets)=fields_layout(&variant.fields,table)?;max_size=max_size.max(s);max_align=max_align.max(a);owned|=o;variant.offsets=offsets;}
            *payload=align(4,max_align);let a=max_align.max(4);(align(*payload+max_size,a),a,owned)
        }
    };Some(TypeLayout{size,align:alignment,owned,shape})
}
impl TypeLayout{
    pub fn scalar(&self)->Option<crate::Type>{match self.shape{Shape::Bool=>Some(crate::Type::int(8)),Shape::Integer{bits,..}=>Some(crate::Type::int(bits)),_=>None}}
    pub fn integer(&self)->Option<(bool,u16)>{match self.shape{Shape::Integer{signed,bits}=>Some((signed,bits)),_=>None}}
}
