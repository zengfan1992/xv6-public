use crate::arch;
use crate::kalloc;
use crate::kmem;
use crate::xapic;
use core::mem;
use core::ptr;
use core::sync::atomic::AtomicBool;
use core::time::Duration;

const VECTOR: u8 = 7;

pub unsafe fn init() {
    use core::intrinsics::volatile_copy_memory;

    extern "C" {
        static apentry: [u8; 0];
        static eapentry: [u8; 0];
    }
    let phys_page: u64 = u64::from(VECTOR) * arch::PAGE_SIZE as u64;
    let dst: *mut u8 = kmem::phys_to_ptr_mut::<u8>(phys_page);
    let src = unsafe { apentry.as_ptr() };
    let end = unsafe { eapentry.as_ptr() };
    let len = end.addr() - src.addr();
    unsafe {
        volatile_copy_memory(dst, src, len);
    }
}

pub unsafe fn start_others(cpus: &[u32]) {
    for (id, &cpu) in cpus.iter().enumerate() {
        unsafe {
            start1(id, cpu);
        }
    }
}

unsafe fn start1(id: usize, apic_id: u32) {
    use core::sync::atomic::Ordering;
    fn wait(semaphore: &AtomicBool, timeout: Duration) -> bool {
        for _ in 0..timeout.as_micros() {
            arch::sleep(USEC);
            if semaphore.load(Ordering::Acquire) {
                return true;
            }
            arch::cpu_relax();
        }
        return false;
    }

    const USEC: Duration = Duration::from_micros(1);
    const MSEC: Duration = Duration::from_millis(1);

    if apic_id == arch::mycpu_id() {
        return;
    }
    let phys_page: u64 = u64::from(VECTOR) * arch::PAGE_SIZE as u64;
    let apentry_page: *mut u8 = kmem::phys_to_ptr_mut::<u8>(phys_page);
    let ptrs_offset = arch::PAGE_SIZE - 3 * mem::size_of::<usize>();
    #[allow(clippy::cast_ptr_alignment)]
    let ptrs = apentry_page.wrapping_add(ptrs_offset) as *mut usize;
    let percpu = kalloc::alloc().expect("start_others: AP percpu alloc failed");
    unsafe {
        let percpu = percpu as *mut arch::Page;
        ptr::write_volatile(ptrs, percpu.addr());
        ptr::write_volatile(ptrs.add(1), id);
    }
    let semaphore = AtomicBool::new(false);
    unsafe {
        let semaphore = &semaphore as *const AtomicBool;
        ptr::write_volatile(ptrs.add(2), semaphore.addr());
        xapic::send_init_ipi(apic_id);
    }
    arch::sleep(10 * MSEC);
    for &timeout in [200 * USEC, 200 * USEC].iter() {
        unsafe {
            xapic::send_sipi(apic_id, VECTOR);
        }
        if wait(&semaphore, timeout) {
            return;
        }
    }
    if !wait(&semaphore, 10 * MSEC) {
        panic!("failed to start cpu{} (APIC {})", id, apic_id);
    }
}
