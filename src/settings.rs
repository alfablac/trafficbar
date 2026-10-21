#![allow(unsafe_op_in_unsafe_fn)]
//! Settings window: a native Win32 form with tabs. It reads and writes TrafficBar.toml,
//! so the widget and the file always agree. Checkboxes and dropdowns apply at once,
//! text boxes apply when you leave them, and the widget reloads the file after each save.

use crate::app::WM_APP_RELOAD;
use crate::config::{self, parse_color, Config, Item};
use crate::render::scale;
use crate::stats::{self, Adapter};
use crate::sys::{self, wide};
use std::sync::atomic::{AtomicUsize, Ordering};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "TrafficBarSettings";
const ID_APPLY: usize = 900;
const ID_CLOSE: usize = 901;
const ID_RESET: usize = 902;
const ID_OPENFILE: usize = 903;
const ID_FIELD: usize = 1000;
const ID_ITEM: usize = 2000;
const ITEM_TAB: usize = 5;
const TABS: [&str; 6] = ["General", "Font", "Layout", "Colors", "Format", "Items"];

struct ItemRow {
    kind: &'static str,
    title: &'static str,
    label: &'static str,
    color: &'static str,
}

const ITEM_ROWS: [ItemRow; 5] = [
    ItemRow { kind: "down", title: "Download", label: "▼ ", color: "#E5534B" },
    ItemRow { kind: "up", title: "Upload", label: "▲ ", color: "#57AB5A" },
    ItemRow { kind: "total", title: "Total", label: "Σ ", color: "auto" },
    ItemRow { kind: "cpu", title: "CPU usage", label: "CPU ", color: "auto" },
    ItemRow { kind: "memory", title: "Memory usage", label: "MEM ", color: "auto" },
];

enum Kind {
    Text,
    Num,
    Bool,
    Choice(Vec<(String, String)>), // (stored value, shown label)
}

struct Field {
    tab: usize,
    label: &'static str,
    kind: Kind,
    get: fn(&Config) -> String,
    set: fn(&mut Config, &str) -> Result<(), String>,
}

struct State {
    hwnd: HWND,
    host: HWND,
    hinst: HINSTANCE,
    font: HFONT,
    fields: Vec<Field>,
    ctrls: Vec<HWND>,
    item_ctrls: Vec<[HWND; 5]>,
    page: Vec<(usize, HWND)>,
    tab: HWND,
    suppress: bool,
}

static WINDOW: AtomicUsize = AtomicUsize::new(0);

pub fn is_open() -> bool {
    WINDOW.load(Ordering::Relaxed) != 0
}

pub fn window() -> HWND {
    WINDOW.load(Ordering::Relaxed) as HWND
}

// ------------------------------------------------------------ value helpers
fn num<T: std::str::FromStr>(s: &str) -> Result<T, String> {
    s.trim().parse::<T>().map_err(|_| "must be a number".to_string())
}

fn boolv(s: &str) -> Result<bool, String> {
    Ok(s == "true")
}

fn bstr(v: bool) -> String {
    (if v { "true" } else { "false" }).to_string()
}

fn color(s: &str) -> Result<String, String> {
    let t = s.trim();
    if t.eq_ignore_ascii_case("auto") || parse_color(t).is_some() {
        Ok(t.to_string())
    } else {
        Err("must be \"auto\" or a color like #RRGGBB".to_string())
    }
}

fn choice(list: &[(&str, &str)]) -> Kind {
    Kind::Choice(list.iter().map(|(v, l)| (v.to_string(), l.to_string())).collect())
}

fn build_fields(adapters: &[Adapter], fonts: &[String]) -> Vec<Field> {
    let mut adapter_choices = vec![(String::new(), "All physical adapters (automatic)".to_string())];
    for a in adapters {
        let label = if a.hardware { a.alias.clone() } else { format!("{} (virtual)", a.alias) };
        adapter_choices.push((a.alias.clone(), label));
    }
    let font_choices: Vec<(String, String)> = fonts.iter().map(|f| (f.clone(), f.clone())).collect();

    vec![
        // ---- General
        Field { tab: 0, label: "Update interval (ms)", kind: Kind::Num,
            get: |c| c.general.update_interval_ms.to_string(),
            set: |c, s| { c.general.update_interval_ms = num(s)?; Ok(()) } },
        Field { tab: 0, label: "Smoothing (0 = off, max 0.95)", kind: Kind::Num,
            get: |c| c.general.smoothing.to_string(),
            set: |c, s| { c.general.smoothing = num(s)?; Ok(()) } },
        Field { tab: 0, label: "Network adapter", kind: Kind::Choice(adapter_choices),
            get: |c| c.general.interface.clone(),
            set: |c, s| { c.general.interface = s.to_string(); Ok(()) } },
        Field { tab: 0, label: "Include virtual adapters (auto only)", kind: Kind::Bool,
            get: |c| bstr(c.general.include_virtual),
            set: |c, s| { c.general.include_virtual = boolv(s)?; Ok(()) } },
        Field { tab: 0, label: "Start with Windows", kind: Kind::Bool,
            get: |c| bstr(c.general.start_with_windows),
            set: |c, s| { c.general.start_with_windows = boolv(s)?; Ok(()) } },
        Field { tab: 0, label: "Show tray icon", kind: Kind::Bool,
            get: |c| bstr(c.general.tray_icon),
            set: |c, s| { c.general.tray_icon = boolv(s)?; Ok(()) } },
        Field { tab: 0, label: "Double-click action", kind: choice(&[
                ("settings", "Open settings"), ("taskmgr", "Open Task Manager"),
                ("config", "Open config file"), ("none", "Nothing")]),
            get: |c| c.general.double_click_action.clone(),
            set: |c, s| { c.general.double_click_action = s.to_string(); Ok(()) } },
        Field { tab: 0, label: "Taskbar type", kind: choice(&[
                ("auto", "Automatic"), ("win11", "Windows 11"),
                ("classic", "Classic (Windows 10 / ExplorerPatcher)")]),
            get: |c| c.general.taskbar_mode.clone(),
            set: |c, s| { c.general.taskbar_mode = s.to_string(); Ok(()) } },
        Field { tab: 0, label: "Rendering", kind: choice(&[
                ("auto", "Automatic"), ("alpha", "Smooth transparency"),
                ("colorkey", "Color key (TrafficMonitor style)")]),
            get: |c| c.general.render_mode.clone(),
            set: |c, s| { c.general.render_mode = s.to_string(); Ok(()) } },
        // ---- Font
        Field { tab: 1, label: "Font family", kind: Kind::Choice(font_choices),
            get: |c| c.font.family.clone(),
            set: |c, s| { c.font.family = s.trim().to_string(); Ok(()) } },
        Field { tab: 1, label: "Size (pt)", kind: Kind::Num,
            get: |c| c.font.size.to_string(),
            set: |c, s| { c.font.size = num(s)?; Ok(()) } },
        Field { tab: 1, label: "Weight (100–900)", kind: Kind::Num,
            get: |c| c.font.weight.to_string(),
            set: |c, s| { c.font.weight = num(s)?; Ok(()) } },
        Field { tab: 1, label: "Italic", kind: Kind::Bool,
            get: |c| bstr(c.font.italic),
            set: |c, s| { c.font.italic = boolv(s)?; Ok(()) } },
        Field { tab: 1, label: "Quality", kind: choice(&[
                ("antialiased", "Antialiased"), ("cleartype", "ClearType (colorkey mode)"), ("none", "None")]),
            get: |c| c.font.quality.clone(),
            set: |c, s| { c.font.quality = s.to_string(); Ok(()) } },
        Field { tab: 1, label: "Text gamma (alpha mode)", kind: Kind::Num,
            get: |c| c.font.text_gamma.to_string(),
            set: |c, s| { c.font.text_gamma = num(s)?; Ok(()) } },
        // ---- Layout
        Field { tab: 2, label: "Items per column", kind: Kind::Num,
            get: |c| c.layout.rows.to_string(),
            set: |c, s| { c.layout.rows = num(s)?; Ok(()) } },
        Field { tab: 2, label: "Side padding (px)", kind: Kind::Num,
            get: |c| c.layout.padding_x.to_string(),
            set: |c, s| { c.layout.padding_x = num(s)?; Ok(()) } },
        Field { tab: 2, label: "Line spacing (px)", kind: Kind::Num,
            get: |c| c.layout.line_spacing.to_string(),
            set: |c, s| { c.layout.line_spacing = num(s)?; Ok(()) } },
        Field { tab: 2, label: "Column spacing (px)", kind: Kind::Num,
            get: |c| c.layout.column_spacing.to_string(),
            set: |c, s| { c.layout.column_spacing = num(s)?; Ok(()) } },
        Field { tab: 2, label: "Label–value gap (px)", kind: Kind::Num,
            get: |c| c.layout.label_value_gap.to_string(),
            set: |c, s| { c.layout.label_value_gap = num(s)?; Ok(()) } },
        Field { tab: 2, label: "Minimum width (px)", kind: Kind::Num,
            get: |c| c.layout.min_width.to_string(),
            set: |c, s| { c.layout.min_width = num(s)?; Ok(()) } },
        Field { tab: 2, label: "Value alignment", kind: choice(&[("right", "Right"), ("left", "Left")]),
            get: |c| c.layout.value_align.clone(),
            set: |c, s| { c.layout.value_align = s.to_string(); Ok(()) } },
        Field { tab: 2, label: "Fixed width (no jitter)", kind: Kind::Bool,
            get: |c| bstr(c.layout.fixed_width),
            set: |c, s| { c.layout.fixed_width = boolv(s)?; Ok(()) } },
        Field { tab: 2, label: "Win11 position", kind: choice(&[
                ("right", "Right of tray, outside ^"), ("left", "Left side of taskbar")]),
            get: |c| c.layout.position.clone(),
            set: |c, s| { c.layout.position = s.to_string(); Ok(()) } },
        Field { tab: 2, label: "Offset X (px)", kind: Kind::Num,
            get: |c| c.layout.offset_x.to_string(),
            set: |c, s| { c.layout.offset_x = num(s)?; Ok(()) } },
        Field { tab: 2, label: "Offset Y (px)", kind: Kind::Num,
            get: |c| c.layout.offset_y.to_string(),
            set: |c, s| { c.layout.offset_y = num(s)?; Ok(()) } },
        Field { tab: 2, label: "Win11 fallback tray width (px)", kind: Kind::Num,
            get: |c| c.layout.win11_right_space.to_string(),
            set: |c, s| { c.layout.win11_right_space = num(s)?; Ok(()) } },
        // ---- Colors
        Field { tab: 3, label: "Text color", kind: Kind::Text,
            get: |c| c.colors.text.clone(),
            set: |c, s| { c.colors.text = color(s)?; Ok(()) } },
        Field { tab: 3, label: "Background color", kind: Kind::Text,
            get: |c| c.colors.background.clone(),
            set: |c, s| { c.colors.background = color(s)?; Ok(()) } },
        Field { tab: 3, label: "Background opacity (0–255)", kind: Kind::Num,
            get: |c| c.colors.background_opacity.to_string(),
            set: |c, s| { c.colors.background_opacity = num(s)?; Ok(()) } },
        Field { tab: 3, label: "Color-key color (colorkey mode)", kind: Kind::Text,
            get: |c| c.colors.colorkey.clone(),
            set: |c, s| { c.colors.colorkey = color(s)?; Ok(()) } },
        Field { tab: 3, label: "Graph bar color", kind: Kind::Text,
            get: |c| c.colors.graph_color.clone(),
            set: |c, s| { c.colors.graph_color = color(s)?; Ok(()) } },
        Field { tab: 3, label: "Graph bar opacity (0–255)", kind: Kind::Num,
            get: |c| c.colors.graph_opacity.to_string(),
            set: |c, s| { c.colors.graph_opacity = num(s)?; Ok(()) } },
        // ---- Format
        Field { tab: 4, label: "Unit", kind: choice(&[
                ("auto", "Automatic (B/s, KB/s, MB/s, GB/s)"), ("B", "Always B/s"),
                ("KB", "Always KB/s"), ("MB", "Always MB/s"), ("GB", "Always GB/s")]),
            get: |c| c.format.unit.clone(),
            set: |c, s| { c.format.unit = s.to_string(); Ok(()) } },
        Field { tab: 4, label: "Use bits (b/s)", kind: Kind::Bool,
            get: |c| bstr(c.format.bits),
            set: |c, s| { c.format.bits = boolv(s)?; Ok(()) } },
        Field { tab: 4, label: "Short mode (8.9 K)", kind: Kind::Bool,
            get: |c| bstr(c.format.short),
            set: |c, s| { c.format.short = boolv(s)?; Ok(()) } },
        Field { tab: 4, label: "Space before unit", kind: Kind::Bool,
            get: |c| bstr(c.format.space),
            set: |c, s| { c.format.space = boolv(s)?; Ok(()) } },
        Field { tab: 4, label: "Hide unit", kind: Kind::Bool,
            get: |c| bstr(c.format.hide_unit),
            set: |c, s| { c.format.hide_unit = boolv(s)?; Ok(()) } },
        Field { tab: 4, label: "Append /s", kind: Kind::Bool,
            get: |c| bstr(c.format.per_second),
            set: |c, s| { c.format.per_second = boolv(s)?; Ok(()) } },
        Field { tab: 4, label: "Show bytes tier (auto)", kind: Kind::Bool,
            get: |c| bstr(c.format.show_bytes_tier),
            set: |c, s| { c.format.show_bytes_tier = boolv(s)?; Ok(()) } },
        Field { tab: 4, label: "Base (1024 or 1000)", kind: choice(&[("1024", "1024 (like TrafficMonitor)"), ("1000", "1000 (SI)")]),
            get: |c| c.format.kilo.to_string(),
            set: |c, s| { c.format.kilo = num(s)?; Ok(()) } },
        Field { tab: 4, label: "Minimum number width (chars)", kind: Kind::Num,
            get: |c| c.format.min_value_chars.to_string(),
            set: |c, s| { c.format.min_value_chars = num(s)?; Ok(()) } },
    ]
}

// ------------------------------------------------------------ fonts
unsafe extern "system" fn enum_font(lf: *const LOGFONTW, _tm: *const TEXTMETRICW, _ty: u32, lp: LPARAM) -> i32 {
    let names = &mut *(lp as *mut Vec<String>);
    let face = &(*lf).lfFaceName;
    let n = face.iter().position(|&c| c == 0).unwrap_or(face.len());
    let s = String::from_utf16_lossy(&face[..n]);
    if !s.is_empty() && !s.starts_with('@') {
        names.push(s);
    }
    1
}

fn font_families() -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    unsafe {
        let dc = GetDC(std::ptr::null_mut());
        let mut lf: LOGFONTW = std::mem::zeroed();
        lf.lfCharSet = DEFAULT_CHARSET as u8;
        EnumFontFamiliesExW(dc, &lf, Some(enum_font), &mut names as *mut Vec<String> as LPARAM, 0);
        ReleaseDC(std::ptr::null_mut(), dc);
    }
    names.sort_by_key(|s| s.to_lowercase());
    names.dedup();
    names
}

// ------------------------------------------------------------ geometry
/// Client size of the settings window at a given DPI.
fn metrics(dpi: u32) -> (i32, i32) {
    let pad = scale(12, dpi);
    let y0 = pad + scale(34, dpi);
    (scale(540, dpi), y0 + scale(30, dpi) * 12 + scale(70, dpi))
}

unsafe fn mk(font: HFONT, parent: HWND, hinst: HINSTANCE, class: &str, text: &str, style: u32, ex: u32,
    x: i32, y: i32, w: i32, h: i32, id: usize) -> HWND {
    let c = wide(class);
    let t = wide(text);
    let hwnd = CreateWindowExW(ex, c.as_ptr(), t.as_ptr(), WS_CHILD | style, x, y, w, h,
        parent, id as HMENU, hinst, std::ptr::null());
    SendMessageW(hwnd, WM_SETFONT, font as usize, 1);
    hwnd
}

unsafe fn build(st: &mut State, fields: Vec<Field>, dpi: u32) {
    let (hwnd, hinst, font) = (st.hwnd, st.hinst, st.font);
    let mut cr: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut cr);
    let (cw, ch) = (cr.right, cr.bottom);
    let pad = scale(12, dpi);
    let lw = scale(190, dpi);
    let row = scale(30, dpi);
    let y0 = pad + scale(34, dpi);
    let label_x = pad * 2;
    let ctrl_x = label_x + lw + scale(6, dpi);
    let page_r = cw - pad;
    let h22 = scale(22, dpi);
    let push = WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32;
    let defpush = WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON as u32;

    st.tab = mk(font, hwnd, hinst, "SysTabControl32", "", WS_VISIBLE | WS_TABSTOP, 0,
        pad, pad, cw - 2 * pad, ch - scale(56, dpi) - pad, 0);
    for (i, name) in TABS.iter().enumerate() {
        let t = wide(name);
        let mut it: TCITEMW = std::mem::zeroed();
        it.mask = TCIF_TEXT;
        it.pszText = t.as_ptr() as *mut u16;
        SendMessageW(st.tab, TCM_INSERTITEMW, i, &it as *const TCITEMW as LPARAM);
    }

    let mut rows = [0i32; 6];
    let mut ctrls = Vec::with_capacity(fields.len());
    for (idx, f) in fields.iter().enumerate() {
        let r = rows[f.tab];
        rows[f.tab] += 1;
        let y = y0 + r * row;
        let id = ID_FIELD + idx;
        let (ctrl, lbl) = match &f.kind {
            Kind::Bool => {
                let c = mk(font, hwnd, hinst, "BUTTON", f.label, BS_AUTOCHECKBOX as u32 | WS_TABSTOP, 0,
                    label_x, y, page_r - label_x - pad, h22, id);
                (c, None)
            }
            Kind::Choice(choices) => {
                let l = mk(font, hwnd, hinst, "STATIC", f.label, 0, 0, label_x, y + scale(4, dpi), lw, h22, 0);
                let c = mk(font, hwnd, hinst, "COMBOBOX", "", CBS_DROPDOWNLIST as u32 | WS_VSCROLL | WS_TABSTOP, 0,
                    ctrl_x, y, page_r - pad - ctrl_x, scale(200, dpi), id);
                for (_, label) in choices {
                    let t = wide(label);
                    SendMessageW(c, CB_ADDSTRING, 0, t.as_ptr() as LPARAM);
                }
                (c, Some(l))
            }
            Kind::Text | Kind::Num => {
                let l = mk(font, hwnd, hinst, "STATIC", f.label, 0, 0, label_x, y + scale(4, dpi), lw, h22, 0);
                let c = mk(font, hwnd, hinst, "EDIT", "", WS_BORDER | ES_AUTOHSCROLL as u32 | WS_TABSTOP, 0,
                    ctrl_x, y, page_r - pad - ctrl_x, h22, id);
                (c, Some(l))
            }
        };
        st.page.push((f.tab, ctrl));
        if let Some(l) = lbl {
            st.page.push((f.tab, l));
        }
        ctrls.push(ctrl);
    }
    st.ctrls = ctrls;
    st.fields = fields;

    // ---- Items tab: a small grid
    let cols = [label_x, label_x + scale(130, dpi), label_x + scale(220, dpi), label_x + scale(300, dpi), label_x + scale(380, dpi)];
    let widths = [scale(125, dpi), scale(85, dpi), scale(75, dpi), scale(75, dpi), scale(90, dpi)];
    let heads = ["Item", "Label", "Label color", "Value color", "Graph Mbit/s"];
    for c in 0..5 {
        let h = mk(font, hwnd, hinst, "STATIC", heads[c], 0, 0, cols[c], y0, widths[c], h22, 0);
        st.page.push((ITEM_TAB, h));
    }
    for (r, row_def) in ITEM_ROWS.iter().enumerate() {
        let y = y0 + (r as i32 + 1) * row;
        let id = ID_ITEM + r * 5;
        let en = mk(font, hwnd, hinst, "BUTTON", row_def.title, BS_AUTOCHECKBOX as u32 | WS_TABSTOP, 0,
            cols[0], y, widths[0], h22, id);
        let mut cells = [en; 5];
        for col in 1..5 {
            cells[col] = mk(font, hwnd, hinst, "EDIT", "", WS_BORDER | ES_AUTOHSCROLL as u32 | WS_TABSTOP, 0,
                cols[col], y, widths[col], h22, id + col);
        }
        for h in cells {
            st.page.push((ITEM_TAB, h));
        }
        st.item_ctrls.push(cells);
    }
    let hint = mk(font, hwnd, hinst, "STATIC", "Colors: auto or #RRGGBB. Graph: bar full at this Mbit/s (0 = off).", 0, 0,
        label_x, y0 + row * 7, page_r - label_x - pad, h22, 0);
    st.page.push((ITEM_TAB, hint));

    // ---- Bottom buttons (always visible)
    let by = ch - scale(40, dpi);
    let bw = scale(110, dpi);
    let bh = scale(26, dpi);
    mk(font, hwnd, hinst, "BUTTON", "Reset to defaults", push, 0, label_x, by, scale(140, dpi), bh, ID_RESET);
    mk(font, hwnd, hinst, "BUTTON", "Open TOML file", push, 0, label_x + scale(150, dpi), by, scale(120, dpi), bh, ID_OPENFILE);
    mk(font, hwnd, hinst, "BUTTON", "Apply", defpush, 0, page_r - 2 * bw - scale(10, dpi), by, bw, bh, ID_APPLY);
    mk(font, hwnd, hinst, "BUTTON", "Close", push, 0, page_r - bw, by, bw, bh, ID_CLOSE);
}

// ------------------------------------------------------------ populate / read
unsafe fn set_text(h: HWND, s: &str) {
    let w = wide(s);
    SetWindowTextW(h, w.as_ptr());
}

unsafe fn get_text(h: HWND) -> String {
    let n = GetWindowTextLengthW(h).max(0) as usize;
    let mut buf = vec![0u16; n + 1];
    GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32);
    String::from_utf16_lossy(&buf[..n])
}

unsafe fn populate(st: &mut State, cfg: &Config) {
    st.suppress = true;
    for i in 0..st.fields.len() {
        let val = (st.fields[i].get)(cfg);
        let h = st.ctrls[i];
        match &mut st.fields[i].kind {
            Kind::Bool => {
                SendMessageW(h, BM_SETCHECK, (val == "true") as usize, 0);
            }
            Kind::Text | Kind::Num => set_text(h, &val),
            Kind::Choice(choices) => {
                let idx = match choices.iter().position(|(v, _)| *v == val) {
                    Some(p) => p,
                    None => {
                        // value not in the list (e.g. font not installed): show it anyway
                        choices.push((val.clone(), val.clone()));
                        let t = wide(&val);
                        SendMessageW(h, CB_ADDSTRING, 0, t.as_ptr() as LPARAM);
                        choices.len() - 1
                    }
                };
                SendMessageW(h, CB_SETCURSEL, idx, 0);
            }
        }
    }
    for (r, row) in ITEM_ROWS.iter().enumerate() {
        let c = st.item_ctrls[r];
        let found = cfg.items.iter().find(|i| i.kind.eq_ignore_ascii_case(row.kind));
        SendMessageW(c[0], BM_SETCHECK, found.is_some() as usize, 0);
        set_text(c[1], found.map_or(row.label, |i| i.label.as_str()));
        set_text(c[2], found.map_or(row.color, |i| i.label_color.as_str()));
        set_text(c[3], found.map_or("auto", |i| i.value_color.as_str()));
        set_text(c[4], &found.map_or(0.0, |i| i.graph_limit()).to_string());
    }
    st.suppress = false;
}

unsafe fn show_tab(st: &State, tab: usize) {
    for (t, h) in &st.page {
        ShowWindow(*h, if *t == tab { SW_SHOW } else { SW_HIDE });
    }
}

unsafe fn collect(st: &State) -> Result<Config, String> {
    let text = std::fs::read_to_string(sys::config_path()).unwrap_or_default();
    let mut cfg = config::parse(text.trim_start_matches('\u{feff}')).unwrap_or_default();
    for i in 0..st.fields.len() {
        let f = &st.fields[i];
        let val = match &f.kind {
            Kind::Bool => bstr(SendMessageW(st.ctrls[i], BM_GETCHECK, 0, 0) == BST_CHECKED as isize),
            Kind::Text | Kind::Num => get_text(st.ctrls[i]),
            Kind::Choice(ch) => {
                let idx = SendMessageW(st.ctrls[i], CB_GETCURSEL, 0, 0);
                ch.get(idx as usize).map(|c| c.0.clone()).unwrap_or_default()
            }
        };
        (f.set)(&mut cfg, &val).map_err(|e| format!("{}: {}", f.label, e))?;
    }
    let mut items = Vec::new();
    for (r, row) in ITEM_ROWS.iter().enumerate() {
        let c = st.item_ctrls[r];
        if SendMessageW(c[0], BM_GETCHECK, 0, 0) == BST_CHECKED as isize {
            let lc = color(&get_text(c[2])).map_err(|e| format!("{} label color: {e}", row.title))?;
            let vc = color(&get_text(c[3])).map_err(|e| format!("{} value color: {e}", row.title))?;
            let graph_limit_mbit: f64 = get_text(c[4]).trim().parse()
                .map_err(|_| format!("{} graph Mbit/s: must be a number", row.title))?;
            items.push(Item { kind: row.kind.to_string(), label: get_text(c[1]), label_color: lc,
                value_color: vc, graph_limit_mbit: Some(graph_limit_mbit) });
        }
    }
    if items.is_empty() {
        return Err("Turn on at least one item.".to_string());
    }
    cfg.items = items;
    Ok(cfg)
}

unsafe fn warn(hwnd: HWND, msg: &str) {
    let t = wide(msg);
    let c = wide("TrafficBar");
    MessageBoxW(hwnd, t.as_ptr(), c.as_ptr(), MB_ICONWARNING);
}

/// Reads the controls, validates, and writes the file if it changed.
unsafe fn apply(st: &State) -> bool {
    let cfg = match collect(st) {
        Ok(c) => c,
        Err(e) => {
            warn(st.hwnd, &e);
            return false;
        }
    };
    let text = config::to_toml(&cfg);
    let path = sys::config_path();
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    if old.replace("\r\n", "\n") == text.replace("\r\n", "\n") {
        return true;
    }
    if let Err(e) = std::fs::write(&path, &text) {
        warn(st.hwnd, &format!("Could not save settings: {e}"));
        return false;
    }
    PostMessageW(st.host, WM_APP_RELOAD, 0, 0);
    true
}

// ------------------------------------------------------------ window
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let sp = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
    if sp.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let st = &mut *sp;
    match msg {
        WM_COMMAND => {
            let id = wp & 0xffff;
            let code = ((wp >> 16) & 0xffff) as u32;
            match id {
                ID_APPLY => {
                    apply(st);
                }
                ID_RESET => {
                    let t = wide("Reset every TrafficBar setting to its default?");
                    let c = wide("TrafficBar");
                    if MessageBoxW(hwnd, t.as_ptr(), c.as_ptr(), MB_YESNO | MB_ICONQUESTION) == IDYES {
                        populate(st, &Config::default());
                        apply(st);
                    }
                }
                ID_OPENFILE => crate::app::open_config(),
                ID_CLOSE => {
                    DestroyWindow(hwnd);
                }
                x if (ID_FIELD..ID_FIELD + 500).contains(&x) || (ID_ITEM..ID_ITEM + 200).contains(&x) => {
                    if !st.suppress && (code == BN_CLICKED || code == CBN_SELCHANGE || code == EN_KILLFOCUS) {
                        apply(st);
                    }
                }
                _ => {}
            }
            0
        }
        WM_NOTIFY => {
            let nm = &*(lp as *const NMHDR);
            if nm.code == TCN_SELCHANGE {
                let idx = SendMessageW(st.tab, TCM_GETCURSEL, 0, 0) as usize;
                show_tab(st, idx);
            }
            0
        }
        WM_CTLCOLORSTATIC => {
            SetBkMode(wp as HDC, TRANSPARENT as i32);
            GetStockObject(NULL_BRUSH) as LRESULT
        }
        WM_DESTROY => {
            WINDOW.store(0, Ordering::Relaxed);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Box::from_raw(sp));
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Opens the settings window, or brings the open one to the front.
pub fn open(host: HWND, hinst: HINSTANCE, cfg: &Config) {
    unsafe {
        if is_open() {
            let h = window();
            ShowWindow(h, SW_SHOW);
            SetForegroundWindow(h);
            return;
        }
        let icc = INITCOMMONCONTROLSEX { dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32, dwICC: ICC_TAB_CLASSES };
        InitCommonControlsEx(&icc);

        let cls = wide(CLASS);
        let mut wc: WNDCLASSEXW = std::mem::zeroed();
        wc.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
        wc.lpfnWndProc = Some(proc);
        wc.hInstance = hinst;
        wc.hCursor = LoadCursorW(std::ptr::null_mut(), IDC_ARROW);
        wc.hbrBackground = (COLOR_BTNFACE + 1) as usize as HBRUSH;
        wc.lpszClassName = cls.as_ptr();
        RegisterClassExW(&wc);

        let dpi = GetDpiForSystem().max(96);
        let (cw, ch) = metrics(dpi);
        let style = WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut rc = RECT { left: 0, top: 0, right: cw, bottom: ch };
        AdjustWindowRectEx(&mut rc, style, 0, 0);
        let (ow, oh) = (rc.right - rc.left, rc.bottom - rc.top);
        let title = wide("TrafficBar settings");
        let hwnd = CreateWindowExW(0, cls.as_ptr(), title.as_ptr(), style, CW_USEDEFAULT, CW_USEDEFAULT,
            ow, oh, std::ptr::null_mut(), std::ptr::null_mut(), hinst, std::ptr::null());
        if hwnd.is_null() {
            return;
        }
        let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        SetWindowPos(hwnd, std::ptr::null_mut(), (sw - ow) / 2, (sh - oh) / 2, 0, 0, SWP_NOSIZE | SWP_NOZORDER);

        let adapters = stats::list_adapters();
        let mut families = font_families();
        families.retain(|f| !f.is_empty());
        let fields = build_fields(&adapters, &families);
        let font = GetStockObject(DEFAULT_GUI_FONT) as HFONT;
        let st = Box::into_raw(Box::new(State {
            hwnd, host, hinst, font, fields: Vec::new(), ctrls: Vec::new(),
            item_ctrls: Vec::new(), page: Vec::new(), tab: std::ptr::null_mut(), suppress: false,
        }));
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, st as isize);
        WINDOW.store(hwnd as usize, Ordering::Relaxed);
        build(&mut *st, fields, dpi);
        populate(&mut *st, cfg);
        show_tab(&*st, 0);
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
    }
}

/// Called after the widget reloads the file, so the form shows the current values.
pub fn on_reload(cfg: &Config) {
    if !is_open() {
        return;
    }
    unsafe {
        let sp = GetWindowLongPtrW(window(), GWLP_USERDATA) as *mut State;
        if !sp.is_null() {
            populate(&mut *sp, cfg);
        }
    }
}
