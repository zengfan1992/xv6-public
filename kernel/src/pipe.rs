use crate::arch::{self, Page};
use crate::file::{self, Like};
use crate::kalloc;
use crate::proc::{self, myproc};
use crate::spinlock::SpinMutex as Mutex;
use crate::volatile;
use crate::Result;
use core::mem;
use core::ptr;
use core::sync::atomic::{AtomicBool, Ordering};
use static_assertions::const_assert;

const fn paspace() -> usize {
    const PASIZE: usize = mem::size_of::<PipeAlloc>();
    const PALIGN: usize = mem::align_of::<PipeAlloc>();
    const ALIGN: usize = 0usize.wrapping_sub(PASIZE % PALIGN);
    PASIZE.wrapping_add(ALIGN)
}
static PIPES: Mutex<Option<&'static mut PipeSlab>> = Mutex::new("pipes", None);

pub unsafe fn init() {
    let mut pipes = PIPES.lock();
    let slab = PipeSlab::new().expect("allocated initial pipe slab");
    *pipes = Some(slab);
}

#[repr(C)]
pub struct Pipe {
    nread: usize,
    nwrite: usize,
    data: *mut Page,
    read_open: bool,
    write_open: bool,
}

impl Pipe {
    pub const fn new() -> Pipe {
        Pipe {
            nread: 0,
            nwrite: 0,
            data: ptr::null_mut(),
            read_open: false,
            write_open: false,
        }
    }

    fn reset(&mut self, page: &mut Page) {
        self.data = page.as_ptr_mut();
        self.read_open = true;
        self.write_open = true;
        self.nread = 0;
        self.nwrite = 0;
    }

    pub fn read_chan(&self) -> usize {
        (&self.nread as *const usize).addr()
    }

    pub fn write_chan(&self) -> usize {
        (&self.nwrite as *const usize).addr()
    }

    fn data(&self) -> &[u8] {
        unsafe { self.data.as_ref().unwrap().as_slice() }
    }

    fn data_mut(&mut self) -> &mut [u8] {
        unsafe { self.data.as_mut().unwrap().as_mut() }
    }

    pub fn is_empty(&self) -> bool {
        self.nread == self.nwrite
    }

    pub fn readable(&self) -> bool {
        !self.is_empty() || !self.write_open
    }

    pub fn read_byte(&mut self) -> u8 {
        assert!(!self.is_empty());
        let data = self.data();
        let b = volatile::read(&data[self.nread % data.len()]);
        self.nread = self.nread.wrapping_add(1);
        b
    }

    pub fn is_full(&self) -> bool {
        let data = self.data();
        self.nread + data.len() == self.nwrite
    }

    pub fn broken(&self) -> bool {
        !self.read_open
    }

    pub fn write_byte(&mut self, b: u8) {
        assert!(!self.is_full());
        let k = self.nwrite % self.data().len();
        volatile::write(&mut self.data_mut()[k], b);
        self.nwrite = self.nwrite.wrapping_add(1);
    }
}

#[repr(transparent)]
pub struct PipeReader<'a> {
    pipe: &'a Mutex<Pipe>,
}

impl<'a> file::Like for PipeReader<'a> {
    fn close(&self) {
        let closed = self.pipe.with_lock(|pipe| {
            pipe.read_open = false;
            proc::wakeup(pipe.write_chan());
            !pipe.write_open
        });
        if closed {
            dealloc(self.pipe);
        }
    }

    fn read(&self, _file: &file::File, buf: &mut [u8]) -> Result<usize> {
        self.pipe.with_lock(|pipe| {
            while !pipe.readable() {
                if myproc().dead() {
                    return Err("dead");
                }
                myproc().sleep(pipe.read_chan(), self.pipe);
            }
            let mut k = 0;
            while k < buf.len() && !pipe.is_empty() {
                buf[k] = pipe.read_byte();
                k += 1;
            }
            proc::wakeup(pipe.write_chan());
            Ok(k)
        })
    }
}

#[repr(transparent)]
pub struct PipeWriter<'a> {
    pipe: &'a Mutex<Pipe>,
}

impl<'a> file::Like for PipeWriter<'a> {
    fn close(&self) {
        let closed = self.pipe.with_lock(|pipe| {
            pipe.write_open = false;
            proc::wakeup(pipe.read_chan());
            !pipe.read_open
        });
        if closed {
            dealloc(self.pipe);
        }
    }

    fn write(&self, _file: &file::File, buf: &[u8]) -> Result<usize> {
        self.pipe.with_lock(|pipe| {
            for &b in buf.iter() {
                while pipe.is_full() {
                    if pipe.broken() {
                        return Err("broken pipe");
                    }
                    proc::wakeup(pipe.read_chan());
                    myproc().sleep(pipe.write_chan(), self.pipe);
                }
                pipe.write_byte(b);
            }
            proc::wakeup(pipe.read_chan());
            Ok(buf.len())
        })
    }
}

#[repr(C, align(64))]
struct PipeAlloc<'a> {
    pipe: Mutex<Pipe>,
    reader: PipeReader<'a>,
    writer: PipeWriter<'a>,
}

const SLAB_NPIPES: usize = (arch::PAGE_SIZE - 64) / paspace();

#[repr(C)]
struct PipeSlab {
    bitmap: u64,
    pipes: *mut PipeAlloc<'static>,
}
const_assert!(SLAB_NPIPES <= 64);
const_assert!(mem::size_of::<PipeSlab>() <= 64);

impl PipeSlab {
    pub fn new() -> Result<&'static mut PipeSlab> {
        let page = kalloc::alloc().ok_or("cannot allocate pipe slab")?;
        let ptr = page.as_mut().as_mut_ptr();
        let ps = unsafe { &mut *(ptr as *mut PipeSlab) };
        ps.pipes = unsafe { ptr.add(64) } as *mut PipeAlloc<'_>;
        for k in 0..SLAB_NPIPES {
            let start = unsafe { ptr.add(64 + k * paspace()) };
            let pa = start as *mut mem::MaybeUninit<PipeAlloc<'_>>;
            unsafe {
                let pa = (*pa).as_mut_ptr();
                ptr::addr_of_mut!((*pa).pipe).write(Mutex::new("apipe", Pipe::new()));
                let pipe = &*(ptr::addr_of_mut!((*pa).pipe));
                ptr::addr_of_mut!((*pa).reader).write(PipeReader { pipe });
                ptr::addr_of_mut!((*pa).writer).write(PipeWriter { pipe });
            }
        }
        Ok(ps)
    }

    pub fn is_empty(&self) -> bool {
        self.bitmap.trailing_ones() as usize >= SLAB_NPIPES
    }

    pub fn is_full(&self) -> bool {
        self.bitmap == 0
    }

    pub fn alloc(&mut self) -> Option<(*const PipeReader<'static>, *const PipeWriter<'static>)> {
        if self.is_empty() {
            return None;
        }
        let page = kalloc::alloc()?;
        let k = self.bitmap.trailing_ones() as usize;
        self.bitmap |= 1 << k;
        let pa = unsafe {
            let ptr = self.pipes.add(k);
            &*ptr
        };
        pa.pipe.lock().reset(page);
        Some((&pa.reader as *const _, &pa.writer as *const _))
    }

    pub fn dealloc(&mut self, pa: &PipeAlloc) {
        let k = ((pa as *const PipeAlloc).addr() % arch::PAGE_SIZE - 64) / paspace();
        assert!(k < SLAB_NPIPES);
        self.bitmap &= !(1 << k);
        kalloc::free(unsafe { &mut *pa.pipe.lock().data });
    }
}

pub fn alloc() -> Result<(&'static file::File, &'static file::File)> {
    let (r, w) = {
        let mut pipes = PIPES.lock();
        if pipes.is_none() {
            let slab = PipeSlab::new()?;
            *pipes = Some(slab);
        }
        let slab = pipes.take().unwrap();
        assert!(!slab.is_empty());
        let (r, w) = slab.alloc().ok_or("pipe allocation failed")?;
        if !slab.is_empty() {
            *pipes = Some(slab);
        }
        unsafe { (&*(r as *const PipeReader), &*(w as *const PipeWriter)) }
    };
    let reader_guard = Guard::new(r);
    let writer_guard = Guard::new(w);
    let reader = file::alloc(file::OpenFlags::Read, r).ok_or("pipe read file alloc failed")?;
    let reader_file_guard = file::Guard::new(reader);
    reader_guard.release();
    let writer = file::alloc(file::OpenFlags::Write, w).ok_or("pipe write file alloc failed")?;
    writer_guard.release();
    reader_file_guard.release();
    Ok((reader, writer))
}

fn dealloc(pipe: &Mutex<Pipe>) {
    let mut pipes = PIPES.lock();
    let pipe_alloc = unsafe { mem::transmute::<_, &PipeAlloc>(pipe) };
    let slab = {
        let raw_slab_addr = (pipe_alloc as *const PipeAlloc).addr() & !(arch::PAGE_SIZE - 1);
        unsafe { &mut *(raw_slab_addr as *mut PipeSlab) }
    };
    slab.dealloc(pipe_alloc);
    if slab.is_full() {
        if let Some(current_slab) = pipes.take() {
            if !ptr::eq(current_slab as *mut _, slab as *mut _) {
                *pipes = Some(current_slab);
            }
        }
        kalloc::free(unsafe { mem::transmute::<_, &mut Page>(slab) });
    } else {
        *pipes = Some(slab);
    }
}

pub struct Guard<'a>(AtomicBool, &'a dyn Like);
impl<'a> Guard<'a> {
    pub fn new(lp: &'a dyn Like) -> Guard<'a> {
        Guard(AtomicBool::new(true), lp)
    }
    pub fn release(&self) {
        self.0.store(false, Ordering::Relaxed);
    }
}
impl<'a> Drop for Guard<'a> {
    fn drop(&mut self) {
        let active = self.0.load(Ordering::Relaxed);
        if active {
            let _ = self.1.close();
        }
    }
}
