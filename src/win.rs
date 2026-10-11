//! Win32 glue: Win+Enter hotkey, accent color, single instance, clipboard, per-monitor placement.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicIsize, Ordering};

use serde::{Deserialize, Serialize};
use windows::Foundation::TypedEventHandler;
use windows::UI::ViewManagement::{UIColorType, UISettings};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint};
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Input::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;
use crate::theme::{FALLBACK_ACCENT, Rgb};

pub const CLASS: &str = "kterm";
const CF_UNICODETEXT: u32 = 13;

static PROXY: Mutex<Option<EventLoopProxy<UserEvent>>> = Mutex::new(None);
/// Hidden message window, used as clipboard owner.
pub static HWND_RAW: AtomicIsize = AtomicIsize::new(0);
/// Title of the hidden message window other kterm launches post to.
const IPC_TITLE: PCWSTR = w!("kterm-ipc");

pub fn set_proxy(p: EventLoopProxy<UserEvent>) {
    *PROXY.lock().unwrap() = Some(p);
}

pub fn send(e: UserEvent) {
    if let Some(p) = PROXY.lock().unwrap().as_ref() {
        let _ = p.send_event(e);
    }
}

// ---- Win+Enter (toggle) / Shift+Win+Enter (new window) ---------------------------------------------------------------------------------
// The i3/Sway/Hyprland "open terminal" key. Windows already reserves Win+Enter as a hotkey, so it
// swallows the keystroke for us; we only need to see it. Raw Input does that without the timeouts
// that make Windows 11 silently drop low-level keyboard hooks, and only wakes us on key events.

pub fn spawn_hotkey_thread() {
    std::thread::spawn(|| unsafe {
        let Ok(sink) = CreateWindowExW(
            Default::default(), w!("STATIC"), IPC_TITLE, Default::default(),
            0, 0, 0, 0, Some(HWND_MESSAGE), None, None, None,
        ) else {
            return;
        };
        // Also the clipboard owner: it outlives every terminal window.
        HWND_RAW.store(sink.0 as isize, Ordering::Relaxed);
        let toggle_msg = toggle_message();
        let keyboard = RAWINPUTDEVICE { usUsagePage: 1, usUsage: 6, dwFlags: RIDEV_INPUTSINK, hwndTarget: sink };
        if RegisterRawInputDevices(&[keyboard], size_of::<RAWINPUTDEVICE>() as u32).is_err() {
            eprintln!("kterm: can't watch the keyboard for Win+Enter");
            return;
        }
        let (mut win_down, mut shift_down, mut enter_down) = (false, false, false);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == toggle_msg {
                send(UserEvent::Toggle); // a second `kterm` launch
                continue;
            }
            if msg.message != WM_INPUT {
                continue;
            }
            let mut raw = RAWINPUT::default();
            let mut size = size_of::<RAWINPUT>() as u32;
            let header = size_of::<RAWINPUTHEADER>() as u32;
            if GetRawInputData(HRAWINPUT(msg.lParam.0 as _), RID_INPUT, Some(&mut raw as *mut _ as _), &mut size, header) == u32::MAX
                || raw.header.dwType != RIM_TYPEKEYBOARD.0
            {
                continue;
            }
            let kb = raw.data.keyboard;
            let down = kb.Flags as u32 & RI_KEY_BREAK == 0;
            match kb.VKey {
                0x5B | 0x5C => win_down = down,          // VK_LWIN, VK_RWIN
                0x10 | 0xA0 | 0xA1 => shift_down = down, // VK_SHIFT, VK_LSHIFT, VK_RSHIFT
                0x0D => {
                    // Windows' own reservation eats the press, so usually only the release
                    // arrives. Toggle on whichever comes first; ignore auto-repeat.
                    if win_down && !enter_down {
                        send(if shift_down { UserEvent::NewWindow } else { UserEvent::Toggle });
                    }
                    enter_down = down;
                }
                _ => {}
            }
        }
    });
}

// ---- Accent color ----------------------------------------------------------------------------

pub fn accent() -> Rgb {
    UISettings::new()
        .and_then(|s| s.GetColorValue(UIColorType::Accent))
        .map(|c| [c.R, c.G, c.B])
        .unwrap_or(FALLBACK_ACCENT)
}

/// Fires `UserEvent::Accent` whenever Windows colors change. Keep the returned value alive.
pub fn watch_accent() -> Option<UISettings> {
    let s = UISettings::new().ok()?;
    s.ColorValuesChanged(&TypedEventHandler::new(|_, _| {
        send(UserEvent::Accent);
        Ok(())
    }))
    .ok()?;
    Some(s)
}

// ---- Single instance -------------------------------------------------------------------------

pub fn toggle_message() -> u32 {
    unsafe { RegisterWindowMessageW(w!("kterm-toggle")) }
}

/// If kterm is already running, ask it to toggle and return true.
pub fn signal_existing_instance() -> bool {
    unsafe {
        // The handle is intentionally leaked: it lives as long as the process.
        let _ = CreateMutexW(None, false, w!("kterm-single-instance"));
        if GetLastError() != ERROR_ALREADY_EXISTS {
            return false;
        }
        if let Ok(hwnd) = FindWindowExW(Some(HWND_MESSAGE), None, w!("STATIC"), IPC_TITLE) {
            let _ = PostMessageW(Some(hwnd), toggle_message(), WPARAM(0), LPARAM(0));
        }
        true
    }
}

// ---- Clipboard -------------------------------------------------------------------------------

fn owner() -> Option<HWND> {
    Some(HWND(HWND_RAW.load(Ordering::Relaxed) as *mut _))
}

pub fn clipboard_get() -> Option<String> {
    unsafe {
        OpenClipboard(owner()).ok()?;
        let text = (|| {
            let h = HGLOBAL(GetClipboardData(CF_UNICODETEXT).ok()?.0);
            let p = GlobalLock(h) as *const u16;
            if p.is_null() {
                return None;
            }
            let len = (0..).take_while(|&i| *p.add(i) != 0).count();
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
            let _ = GlobalUnlock(h);
            Some(s)
        })();
        let _ = CloseClipboard();
        text
    }
}

pub fn clipboard_set(text: &str) {
    let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
    unsafe {
        if OpenClipboard(owner()).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        if let Ok(h) = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2) {
            let p = GlobalLock(h) as *mut u16;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
                let _ = GlobalUnlock(h);
                let _ = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(h.0)));
            }
        }
        let _ = CloseClipboard();
    }
}

/// Work area (monitor minus taskbar) of the monitor containing a point: x, y, w, h.
pub fn work_area(x: i32, y: i32) -> (i32, i32, i32, i32) {
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let _ = GetMonitorInfoW(MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST), &mut info);
    }
    let r = info.rcWork;
    (r.left, r.top, r.right - r.left, r.bottom - r.top)
}

pub fn cursor_pos() -> (i32, i32) {
    let mut p = POINT::default();
    let _ = unsafe { GetCursorPos(&mut p) };
    (p.x, p.y)
}

// ---- Files -----------------------------------------------------------------------------------

pub fn data_dir() -> PathBuf {
    let dir = PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("kterm");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Window rect relative to its monitor's top-left corner.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// One remembered placement per monitor, keyed by device name (`\\.\DISPLAY1`).
pub type Placements = HashMap<String, Placement>;

pub fn load_placements() -> Placements {
    std::fs::read(data_dir().join("placements.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_placements(p: &Placements) {
    if let Ok(json) = serde_json::to_vec_pretty(p) {
        let _ = std::fs::write(data_dir().join("placements.json"), json);
    }
}

/// `kterm --install`: start with Windows, hidden, so Win+Enter works right away.
pub fn install_autostart() -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let status = std::process::Command::new("reg")
        .args(["add", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", "kterm", "/f", "/d"])
        .arg(format!("\"{}\" --hidden", exe.display()))
        .status()?;
    if status.success() { Ok(()) } else { Err(std::io::Error::other("reg add failed")) }
}
