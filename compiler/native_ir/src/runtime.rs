use crate::*;
pub fn runtime_externals()->Vec<External>{
    let p=Type::Ptr;let i=Type::int(32);let n=Type::int(64);let v=Type::Void;
    let rows=vec![
        ("context_new",p.clone(),vec![i.clone(),n.clone(),i.clone(),n.clone(),n.clone(),n.clone(),i.clone()],false),
        ("finish",v.clone(),vec![p.clone()],false),
        ("enter",v.clone(),vec![p.clone(),p.clone(),n.clone()],false),
        ("leave",v.clone(),vec![p.clone()],false),
        ("tick",v.clone(),vec![p.clone(),p.clone(),n.clone()],false),
        ("location",v.clone(),vec![p.clone(),p.clone(),n.clone()],false),
        ("trap",v.clone(),vec![p.clone(),p.clone(),n.clone(),p.clone(),n.clone(),p.clone(),n.clone()],true),
        ("buffer_new",v.clone(),vec![p.clone(),p.clone(),p.clone(),n.clone(),p.clone(),n.clone()],false),
        ("buffer_free",v.clone(),vec![p.clone(),p.clone()],false),
        ("buffer_clone",v.clone(),vec![p.clone(),p.clone(),p.clone(),p.clone(),n.clone()],false),
        ("concat",i.clone(),vec![p.clone(),p.clone(),p.clone(),p.clone(),p.clone(),n.clone()],false),
        ("print_i64",i.clone(),vec![p.clone(),n.clone()],false),
        ("print_buffer",i.clone(),vec![p.clone(),p.clone()],false),
        ("trace_begin",v.clone(),vec![p.clone(),i.clone(),p.clone(),n.clone()],false),
        ("trace_buffer",v.clone(),vec![p.clone(),p.clone()],false),
        ("trace_end",v.clone(),vec![p.clone()],false),
        ("json_raw",v.clone(),vec![p.clone(),p.clone(),n.clone()],false),
        ("json_quoted",v.clone(),vec![p.clone(),p.clone(),n.clone()],false),
        ("json_i64",v.clone(),vec![p.clone(),n.clone()],false),
        ("json_u64",v.clone(),vec![p.clone(),n.clone()],false),
        ("json_bytes",v.clone(),vec![p.clone(),p.clone(),n.clone()],false),
        ("json_buffer",v.clone(),vec![p.clone(),p.clone(),i.clone()],false),
        ("shape_begin",v.clone(),vec![p.clone(),i.clone()],false),
        ("shape_enter",v.clone(),vec![p.clone()],false),
        ("shape_leave",v,vec![p],false),
    ];rows.into_iter().map(|(name,result,parameters,noreturn)|External{symbol:format!("il_rt_{name}"),signature:Signature{result,parameters},noreturn}).collect()
}
