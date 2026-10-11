//! Palette derived from the Windows accent color, built in OKLCH so every hue has
//! the same perceived lightness. ANSI hues stay semantic (red is still red); the
//! accent drives the background tint, cursor, trail, selection and prompt.

pub type Rgb = [u8; 3];

pub struct Theme {
    pub ansi: [Rgb; 16],
    pub fg: Rgb,
    pub bg: Rgb,
    pub cursor: Rgb,
    pub selection: Rgb,
}

/// Arch blue, used when the accent can't be read.
pub const FALLBACK_ACCENT: Rgb = [0x17, 0x93, 0xd1];

const HUES: [f32; 6] = [25.0, 145.0, 95.0, 260.0, 330.0, 200.0]; // r g y b m c

impl Theme {
    pub fn from_accent(accent: Rgb) -> Self {
        let (al, ac, h) = to_oklch(accent);
        let mut ansi = [[0; 3]; 16];
        for (i, &hue) in HUES.iter().enumerate() {
            ansi[1 + i] = oklch(0.72, 0.13, hue);
            ansi[9 + i] = oklch(0.82, 0.12, hue);
        }
        ansi[0] = oklch(0.32, 0.02, h);
        ansi[8] = oklch(0.60, 0.02, h);
        ansi[7] = oklch(0.86, 0.01, h);
        ansi[15] = oklch(0.97, 0.005, h);
        Self {
            ansi,
            fg: oklch(0.92, 0.01, h),
            bg: oklch(0.17, 0.025, h),
            cursor: oklch(al.clamp(0.68, 0.85), ac.max(0.08), h),
            selection: oklch(0.40, 0.08, h),
        }
    }
}

fn to_lin(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn from_lin(c: f32) -> f32 {
    if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

fn to_oklch([r, g, b]: Rgb) -> (f32, f32, f32) {
    let (r, g, b) = (to_lin(r), to_lin(g), to_lin(b));
    let l = (0.4122214708 * r + 0.5363288310 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    let ll = 0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s;
    let a = 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s;
    let bb = 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s;
    (ll, a.hypot(bb), bb.atan2(a).to_degrees().rem_euclid(360.0))
}

fn oklch_lin(l: f32, c: f32, h: f32) -> [f32; 3] {
    let (a, b) = (c * h.to_radians().cos(), c * h.to_radians().sin());
    let l_ = (l + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let m_ = (l - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let s_ = (l - 0.0894841775 * a - 1.2914855480 * b).powi(3);
    [
        4.0767416621 * l_ - 3.3077115913 * m_ + 0.2309699292 * s_,
        -1.2684380046 * l_ + 2.6097574011 * m_ - 0.3413193965 * s_,
        -0.0041960863 * l_ - 0.7034186147 * m_ + 1.7076147010 * s_,
    ]
}

/// OKLCH to sRGB, lowering chroma until the color fits the sRGB gamut.
fn oklch(l: f32, mut c: f32, h: f32) -> Rgb {
    loop {
        let rgb = oklch_lin(l, c, h);
        if c <= 0.0 || rgb.iter().all(|v| (-1e-4..=1.0001).contains(v)) {
            return rgb.map(|v| (from_lin(v.clamp(0.0, 1.0)) * 255.0).round() as u8);
        }
        c -= 0.005;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lum(c: Rgb) -> f32 {
        0.2126 * to_lin(c[0]) + 0.7152 * to_lin(c[1]) + 0.0722 * to_lin(c[2])
    }

    fn contrast(a: Rgb, b: Rgb) -> f32 {
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn readable_for_every_accent() {
        for hue in (0..360).step_by(15) {
            let t = Theme::from_accent(oklch(0.6, 0.15, hue as f32));
            assert!(contrast(t.fg, t.bg) >= 7.0);
            for i in (1..16).filter(|&i| i != 8) {
                assert!(contrast(t.ansi[i], t.bg) >= 4.5, "hue {hue} color {i}");
            }
            assert!(contrast(t.ansi[8], t.bg) >= 3.0, "hue {hue} bright black");
            assert!(contrast(t.fg, t.selection) >= 4.5, "hue {hue} selection");
        }
        let (l, _, h) = to_oklch([0x17, 0x93, 0xd1]);
        assert!((l - 0.62).abs() < 0.03 && (h - 240.0).abs() < 10.0, "round trip {l} {h}");
    }
}
