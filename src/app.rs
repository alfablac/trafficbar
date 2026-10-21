//! Window management, timers, menu, tray icon and the main loop.

use crate::config::{self, Config, Tier};
use crate::format::{effective_tiers, format_speed, widest_samples};
use crate::render::{self, Cell, Frame, Renderer};
use crate::stats::{self, CpuSampler, NetSampler};
use crate::sys::{self, wide, APP_NAME};
use crate::settings;
use crate::taskbar::{Mode, Taskbar};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::time::{Instant, SystemTime};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const T_STATS: usize = 1;
const T_WATCH: usize = 2;
const WM_APP_TRAY: u32 = WM_APP + 1;
const WM_APP_MENU: u32 = WM_APP + 2;
const WM_APP_DBLCLK: u32 = WM_APP + 3;
const WM_APP_SETTINGS: u32 = WM_APP + 4;
pub const WM_APP_RELOAD: u32 = WM_APP + 5;

const CMD_EDIT: usize = 100;
const CMD_RELOAD: usize = 101;
const CMD_FOLDER: usize = 102;
const CMD_AUTOSTART: usize = 103;
const CMD_EXIT: usize = 104;
const CMD_BITS: usize = 105;
const CMD_SHORT: usize = 106;
const CMD_TASKMGR: usize = 107;
const CMD_SETTINGS: usize = 108;
const CMD_UNIT_BASE: usize = 200; // + index into UNITS
const CMD_IFACE_AUTO: usize = 300;
const CMD_IFACE_BASE: usize = 301; // + adapter index
const UNITS: [&str; 5] = ["auto", "B", "KB", "MB", "GB"];

struct App {
    hinst: HINSTANCE,
    host: HWND,
    widget: HWND,
    embedded: bool,
    taskbar: Option<Taskbar>,
    cfg: Config,
    cfg_stamp: Option<SystemTime>,
    tiers: Vec<Tier>,
    samples: Vec<String>,
    net: NetSampler,
    cpu: CpuSampler,
    mem: f64,
    renderer: Renderer,
    frame: Option<Frame>,
    use_alpha: bool,
    light: bool,
    last_theme_check: Instant,
    tray_added: bool,
    taskbar_created: u32,
    icon: HICON,
    menu_adapters: Vec<String>,
    attach_failures: u32,
}

static APP: AtomicPtr<App> = AtomicPtr::new(std::ptr::null_mut());

/// Single-threaded UI: every call happens on the window thread. Handlers never keep
/// this reference across calls that pump messages (menus), so re-entrancy is safe.
fn app() -> Option<&'static mut App> {
    let p = APP.load(Ordering::Relaxed);
    if p.is_null() { None } else { Some(unsafe { &mut *p }) }
}

pub fn run(open_settings_first: bool) {
    unsafe {
        let mutex_name = wide("Local\\TrafficBar-rs-single-instance");
        CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            if open_settings_first {
                let cls = wide("TrafficBarHost");
                let h = FindWindowW(cls.as_ptr(), std::ptr::null());
                if !h.is_null() {
                    PostMessageW(h, WM_APP_SETTINGS, 0, 0);
                }
            }
            return;
        }
        let hinst = GetModuleHandleW(std::ptr::null()) as HINSTANCE;
        let icon = LoadIconW(hinst, 1 as _);

        let host_cls = wide("TrafficBarHost");
        let widget_cls = wide("TrafficBarWidget");
        let mut wc: WNDCLASSEXW = std::mem::zeroed();
        wc.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
        wc.lpfnWndProc = Some(host_proc);
        wc.hInstance = hinst;
        wc.hIcon = icon;
        wc.lpszClassName = host_cls.as_ptr();
        RegisterClassExW(&wc);
        wc.lpfnWndProc = Some(widget_proc);
        wc.style = CS_DBLCLKS;
        wc.hCursor = LoadCursorW(std::ptr::null_mut(), IDC_ARROW);
        wc.lpszClassName = widget_cls.as_ptr();
        RegisterClassExW(&wc);

        let title = wide(APP_NAME);
        let host = CreateWindowExW(WS_EX_TOOLWINDOW, host_cls.as_ptr(), title.as_ptr(), WS_POPUP,
            0, 0, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), hinst, std::ptr::null());

        let tc = wide("TaskbarCreated");
        let boxed = Box::new(App {
            hinst,
            host,
            widget: std::ptr::null_mut(),
            embedded: false,
            taskbar: None,
            cfg: Config::default(),
            cfg_stamp: None,
            tiers: Vec::new(),
            samples: Vec::new(),
            net: NetSampler::new(),
            cpu: CpuSampler::new(),
            mem: 0.0,
            renderer: Renderer::new(),
            frame: None,
            use_alpha: true,
            light: sys::taskbar_is_light(),
            last_theme_check: Instant::now(),
            tray_added: false,
            taskbar_created: RegisterWindowMessageW(tc.as_ptr()),
            icon,
            menu_adapters: Vec::new(),
            attach_failures: 0,
        });
        APP.store(Box::into_raw(boxed), Ordering::Relaxed);

        let a = app().unwrap();
        a.load_config(true);
        a.net.sample(&a.cfg.general.interface, a.cfg.general.include_virtual, 0.0);
        a.cpu.sample();
        a.attach();
        if open_settings_first {
            a.open_settings();
        }
        SetTimer(host, T_STATS, a.cfg.general.update_interval_ms, None);
        SetTimer(host, T_WATCH, 500, None);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            if settings::is_open() && IsDialogMessageW(settings::window(), &msg) != 0 {
                continue; // Tab / Enter navigation inside the settings window
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

impl App {
    // ---------------------------------------------------------------- config
    fn load_config(&mut self, first: bool) {
        let path = sys::config_path();
        if !path.exists() {
            let _ = std::fs::write(&path, config::DEFAULT_TOML.replace('\n', "\r\n"));
        }
        self.cfg_stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let text = text.trim_start_matches('\u{feff}'); // Notepad BOM
        match config::parse(text) {
            Ok(c) => self.cfg = c,
            Err(e) => {
                sys::log(&format!("config error: {e}"));
                if first {
                    self.cfg = Config::default();
                }
                self.notify_error(&format!("Config error, keeping previous settings:\n{}", e.lines().next().unwrap_or("")));
            }
        }
        self.tiers = effective_tiers(&self.cfg.format);
        self.samples = widest_samples(&self.cfg.format, &self.tiers);
        if self.cfg.general.start_with_windows != sys::autostart_enabled() {
            sys::set_autostart(self.cfg.general.start_with_windows);
        }
        self.sync_tray();
        if !first {
            unsafe { SetTimer(self.host, T_STATS, self.cfg.general.update_interval_ms, None) };
        }
    }

    fn config_changed_on_disk(&self) -> bool {
        let now = std::fs::metadata(sys::config_path()).and_then(|m| m.modified()).ok();
        now.is_some() && now != self.cfg_stamp
    }

    fn edit_config_value(&mut self, f: impl FnOnce(&str) -> String) {
        let path = sys::config_path();
        let text = std::fs::read_to_string(&path).unwrap_or_else(|_| config::DEFAULT_TOML.to_string());
        let new = f(text.trim_start_matches('\u{feff}'));
        let _ = std::fs::write(&path, new);
        self.reload();
    }

    fn reload(&mut self) {
        self.load_config(false);
        self.net.reset();
        self.detach();
        self.attach();
        settings::on_reload(&self.cfg);
    }

    fn open_settings(&self) {
        settings::open(self.host, self.hinst, &self.cfg);
    }

    // ---------------------------------------------------------------- taskbar
    fn attach(&mut self) {
        let Some(mut tb) = Taskbar::find(&self.cfg.general.taskbar_mode) else {
            return;
        };
        unsafe {
            let cls = wide("TrafficBarWidget");
            let title = wide(APP_NAME);
            let widget = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE,
                cls.as_ptr(), title.as_ptr(), WS_POPUP, 0, 0, 1, 1,
                std::ptr::null_mut(), std::ptr::null_mut(), self.hinst, std::ptr::null());
            if widget.is_null() {
                return;
            }
            self.embedded = !SetParent(widget, tb.parent()).is_null();
            if !self.embedded {
                sys::log("SetParent failed; floating above the taskbar instead");
            }
            let mode = self.cfg.general.render_mode.to_ascii_lowercase();
            self.use_alpha = mode != "colorkey";
            if !self.use_alpha {
                render::set_colorkey(widget, self.colorkey());
            }
            sys::log(&format!("attached: mode={:?} embedded={} alpha={}", tb.mode, self.embedded, self.use_alpha));
            tb.restore(); // no-op unless classic state is stale
            self.taskbar = Some(tb);
            self.widget = widget;
        }
        self.refresh(true);
    }

    fn detach(&mut self) {
        if let Some(tb) = self.taskbar.as_mut() {
            if tb.alive() {
                tb.restore();
            }
        }
        if !self.widget.is_null() {
            unsafe { DestroyWindow(self.widget) };
        }
        self.widget = std::ptr::null_mut();
        self.taskbar = None;
        self.frame = None;
    }

    fn healthy(&self) -> bool {
        match &self.taskbar {
            Some(tb) => tb.alive() && unsafe { IsWindow(self.widget) } != 0,
            None => false,
        }
    }

    fn default_text_color(&self) -> (u8, u8, u8) {
        let auto = if self.light { (0, 0, 0) } else { (255, 255, 255) };
        render::resolve_color(&self.cfg.colors.text, auto)
    }

    fn colorkey(&self) -> COLORREF {
        let auto = if self.light { (0xF0, 0xF0, 0xF0) } else { (0x1F, 0x1F, 0x1F) };
        let (r, g, b) = render::resolve_color(&self.cfg.colors.colorkey, auto);
        sys::rgb(r, g, b)
    }

    fn build_cells(&self) -> Vec<Cell> {
        let text = self.default_text_color();
        let f = &self.cfg.format;
        self.cfg
            .items
            .iter()
            .map(|it| {
                let (value, samples) = match it.kind.to_ascii_lowercase().as_str() {
                    "down" | "download" => (format_speed(self.net.down, f, &self.tiers), self.samples.clone()),
                    "up" | "upload" => (format_speed(self.net.up, f, &self.tiers), self.samples.clone()),
                    "total" => (format_speed(self.net.down + self.net.up, f, &self.tiers), self.samples.clone()),
                    "cpu" => (format!("{:.0}%", self.cpu.usage), vec!["100%".into()]),
                    "memory" | "mem" | "ram" => (format!("{:.0}%", self.mem), vec!["100%".into()]),
                    _ => (String::new(), vec![]),
                };
                let lim = it.graph_limit();
                let fill = if lim > 0.0 {
                    Some(match it.kind.to_ascii_lowercase().as_str() {
                        "cpu" => self.cpu.usage / 100.0,
                        "memory" | "mem" | "ram" => self.mem / 100.0,
                        "up" | "upload" => mbit(self.net.up) / lim,
                        "total" => mbit(self.net.down + self.net.up) / lim,
                        _ => mbit(self.net.down) / lim,
                    })
                } else {
                    None
                };
                Cell {
                    label: it.label.clone(),
                    value,
                    samples,
                    label_color: render::resolve_color(&it.label_color, text),
                    value_color: render::resolve_color(&it.value_color, text),
                    fill,
                }
            })
            .collect()
    }

    fn refresh(&mut self, force: bool) {
        if !self.healthy() {
            return;
        }
        let tb = self.taskbar.as_ref().unwrap();
        let dpi = match unsafe { GetDpiForWindow(tb.dpi_window()) } {
            0 => 96,
            d => d,
        };
        self.renderer.ensure_font(&self.cfg, dpi, self.use_alpha);
        let cells = self.build_cells();
        let vertical_classic = tb.mode == Mode::Classic && tb.is_vertical();
        let height = if vertical_classic {
            let n = cells.len().min(self.cfg.layout.rows as usize).max(1) as i32;
            n * (self.renderer.line_height() + render::scale(self.cfg.layout.line_spacing, dpi)) + render::scale(6, dpi)
        } else {
            tb.widget_height()
        };
        let frame = self.renderer.layout(&self.cfg, &cells, dpi, height);
        let (w, h) = (frame.w, frame.h);
        let (widget, embedded) = (self.widget, self.embedded);
        let layout = self.cfg.layout.clone();
        self.taskbar.as_mut().unwrap().place(widget, w, h, &layout, dpi, force, embedded);

        if self.use_alpha {
            if !self.renderer.present_alpha(widget, &frame, &self.cfg) {
                let err = unsafe { GetLastError() };
                sys::log(&format!("UpdateLayeredWindow failed ({err}); switching to colorkey rendering"));
                self.use_alpha = false;
                render::set_colorkey(widget, self.colorkey());
                self.renderer.ensure_font(&self.cfg, dpi, false);
            }
        }
        if !self.use_alpha {
            unsafe {
                let dc = GetDC(widget);
                self.renderer.paint_colorkey(dc, &frame, self.colorkey());
                ReleaseDC(widget, dc);
            }
        }
        self.frame = Some(frame);
        self.update_tray_tip();
    }

    fn on_stats_tick(&mut self) {
        let g = &self.cfg.general;
        self.net.sample(&g.interface, g.include_virtual, g.smoothing);
        self.cpu.sample();
        self.mem = stats::memory_usage();
        self.refresh(false);
    }

    fn on_watch_tick(&mut self) {
        if self.config_changed_on_disk() {
            self.reload();
            return;
        }
        if self.last_theme_check.elapsed().as_secs() >= 2 {
            self.last_theme_check = Instant::now();
            let light = sys::taskbar_is_light();
            if light != self.light {
                self.light = light;
                if !self.use_alpha {
                    render::set_colorkey(self.widget, self.colorkey());
                }
                self.refresh(true);
            }
        }
        if !self.healthy() {
            // Explorer restarted or the taskbar isn't up yet.
            self.detach();
            self.attach();
            if !self.healthy() {
                self.attach_failures += 1;
            } else {
                self.attach_failures = 0;
            }
        } else {
            // keep up with tray width changes etc.
            self.refresh(false);
        }
    }

    // ---------------------------------------------------------------- tray
    fn tray_data(&self) -> NOTIFYICONDATAW {
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = self.host;
        nid.uID = 1;
        nid
    }

    fn sync_tray(&mut self) {
        let want = self.cfg.general.tray_icon;
        if want && !self.tray_added {
            let mut nid = self.tray_data();
            nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
            nid.uCallbackMessage = WM_APP_TRAY;
            nid.hIcon = self.icon;
            copy_w(&mut nid.szTip, APP_NAME);
            self.tray_added = unsafe { Shell_NotifyIconW(NIM_ADD, &nid) } != 0;
        } else if !want && self.tray_added {
            let nid = self.tray_data();
            unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
            self.tray_added = false;
        }
    }

    fn update_tray_tip(&self) {
        if !self.tray_added {
            return;
        }
        let f = &self.cfg.format;
        let tip = format!(
            "{APP_NAME}\n↓ {}   ↑ {}\nSession: ↓ {}  ↑ {}",
            format_speed(self.net.down, f, &self.tiers),
            format_speed(self.net.up, f, &self.tiers),
            human_size(self.net.total_rx),
            human_size(self.net.total_tx),
        );
        let mut nid = self.tray_data();
        nid.uFlags = NIF_TIP;
        copy_w(&mut nid.szTip, &tip);
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
    }

    fn notify_error(&self, msg: &str) {
        if !self.tray_added {
            return;
        }
        let mut nid = self.tray_data();
        nid.uFlags = NIF_INFO;
        copy_w(&mut nid.szInfoTitle, APP_NAME);
        copy_w(&mut nid.szInfo, msg);
        nid.dwInfoFlags = NIIF_WARNING;
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
    }

    fn quit(&mut self) {
        self.detach();
        if self.tray_added {
            let nid = self.tray_data();
            unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
            self.tray_added = false;
        }
        unsafe { PostQuitMessage(0) };
    }

    fn double_click(&mut self) {
        match self.cfg.general.double_click_action.to_ascii_lowercase().as_str() {
            "settings" => self.open_settings(),
            "config" => open_config(),
            "none" => {}
            _ => shell_open("taskmgr.exe", ""),
        }
    }
}

fn mbit(bytes_per_s: f64) -> f64 {
    bytes_per_s * 8.0 / 1_000_000.0
}

fn copy_w(dst: &mut [u16], s: &str) {
    let w: Vec<u16> = s.encode_utf16().take(dst.len() - 1).collect();
    dst[..w.len()].copy_from_slice(&w);
    dst[w.len()] = 0;
}

fn human_size(b: u64) -> String {
    let b = b as f64;
    if b < 1024.0 * 1024.0 {
        format!("{:.1} KB", b / 1024.0)
    } else if b < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} MB", b / 1048576.0)
    } else {
        format!("{:.2} GB", b / 1073741824.0)
    }
}

fn shell_open(file: &str, params: &str) {
    let (op, f, p) = (wide("open"), wide(file), wide(params));
    unsafe {
        ShellExecuteW(std::ptr::null_mut(), op.as_ptr(), f.as_ptr(),
            if params.is_empty() { std::ptr::null() } else { p.as_ptr() }, std::ptr::null(), SW_SHOWNORMAL);
    }
}

pub fn open_config() {
    let path = sys::config_path();
    shell_open("notepad.exe", &format!("\"{}\"", path.display()));
}

fn append(menu: HMENU, flags: u32, id: usize, text: &str) {
    let t = wide(text);
    unsafe { AppendMenuW(menu, flags, id, t.as_ptr()) };
}

/// Builds and shows the context menu; handles the chosen command afterwards.
fn show_menu() {
    let Some(a) = app() else { return };
    let cfg = a.cfg.clone();
    let adapters = stats::list_adapters();
    a.menu_adapters = adapters.iter().map(|x| x.alias.clone()).collect();
    let host = a.host;
    let autostart = sys::autostart_enabled();

    let cmd = unsafe {
        let menu = CreatePopupMenu();
        append(menu, MF_STRING, CMD_SETTINGS, "Settings…");
        append(menu, MF_STRING, CMD_EDIT, "Open config file (advanced)");
        append(menu, MF_STRING, CMD_RELOAD, "Reload settings");
        append(menu, MF_STRING, CMD_FOLDER, "Open program folder");
        append(menu, MF_SEPARATOR, 0, "");

        let ifm = CreatePopupMenu();
        let auto_checked = if cfg.general.interface.trim().is_empty() { MF_CHECKED } else { MF_UNCHECKED };
        append(ifm, MF_STRING | auto_checked, CMD_IFACE_AUTO, "All physical adapters (auto)");
        append(ifm, MF_SEPARATOR, 0, "");
        for (i, ad) in adapters.iter().enumerate() {
            let checked = if !cfg.general.interface.is_empty() && cfg.general.interface.eq_ignore_ascii_case(&ad.alias) {
                MF_CHECKED
            } else {
                MF_UNCHECKED
            };
            let label = if ad.hardware { ad.alias.clone() } else { format!("{} (virtual)", ad.alias) };
            append(ifm, MF_STRING | checked, CMD_IFACE_BASE + i, &label);
        }
        append(menu, MF_POPUP, ifm as usize, "Network adapter");

        let um = CreatePopupMenu();
        for (i, u) in UNITS.iter().enumerate() {
            let checked = if cfg.format.unit.eq_ignore_ascii_case(u) { MF_CHECKED } else { MF_UNCHECKED };
            let label = if i == 0 { "Automatic (B/s → KB/s → MB/s)".to_string() } else { format!("Always {u}/s") };
            append(um, MF_STRING | checked, CMD_UNIT_BASE + i, &label);
        }
        append(um, MF_SEPARATOR, 0, "");
        append(um, MF_STRING | if cfg.format.bits { MF_CHECKED } else { 0 }, CMD_BITS, "Use bits (Kb/s, Mb/s)");
        append(um, MF_STRING | if cfg.format.short { MF_CHECKED } else { 0 }, CMD_SHORT, "Short mode (8.9 K)");
        append(menu, MF_POPUP, um as usize, "Speed unit");

        append(menu, MF_SEPARATOR, 0, "");
        append(menu, MF_STRING | if autostart { MF_CHECKED } else { 0 }, CMD_AUTOSTART, "Start with Windows");
        append(menu, MF_STRING, CMD_TASKMGR, "Task Manager");
        append(menu, MF_SEPARATOR, 0, "");
        append(menu, MF_STRING, CMD_EXIT, "Exit");

        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        SetForegroundWindow(host);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, 0, host, std::ptr::null());
        PostMessageW(host, WM_NULL, 0, 0);
        DestroyMenu(menu); // destroys submenus too
        cmd as usize
    };
    if cmd == 0 {
        return;
    }
    let Some(a) = app() else { return };
    match cmd {
        CMD_SETTINGS => a.open_settings(),
        CMD_EDIT => open_config(),
        CMD_RELOAD => a.reload(),
        CMD_FOLDER => {
            let p = sys::exe_path();
            shell_open("explorer.exe", &format!("/select,\"{}\"", p.display()));
        }
        CMD_TASKMGR => shell_open("taskmgr.exe", ""),
        CMD_AUTOSTART => {
            let v = !autostart;
            a.edit_config_value(|t| config::set_bool_in_text(t, "general", "start_with_windows", v));
            sys::set_autostart(v);
        }
        CMD_BITS => {
            let v = !cfg.format.bits;
            a.edit_config_value(|t| config::set_bool_in_text(t, "format", "bits", v));
        }
        CMD_SHORT => {
            let v = !cfg.format.short;
            a.edit_config_value(|t| config::set_bool_in_text(t, "format", "short", v));
        }
        CMD_IFACE_AUTO => a.edit_config_value(|t| config::set_string_in_text(t, "general", "interface", "")),
        CMD_EXIT => a.quit(),
        c if (CMD_UNIT_BASE..CMD_UNIT_BASE + UNITS.len()).contains(&c) => {
            let u = UNITS[c - CMD_UNIT_BASE];
            a.edit_config_value(|t| config::set_string_in_text(t, "format", "unit", u));
        }
        c if c >= CMD_IFACE_BASE => {
            if let Some(name) = a.menu_adapters.get(c - CMD_IFACE_BASE).cloned() {
                a.edit_config_value(|t| config::set_string_in_text(t, "general", "interface", &name));
            }
        }
        _ => {}
    }
}

unsafe extern "system" fn host_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            if let Some(a) = app() {
                match wp {
                    T_STATS => a.on_stats_tick(),
                    T_WATCH => a.on_watch_tick(),
                    _ => {}
                }
            }
            0
        }
        WM_APP_TRAY => {
            match lp as u32 {
                WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(),
                WM_LBUTTONDBLCLK => {
                    if let Some(a) = app() {
                        a.double_click()
                    }
                }
                _ => {}
            }
            0
        }
        WM_APP_SETTINGS => {
            if let Some(a) = app() {
                a.open_settings();
            }
            0
        }
        WM_APP_RELOAD => {
            if let Some(a) = app() {
                a.reload();
            }
            0
        }
        WM_APP_MENU => {
            show_menu();
            0
        }
        WM_APP_DBLCLK => {
            if let Some(a) = app() {
                a.double_click();
            }
            0
        }
        WM_ENDSESSION if wp != 0 => {
            if let Some(a) = app() {
                a.quit();
            }
            0
        }
        _ => {
            if let Some(a) = app() {
                if msg == a.taskbar_created && msg != 0 {
                    // Explorer restarted: re-add tray icon and re-embed.
                    a.tray_added = false;
                    a.sync_tray();
                    a.taskbar = None; // old one is gone, nothing to restore
                    if !a.widget.is_null() {
                        unsafe { DestroyWindow(a.widget) };
                        a.widget = std::ptr::null_mut();
                    }
                    a.attach();
                    return 0;
                }
            }
            unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
        }
    }
}

unsafe extern "system" fn widget_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => unsafe {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            if let Some(a) = app() {
                if !a.use_alpha && a.widget == hwnd {
                    if let Some(f) = &a.frame {
                        a.renderer.paint_colorkey(dc, f, a.colorkey());
                    }
                }
            }
            EndPaint(hwnd, &ps);
            0
        },
        WM_RBUTTONUP => {
            if let Some(a) = app() {
                unsafe { PostMessageW(a.host, WM_APP_MENU, 0, 0) };
            }
            0
        }
        WM_LBUTTONDBLCLK => {
            if let Some(a) = app() {
                unsafe { PostMessageW(a.host, WM_APP_DBLCLK, 0, 0) };
            }
            0
        }
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}
