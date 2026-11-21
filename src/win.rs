//! Thin wrappers over the Win32 calls gdash needs.

use std::path::Path;

use windows::Win32::Foundation::{ERROR_SUCCESS, HWND};
use windows::Win32::Graphics::Gdi::{
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject,
    GetDIBits, HGDIOBJ,
};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
    RegSetKeyValueW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VK_MENU, keybd_event,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, FindWindowW, GetIconInfo, HICON, ICONINFO, PrivateExtractIconsW,
    SW_RESTORE, SW_SHOWNORMAL, SetForegroundWindow, ShowWindow,
};
use windows::core::{HSTRING, PCWSTR};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub fn reg_string(subkey: &str, value: &str) -> Option<String> {
    let mut buf = vec![0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let err = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(subkey),
            &HSTRING::from(value),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if err != ERROR_SUCCESS {
        return None;
    }
    let len = (size as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..len.min(buf.len())]))
}

pub fn reg_dword(subkey: &str, value: &str) -> Option<u32> {
    let mut v = 0u32;
    let mut size = 4u32;
    let err = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(subkey),
            &HSTRING::from(value),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut v as *mut u32).cast()),
            Some(&mut size),
        )
    };
    (err == ERROR_SUCCESS).then_some(v)
}

/// The app id Steam reports as running (0 when none).
pub fn steam_running_appid() -> u32 {
    reg_dword(r"Software\Valve\Steam", "RunningAppID").unwrap_or(0)
}

pub fn autostart_enabled() -> bool {
    reg_string(RUN_KEY, "gdash").is_some()
}

pub fn set_autostart(on: bool) -> bool {
    unsafe {
        if on {
            let Ok(exe) = std::env::current_exe() else { return false };
            let cmd: Vec<u16> = format!("\"{}\"", exe.display())
                .encode_utf16()
                .chain(Some(0))
                .collect();
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(RUN_KEY),
                &HSTRING::from("gdash"),
                REG_SZ.0,
                Some(cmd.as_ptr().cast()),
                (cmd.len() * 2) as u32,
            ) == ERROR_SUCCESS
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(RUN_KEY), &HSTRING::from("gdash"))
                == ERROR_SUCCESS
        }
    }
}

/// Opens a URI or file with its default handler (e.g. `steam://rungameid/123`).
/// Lets a GUI-subsystem process print to the terminal it was started from (if any). When
/// output is already redirected (a pipe or file), that is kept.
pub fn attach_parent_console() {
    use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

pub fn shell_open(target: &str) -> bool {
    let r = unsafe {
        ShellExecuteW(
            None,
            &HSTRING::from("open"),
            &HSTRING::from(target),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    r.0 as isize > 32
}

fn find_window(title: &str) -> Option<HWND> {
    unsafe { FindWindowW(PCWSTR::null(), &HSTRING::from(title)).ok() }
}

/// Restores and brings our window to the front. Windows only lets the foreground
/// process change focus, so a synthetic Alt tap is used to unlock it.
pub fn restore_window(title: &str) {
    if let Some(h) = find_window(title) {
        unsafe {
            let _ = ShowWindow(h, SW_RESTORE);
            keybd_event(VK_MENU.0 as u8, 0, KEYBD_EVENT_FLAGS(0), 0);
            keybd_event(VK_MENU.0 as u8, 0, KEYEVENTF_KEYUP, 0);
            let _ = SetForegroundWindow(h);
        }
    }
}

/// Extracts the exe's largest icon (up to `size`) as straight-alpha RGBA.
pub fn exe_icon_rgba(exe: &Path, size: u32) -> Option<(u32, u32, Vec<u8>)> {
    let mut name = [0u16; 260];
    let wide: Vec<u16> = exe.as_os_str().to_string_lossy().encode_utf16().collect();
    if wide.len() >= name.len() {
        return None;
    }
    name[..wide.len()].copy_from_slice(&wide);

    let mut icons = [HICON::default()];
    let n = unsafe {
        PrivateExtractIconsW(&name, 0, size as i32, size as i32, Some(&mut icons), None, 0)
    };
    if n == 0 || n == u32::MAX || icons[0].is_invalid() {
        return None;
    }
    let icon = icons[0];
    let out = unsafe { icon_to_rgba(icon) };
    unsafe {
        let _ = DestroyIcon(icon);
    }
    out
}

unsafe fn icon_to_rgba(icon: HICON) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let mut info = ICONINFO::default();
        GetIconInfo(icon, &mut info).ok()?;
        let color = info.hbmColor;
        let mask = info.hbmMask;
        let result = (|| {
            if color.is_invalid() {
                return None;
            }
            let dc = CreateCompatibleDC(None);
            let mut bmi = BITMAPINFO::default();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            // First call fills in the size.
            if GetDIBits(dc, color, 0, 0, None, &mut bmi, DIB_RGB_COLORS) == 0 {
                let _ = DeleteDC(dc);
                return None;
            }
            let w = bmi.bmiHeader.biWidth.unsigned_abs();
            let h = bmi.bmiHeader.biHeight.unsigned_abs();
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB.0;
            bmi.bmiHeader.biHeight = -(h as i32); // top-down
            bmi.bmiHeader.biPlanes = 1;
            let mut px = vec![0u8; (w * h * 4) as usize];
            let lines = GetDIBits(dc, color, 0, h, Some(px.as_mut_ptr().cast()), &mut bmi, DIB_RGB_COLORS);
            let _ = DeleteDC(dc);
            if lines == 0 {
                return None;
            }
            let has_alpha = px.chunks_exact(4).any(|p| p[3] != 0);
            for p in px.chunks_exact_mut(4) {
                p.swap(0, 2); // BGRA -> RGBA
                if !has_alpha {
                    p[3] = 255;
                }
            }
            Some((w, h, px))
        })();
        if !color.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(color.0));
        }
        if !mask.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(mask.0));
        }
        result
    }
}

/// Local wall-clock time like "3:07 PM".
pub fn local_time_hm() -> String {
    let st = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    let h = st.wHour % 12;
    let h = if h == 0 { 12 } else { h };
    let ampm = if st.wHour < 12 { "AM" } else { "PM" };
    format!("{h}:{:02} {ampm}", st.wMinute)
}
