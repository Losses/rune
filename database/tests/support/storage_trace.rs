//! Linux x86_64 test interposer for one SQLite database and rollback journal.
//! The platform's calling convention permits the fixed third `open` argument:
//! it is read only when the caller supplied O_CREAT. All calls use raw syscalls.
#![allow(static_mut_refs)]
use std::{
    ffi::{CStr, c_char, c_int, c_long, c_void},
    sync::{
        Mutex,
        atomic::{AtomicI64, Ordering},
    },
};
extern "C" {
    fn syscall(number: c_long, ...) -> c_long;
    fn getenv(name: *const c_char) -> *mut c_char;
    fn __errno_location() -> *mut c_int;
    fn _exit(status: c_int) -> !;
}
const SYS_WRITE: c_long = 1;
const SYS_LSEEK: c_long = 8;
const SYS_PWRITE: c_long = 18;
const SYS_ACCESS: c_long = 21;
const SYS_FSYNC: c_long = 74;
const SYS_FDATASYNC: c_long = 75;
const SYS_FTRUNCATE: c_long = 77;
const SYS_READLINK: c_long = 89;
const SYS_GETTID: c_long = 186;
const SYS_OPENAT: c_long = 257;
const SYS_UNLINKAT: c_long = 263;
const AT_FDCWD: c_int = -100;
const O_CREAT: c_int = 64;
const O_TRUNC: c_int = 512;
static LOCK: Mutex<()> = Mutex::new(());
static WRITER: AtomicI64 = AtomicI64::new(0);
static mut TRACE: c_int = -1;
static mut DB: [u8; 4096] = [0; 4096];
static mut DB_LEN: usize = 0;
static mut PARENT_LEN: usize = 0;
#[used]
#[link_section = ".init_array"]
static INIT: extern "C" fn() = initialize;
extern "C" fn initialize() {
    unsafe {
        let db = getenv(b"RUNE_SYNC_STORAGE_DB\0".as_ptr().cast());
        let output = getenv(b"RUNE_SYNC_STORAGE_TRACE\0".as_ptr().cast());
        if db.is_null() || output.is_null() {
            return;
        }
        let path = CStr::from_ptr(db).to_bytes();
        if path.len() >= 4096 {
            return;
        }
        let Some(slash) = path.iter().rposition(|c| *c == b'/') else {
            return;
        };
        DB[..path.len()].copy_from_slice(path);
        DB_LEN = path.len();
        PARENT_LEN = slash;
        TRACE = syscall(SYS_OPENAT, AT_FDCWD, output, 1 | O_CREAT | O_TRUNC, 0o600) as c_int;
        if TRACE < 0 {
            _exit(120);
        }
    }
}
unsafe fn kind_path(path: &[u8]) -> u8 {
    if TRACE < 0 {
        return 0;
    }
    let db = &DB[..DB_LEN];
    if path == db {
        return b'D';
    }
    if path.len() == DB_LEN + 8 && path.starts_with(db) && &path[DB_LEN..] == b"-journal" {
        return b'J';
    }
    if path == &DB[..PARENT_LEN] {
        return b'P';
    }
    0
}
unsafe fn kind_fd(fd: c_int) -> u8 {
    if TRACE < 0 || fd == TRACE || fd < 0 {
        return 0;
    }
    let mut name = [0u8; 64];
    let prefix = b"/proc/self/fd/";
    name[..prefix.len()].copy_from_slice(prefix);
    let mut x = fd as u32;
    let mut digits = [0u8; 10];
    let mut n = 0;
    loop {
        digits[n] = b'0' + (x % 10) as u8;
        n += 1;
        x /= 10;
        if x == 0 {
            break;
        }
    }
    for i in 0..n {
        name[prefix.len() + i] = digits[n - i - 1];
    }
    let mut path = [0u8; 4096];
    let count = syscall(
        SYS_READLINK,
        name.as_ptr(),
        path.as_mut_ptr(),
        path.len() - 1,
    );
    if count < 0 {
        return 0;
    }
    let mut len = count as usize;
    let suffix = b" (deleted)";
    if path[..len].ends_with(suffix) {
        len -= suffix.len();
    }
    kind_path(&path[..len])
}
unsafe fn raw_write(mut bytes: *const u8, mut len: usize) {
    while len > 0 {
        let n = syscall(SYS_WRITE, TRACE, bytes, len);
        if n <= 0 {
            _exit(121);
        }
        bytes = bytes.add(n as usize);
        len -= n as usize;
    }
}
unsafe fn event(op: u8, kind: u8, offset: u64, payload: *const u8, len: usize) {
    if kind == 0 || TRACE < 0 {
        return;
    }
    let saved = *__errno_location();
    let _guard = match LOCK.lock() {
        Ok(g) => g,
        Err(_) => _exit(124),
    };
    let tid = syscall(SYS_GETTID) as i64;
    let previous = WRITER.load(Ordering::Relaxed);
    if previous != 0 && previous != tid {
        _exit(122);
    }
    WRITER.store(tid, Ordering::Relaxed);
    let mut header = [0u8; 18];
    header[0] = op;
    header[1] = kind;
    header[2..10].copy_from_slice(&offset.to_le_bytes());
    header[10..18].copy_from_slice(&(len as u64).to_le_bytes());
    raw_write(header.as_ptr(), header.len());
    if op == b'W' {
        raw_write(payload, len);
    }
    *__errno_location() = saved;
}
unsafe fn do_open(path: *const c_char, flags: c_int, mode: c_int) -> c_int {
    let kind = if path.is_null() {
        0
    } else {
        kind_path(CStr::from_ptr(path).to_bytes())
    };
    let existed = kind != 0 && syscall(SYS_ACCESS, path, 0) == 0;
    let fd = syscall(SYS_OPENAT, AT_FDCWD, path, flags, mode) as c_int;
    let saved = *__errno_location();
    if fd >= 0 && kind != 0 && kind != b'P' {
        if !existed && flags & O_CREAT != 0 {
            event(b'C', kind, 0, std::ptr::null(), 0);
        }
        if flags & O_TRUNC != 0 {
            event(b'T', kind, 0, std::ptr::null(), 0);
        }
    }
    *__errno_location() = saved;
    fd
}
#[no_mangle]
pub unsafe extern "C" fn open(path: *const c_char, flags: c_int, mode: c_int) -> c_int {
    do_open(path, flags, mode)
}
#[no_mangle]
pub unsafe extern "C" fn open64(path: *const c_char, flags: c_int, mode: c_int) -> c_int {
    do_open(path, flags, mode)
}
unsafe fn do_pwrite(fd: c_int, data: *const c_void, len: usize, offset: i64) -> isize {
    let n = syscall(SYS_PWRITE, fd, data, len, offset) as isize;
    let saved = *__errno_location();
    if n > 0 {
        event(b'W', kind_fd(fd), offset as u64, data.cast(), n as usize);
    }
    *__errno_location() = saved;
    n
}
#[no_mangle]
pub unsafe extern "C" fn pwrite(fd: c_int, data: *const c_void, len: usize, offset: i64) -> isize {
    do_pwrite(fd, data, len, offset)
}
#[no_mangle]
pub unsafe extern "C" fn pwrite64(
    fd: c_int,
    data: *const c_void,
    len: usize,
    offset: i64,
) -> isize {
    do_pwrite(fd, data, len, offset)
}
#[no_mangle]
pub unsafe extern "C" fn write(fd: c_int, data: *const c_void, len: usize) -> isize {
    let kind = kind_fd(fd);
    let offset = if kind != 0 {
        syscall(SYS_LSEEK, fd, 0, 1)
    } else {
        0
    };
    let n = syscall(SYS_WRITE, fd, data, len) as isize;
    let saved = *__errno_location();
    if n > 0 && kind != 0 {
        event(b'W', kind, offset as u64, data.cast(), n as usize);
    }
    *__errno_location() = saved;
    n
}
unsafe fn sync_call(fd: c_int, call: c_long) -> c_int {
    let n = syscall(call, fd) as c_int;
    let saved = *__errno_location();
    if n == 0 {
        event(b'S', kind_fd(fd), 0, std::ptr::null(), 0);
    }
    *__errno_location() = saved;
    n
}
#[no_mangle]
pub unsafe extern "C" fn fsync(fd: c_int) -> c_int {
    sync_call(fd, SYS_FSYNC)
}
#[no_mangle]
pub unsafe extern "C" fn fdatasync(fd: c_int) -> c_int {
    sync_call(fd, SYS_FDATASYNC)
}
unsafe fn truncate_call(fd: c_int, len: i64) -> c_int {
    let n = syscall(SYS_FTRUNCATE, fd, len) as c_int;
    let saved = *__errno_location();
    if n == 0 {
        event(b'T', kind_fd(fd), len as u64, std::ptr::null(), 0);
    }
    *__errno_location() = saved;
    n
}
#[no_mangle]
pub unsafe extern "C" fn ftruncate(fd: c_int, len: i64) -> c_int {
    truncate_call(fd, len)
}
#[no_mangle]
pub unsafe extern "C" fn ftruncate64(fd: c_int, len: i64) -> c_int {
    truncate_call(fd, len)
}
#[no_mangle]
pub unsafe extern "C" fn unlink(path: *const c_char) -> c_int {
    let kind = if path.is_null() {
        0
    } else {
        kind_path(CStr::from_ptr(path).to_bytes())
    };
    let n = syscall(SYS_UNLINKAT, AT_FDCWD, path, 0) as c_int;
    let saved = *__errno_location();
    if n == 0 {
        event(b'U', kind, 0, std::ptr::null(), 0);
    }
    *__errno_location() = saved;
    n
}
