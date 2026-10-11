//! JetBrains Mono Nerd Font ships inside kterm (OFL, see assets/fonts/OFL.txt) and is installed for
//! the current Windows user on first run if it isn't there yet. Per-user install: no admin needed.

use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::AddFontResourceW;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegSetKeyValueW};
use windows::Win32::UI::WindowsAndMessaging::{HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_FONTCHANGE};
use windows::core::{HSTRING, w};

macro_rules! font {
    ($file:literal, $name:literal) => {
        ($file, $name, include_bytes!(concat!("../assets/fonts/", $file)) as &[u8])
    };
}

const FONTS: [(&str, &str, &[u8]); 4] = [
    font!("JetBrainsMonoNerdFont-Regular.ttf", "JetBrainsMono Nerd Font Regular (TrueType)"),
    font!("JetBrainsMonoNerdFont-Bold.ttf", "JetBrainsMono Nerd Font Bold (TrueType)"),
    font!("JetBrainsMonoNerdFont-Italic.ttf", "JetBrainsMono Nerd Font Italic (TrueType)"),
    font!("JetBrainsMonoNerdFont-BoldItalic.ttf", "JetBrainsMono Nerd Font Bold Italic (TrueType)"),
];

pub fn ensure_installed() {
    let env_dir = |var: &str, fallback: &str| PathBuf::from(std::env::var_os(var).unwrap_or(fallback.into()));
    let user_dir = env_dir("LOCALAPPDATA", "").join(r"Microsoft\Windows\Fonts");
    let system_dir = env_dir("WINDIR", r"C:\Windows").join("Fonts");
    let mut added = false;
    for (file, name, bytes) in FONTS {
        if system_dir.join(file).exists() || user_dir.join(file).exists() {
            continue;
        }
        let path = user_dir.join(file);
        if std::fs::create_dir_all(&user_dir).is_err() || std::fs::write(&path, bytes).is_err() {
            continue;
        }
        let value: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        unsafe {
            // Registering under HKCU is what makes it a permanent per-user install.
            let _ = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                w!(r"Software\Microsoft\Windows NT\CurrentVersion\Fonts"),
                &HSTRING::from(name),
                REG_SZ.0,
                Some(value.as_ptr().cast()),
                (value.len() * 2) as u32,
            );
            AddFontResourceW(&HSTRING::from(path.as_os_str()));
        }
        added = true;
    }
    if added {
        // Tell running apps (and the font cache) that fonts changed.
        unsafe {
            let _ = SendMessageTimeoutW(HWND_BROADCAST, WM_FONTCHANGE, WPARAM(0), LPARAM(0), SMTO_ABORTIFHUNG, 1000, None);
        }
    }
}
