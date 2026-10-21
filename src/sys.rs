//! Small Win32 helpers: wide strings, registry, autostart, theme detection.

use std::path::PathBuf;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows_sys::Win32::System::Registry::*;

pub const APP_NAME: &str = "TrafficBar";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn exe_path() -> PathBuf {
    let mut buf = vec![0u16; 32768];
    let n = unsafe { GetModuleFileNameW(std::ptr::null_mut(), buf.as_mut_ptr(), buf.len() as u32) };
    PathBuf::from(String::from_utf16_lossy(&buf[..n as usize]))
}

pub fn config_path() -> PathBuf {
    exe_path().with_file_name(format!("{APP_NAME}.toml"))
}

pub fn log_path() -> PathBuf {
    exe_path().with_file_name(format!("{APP_NAME}.log"))
}

pub fn log(msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log_path()) {
        // keep the log small
        if f.metadata().map(|m| m.len() > 256 * 1024).unwrap_or(false) {
            let _ = f.set_len(0);
        }
        let _ = writeln!(f, "{msg}");
    }
}

fn reg_get_dword(root: HKEY, sub: &str, name: &str) -> Option<u32> {
    let (sub, name) = (wide(sub), wide(name));
    let mut data: u32 = 0;
    let mut size = 4u32;
    let r = unsafe {
        RegGetValueW(root, sub.as_ptr(), name.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(),
            &mut data as *mut u32 as *mut _, &mut size)
    };
    (r == ERROR_SUCCESS).then_some(data)
}

fn reg_get_string(root: HKEY, sub: &str, name: &str) -> Option<String> {
    let (sub, name) = (wide(sub), wide(name));
    let mut buf = vec![0u16; 2048];
    let mut size = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(root, sub.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(),
            buf.as_mut_ptr() as *mut _, &mut size)
    };
    if r != ERROR_SUCCESS {
        return None;
    }
    let len = (size as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..len]))
}

/// True when the taskbar uses the light theme.
pub fn taskbar_is_light() -> bool {
    reg_get_dword(HKEY_CURRENT_USER,
        r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "SystemUsesLightTheme")
        .map(|v| v != 0)
        .unwrap_or(false)
}

pub fn windows_build() -> u32 {
    reg_get_string(HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion", "CurrentBuildNumber")
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Win11 "taskbar alignment": 0 = left, 1 = center (default).
pub fn win11_taskbar_centered() -> bool {
    reg_get_dword(HKEY_CURRENT_USER,
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced", "TaskbarAl")
        .map(|v| v != 0)
        .unwrap_or(true)
}

pub fn autostart_command() -> String {
    format!("\"{}\"", exe_path().display())
}

pub fn autostart_enabled() -> bool {
    reg_get_string(HKEY_CURRENT_USER, RUN_KEY, APP_NAME)
        .map(|v| v.eq_ignore_ascii_case(&autostart_command()))
        .unwrap_or(false)
}

pub fn set_autostart(enable: bool) {
    unsafe {
        let mut key: HKEY = std::ptr::null_mut();
        let sub = wide(RUN_KEY);
        if RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, KEY_SET_VALUE | KEY_QUERY_VALUE, &mut key)
            != ERROR_SUCCESS
        {
            return;
        }
        let name = wide(APP_NAME);
        if enable {
            let val = wide(&autostart_command());
            RegSetValueExW(key, name.as_ptr(), 0, REG_SZ, val.as_ptr() as *const u8, (val.len() * 2) as u32);
        } else {
            RegDeleteValueW(key, name.as_ptr());
        }
        RegCloseKey(key);
    }
}

pub fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    r as u32 | ((g as u32) << 8) | ((b as u32) << 16)
}
