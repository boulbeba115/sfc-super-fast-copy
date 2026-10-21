//! Explorer integration: the clipboard behind "Copy with SFC" /
//! "Paste with SFC", plus small shell helpers for the GUI.
//!
//! "Copy with SFC" puts files on the normal Windows clipboard (exactly
//! like Ctrl+C), so Explorer's own Paste works too, and "Paste with
//! SFC" also accepts files copied with Ctrl+C.

use std::ffi::{c_void, OsString};
use std::io;
use std::mem::size_of;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::time::Duration;

use windows_sys::Win32::Foundation::{CloseHandle, GlobalFree, HWND, POINT};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE, GMEM_ZEROINIT};
use windows_sys::Win32::System::SystemInformation::GetTickCount64;
use windows_sys::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
use windows_sys::Win32::UI::Shell::{DragQueryFileW, DROPFILES};
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, HWND_MESSAGE};

const CF_HDROP: u32 = 15;
const DROPEFFECT_COPY: u32 = 1;
const DROPEFFECT_MOVE: u32 = 2;
/// Explorer starts one process per selected item, all within moments of
/// each other; additions this close together belong to one selection.
const SESSION_MS: u64 = 1500;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn wide_path(p: &Path) -> Vec<u16> {
    p.as_os_str().encode_wide().chain([0]).collect()
}

/// Per-user data folder: %LOCALAPPDATA%\SuperFastCopy.
pub fn data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("SuperFastCopy")
}

fn open_clipboard(hwnd: HWND) -> bool {
    // Other programs (clipboard managers, Explorer) hold it briefly.
    for _ in 0..100 {
        if unsafe { OpenClipboard(hwnd) } != 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn drop_effect_format() -> u32 {
    unsafe { RegisterClipboardFormatW(wide("Preferred DropEffect").as_ptr()) }
}

/// Files on the clipboard, and whether they were cut (Ctrl+X) rather than copied.
pub struct ClipboardFiles {
    pub paths: Vec<PathBuf>,
    pub cut: bool,
}

pub fn read_clipboard() -> ClipboardFiles {
    let mut out = ClipboardFiles { paths: Vec::new(), cut: false };
    if !open_clipboard(null_mut()) {
        return out;
    }
    unsafe {
        let hdrop = GetClipboardData(CF_HDROP);
        if !hdrop.is_null() {
            let count = DragQueryFileW(hdrop, u32::MAX, null_mut(), 0);
            for i in 0..count {
                let len = DragQueryFileW(hdrop, i, null_mut(), 0);
                let mut buf = vec![0u16; len as usize + 1];
                DragQueryFileW(hdrop, i, buf.as_mut_ptr(), len + 1);
                out.paths.push(PathBuf::from(OsString::from_wide(&buf[..len as usize])));
            }
        }
        let effect = GetClipboardData(drop_effect_format());
        if !effect.is_null() {
            let p = GlobalLock(effect) as *const u32;
            if !p.is_null() {
                out.cut = *p & DROPEFFECT_MOVE != 0;
                GlobalUnlock(effect);
            }
        }
        CloseClipboard();
    }
    out
}

/// Puts `paths` on the clipboard as a file copy (CF_HDROP).
pub fn write_clipboard(paths: &[PathBuf]) -> io::Result<()> {
    let mut list: Vec<u16> = Vec::new();
    for p in paths {
        list.extend(p.as_os_str().encode_wide());
        list.push(0);
    }
    list.push(0);
    let header = size_of::<DROPFILES>();

    unsafe {
        let files = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, header + list.len() * 2);
        let effect = GlobalAlloc(GMEM_MOVEABLE, 4);
        if files.is_null() || effect.is_null() {
            return Err(io::Error::last_os_error());
        }
        let p = GlobalLock(files) as *mut u8;
        *(p as *mut DROPFILES) = DROPFILES { pFiles: header as u32, pt: POINT { x: 0, y: 0 }, fNC: 0, fWide: 1 };
        std::ptr::copy_nonoverlapping(list.as_ptr(), p.add(header) as *mut u16, list.len());
        GlobalUnlock(files);
        *(GlobalLock(effect) as *mut u32) = DROPEFFECT_COPY;
        GlobalUnlock(effect);

        // SetClipboardData fails unless the clipboard was opened by a window.
        let class = wide("STATIC");
        let hwnd = CreateWindowExW(0, class.as_ptr(), null(), 0, 0, 0, 0, 0, HWND_MESSAGE, null_mut(), null_mut(), null());
        if !open_clipboard(hwnd) {
            GlobalFree(files);
            GlobalFree(effect);
            DestroyWindow(hwnd);
            return Err(io::Error::new(io::ErrorKind::TimedOut, "clipboard is busy"));
        }
        EmptyClipboard();
        let ok = !SetClipboardData(CF_HDROP, files).is_null();
        if !ok {
            GlobalFree(files);
        }
        if SetClipboardData(drop_effect_format(), effect).is_null() {
            GlobalFree(effect);
        }
        CloseClipboard();
        DestroyWindow(hwnd);
        if ok {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

/// "Copy with SFC": adds `paths` to the clipboard file list. Calls that
/// arrive in quick succession (one per selected item) are merged; a later
/// call, or any other clipboard change in between, starts a new list.
pub fn add_to_copy_list(paths: &[PathBuf]) -> io::Result<()> {
    let name = wide("Local\\SuperFastCopy.CopyList");
    let mutex = unsafe { CreateMutexW(null(), 0, name.as_ptr()) };
    if !mutex.is_null() {
        unsafe { WaitForSingleObject(mutex, 10_000) };
    }
    let result = (|| {
        let state = data_dir().join("copy-list.txt");
        let now = unsafe { GetTickCount64() };
        let seq = unsafe { GetClipboardSequenceNumber() };

        let mut list: Vec<PathBuf> = Vec::new();
        if let Ok(text) = std::fs::read_to_string(&state) {
            let mut lines = text.lines();
            let mut head = lines.next().unwrap_or("").split_whitespace().map(|v| v.parse::<u64>().unwrap_or(0));
            let (t, s) = (head.next().unwrap_or(0), head.next().unwrap_or(0));
            if now.saturating_sub(t) < SESSION_MS && s == seq as u64 {
                list = lines.map(PathBuf::from).collect();
            }
        }
        for p in paths {
            let p = std::path::absolute(p).unwrap_or_else(|_| p.clone());
            if !list.contains(&p) {
                list.push(p);
            }
        }

        write_clipboard(&list)?;
        let seq = unsafe { GetClipboardSequenceNumber() };
        let mut text = format!("{now} {seq}\n");
        for p in &list {
            text.push_str(&p.to_string_lossy());
            text.push('\n');
        }
        std::fs::create_dir_all(data_dir())?;
        std::fs::write(&state, text)
    })();
    if !mutex.is_null() {
        unsafe {
            ReleaseMutex(mutex);
            CloseHandle(mutex);
        }
    }
    result
}

/// (free, total) bytes on the volume holding `path` or its nearest
/// existing parent.
pub fn disk_space(path: &Path) -> Option<(u64, u64)> {
    let mut cur = Some(path);
    while let Some(p) = cur {
        if p.exists() {
            let (mut free, mut total) = (0u64, 0u64);
            let ok = unsafe { GetDiskFreeSpaceExW(wide_path(p).as_ptr(), &mut free, &mut total, null_mut()) };
            return (ok != 0).then_some((free, total));
        }
        cur = p.parent();
    }
    None
}

/// Dark title bar in the app's own colors (Windows 11; ignored elsewhere).
pub fn style_titlebar(hwnd: *mut c_void, caption: [u8; 3], text: [u8; 3]) {
    const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWA_BORDER_COLOR: u32 = 34;
    const DWMWA_CAPTION_COLOR: u32 = 35;
    const DWMWA_TEXT_COLOR: u32 = 36;
    let rgb = |c: [u8; 3]| c[0] as u32 | (c[1] as u32) << 8 | (c[2] as u32) << 16;
    let set = |attr: u32, value: u32| unsafe {
        DwmSetWindowAttribute(hwnd, attr, &value as *const u32 as *const c_void, 4);
    };
    set(DWMWA_USE_IMMERSIVE_DARK_MODE, 1);
    set(DWMWA_WINDOW_CORNER_PREFERENCE, 2);
    set(DWMWA_CAPTION_COLOR, rgb(caption));
    set(DWMWA_BORDER_COLOR, rgb(caption));
    set(DWMWA_TEXT_COLOR, rgb(text));
}
