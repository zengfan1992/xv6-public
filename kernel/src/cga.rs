use crate::volatile;
use crate::x86_64::outb;
use core::ptr::NonNull;

const BASE_ADDR: usize = 0xffff_8000_000b_8000;
const DISPLAY_HEIGHT: usize = 25;
const DISPLAY_WIDTH: usize = 80;
const DISPLAY_LINE_SIZE: usize = DISPLAY_WIDTH * 2;
const DISPLAY_SIZE: usize = DISPLAY_LINE_SIZE * DISPLAY_HEIGHT;
const ATTRIBUTE: u8 = 0x07u8;

pub struct Cga {
    line: usize,
    column: usize,
    buffer: NonNull<[u8; DISPLAY_SIZE]>,
}

fn bshift(dst: &mut [u8], offset: usize) {
    let len = dst.len().checked_sub(offset).expect("offset");
    let dp = dst[..len].as_mut_ptr();
    let sp = dst[offset..offset + len].as_ptr();
    unsafe {
        core::intrinsics::volatile_copy_memory(dp, sp, len);
    }
}

impl Cga {
    pub const fn new() -> Cga {
        Cga {
            line: 0,
            column: 0,
            buffer: unsafe { NonNull::new_unchecked(BASE_ADDR as *mut _) },
        }
    }

    fn buffer_mut_slice(&mut self) -> &mut [u8] {
        unsafe { self.buffer.as_mut() }
    }

    pub fn blank(&mut self) {
        self.line = 0;
        self.column = 0;
        volatile::zero_slice(self.buffer_mut_slice());
    }

    fn scroll(&mut self) {
        let len = DISPLAY_SIZE - DISPLAY_LINE_SIZE;
        let buffer = self.buffer_mut_slice();
        bshift(buffer, DISPLAY_LINE_SIZE);
        volatile::zero_slice(&mut buffer[len..len + DISPLAY_LINE_SIZE]);
        self.line -= 1;
        self.set_cursor();
    }

    fn set_cursor(&mut self) {
        let off = self.line * DISPLAY_LINE_SIZE + self.column * 2;
        let buf = [b' ', 0x07u8];
        volatile::copy_slice(&mut self.buffer_mut_slice()[off..off + 2], &buf);
        let pos = self.line * DISPLAY_WIDTH + self.column;
        const INDEX_REG: u16 = 0x3d4;
        const DATA_REG: u16 = 0x3d5;
        const CURSOR_LOC_HI: u8 = 0x0E;
        const CURSOR_LOC_LO: u8 = 0x0F;
        unsafe {
            outb(INDEX_REG, CURSOR_LOC_HI);
            outb(DATA_REG, (pos >> 8) as u8);
            outb(INDEX_REG, CURSOR_LOC_LO);
            outb(DATA_REG, pos as u8);
        }
    }

    fn newline(&mut self) {
        self.column = 0;
        self.line += 1;
        if self.line == DISPLAY_HEIGHT {
            self.scroll();
        }
    }

    pub fn putb(&mut self, b: u8) {
        const BS: u8 = b'\x08'; // Backspace.
        match b {
            b'\n' => self.newline(),
            b'\r' => {}
            b'\t' => {
                let mut tabstop = self.column + 8 - (self.column % 8);
                if tabstop >= DISPLAY_WIDTH {
                    self.newline();
                    tabstop -= DISPLAY_WIDTH
                }
                while self.column < tabstop {
                    self.putb(b' ')
                }
            }
            BS => {
                if self.column > 0 {
                    self.column -= 1;
                    self.putb(b' ');
                    self.column -= 1;
                }
            }
            _ => {
                let buf = [b, ATTRIBUTE];
                let off = self.column * 2 + (self.line * DISPLAY_LINE_SIZE);
                volatile::copy_slice(&mut self.buffer_mut_slice()[off..off + 2], &buf);
                self.column += 1;
                if self.column >= DISPLAY_WIDTH {
                    self.putb(b'\n');
                }
            }
        }
        self.set_cursor();
    }
}
