#![allow(non_snake_case)]

use std::{ffi::c_void, os::windows::ffi::OsStrExt, ptr};

#[repr(C)]
struct FileTime { low: u32, high: u32 }

#[repr(C)]
struct CredentialW {
    flags: u32, kind: u32, target: *mut u16, comment: *mut u16,
    written: FileTime, blob_size: u32, blob: *mut u8, persist: u32,
    attribute_count: u32, attributes: *mut c_void, alias: *mut u16, user: *mut u16,
}

#[link(name = "Advapi32")]
extern "system" {
    fn CredWriteW(credential: *const CredentialW, flags: u32) -> i32;
    fn CredReadW(target: *const u16, kind: u32, flags: u32, result: *mut *mut CredentialW) -> i32;
    fn CredDeleteW(target: *const u16, kind: u32, flags: u32) -> i32;
    fn CredFree(buffer: *mut c_void);
    fn RegOpenKeyExW(root: isize, subkey: *const u16, options: u32, access: u32, result: *mut isize) -> i32;
    fn RegQueryValueExW(key: isize, value: *const u16, reserved: *mut u32, kind: *mut u32, data: *mut u8, size: *mut u32) -> i32;
    fn RegNotifyChangeKeyValue(key: isize, watch_subtree: i32, filter: u32, event: isize, async_: i32) -> i32;
    fn RegCloseKey(key: isize) -> i32;
}

fn wide(value: &str) -> Vec<u16> { std::ffi::OsStr::new(value).encode_wide().chain(Some(0)).collect() }
fn target(origin: &str) -> Vec<u16> { wide(&format!("CodexMobile.Native|{origin}")) }

pub fn save_secret(origin: &str, secret: &str) -> Result<(), String> {
    let mut name = target(origin);
    let mut user = wide("Codex Mobile");
    let mut bytes = secret.as_bytes().to_vec();
    if bytes.is_empty() || bytes.len() > 2500 { return Err("设备凭证大小无效".into()); }
    let cred = CredentialW {
        flags: 0, kind: 1, target: name.as_mut_ptr(), comment: ptr::null_mut(),
        written: FileTime { low: 0, high: 0 }, blob_size: bytes.len() as u32,
        blob: bytes.as_mut_ptr(), persist: 2, attribute_count: 0,
        attributes: ptr::null_mut(), alias: ptr::null_mut(), user: user.as_mut_ptr(),
    };
    let ok = unsafe { CredWriteW(&cred, 0) } != 0;
    bytes.fill(0);
    if ok { Ok(()) } else { Err(format!("Windows 凭据管理器写入失败：{}", std::io::Error::last_os_error())) }
}

pub fn load_secret(origin: &str) -> Option<String> {
    let name = target(origin);
    let mut result = ptr::null_mut();
    if unsafe { CredReadW(name.as_ptr(), 1, 0, &mut result) } == 0 || result.is_null() { return None; }
    let secret = unsafe {
        let cred = &*result;
        let bytes = std::slice::from_raw_parts(cred.blob, cred.blob_size as usize);
        String::from_utf8(bytes.to_vec()).ok()
    };
    unsafe { CredFree(result.cast()) };
    secret
}

pub fn forget_secret(origin: &str) {
    let name = target(origin);
    unsafe { CredDeleteW(name.as_ptr(), 1, 0); }
}

const HKCU: isize = 0x8000_0001u32 as isize;
const KEY_READ: u32 = 0x20019;
const REG_NOTIFY_CHANGE_LAST_SET: u32 = 0x0000_0004;

fn theme_key() -> Option<isize> {
    let mut key = 0;
    let path = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    if unsafe { RegOpenKeyExW(HKCU, path.as_ptr(), 0, KEY_READ, &mut key) } == 0 { Some(key) } else { None }
}

fn read_light(key: isize) -> bool {
    let mut value = 1u32;
    let mut size = 4u32;
    let mut kind = 0u32;
    let name = wide("AppsUseLightTheme");
    let ok = unsafe { RegQueryValueExW(key, name.as_ptr(), ptr::null_mut(), &mut kind, (&mut value as *mut u32).cast(), &mut size) } == 0;
    !ok || value != 0
}

pub fn system_dark() -> bool {
    let Some(key) = theme_key() else { return false; };
    let dark = !read_light(key);
    unsafe { RegCloseKey(key); }
    dark
}

pub fn watch_theme(mut changed: impl FnMut(bool) + Send + 'static) {
    std::thread::spawn(move || {
        let Some(key) = theme_key() else { return; };
        loop {
            if unsafe { RegNotifyChangeKeyValue(key, 0, REG_NOTIFY_CHANGE_LAST_SET, 0, 0) } != 0 { break; }
            changed(!read_light(key));
        }
        unsafe { RegCloseKey(key); }
    });
}
