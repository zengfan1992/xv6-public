use crate::bio;
use crate::fs;
use crate::param;
use crate::spinlock::SpinMutex as Mutex;
use crate::volatile;
use core::ptr;
use core::slice;
use static_assertions::const_assert;

static LOG_STATE: Mutex<LogState> = Mutex::new("log", LogState::new());
//static LOG: RacyCell<Log> = RacyCell::new(Log::new());
static mut LOG: Log = Log::new();

/// Simple logging that allows concurrent FS system calls.
///
/// A log transaction contains the updates of multiple FS system
/// calls. The logging system only commits when there are
/// no FS system calls active. Thus there is never
/// any reasoning required about whether a commit might
/// write an uncommitted system call's updates to disk.
///
/// A system call should call op::begin()/op::end() to mark
/// its start and end. Usually op::begin() just increments
/// the count of in-progress FS system calls and returns.
/// But if it thinks the log is close to running out, it
/// sleeps until the last outstanding op::end() commits.
///
/// The log is a physical re-do log containing disk blocks.
/// The on-disk log format:
///   header block, containing block #s for block A, B, C, ...
///   block A
///   block B
///   block C
///   ...
/// Log appends are synchronous.
///
/// Contents of the "blocks" array are used for both the
/// stored header block addresses and keeping track of logged
/// block numbers in memory before commit.
#[repr(C)]
struct Log {
    dev: u32,
    start: u64,
    size: usize,
    len: usize,
    blocks: [u64; param::LOGSIZE],
}
const_assert!(core::mem::size_of::<[u64; param::LOGSIZE + 1]>() <= fs::BSIZE);

impl Log {
    pub const fn new() -> Log {
        Log {
            dev: 0,
            start: 0,
            size: 0,
            len: 0,
            blocks: [0; param::LOGSIZE],
        }
    }

    pub fn set_metadata(&mut self, dev: u32, start: u64, size: usize) {
        self.dev = dev;
        self.start = start;
        self.size = size;
    }

    fn header(&self) -> &[u64] {
        &self.blocks[..self.len]
    }

    pub fn insert(&mut self, blockno: u64) {
        if !self.header().contains(&blockno) {
            self.blocks[self.len] = blockno;
            self.len += 1;
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn is_full(&self) -> bool {
        self.len >= self.size - 1 || self.len >= param::LOGSIZE
    }

    pub fn read(&mut self) {
        bio::with_block(self.dev, self.start, |hb| {
            let src = unsafe {
                let ptr = hb.data() as *const u64;
                let len = ptr::read(ptr) as usize;
                let src = slice::from_raw_parts(ptr, len + 1);
                &src[1..]
            };
            if src.len() >= param::LOGSIZE || src.len() >= self.size - 1 {
                panic!("corrupt log too large: {}", src.len());
            }
            self.clear();
            for &blockno in src {
                self.insert(blockno);
            }
        })
        .unwrap();
    }

    pub fn write(&self) {
        bio::with_block(self.dev, self.start, |hb| {
            let dst = unsafe {
                let dst = hb.data_mut().as_mut_ptr() as *mut u64;
                let len = param::LOGSIZE + 1;
                slice::from_raw_parts_mut(dst, len)
            };
            volatile::write(&mut dst[0], self.len() as u64);
            volatile::copy_slice(&mut dst[1..1 + self.len], &self.blocks[..self.len]);
            hb.write();
        })
        .unwrap();
    }

    fn sync(&mut self) {
        for (tail, &blockno) in self.header().iter().enumerate() {
            let logblockno = self.start + tail as u64 + 1;
            bio::with_block(self.dev, logblockno, |to| {
                bio::with_block(self.dev, blockno, |from| {
                    let dst = to.data_mut();
                    let src = from.data_ref();
                    volatile::copy_slice(dst, src);
                    to.write();
                })
                .unwrap();
            })
            .unwrap();
        }
    }

    fn commit(&self) {
        for (tail, blockno) in self.header().iter().enumerate() {
            let logblockno = self.start + tail as u64 + 1;
            bio::with_block(self.dev, logblockno, |from| {
                bio::with_block(self.dev, *blockno, |to| {
                    let src = from.data_ref();
                    let dst = to.data_mut();
                    volatile::copy_slice(dst, src);
                    to.write();
                })
                .unwrap();
            })
            .unwrap();
        }
    }
}

struct LogState {
    outstanding: usize,
    committing: bool,
}

impl LogState {
    const fn new() -> LogState {
        LogState {
            outstanding: 0,
            committing: false,
        }
    }

    fn as_chan(&self) -> usize {
        (self as *const Self).addr()
    }
}

pub mod op {
    use super::{LOG, LOG_STATE};
    use crate::param;
    use crate::proc::{self, myproc};

    pub struct Transaction {}

    pub fn begin() -> Transaction {
        let mut state = LOG_STATE.lock();
        loop {
            if state.committing {
                myproc().sleep(state.as_chan(), &LOG_STATE);
                continue;
            }
            let needed = (state.outstanding + 1) * param::MAXOPBLOCKS;
            let len = unsafe { LOG.len() };
            if len + needed > param::LOGSIZE {
                myproc().sleep(state.as_chan(), &LOG_STATE);
                continue;
            }
            state.outstanding += 1;
            break;
        }
        Transaction {}
    }

    pub fn end(mut _txn: Transaction) {
        let do_commit = {
            let mut state = LOG_STATE.lock();
            if state.committing {
                panic!("op end during commit");
            }
            state.outstanding -= 1;
            if state.outstanding == 0 {
                state.committing = true;
                true
            } else {
                proc::wakeup(state.as_chan());
                false
            }
        };
        if do_commit {
            commit();
            let mut state = LOG_STATE.lock();
            state.committing = false;
            proc::wakeup(state.as_chan())
        }
    }

    fn commit() {
        let log = unsafe { &mut LOG };
        if !log.is_empty() {
            log.sync();
            log.write();
            log.commit();
            log.clear();
            log.write();
        }
    }
}

pub fn with_op<U, F: FnMut() -> U>(mut thunk: F) -> U {
    let txn = op::begin();
    let r = thunk();
    op::end(txn);
    r
}

pub unsafe fn init(dev: u32, sb: &fs::Superblock) {
    fn recover() {
        let log = unsafe { &mut LOG };
        log.read();
        log.commit();
        log.clear();
        log.write();
    }
    unsafe {
        LOG.set_metadata(dev, sb.log_start, sb.nlog as usize);
    }
    recover();
}

pub fn write(bp: &bio::Buf) {
    assert!(bp.is_locked());
    let state = LOG_STATE.lock();
    let log = unsafe { &mut LOG };
    if log.is_full() {
        panic!("transaction too big");
    }
    if state.outstanding < 1 {
        panic!("logged write outside of transaction");
    }
    log.insert(bp.blockno());
    let flags = bp.flags() | bio::BufFlags::DIRTY;
    bp.set_flags(flags)
}
