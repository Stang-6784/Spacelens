use crate::win::wide;
use windows_sys::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Fixed,
    Removable,
    Network,
    Optical,
}

pub struct Drive {
    pub root: String,
    pub letter: char,
    pub label: String,
    pub fs: String,
    pub total: u64,
    pub free: u64,
    pub kind: Kind,
}

pub fn list() -> Vec<Drive> {
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root = format!("{letter}:\\");
        let w = wide(&root);
        let kind = match unsafe { GetDriveTypeW(w.as_ptr()) } {
            2 => Kind::Removable,
            3 => Kind::Fixed,
            4 => Kind::Network,
            5 => Kind::Optical,
            _ => continue,
        };
        let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
        if unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, &mut total, &mut free) } == 0 {
            continue; // not ready (empty card reader, disconnected share…)
        }
        let (label, fs) = volume_info(&root);
        let label = if label.is_empty() {
            match kind {
                Kind::Fixed => "Local Disk".into(),
                Kind::Removable => "Removable Disk".into(),
                Kind::Network => "Network Drive".into(),
                Kind::Optical => "Optical Drive".into(),
            }
        } else {
            label
        };
        out.push(Drive { root, letter, label, fs, total, free, kind });
    }
    out
}

fn volume_info(root: &str) -> (String, String) {
    let mut name = [0u16; 261];
    let mut fs = [0u16; 261];
    let ok = unsafe {
        GetVolumeInformationW(
            wide(root).as_ptr(),
            name.as_mut_ptr(),
            name.len() as u32,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            fs.as_mut_ptr(),
            fs.len() as u32,
        )
    };
    if ok == 0 {
        return (String::new(), String::new());
    }
    let s = |b: &[u16]| String::from_utf16_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]);
    (s(&name), s(&fs))
}

pub fn fs_name(root: &str) -> String {
    volume_info(root).1
}
