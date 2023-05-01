use crate::arch;
use crate::kalloc;
use crate::param;
use crate::sd;
use crate::sleeplock::Sleeplock;
use crate::spinlock::SpinMutex as Mutex;
use crate::Result;
use bitflags::bitflags;
use core::cell::{Cell, RefCell};
use core::ptr::null_mut;

bitflags! {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct BufFlags: u32 {
        const EMPTY = 0;
        const VALID = 1 << 1; // buffer has been read from disk
        const DIRTY = 1 << 2; // buffer needs to be written to disk
    }
}

const LIST_NONE: usize = usize::max_value();

struct BCache {
    head: usize,
    tail: usize,
    bufs: [Buf; param::NBUF],
}

impl BCache {
    pub const fn new() -> BCache {
        BCache {
            head: 0,
            tail: 0,
            bufs: [const { Buf::new() }; param::NBUF],
        }
    }
}

// Interior mutable block metadata.  Protected by the
// buffer cache lock.
#[derive(Debug)]
pub struct BufMeta {
    dev: u32,
    blockno: u64,
    ref_cnt: u32,
    self_ptr: usize,
    prev: usize,
    next: usize,
}

impl BufMeta {
    pub const fn empty() -> BufMeta {
        BufMeta {
            dev: 0,
            blockno: 0,
            ref_cnt: 0,
            self_ptr: LIST_NONE,
            prev: LIST_NONE,
            next: LIST_NONE,
        }
    }
}

// A buffer. Note that the flags member is protected by
// the sleep lock, while `qnext` is only accessed in the
// storage driver.
#[derive(Debug)]
pub struct Buf {
    lock: Sleeplock,
    flags: Cell<BufFlags>,
    meta: RefCell<BufMeta>,
    qnext: Cell<usize>,
    data: *mut arch::Page,
}

impl Buf {
    pub const fn new() -> Buf {
        Buf {
            lock: Sleeplock::new("buffer"),
            flags: Cell::new(BufFlags::EMPTY),
            meta: RefCell::new(BufMeta::empty()),
            qnext: Cell::new(LIST_NONE),
            data: null_mut(),
        }
    }

    pub fn as_chan(&self) -> usize {
        self as *const _ as usize
    }

    pub fn data(&self) -> *mut arch::Page {
        self.data
    }

    pub fn data_page(&self) -> &arch::Page {
        unsafe { self.data.as_ref().unwrap() }
    }

    pub fn data_page_mut(&self) -> &mut arch::Page {
        unsafe { self.data.as_mut().unwrap() }
    }

    pub fn data_ref(&self) -> &[u8] {
        unsafe { self.data.as_ref().unwrap().as_slice() }
    }

    #[allow(clippy::mut_from_ref)]
    pub fn data_mut(&self) -> &mut [u8] {
        unsafe { self.data.as_mut().unwrap().as_mut() }
    }

    pub fn flags(&self) -> BufFlags {
        self.flags.get()
    }

    pub fn set_flags(&self, flags: BufFlags) {
        self.flags.set(flags);
    }

    pub fn is_locked(&self) -> bool {
        self.lock.holding()
    }

    pub fn read(&'static self) {
        if !self.flags().contains(BufFlags::VALID) {
            sd::rdwr(self);
        }
    }

    pub fn blockno(&self) -> u64 {
        self.meta.borrow().blockno
    }

    pub fn qnext(&self) -> usize {
        self.qnext.get()
    }

    pub fn write(&'static self) {
        assert!(self.is_locked());
        let flags = self.flags() | BufFlags::DIRTY;
        self.set_flags(flags);
        sd::rdwr(self);
    }

    // The seeming misspelling of this function name is deliberate.
    // One must occasionally make homage to one's inspirations.
    pub fn relse(&self) {
        assert!(self.lock.holding());
        self.lock.release();

        BCACHE.with_lock(|cache| {
            let mut meta = self.meta.borrow_mut();
            meta.ref_cnt -= 1;
            if meta.ref_cnt == 0 {
                if cache.head != meta.self_ptr {
                    if cache.tail == meta.self_ptr {
                        cache.tail = meta.prev;
                    }
                    if meta.next != LIST_NONE {
                        let next = &cache.bufs[meta.next];
                        next.meta.borrow_mut().prev = meta.prev;
                    }
                    if meta.prev != LIST_NONE {
                        let prev = &cache.bufs[meta.prev];
                        prev.meta.borrow_mut().next = meta.next;
                    }
                    let head = &cache.bufs[cache.head];
                    let mut meta_head = head.meta.borrow_mut();
                    meta.prev = LIST_NONE;
                    meta_head.prev = meta.self_ptr;
                    meta.next = meta_head.self_ptr;
                    cache.head = meta.self_ptr;
                }
            }
        });
    }
}

static BCACHE: Mutex<BCache> = Mutex::new("bufs", BCache::new());

pub unsafe fn init() {
    let mut cache = BCACHE.lock();
    let len = cache.bufs.len();
    assert!(len > 1, "insufficient number of buffers");
    for k in 0..len {
        let b = &mut cache.bufs[k];
        let mut meta = b.meta.borrow_mut();
        if k + 1 < len {
            meta.next = k + 1;
        }
        meta.self_ptr = k;
        if k > 0 {
            meta.prev = k - 1;
        }
        let data = kalloc::alloc().expect("buffer data alloc failed");
        b.data = data;
    }
    cache.head = 0;
    cache.tail = len - 1;
}

fn bget(dev: u32, blockno: u64) -> Result<&'static Buf> {
    let buf = BCACHE.with_lock(|cache| {
        // Is the block already cached?
        let mut p = cache.head;
        while p != LIST_NONE {
            let b = &cache.bufs[p];
            let mut meta = b.meta.borrow_mut();
            if meta.dev == dev && meta.blockno == blockno {
                meta.ref_cnt += 1;
                return Ok(unsafe { &*(b as *const Buf) });
            }
            p = meta.next;
        }
        // Not in the cache, so recycle an unused buffer.
        // Even if ref_cnt is 0, DIRTY indicates a buffer is in use
        // because the log has modified it but not committed it yet.
        p = cache.tail;
        while p != LIST_NONE {
            let b = &cache.bufs[p];
            let mut meta = b.meta.borrow_mut();
            if meta.ref_cnt == 0 && !b.flags().contains(BufFlags::DIRTY) {
                meta.dev = dev;
                meta.blockno = blockno;
                meta.ref_cnt = 1;
                b.set_flags(BufFlags::EMPTY);
                return Ok(unsafe { &*(b as *const Buf) });
            }
            p = meta.prev;
        }
        Err("bget: no buffers")
    })?;
    buf.lock.acquire();
    Ok(buf)
}

pub fn dequeue(head: Option<&Buf>) -> Option<(&Buf, Option<&Buf>)> {
    if let Some(head) = head {
        let next = BCACHE
            .with_lock(|cache| {
                let ni = head.qnext();
                let next = (ni != LIST_NONE).then(|| unsafe { &*(&cache.bufs[ni] as *const Buf) });
                Ok::<Option<&'static Buf>, ()>(next)
            })
            .expect("queue not corrupt");
        Some((head, next))
    } else {
        None
    }
}

pub fn enqueue<'a>(head: Option<&'a Buf>, buf: &'a Buf) -> Option<&'a Buf> {
    buf.qnext.set(LIST_NONE);
    if let Some(head) = head {
        let bi = buf.meta.borrow().self_ptr;
        BCACHE.with_lock(|cache| {
            let mut head = head;
            while head.qnext() != LIST_NONE {
                head = &cache.bufs[head.qnext()];
            }
            head.qnext.set(bi);
        });
        Some(head)
    } else {
        Some(buf)
    }
}

pub fn read(dev: u32, blockno: u64) -> Result<&'static Buf> {
    let buf = bget(dev, blockno)?;
    buf.read();
    Ok(buf)
}

pub fn with_block<U, F: FnMut(&'static Buf) -> U>(
    dev: u32,
    blockno: u64,
    mut thunk: F,
) -> Result<U> {
    let bp = read(dev, blockno)?;
    let r = thunk(bp);
    bp.relse();
    Ok(r)
}
