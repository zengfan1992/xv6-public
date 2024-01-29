// Lightly modified K&R allocator.  Note that sizes are in "units",
// not bytes.
use core::cmp;
use core::mem;
use core::ptr;

#[repr(C)]
#[derive(Debug)]
struct Header {
    next: *mut Header,
    nunits: usize,
}

impl Header {
    pub fn new(nunits: usize, next: *mut Header) -> Header {
        Header {
            next,
            nunits,
        }
    }

    pub fn end(&mut self) -> usize {
        let ptr = self as *mut Header;
        unsafe { ptr.add(self.nunits).addr() }
    }
}

static mut FREE_LIST: Option<*mut Header> = None;
static mut BASE: Header = Header {
    next: ptr::null_mut(),
    nunits: 0,
};

fn bytes2units(bytes: usize) -> usize {
    const UNIT_SIZE: usize = mem::size_of::<Header>();
    bytes.checked_add(UNIT_SIZE - 1).unwrap() / UNIT_SIZE
}

pub unsafe extern "C" fn krmalloc(n: usize) -> *mut u8 {
    if let Some(s) = inner_malloc(unsafe { &mut *ptr::addr_of_mut!(FREE_LIST) }, n) {
        unsafe { s.add(1).cast::<u8>() }
    } else {
        ptr::null_mut()
    }
}

fn inner_malloc(free_list: &mut Option<*mut Header>, n: usize) -> Option<*mut Header> {
    if n != 0 {
        if free_list.is_none() {
            let base = unsafe { &mut *ptr::addr_of_mut!(BASE) };
            unsafe {
                ptr::write(base, Header::new(0, base));
            }
            *free_list = Some(base);
        }
        let nunits = bytes2units(n) + 1;
        let freep = free_list.unwrap();
        let mut prevp = freep;
        let mut ptr = unsafe { prevp.as_ref().unwrap().next };
        loop {
            let pnunits = unsafe { ptr.as_ref().unwrap().nunits };
            if pnunits >= nunits {
                let mp = unsafe { ptr.as_mut().unwrap() };
                let p = if pnunits == nunits {
                    let prev = unsafe { prevp.as_mut().unwrap() };
                    prev.next = mp.next;
                    mp
                } else {
                    mp.nunits -= nunits;
                    let p = unsafe { (mp as *mut Header).add(mp.nunits) };
                    unsafe {
                        ptr::write(ptr::from_exposed_addr_mut(p.addr()), Header::new(nunits, mp.next));
                    }
                    p
                };
                *free_list = Some(prevp);
                return Some(p);
            }
            if ptr == freep {
                let units = more_units(nunits)?;
                ptr = inner_free(free_list, units);
            }
            prevp = ptr;
            ptr = unsafe { ptr.as_ref().unwrap().next };
        }
    }
    None
}

fn more_units(nunits: usize) -> Option<&'static mut Header> {
    let nunits = cmp::max(nunits, 4096);
    let ptr = safe_sbrk(nunits * mem::size_of::<Header>())?;
    assert_eq!(ptr.addr() % mem::align_of::<Header>(), 0);
    let next = ptr.cast::<Header>();
    unsafe {
        ptr::write(next, Header::new(nunits, next));
    }
    Some(unsafe { &mut *next })
}

fn safe_sbrk(nbytes: usize) -> Option<*mut u8> {
    extern "C" {
        fn sbrk(nbytes: isize) -> *mut u8;
    }

    let p = unsafe { sbrk(nbytes as isize) };
    if p.addr() == !0 {
        None
    } else {
        Some(p)
    }
}

pub unsafe extern "C" fn krfree(p: *mut u8) {
    fn ptr2tag(p: *mut u8) -> &'static mut Header {
        assert_eq!(p.addr() % mem::align_of::<Header>(), 0);
        let hp = p.addr();
        unsafe {
            &mut *(ptr::from_exposed_addr_mut::<Header>(hp).sub(1))
        }
    }
    if p.eq(&ptr::null_mut()) {
        return;
    }
    inner_free(unsafe { &mut *ptr::addr_of_mut!(FREE_LIST) }, ptr2tag(p));
}

fn inner_free(free_list: &mut Option<*mut Header>, tag: &mut Header) -> *mut Header {
    assert_ne!(tag.nunits, 0);
    if free_list.is_none() {
        let tagp = tag as *mut Header;
        *free_list = Some(tagp);
        return tagp;
    }
    fn pv(p: *mut Header) -> usize {
        p.addr()
    }

    let mut p = free_list.unwrap();
    loop {
        let nextp = unsafe { p.as_ref().unwrap().next };
        let pp = pv(p);
        let bp = pv(tag);
        let np = pv(nextp);
        if (pp < bp && bp < np) || (pp >= np && (pp < bp || bp < np)) {
            if tag.end() == np {
                let next = unsafe { nextp.as_ref().unwrap() };
                tag.nunits += next.nunits;
                tag.next = next.next;
            } else {
                tag.next = nextp;
            }
            let current = unsafe { p.as_mut().unwrap() };
            if current.end() == bp {
                current.nunits += tag.nunits;
                current.next = tag.next;
            } else {
                current.next = tag as *mut Header;
            }
            *free_list = Some(p);
            return p;
        }
        p = nextp;
    }
}

#[cfg(test)]
mod tests {
    use super::{bytes2units, krfree, krmalloc};
    use core::ptr;
    use std::sync::Mutex;

    static MSYNC: Mutex<()> = Mutex::new(());

    fn printfree() {
        let Some(free_list) = (unsafe { &*ptr::addr_of!(super::FREE_LIST) }) else {
            println!("None");
            return;
        };
        let freep = free_list.clone();
        let mut ptr = unsafe { freep.as_ref().unwrap().next };
        loop {
            let p = unsafe { ptr.as_mut().unwrap() };
            println!(
                "Header at {ptr:x?} end={end:x?} next={next:x?} nunits={nunits}",
                end = p.end(),
                next = p.next,
                nunits = p.nunits,
            );
            if ptr == freep {
                break;
            }
            ptr = p.next;
        }
    }

    #[test]
    fn bytes2units_works() {
        assert_eq!(bytes2units(0), 0);
        assert_eq!(bytes2units(1), 1);
        assert_eq!(bytes2units(8), 1);
        assert_eq!(bytes2units(15), 1);
        assert_eq!(bytes2units(16), 1);
        assert_eq!(bytes2units(17), 2);
        assert_eq!(bytes2units(20), 2);
        assert_eq!(bytes2units(32), 2);
    }

    #[test]
    fn malloc0() {
        let _g = MSYNC.lock();
        let p = unsafe { krmalloc(0) };
        assert_eq!(p, ptr::null_mut());
        unsafe {
            krfree(p);
        }
    }

    #[test]
    fn malloc1() {
        let _g = MSYNC.lock();
        let p = unsafe { krmalloc(1) };
        assert_ne!(p, ptr::null_mut());
        unsafe {
            krfree(p);
        }
    }

    #[test]
    fn malloc_med() {
        let _g = MSYNC.lock();
        let p = unsafe { krmalloc(1024 * 1024) };
        assert_ne!(p, ptr::null_mut());
        unsafe {
            krfree(p);
        }
    }

    #[test]
    fn malloc_free_malloc() {
        let _g = MSYNC.lock();
        let p = unsafe { krmalloc(1024 * 1024) };
        assert_ne!(p, ptr::null_mut());
        let q = unsafe { krmalloc(1024 * 1024) };
        assert_ne!(q, ptr::null_mut());
        unsafe {
            krfree(p);
            printfree();
            krfree(q);
            printfree();
        }
        let p = unsafe { krmalloc(20) };
        assert_ne!(p, ptr::null_mut());
        unsafe {
            krfree(p);
        }
    }
}
