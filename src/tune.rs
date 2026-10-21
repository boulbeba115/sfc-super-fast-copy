//! Picks a thread count from the kind of drives involved.

use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, GetDriveTypeW, GetVolumePathNameW, QueryDosDeviceW, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows_sys::Win32::System::Ioctl::{
    PropertyStandardQuery, StorageDeviceSeekPenaltyProperty, DEVICE_SEEK_PENALTY_DESCRIPTOR,
    IOCTL_STORAGE_QUERY_PROPERTY, STORAGE_PROPERTY_QUERY,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

use crate::win::zstr;

const DRIVE_REMOTE: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Media {
    Ssd,
    Hdd,
    Network,
    Unknown,
}

impl Media {
    pub fn name(self) -> &'static str {
        match self {
            Media::Ssd => "SSD",
            Media::Hdd => "HDD",
            Media::Network => "network",
            Media::Unknown => "unknown",
        }
    }
}

/// Classifies the volume that holds `path` (an existing `\\?\` path).
pub fn media(path: &[u16]) -> Media {
    // GetVolumePathName treats a subst drive's folder as its own "volume",
    // so map it back to the real path first.
    let s = String::from_utf16_lossy(path);
    let local = s.strip_prefix(r"\\?\").unwrap_or(&s);
    if local.as_bytes().get(1) == Some(&b':') {
        if let Some(mut real) = subst_target(&local[..2]) {
            real.extend(local[2..].encode_utf16());
            return media(&real);
        }
    }

    let z = zstr(path);
    let mut vol = [0u16; 1024];
    unsafe {
        if GetVolumePathNameW(z.as_ptr(), vol.as_mut_ptr(), vol.len() as u32) == 0 {
            return Media::Unknown;
        }
    }
    let len = vol.iter().position(|&c| c == 0).unwrap_or(0);
    let root = String::from_utf16_lossy(&vol[..len]);
    if root.starts_with(r"\\?\UNC\") || (root.starts_with(r"\\") && !root.starts_with(r"\\?\")) {
        return Media::Network;
    }
    unsafe {
        if GetDriveTypeW(zstr(&vol[..len]).as_ptr()) == DRIVE_REMOTE {
            return Media::Network;
        }
    }

    let bare = root.trim_start_matches(r"\\?\").trim_end_matches('\\');
    let device = if bare.len() == 2 && bare.ends_with(':') {
        format!(r"\\.\{bare}")
    } else if bare.starts_with("Volume{") {
        format!(r"\\?\{bare}")
    } else {
        return Media::Unknown;
    };
    let device: Vec<u16> = device.encode_utf16().chain([0]).collect();

    unsafe {
        let h = CreateFileW(
            device.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            0,
            null_mut(),
        );
        if h == INVALID_HANDLE_VALUE {
            return Media::Unknown;
        }
        let mut query: STORAGE_PROPERTY_QUERY = zeroed();
        query.PropertyId = StorageDeviceSeekPenaltyProperty;
        query.QueryType = PropertyStandardQuery;
        let mut desc: DEVICE_SEEK_PENALTY_DESCRIPTOR = zeroed();
        let mut returned = 0u32;
        let ok = DeviceIoControl(
            h,
            IOCTL_STORAGE_QUERY_PROPERTY,
            &query as *const _ as *const c_void,
            size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            &mut desc as *mut _ as *mut c_void,
            size_of::<DEVICE_SEEK_PENALTY_DESCRIPTOR>() as u32,
            &mut returned,
            null_mut(),
        );
        CloseHandle(h);
        if ok == 0 || (returned as usize) < size_of::<DEVICE_SEEK_PENALTY_DESCRIPTOR>() {
            return Media::Unknown;
        }
        if desc.IncursSeekPenalty != 0 {
            Media::Hdd
        } else {
            Media::Ssd
        }
    }
}

/// Folder behind a `subst` drive letter (e.g. `X:` -> `\\?\C:\work`), if any.
/// Real volumes map to `\Device\...` instead and return None.
fn subst_target(drive: &str) -> Option<Vec<u16>> {
    let name: Vec<u16> = drive.encode_utf16().chain([0]).collect();
    let mut buf = [0u16; 1024];
    let n = unsafe { QueryDosDeviceW(name.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) };
    if n == 0 {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
    let target = String::from_utf16_lossy(&buf[..len]);
    let rest = target.strip_prefix(r"\??\")?;
    Some(format!(r"\\?\{rest}").encode_utf16().collect())
}

/// Thread count for copying from `src` to `dst`, plus a short description.
///
/// Small-file copying is latency bound (open/create/close dominate), so
/// SSDs and network shares want many requests in flight. Spinning disks
/// lose throughput to seeking when too many files are open at once.
pub fn auto_threads(src: Media, dst: Media) -> usize {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    if src == Media::Hdd || dst == Media::Hdd {
        6
    } else if src == Media::Network || dst == Media::Network {
        32
    } else {
        (cores * 2).clamp(8, 64)
    }
}
