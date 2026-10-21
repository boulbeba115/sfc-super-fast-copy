//! Thin wrappers over the Win32 file APIs used by the engine.
//!
//! Paths are kept as UTF-16 in `\\?\` (verbatim) form so long paths work and
//! Windows skips its path normalization on every call.

use std::ffi::c_void;
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::atomic::AtomicI32;
use std::thread::JoinHandle;

use crossbeam_channel::Sender;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND,
    ERROR_NO_MORE_FILES, FILETIME, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::*;

pub type Wide = Vec<u16>;

const BS: u16 = b'\\' as u16;

// From WinBase.h (windows-sys files these under WindowsProgramming).
const COPY_FILE_NO_BUFFERING: u32 = 0x1000;
const PROGRESS_CONTINUE: u32 = 0;

/// Files with these attributes go through CopyFileEx, which knows how to
/// handle sparse, compressed, encrypted and cloud placeholder files.
pub const SPECIAL_ATTRS: u32 = FILE_ATTRIBUTE_SPARSE_FILE
    | FILE_ATTRIBUTE_ENCRYPTED
    | FILE_ATTRIBUTE_COMPRESSED
    | FILE_ATTRIBUTE_REPARSE_POINT
    | FILE_ATTRIBUTE_OFFLINE
    | FILE_ATTRIBUTE_RECALL_ON_OPEN
    | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS;

/// Attributes carried over to the copy.
const COPY_ATTRS: u32 = FILE_ATTRIBUTE_READONLY
    | FILE_ATTRIBUTE_HIDDEN
    | FILE_ATTRIBUTE_SYSTEM
    | FILE_ATTRIBUTE_ARCHIVE
    | FILE_ATTRIBUTE_NOT_CONTENT_INDEXED;

fn wide(s: &str) -> Wide {
    s.encode_utf16().collect()
}

fn last_error() -> io::Error {
    io::Error::last_os_error()
}

/// Absolute `\\?\` form of `p`, without a trailing NUL or separator
/// (except for drive roots such as `\\?\C:\`).
pub fn to_verbatim(p: &Path) -> io::Result<Wide> {
    let abs = std::path::absolute(p)?;
    let s: Wide = abs.as_os_str().encode_wide().collect();
    let mut out = if s.starts_with(&wide(r"\\?\")) || s.starts_with(&wide(r"\\.\")) {
        s
    } else if s.starts_with(&wide(r"\\")) {
        let mut o = wide(r"\\?\UNC\");
        o.extend_from_slice(&s[2..]);
        o
    } else {
        let mut o = wide(r"\\?\");
        o.extend(s);
        o
    };
    while out.last() == Some(&BS) {
        out.pop();
    }
    if out.last() == Some(&(b':' as u16)) {
        out.push(BS);
    }
    Ok(out)
}

/// Human-readable form of a wide path (drops the `\\?\` prefix and NUL).
pub fn display(w: &[u16]) -> String {
    let w = w.strip_suffix(&[0]).unwrap_or(w);
    let s = String::from_utf16_lossy(w);
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s
    }
}

/// Last path component (empty for drive roots).
pub fn file_name(w: &[u16]) -> &[u16] {
    let trimmed = w.strip_suffix(&[BS]).unwrap_or(w);
    match trimmed.iter().rposition(|&c| c == BS) {
        Some(i) => &trimmed[i + 1..],
        None => trimmed,
    }
    .split(|&c| c == b':' as u16)
    .last()
    .unwrap_or(&[])
}

pub fn join(base: &[u16], name: &[u16]) -> Wide {
    let mut v = Vec::with_capacity(base.len() + name.len() + 2);
    v.extend_from_slice(base);
    if v.last() != Some(&BS) {
        v.push(BS);
    }
    v.extend_from_slice(name);
    v
}

/// `join` plus a trailing NUL, ready to hand to Win32.
pub fn join_z(base: &[u16], name: &[u16]) -> Wide {
    let mut v = join(base, name);
    v.push(0);
    v
}

pub fn zstr(w: &[u16]) -> Wide {
    let mut v = Vec::with_capacity(w.len() + 1);
    v.extend_from_slice(w);
    v.push(0);
    v
}

/// Case-insensitive (ASCII) compare helper for "is `inner` inside `outer`".
pub fn is_within(inner: &[u16], outer: &[u16]) -> bool {
    let lower = |c: &u16| if (b'A' as u16..=b'Z' as u16).contains(c) { c + 32 } else { *c };
    let a: Wide = inner.iter().map(lower).collect();
    let b: Wide = outer.iter().map(lower).collect();
    a == b || (a.starts_with(&b) && (b.last() == Some(&BS) || a.get(b.len()) == Some(&BS)))
}

/// One directory entry as returned by FindFirstFileEx, which already gives
/// us size, attributes and timestamps, so no per-file stat is needed.
#[derive(Clone, Debug, Default)]
pub struct Entry {
    pub name: Wide,
    pub attrs: u32,
    pub size: u64,
    pub ctime: i64,
    pub atime: i64,
    pub mtime: i64,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.attrs & FILE_ATTRIBUTE_DIRECTORY != 0
    }

    pub fn is_reparse(&self) -> bool {
        self.attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
}

fn ft(f: FILETIME) -> i64 {
    (((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64) as i64
}

/// Lists a directory using the large-fetch, no-short-name variant of
/// FindFirstFileEx (the fastest documented enumeration API).
pub fn list_dir(dir: &[u16]) -> io::Result<Vec<Entry>> {
    let mut pat = Vec::with_capacity(dir.len() + 3);
    pat.extend_from_slice(dir);
    if pat.last() != Some(&BS) {
        pat.push(BS);
    }
    pat.extend_from_slice(&[b'*' as u16, 0]);

    let mut out = Vec::new();
    unsafe {
        let mut fd: WIN32_FIND_DATAW = zeroed();
        let h = FindFirstFileExW(
            pat.as_ptr(),
            FindExInfoBasic,
            &mut fd as *mut _ as *mut c_void,
            FindExSearchNameMatch,
            null(),
            FIND_FIRST_EX_LARGE_FETCH,
        );
        if h == INVALID_HANDLE_VALUE {
            return match GetLastError() {
                ERROR_FILE_NOT_FOUND => Ok(out),
                e => Err(io::Error::from_raw_os_error(e as i32)),
            };
        }
        loop {
            let len = fd.cFileName.iter().position(|&c| c == 0).unwrap_or(fd.cFileName.len());
            let name = &fd.cFileName[..len];
            if name != [b'.' as u16] && name != [b'.' as u16, b'.' as u16] {
                out.push(Entry {
                    name: name.to_vec(),
                    attrs: fd.dwFileAttributes,
                    size: ((fd.nFileSizeHigh as u64) << 32) | fd.nFileSizeLow as u64,
                    ctime: ft(fd.ftCreationTime),
                    atime: ft(fd.ftLastAccessTime),
                    mtime: ft(fd.ftLastWriteTime),
                });
            }
            if FindNextFileW(h, &mut fd) == 0 {
                let e = GetLastError();
                FindClose(h);
                return if e == ERROR_NO_MORE_FILES {
                    Ok(out)
                } else {
                    Err(io::Error::from_raw_os_error(e as i32))
                };
            }
        }
    }
}

/// Metadata of a single path (name left empty).
pub fn metadata(path: &[u16]) -> io::Result<Entry> {
    let z = zstr(path);
    unsafe {
        let mut d: WIN32_FILE_ATTRIBUTE_DATA = zeroed();
        if GetFileAttributesExW(z.as_ptr(), GetFileExInfoStandard, &mut d as *mut _ as *mut c_void) == 0 {
            return Err(last_error());
        }
        Ok(Entry {
            name: Vec::new(),
            attrs: d.dwFileAttributes,
            size: ((d.nFileSizeHigh as u64) << 32) | d.nFileSizeLow as u64,
            ctime: ft(d.ftCreationTime),
            atime: ft(d.ftLastAccessTime),
            mtime: ft(d.ftLastWriteTime),
        })
    }
}

/// Creates one directory. Returns `Ok(true)` if it was created, `Ok(false)`
/// if it already existed.
pub fn create_dir(path: &[u16]) -> io::Result<bool> {
    let z = zstr(path);
    unsafe {
        if CreateDirectoryW(z.as_ptr(), null()) != 0 {
            return Ok(true);
        }
        match GetLastError() {
            ERROR_ALREADY_EXISTS => Ok(false),
            e => Err(io::Error::from_raw_os_error(e as i32)),
        }
    }
}

/// An owned Win32 handle, closed on drop.
pub struct Handle(HANDLE);

// Handles are plain kernel object references and can be closed from any thread.
unsafe impl Send for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn check_handle(h: HANDLE) -> io::Result<Handle> {
    if h == INVALID_HANDLE_VALUE {
        Err(last_error())
    } else {
        Ok(Handle(h))
    }
}

/// Closing a freshly written file is the slowest step for small files on
/// Windows, because antivirus and other filter drivers do their work in
/// CloseHandle. A dedicated pool of closer threads takes that off the
/// copy threads.
pub struct Closer {
    tx: Option<Sender<Handle>>,
    workers: Vec<JoinHandle<()>>,
}

impl Closer {
    pub fn new(threads: usize) -> Self {
        let (tx, rx) = crossbeam_channel::bounded::<Handle>(4096);
        let workers = (0..threads.max(1))
            .map(|i| {
                let rx = rx.clone();
                std::thread::Builder::new()
                    .name(format!("sfc-close-{i}"))
                    .spawn(move || rx.into_iter().for_each(drop))
                    .expect("spawn closer thread")
            })
            .collect();
        Self { tx: Some(tx), workers }
    }

    pub fn sender(&self) -> Sender<Handle> {
        self.tx.clone().expect("closer already finished")
    }

    /// Waits until every queued handle is closed. All senders handed out
    /// must have been dropped before this is called.
    pub fn finish(mut self) {
        self.tx = None;
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

fn open_read(z: &[u16]) -> io::Result<Handle> {
    unsafe {
        check_handle(CreateFileW(
            z.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_SEQUENTIAL_SCAN,
            null_mut(),
        ))
    }
}

fn create_write(z: &[u16]) -> io::Result<Handle> {
    let open = || unsafe {
        check_handle(CreateFileW(
            z.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ,
            null(),
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_SEQUENTIAL_SCAN,
            null_mut(),
        ))
    };
    match open() {
        // Existing read-only, hidden or system files refuse CREATE_ALWAYS.
        Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => {
            clear_attributes(z);
            open()
        }
        r => r,
    }
}

fn clear_attributes(z: &[u16]) {
    unsafe {
        SetFileAttributesW(z.as_ptr(), FILE_ATTRIBUTE_NORMAL);
    }
}

fn read(h: &Handle, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0u32;
    unsafe {
        if ReadFile(h.0, buf.as_mut_ptr(), buf.len() as u32, &mut n, null_mut()) == 0 {
            return Err(last_error());
        }
    }
    Ok(n as usize)
}

fn write_all(h: &Handle, mut data: &[u8]) -> io::Result<()> {
    while !data.is_empty() {
        let mut n = 0u32;
        unsafe {
            if WriteFile(h.0, data.as_ptr(), data.len() as u32, &mut n, null_mut()) == 0 {
                return Err(last_error());
            }
        }
        if n == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        data = &data[n as usize..];
    }
    Ok(())
}

fn set_eof(h: &Handle, size: u64) -> io::Result<()> {
    let info = FILE_END_OF_FILE_INFO { EndOfFile: size as i64 };
    unsafe {
        if SetFileInformationByHandle(
            h.0,
            FileEndOfFileInfo,
            &info as *const _ as *const c_void,
            size_of::<FILE_END_OF_FILE_INFO>() as u32,
        ) == 0
        {
            return Err(last_error());
        }
    }
    Ok(())
}

/// Sets timestamps and attributes in one call. Explicitly setting the
/// times also stops NTFS from bumping the write time when the handle closes.
fn set_basic(h: &Handle, e: &Entry) -> io::Result<()> {
    let attrs = e.attrs & COPY_ATTRS;
    let info = FILE_BASIC_INFO {
        CreationTime: e.ctime,
        LastAccessTime: e.atime,
        LastWriteTime: e.mtime,
        ChangeTime: 0,
        FileAttributes: if attrs == 0 { FILE_ATTRIBUTE_NORMAL } else { attrs },
    };
    unsafe {
        if SetFileInformationByHandle(
            h.0,
            FileBasicInfo,
            &info as *const _ as *const c_void,
            size_of::<FILE_BASIC_INFO>() as u32,
        ) == 0
        {
            return Err(last_error());
        }
    }
    Ok(())
}

/// Small-file fast path: open, read/write through a reused buffer, set
/// metadata on the open handle, then hand the handle to the closer pool.
/// Roughly half the system calls CopyFileEx makes for a small file.
pub fn copy_fast(
    src: &[u16],
    dst: &[u16],
    e: &Entry,
    buf: &mut [u8],
    closer: &Sender<Handle>,
    progress: &mut dyn FnMut(u64),
) -> io::Result<()> {
    let hs = open_read(src)?;
    let hd = create_write(dst)?;
    let result = (|| {
        if e.size > buf.len() as u64 {
            set_eof(&hd, e.size)?;
        }
        let mut total = 0u64;
        loop {
            let n = read(&hs, buf)?;
            if n == 0 {
                break;
            }
            write_all(&hd, &buf[..n])?;
            total += n as u64;
            progress(n as u64);
        }
        if total != e.size && e.size > buf.len() as u64 {
            // Source shrank while we were copying; trim the preallocation.
            set_eof(&hd, total)?;
        }
        set_basic(&hd, e)
    })();
    drop(hs);
    match result {
        Ok(()) => {
            if let Err(err) = closer.send(hd) {
                drop(err.into_inner());
            }
            Ok(())
        }
        Err(err) => {
            drop(hd);
            unsafe {
                DeleteFileW(dst.as_ptr());
            }
            Err(err)
        }
    }
}

struct Progress<'a> {
    last: u64,
    f: &'a mut dyn FnMut(u64),
}

unsafe extern "system" fn progress_cb(
    _total: i64,
    transferred: i64,
    _stream_size: i64,
    _stream_transferred: i64,
    _stream: u32,
    _reason: LPPROGRESS_ROUTINE_CALLBACK_REASON,
    _src: HANDLE,
    _dst: HANDLE,
    data: *const c_void,
) -> u32 {
    let p = &mut *(data as *mut Progress);
    let t = transferred as u64;
    if t > p.last {
        (p.f)(t - p.last);
        p.last = t;
    }
    PROGRESS_CONTINUE
}

/// Large/special-file path: CopyFileEx, which pipelines reads and writes
/// internally, copies alternate data streams, and (with `unbuffered`)
/// bypasses the file cache for huge files.
pub fn copy_native(
    src: &[u16],
    dst: &[u16],
    unbuffered: bool,
    cancel: &AtomicI32,
    progress: &mut dyn FnMut(u64),
) -> io::Result<()> {
    let mut p = Progress { last: 0, f: progress };
    let flags = if unbuffered { COPY_FILE_NO_BUFFERING } else { 0 };
    let mut call = || unsafe {
        CopyFileExW(
            src.as_ptr(),
            dst.as_ptr(),
            Some(progress_cb),
            &mut p as *mut _ as *const c_void,
            cancel.as_ptr(),
            flags,
        ) != 0
    };
    if call() {
        return Ok(());
    }
    if unsafe { GetLastError() } == ERROR_ACCESS_DENIED {
        clear_attributes(dst);
        if call() {
            return Ok(());
        }
    }
    Err(last_error())
}

/// Applies timestamps and attributes to an existing directory.
pub fn set_dir_meta(path: &[u16], e: &Entry) -> io::Result<()> {
    let z = zstr(path);
    let h = unsafe {
        check_handle(CreateFileW(
            z.as_ptr(),
            FILE_WRITE_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            null_mut(),
        ))?
    };
    let mut e = e.clone();
    e.attrs &= !FILE_ATTRIBUTE_READONLY;
    set_basic(&h, &e)
}
