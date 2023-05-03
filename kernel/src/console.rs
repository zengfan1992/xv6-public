use crate::cga::Cga;
use crate::file::{self, File};
use crate::proc;
use crate::spinlock::SpinMutex as Mutex;
use crate::uart::Uart;
use crate::Result;
use core::fmt;
use syslib::stat::{FileType, Stat};

const fn ctrl(b: u8) -> u8 {
    b - b'@'
}

const BACKSPACE: u8 = ctrl(b'H');
const DELETE: u8 = 0x7F;
const CTLD: u8 = ctrl(b'D');
const CTLP: u8 = ctrl(b'P');
const CTLU: u8 = ctrl(b'U');

pub struct Writers {
    uart: Option<Uart>,
    cga: Option<Cga>,
}

impl Writers {
    fn putb(&mut self, b: u8) {
        if let Some(uart) = self.uart.as_mut() {
            if b == b'\n' {
                uart.putb(b'\r');
            } else if b == BACKSPACE {
                uart.putb(b);
                uart.putb(b' ');
            }
            uart.putb(b);
        }
        if let Some(cga) = self.cga.as_mut() {
            cga.putb(b);
        }
    }
}

impl fmt::Write for Writers {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            self.putb(b);
        }
        Ok(())
    }
}

pub static WRITER: Mutex<Writers> = Mutex::new(
    "cons",
    Writers {
        uart: Some(Uart::uart0()),
        cga: Some(Cga::new()),
    },
);

// For debugging only.
#[allow(dead_code)]
pub fn puts(s: &[u8]) {
    let mut writer = WRITER.lock();
    for &b in s {
        writer.putb(b);
    }
    writer.putb(b'\n');
}

pub unsafe fn init() {
    let mut writer = WRITER.lock();
    if let Some(cga) = writer.cga.as_mut() {
        cga.blank();
    }
}

// The standard kernel println!() is protected by a mutex.
#[cfg(not(test))]
#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

#[cfg(not(test))]
#[macro_export]
macro_rules! print {
    ($($args:tt)*) => ({
        use $crate::console::print;
        print(format_args!($($args)*));
    })
}

pub fn print(args: core::fmt::Arguments) {
    use core::fmt::Write;
    WRITER.lock().write_fmt(args).unwrap();
}

// These macros do not lock, so that they can be called from
// a panic!() handler on a potentially wedged machine.
#[cfg(not(test))]
#[macro_export]
macro_rules! panic_println {
    () => ($crate::uart_print!("\n"));
    ($($arg:tt)*) => ($crate::panic_print!("{}\n", format_args!($($arg)*)));
}

#[cfg(not(test))]
#[macro_export]
macro_rules! panic_print {
    ($($args:tt)*) => ({
        use core::fmt::Write;
        let mut writer = $crate::uart::Uart::uart0();
        writer.write_fmt(format_args!($($args)*)).unwrap();
    })
}

/// The console reader
///
/// In most respects, this is a conventional producer-consumer
/// queue implemented as a ring buffer.  Data input from either
/// the UART or from the legacy keyboard may produce bytes into
/// the buffer, while reads from the console device may consume
/// bytes from the buffer.
///
/// However, there is one wrinkle: the user may edit data that
/// has not yet been fully "written" to the system: e.g., the
/// user may correct typing mistakes by use of the backspace
/// key, or may kill a line of input entirely by using the "^U"
/// sequence.  The user signals that their input is finally
/// ready for system consumption by typing the "Enter" or
/// "Return" key, at which point the input becomes immutable.
/// To account for this, we maintain a third pointer into the
/// buffer: the edit pointer.  As the user types, the edit
/// edit pointer is advanced or retreats, with the invariant
/// that it is always greater than or equal to the write pointer.
/// Once the user hits "Return", the write pointer is advanced
/// to the edit pointer.

const CAPACITY: usize = 256;

struct Reader {
    buffer: [u8; CAPACITY],
    read_index: usize,
    write_index: usize,
    edit_index: usize,
}

impl Reader {
    fn len(&self) -> usize {
        self.write_index.wrapping_sub(self.read_index)
    }

    fn is_empty(&self) -> bool {
        self.read_index == self.write_index
    }

    fn is_full(&self) -> bool {
        self.edit_index.wrapping_sub(self.read_index) == CAPACITY
    }

    fn backspace(&mut self) {
        if self.edit_index != self.write_index {
            self.edit_index = self.edit_index.wrapping_sub(1);
            WRITER.lock().putb(BACKSPACE);
        }
    }

    fn kill(&mut self) {
        while self.edit_index != self.write_index {
            self.backspace();
        }
    }

    pub fn put(&mut self, b: u8) -> Result<usize> {
        match b {
            BACKSPACE | DELETE => {
                self.backspace();
            }
            CTLP => {
                proc::dump();
            }
            CTLU => {
                // Kill line.
                self.kill();
            }
            _ => {
                if self.is_full() {
                    return Err("console overflow");
                }
                let b = if b == b'\r' { b'\n' } else { b };
                self.buffer[self.edit_index % CAPACITY] = b;
                self.edit_index = self.edit_index.wrapping_add(1);
                WRITER.lock().putb(b);
                if b == b'\n'
                    || b == CTLD
                    || self.edit_index == self.read_index.wrapping_add(CAPACITY)
                {
                    self.write_index = self.edit_index;
                    proc::wakeup(self.read_chan());
                }
            }
        }
        Ok(self.len())
    }

    pub fn peek(&self) -> Result<u8> {
        if self.is_empty() {
            return Err("console underflow");
        }
        Ok(self.buffer[self.read_index % CAPACITY])
    }

    pub fn get(&mut self) -> Result<u8> {
        let b = self.peek()?;
        self.read_index = self.read_index.wrapping_add(1);
        Ok(b)
    }

    pub fn read_chan(&self) -> usize {
        &self.read_index as *const _ as usize
    }
}

static READER: Mutex<Reader> = Mutex::new(
    "input",
    Reader {
        buffer: [0u8; CAPACITY],
        read_index: 0,
        write_index: 0,
        edit_index: 0,
    },
);

pub struct Console {}

impl file::Like for Console {
    fn close(&self) {}

    fn read(&self, _: &File, buf: &mut [u8]) -> Result<usize> {
        let mut n = 0;
        while n < buf.len() {
            let mut reader = READER.lock();
            while reader.is_empty() {
                if proc::myproc().dead() {
                    return Err("killed");
                }
                let rchan = reader.read_chan();
                proc::myproc().sleep(rchan, &READER);
            }
            let b = reader.peek().expect("console buffer empty");
            if b == CTLD {
                let _ = reader.get();
                break;
            }
            buf[n] = reader.get().unwrap();
            n += 1;
            if b == b'\n' {
                break;
            }
        }
        Ok(n)
    }

    fn write(&self, _: &File, buf: &[u8]) -> Result<usize> {
        let mut writer = WRITER.lock();
        for &b in buf {
            writer.putb(b);
        }
        Ok(buf.len())
    }

    fn stat(&self) -> Result<Stat> {
        Ok(Stat {
            typ: FileType::Dev,
            dev: 0,
            ino: 0,
            nlink: 0,
            size: 0,
        })
    }
}

static CONSOLE: Console = Console {};

pub fn interrupt<F: FnMut() -> Option<u8>>(mut getb: F) {
    while let Some(b) = getb() {
        let mut reader = READER.lock();
        let _ = reader.put(b);
    }
}

pub const CONSOLE_MAJOR: u32 = 0;

pub fn consdev() -> &'static dyn file::Like {
    &CONSOLE
}
