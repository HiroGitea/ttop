//! Local TPU discovery via PCI sysfs, mirroring `tpu_info.device`.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use serde::Serialize;

pub const GOOGLE_PCI_VENDOR_ID: &str = "0x1ae0";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub enum TpuChip {
    V2,
    V3,
    V4,
    V5E,
    V5P,
    V6E,
    V7X,
}

impl TpuChip {
    pub fn name(self) -> &'static str {
        match self {
            TpuChip::V2 => "v2",
            TpuChip::V3 => "v3",
            TpuChip::V4 => "v4",
            TpuChip::V5E => "v5e",
            TpuChip::V5P => "v5p",
            TpuChip::V6E => "v6e",
            TpuChip::V7X => "7x",
        }
    }

    pub fn display(self) -> String {
        match self {
            TpuChip::V7X => "TPU7x".to_string(),
            c => format!("TPU {}", c.name()),
        }
    }

    pub fn hbm_gib(self) -> u64 {
        match self {
            TpuChip::V2 => 8,
            TpuChip::V3 => 16,
            TpuChip::V4 => 32,
            TpuChip::V5E => 16,
            TpuChip::V5P => 95,
            TpuChip::V6E => 32,
            TpuChip::V7X => 192,
        }
    }

    /// Number of logical devices (TensorCores exposed to the runtime) per chip.
    pub fn devices_per_chip(self) -> usize {
        match self {
            TpuChip::V2 | TpuChip::V3 | TpuChip::V7X => 2,
            _ => 1,
        }
    }

    pub fn from_pci_ids(device_id: &str, subsystem_id: &str) -> Option<Self> {
        match device_id {
            "0x0027" => match subsystem_id {
                "0x004e" => Some(TpuChip::V2),
                "0x004f" => Some(TpuChip::V3),
                _ => None,
            },
            "0x005e" => Some(TpuChip::V4),
            "0x0063" => Some(TpuChip::V5E),
            "0x0062" => Some(TpuChip::V5P),
            "0x006f" => Some(TpuChip::V6E),
            "0x0076" => Some(TpuChip::V7X),
            _ => None,
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "v2" => Some(TpuChip::V2),
            "v3" => Some(TpuChip::V3),
            "v4" => Some(TpuChip::V4),
            "v5e" | "v5litepod" => Some(TpuChip::V5E),
            "v5p" => Some(TpuChip::V5P),
            "v6e" => Some(TpuChip::V6E),
            "7x" | "v7x" => Some(TpuChip::V7X),
            _ => None,
        }
    }

    /// Expected `/dev` path for logical device `index`.
    pub fn chip_path(self, index: usize) -> String {
        match self {
            TpuChip::V5E | TpuChip::V5P | TpuChip::V6E | TpuChip::V7X => {
                format!("/dev/vfio/{index}")
            }
            _ => format!("/dev/accel{index}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CoreInfo {
    pub core_index: usize,
    pub device_id: String,
    pub vfio_path: String,
    pub full_addr: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ChipInfo {
    pub base_addr: String,
    pub cores: BTreeMap<usize, CoreInfo>,
}

impl ChipInfo {
    pub fn representative_core(&self) -> Option<&CoreInfo> {
        self.cores.get(&0).or_else(|| self.cores.values().next())
    }
}

fn read_trim(p: &Path) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

/// Returns `(chip type, count)` of Google TPU PCI functions, like
/// `tpu_info.device.get_local_chips`.
pub fn get_local_chips() -> (Option<TpuChip>, usize) {
    let mut count: HashMap<TpuChip, usize> = HashMap::new();
    let Ok(rd) = fs::read_dir("/sys/bus/pci/devices") else {
        return (None, 0);
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if read_trim(&p.join("vendor")).as_deref() != Some(GOOGLE_PCI_VENDOR_ID) {
            continue;
        }
        let dev = read_trim(&p.join("device")).unwrap_or_default();
        let sub = read_trim(&p.join("subsystem_device")).unwrap_or_default();
        if let Some(c) = TpuChip::from_pci_ids(&dev, &sub) {
            *count.entry(c).or_default() += 1;
        }
    }
    count
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(c, n)| (Some(c), n))
        .unwrap_or((None, 0))
}

/// Chips grouped by PCI base address with per-core VFIO paths.
pub fn get_chips() -> Vec<ChipInfo> {
    let mut by_base: BTreeMap<String, ChipInfo> = BTreeMap::new();
    let Ok(rd) = fs::read_dir("/sys/bus/pci/devices") else {
        return vec![];
    };
    for entry in rd.flatten() {
        let p = entry.path();
        let pci_addr = entry.file_name().to_string_lossy().to_string();
        if read_trim(&p.join("vendor")).as_deref() != Some(GOOGLE_PCI_VENDOR_ID) {
            continue;
        }
        let Ok(link) = fs::read_link(p.join("iommu_group")) else {
            continue;
        };
        let iommu_group = link
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let device_id = read_trim(&p.join("device")).unwrap_or_default();
        let (base, func) = match pci_addr.rsplit_once('.') {
            Some((b, f)) => (b.to_string(), f.parse::<usize>().unwrap_or(0)),
            None => (pci_addr.clone(), 0),
        };
        let chip = by_base.entry(base.clone()).or_insert_with(|| ChipInfo {
            base_addr: base.clone(),
            cores: BTreeMap::new(),
        });
        chip.cores.insert(
            func,
            CoreInfo {
                core_index: func,
                device_id,
                vfio_path: format!("/dev/vfio/{iommu_group}"),
                full_addr: pci_addr,
            },
        );
    }
    let mut chips: Vec<ChipInfo> = by_base.into_values().collect();
    chips.sort_by_key(|c| {
        c.representative_core()
            .and_then(|k| natural_key(&k.vfio_path))
            .unwrap_or(u64::MAX)
    });
    chips
}

fn natural_key(path: &str) -> Option<u64> {
    path.rsplit('/').next()?.parse().ok()
}

/// Map of `/dev/accelN` or `/dev/vfio/N` -> pid of the process holding it open.
pub fn get_chip_owners() -> HashMap<String, u32> {
    let mut owners = HashMap::new();
    let Ok(rd) = fs::read_dir("/proc") else {
        return owners;
    };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let Ok(pid) = name.to_string_lossy().parse::<u32>() else {
            continue;
        };
        let fd_dir = entry.path().join("fd");
        let Ok(fds) = fs::read_dir(&fd_dir) else {
            continue;
        };
        for fd in fds.flatten() {
            let Ok(target) = fs::read_link(fd.path()) else {
                continue;
            };
            let t = target.to_string_lossy();
            if is_tpu_dev_path(&t) {
                owners.insert(t.to_string(), pid);
            }
        }
    }
    owners
}

fn is_tpu_dev_path(p: &str) -> bool {
    let rest = if let Some(r) = p.strip_prefix("/dev/accel") {
        r
    } else if let Some(r) = p.strip_prefix("/dev/vfio/") {
        r
    } else {
        return false;
    };
    !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit())
}

/// Resolve the owner (pid, path) of logical device `device_id`, mirroring
/// `cli_helper.get_owner_for_device`.
pub fn owner_for_device(
    device_id: usize,
    chip_type: TpuChip,
    chips: &[ChipInfo],
    owners: &HashMap<String, u32>,
) -> (Option<u32>, String) {
    let dev_path = chip_type.chip_path(device_id);
    if let Some(pid) = owners.get(&dev_path) {
        return (Some(*pid), dev_path);
    }
    let dpc = chip_type.devices_per_chip();
    let chip_index = device_id / dpc;
    let core_index = device_id % dpc;
    if let Some(chip) = chips.get(chip_index) {
        if let Some(core) = chip.cores.get(&core_index) {
            if let Some(pid) = owners.get(&core.vfio_path) {
                return (Some(*pid), core.vfio_path.clone());
            }
            if let Some(pid) = owners.get(&core.full_addr) {
                return (Some(*pid), core.full_addr.clone());
            }
        }
        for core in chip.cores.values() {
            if let Some(pid) = owners.get(&core.vfio_path) {
                return (Some(*pid), core.vfio_path.clone());
            }
        }
        if let Some(core) = chip.cores.get(&core_index) {
            return (None, core.vfio_path.clone());
        }
    }
    (None, dev_path)
}

/// Topology string: `TPU_CHIPS_PER_HOST_BOUNDS` if set, else `<n>x<chip>`.
pub fn accelerator_topology(chip: Option<TpuChip>, count: usize) -> String {
    if let Ok(v) = std::env::var("TPU_CHIPS_PER_HOST_BOUNDS") {
        return v;
    }
    match chip {
        Some(c) if count > 0 => format!("{}x{}", count, c.name()),
        _ => "N/A".into(),
    }
}

/// Ports of libtpu metrics servers to query (`TPU_RUNTIME_METRICS_PORTS`,
/// default 8431).
pub fn metrics_ports_from_env() -> Vec<u16> {
    std::env::var("TPU_RUNTIME_METRICS_PORTS")
        .ok()
        .map(|s| {
            s.split(',')
                .filter_map(|p| p.trim().parse().ok())
                .collect::<Vec<u16>>()
        })
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| vec![8431])
}
