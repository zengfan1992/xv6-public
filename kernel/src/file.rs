use crate::console;
use crate::param;
use crate::spinlock::SpinMutex as Mutex;
use crate::Result;
use core::cell::Cell;
use core::sync::atomic::{AtomicBool, Ordering};
use syslib::stat::Stat;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenFlags {
    None,
    Read,
    Write,
    ReadWrite,
}

pub trait Like {
    fn close(&self);

    fn stat(&self) -> Result<Stat> {
        Err("cannot stat")
    }

    fn read(&self, _file: &File, _buf: &mut [u8]) -> Result<usize> {
        Err("unimplemented")
    }

    fn write(&self, _file: &File, _buf: &[u8]) -> Result<usize> {
        Err("unimplemented")
    }
}

static FILES: Mutex<[File; param::NFILE]> =
    Mutex::new("files", [const { File::new() }; param::NFILE]);

pub struct File {
    flags: Cell<OpenFlags>,
    fp: Cell<Option<&'static dyn Like>>,
    off: Cell<usize>,
    ref_cnt: Cell<u32>,
}

impl File {
    pub const fn new() -> File {
        File {
            flags: Cell::new(OpenFlags::None),
            fp: Cell::new(None),
            off: Cell::new(0),
            ref_cnt: Cell::new(0),
        }
    }

    fn set_flags(&self, flags: OpenFlags) {
        self.flags.set(flags)
    }

    pub fn set_fp(&self, fp: &'static dyn Like) {
        self.fp.set(Some(fp));
    }

    pub fn off(&self) -> usize {
        self.off.get()
    }

    pub fn inc_off(&self, inc: usize) {
        self.off.set(self.off.get() + inc);
    }

    fn ref_cnt(&self) -> u32 {
        self.ref_cnt.get()
    }

    fn inc_ref_cnt(&self) {
        self.ref_cnt.set(self.ref_cnt.get() + 1);
    }

    fn dec_ref_cnt(&self) -> u32 {
        let rc = self.ref_cnt.get() - 1;
        self.ref_cnt.set(rc);
        rc
    }

    pub fn dup(&self) -> &File {
        FILES.with_lock(|_| self.inc_ref_cnt());
        self
    }

    pub fn stat(&self) -> Result<Stat> {
        self.fp.get().expect("stat nil file").stat()
    }

    pub fn close(&self) {
        if let Some(fp) = FILES.with_lock(|_| {
            assert!(self.ref_cnt() > 0, "closing unref file");
            let rc = self.dec_ref_cnt();
            if rc > 0 {
                return None;
            }
            let fp = self.fp.get().expect("close nil file");
            self.flags.set(OpenFlags::None);
            self.fp.set(None);
            self.off.set(0);
            Some(fp)
        }) {
            fp.close();
        }
    }

    fn readable(&self) -> bool {
        let flags = self.flags.get();
        flags == OpenFlags::Read || flags == OpenFlags::ReadWrite
    }

    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        if !self.readable() {
            return Err("file not readable");
        }
        let fp = self.fp.get().expect("read nil file");
        fp.read(self, buf)
    }

    fn writable(&self) -> bool {
        let flags = self.flags.get();
        flags == OpenFlags::Write || flags == OpenFlags::ReadWrite
    }

    pub fn write(&self, buf: &[u8]) -> Result<usize> {
        if !self.writable() {
            return Err("file not writable");
        }
        let fp = self.fp.get().expect("write nil file");
        fp.write(self, buf)
    }
}

pub fn alloc(flags: OpenFlags, fp: &'static dyn Like) -> Option<&'static File> {
    let files = FILES.lock();
    for file in files.iter() {
        if file.ref_cnt() == 0 {
            file.inc_ref_cnt();
            file.set_flags(flags);
            file.set_fp(fp);
            return Some(unsafe { &*(file as *const File) });
        }
    }
    None
}

pub struct Guard<'a>(AtomicBool, &'a File);
impl<'a> Guard<'a> {
    pub fn new(fp: &'a File) -> Guard<'a> {
        Guard(AtomicBool::new(true), fp)
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

pub fn devsw(major: u32) -> Result<&'static dyn Like> {
    match major {
        console::CONSOLE_MAJOR => Ok(console::consdev()),
        _ => Err("bad device major number"),
    }
}
