#![no_std]
//! Freestanding Linux x86-64 runtime: no allocator, libc, context or managed values.
use core::arch::{asm,global_asm};
global_asm!(".global _start", ".type _start,@function", "_start:", "xor rbp, rbp", "and rsp, -16", "call il_min_start", "ud2");
unsafe extern "C"{fn il_entry_main()->i32;}
#[inline(always)]
unsafe fn syscall3(number:usize,a:usize,b:usize,c:usize)->isize{let result:isize;unsafe{asm!("syscall",inlateout("rax")number=>result,in("rdi")a,in("rsi")b,in("rdx")c,lateout("rcx")_,lateout("r11")_,options(nostack));}result}
fn exit(code:i32)->!{unsafe{syscall3(60,code as usize,0,0);}loop{core::hint::spin_loop();}}
#[no_mangle]
pub extern "C" fn il_min_start()->!{
    // Ignore SIGPIPE so stdout failures become the declared IoError::Write.
    let action=[1usize,0,0,0];let result:isize;
    unsafe{asm!("syscall",inlateout("rax")13usize=>result,in("rdi")13usize,in("rsi")action.as_ptr(),in("rdx")0usize,in("r10")8usize,lateout("rcx")_,lateout("r11")_,options(nostack));}
    if result<0{exit(101);}exit(unsafe{il_entry_main()})
}
unsafe fn write_all(fd:usize,bytes:*const u8,length:usize)->bool{let mut offset=0;while offset<length{let result=unsafe{syscall3(1,fd,bytes.add(offset)as usize,length-offset)};if result==-4{continue;}if result<=0{return false;}offset+=result as usize;}true}
#[no_mangle]
pub unsafe extern "C" fn il_min_trap(code:*const u8,code_len:u64,_message:*const u8,_message_len:u64,entity:*const u8,entity_len:u64)->!{unsafe{write_all(2,code,code_len as usize);write_all(2,b" at ".as_ptr(),4);write_all(2,entity,entity_len as usize);write_all(2,b"\n".as_ptr(),1);}exit(101)}
#[no_mangle]
pub extern "C" fn il_min_print_i64(value:i64)->u32{let mut buffer=[0u8;20];let mut at=20usize;let mut remaining=value.unsigned_abs();loop{at-=1;buffer[at]=(remaining%10)as u8+b'0';remaining/=10;if remaining==0{break;}}if value<0{at-=1;buffer[at]=b'-';}if unsafe{write_all(1,buffer.as_ptr().add(at),20-at)}{0}else{2}}
#[no_mangle]
pub unsafe extern "C" fn memset(destination:*mut u8,value:i32,length:usize)->*mut u8{for index in 0..length{unsafe{destination.add(index).write_volatile(value as u8);}}destination}
#[no_mangle]
pub unsafe extern "C" fn memmove(destination:*mut u8,source:*const u8,length:usize)->*mut u8{if destination as usize<=source as usize{for index in 0..length{unsafe{destination.add(index).write_volatile(source.add(index).read_volatile());}}}else{for index in (0..length).rev(){unsafe{destination.add(index).write_volatile(source.add(index).read_volatile());}}}destination}
#[no_mangle]
pub unsafe extern "C" fn memcpy(destination:*mut u8,source:*const u8,length:usize)->*mut u8{unsafe{memmove(destination,source,length)}}
#[panic_handler]
fn panic(_info:&core::panic::PanicInfo<'_>)->!{exit(101)}
