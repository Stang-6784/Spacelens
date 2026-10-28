//! Small Win32 helpers: elevation, shell actions, window styling, time formatting.

use crate::theme;
use windows_sys::Win32::Foundation::{GetLastError, BOOL, FILETIME, HWND, LPARAM, SYSTEMTIME};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Storage::FileSystem::FileTimeToLocalFileTime;
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::System::Time::FileTimeToSystemTime;
use windows_sys::Win32::UI::Shell::{IsUserAnAdmin, ShellExecuteW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, IsWindowVisible, SW_SHOWNORMAL,
};

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn last_error(what: &str) -> String {
    let code = unsafe { GetLastError() };
    let msg = std::io::Error::from_raw_os_error(code as i32);
    format!("{what}: {msg}")
}

pub fn is_admin() -> bool {
    unsafe { IsUserAnAdmin() != 0 }
}

/// Relaunches this exe elevated. Returns false if the user declined the UAC prompt.
pub fn relaunch_as_admin(path: Option<&str>) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let params = path.map(|p| format!("\"{}\"", p.trim_end_matches('\\'))).unwrap_or_default();
    let (verb, file, params) = (wide("runas"), wide(&exe.to_string_lossy()), wide(&params));
    let r = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            params.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    r as isize > 32
}

pub fn open(path: &str) {
    let _ = std::process::Command::new("explorer").arg(path).spawn();
}

pub fn reveal(path: &str) {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("explorer").raw_arg(format!("/select,\"{path}\"")).spawn();
}

pub fn format_filetime(ft: u64) -> String {
    if ft == 0 {
        return String::new();
    }
    unsafe {
        let utc = FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft >> 32) as u32 };
        let mut local: FILETIME = std::mem::zeroed();
        let mut st: SYSTEMTIME = std::mem::zeroed();
        if FileTimeToLocalFileTime(&utc, &mut local) == 0 || FileTimeToSystemTime(&local, &mut st) == 0 {
            return String::new();
        }
        format!("{:04}-{:02}-{:02} {:02}:{:02}", st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute)
    }
}

const fn colorref(c: [u8; 3]) -> u32 {
    c[0] as u32 | (c[1] as u32) << 8 | (c[2] as u32) << 16
}

unsafe extern "system" fn style_cb(hwnd: HWND, found: LPARAM) -> BOOL {
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid == GetCurrentProcessId() && IsWindowVisible(hwnd) != 0 {
        let set = |attr: u32, v: u32| {
            DwmSetWindowAttribute(hwnd, attr, (&v as *const u32).cast(), 4);
        };
        set(20, 1); // DWMWA_USE_IMMERSIVE_DARK_MODE
        set(34, colorref(theme::RGB_BORDER)); // DWMWA_BORDER_COLOR
        set(35, colorref(theme::RGB_BG)); // DWMWA_CAPTION_COLOR
        set(36, colorref(theme::RGB_TEXT)); // DWMWA_TEXT_COLOR
        *(found as *mut bool) = true;
    }
    1
}

/// Gives the native title bar the app's dark colours (Windows 11). Returns true once applied.
pub fn style_own_window() -> bool {
    let mut found = false;
    unsafe { EnumWindows(Some(style_cb), &mut found as *mut bool as LPARAM) };
    found
}
