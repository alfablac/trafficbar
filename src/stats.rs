//! Network / CPU / memory sampling.

use std::collections::HashMap;
use std::time::Instant;
use windows_sys::Win32::Foundation::{FILETIME, NO_ERROR};
use windows_sys::Win32::NetworkManagement::IpHelper::*;
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::GetSystemTimes;

const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
const IF_TYPE_TUNNEL: u32 = 131;
const OPER_UP: i32 = 1;

#[derive(Clone, Debug)]
pub struct Adapter {
    pub alias: String,
    pub description: String,
    pub hardware: bool,
}

fn u16z(s: &[u16]) -> String {
    let n = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(&s[..n])
}

struct Row {
    luid: u64,
    adapter: Adapter,
    filter: bool,
    up: bool,
    loopback: bool,
    tunnel: bool,
    rx: u64,
    tx: u64,
}

fn read_rows() -> Vec<Row> {
    let mut out = Vec::new();
    unsafe {
        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut table) != NO_ERROR || table.is_null() {
            return out;
        }
        let n = (*table).NumEntries as usize;
        let rows = std::slice::from_raw_parts((*table).Table.as_ptr(), n);
        for r in rows {
            let flags = r.InterfaceAndOperStatusFlags._bitfield;
            out.push(Row {
                luid: r.InterfaceLuid.Value,
                adapter: Adapter {
                    alias: u16z(&r.Alias),
                    description: u16z(&r.Description),
                    hardware: flags & 0x1 != 0,
                },
                filter: flags & 0x2 != 0,
                up: r.OperStatus == OPER_UP,
                loopback: r.Type == IF_TYPE_SOFTWARE_LOOPBACK,
                tunnel: r.Type == IF_TYPE_TUNNEL,
                rx: r.InOctets,
                tx: r.OutOctets,
            });
        }
        FreeMibTable(table as *const _);
    }
    out
}

/// Connected adapters a user might want to pick from (for the menu).
pub fn list_adapters() -> Vec<Adapter> {
    let mut v: Vec<Adapter> = read_rows()
        .into_iter()
        .filter(|r| r.up && !r.loopback && !r.filter && !r.tunnel)
        .map(|r| r.adapter)
        .collect();
    v.sort_by(|a, b| b.hardware.cmp(&a.hardware).then(a.alias.cmp(&b.alias)));
    v.dedup_by(|a, b| a.alias == b.alias);
    v
}

pub struct NetSampler {
    last: HashMap<u64, (u64, u64)>,
    last_t: Option<Instant>,
    pub down: f64,
    pub up: f64,
    pub total_rx: u64,
    pub total_tx: u64,
}

impl NetSampler {
    pub fn new() -> Self {
        Self { last: HashMap::new(), last_t: None, down: 0.0, up: 0.0, total_rx: 0, total_tx: 0 }
    }

    pub fn reset(&mut self) {
        self.last.clear();
        self.last_t = None;
        self.down = 0.0;
        self.up = 0.0;
    }

    pub fn sample(&mut self, iface: &str, include_virtual: bool, smoothing: f64) {
        let needle = iface.trim().to_lowercase();
        let rows = read_rows();
        let selected = rows.into_iter().filter(|r| {
            if r.loopback || r.filter || !r.up {
                return false;
            }
            if needle.is_empty() {
                r.adapter.hardware || include_virtual
            } else {
                r.adapter.alias.to_lowercase().contains(&needle)
                    || r.adapter.description.to_lowercase().contains(&needle)
            }
        });

        let now = Instant::now();
        let mut cur = HashMap::new();
        let (mut drx, mut dtx) = (0u64, 0u64);
        for r in selected {
            if let Some(&(orx, otx)) = self.last.get(&r.luid) {
                drx += r.rx.saturating_sub(orx);
                dtx += r.tx.saturating_sub(otx);
            }
            cur.insert(r.luid, (r.rx, r.tx));
        }
        if let Some(t) = self.last_t {
            let dt = now.duration_since(t).as_secs_f64().max(0.001);
            let (d, u) = (drx as f64 / dt, dtx as f64 / dt);
            let a = smoothing;
            self.down = if a > 0.0 { self.down * a + d * (1.0 - a) } else { d };
            self.up = if a > 0.0 { self.up * a + u * (1.0 - a) } else { u };
            self.total_rx += drx;
            self.total_tx += dtx;
        }
        self.last = cur;
        self.last_t = Some(now);
    }
}

fn ft(f: &FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

pub struct CpuSampler {
    last: Option<(u64, u64, u64)>,
    pub usage: f64,
}

impl CpuSampler {
    pub fn new() -> Self {
        Self { last: None, usage: 0.0 }
    }
    pub fn sample(&mut self) {
        let mut i = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut k, mut u) = (i, i);
        if unsafe { GetSystemTimes(&mut i, &mut k, &mut u) } == 0 {
            return;
        }
        let cur = (ft(&i), ft(&k), ft(&u));
        if let Some((li, lk, lu)) = self.last {
            let idle = cur.0.saturating_sub(li) as f64;
            let total = (cur.1.saturating_sub(lk) + cur.2.saturating_sub(lu)) as f64; // kernel includes idle
            if total > 0.0 {
                self.usage = ((total - idle) / total * 100.0).clamp(0.0, 100.0);
            }
        }
        self.last = Some(cur);
    }
}

pub fn memory_usage() -> f64 {
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    if unsafe { GlobalMemoryStatusEx(&mut m) } == 0 || m.ullTotalPhys == 0 {
        return 0.0;
    }
    (m.ullTotalPhys - m.ullAvailPhys) as f64 / m.ullTotalPhys as f64 * 100.0
}
