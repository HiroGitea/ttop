//! Host (CPU / memory / network / disk / process) statistics from procfs.

use std::collections::HashMap;
use std::fs;
use std::time::Instant;

use crate::model::{HostStat, ProcStat};

#[derive(Clone, Copy, Default)]
struct CpuTimes {
    idle: u64,
    total: u64,
}

fn parse_cpu_line(line: &str) -> Option<CpuTimes> {
    let mut it = line.split_whitespace();
    let name = it.next()?;
    if !name.starts_with("cpu") {
        return None;
    }
    let vals: Vec<u64> = it.filter_map(|v| v.parse().ok()).collect();
    if vals.len() < 4 {
        return None;
    }
    let idle = vals[3] + vals.get(4).copied().unwrap_or(0);
    let total: u64 = vals.iter().sum();
    Some(CpuTimes { idle, total })
}

fn read_cpu() -> (CpuTimes, Vec<CpuTimes>) {
    let mut total = CpuTimes::default();
    let mut cores = vec![];
    if let Ok(s) = fs::read_to_string("/proc/stat") {
        for line in s.lines() {
            if line.starts_with("cpu ") {
                total = parse_cpu_line(line).unwrap_or_default();
            } else if line.starts_with("cpu") {
                if let Some(c) = parse_cpu_line(line) {
                    cores.push(c);
                }
            } else {
                break;
            }
        }
    }
    (total, cores)
}

fn cpu_pct(prev: CpuTimes, cur: CpuTimes) -> f64 {
    let dt = cur.total.saturating_sub(prev.total);
    if dt == 0 {
        return 0.0;
    }
    let di = cur.idle.saturating_sub(prev.idle);
    (1.0 - di as f64 / dt as f64).clamp(0.0, 1.0) * 100.0
}

fn read_meminfo() -> HashMap<String, u64> {
    let mut m = HashMap::new();
    if let Ok(s) = fs::read_to_string("/proc/meminfo") {
        for line in s.lines() {
            if let Some((k, v)) = line.split_once(':') {
                let kb: u64 = v
                    .trim()
                    .split_whitespace()
                    .next()
                    .and_then(|x| x.parse().ok())
                    .unwrap_or(0);
                m.insert(k.to_string(), kb * 1024);
            }
        }
    }
    m
}

fn read_net() -> (u64, u64) {
    let (mut rx, mut tx) = (0u64, 0u64);
    if let Ok(s) = fs::read_to_string("/proc/net/dev") {
        for line in s.lines().skip(2) {
            let Some((iface, rest)) = line.split_once(':') else {
                continue;
            };
            let iface = iface.trim();
            if iface == "lo" || iface.starts_with("docker") || iface.starts_with("veth") {
                continue;
            }
            let v: Vec<u64> = rest
                .split_whitespace()
                .filter_map(|x| x.parse().ok())
                .collect();
            if v.len() >= 9 {
                rx += v[0];
                tx += v[8];
            }
        }
    }
    (rx, tx)
}

fn read_disk() -> (u64, u64) {
    // Sum sectors read/written across whole block devices (skip partitions
    // and virtual devices like loop/ram).
    let (mut r, mut w) = (0u64, 0u64);
    if let Ok(s) = fs::read_to_string("/proc/diskstats") {
        for line in s.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 14 {
                continue;
            }
            let name = f[2];
            if name.starts_with("loop") || name.starts_with("ram") || name.starts_with("dm-") {
                continue;
            }
            let is_partition = name
                .chars()
                .last()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
                && !name.starts_with("nvme")
                && !name.starts_with("md");
            if is_partition {
                continue;
            }
            if name.starts_with("nvme") && name.contains('p') {
                continue;
            }
            r += f[5].parse::<u64>().unwrap_or(0);
            w += f[9].parse::<u64>().unwrap_or(0);
        }
    }
    (r * 512, w * 512)
}

#[derive(Default)]
struct ProcPrev {
    ticks: u64,
}

pub struct HostSampler {
    prev_total: CpuTimes,
    prev_cores: Vec<CpuTimes>,
    prev_net: (u64, u64),
    prev_disk: (u64, u64),
    prev_t: Instant,
    prev_procs: HashMap<u32, ProcPrev>,
    clk_tck: f64,
    page_size: u64,
    hostname: String,
}

impl HostSampler {
    pub fn new() -> Self {
        let (t, c) = read_cpu();
        let hostname = fs::read_to_string("/proc/sys/kernel/hostname")
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "localhost".into());
        Self {
            prev_total: t,
            prev_cores: c,
            prev_net: read_net(),
            prev_disk: read_disk(),
            prev_t: Instant::now(),
            prev_procs: HashMap::new(),
            clk_tck: 100.0,
            page_size: 4096,
            hostname,
        }
    }

    pub fn sample(&mut self) -> HostStat {
        let now = Instant::now();
        let dt = now.duration_since(self.prev_t).as_secs_f64().max(1e-3);
        let (t, c) = read_cpu();
        let cpu_total_pct = cpu_pct(self.prev_total, t);
        let cpu_per_core_pct = c
            .iter()
            .enumerate()
            .map(|(i, cur)| cpu_pct(self.prev_cores.get(i).copied().unwrap_or_default(), *cur))
            .collect();
        self.prev_total = t;
        self.prev_cores = c;

        let net = read_net();
        let net_rx_bps = net.0.saturating_sub(self.prev_net.0) as f64 / dt;
        let net_tx_bps = net.1.saturating_sub(self.prev_net.1) as f64 / dt;
        self.prev_net = net;

        let disk = read_disk();
        let disk_read_bps = disk.0.saturating_sub(self.prev_disk.0) as f64 / dt;
        let disk_write_bps = disk.1.saturating_sub(self.prev_disk.1) as f64 / dt;
        self.prev_disk = disk;
        self.prev_t = now;

        let mi = read_meminfo();
        let g = |k: &str| mi.get(k).copied().unwrap_or(0);
        let mem_total = g("MemTotal");
        let mem_available = g("MemAvailable");
        let swap_total = g("SwapTotal");
        let swap_used = swap_total.saturating_sub(g("SwapFree"));

        let load = fs::read_to_string("/proc/loadavg")
            .ok()
            .map(|s| {
                let v: Vec<f64> = s
                    .split_whitespace()
                    .take(3)
                    .filter_map(|x| x.parse().ok())
                    .collect();
                [
                    v.first().copied().unwrap_or(0.0),
                    v.get(1).copied().unwrap_or(0.0),
                    v.get(2).copied().unwrap_or(0.0),
                ]
            })
            .unwrap_or([0.0; 3]);
        let uptime_secs = fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
            .unwrap_or(0.0) as u64;

        HostStat {
            hostname: self.hostname.clone(),
            uptime_secs,
            load,
            cpu_total_pct,
            cpu_per_core_pct,
            mem_total,
            mem_used: mem_total.saturating_sub(mem_available),
            mem_available,
            swap_total,
            swap_used,
            net_rx_bps,
            net_tx_bps,
            disk_read_bps,
            disk_write_bps,
        }
    }

    /// Per-process stats for the given pids. `dt` must be the wall-clock
    /// seconds since the previous call for CPU% to be meaningful.
    pub fn processes(&mut self, pids: &[u32], dt_secs: f64) -> Vec<ProcStat> {
        let mut out = vec![];
        let uptime = fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
            .unwrap_or(0.0);
        let mut seen = HashMap::new();
        for &pid in pids {
            if seen.contains_key(&pid) {
                continue;
            }
            let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
                continue;
            };
            // comm may contain spaces; it is wrapped in parentheses.
            let Some(rp) = stat.rfind(')') else { continue };
            let name = stat
                .find('(')
                .map(|lp| stat[lp + 1..rp].to_string())
                .unwrap_or_default();
            let f: Vec<&str> = stat[rp + 1..].split_whitespace().collect();
            // after ')': state(0) ppid(1) ... utime(11) stime(12) ... starttime(19) vsize(20) rss(21)
            let utime: u64 = f.get(11).and_then(|x| x.parse().ok()).unwrap_or(0);
            let stime: u64 = f.get(12).and_then(|x| x.parse().ok()).unwrap_or(0);
            let starttime: u64 = f.get(19).and_then(|x| x.parse().ok()).unwrap_or(0);
            let rss_pages: u64 = f.get(21).and_then(|x| x.parse().ok()).unwrap_or(0);
            let ticks = utime + stime;
            let prev = self.prev_procs.get(&pid).map(|p| p.ticks);
            let cpu_pct = match prev {
                Some(p) if dt_secs > 0.0 => {
                    (ticks.saturating_sub(p) as f64 / self.clk_tck) / dt_secs * 100.0
                }
                _ => 0.0,
            };
            self.prev_procs.insert(pid, ProcPrev { ticks });
            let cmdline = fs::read(format!("/proc/{pid}/cmdline"))
                .map(|b| {
                    String::from_utf8_lossy(&b)
                        .split('\0')
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            let elapsed = (uptime - starttime as f64 / self.clk_tck).max(0.0) as u64;
            seen.insert(pid, ());
            out.push(ProcStat {
                pid,
                name,
                cmdline,
                cpu_pct,
                rss_bytes: rss_pages * self.page_size,
                devices: vec![],
                hbm_used: 0,
                elapsed_secs: elapsed,
            });
        }
        self.prev_procs.retain(|pid, _| seen.contains_key(pid));
        out
    }
}
