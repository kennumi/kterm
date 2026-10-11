//! One terminal window: its shell session, input handling, drawing and animations.

use std::sync::Arc;
use std::time::Instant;

use alacritty_terminal::event::{Event as TermEvent, EventListener, Notify, OnResize, WindowSize};
use alacritty_terminal::event_loop::{EventLoop as PtyLoop, Msg, Notifier};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config as TermConfig, Term, TermMode, viewport_to_point};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color, CursorShape, CursorStyle, NamedColor, Rgb as VteRgb};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta};
use winit::event_loop::EventLoopProxy;
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::platform::windows::WindowExtWindows;
use winit::window::{CursorIcon, ResizeDirection, Window, WindowId};

use crate::UserEvent;
use crate::config::Config;
use crate::cursor::Trail;
use crate::render::{CellInst, Gpu, QuadInst, Surface, pack, rect};
use crate::theme::{Rgb, Theme};
use crate::win;

const OPEN_SECS: f32 = 0.16;
const CLOSE_SECS: f32 = 0.22;
const MOVE_SECS: f32 = 0.20; // tiling re-layout
const BELL_SECS: f32 = 0.15;
const BLINK_SECS: f32 = 1.0; // one on/off cycle
const BLINK_STOP_AFTER: f32 = 15.0;
const RESIZE_BORDER: f64 = 4.0; // logical px, outermost part of the padding
const GRAB_FADE_SECS: f32 = 0.15;
const DOUBLE_CLICK_SECS: f32 = 0.4;

/// x, y, w, h in physical pixels.
pub type Rect = (i32, i32, i32, i32);

/// State shared by all windows.
pub struct Shared {
    pub gpu: Gpu,
    pub cfg: Config,
    pub theme: Theme,
    pub palette: [Rgb; 269],
    pub user: Vec<char>, // %USERNAME%, masked on screen when `hide_user` is on
}

impl Shared {
    pub fn set_theme(&mut self, theme: Theme) {
        self.palette = build_palette(&theme);
        self.theme = theme;
    }

    pub fn term_config(&self) -> TermConfig {
        let shape = match self.cfg.cursor_shape.as_str() {
            "beam" => CursorShape::Beam,
            "underline" => CursorShape::Underline,
            _ => CursorShape::Block,
        };
        TermConfig { default_cursor_style: CursorStyle { shape, blinking: self.cfg.cursor_blink }, ..Default::default() }
    }
}

#[derive(Clone)]
pub struct Listener(WindowId, EventLoopProxy<UserEvent>);

impl EventListener for Listener {
    fn send_event(&self, e: TermEvent) {
        let _ = self.1.send_event(UserEvent::Term(self.0, e));
    }
}

/// A shell running in a ConPTY, parsed into an alacritty `Term` on its own thread.
pub struct Session {
    pub term: Arc<FairMutex<Term<Listener>>>,
    pty: Notifier,
}

impl Session {
    fn spawn(id: WindowId, proxy: &EventLoopProxy<UserEvent>, size: WindowSize, sh: &Shared) -> Self {
        let profile = win::data_dir().join("profile.ps1");
        let _ = std::fs::write(&profile, include_str!("../shell/profile.ps1"));
        let on_path = |exe: &str| {
            std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(exe).is_file()))
        };
        let shell = if on_path("pwsh.exe") { "pwsh.exe" } else { "powershell.exe" };
        // Dot-sourced so its functions and prompt land in the global scope.
        let args = ["-NoLogo", "-NoExit", "-ExecutionPolicy", "Bypass", "-Command"].map(String::from);
        let dot = format!(". '{}'", profile.display());
        let accent = sh.theme.cursor;
        let options = tty::Options {
            shell: Some(tty::Shell::new(shell.into(), [&args[..], &[dot]].concat())),
            working_directory: std::env::var_os("USERPROFILE").map(Into::into),
            drain_on_exit: false,
            env: [
                ("TERM", "xterm-256color".into()),
                ("COLORTERM", "truecolor".into()),
                ("TERM_PROGRAM", "kterm".into()),
                ("KTERM_ACCENT", format!("{};{};{}", accent[0], accent[1], accent[2])),
            ]
            .map(|(k, v)| (k.to_string(), v))
            .into(),
            escape_args: true,
        };
        let listener = Listener(id, proxy.clone());
        let dims = TermSize::new(size.num_cols as usize, size.num_lines as usize);
        let term = Arc::new(FairMutex::new(Term::new(sh.term_config(), &dims, listener.clone())));
        let pty = tty::new(&options, size, 0).expect("failed to start shell");
        let pty_loop = PtyLoop::new(term.clone(), listener, pty, false, false).expect("pty event loop");
        let notifier = Notifier(pty_loop.channel());
        pty_loop.spawn();
        Self { term, pty: notifier }
    }
}

impl Drop for Session {
    /// Stops the PTY thread; dropping the ConPTY ends the shell and everything it started.
    fn drop(&mut self) {
        let _ = self.pty.0.send(Msg::Shutdown);
    }
}

fn build_palette(t: &Theme) -> [Rgb; 269] {
    let mut p = [[0; 3]; 269];
    p[..16].copy_from_slice(&t.ansi);
    let level = |x: usize| if x == 0 { 0 } else { (x * 40 + 55) as u8 };
    for i in 0..216 {
        p[16 + i] = [level(i / 36), level(i / 6 % 6), level(i % 6)];
    }
    for i in 0..24 {
        p[232 + i] = [(8 + i * 10) as u8; 3];
    }
    p[NamedColor::Foreground as usize] = t.fg;
    p[NamedColor::Background as usize] = t.bg;
    p[NamedColor::Cursor as usize] = t.cursor;
    for i in 0..8 {
        p[NamedColor::DimBlack as usize + i] = dim(t.ansi[i]);
    }
    p[NamedColor::BrightForeground as usize] = t.fg;
    p[NamedColor::DimForeground as usize] = dim(t.fg);
    p
}

/// Start of every case-insensitive, non-overlapping match of `needle` in `hay`. Matches span row
/// ends too, so a name wrapped onto the next line is still found.
fn find_ci(hay: &[char], needle: &[char]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut i = 0;
    while !needle.is_empty() && i + needle.len() <= hay.len() {
        if hay[i..i + needle.len()].iter().zip(needle).all(|(a, b)| a.to_lowercase().eq(b.to_lowercase())) {
            out.push(i);
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

fn dim(c: Rgb) -> Rgb {
    c.map(|x| (x as u16 * 2 / 3) as u8)
}

fn resolve(palette: &[Rgb; 269], colors: &Colors, c: Color) -> Rgb {
    let i = match c {
        Color::Spec(c) => return [c.r, c.g, c.b],
        Color::Named(n) => n as usize,
        Color::Indexed(i) => i as usize,
    };
    colors[i].map_or(palette[i], |c| [c.r, c.g, c.b])
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

fn ease_in_out(t: f32) -> f32 {
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}

struct Motion {
    from: Rect,
    to: Rect,
    start: Instant,
}

pub struct TermWin {
    pub window: Arc<Window>,
    surf: Surface,
    pub s: Session,
    /// Laid out by the tiler (as opposed to floating at its remembered spot).
    pub tiled: bool,
    /// Close animation finished: the app should drop this window.
    pub closed: bool,
    closing: bool,
    anim: Option<Instant>, // open/close animation start
    motion: Option<Motion>,
    trail: Trail,
    /// Until then the cursor moves without a trail: the grid was just re-laid out (font, padding,
    /// window size) and the shell is still redrawing.
    snap_trail_until: Instant,
    pub mods: ModifiersState,
    mouse: PhysicalPosition<f64>,
    selecting: bool,
    clicks: (Instant, u8),
    scroll_px: f64,
    last_frame: Instant,
    pub activity: Instant,
    /// Still animating: the app draws the next frame from `about_to_wait`.
    pub wants_frame: bool,
    bell: Option<Instant>,
    grab: (bool, Instant), // pointer over the move frame, since when
    cells: Vec<CellInst>,
    quads: Vec<QuadInst>,
}

impl TermWin {
    /// `rect` is where the (still hidden) window should appear.
    pub fn new(window: Arc<Window>, surf: Surface, rect: Rect, proxy: &EventLoopProxy<UserEvent>, sh: &mut Shared) -> Self {
        window.set_outer_position(PhysicalPosition::new(rect.0, rect.1));
        let _ = window.request_inner_size(PhysicalSize::new(rect.2 as u32, rect.3 as u32));
        window.set_outer_position(PhysicalPosition::new(rect.0, rect.1)); // DPI changes can shift it
        let size = grid_size(&window, sh);
        let s = Session::spawn(window.id(), proxy, size, sh);
        let now = Instant::now();
        let mut w = Self {
            window,
            surf,
            s,
            tiled: false,
            closed: false,
            closing: false,
            anim: None,
            motion: None,
            trail: Trail::new(),
            snap_trail_until: now,
            mods: ModifiersState::empty(),
            mouse: PhysicalPosition::new(0.0, 0.0),
            selecting: false,
            clicks: (now, 0),
            scroll_px: 0.0,
            last_frame: now,
            activity: now,
            wants_frame: false,
            bell: None,
            grab: (false, now),
            cells: Vec::with_capacity(16384),
            quads: Vec::with_capacity(256),
        };
        let inner = w.window.inner_size();
        sh.gpu.resize(&mut w.surf, inner.width, inner.height);
        w.window.set_visible(true);
        w.window.focus_window();
        w.anim = Some(now);
        w.window.request_redraw();
        w
    }

    pub fn is_closing(&self) -> bool {
        self.closing
    }

    fn scale(&self) -> f32 {
        self.window.scale_factor() as f32
    }

    fn pad(&self, sh: &Shared) -> f32 {
        (sh.cfg.padding * self.scale()).round()
    }

    pub fn write(&self, bytes: Vec<u8>) {
        self.s.pty.notify(bytes);
    }

    pub fn close(&mut self) {
        if !self.closing {
            self.closing = true;
            self.anim = Some(Instant::now());
            // The DWM shadow can't fade; drop it first so it doesn't linger after the content.
            self.window.set_undecorated_shadow(false);
            self.window.request_redraw();
        }
    }

    pub fn rect(&self) -> Rect {
        let p = self.window.outer_position().unwrap_or_default();
        let s = self.window.outer_size();
        (p.x, p.y, s.width as i32, s.height as i32)
    }

    /// Glide to a new rect. The PTY is resized once, at the end.
    pub fn move_to(&mut self, to: Rect) {
        if self.rect() != to {
            self.motion = Some(Motion { from: self.rect(), to, start: Instant::now() });
            self.wants_frame = true;
        }
    }

    pub fn on_surface_resized(&mut self, sh: &mut Shared, w: u32, h: u32) {
        sh.gpu.resize(&mut self.surf, w, h);
        if self.motion.is_none() {
            self.on_resize(sh);
        }
    }

    /// Re-fit the terminal grid to the window.
    pub fn on_resize(&mut self, sh: &mut Shared) {
        let ws = grid_size(&self.window, sh);
        self.s.term.lock().resize(TermSize::new(ws.num_cols as usize, ws.num_lines as usize));
        self.s.pty.on_resize(ws);
        self.snap_trail_until = Instant::now() + std::time::Duration::from_millis(300);
        self.window.request_redraw();
    }

    pub fn on_focus(&mut self, focused: bool) {
        let mut term = self.s.term.lock();
        term.is_focused = focused;
        let report = term.mode().contains(TermMode::FOCUS_IN_OUT);
        drop(term);
        if report {
            self.write(if focused { b"\x1b[I".to_vec() } else { b"\x1b[O".to_vec() });
        }
        self.activity = Instant::now();
        self.window.request_redraw();
    }

    /// Handle an event from this window's terminal. Returns true when the shell has exited.
    pub fn on_term_event(&mut self, ev: TermEvent, sh: &mut Shared) -> bool {
        match ev {
            TermEvent::Wakeup | TermEvent::CursorBlinkingChange => self.window.request_redraw(),
            TermEvent::Bell => {
                self.bell = Some(Instant::now());
                self.window.request_redraw();
            }
            TermEvent::PtyWrite(s) => self.write(s.into_bytes()),
            TermEvent::ClipboardStore(_, s) => win::clipboard_set(&s),
            TermEvent::ClipboardLoad(_, f) => self.write(f(&win::clipboard_get().unwrap_or_default()).into_bytes()),
            TermEvent::ColorRequest(i, f) => {
                let [r, g, b] = self.s.term.lock().colors()[i].map_or(sh.palette[i], |c| [c.r, c.g, c.b]);
                self.write(f(VteRgb { r, g, b }).into_bytes());
            }
            TermEvent::TextAreaSizeRequest(f) => self.write(f(grid_size(&self.window, sh)).into_bytes()),
            TermEvent::ChildExit(_) => return true,
            TermEvent::Title(t) => self.window.set_title(&t),
            TermEvent::ResetTitle => self.window.set_title("KTerm"),
            _ => {}
        }
        false
    }

    // ---- input -------------------------------------------------------------------------------

    /// Returns true when the key asks to close this window (Ctrl+W).
    pub fn on_key(&mut self, ev: KeyEvent) -> bool {
        if ev.state != ElementState::Pressed || self.closing {
            return false;
        }
        let m = self.mods;
        if let Key::Character(c) = &ev.logical_key {
            if m.control_key() && !m.alt_key() && c.eq_ignore_ascii_case("w") {
                return true;
            }
            if m.control_key() && !m.alt_key() && c.eq_ignore_ascii_case("c") {
                // Ctrl+Shift+C always copies; Ctrl+C copies only when there is a selection.
                if m.shift_key() || self.s.term.lock().selection.is_some() {
                    self.copy();
                    return false;
                }
            }
            if m.control_key() && m.shift_key() && c.eq_ignore_ascii_case("v") {
                self.paste();
                return false;
            }
        }
        let app_cursor = self.s.term.lock().mode().contains(TermMode::APP_CURSOR);
        if let Some(bytes) = key_bytes(&ev, m, app_cursor) {
            {
                let mut term = self.s.term.lock();
                term.selection = None;
                term.scroll_display(Scroll::Bottom);
            }
            self.activity = Instant::now();
            self.write(bytes);
        }
        false
    }

    fn copy(&mut self) {
        let mut term = self.s.term.lock();
        if let Some(text) = term.selection_to_string() {
            win::clipboard_set(&text);
        }
        term.selection = None;
        drop(term);
        self.window.request_redraw();
    }

    fn paste(&mut self) {
        let Some(text) = win::clipboard_get() else { return };
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        let bytes = if self.s.term.lock().mode().contains(TermMode::BRACKETED_PASTE) {
            format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', ""))
        } else {
            text
        };
        self.write(bytes.into_bytes());
    }

    fn grid_point(&self, sh: &mut Shared) -> (Point, Side) {
        let (cw, ch, _) = sh.gpu.cell(self.scale());
        let pad = self.pad(sh) as f64;
        let term = self.s.term.lock();
        let x = ((self.mouse.x - pad) / cw as f64).max(0.0);
        let y = ((self.mouse.y - pad) / ch as f64).max(0.0);
        let col = (x as usize).min(term.columns() - 1);
        let row = (y as usize).min(term.screen_lines() - 1);
        let side = if x.fract() < 0.5 { Side::Left } else { Side::Right };
        (viewport_to_point(term.grid().display_offset(), Point::new(row, Column(col))), side)
    }

    fn edge(&self) -> Option<ResizeDirection> {
        use ResizeDirection::*;
        let s = self.window.inner_size();
        let b = RESIZE_BORDER * self.window.scale_factor();
        let (x, y) = (self.mouse.x, self.mouse.y);
        let (l, r, t, d) = (x < b, x > s.width as f64 - b, y < b, y > s.height as f64 - b);
        Some(match (l, r, t, d) {
            (true, _, true, _) => NorthWest,
            (_, true, true, _) => NorthEast,
            (true, _, _, true) => SouthWest,
            (_, true, _, true) => SouthEast,
            (true, ..) => West,
            (_, true, ..) => East,
            (_, _, true, _) => North,
            (_, _, _, true) => South,
            _ => return None,
        })
    }

    /// The padding around the text is a frame: outer edge resizes, the rest moves the window.
    fn on_frame(&self, sh: &mut Shared) -> bool {
        let (cw, ch, _) = sh.gpu.cell(self.scale());
        let ws = grid_size(&self.window, sh);
        let pad = self.pad(sh) as f64;
        let (x, y) = (self.mouse.x, self.mouse.y);
        x < pad || y < pad || x >= pad + ws.num_cols as f64 * cw as f64 || y >= pad + ws.num_lines as f64 * ch as f64
    }

    pub fn on_mouse(&mut self, state: ElementState, button: MouseButton, sh: &mut Shared) {
        match (state, button) {
            (ElementState::Pressed, MouseButton::Left) => {
                if let Some(dir) = self.edge() {
                    let _ = self.window.drag_resize_window(dir);
                } else if self.mods.alt_key() || self.on_frame(sh) {
                    let _ = self.window.drag_window();
                } else {
                    let now = Instant::now();
                    let quick = (now - self.clicks.0).as_secs_f32() < DOUBLE_CLICK_SECS;
                    self.clicks = (now, if quick { self.clicks.1 % 3 + 1 } else { 1 });
                    let ty = match self.clicks.1 {
                        2 => SelectionType::Semantic,
                        3 => SelectionType::Lines,
                        _ => SelectionType::Simple,
                    };
                    let (point, side) = self.grid_point(sh);
                    self.s.term.lock().selection = Some(Selection::new(ty, point, side));
                    self.selecting = true;
                    self.window.request_redraw();
                }
            }
            (ElementState::Released, MouseButton::Left) => self.selecting = false,
            // Windows Terminal habit: right click copies a selection, otherwise pastes.
            (ElementState::Pressed, MouseButton::Right) => {
                if self.s.term.lock().selection.as_ref().is_some_and(|s| !s.is_empty()) {
                    self.copy();
                } else {
                    self.paste();
                }
            }
            _ => {}
        }
    }

    pub fn on_cursor_moved(&mut self, pos: PhysicalPosition<f64>, sh: &mut Shared) {
        self.mouse = pos;
        if self.selecting {
            let (point, side) = self.grid_point(sh);
            if let Some(sel) = self.s.term.lock().selection.as_mut() {
                sel.update(point, side);
            }
            self.window.request_redraw();
            return;
        }
        use ResizeDirection::*;
        let edge = self.edge();
        let frame = edge.is_none() && self.on_frame(sh);
        self.set_grab(frame);
        self.window.set_cursor(match edge {
            Some(North | South) => CursorIcon::NsResize,
            Some(East | West) => CursorIcon::EwResize,
            Some(NorthWest | SouthEast) => CursorIcon::NwseResize,
            Some(NorthEast | SouthWest) => CursorIcon::NeswResize,
            None if frame || self.mods.alt_key() => CursorIcon::Move,
            None => CursorIcon::Text,
        });
    }

    /// Fades the grab handle in or out.
    pub fn set_grab(&mut self, on: bool) {
        if self.grab.0 != on {
            self.grab = (on, Instant::now());
            self.window.request_redraw();
        }
    }

    pub fn on_wheel(&mut self, delta: MouseScrollDelta, sh: &mut Shared) {
        let ch = sh.gpu.cell(self.scale()).1 as f64;
        self.scroll_px += match delta {
            MouseScrollDelta::LineDelta(_, y) => y as f64 * 3.0 * ch,
            MouseScrollDelta::PixelDelta(p) => p.y,
        };
        let lines = (self.scroll_px / ch) as i32;
        if lines == 0 {
            return;
        }
        self.scroll_px -= lines as f64 * ch;
        let mut term = self.s.term.lock();
        let mode = *term.mode();
        if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            // Full-screen apps (less, vim) get arrow keys instead.
            let key: &[u8] = match (lines > 0, mode.contains(TermMode::APP_CURSOR)) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            drop(term);
            self.write(key.repeat(lines.unsigned_abs() as usize));
        } else {
            term.scroll_display(Scroll::Delta(lines));
            drop(term);
            self.window.request_redraw();
        }
    }

    // ---- drawing -----------------------------------------------------------------------------

    pub fn draw(&mut self, sh: &mut Shared) {
        self.wants_frame = false;
        if self.closed {
            return;
        }
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        let mut animating = false;

        // Tiling glide: move/resize the native window a step per frame.
        if let Some(m) = &self.motion {
            let t = ((now - m.start).as_secs_f32() / MOVE_SECS).min(1.0);
            let e = ease_out(t);
            let lerp = |a: i32, b: i32| a + ((b - a) as f32 * e).round() as i32;
            let (f, to) = (m.from, m.to);
            let r = (lerp(f.0, to.0), lerp(f.1, to.1), lerp(f.2, to.2), lerp(f.3, to.3));
            self.window.set_outer_position(PhysicalPosition::new(r.0, r.1));
            let _ = self.window.request_inner_size(PhysicalSize::new(r.2 as u32, r.3 as u32));
            if t < 1.0 {
                animating = true;
            } else {
                self.motion = None;
                self.on_resize(sh);
            }
        }

        let (mut opacity, mut zoom) = (1.0f32, 1.0f32);
        if let Some(start) = self.anim {
            let secs = if self.closing { CLOSE_SECS } else { OPEN_SECS };
            let t = ((now - start).as_secs_f32() / secs).min(1.0);
            let k = if self.closing { 1.0 - ease_in_out(t) } else { ease_out(t) };
            opacity = k;
            zoom = if self.closing { 0.94 + 0.06 * k } else { 0.96 + 0.04 * k };
            if t < 1.0 {
                animating = true;
            } else {
                self.anim = None;
                if self.closing {
                    self.window.set_visible(false);
                    self.closed = true;
                    return;
                }
            }
        }

        let scale = self.scale();
        let (cw, ch, base) = sh.gpu.cell(scale);
        let pad = self.pad(sh);
        let focused = self.window.has_focus();
        self.cells.clear();
        self.quads.clear();

        let term_arc = self.s.term.clone();
        let term = term_arc.lock();
        let content = term.renderable_content();
        let offset = content.display_offset as i32;
        let (cols, lines) = (term.columns(), term.screen_lines());
        let mut text = Vec::new(); // the screen's chars, one per cell, for `hideme`
        for cell in content.display_iter {
            text.push(cell.c);
            let (row, col) = ((cell.point.line.0 + offset) as u32, cell.point.column.0 as u32);
            let flags = cell.flags;
            let mut fg = resolve(&sh.palette, content.colors, cell.fg);
            let mut bg = resolve(&sh.palette, content.colors, cell.bg);
            let mut bg_default = cell.bg == Color::Named(NamedColor::Background);
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
                bg_default = false;
            }
            if flags.contains(Flags::DIM) {
                fg = dim(fg);
            }
            if content.selection.is_some_and(|s| s.contains(cell.point)) {
                bg = sh.theme.selection;
                bg_default = false;
            }
            let glyph = if flags.intersects(Flags::HIDDEN | Flags::WIDE_CHAR_SPACER) {
                0
            } else {
                let style = flags.contains(Flags::BOLD) as u8 | (flags.contains(Flags::ITALIC) as u8) << 1;
                sh.gpu.glyph(cell.c, style, scale)
            };
            let (x, y) = (pad + col as f32 * cw, pad + row as f32 * ch);
            if flags.intersects(Flags::ALL_UNDERLINES) {
                self.quads.push(rect(x, y + base + scale.round(), cw, scale.round(), pack(fg, 255)));
            }
            if flags.contains(Flags::STRIKEOUT) {
                self.quads.push(rect(x, y + (ch / 2.0).round(), cw, scale.round(), pack(fg, 255)));
            }
            self.cells.push(CellInst {
                pos: col | row << 16,
                glyph,
                fg: pack(fg, 255),
                bg: pack(bg, if bg_default { 0 } else { 255 }),
            });
        }
        // `hideme`: draw the username as stars wherever it shows up. Only the drawing changes, so
        // copying still gets the real text.
        if sh.cfg.hide_user {
            let star = sh.gpu.glyph('*', 0, scale);
            for i in find_ci(&text, &sh.user) {
                for c in &mut self.cells[i..i + sh.user.len()] {
                    c.glyph = star;
                }
            }
        }

        // Cursor + kitty trail.
        let cur = content.cursor;
        let crow = cur.point.line.0 + offset;
        if cur.shape != CursorShape::Hidden && (0..lines as i32).contains(&crow) {
            let (x, y) = (pad + cur.point.column.0 as f32 * cw, pad + crow as f32 * ch);
            self.trail.set_target(x, y, cw, ch);
            if !sh.cfg.cursor_trail || now < self.snap_trail_until {
                self.trail.snap();
            }
            let moving = self.trail.step(dt * sh.cfg.trail_speed.max(0.1));
            let color = sh.theme.cursor;
            if moving {
                animating = true;
                self.quads.push(QuadInst { pts: self.trail.corners, color: pack(color, 170) });
            }
            let idle = (now - self.activity).as_secs_f32();
            let alpha = if !focused || moving || !sh.cfg.cursor_blink || idle > BLINK_STOP_AFTER {
                1.0
            } else {
                animating = true;
                // Eased blink: plateaus at on/off with smooth fades in between.
                let s = (idle / BLINK_SECS * std::f32::consts::TAU).cos();
                ((s * 2.0).clamp(-1.0, 1.0) + 1.0) * 0.5
            };
            let c = pack(color, (alpha * 255.0) as u8);
            let t = scale.round().max(1.0) * 2.0;
            match if focused { cur.shape } else { CursorShape::HollowBlock } {
                CursorShape::Beam => self.quads.push(rect(x, y, t, ch, c)),
                CursorShape::Underline => self.quads.push(rect(x, y + ch - t, cw, t, c)),
                CursorShape::HollowBlock => {
                    let t = t / 2.0;
                    self.quads.extend([
                        rect(x, y, cw, t, c),
                        rect(x, y + ch - t, cw, t, c),
                        rect(x, y, t, ch, c),
                        rect(x + cw - t, y, t, ch, c),
                    ]);
                }
                _ => {
                    self.quads.push(rect(x, y, cw, ch, c));
                    // Text under a block cursor takes the background color.
                    let i = crow as usize * cols + cur.point.column.0;
                    if alpha > 0.5 && let Some(cell) = self.cells.get_mut(i) {
                        cell.fg = pack(sh.theme.bg, 255);
                    }
                }
            }
        }
        drop(term);

        // Grab handle: a small pill centered in the top padding, shown while hovering the frame.
        let t = ((now - self.grab.1).as_secs_f32() / GRAB_FADE_SECS).min(1.0);
        let k = if self.grab.0 { t } else { 1.0 - t };
        if t < 1.0 {
            animating = true;
        }
        if k > 0.0 {
            let (w, h) = (40.0 * scale, (3.0 * scale).round());
            let x = ((self.window.inner_size().width as f32 - w) / 2.0).round();
            self.quads.push(rect(x, ((pad - h) / 2.0).round(), w, h, pack(sh.theme.cursor, (k * 200.0) as u8)));
        }

        let mut bg = sh.theme.bg.map(|c| c as f64 / 255.0);
        if let Some(start) = self.bell {
            let t = (now - start).as_secs_f32() / BELL_SECS;
            if t < 1.0 {
                animating = true;
                let k = (1.0 - t as f64) * 0.35;
                for (b, a) in bg.iter_mut().zip(sh.theme.cursor) {
                    *b += (a as f64 / 255.0 - *b) * k;
                }
            } else {
                self.bell = None;
            }
        }
        let a = sh.cfg.opacity.clamp(0.0, 1.0) as f64 * opacity as f64;
        sh.gpu.render(&mut self.surf, scale, &self.cells, &self.quads, [bg[0] * a, bg[1] * a, bg[2] * a, a], pad, zoom, opacity);
        if std::mem::take(&mut sh.gpu.atlas_reset) {
            animating = true;
        }
        self.wants_frame = animating;
    }
}

pub fn grid_size(window: &Window, sh: &mut Shared) -> WindowSize {
    let scale = window.scale_factor() as f32;
    let (cw, ch, _) = sh.gpu.cell(scale);
    let s = window.inner_size();
    let pad = 2.0 * (sh.cfg.padding * scale).round();
    WindowSize {
        num_cols: (((s.width as f32 - pad) / cw) as u16).max(2),
        num_lines: (((s.height as f32 - pad) / ch) as u16).max(1),
        cell_width: cw as u16,
        cell_height: ch as u16,
    }
}

/// Keyboard event to the bytes a VT application expects (xterm encoding).
fn key_bytes(ev: &KeyEvent, m: ModifiersState, app_cursor: bool) -> Option<Vec<u8>> {
    let (shift, alt, ctrl) = (m.shift_key(), m.alt_key(), m.control_key());
    let modn = 1 + shift as u8 + 2 * alt as u8 + 4 * ctrl as u8;
    let alt_prefix = |b: &[u8]| if alt { [b"\x1b", b].concat() } else { b.to_vec() };
    let cursor = |c: char| match (modn, app_cursor) {
        (1, true) => format!("\x1bO{c}"),
        (1, false) => format!("\x1b[{c}"),
        _ => format!("\x1b[1;{modn}{c}"),
    };
    let tilde = |n: u8| if modn == 1 { format!("\x1b[{n}~") } else { format!("\x1b[{n};{modn}~") };
    let ss3 = |c: char| if modn == 1 { format!("\x1bO{c}") } else { format!("\x1b[1;{modn}{c}") };
    let s = match &ev.logical_key {
        Key::Named(k) => match k {
            NamedKey::Enter => return Some(alt_prefix(b"\r")),
            NamedKey::Tab if shift => "\x1b[Z".into(),
            NamedKey::Tab => return Some(alt_prefix(b"\t")),
            NamedKey::Backspace if ctrl => "\x08".into(),
            NamedKey::Backspace => return Some(alt_prefix(b"\x7f")),
            NamedKey::Escape => "\x1b".into(),
            NamedKey::Space if ctrl => "\0".into(),
            NamedKey::Space => return Some(alt_prefix(b" ")),
            NamedKey::ArrowUp => cursor('A'),
            NamedKey::ArrowDown => cursor('B'),
            NamedKey::ArrowRight => cursor('C'),
            NamedKey::ArrowLeft => cursor('D'),
            NamedKey::Home => cursor('H'),
            NamedKey::End => cursor('F'),
            NamedKey::Insert => tilde(2),
            NamedKey::Delete => tilde(3),
            NamedKey::PageUp => tilde(5),
            NamedKey::PageDown => tilde(6),
            NamedKey::F1 => ss3('P'),
            NamedKey::F2 => ss3('Q'),
            NamedKey::F3 => ss3('R'),
            NamedKey::F4 => ss3('S'),
            NamedKey::F5 => tilde(15),
            NamedKey::F6 => tilde(17),
            NamedKey::F7 => tilde(18),
            NamedKey::F8 => tilde(19),
            NamedKey::F9 => tilde(20),
            NamedKey::F10 => tilde(21),
            NamedKey::F11 => tilde(23),
            NamedKey::F12 => tilde(24),
            _ => return ev.text.as_deref().map(|t| alt_prefix(t.as_bytes())),
        },
        Key::Character(c) => {
            if ctrl && !alt && let [b] = c.as_bytes() {
                let code = match b.to_ascii_lowercase() {
                    b @ b'a'..=b'z' => Some(b & 0x1f),
                    b'[' | b'3' => Some(0x1b),
                    b'\\' | b'4' => Some(0x1c),
                    b']' | b'5' => Some(0x1d),
                    b'^' | b'6' => Some(0x1e),
                    b'_' | b'-' | b'/' | b'7' => Some(0x1f),
                    b'@' | b'2' => Some(0),
                    _ => None,
                };
                if let Some(code) = code {
                    return Some(vec![code]);
                }
            }
            // AltGr (Ctrl+Alt) characters arrive as plain text.
            let t = ev.text.as_deref()?;
            return Some(if alt && !ctrl { alt_prefix(t.as_bytes()) } else { t.as_bytes().to_vec() });
        }
        _ => return None,
    };
    Some(s.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::find_ci;

    #[test]
    fn finds_username_any_case() {
        let hay: Vec<char> = r"C:\Users\Yousif\x yousifYOUSIF".chars().collect();
        let user: Vec<char> = "Yousif".chars().collect();
        assert_eq!(find_ci(&hay, &user), vec![9, 18, 24]);
        assert!(find_ci(&hay, &[]).is_empty());
    }
}
