//! Helpers for safe(r) volatile access.

use core::intrinsics;
use core::ptr;

pub fn write<T>(r: &mut T, v: T) {
    unsafe {
        ptr::write_volatile(r, v);
    }
}

pub fn read<T>(r: &T) -> T {
    unsafe { ptr::read_volatile(r) }
}

pub fn zero<T>(d: &mut T) {
    mem_set(d, 0);
}

pub fn mem_set<T>(d: &mut T, b: u8) {
    unsafe {
        intrinsics::volatile_set_memory(d, b, 1);
    }
}

pub fn zero_slice<T>(bs: &mut [T]) {
    unsafe {
        intrinsics::volatile_set_memory(bs.as_mut_ptr(), 0, bs.len());
    }
}

pub fn copy<T>(dst: &mut T, src: &T) {
    unsafe {
        intrinsics::volatile_copy_memory(dst, src, 1);
    }
}

pub unsafe fn copy_ptr<T>(dst: &mut T, src: *const T) {
    unsafe {
        intrinsics::volatile_copy_memory(dst, src, 1);
    }
}

pub fn copy_slice<T>(dst: &mut [T], src: &[T]) {
    assert_eq!(dst.len(), src.len());
    unsafe {
        intrinsics::volatile_copy_memory(dst.as_mut_ptr(), src.as_ptr(), dst.len());
    }
}

pub mod bit {
    use super::{read, write};
    use core::ops::{BitAnd, BitOr, Not};

    pub fn set<T: BitOr<Output = T> + BitAnd<Output = T> + Not<Output = T>>(r: &mut T, v: T) {
        let tmp = read(r);
        write(r, tmp | v);
    }

    pub fn clear<T: BitAnd<Output = T> + BitOr<Output = T> + Not<Output = T>>(r: &mut T, v: T) {
        let tmp = read(r);
        write(r, tmp & !v);
    }
}
