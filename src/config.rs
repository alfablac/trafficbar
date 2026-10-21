//! Configuration (TrafficBar.toml next to the .exe). Every field is optional;
//! anything missing falls back to the defaults below.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub general: General,
    pub font: Font,
    pub layout: Layout,
    pub colors: Colors,
    pub format: FormatCfg,
    #[serde(default = "default_items")]
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct General {
    pub update_interval_ms: u32,
    pub start_with_windows: bool,
    pub tray_icon: bool,
    pub double_click_action: String,
    pub interface: String,
    pub include_virtual: bool,
    pub taskbar_mode: String,
    pub render_mode: String,
    pub smoothing: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Font {
    pub family: String,
    pub size: f64,
    pub weight: i32,
    pub italic: bool,
    pub quality: String,
    pub text_gamma: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Layout {
    pub rows: u32,
    pub padding_x: i32,
    pub line_spacing: i32,
    pub column_spacing: i32,
    pub label_value_gap: i32,
    pub value_align: String,
    pub fixed_width: bool,
    pub position: String,
    pub offset_x: i32,
    pub offset_y: i32,
    pub win11_right_space: i32,
    pub min_width: i32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Colors {
    pub text: String,
    pub background: String,
    pub background_opacity: u8,
    pub colorkey: String,
    pub graph_color: String,
    pub graph_opacity: u8,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct FormatCfg {
    pub unit: String,
    pub bits: bool,
    pub short: bool,
    pub space: bool,
    pub hide_unit: bool,
    pub per_second: bool,
    pub show_bytes_tier: bool,
    pub kilo: u32,
    pub min_value_chars: u32,
    pub tiers: Vec<Tier>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Tier {
    /// Upper bound (exclusive) in bytes/s, or bits/s when `bits = true`. Omit for "infinity".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub below: Option<f64>,
    #[serde(default = "one")]
    pub divisor: f64,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub decimals: u32,
}
fn one() -> f64 {
    1.0
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Item {
    /// down | up | total | cpu | memory | text
    pub kind: String,
    pub label: String,
    pub label_color: String,
    pub value_color: String,
    /// Speed (Mbit/s) at which the background bar is full. None = per-kind default; 0 = no bar.
    pub graph_limit_mbit: Option<f64>,
}

impl Default for General {
    fn default() -> Self {
        Self {
            update_interval_ms: 1000,
            start_with_windows: true,
            tray_icon: true,
            double_click_action: "taskmgr".into(),
            interface: String::new(),
            include_virtual: false,
            taskbar_mode: "auto".into(),
            render_mode: "auto".into(),
            smoothing: 0.0,
        }
    }
}
impl Default for Font {
    fn default() -> Self {
        Self {
            family: "JetBrains Mono".into(),
            size: 9.0,
            weight: 400,
            italic: false,
            quality: "antialiased".into(),
            text_gamma: 1.0,
        }
    }
}
impl Default for Layout {
    fn default() -> Self {
        Self {
            rows: 2,
            padding_x: 4,
            line_spacing: 0,
            column_spacing: 10,
            label_value_gap: 0,
            value_align: "right".into(),
            fixed_width: true,
            position: "right".into(),
            offset_x: 0,
            offset_y: 0,
            win11_right_space: 120,
            min_width: 0,
        }
    }
}
impl Default for Colors {
    fn default() -> Self {
        Self {
            text: "auto".into(),
            background: "#000000".into(),
            background_opacity: 1,
            colorkey: "auto".into(),
            graph_color: "#6E7681".into(),
            graph_opacity: 90,
        }
    }
}
impl Default for FormatCfg {
    fn default() -> Self {
        Self {
            unit: "auto".into(),
            bits: false,
            short: false,
            space: true,
            hide_unit: false,
            per_second: true,
            show_bytes_tier: true,
            kilo: 1024,
            min_value_chars: 0,
            tiers: Vec::new(),
        }
    }
}
impl Default for Item {
    fn default() -> Self {
        Self {
            kind: "down".into(),
            label: String::new(),
            label_color: "auto".into(),
            value_color: "auto".into(),
            graph_limit_mbit: None,
        }
    }
}
impl Default for Config {
    fn default() -> Self {
        Self {
            general: General::default(),
            font: Font::default(),
            layout: Layout::default(),
            colors: Colors::default(),
            format: FormatCfg::default(),
            items: default_items(),
        }
    }
}

impl Item {
    /// Effective bar limit in Mbit/s. Files written before the setting existed get the defaults.
    pub fn graph_limit(&self) -> f64 {
        self.graph_limit_mbit.unwrap_or(match self.kind.to_ascii_lowercase().as_str() {
            "down" | "download" => 1000.0,
            "up" | "upload" => 500.0,
            _ => 0.0,
        })
    }
}

pub fn default_items() -> Vec<Item> {
    vec![
        Item {
            kind: "down".into(),
            label: "▼ ".into(),
            label_color: "#E5534B".into(),
            value_color: "auto".into(),
            graph_limit_mbit: Some(1000.0),
        },
        Item {
            kind: "up".into(),
            label: "▲ ".into(),
            label_color: "#57AB5A".into(),
            value_color: "auto".into(),
            graph_limit_mbit: Some(500.0),
        },
    ]
}

pub fn parse(text: &str) -> Result<Config, String> {
    let mut c: Config = toml::from_str(text).map_err(|e| e.to_string())?;
    c.layout.rows = c.layout.rows.clamp(1, 6);
    c.general.update_interval_ms = c.general.update_interval_ms.clamp(100, 60_000);
    c.general.smoothing = c.general.smoothing.clamp(0.0, 0.95);
    if !(c.font.size > 1.0 && c.font.size < 100.0) {
        c.font.size = 9.0;
    }
    if !(c.font.text_gamma > 0.1 && c.font.text_gamma < 5.0) {
        c.font.text_gamma = 1.0;
    }
    Ok(c)
}

/// Serializes settings to TOML (the Settings window writes this file).
pub fn to_toml(c: &Config) -> String {
    let body = toml::to_string_pretty(c).unwrap_or_default();
    let head = "# TrafficBar settings. Edit them in the Settings window (right-click the meter > Settings).\n# Saving from that window rewrites this file.\n\n";
    format!("{head}{body}").replace('\n', "\r\n")
}

/// Parses "#RRGGBB", "RRGGBB" or "#RGB" into (r, g, b). "auto"/invalid -> None.
pub fn parse_color(s: &str) -> Option<(u8, u8, u8)> {
    let h = s.trim().trim_start_matches('#');
    let h: String = if h.len() == 3 {
        h.chars().flat_map(|c| [c, c]).collect()
    } else {
        h.to_string()
    };
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(&h, 16).ok()?;
    Some(((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// Flips `start_with_windows = ...` in the config text while keeping comments intact.
pub fn set_bool_in_text(text: &str, section: &str, key: &str, value: bool) -> String {
    set_raw_in_text(text, section, key, if value { "true" } else { "false" })
}

/// Sets `key` in `[section]` to a TOML string value.
pub fn set_string_in_text(text: &str, section: &str, key: &str, value: &str) -> String {
    let esc = value.replace('\\', "\\\\").replace('"', "\\\"");
    set_raw_in_text(text, section, key, &format!("\"{esc}\""))
}

/// Replaces `key = ...` inside `[section]` keeping its trailing comment and every
/// other line intact; adds the key (and the section) when missing.
pub fn set_raw_in_text(text: &str, section: &str, key: &str, value: &str) -> String {
    let header = format!("[{section}]");
    let mut out: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    let mut cur = String::new();
    let mut header_idx = None;
    let mut done = false;
    for (i, l) in out.iter_mut().enumerate() {
        let t = l.trim_start();
        if t.starts_with('[') {
            cur = t.split('#').next().unwrap_or("").trim().to_string();
            if cur == header {
                header_idx = Some(i);
            }
            continue;
        }
        if cur == header && t.starts_with(key) && t[key.len()..].trim_start().starts_with('=') {
            let indent = l[..l.len() - t.len()].to_string();
            let comment = find_comment(t).map(|i| t[i..].to_string()).unwrap_or_default();
            let mut s = format!("{indent}{key} = {value}");
            if !comment.is_empty() {
                s.push_str("   ");
                s.push_str(&comment);
            }
            *l = s;
            done = true;
            break;
        }
    }
    if !done {
        match header_idx {
            Some(i) => out.insert(i + 1, format!("{key} = {value}")),
            None => {
                out.insert(0, String::new());
                out.insert(0, format!("{key} = {value}"));
                out.insert(0, header);
            }
        }
    }
    let mut s = out.join("\r\n");
    s.push_str("\r\n");
    s
}

/// Position of a `#` comment outside of a quoted string.
fn find_comment(line: &str) -> Option<usize> {
    let mut in_str = false;
    let mut esc = false;
    for (i, c) in line.char_indices() {
        match c {
            '\\' if in_str => esc = !esc,
            '"' if !esc => in_str = !in_str,
            '#' if !in_str => return Some(i),
            _ => esc = false,
        }
        if c != '\\' {
            esc = false;
        }
    }
    None
}

pub const DEFAULT_TOML: &str = r##"# =====================================================================
#  TrafficBar configuration
#  Saved changes are applied automatically (hot reload) - no restart needed.
#  Colors are "#RRGGBB" or "auto" (white on a dark taskbar, black on a light one).
# =====================================================================

[general]
update_interval_ms = 1000        # how often stats refresh (100..60000)
start_with_windows = true        # adds/removes HKCU\...\Run entry automatically
tray_icon = true                 # notification-area icon with the menu (right-click the widget works too)
double_click_action = "taskmgr"  # "taskmgr" | "config" | "none"
interface = ""                   # "" = sum of all connected physical adapters,
                                 # or part of an adapter name/description, e.g. "Wi-Fi", "Ethernet", "Intel"
include_virtual = false          # when interface = "", also count virtual adapters (VPN, Hyper-V, WSL...)
taskbar_mode = "auto"            # "auto" | "win11" | "classic" (Win10 / ExplorerPatcher taskbar)
render_mode = "auto"             # "auto" | "alpha" (smooth per-pixel transparency) | "colorkey" (TrafficMonitor-style)
smoothing = 0.0                  # 0 = raw values, up to 0.95 = heavy exponential smoothing

[font]
family = "JetBrains Mono"        # any installed font
size = 9.0                       # points (scaled with the taskbar DPI)
weight = 400                     # 100..900, 700 = bold
italic = false
quality = "antialiased"          # "antialiased" | "cleartype" (colorkey mode only) | "none"
text_gamma = 1.0                 # >1 makes antialiased text bolder in alpha mode

[layout]
rows = 2                         # items per column; extra items flow into new columns
padding_x = 4                    # px (at 100% scaling) left/right of the widget
line_spacing = 0                 # extra px between rows (can be negative)
column_spacing = 10              # px between columns
label_value_gap = 0              # extra px between label and value
value_align = "right"            # "right" | "left"
fixed_width = true               # reserve room for the widest value so nothing jumps around
min_width = 0                    # px, minimum widget width
position = "right"               # Win11: "right" (next to the tray, outside the ^ overflow) | "left" (left edge)
offset_x = 0                     # px nudge (negative = left)
offset_y = 0                     # px nudge (negative = up)
win11_right_space = 120          # fallback tray width if the tray area can't be found

[colors]
text = "auto"                    # default value color
background = "#000000"           # widget background
background_opacity = 1           # 0..255. 0 = fully transparent AND click-through,
                                 # 1 = looks transparent but still catches right-clicks, 255 = solid
graph_color = "auto"            # background bar color (auto = grey)
graph_opacity = 90               # bar opacity 0..255
colorkey = "auto"                # colorkey mode only: transparent key color (set close to your taskbar color)

[format]
unit = "auto"                    # "auto" | "B" | "KB" | "MB" | "GB"
bits = false                     # true = b/s, Kb/s, Mb/s instead of B/s, KB/s, MB/s
short = false                    # TrafficMonitor "short mode": 8.9 K instead of 8.92 KB/s
space = true                     # space between value and unit
hide_unit = false
per_second = true                # append "/s"
show_bytes_tier = true           # auto mode shows "512 B/s" below 1 KB (false = "0.50 KB/s", like TrafficMonitor)
kilo = 1024                      # 1024 (KiB-style, like TrafficMonitor) or 1000 (SI)
min_value_chars = 0              # left-pad the number to this many chars

# Optional: replace the automatic unit tiers completely. First tier whose
# `below` is greater than the value wins (values in bytes/s, or bits/s with bits = true).
# [[format.tiers]]
# below = 1024
# divisor = 1
# unit = "B"
# decimals = 0
# [[format.tiers]]
# below = 1048576
# divisor = 1024
# unit = "KB"
# decimals = 1
# [[format.tiers]]
# divisor = 1048576
# unit = "MB"
# decimals = 2

# What to show, in order. kind = "down" | "up" | "total" | "cpu" | "memory" | "text"
[[items]]
kind = "down"
label = "▼ "
label_color = "#E5534B"
value_color = "auto"
graph_limit_mbit = 1000          # bar fills to 100% at this download speed in Mbit/s (0 = no bar)

[[items]]
kind = "up"
label = "▲ "
label_color = "#57AB5A"
value_color = "auto"
graph_limit_mbit = 500           # bar fills to 100% at this upload speed in Mbit/s (0 = no bar)

# [[items]]
# kind = "cpu"
# label = "CPU "
# label_color = "auto"
# value_color = "auto"
#
# [[items]]
# kind = "memory"
# label = "MEM "
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_toml_parses_to_defaults() {
        let c = parse(DEFAULT_TOML).unwrap();
        assert_eq!(c.items.len(), 2);
        assert_eq!(c.font.family, "JetBrains Mono");
        assert!(c.general.start_with_windows);
        assert!(c.format.tiers.is_empty());
    }

    #[test]
    fn partial_config() {
        let c = parse("[font]\nsize = 11\n").unwrap();
        assert_eq!(c.font.size, 11.0);
        assert_eq!(c.items.len(), 2);
    }

    #[test]
    fn to_toml_round_trips() {
        let mut c = Config::default();
        c.general.interface = "Wi-Fi".into();
        c.format.tiers = vec![
            Tier { below: Some(1024.0), divisor: 1.0, unit: "B".into(), decimals: 0 },
            Tier { below: None, divisor: 1048576.0, unit: "MB".into(), decimals: 2 },
        ];
        c.items.push(Item { kind: "cpu".into(), label: "CPU ".into(), label_color: "auto".into(), value_color: "#ff0000".into(), graph_limit_mbit: Some(0.0) });
        let back = parse(&to_toml(&c)).unwrap();
        assert_eq!(back.general.interface, "Wi-Fi");
        assert_eq!(back.format.tiers.len(), 2);
        assert_eq!(back.format.tiers[1].below, None);
        assert_eq!(back.items.len(), 3);
        assert_eq!(back.items[2].kind, "cpu");
    }

    #[test]
    fn old_file_gets_graph_defaults() {
        let c = parse("[[items]]\nkind = \"down\"\n[[items]]\nkind = \"up\"\n[[items]]\nkind = \"cpu\"\n").unwrap();
        assert_eq!(c.items[0].graph_limit(), 1000.0);
        assert_eq!(c.items[1].graph_limit(), 500.0);
        assert_eq!(c.items[2].graph_limit(), 0.0);
    }

    #[test]
    fn colors() {
        assert_eq!(parse_color("#E5534B"), Some((0xE5, 0x53, 0x4B)));
        assert_eq!(parse_color("#fff"), Some((255, 255, 255)));
        assert_eq!(parse_color("auto"), None);
    }

    #[test]
    fn flip_bool() {
        let t = set_bool_in_text(DEFAULT_TOML, "general", "start_with_windows", false);
        let c = parse(&t).unwrap();
        assert!(!c.general.start_with_windows);
        assert!(t.contains("# adds/removes"));
        let t = set_string_in_text(&t, "general", "interface", "Wi-Fi");
        let c = parse(&t).unwrap();
        assert_eq!(c.general.interface, "Wi-Fi");
        assert!(t.contains("# \"\" = sum"));
        let t = set_bool_in_text("[general]\nx = 1\n[format]\nshort = false\n", "format", "bits", true);
        let c = parse(&t).unwrap();
        assert!(c.format.bits);
        let t = set_bool_in_text("[general]\nx = 1\n", "format", "short", true);
        assert!(parse(&t).unwrap().format.short);
    }
}
