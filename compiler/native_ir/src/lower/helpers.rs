use super::*;
impl Lower{
    pub(super) fn helpers(&mut self)->Result<()>{
        let types:Vec<_>=self.program.layouts.keys().cloned().collect();
        let mut resources=std::collections::BTreeSet::new();
        loop{let before=resources.len();for (ty,layout) in &self.program.layouts{let children=match &layout.shape{Shape::Resource{..}=>{resources.insert(ty.clone());continue;},Shape::Record{fields,..}|Shape::Tuple{fields,..}=>fields.clone(),Shape::Sum{variants,..}=>variants.iter().flat_map(|v|v.fields.clone()).collect(),_=>vec![]};if children.iter().any(|child|resources.contains(child)){resources.insert(ty.clone());}}if resources.len()==before{break;}}
        for ty in types{for operation in ["trace","drop","clone","json"]{
            if operation=="clone"&&resources.contains(&ty){continue;}
            let parameters=if operation=="clone"{vec!["ctx","out","src","entity","length"]}else{vec!["ctx","src"]};
            let mut signature=Signature{result:Type::Void,parameters:vec![Type::Ptr;parameters.len()]};if operation=="clone"{signature.parameters[4]=Type::int(64);}
            let mut b=Builder::new(helper(operation,&ty),format!("native.{operation}.{ty}"),signature,parameters.into_iter().map(str::to_owned).collect());
            if matches!(operation,"clone"|"json"){b.rt("shape_enter",vec![Operand::reg("ctx")],false);}
            self.helper_body(&mut b,operation,&ty,Operand::reg("src"),Operand::reg("out"))?;
            if matches!(operation,"clone"|"json"){b.rt("shape_leave",vec![Operand::reg("ctx")],false);}b.ret(None);self.program.functions.push(b.function);
        }}Ok(())
    }
    fn raw(&mut self,b:&mut Builder,value:&str){let(p,n)=self.text(value);b.rt("json_raw",vec![Operand::reg("ctx"),p,n],false);}
    fn json_string(&mut self,b:&mut Builder,value:&str){let(p,n)=self.text(value);b.rt("json_quoted",vec![Operand::reg("ctx"),p,n],false);}
    fn child(&mut self,b:&mut Builder,operation:&str,ty:&str,src:Operand,out:Operand){
        let mut args=vec![Operand::reg("ctx")];if operation=="clone"{args.push(out);}args.push(src);if operation=="clone"{args.extend([Operand::reg("entity"),Operand::reg("length")]);}b.call(helper(operation,ty),args,false);
    }
    fn helper_body(&mut self,b:&mut Builder,operation:&str,ty:&str,src:Operand,out:Operand)->Result<()>{
        let layout=self.program.layouts[ty].clone();let ctx=Operand::reg("ctx");
        if operation=="json"{self.raw(b,"{\"type\":");self.json_string(b,ty);self.raw(b,",\"data\":{\"kind\":");let kind=match layout.shape{Shape::Unit|Shape::Never=>"unit",Shape::Bool=>"bool",Shape::Integer{..}=>"integer",Shape::Buffer{utf8:true}=>"string",Shape::Buffer{utf8:false}=>"bytes",Shape::Resource{..}=>"resource",Shape::Record{..}=>"record",Shape::Tuple{..}=>"tuple",Shape::Sum{..}=>"variant"};self.json_string(b,kind);if !matches!(layout.shape,Shape::Unit|Shape::Never){self.raw(b,",\"value\":");}}
        if operation=="clone"&&!layout.owned{b.copy(out,src,layout.size);return Ok(());}
        if matches!(operation,"trace"|"drop")&&!layout.owned{return Ok(());}
        match layout.shape{
            Shape::Unit|Shape::Never=>{},
            Shape::Bool=>{if operation=="json"{let value=b.load(src,Type::int(8),1);let condition=b.cmp(Predicate::Ne,value,Operand::int(8,0));let yes=b.label("true");let no=b.label("false");let done=b.label("done");b.function.blocks[b.at].terminator=Terminator::CondBranch{condition,yes:yes.clone(),no:no.clone()};b.at=b.block(yes);self.raw(b,"true");b.goto(&done);b.at=b.block(no);self.raw(b,"false");b.goto(&done);b.at=b.block(done);}},
            Shape::Integer{signed,bits}=>{if operation=="json"{let value=b.load(src,Type::int(bits),layout.align);let value=b.cast(value,bits,64,signed);b.rt(if signed{"json_i64"}else{"json_u64"},vec![ctx,value],false);}},
            Shape::Resource{resource}=>{let name=match resource{il_graph::ResourceKind::File=>"file",il_graph::ResourceKind::Listener=>"listener",il_graph::ResourceKind::Stream=>"stream"};let symbol=match operation{"trace"=>format!("trace_{name}"),"drop"=>format!("{name}_drop"),"json"=>format!("json_{name}"),_=>return Err(self.error(ty,"E_UNSUPPORTED_FEATURE","resources cannot be cloned"))};b.rt(&symbol,vec![ctx,src],false);},
            Shape::Buffer{utf8}=>match operation{
                "trace"=>{b.rt("trace_buffer",vec![ctx,src],false);},
                "drop"=>{b.rt("buffer_free",vec![ctx,src],false);},
                "clone"=>{b.rt("buffer_clone",vec![ctx,out,src,Operand::reg("entity"),Operand::reg("length")],false);},
                "json"=>{b.rt("json_buffer",vec![ctx,src,Operand::int(32,utf8 as u8)],false);},_=>unreachable!()
            },
            Shape::Record{fields,offsets,..}|Shape::Tuple{fields,offsets}=>{
                if operation=="clone"{b.zero(out.clone(),layout.size);}if operation=="json"{self.raw(b,"[");}
                for(index,(field,offset))in fields.iter().zip(offsets).enumerate(){if operation=="json"&&index>0{self.raw(b,",");}let source=b.offset(src.clone(),offset);let target=if operation=="clone"{b.offset(out.clone(),offset)}else{Operand::Null};self.child(b,operation,field,source,target);}
                if operation=="json"{self.raw(b,"]");}
            }
            Shape::Sum{payload,variants}=>{
                let tag=b.load(src.clone(),Type::int(32),4);if operation=="clone"{b.zero(out.clone(),layout.size);b.store(out.clone(),tag.clone(),4);}
                let done=b.label("sum_done");let invalid=b.label("sum_invalid");let cases:Vec<_>=variants.iter().enumerate().map(|(index,_)|(index as u64,b.label("variant"))).collect();b.function.blocks[b.at].terminator=Terminator::Switch{value:tag,cases:cases.clone(),default:invalid.clone()};
                for((_,label),variant)in cases.into_iter().zip(variants){b.at=b.block(label);if operation=="json"{self.raw(b,"{\"tag\":");self.json_string(b,&variant.name);self.raw(b,",\"fields\":[");}for(index,(field,offset))in variant.fields.iter().zip(variant.offsets).enumerate(){if operation=="json"&&index>0{self.raw(b,",");}let source=b.offset(src.clone(),payload+offset);let target=if operation=="clone"{b.offset(out.clone(),payload+offset)}else{Operand::Null};self.child(b,operation,field,source,target);}if operation=="json"{self.raw(b,"]}");}b.goto(&done);}
                b.at=b.block(invalid);let entity=b.entity.clone();self.trap(b,ctx,"E_INVALID_DISCRIMINANT","invalid native sum discriminant",&entity);b.at=b.block(done);
            }
        }
        if operation=="json"{self.raw(b,"}}");}Ok(())
    }
}
