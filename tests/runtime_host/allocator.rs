use il_runtime_allocator::*;
use il_runtime_startup::Faults;

#[test]
fn cloned_allocator_shares_attempts_quota_and_release(){
    let allocator=Allocator::new(None,Some(5));let shared=allocator.clone();let first=allocator.copy(b"abc").unwrap();assert_eq!(first.as_slice(),b"abc");assert_eq!(shared.live_bytes(),3);assert_eq!(shared.allocate(3).err(),Some(AllocFailure::ResourceLimit));drop(first);let mut second=shared.allocate(5).unwrap();assert_eq!(allocator.live_bytes(),5);second.truncate(2);assert_eq!(allocator.live_bytes(),2);assert_eq!(allocator.peak_bytes(),5);drop(second);assert_eq!(allocator.live_bytes(),0);assert_eq!(allocator.live_allocations(),0);assert_eq!(allocator.attempts(),3);
}

#[test]
fn allocation_fault_thresholds_are_execution_local_and_zero_based(){
    for threshold in [0,1,2]{let faults=Faults{allocation_fail_after:Some(threshold),io_max_chunk:None,io_fail_after:None,accept_fail_after:None};let allocator=Allocator::new(Some(&faults),None);for _ in 0..threshold{drop(allocator.allocate(1).unwrap());}assert_eq!(allocator.allocate(1).err(),Some(AllocFailure::OutOfMemory));assert_eq!(allocator.allocate(1).err(),Some(AllocFailure::OutOfMemory));assert_eq!(allocator.attempts(),threshold+2);}
    assert!(Allocator::new(None,None).allocate(1).is_ok());
}

#[test]
fn empty_owned_buffers_have_distinct_identity_without_payload_charge(){let allocator=Allocator::new(None,Some(0));let a=allocator.allocate(0).unwrap();let b=allocator.allocate(0).unwrap();assert_ne!(a.as_ptr(),b.as_ptr());assert_eq!(allocator.live_bytes(),0);assert_eq!(allocator.live_allocations(),2);drop(a);drop(b);assert_eq!(allocator.live_allocations(),0);}
