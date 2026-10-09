//! Samplers: the real one (PCI + libtpu gRPC + procfs) and a mock for demos.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::Local;

use crate::device::{self, TpuChip};
use crate::grpc::RuntimeMetricClient;
use crate::model::*;
use crate::proto::Metric;

pub const M_HBM_TOTAL: &str = "tpu.runtime.hbm.memory.total.bytes";
pub const M_HBM_USAGE: &str = "tpu.runtime.hbm.memory.usage.bytes";
pub const M_DUTY_CYCLE: &str = "tpu.runtime.tensorcore.dutycycle.percent";
pub const M_HBM_BW_UTIL: &str = "tpu.runtime.hbm.utilization.percent";
pub const M_TC_IDLE: &str = "tpu.runtime.tensorcore.idle_duration.seconds";
pub const M_HLO_QUEUE: &str = "hlo.queue.size.gauge";
pub const M_HLO_TIMING: &str = "hlo.execution.timing.distribution.microseconds";

pub trait Backend: Send {
    fn sample(&mut self) -> Snapshot;
}

/// Percentile estimate from an exponential-bucket distribution, matching
/// `tpu_info.metrics._get_percentile`.
pub fn percentile(pct: f64, count: i64, buckets: &[i64], scale: f64, growth: f64) -> f64 {
    if count <= 0 {
        return 1.0;
    }
    let target = (count as f64 * pct) as i64;
    let mut remaining = count;
    for i in (1..buckets.len()).rev() {
        remaining -= buckets[i];
        if remaining <= target {
            let delta = (target - remaining) as f64;
            let lower = scale * growth.powi(i as i32 - 1);
            let b = buckets[i].max(1) as f64;
            return lower * (1.0 + (delta / b) * (growth - 1.0));
        }
    }
    1.0
}

fn hlo_timing_from(m: &Metric) -> Option<HloTiming> {
    let d = m.distribution()?;
    let (scale, growth) = match d.bucket_options.as_ref().and_then(|b| b.options.as_ref()) {
        Some(crate::proto::bucket_options::Options::ExponentialBuckets(e)) => {
            (e.scale, e.growth_factor)
        }
        _ => (1.0, 2.0),
    };
    let p = |q: f64| percentile(q, d.count, &d.bucket_counts, scale, growth);
    Some(HloTiming {
        mean_us: d.mean,
        p50_us: p(0.5),
        p90_us: p(0.9),
        p95_us: p(0.95),
        p999_us: p(0.999),
        count: d.count,
    })
}

struct RuntimeMetrics {
    hbm_total: Vec<(i64, u64)>,
    hbm_usage: Vec<(i64, u64)>,
    duty: Vec<(i64, f64)>,
    bw: Vec<(i64, f64)>,
    idle: Vec<(i64, f64)>,
    queue: Vec<(i64, i64)>,
    timing: Vec<(i64, HloTiming)>,
}

async fn fetch_runtime(c: &mut RuntimeMetricClient) -> Result<RuntimeMetrics> {
    let sorted_f64 = |ms: Vec<Metric>| -> Vec<(i64, f64)> {
        let mut v: Vec<(i64, f64)> = ms
            .iter()
            .filter_map(|m| Some((m.device_id()?, m.gauge_f64()?)))
            .collect();
        v.sort_by_key(|x| x.0);
        v
    };
    // Required metrics: errors propagate (server unavailable etc).
    let hbm_total = {
        let mut v: Vec<(i64, u64)> = c
            .get_runtime_metric(M_HBM_TOTAL)
            .await?
            .iter()
            .filter_map(|m| Some((m.device_id()?, m.gauge_i64()?.max(0) as u64)))
            .collect();
        v.sort_by_key(|x| x.0);
        v
    };
    let hbm_usage = {
        let mut v: Vec<(i64, u64)> = c
            .get_runtime_metric(M_HBM_USAGE)
            .await?
            .iter()
            .filter_map(|m| Some((m.device_id()?, m.gauge_i64()?.max(0) as u64)))
            .collect();
        v.sort_by_key(|x| x.0);
        v
    };
    let duty = sorted_f64(c.get_runtime_metric(M_DUTY_CYCLE).await?);
    // Optional metrics: missing on older libtpu, ignore errors.
    let bw = sorted_f64(c.get_runtime_metric(M_HBM_BW_UTIL).await.unwrap_or_default());
    let idle = sorted_f64(c.get_runtime_metric(M_TC_IDLE).await.unwrap_or_default());
    let queue = {
        let mut v: Vec<(i64, i64)> = c
            .get_runtime_metric(M_HLO_QUEUE)
            .await
            .unwrap_or_default()
            .iter()
            .filter_map(|m| Some((m.device_id()?, m.gauge_i64()?)))
            .collect();
        v.sort_by_key(|x| x.0);
        v
    };
    let timing = {
        let mut v: Vec<(i64, HloTiming)> = c
            .get_runtime_metric(M_HLO_TIMING)
            .await
            .unwrap_or_default()
            .iter()
            .filter_map(|m| Some((m.device_id()?, hlo_timing_from(m)?)))
            .collect();
        v.sort_by_key(|x| x.0);
        v
    };
    Ok(RuntimeMetrics {
        hbm_total,
        hbm_usage,
        duty,
        bw,
        idle,
        queue,
        timing,
    })
}

pub struct RealBackend {
    rt: tokio::runtime::Runtime,
    host: crate::host::HostSampler,
    ports: Vec<u16>,
    timeout: Duration,
    chip: Option<TpuChip>,
    count: usize,
    chips: Vec<device::ChipInfo>,
    libtpu_version: Option<String>,
    last: Instant,
    clients: HashMap<u16, RuntimeMetricClient>,
}

impl RealBackend {
    pub fn new(ports: Vec<u16>, timeout: Duration, chip_override: Option<TpuChip>) -> Result<Self> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let (mut chip, mut count) = device::get_local_chips();
        if let Some(c) = chip_override {
            chip = Some(c);
            if count == 0 {
                count = 1;
            }
        }
        Ok(Self {
            rt,
            host: crate::host::HostSampler::new(),
            ports,
            timeout,
            chip,
            count,
            chips: device::get_chips(),
            libtpu_version: detect_libtpu_version(),
            last: Instant::now(),
            clients: HashMap::new(),
        })
    }

    pub fn list_metrics(&self) -> Result<Vec<String>> {
        let port = self.ports[0];
        let timeout = self.timeout;
        self.rt.block_on(async move {
            let mut c = RuntimeMetricClient::connect(port, timeout).await?;
            c.list_supported_metrics().await
        })
    }
}

fn detect_libtpu_version() -> Option<String> {
    // Best effort: look for an installed libtpu python package metadata.
    let home = std::env::var("HOME").unwrap_or_default();
    let mut candidates = vec![];
    for root in [
        "/usr/lib/python3/dist-packages",
        "/usr/local/lib/python3.10/dist-packages",
        "/usr/local/lib/python3.11/dist-packages",
        "/usr/local/lib/python3.12/dist-packages",
    ] {
        candidates.push(root.to_string());
    }
    for v in ["3.10", "3.11", "3.12", "3.13"] {
        candidates.push(format!("{home}/.local/lib/python{v}/site-packages"));
    }
    if let Ok(venv) = std::env::var("VIRTUAL_ENV") {
        for v in ["3.10", "3.11", "3.12", "3.13"] {
            candidates.push(format!("{venv}/lib/python{v}/site-packages"));
        }
    }
    for root in candidates {
        let Ok(rd) = std::fs::read_dir(&root) else { continue };
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if let Some(rest) = n.strip_prefix("libtpu-") {
                if let Some(ver) = rest.strip_suffix(".dist-info") {
                    return Some(ver.to_string());
                }
            }
        }
    }
    None
}

impl Backend for RealBackend {
    fn sample(&mut self) -> Snapshot {
        let t0 = Instant::now();
        let dt = t0.duration_since(self.last).as_secs_f64();
        self.last = t0;

        let chip = self.chip;
        let count = self.count;
        // Skeleton device count, used when libtpu is not reachable.  v2/v3
        // expose one PCI function per chip with two TensorCores; newer chips
        // expose one PCI function per logical device.  Once libtpu answers,
        // its device list is authoritative.
        let n_devices = match chip {
            Some(TpuChip::V2) | Some(TpuChip::V3) => count * 2,
            _ => count,
        };

        // ---- devices skeleton from PCI ----
        let owners = device::get_chip_owners();
        let mut devices: Vec<DeviceStat> = (0..n_devices)
            .map(|i| {
                let (pid, path) = match chip {
                    Some(c) => device::owner_for_device(i, c, &self.chips, &owners),
                    None => (None, format!("device{i}")),
                };
                DeviceStat {
                    device_id: i,
                    path,
                    pid,
                    ..Default::default()
                }
            })
            .collect();

        // ---- libtpu runtime metrics ----
        let mut runtime = RuntimeState::Unavailable;
        let mut runtime_message = None;
        let mut any_ok = false;
        let mut last_err: Option<String> = None;
        let mut reported_devices = 0usize;
        for &port in &self.ports {
            let timeout = self.timeout;
            let rt = &self.rt;
            let clients = &mut self.clients;
            let res = rt.block_on(async {
                if !clients.contains_key(&port) {
                    let c = RuntimeMetricClient::connect(port, timeout).await?;
                    clients.insert(port, c);
                }
                fetch_runtime(clients.get_mut(&port).expect("client inserted")).await
            });
            match res {
                Ok(rm) => {
                    any_ok = true;
                    reported_devices = reported_devices.max(merge_runtime(&mut devices, rm, chip));
                }
                Err(e) => {
                    self.clients.remove(&port);
                    last_err = Some(format!("{e:#}"));
                }
            }
        }
        if any_ok && reported_devices > 0 {
            devices.truncate(reported_devices);
        }
        if any_ok {
            runtime = RuntimeState::Connected;
        } else if let Some(e) = last_err {
            let lower = e.to_lowercase();
            if lower.contains("connect") || lower.contains("refused") || lower.contains("unavailable")
            {
                runtime = RuntimeState::Unavailable;
                runtime_message = Some(
                    "libtpu metrics unavailable: no framework (JAX/PyTorch) is using the TPU"
                        .to_string(),
                );
            } else {
                runtime = RuntimeState::Error;
                runtime_message = Some(e);
            }
        }

        // ---- host + processes ----
        let host = self.host.sample();
        let pids: Vec<u32> = devices.iter().filter_map(|d| d.pid).collect();
        let mut processes = self.host.processes(&pids, dt);
        for p in &mut processes {
            for d in &devices {
                if d.pid == Some(p.pid) {
                    p.devices.push(d.device_id);
                    p.hbm_used += d.hbm_used.unwrap_or(0);
                }
            }
        }
        let pid_names: HashMap<u32, String> =
            processes.iter().map(|p| (p.pid, p.name.clone())).collect();
        for d in &mut devices {
            if let Some(pid) = d.pid {
                d.process_name = pid_names.get(&pid).cloned();
            }
        }

        Snapshot {
            timestamp: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            chip,
            chip_name: chip.map(|c| c.display()).unwrap_or_else(|| "no TPU".into()),
            chip_count: if !self.chips.is_empty() {
                self.chips.len()
            } else if any_ok {
                // No PCI info (e.g. --chip override): derive from libtpu.
                devices.len().div_ceil(chip.map(|c| c.devices_per_chip()).unwrap_or(1))
            } else {
                count
            },
            topology: device::accelerator_topology(chip, count),
            libtpu_version: self.libtpu_version.clone(),
            metrics_ports: self.ports.clone(),
            runtime,
            runtime_message,
            devices,
            processes,
            host,
            sample_ms: t0.elapsed().as_millis() as u64,
        }
    }
}

/// Merge gRPC metric vectors into the device list. Memory is reported per
/// logical device; duty cycle may be reported per chip (v2/v3), in which case
/// it is repeated across the chip's devices like tpu-info does.
///
/// Returns the number of devices libtpu reported (max device id + 1).
fn merge_runtime(devices: &mut Vec<DeviceStat>, rm: RuntimeMetrics, chip: Option<TpuChip>) -> usize {
    // Make sure the device list is at least as long as what libtpu reports.
    let max_id = rm
        .hbm_usage
        .iter()
        .chain(rm.hbm_total.iter())
        .map(|x| x.0)
        .max()
        .unwrap_or(-1);
    while (devices.len() as i64) <= max_id {
        let i = devices.len();
        devices.push(DeviceStat {
            device_id: i,
            path: chip.map(|c| c.chip_path(i)).unwrap_or_else(|| format!("device{i}")),
            ..Default::default()
        });
    }
    let set = |devices: &mut Vec<DeviceStat>, id: i64, f: &dyn Fn(&mut DeviceStat)| {
        if id >= 0 {
            if let Some(d) = devices.get_mut(id as usize) {
                f(d);
            }
        }
    };
    for (id, v) in &rm.hbm_total {
        set(devices, *id, &|d| d.hbm_total = Some(*v));
    }
    for (id, v) in &rm.hbm_usage {
        set(devices, *id, &|d| d.hbm_used = Some(*v));
    }
    let n_mem = rm.hbm_usage.len();
    let n_duty = rm.duty.len();
    if n_duty > 0 && n_mem > n_duty && n_mem % n_duty == 0 {
        // per-chip duty cycle -> repeat over the chip's devices
        let k = n_mem / n_duty;
        for (ci, (_, v)) in rm.duty.iter().enumerate() {
            for j in 0..k {
                let id = (ci * k + j) as i64;
                set(devices, id, &|d| d.duty_cycle = Some(*v));
            }
        }
    } else {
        for (id, v) in &rm.duty {
            set(devices, *id, &|d| d.duty_cycle = Some(*v));
        }
    }
    for (id, v) in &rm.bw {
        set(devices, *id, &|d| d.hbm_bw_util = Some(*v));
    }
    for (id, v) in &rm.idle {
        set(devices, *id, &|d| d.tc_idle_secs = Some(*v));
    }
    for (id, v) in &rm.queue {
        set(devices, *id, &|d| d.hlo_queue_size = Some(*v));
    }
    for (id, v) in &rm.timing {
        set(devices, *id, &|d| d.hlo_timing = Some(v.clone()));
    }
    (max_id + 1).max(0) as usize
}

// ---------------------------------------------------------------------------
// Mock backend for `--demo`
// ---------------------------------------------------------------------------

pub struct MockBackend {
    chip: TpuChip,
    n: usize,
    t: f64,
    host: crate::host::HostSampler,
    hbm: Vec<f64>,
    rng: rand::rngs::StdRng,
    start: Instant,
}

impl MockBackend {
    pub fn new(chip: TpuChip, n: usize) -> Self {
        use rand::SeedableRng;
        Self {
            chip,
            n,
            t: 0.0,
            host: crate::host::HostSampler::new(),
            hbm: (0..n).map(|i| 0.35 + 0.05 * (i % 3) as f64).collect(),
            rng: rand::rngs::StdRng::seed_from_u64(42),
            start: Instant::now(),
        }
    }
}

impl Backend for MockBackend {
    fn sample(&mut self) -> Snapshot {
        use rand::Rng;
        self.t += 1.0;
        let total = self.chip.hbm_gib() * 1024 * 1024 * 1024;
        let mut devices = vec![];
        for i in 0..self.n {
            let phase = i as f64 * 0.7;
            let duty = (55.0
                + 35.0 * ((self.t / 9.0 + phase).sin())
                + self.rng.gen_range(-6.0..6.0))
            .clamp(0.0, 100.0);
            self.hbm[i] = (self.hbm[i] + self.rng.gen_range(-0.015..0.02)).clamp(0.05, 0.97);
            let used = (total as f64 * self.hbm[i]) as u64;
            let bw = (duty * 0.75 + self.rng.gen_range(-8.0..8.0)).clamp(0.0, 100.0);
            let pid = 4242 + (i / 4) as u32;
            devices.push(DeviceStat {
                device_id: i,
                path: self.chip.chip_path(i),
                pid: Some(pid),
                process_name: Some("python3".into()),
                hbm_used: Some(used),
                hbm_total: Some(total),
                duty_cycle: Some(duty),
                hbm_bw_util: Some(bw),
                tc_idle_secs: Some(self.rng.gen_range(0.0..0.5)),
                hlo_queue_size: Some(self.rng.gen_range(0..6)),
                hlo_timing: Some(HloTiming {
                    mean_us: 1850.0 + self.rng.gen_range(-200.0..200.0),
                    p50_us: 1700.0,
                    p90_us: 2600.0,
                    p95_us: 3100.0,
                    p999_us: 9800.0,
                    count: 1000 + self.t as i64 * 7,
                }),
            });
        }
        let host = self.host.sample();
        let mut processes = vec![];
        let nproc = (self.n + 3) / 4;
        for p in 0..nproc {
            let devs: Vec<usize> = (0..self.n).filter(|i| i / 4 == p).collect();
            processes.push(ProcStat {
                pid: 4242 + p as u32,
                name: "python3".into(),
                cmdline: format!("python3 train.py --model llama3-8b --shard {p}"),
                cpu_pct: 120.0 + self.rng.gen_range(-30.0..60.0),
                rss_bytes: 6 * 1024 * 1024 * 1024 + self.rng.gen_range(0..500_000_000),
                hbm_used: devs.iter().map(|&i| devices[i].hbm_used.unwrap_or(0)).sum(),
                devices: devs,
                elapsed_secs: self.start.elapsed().as_secs() + 3723,
            });
        }
        Snapshot {
            timestamp: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            chip: Some(self.chip),
            chip_name: self.chip.display(),
            chip_count: self.n / self.chip.devices_per_chip().max(1),
            topology: format!("{}x{} (demo)", self.n / self.chip.devices_per_chip().max(1), self.chip.name()),
            libtpu_version: Some("0.0.0-demo".into()),
            metrics_ports: vec![8431],
            runtime: RuntimeState::Connected,
            runtime_message: None,
            devices,
            processes,
            host,
            sample_ms: 3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::*;

    fn simple(id: i64, v: gauge::Value) -> Metric {
        Metric {
            attribute: Some(Attribute {
                key: "device-id".into(),
                value: Some(AttrValue { attr: Some(attr_value::Attr::IntAttr(id)) }),
            }),
            measure: Some(metric::Measure::Gauge(Gauge { value: Some(v) })),
        }
    }

    #[test]
    fn percentile_matches_tpu_info() {
        // Same algorithm as tpu_info.metrics._get_percentile.
        let buckets = [0, 10, 20, 30, 40];
        let count = 100;
        // p50 -> target 50: remaining 100-40=60 >50, 60-30=30 <=50 at i=3
        // lower = 1 * 2^(2) = 4 ; delta = 20 ; 4*(1+20/30*1) = 6.666..
        let p = percentile(0.5, count, &buckets, 1.0, 2.0);
        assert!((p - 6.666_666).abs() < 1e-3, "{p}");
        assert_eq!(percentile(0.5, 0, &buckets, 1.0, 2.0), 1.0);
    }

    #[test]
    fn protobuf_roundtrip_and_device_ids() {
        use prost::Message;
        let resp = MetricResponse {
            metric: Some(TpuMetric {
                name: M_HBM_USAGE.into(),
                description: String::new(),
                metrics: vec![simple(1, gauge::Value::AsInt(42)), simple(0, gauge::Value::AsInt(7))],
            }),
        };
        let bytes = resp.encode_to_vec();
        let back = MetricResponse::decode(bytes.as_slice()).unwrap();
        let ms = back.metric.unwrap().metrics;
        assert_eq!(ms[0].device_id(), Some(1));
        assert_eq!(ms[0].gauge_i64(), Some(42));

        let kv = Metric {
            attribute: Some(Attribute {
                key: "".into(),
                value: Some(AttrValue {
                    attr: Some(attr_value::Attr::KvlistAttr(KeyValueList {
                        attributes: vec![Attribute {
                            key: "device_ordinal".into(),
                            value: Some(AttrValue { attr: Some(attr_value::Attr::StringAttr("3".into())) }),
                        }],
                    })),
                }),
            }),
            measure: None,
        };
        assert_eq!(kv.device_id(), Some(3));
    }

    #[test]
    fn per_chip_duty_cycle_is_repeated() {
        let mut devs = vec![];
        let rm = RuntimeMetrics {
            hbm_total: vec![(0, 10), (1, 10), (2, 10), (3, 10)],
            hbm_usage: vec![(0, 1), (1, 2), (2, 3), (3, 4)],
            duty: vec![(0, 50.0), (1, 80.0)],
            bw: vec![],
            idle: vec![],
            queue: vec![],
            timing: vec![],
        };
        let n = merge_runtime(&mut devs, rm, Some(TpuChip::V3));
        assert_eq!(n, 4);
        let d: Vec<f64> = devs.iter().map(|d| d.duty_cycle.unwrap()).collect();
        assert_eq!(d, vec![50.0, 50.0, 80.0, 80.0]);
        assert_eq!(devs[3].hbm_used, Some(4));
    }
}
