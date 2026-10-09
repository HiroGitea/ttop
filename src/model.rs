//! Data model shared between the samplers, the TUI and the JSON output.

use serde::Serialize;

use crate::device::TpuChip;

#[derive(Clone, Debug, Default, Serialize)]
pub struct HloTiming {
    pub mean_us: f64,
    pub p50_us: f64,
    pub p90_us: f64,
    pub p95_us: f64,
    pub p999_us: f64,
    pub count: i64,
}

/// One logical TPU device (TensorCore) as seen by libtpu.
#[derive(Clone, Debug, Default, Serialize)]
pub struct DeviceStat {
    pub device_id: usize,
    pub path: String,
    pub pid: Option<u32>,
    pub process_name: Option<String>,
    pub hbm_used: Option<u64>,
    pub hbm_total: Option<u64>,
    /// TensorCore duty cycle, percent.
    pub duty_cycle: Option<f64>,
    /// HBM bandwidth utilisation, percent.
    pub hbm_bw_util: Option<f64>,
    /// Seconds the TensorCore has been idle.
    pub tc_idle_secs: Option<f64>,
    pub hlo_queue_size: Option<i64>,
    pub hlo_timing: Option<HloTiming>,
}

impl DeviceStat {
    pub fn hbm_pct(&self) -> Option<f64> {
        match (self.hbm_used, self.hbm_total) {
            (Some(u), Some(t)) if t > 0 => Some(u as f64 / t as f64 * 100.0),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ProcStat {
    pub pid: u32,
    pub name: String,
    pub cmdline: String,
    pub cpu_pct: f64,
    pub rss_bytes: u64,
    pub devices: Vec<usize>,
    pub hbm_used: u64,
    pub elapsed_secs: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct HostStat {
    pub hostname: String,
    pub uptime_secs: u64,
    pub load: [f64; 3],
    pub cpu_total_pct: f64,
    pub cpu_per_core_pct: Vec<f64>,
    pub mem_total: u64,
    pub mem_used: u64,
    pub mem_available: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub net_rx_bps: f64,
    pub net_tx_bps: f64,
    pub disk_read_bps: f64,
    pub disk_write_bps: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum RuntimeState {
    /// libtpu metrics server reachable.
    Connected,
    /// No metrics server: no framework currently using the TPU.
    Unavailable,
    /// Some other error talking to the server.
    Error,
}

impl Default for RuntimeState {
    fn default() -> Self {
        RuntimeState::Unavailable
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Snapshot {
    pub timestamp: String,
    pub chip: Option<TpuChip>,
    pub chip_name: String,
    pub chip_count: usize,
    pub topology: String,
    pub libtpu_version: Option<String>,
    pub metrics_ports: Vec<u16>,
    pub runtime: RuntimeState,
    pub runtime_message: Option<String>,
    pub devices: Vec<DeviceStat>,
    pub processes: Vec<ProcStat>,
    pub host: HostStat,
    pub sample_ms: u64,
}

impl Snapshot {
    pub fn total_hbm_used(&self) -> u64 {
        self.devices.iter().filter_map(|d| d.hbm_used).sum()
    }
    pub fn total_hbm(&self) -> u64 {
        self.devices.iter().filter_map(|d| d.hbm_total).sum()
    }
    pub fn avg_duty(&self) -> Option<f64> {
        let v: Vec<f64> = self.devices.iter().filter_map(|d| d.duty_cycle).collect();
        if v.is_empty() {
            None
        } else {
            Some(v.iter().sum::<f64>() / v.len() as f64)
        }
    }
}
