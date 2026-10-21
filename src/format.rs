//! Speed / size formatting, modelled on TrafficMonitor's `DataSizeToString`,
//! with an extra "B/s" tier and fully user-definable tiers.

use crate::config::{FormatCfg, Tier};

/// Builds the list of tiers that will actually be used for formatting.
pub fn effective_tiers(f: &FormatCfg) -> Vec<Tier> {
    if !f.tiers.is_empty() {
        return f.tiers.clone();
    }
    let k = f.kilo.max(2) as f64;
    let (b, kb, mb, gb) = if f.short {
        if f.bits { ("b", "Kb", "Mb", "Gb") } else { ("B", "K", "M", "G") }
    } else if f.bits {
        ("b", "Kb", "Mb", "Gb")
    } else {
        ("B", "KB", "MB", "GB")
    };
    let t = |below: Option<f64>, divisor: f64, unit: &str, decimals: u32| Tier {
        below,
        divisor,
        unit: unit.to_string(),
        decimals,
    };
    let (dk_small, dk_big, dm, dg) = if f.short { (1, 0, 1, 2) } else { (2, 1, 2, 2) };

    match f.unit.to_ascii_lowercase().as_str() {
        "b" => vec![t(None, 1.0, b, 0)],
        "kb" | "k" => vec![
            t(Some(10.0 * k), k, kb, dk_small),
            t(None, k, kb, dk_big),
        ],
        "mb" | "m" => vec![t(None, k * k, mb, dm)],
        "gb" | "g" => vec![t(None, k * k * k, gb, dg)],
        _ => {
            let mut v = Vec::new();
            if f.show_bytes_tier {
                v.push(t(Some(k), 1.0, b, 0));
            }
            v.push(t(Some(10.0 * k), k, kb, dk_small));
            v.push(t(Some(1000.0 * k), k, kb, dk_big));
            v.push(t(Some(1000.0 * k * k), k * k, mb, dm));
            v.push(t(None, k * k * k, gb, dg));
            v
        }
    }
}

/// Format a byte-per-second rate.
pub fn format_speed(bytes_per_sec: f64, f: &FormatCfg, tiers: &[Tier]) -> String {
    let v = if f.bits { bytes_per_sec * 8.0 } else { bytes_per_sec };
    let v = v.max(0.0);
    let tier = tiers
        .iter()
        .find(|t| t.below.map_or(true, |b| v < b))
        .or_else(|| tiers.last());
    let Some(tier) = tier else {
        return format!("{v:.0}");
    };
    let div = if tier.divisor <= 0.0 { 1.0 } else { tier.divisor };
    let num = format!("{:.*}", tier.decimals as usize, v / div);
    let num = if f.min_value_chars > 0 {
        format!("{:>w$}", num, w = f.min_value_chars as usize)
    } else {
        num
    };
    if f.hide_unit {
        return num;
    }
    let mut unit = tier.unit.clone();
    if f.per_second && !f.short {
        unit.push_str("/s");
    }
    if f.space && !unit.is_empty() {
        format!("{num} {unit}")
    } else {
        format!("{num}{unit}")
    }
}

/// Strings that represent the widest output each tier can produce;
/// used to keep the taskbar window at a stable width.
pub fn widest_samples(f: &FormatCfg, tiers: &[Tier]) -> Vec<String> {
    tiers
        .iter()
        .map(|t| {
            let div = if t.divisor <= 0.0 { 1.0 } else { t.divisor };
            // Largest value representable in this tier, rounded up the way {:.N} would.
            let top = t.below.map(|b| b / div).unwrap_or(100.0);
            let s = format_speed(top.min(9999.0) * div * 0.9999, f, std::slice::from_ref(t));
            // Every digit becomes '8' (usually the widest digit in proportional fonts).
            s.chars().map(|c| if c.is_ascii_digit() { '8' } else { c }).collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FormatCfg;

    fn fmt(v: f64, f: &FormatCfg) -> String {
        format_speed(v, f, &effective_tiers(f))
    }

    #[test]
    fn auto_tiers() {
        let f = FormatCfg::default();
        assert_eq!(fmt(0.0, &f), "0 B/s");
        assert_eq!(fmt(512.0, &f), "512 B/s");
        assert_eq!(fmt(9134.0, &f), "8.92 KB/s");
        assert_eq!(fmt(10650.0, &f), "10.4 KB/s");
        assert_eq!(fmt(5.5 * 1024.0 * 1024.0, &f), "5.50 MB/s");
        assert_eq!(fmt(2.0 * 1024.0 * 1024.0 * 1024.0, &f), "2.00 GB/s");
    }

    #[test]
    fn short_and_bits() {
        let f = FormatCfg { short: true, ..Default::default() };
        assert_eq!(fmt(9134.0, &f), "8.9 K");
        assert_eq!(fmt(500.0 * 1024.0, &f), "500 K");
        let f = FormatCfg { bits: true, ..Default::default() };
        assert_eq!(fmt(100.0, &f), "800 b/s");
        assert_eq!(fmt(1024.0 * 1024.0, &f), "8.00 Mb/s");
        let f = FormatCfg { space: false, ..Default::default() };
        assert_eq!(fmt(10650.0, &f), "10.4KB/s");
    }

    #[test]
    fn fixed_unit() {
        let f = FormatCfg { unit: "MB".into(), ..Default::default() };
        assert_eq!(fmt(1024.0 * 1024.0 * 3.0, &f), "3.00 MB/s");
        let f = FormatCfg { show_bytes_tier: false, ..Default::default() };
        assert_eq!(fmt(100.0, &f), "0.10 KB/s");
    }

    #[test]
    fn samples_have_digits_replaced() {
        let f = FormatCfg::default();
        let s = widest_samples(&f, &effective_tiers(&f));
        assert!(s.iter().any(|x| x == "888.8 KB/s"), "{s:?}");
        assert!(s.iter().any(|x| x == "8888 B/s"), "{s:?}");
    }
}
