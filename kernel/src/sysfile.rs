use crate::exec;
use crate::file;
use crate::fs;
use crate::fslog;
use crate::param;
use crate::pipe;
use crate::proc::Proc;
use crate::Result;
use core::mem;
use core::ptr;
use syslib::stat::{FileType, Stat};
use syslib::syscall;

fn parse_flags(flags: usize) -> Result<(file::OpenFlags, bool)> {
    let create = flags & syscall::O_CREATE == syscall::O_CREATE;
    match flags & !syscall::O_CREATE {
        syscall::O_READ => Ok((file::OpenFlags::Read, create)),
        syscall::O_WRITE => Ok((file::OpenFlags::Write, create)),
        syscall::O_RDWR => Ok((file::OpenFlags::ReadWrite, create)),
        _ => Err("bad open mode"),
    }
}
pub fn open(proc: &Proc, path_ptr: usize, flags: usize) -> Result<usize> {
    let path = proc.fetch_str(path_ptr).ok_or("bad path")?;
    let (mode, create) = parse_flags(flags)?;
    fslog::with_op(|| {
        let ip = if create {
            fs::create(path, fs::CreateType::File)
        } else {
            let ip = fs::namei(path)?;
            ip.lock();
            Ok(ip)
        }?;
        let guard = fs::PutLockGuard::new_locked(ip);
        let like = match ip.typ() {
            FileType::Dir if mode != file::OpenFlags::Read => return Err("open writeable dir"),
            FileType::Dir | FileType::File => ip,
            FileType::Dev => file::devsw(ip.major())?,
            _ => return Err("opening file type none"),
        };
        let file = file::alloc(mode, like).ok_or("cannot allocate file")?;
        let file_guard = file::Guard::new(file);
        let fd = proc
            .alloc_fd(file)
            .ok_or("cannot allocate file descriptor")?;
        file_guard.release();
        guard.release();
        ip.unlock();
        Ok(fd)
    })
}

pub fn close(proc: &Proc, fd: usize) -> Result<()> {
    if let Some(file) = proc.free_fd(fd) {
        file.close();
        Ok(())
    } else {
        Err("bad file descriptor")
    }
}

pub fn write(proc: &Proc, fd: usize, addr: usize, len: usize) -> Result<usize> {
    let file = proc.get_fd(fd).ok_or("bad file")?;
    let buf = proc.fetch_slice(addr, len).ok_or("bad pointer")?;
    file.write(buf)
}

pub fn read(proc: &Proc, fd: usize, addr: usize, len: usize) -> Result<usize> {
    let file = proc.get_fd(fd).ok_or("bad file")?;
    let buf = proc.fetch_slice_mut(addr, len).ok_or("bad pointer")?;
    file.read(buf)
}

pub fn exec(proc: &Proc, path_ptr: usize, args_ptr: usize) -> Result<()> {
    let path = proc.fetch_str(path_ptr).ok_or("bad path")?;
    let mut args = [&[] as &[u8]; param::MAXARG];
    let mut k = 0;
    let mut ptr;
    while {
        let uargp = args_ptr + k * mem::size_of::<usize>();
        ptr = proc.fetch_usize(uargp).ok_or("bad argv")?;
        k < param::MAXARG && ptr != 0
    } {
        args[k] = proc.fetch_str(ptr).ok_or("bad argument")?;
        k += 1;
    }
    let argv = &args[..k];
    exec::exec(proc, path, argv)
}

pub fn stat(proc: &Proc, fd: usize, addr: usize) -> Result<()> {
    let file = proc.get_fd(fd).ok_or("bad file")?;
    let sb = file.stat()?;
    // By fetching the slice, we assert that there is enough space
    // in the process to accommodate the entire Stat structure.
    let user_sb_slice = proc
        .fetch_slice_mut(addr, mem::size_of::<Stat>())
        .ok_or("bad pointer")?;
    unsafe {
        use core::intrinsics::volatile_copy_memory;
        volatile_copy_memory(
            user_sb_slice.as_mut_ptr(),
            &sb as *const _ as *const u8,
            user_sb_slice.len(),
        );
    }
    Ok(())
}

pub fn link(proc: &Proc, path_ptr: usize, new_path_ptr: usize) -> Result<()> {
    let path = proc.fetch_str(path_ptr).ok_or("bad path")?;
    let new_name = proc.fetch_str(new_path_ptr).ok_or("bad new path")?;
    fslog::with_op(|| {
        let ip = fs::namei(path)?;
        let guard = fs::PutLockGuard::new(ip);
        if ip.typ() == FileType::Dir {
            return Err("link dir");
        }
        ip.nlink_inc();
        ip.update()?;
        guard.release();
        let dev = ip.dev();
        let inum = ip.inum();
        ip.unlock();
        let error = |m| {
            ip.with_putlock(|ip| {
                ip.nlink_dec();
                let _ = ip.update();
                Err(m)
            })
        };
        let (dp, name) = fs::namei_parent(new_name)?;
        let guard = fs::PutLockGuard::new(dp);
        if dp.dev() != dev {
            return error("cross-device link");
        }
        if let Err(e) = dp.dir_link(name, inum) {
            return error(e);
        }
        mem::drop(guard);
        ip.put()
    })
}

pub fn unlink(proc: &Proc, path_ptr: usize) -> Result<()> {
    let path = proc.fetch_str(path_ptr).ok_or("bad path")?;
    fslog::with_op(|| {
        let (dp, name) = fs::namei_parent(path)?;
        if name == b"." || name == b".." {
            return Err("unlink . or ..");
        }
        dp.dir_unlink(name)
    })
}

pub fn mkdir(proc: &Proc, path_ptr: usize) -> Result<()> {
    let path = proc.fetch_str(path_ptr).ok_or("bad path")?;
    fslog::with_op(|| {
        let ip = fs::create(path, fs::CreateType::Dir)?;
        ip.unlock_put()
    })
}

pub fn mknod(proc: &Proc, path_ptr: usize, major: u32, minor: u32) -> Result<()> {
    let path = proc.fetch_str(path_ptr).ok_or("bad path")?;
    fslog::with_op(|| {
        let ip = fs::create(path, fs::CreateType::Dev(major, minor))?;
        ip.unlock_put()
    })
}

pub fn chdir(proc: &Proc, path_ptr: usize) -> Result<()> {
    let path = proc.fetch_str(path_ptr).ok_or("bad path")?;
    let ip = fslog::with_op(|| {
        let ip = fs::namei(path)?;
        let guard = fs::PutLockGuard::new(ip);
        if ip.typ() != FileType::Dir {
            return Err("chdir to non-directory");
        }
        guard.release();
        ip.unlock();
        let cwd = proc.cwd();
        let _ = cwd.put();
        Ok(ip)
    })?;
    proc.set_cwd(ip);
    Ok(())
}

pub fn dup(proc: &'static Proc, fd: usize) -> Result<usize> {
    let file = proc.get_fd(fd).ok_or("bad file")?;
    let fd = proc
        .alloc_fd(file)
        .ok_or("cannot allocate file descriptor")?;
    file.dup();
    Ok(fd)
}

pub fn pipe(proc: &Proc, fd_ptr: usize) -> Result<()> {
    let fds_ptr = proc
        .fetch_ptr_mut::<i32>(fd_ptr, 2)
        .ok_or("bad pipe pointer")?;
    let (r, w) = pipe::alloc()?;
    let rguard = file::Guard::new(r);
    let wguard = file::Guard::new(w);
    let rfd = proc
        .alloc_fd(r)
        .ok_or("cannot allocate pipe read descriptor")?;
    let maybe = proc.alloc_fd(w);
    if maybe.is_none() {
        proc.free_fd(rfd);
        return Err("cannot allocate pipe write descriptor");
    }
    let wfd = maybe.unwrap();
    rguard.release();
    wguard.release();
    unsafe {
        ptr::write_volatile(fds_ptr, rfd as i32);
        ptr::write_volatile(fds_ptr.add(1), wfd as i32);
    }
    Ok(())
}
