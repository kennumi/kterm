//! User settings in %APPDATA%\kterm\config.json. The shell's `config` command edits the file and
//! kterm picks changes up live.

use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

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
    pub gaps: f32,        // tiling gaps, logical px
    pub hide_user: bool,  // read by the shell prompt
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
            gaps: 10.0,
            hide_user: false,
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

#[cfg(test)]
mod tests {
    use super::*;

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
