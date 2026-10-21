//! Layout and GDI rendering (per-pixel alpha or TrafficMonitor-style color key).

use crate::config::{parse_color, Config};
use crate::sys::{rgb, wide};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub struct Run {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub text: Vec<u16>,
    pub color: (u8, u8, u8),
}

pub struct Frame {
    pub w: i32,
    pub h: i32,
    pub runs: Vec<Run>,
    /// Filled part of each background bar: (x, y, w, h).
    pub bars: Vec<(i32, i32, i32, i32)>,
    pub bar_color: (u8, u8, u8),
    pub bar_alpha: u8,
}

/// One line of text to show: label + value + which samples fix its width.
pub struct Cell {
    pub label: String,
    pub value: String,
    pub samples: Vec<String>,
    pub label_color: (u8, u8, u8),
    pub value_color: (u8, u8, u8),
    /// Background bar fraction (0..1), None = no bar.
    pub fill: Option<f64>,
}

#[derive(PartialEq, Clone)]
struct FontKey {
    family: String,
    size_tenths: i64,
    weight: i32,
    italic: bool,
    quality: u32,
    dpi: u32,
}

pub struct Renderer {
    font: HFONT,
    key: Option<FontKey>,
    line_h: i32,
}

impl Drop for Renderer {
    fn drop(&mut self) {
        if !self.font.is_null() {
            unsafe { DeleteObject(self.font) };
        }
    }
}

pub fn scale(px: i32, dpi: u32) -> i32 {
    ((px as i64 * dpi as i64 + 48) / 96) as i32
}

pub fn resolve_color(s: &str, fallback: (u8, u8, u8)) -> (u8, u8, u8) {
    parse_color(s).unwrap_or(fallback)
}

fn quality_of(s: &str, alpha: bool) -> u32 {
    match s.to_ascii_lowercase().as_str() {
        "none" | "aliased" => NONANTIALIASED_QUALITY as u32,
        "cleartype" if !alpha => CLEARTYPE_QUALITY as u32,
        _ => ANTIALIASED_QUALITY as u32,
    }
}

impl Renderer {
    pub fn new() -> Self {
        Self { font: std::ptr::null_mut(), key: None, line_h: 16 }
    }

    pub fn ensure_font(&mut self, cfg: &Config, dpi: u32, alpha: bool) {
        let key = FontKey {
            family: cfg.font.family.clone(),
            size_tenths: (cfg.font.size * 10.0).round() as i64,
            weight: cfg.font.weight.clamp(100, 900),
            italic: cfg.font.italic,
            quality: quality_of(&cfg.font.quality, alpha),
            dpi,
        };
        if self.key.as_ref() == Some(&key) {
            return;
        }
        unsafe {
            if !self.font.is_null() {
                DeleteObject(self.font);
            }
            let height = -((cfg.font.size * dpi as f64 / 72.0).round() as i32);
            let face = wide(&key.family);
            self.font = CreateFontW(height, 0, 0, 0, key.weight, key.italic as u32, 0, 0,
                DEFAULT_CHARSET as u32, OUT_TT_PRECIS as u32, CLIP_DEFAULT_PRECIS as u32,
                key.quality, (FIXED_PITCH | FF_MODERN) as u32, face.as_ptr());
            let dc = GetDC(std::ptr::null_mut());
            let old = SelectObject(dc, self.font);
            let mut tm: TEXTMETRICW = std::mem::zeroed();
            GetTextMetricsW(dc, &mut tm);
            self.line_h = tm.tmHeight.max(1);
            SelectObject(dc, old);
            ReleaseDC(std::ptr::null_mut(), dc);
        }
        self.key = Some(key);
    }

    pub fn line_height(&self) -> i32 {
        self.line_h
    }

    fn measure(&self, dc: HDC, s: &str) -> i32 {
        if s.is_empty() {
            return 0;
        }
        let w: Vec<u16> = s.encode_utf16().collect();
        let mut sz = SIZE { cx: 0, cy: 0 };
        unsafe { GetTextExtentPoint32W(dc, w.as_ptr(), w.len() as i32, &mut sz) };
        sz.cx
    }

    /// Lays cells out in columns of `rows` lines. `height` is the window height
    /// (content is centered vertically inside it).
    pub fn layout(&self, cfg: &Config, cells: &[Cell], dpi: u32, height: i32) -> Frame {
        let l = &cfg.layout;
        let rows = l.rows.max(1) as usize;
        let pad = scale(l.padding_x, dpi);
        let gap = scale(l.label_value_gap, dpi);
        let col_gap = scale(l.column_spacing, dpi);
        let line_gap = scale(l.line_spacing, dpi);
        let right = !l.value_align.eq_ignore_ascii_case("left");

        let dc = unsafe { GetDC(std::ptr::null_mut()) };
        let old = unsafe { SelectObject(dc, self.font) };

        let mut runs = Vec::new();
        let mut bars = Vec::new();
        let mut x = pad;
        let lines_in_first = cells.len().min(rows) as i32;
        let content_h = lines_in_first * self.line_h + (lines_in_first - 1).max(0) * line_gap;
        let top = (height - content_h) / 2 + scale(l.offset_y, dpi);

        for col in cells.chunks(rows) {
            // widths
            let metrics: Vec<(i32, i32)> = col
                .iter()
                .map(|c| {
                    let lw = self.measure(dc, &c.label);
                    let mut vw = self.measure(dc, &c.value);
                    if l.fixed_width {
                        for s in &c.samples {
                            vw = vw.max(self.measure(dc, s));
                        }
                    }
                    (lw, vw)
                })
                .collect();
            let max_label = metrics.iter().map(|m| m.0).max().unwrap_or(0);
            let max_value = metrics.iter().map(|m| m.1).max().unwrap_or(0);
            let col_w = max_label + if max_value > 0 { gap + max_value } else { 0 };

            for (i, (c, m)) in col.iter().zip(&metrics).enumerate() {
                let y = top + i as i32 * (self.line_h + line_gap);
                if let Some(fr) = c.fill {
                    let fw = (col_w as f64 * fr.clamp(0.0, 1.0)).round() as i32;
                    if fw > 0 {
                        bars.push((x, y, fw, self.line_h));
                    }
                }
                if !c.label.is_empty() {
                    runs.push(Run { x, y, w: m.0, h: self.line_h, text: c.label.encode_utf16().collect(), color: c.label_color });
                }
                if !c.value.is_empty() {
                    let vw = self.measure(dc, &c.value);
                    let vx = if right { x + col_w - vw } else { x + max_label + gap };
                    runs.push(Run { x: vx, y, w: vw, h: self.line_h, text: c.value.encode_utf16().collect(), color: c.value_color });
                }
            }
            x += col_w + col_gap;
        }
        unsafe {
            SelectObject(dc, old);
            ReleaseDC(std::ptr::null_mut(), dc);
        }
        let w = (x - col_gap + pad).max(scale(l.min_width, dpi)).max(1);
        Frame {
            w,
            h: height.max(1),
            runs,
            bars,
            bar_color: resolve_color(&cfg.colors.graph_color, (0x6E, 0x76, 0x81)),
            bar_alpha: cfg.colors.graph_opacity,
        }
    }

    /// Per-pixel alpha path. Returns false if UpdateLayeredWindow is refused.
    pub fn present_alpha(&self, hwnd: HWND, f: &Frame, cfg: &Config) -> bool {
        unsafe {
            let screen = GetDC(std::ptr::null_mut());
            let mem = CreateCompatibleDC(screen);
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = f.w;
            bmi.bmiHeader.biHeight = -f.h;
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB;
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bmp = CreateDIBSection(screen, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            if bmp.is_null() || bits.is_null() {
                DeleteDC(mem);
                ReleaseDC(std::ptr::null_mut(), screen);
                return false;
            }
            let oldbmp = SelectObject(mem, bmp);
            let oldfont = SelectObject(mem, self.font);
            let n = (f.w * f.h) as usize;
            let px = std::slice::from_raw_parts_mut(bits as *mut u8, n * 4);
            px.fill(0);

            SetBkMode(mem, TRANSPARENT as i32);
            SetTextColor(mem, rgb(255, 255, 255));
            let mut owner = vec![u16::MAX; n];
            for (ri, r) in f.runs.iter().enumerate() {
                TextOutW(mem, r.x, r.y, r.text.as_ptr(), r.text.len() as i32);
                // ownership map (slightly enlarged for glyph overhang)
                for yy in (r.y - 1).max(0)..(r.y + r.h + 1).min(f.h) {
                    for xx in (r.x - 2).max(0)..(r.x + r.w + 2).min(f.w) {
                        owner[(yy * f.w + xx) as usize] = ri as u16;
                    }
                }
            }
            GdiFlush();

            let (br, bg, bb) = resolve_color(&cfg.colors.background, (0, 0, 0));
            let ba = cfg.colors.background_opacity as f32 / 255.0;
            let inv_gamma = 1.0 / cfg.font.text_gamma as f32;
            let default_col = f.runs.last().map(|r| r.color).unwrap_or((255, 255, 255));
            let (gr, gg, gb) = f.bar_color;
            let ga = f.bar_alpha as f32 / 255.0;
            let mut filled = vec![false; n];
            for &(x, y, w, h) in &f.bars {
                for yy in y.max(0)..(y + h).min(f.h) {
                    for xx in x.max(0)..(x + w).min(f.w) {
                        filled[(yy * f.w + xx) as usize] = true;
                    }
                }
            }
            for i in 0..n {
                let p = &mut px[i * 4..i * 4 + 4];
                let mut cov = p[0].max(p[1]).max(p[2]) as f32 / 255.0;
                if inv_gamma != 1.0 && cov > 0.0 {
                    cov = cov.powf(inv_gamma);
                }
                let (tr, tg, tb) = match owner[i] {
                    u16::MAX => default_col,
                    k => f.runs[k as usize].color,
                };
                // layer underneath the text: background, or the graph bar over it
                let (lr, lg, lb, la) = if filled[i] {
                    (gr as f32 * ga + br as f32 * ba * (1.0 - ga),
                     gg as f32 * ga + bg as f32 * ba * (1.0 - ga),
                     gb as f32 * ga + bb as f32 * ba * (1.0 - ga),
                     ga + ba * (1.0 - ga))
                } else {
                    (br as f32 * ba, bg as f32 * ba, bb as f32 * ba, ba)
                };
                let rest = 1.0 - cov;
                let a = cov + la * rest;
                p[0] = (tb as f32 * cov + lb * rest).round().min(255.0) as u8;
                p[1] = (tg as f32 * cov + lg * rest).round().min(255.0) as u8;
                p[2] = (tr as f32 * cov + lr * rest).round().min(255.0) as u8;
                p[3] = (a * 255.0).round().min(255.0) as u8;
            }

            let size = SIZE { cx: f.w, cy: f.h };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let ok = UpdateLayeredWindow(hwnd, screen, std::ptr::null(), &size, mem, &src, 0, &blend, ULW_ALPHA) != 0;

            SelectObject(mem, oldfont);
            SelectObject(mem, oldbmp);
            DeleteObject(bmp);
            DeleteDC(mem);
            ReleaseDC(std::ptr::null_mut(), screen);
            ok
        }
    }

    /// Color-key path: paint onto `dc` with `key` as the (transparent) background.
    pub fn paint_colorkey(&self, dc: HDC, f: &Frame, key: COLORREF) {
        unsafe {
            let mem = CreateCompatibleDC(dc);
            let bmp = CreateCompatibleBitmap(dc, f.w, f.h);
            let oldbmp = SelectObject(mem, bmp);
            let brush = CreateSolidBrush(key);
            let rc = RECT { left: 0, top: 0, right: f.w, bottom: f.h };
            FillRect(mem, &rc, brush);
            DeleteObject(brush);
            let gbrush = CreateSolidBrush(rgb(f.bar_color.0, f.bar_color.1, f.bar_color.2));
            for &(x, y, w, h) in &f.bars {
                let rc = RECT { left: x, top: y, right: x + w, bottom: y + h };
                FillRect(mem, &rc, gbrush);
            }
            DeleteObject(gbrush);
            let oldfont = SelectObject(mem, self.font);
            SetBkMode(mem, TRANSPARENT as i32);
            for r in &f.runs {
                let mut c = rgb(r.color.0, r.color.1, r.color.2);
                if c == key {
                    c ^= 0x010101; // never draw in the key color
                }
                SetTextColor(mem, c);
                TextOutW(mem, r.x, r.y, r.text.as_ptr(), r.text.len() as i32);
            }
            BitBlt(dc, 0, 0, f.w, f.h, mem, 0, 0, SRCCOPY);
            SelectObject(mem, oldfont);
            SelectObject(mem, oldbmp);
            DeleteObject(bmp);
            DeleteDC(mem);
        }
    }
}

pub fn set_colorkey(hwnd: HWND, key: COLORREF) {
    unsafe {
        SetLayeredWindowAttributes(hwnd, key, 255, LWA_COLORKEY);
    }
}
