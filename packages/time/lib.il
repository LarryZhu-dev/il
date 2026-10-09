@id("time") module time visibility public imports [core] {
 @id("time.deadline_after_ms") fn deadline_after_ms(now:U64,milliseconds:U64)->core.Deadline {let instant:U64=now+milliseconds*1000000;return core.Deadline::At(instant);}
 @id("time.elapsed_ns") fn elapsed_ns(start:U64,finish:U64)->U64 {return finish-start;}
 @id("time.expired") fn expired(deadline:core.Deadline,now:U64)->Bool {match deadline {Infinite=>{return false;}At(instant)=>{return now>=instant;}}}
}
