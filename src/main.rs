#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod cursor;
mod fonts;
mod render;
mod theme;
mod win;
mod window;

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use alacritty_terminal::event::Event as TermEvent;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::monitor::MonitorHandle;
use winit::dpi::PhysicalSize;
use winit::platform::windows::{CornerPreference, IconExtWindows, WindowAttributesExtWindows, WindowExtWindows};
use winit::window::{Icon, Window, WindowId};

use config::Config;
use render::Gpu;
use theme::Theme;
use window::{Rect, Shared, TermWin};

const CONFIG_POLL: Duration = Duration::from_millis(200);

pub enum UserEvent {
    Term(WindowId, TermEvent),
    /// Win+Enter: close the focused terminal, else focus one, else open one.
    Toggle,
    /// Shift+Win+Enter: always open another terminal, tiling it next to the others.
    NewWindow,
    /// Tray menu: bring this terminal forward.
    Focus(WindowId),
    /// Tray menu: close every terminal and exit.
    Quit,
    Accent,
    /// Global close / hide binds that use Win (the window never sees those keys).
    CloseFocused,
    HideFocused,
}

/// Dwindle tiling (Hyprland's default): each window takes half of what's left, splitting along the
/// longer side, so the first window ends up left/top and later ones fill in to the right/bottom.
fn dwindle(area: Rect, n: usize, gap: i32) -> Vec<Rect> {
    let (mut x, mut y, mut w, mut h) = (area.0 + gap, area.1 + gap, area.2 - 2 * gap, area.3 - 2 * gap);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        if i + 1 == n {
            out.push((x, y, w, h));
        } else if w >= h {
            let half = (w - gap) / 2;
            out.push((x, y, half, h));
            (x, w) = (x + half + gap, w - half - gap);
        } else {
            let half = (h - gap) / 2;
            out.push((x, y, w, half));
            (y, h) = (y + half + gap, h - half - gap);
        }
    }
    out
}

fn monitor_at(el: &ActiveEventLoop, (x, y): (i32, i32)) -> Option<MonitorHandle> {
    el.available_monitors()
        .find(|m| {
            let (p, s) = (m.position(), m.size());
            (p.x..p.x + s.width as i32).contains(&x) && (p.y..p.y + s.height as i32).contains(&y)
        })
        .or_else(|| el.primary_monitor())
}

fn monitor_name(w: &Window) -> Option<String> {
    w.current_monitor().and_then(|m| m.name())
}

struct App {
    proxy: EventLoopProxy<UserEvent>,
    sh: Option<Shared>,
    wins: Vec<TermWin>, // spawn order = tiling order
    placements: win::Placements,
    last_focus: Option<WindowId>,
    cfg_mtime: Option<SystemTime>,
    cfg_checked: Instant,
    cfg_pending: bool, // a check was throttled; do it when the poll interval is up
    start_hidden: bool,
    started: bool,
    admin: bool,
    _accent_watch: Option<windows::UI::ViewManagement::UISettings>,
}

impl App {
    fn theme(cfg: &Config) -> Theme {
        Theme::from_accent(cfg.accent().unwrap_or_else(win::accent))
    }

    /// Where a lone window sits on a monitor: its remembered spot, else centered at 60%.
    fn floating_rect(&self, m: &MonitorHandle) -> Rect {
        let (mp, ms) = (m.position(), m.size());
        let p = m
            .name()
            .and_then(|n| self.placements.get(&n).copied())
            .filter(|p| p.w <= ms.width && p.h <= ms.height) // monitor may have changed resolution
            .unwrap_or_else(|| {
                let (w, h) = (ms.width * 3 / 5, ms.height * 3 / 5);
                win::Placement { x: ((ms.width - w) / 2) as i32, y: ((ms.height - h) / 2) as i32, w, h }
            });
        (mp.x + p.x, mp.y + p.y, p.w as i32, p.h as i32)
    }

    /// Rects for `n` windows on monitor `m`.
    fn layout(&self, m: &MonitorHandle, n: usize) -> Vec<Rect> {
        if n <= 1 {
            return vec![self.floating_rect(m)];
        }
        let (p, s) = (m.position(), m.size());
        let area = win::work_area(p.x + s.width as i32 / 2, p.y + s.height as i32 / 2);
        let gap = (self.sh.as_ref().map_or(10.0, |sh| sh.cfg.gaps) * m.scale_factor() as f32).round() as i32;
        dwindle(area, n, gap)
    }

    /// With tiling off, the next window opens a step down-right of the newest one, like Windows'
    /// own cascade, starting over at the floating spot once that would leave the work area.
    fn cascade_rect(&self, m: &MonitorHandle) -> Rect {
        let name = m.name();
        let Some(prev) = self.wins.iter().rev().find(|w| !w.is_closing() && monitor_name(&w.window) == name) else {
            return self.floating_rect(m);
        };
        let step = (32.0 * m.scale_factor()).round() as i32;
        let (x, y, w, h) = prev.rect();
        let (p, s) = (m.position(), m.size());
        let area = win::work_area(p.x + s.width as i32 / 2, p.y + s.height as i32 / 2);
        if x + step + w > area.0 + area.2 || y + step + h > area.1 + area.3 {
            return self.floating_rect(m);
        }
        (x + step, y + step, w, h)
    }

    /// Re-tile every live window on monitor `m`.
    fn relayout(&mut self, m: &MonitorHandle) {
        let name = m.name();
        let idx: Vec<usize> = (0..self.wins.len())
            .filter(|&i| !self.wins[i].is_closing() && monitor_name(&self.wins[i].window) == name)
            .collect();
        if self.sh.as_ref().is_some_and(|sh| !sh.cfg.tiling) {
            // Windows stay where they are; each is floating, so its spot is remembered again.
            idx.iter().for_each(|&i| self.wins[i].tiled = false);
            return;
        }
        let rects = self.layout(m, idx.len());
        for (&i, r) in idx.iter().zip(rects) {
            self.wins[i].tiled = idx.len() > 1;
            self.wins[i].move_to(r);
        }
    }

    fn relayout_all(&mut self) {
        let monitors: Vec<MonitorHandle> = self.wins.iter().filter_map(|w| w.window.current_monitor()).collect();
        for m in monitors {
            self.relayout(&m);
        }
    }

    fn open_window(&mut self, el: &ActiveEventLoop) {
        let Some(m) = monitor_at(el, win::cursor_pos()) else { return };
        let attrs = Window::default_attributes()
            .with_title("KTerm")
            .with_skip_taskbar(true) // lives in the tray instead
            .with_decorations(false)
            .with_transparent(true)
            .with_visible(false)
            .with_class_name(win::CLASS)
            .with_no_redirection_bitmap(true)
            .with_undecorated_shadow(true)
            .with_corner_preference(CornerPreference::Round)
            .with_position(m.position()); // start on the right monitor so DPI is right from frame one
        let Ok(window) = el.create_window(attrs) else { return };
        // Load the icon (embedded by build.rs) at this monitor's DPI; the default size is 96-DPI and blurs.
        let icon = |px: f64| Icon::from_resource(1, Some(PhysicalSize::new(px as u32, px as u32))).ok();
        let sf = window.scale_factor();
        window.set_window_icon(icon((16.0 * sf).round()));
        window.set_taskbar_icon(icon((32.0 * sf).round())); // Alt+Tab
        let window = Arc::new(window);

        let surf = match &mut self.sh {
            Some(sh) => sh.gpu.surface(window.clone()),
            None => {
                let cfg = Config::load();
                self.cfg_mtime = config::mtime();
                let (gpu, surf) = Gpu::new(window.clone(), &cfg.font_family, cfg.font_size);
                let theme = Self::theme(&cfg);
                let mut sh = Shared { gpu, cfg, theme, palette: [[0; 3]; 269], user: std::env::var("USERNAME").unwrap_or_default().chars().collect() };
                sh.set_theme(Self::theme(&sh.cfg));
                self.sh = Some(sh);
                surf
            }
        };
        let name = m.name();
        let siblings = self.wins.iter().filter(|w| !w.is_closing() && monitor_name(&w.window) == name).count();
        let tiling = self.sh.as_ref().unwrap().cfg.tiling;
        let rect = if tiling { *self.layout(&m, siblings + 1).last().unwrap() } else { self.cascade_rect(&m) };
        let sh = self.sh.as_mut().unwrap();
        let w = TermWin::new(window, surf, rect, &self.proxy, sh);
        self.last_focus = Some(w.window.id());
        self.wins.push(w);
        if siblings > 0 {
            self.relayout(&m);
        }
    }

    fn save_placement(&mut self, i: usize) {
        let w = &self.wins[i];
        if w.tiled {
            return; // only a lone, floating window's spot is remembered
        }
        let (Some(m), Ok(p)) = (w.window.current_monitor(), w.window.outer_position()) else { return };
        let Some(name) = m.name() else { return };
        let (mp, s) = (m.position(), w.window.outer_size());
        let placement = win::Placement { x: p.x - mp.x, y: p.y - mp.y, w: s.width, h: s.height };
        if self.placements.insert(name, placement) != Some(placement) {
            win::save_placements(&self.placements);
        }
    }

    fn close(&mut self, i: usize) {
        self.save_placement(i);
        self.wins[i].close();
    }

    fn find(&self, id: WindowId) -> Option<usize> {
        self.wins.iter().position(|w| w.window.id() == id)
    }

    fn toggle(&mut self, el: &ActiveEventLoop) {
        let live = |w: &&TermWin| !w.is_closing();
        if let Some(i) = self.wins.iter().position(|w| !w.is_closing() && w.window.has_focus()) {
            self.close(i);
        } else if let Some(w) = self
            .wins
            .iter()
            .filter(live)
            .find(|w| Some(w.window.id()) == self.last_focus)
            .or_else(|| self.wins.iter().filter(live).last())
        {
            w.window.set_visible(true);
            w.window.focus_window();
        } else {
            self.open_window(el);
        }
    }

    /// Pick up edits made by the shell's `config` command.
    fn reload_config(&mut self) {
        if self.cfg_checked.elapsed() < CONFIG_POLL {
            self.cfg_pending = true;
            return;
        }
        self.cfg_checked = Instant::now();
        self.cfg_pending = false;
        let mtime = config::mtime();
        if mtime == self.cfg_mtime {
            return;
        }
        self.cfg_mtime = mtime;
        let Some(sh) = &mut self.sh else { return };
        let new = Config::load();
        if new == sh.cfg {
            return;
        }
        let old = std::mem::replace(&mut sh.cfg, new);
        win::set_hotkeys(&sh.cfg);
        let font = sh.gpu.set_font(&sh.cfg.font_family, sh.cfg.font_size);
        if old.accent != sh.cfg.accent {
            sh.set_theme(Self::theme(&sh.cfg));
        }
        let term_cfg = sh.term_config();
        for w in &mut self.wins {
            w.s.term.lock().set_options(term_cfg.clone());
            if font || old.padding != sh.cfg.padding {
                w.on_resize(sh);
            }
            w.window.request_redraw();
        }
        if old.gaps != sh.cfg.gaps || old.tiling != sh.cfg.tiling {
            self.relayout_all();
        }
    }

    /// Drop windows whose close animation finished and re-tile what's left.
    fn reap(&mut self) {
        while let Some(i) = self.wins.iter().position(|w| w.closed) {
            let m = self.wins[i].window.current_monitor();
            self.wins.remove(i); // drops the session: the shell is terminated
            if let Some(m) = m {
                self.relayout(&m);
            }
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if !self.started {
            self.started = true;
            if !self.start_hidden {
                self.open_window(el);
            }
        }
    }

    fn window_event(&mut self, _: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let (Some(i), Some(sh)) = (self.find(id), self.sh.as_mut()) else { return };
        let w = &mut self.wins[i];
        match event {
            WindowEvent::CloseRequested => self.close(i),
            WindowEvent::Resized(size) => w.on_surface_resized(sh, size.width, size.height),
            WindowEvent::ScaleFactorChanged { .. } => w.on_resize(sh),
            WindowEvent::Focused(focused) => {
                w.on_focus(focused);
                if focused {
                    self.last_focus = Some(id);
                    self.reload_config();
                } else {
                    self.save_placement(i);
                }
            }
            WindowEvent::ModifiersChanged(m) => w.mods = m.state(),
            WindowEvent::KeyboardInput { event, .. } => {
                if w.on_key(event, &sh.cfg) {
                    self.close(i);
                }
            }
            WindowEvent::CursorMoved { position, .. } => w.on_cursor_moved(position, sh),
            WindowEvent::MouseInput { state, button, .. } => w.on_mouse(state, button, sh),
            WindowEvent::MouseWheel { delta, .. } => w.on_wheel(delta, sh),
            WindowEvent::CursorLeft { .. } => w.set_grab(false),
            WindowEvent::RedrawRequested => w.draw(sh),
            _ => {}
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Toggle => self.toggle(el),
            UserEvent::NewWindow => self.open_window(el),
            UserEvent::Focus(id) => {
                if let Some(i) = self.find(id) {
                    self.wins[i].window.set_visible(true);
                    self.wins[i].window.focus_window();
                }
            }
            UserEvent::Quit => {
                win::remove_tray_icon();
                el.exit(); // dropping the sessions ends their shells
            }
            UserEvent::CloseFocused | UserEvent::HideFocused => {
                let hide = matches!(event, UserEvent::HideFocused);
                if let Some(i) = self.wins.iter().position(|w| !w.is_closing() && w.window.has_focus()) {
                    if hide {
                        self.wins[i].window.set_visible(false);
                    } else {
                        self.close(i);
                    }
                }
            }
            UserEvent::Accent => {
                if let Some(sh) = &mut self.sh
                    && sh.cfg.accent().is_none()
                {
                    sh.set_theme(Self::theme(&sh.cfg));
                    self.wins.iter().for_each(|w| w.window.request_redraw());
                }
            }
            UserEvent::Term(id, ev) => {
                if matches!(ev, TermEvent::Wakeup) {
                    self.reload_config();
                }
                let (Some(i), Some(sh)) = (self.find(id), self.sh.as_mut()) else { return };
                // `exit` in the shell closes its window.
                if self.wins[i].on_term_event(ev, sh) {
                    self.close(i);
                }
            }
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        self.reap();
        if self.admin && self.started && self.wins.is_empty() {
            el.exit();
            return;
        }
        let live = self.wins.iter().filter(|w| !w.is_closing()).map(|w| (w.window.id(), w.window.title()));
        *win::TERMINALS.lock().unwrap() = live.collect();
        // Animation frames are drawn here rather than via request_redraw: winit dispatches only one
        // RedrawRequested per loop round, so a window that always wants another frame (blinking
        // cursor) would starve the rest, e.g. tiled windows wouldn't glide until clicked.
        if let Some(sh) = self.sh.as_mut() {
            for w in self.wins.iter_mut().filter(|w| w.wants_frame) {
                w.draw(sh);
            }
        }
        if self.cfg_pending {
            self.reload_config();
        }
        el.set_control_flow(if self.wins.iter().any(|w| w.wants_frame) {
            ControlFlow::Poll // paced by the Fifo present
        } else if self.cfg_pending {
            ControlFlow::WaitUntil(self.cfg_checked + CONFIG_POLL)
        } else {
            ControlFlow::Wait
        });
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--install") {
        if let Err(e) = win::install_autostart() {
            eprintln!("kterm: {e}");
        }
        return;
    }
    if args.iter().any(|a| a == "--update") {
        if let Err(e) = win::self_update() {
            eprintln!("kterm: {e}");
        }
        return;
    }
    // `--admin`: the elevated terminal the shell's `admin` command opens. A separate process that
    // skips the single-instance handoff, tray and hotkey, and exits with its last window.
    let admin = args.iter().any(|a| a == "--admin");
    if !admin && win::signal_existing_instance() {
        return;
    }
    // Release builds have no console: leave panics somewhere findable.
    std::panic::set_hook(Box::new(|info| {
        let trace = std::backtrace::Backtrace::force_capture();
        let _ = std::fs::write(win::data_dir().join("crash.log"), format!("{info}\n{trace}"));
    }));

    fonts::ensure_installed();

    let event_loop = EventLoop::<UserEvent>::with_user_event().build().expect("event loop");
    let proxy = event_loop.create_proxy();
    win::set_proxy(proxy.clone());
    win::spawn_hotkey_thread(admin);
    if !admin {
        win::spawn_tray_thread();
    }
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App {
        proxy,
        sh: None,
        wins: Vec::new(),
        placements: win::load_placements(),
        last_focus: None,
        cfg_mtime: None,
        cfg_checked: Instant::now() - CONFIG_POLL,
        cfg_pending: false,
        start_hidden: args.iter().any(|a| a == "--hidden"),
        started: false,
        admin,
        _accent_watch: win::watch_accent(),
    };
    event_loop.run_app(&mut app).expect("event loop");
}

#[cfg(test)]
mod tests {
    use super::dwindle;

    #[test]
    fn dwindle_splits_like_hyprland() {
        let area = (0, 0, 1000, 600);
        assert_eq!(dwindle(area, 1, 10), vec![(10, 10, 980, 580)]);
        // Two: left / right halves with a gap between.
        assert_eq!(dwindle(area, 2, 10), vec![(10, 10, 485, 580), (505, 10, 485, 580)]);
        // Three: left, then the right half splits top / bottom.
        let three = dwindle(area, 3, 10);
        assert_eq!(three[0], (10, 10, 485, 580));
        assert_eq!(three[1], (505, 10, 485, 285));
        assert_eq!(three[2], (505, 305, 485, 285));
    }
}
