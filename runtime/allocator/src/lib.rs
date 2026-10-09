//! Explicit allocation attempts, quotas and RAII payload ownership shared by execution engines.
use il_runtime_startup::Faults;
use std::{cell::RefCell,rc::Rc};

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum AllocFailure{OutOfMemory,CapacityOverflow,ResourceLimit}
struct State{attempts:u64,fail_after:Option<u64>,limit:Option<u64>,live:u64,peak:u64,allocations:u64}
#[derive(Clone)]
pub struct Allocator{state:Rc<RefCell<State>>}
pub struct OwnedBuffer{bytes:Vec<u8>,charged:u64,state:Rc<RefCell<State>>}
impl Allocator{
    pub fn new(faults:Option<&Faults>,heap_limit:Option<u64>)->Self{Self{state:Rc::new(RefCell::new(State{attempts:0,fail_after:faults.and_then(|f|f.allocation_fail_after),limit:heap_limit,live:0,peak:0,allocations:0}))}}
    pub fn allocate(&self,length:usize)->Result<OwnedBuffer,AllocFailure>{
        if length>isize::MAX as usize{return Err(AllocFailure::CapacityOverflow)}
        let mut state=self.state.borrow_mut();let attempt=state.attempts;state.attempts=state.attempts.saturating_add(1);
        if state.fail_after.is_some_and(|limit|attempt>=limit){return Err(AllocFailure::OutOfMemory)}
        if state.limit.is_some_and(|limit|length as u64>limit.saturating_sub(state.live)){return Err(AllocFailure::ResourceLimit)}
        let mut bytes=Vec::new();bytes.try_reserve_exact(length.max(1)).map_err(|_|AllocFailure::OutOfMemory)?;bytes.resize(length,0);
        state.live+=length as u64;state.peak=state.peak.max(state.live);state.allocations+=1;
        Ok(OwnedBuffer{bytes,charged:length as u64,state:self.state.clone()})
    }
    /// Check a prospective guest result without consuming an allocation attempt.
    pub fn check_capacity(&self,length:usize)->Result<(),AllocFailure>{
        if length>isize::MAX as usize{return Err(AllocFailure::CapacityOverflow)}
        let state=self.state.borrow();if state.limit.is_some_and(|limit|length as u64>limit.saturating_sub(state.live)){return Err(AllocFailure::ResourceLimit)}Ok(())
    }
    pub fn copy(&self,bytes:&[u8])->Result<OwnedBuffer,AllocFailure>{let mut buffer=self.allocate(bytes.len())?;buffer.as_mut_slice().copy_from_slice(bytes);Ok(buffer)}
    pub fn attempts(&self)->u64{self.state.borrow().attempts}
    pub fn live_bytes(&self)->u64{self.state.borrow().live}
    pub fn peak_bytes(&self)->u64{self.state.borrow().peak}
    pub fn live_allocations(&self)->u64{self.state.borrow().allocations}
}
impl OwnedBuffer{
    pub fn as_slice(&self)->&[u8]{&self.bytes}
    pub fn as_mut_slice(&mut self)->&mut[u8]{&mut self.bytes}
    pub fn as_ptr(&self)->*const u8{self.bytes.as_ptr()}
    pub fn as_mut_ptr(&mut self)->*mut u8{self.bytes.as_mut_ptr()}
    pub fn len(&self)->usize{self.bytes.len()}
    pub fn is_empty(&self)->bool{self.bytes.is_empty()}
    pub fn capacity(&self)->usize{self.bytes.capacity()}
    pub fn truncate(&mut self,length:usize){if length<self.bytes.len(){self.bytes.truncate(length);let new=length as u64;self.state.borrow_mut().live-=self.charged-new;self.charged=new;}}
}
impl Drop for OwnedBuffer{fn drop(&mut self){let mut state=self.state.borrow_mut();state.live-=self.charged;state.allocations-=1;}}
