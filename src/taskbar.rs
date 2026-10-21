//! Finding the taskbar, embedding the widget into it and keeping it positioned.
//! Mirrors TrafficMonitor's CWin11TaskbarDlg / CClassicalTaskbarDlg logic.

use crate::render::scale;
use crate::sys::{wide, win11_taskbar_centered, windows_build};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    Win11,
    Classic,
}

pub struct Taskbar {
    pub tray: HWND,
    pub mode: Mode,
    rebar: HWND,
    min: HWND,
    notify: HWND,
    start: HWND,
    // classic: full length of the task list before we shrank it, and what we set it to
    classic_orig: i32,
    classic_last_set: Option<i32>,
    classic_our: i32,
    last_rect: RECT,
}

fn find_child(parent: HWND, class: &str) -> HWND {
    let c = wide(class);
    unsafe { FindWindowExW(parent, std::ptr::null_mut(), c.as_ptr(), std::ptr::null()) }
}

pub fn rect_of(h: HWND) -> RECT {
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    if !h.is_null() {
        unsafe { GetWindowRect(h, &mut r) };
    }
    r
}

fn w(r: &RECT) -> i32 {
    r.right - r.left
}
fn h(r: &RECT) -> i32 {
    r.bottom - r.top
}

impl Taskbar {
    pub fn find(mode_cfg: &str) -> Option<Taskbar> {
        let cls = wide("Shell_TrayWnd");
        let tray = unsafe { FindWindowW(cls.as_ptr(), std::ptr::null()) };
        if tray.is_null() {
            return None;
        }
        let rebar = find_child(tray, "ReBarWindow32");
        let min = if rebar.is_null() { std::ptr::null_mut() } else { find_child(rebar, "MSTaskSwWClass") };
        let notify = find_child(tray, "TrayNotifyWnd");
        let start = find_child(tray, "Start");

        let classic_possible = !rebar.is_null()
            && !min.is_null()
            && unsafe { IsWindowVisible(rebar) } != 0
            && w(&rect_of(rebar)) > 0;
        let mode = match mode_cfg.to_ascii_lowercase().as_str() {
            "classic" if classic_possible => Mode::Classic,
            "win11" => Mode::Win11,
            _ => {
                // Native Win11 taskbar is a XAML island; ExplorerPatcher's classic one isn't.
                let xaml = !find_child(tray, "Windows.UI.Composition.DesktopWindowContentBridge").is_null();
                let win11 = windows_build() >= 22000 && xaml;
                if classic_possible && !win11 {
                    Mode::Classic
                } else {
                    Mode::Win11
                }
            }
        };
        Some(Taskbar {
            tray,
            mode,
            rebar,
            min,
            notify,
            start,
            classic_orig: 0,
            classic_last_set: None,
            classic_our: 0,
            last_rect: RECT { left: 0, top: 0, right: 0, bottom: 0 },
        })
    }

    pub fn alive(&self) -> bool {
        unsafe { IsWindow(self.tray) != 0 }
    }

    pub fn parent(&self) -> HWND {
        match self.mode {
            Mode::Win11 => self.tray,
            Mode::Classic => self.rebar,
        }
    }

    /// Window used to pick the DPI.
    pub fn dpi_window(&self) -> HWND {
        self.tray
    }

    pub fn is_vertical(&self) -> bool {
        let r = rect_of(self.tray);
        h(&r) > w(&r)
    }

    /// Height available for the widget (horizontal taskbars).
    pub fn widget_height(&self) -> i32 {
        match self.mode {
            Mode::Win11 => {
                let s = rect_of(self.start);
                if h(&s) > 0 { h(&s) } else { h(&rect_of(self.tray)) }
            }
            Mode::Classic => h(&rect_of(self.rebar)),
        }
    }

    /// Positions `widget` (size `ww` x `wh`). `force` re-applies even if nothing moved.
    pub fn place(&mut self, widget: HWND, ww: i32, wh: i32, cfg: &crate::config::Layout, dpi: u32, force: bool, embedded: bool) {
        let mut r = match self.mode {
            Mode::Win11 => self.place_win11(ww, wh, cfg, dpi),
            Mode::Classic => match self.place_classic(ww, wh, cfg, dpi, force) {
                Some(r) => r,
                None => return,
            },
        };
        if !embedded {
            // SetParent failed: float above the taskbar at the same spot, in screen coords.
            let p = rect_of(self.parent());
            r.left += p.left;
            r.right += p.left;
            r.top += p.top;
            r.bottom += p.top;
        }
        if force || r.left != self.last_rect.left || r.top != self.last_rect.top || w(&r) != w(&self.last_rect) || h(&r) != h(&self.last_rect) {
            unsafe {
                SetWindowPos(widget, if embedded { HWND_TOP } else { HWND_TOPMOST }, r.left, r.top, w(&r), h(&r), SWP_NOACTIVATE | SWP_SHOWWINDOW);
            }
            self.last_rect = r;
        }
    }

    fn place_win11(&self, ww: i32, wh: i32, cfg: &crate::config::Layout, dpi: u32) -> RECT {
        let tr = rect_of(self.tray);
        let nr = rect_of(self.notify);
        let sr = rect_of(self.start);
        let left_mode = cfg.position.eq_ignore_ascii_case("left") && win11_taskbar_centered();
        let mut x = if left_mode {
            2
        } else {
            let mut notify_x = nr.left - tr.left;
            if self.notify.is_null() || w(&nr) <= 0 || notify_x <= 0 {
                notify_x = w(&tr) - scale(cfg.win11_right_space, dpi);
            }
            notify_x - ww + 2
        };
        x += scale(cfg.offset_x, dpi);
        let start_h = if h(&sr) > 0 { h(&sr) } else { h(&tr) };
        let y = (start_h - wh) / 2 + (h(&tr) - start_h);
        RECT { left: x, top: y, right: x + ww, bottom: y + wh }
    }

    fn place_classic(&mut self, ww: i32, wh: i32, cfg: &crate::config::Layout, dpi: u32, force: bool) -> Option<RECT> {
        let br = rect_of(self.rebar);
        let mut mr = rect_of(self.min);
        // relative to the rebar
        mr.left -= br.left;
        mr.right -= br.left;
        mr.top -= br.top;
        mr.bottom -= br.top;
        let vertical = h(&br) > w(&br);
        let cur_len = if vertical { h(&mr) } else { w(&mr) };
        let our_len = if vertical { wh } else { ww };

        if self.classic_last_set != Some(cur_len) {
            // Explorer re-laid out the task list: it owns the full length again.
            self.classic_orig = cur_len;
        } else if !force && self.classic_our == our_len {
            return None;
        }
        let new_len = (self.classic_orig - our_len).max(0);
        unsafe {
            if vertical {
                MoveWindow(self.min, mr.left, mr.top, w(&mr), new_len, 1);
            } else {
                MoveWindow(self.min, mr.left, mr.top, new_len, h(&mr), 1);
            }
        }
        self.classic_last_set = Some(new_len);
        self.classic_our = our_len;
        let off_x = scale(cfg.offset_x, dpi);
        Some(if vertical {
            let y = mr.top + new_len;
            RECT { left: off_x, top: y, right: off_x + w(&br), bottom: y + wh }
        } else {
            let x = mr.left + new_len + off_x;
            let y = (h(&br) - wh) / 2;
            RECT { left: x, top: y, right: x + ww, bottom: y + wh }
        })
    }

    /// Give the task list its space back (classic mode).
    pub fn restore(&mut self) {
        if self.mode == Mode::Classic && self.classic_last_set.is_some() && unsafe { IsWindow(self.min) } != 0 {
            let br = rect_of(self.rebar);
            let mut mr = rect_of(self.min);
            mr.left -= br.left;
            mr.top -= br.top;
            let vertical = h(&br) > w(&br);
            unsafe {
                if vertical {
                    MoveWindow(self.min, mr.left, mr.top, w(&br), self.classic_orig, 1);
                } else {
                    MoveWindow(self.min, mr.left, mr.top, self.classic_orig, h(&br), 1);
                }
            }
            self.classic_last_set = None;
        }
    }
}
