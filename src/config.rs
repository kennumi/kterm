//! User settings in %APPDATA%\kterm\config.json. The shell's `config` command edits the file and
//! kterm picks changes up live.

use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState};

use crate::theme::Rgb;
use crate::win;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Config {
    pub font_family: String,
    pub font_size: f32,
    pub opacity: f32,
    pub padding: f32,
    pub cursor_shape: String, // block | beam | underline
    pub cursor_blink: bool,
    pub cursor_trail: bool,
    pub trail_speed: f32, // 1 = kitty's default; higher is faster
    pub accent: String,   // "auto" (Windows accent) or "#rrggbb"
    pub tiling: bool,     // off: new windows cascade over the others instead
    pub gaps: f32,        // tiling gaps, logical px
    pub hide_user: bool,  // shell prompt + on-screen masking of %USERNAME%
    // Key binds, "ctrl+shift+h" style: ctrl/shift/alt/win + a letter, digit, f1-f12, escape, enter, tab, space, `...
    pub key_toggle: String, // global: open / hide KTerm from anywhere
    pub key_new: String,    // global: open another window
    pub key_close: String,
    pub key_hide: String, // hide the window, shell keeps running; key_toggle brings it back
    pub key_copy: String,
    pub key_paste: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font_family: "JetBrainsMono Nerd Font".into(),
            font_size: 11.0,
            opacity: 0.92,
            padding: 12.0,
            cursor_shape: "block".into(),
            cursor_blink: true,
            cursor_trail: true,
            trail_speed: 1.0,
            accent: "auto".into(),
            tiling: true,
            gaps: 10.0,
            hide_user: false,
            key_toggle: "win+enter".into(),
            key_new: "win+shift+enter".into(),
            key_close: "ctrl+w".into(),
            key_hide: "ctrl+shift+h".into(),
            key_copy: "ctrl+shift+c".into(),
            key_paste: "ctrl+shift+v".into(),
        }
    }
}

pub fn path() -> PathBuf {
    win::data_dir().join("config.json")
}

pub fn mtime() -> Option<SystemTime> {
    std::fs::metadata(path()).and_then(|m| m.modified()).ok()
}

impl Config {
    /// Load, writing defaults on first run so the shell's `config` command has a file to edit.
    pub fn load() -> Self {
        match std::fs::read(path()) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => {
                let c = Self::default();
                if let Ok(json) = serde_json::to_vec_pretty(&c) {
                    let _ = std::fs::write(path(), json);
                }
                c
            }
        }
    }

    /// The fixed accent, or None to follow Windows.
    pub fn accent(&self) -> Option<Rgb> {
        let hex = self.accent.strip_prefix('#')?;
        let v = u32::from_str_radix(hex, 16).ok().filter(|_| hex.len() == 6)?;
        Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
    }
}

/// A key bind like "ctrl+shift+h" or "win+enter".
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Bind {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub win: bool,
    pub key: String, // lowercase: a letter, digit, f1-f12, escape, enter, tab, space, ...
}

impl Bind {
    pub fn parse(spec: &str) -> Self {
        let mut b = Self::default();
        for part in spec.split('+').map(|p| p.trim().to_lowercase()) {
            match part.as_str() {
                "ctrl" | "control" => b.ctrl = true,
                "shift" => b.shift = true,
                "alt" => b.alt = true,
                "win" | "super" => b.win = true,
                "esc" => b.key = "escape".into(),
                _ => b.key = part,
            }
        }
        b
    }

    pub fn mods_match(&self, ctrl: bool, shift: bool, alt: bool, win: bool) -> bool {
        (self.ctrl, self.shift, self.alt, self.win) == (ctrl, shift, alt, win)
    }

    /// Windows virtual-key code of the main key.
    pub fn vk(&self) -> Option<u16> {
        let k = self.key.as_str();
        let c = k.as_bytes();
        Some(match k {
            "enter" => 0x0D,
            "escape" => 0x1B,
            "tab" => 0x09,
            "space" => 0x20,
            "backspace" => 0x08,
            "`" | "grave" | "backtick" => 0xC0, // VK_OEM_3 on US layouts
            _ if c.len() == 1 && c[0].is_ascii_alphanumeric() => c[0].to_ascii_uppercase() as u16,
            _ if c.len() > 1 && c[0] == b'f' => 0x6F + k[1..].parse::<u16>().ok().filter(|n| (1..=12).contains(n))?,
            _ => return None,
        })
    }
}

/// Does this key event match a bind? Modifiers must match exactly.
pub fn key_matches(spec: &str, ev: &KeyEvent, m: ModifiersState) -> bool {
    let name = match &ev.logical_key {
        Key::Character(c) => c.to_lowercase(),
        Key::Named(n) => format!("{n:?}").to_lowercase(),
        _ => return false,
    };
    let b = Bind::parse(spec);
    !b.key.is_empty() && b.key == name && b.mods_match(m.control_key(), m.shift_key(), m.alt_key(), m.super_key())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds() {
        let b = Bind::parse("Win+Shift+Enter");
        assert!(b.win && b.shift && !b.ctrl && b.vk() == Some(0x0D));
        assert_eq!(Bind::parse("ctrl+F5").vk(), Some(0x74));
        assert_eq!(Bind::parse("alt+t").vk(), Some(b'T' as u16));
        assert_eq!(Bind::parse("win+`").vk(), Some(0xC0));
    }

    #[test]
    fn partial_files_and_accents() {
        let c: Config = serde_json::from_str(r##"{"font_size": 14, "accent": "#1793d1"}"##).unwrap();
        assert_eq!(c.font_size, 14.0);
        assert_eq!(c.opacity, Config::default().opacity);
        assert_eq!(c.accent(), Some([0x17, 0x93, 0xd1]));
        assert_eq!(Config::default().accent(), None);
        let bad = Config { accent: "#12345".into(), ..Default::default() };
        assert_eq!(bad.accent(), None);
    }
}
