use crate::arch;
use crate::kbd;
use crate::proc::{self, Proc};
use crate::sd;
use crate::spinlock::SpinMutex as Mutex;
use crate::uart;
use crate::volatile;
use crate::xapic;
use crate::Result;

pub(crate) const INTR0: u32 = 32;
const KBD_INTR: u32 = INTR0 + kbd::INTR;
const EIA0_INTR: u32 = INTR0 + uart::INTR_EIA0;
const TIMER_INTR: u32 = INTR0 + xapic::INTR_TIMER;
const SD_INTR: u32 = INTR0 + sd::INTR_SD0;

const PAGE_FAULT: u32 = 14;

static TICKS: Mutex<u64> = Mutex::new("time", 0);

pub fn ticks() -> u64 {
    *TICKS.lock()
}

pub fn tickchan() -> usize {
    (&TICKS as *const Mutex<u64>).addr()
}

pub fn ticksleep(proc: &Proc, nticks: u64) -> Result<()> {
    let ticks0 = ticks();
    TICKS.with_lock(|ticks| {
        while volatile::read(ticks) - ticks0 < nticks {
            if proc.dead() {
                return Err("killed");
            }
            proc.sleep(tickchan(), &TICKS)
        }
        Ok(())
    })
}

pub extern "C" fn trap(vecnum: u32, frame: &mut arch::TrapFrame) {
    match vecnum {
        PAGE_FAULT => {
            if !frame.is_user() {
                panic!(
                    "page fault at {:x}, rip = {:x}, error = {:x}",
                    arch::fault_addr(),
                    frame.rip,
                    frame.error
                );
            }
            proc::myproc().kill();
        }
        KBD_INTR => {
            assert!(arch::mycpu_id() == 0);
            kbd::interrupt();
        }
        EIA0_INTR => {
            assert!(arch::mycpu_id() == 0);
            uart::interrupt();
        }
        TIMER_INTR => {
            if arch::mycpu_id() == 0 {
                TICKS.with_lock(|ticks| {
                    *ticks = ticks.wrapping_add(1);
                    proc::wakeup(tickchan());
                });
            }
            unsafe {
                xapic::eoi();
            }
        }
        SD_INTR => {
            assert!(arch::mycpu_id() == 0);
            sd::interrupt();
        }
        _ => {
            if !frame.is_user() || proc::try_myproc().is_none() {
                crate::println!(
                    "unexpected trap on cpu {} frame: {:#x?}!",
                    arch::mycpu_id(),
                    frame
                );
                panic!("unanticipated interrupt");
            }
            let proc = proc::myproc();
            proc.kill();
        }
    }

    // Force process exit if it has been killed and is in user
    // space.  If it is still executing in the kernel, let it
    // keep running until it gets to the regular return.
    if frame.is_user() {
        proc::die_if_dead();
    }

    // Force process to give up CPU on clock tick.  If
    // interrupts were on while locks held, would need to check
    // nlock.
    if vecnum == TIMER_INTR {
        proc::yield_if_running();
    }

    // Check if the process has been killed since we yielded.
    if frame.is_user() {
        proc::die_if_dead();
    }
}

static mut IDT: arch::IDT = arch::IDT::empty();

pub unsafe fn vector_init() {
    unsafe {
        IDT.init();
    }
}

pub unsafe fn init() {
    unsafe {
        IDT.load();
    }
}
