//! UI state: latest snapshot, metric history, selection and key handling.

use std::collections::VecDeque;
use std::time::Duration;

use crate::model::Snapshot;

pub const HISTORY: usize = 600;

#[derive(Clone, Debug)]
pub struct History {
    data: VecDeque<f64>,
}

impl History {
    pub fn new() -> Self {
        Self {
            data: VecDeque::with_capacity(HISTORY),
        }
    }
    pub fn push(&mut self, v: f64) {
        if self.data.len() == HISTORY {
            self.data.pop_front();
        }
        self.data.push_back(v);
    }
    pub fn last(&self) -> Option<f64> {
        self.data.back().copied()
    }
    /// Points as (x, y) for a chart of width `w` (most recent on the right).
    pub fn points(&self, w: usize) -> Vec<(f64, f64)> {
        let n = self.data.len().min(w);
        let skip = self.data.len() - n;
        self.data
            .iter()
            .skip(skip)
            .enumerate()
            .map(|(i, v)| ((w - n + i) as f64, *v))
            .collect()
    }
    pub fn tail_u64(&self, w: usize) -> Vec<u64> {
        let n = self.data.len().min(w);
        let skip = self.data.len() - n;
        self.data.iter().skip(skip).map(|v| v.round() as u64).collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChartMetric {
    Duty,
    Hbm,
    Bandwidth,
}

impl ChartMetric {
    pub fn next(self) -> Self {
        match self {
            ChartMetric::Duty => ChartMetric::Hbm,
            ChartMetric::Hbm => ChartMetric::Bandwidth,
            ChartMetric::Bandwidth => ChartMetric::Duty,
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            ChartMetric::Duty => "TensorCore duty cycle %",
            ChartMetric::Hbm => "HBM used %",
            ChartMetric::Bandwidth => "HBM bandwidth util %",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Devices,
    Processes,
}

/// Persistent footer message, e.g. from the auto-updater.
pub struct Notice {
    pub text: String,
    pub warn: bool,
}

pub struct App {
    pub snap: Snapshot,
    pub duty: Vec<History>,
    pub hbm: Vec<History>,
    pub bw: Vec<History>,
    pub cpu: History,
    pub net_rx: History,
    pub net_tx: History,
    pub selected: usize,
    pub proc_selected: usize,
    pub focus: Focus,
    pub paused: bool,
    pub show_help: bool,
    pub chart_left: ChartMetric,
    pub chart_right: ChartMetric,
    /// Plot every device (true) or only the selected one (false).
    pub chart_all: bool,
    pub interval: Duration,
    pub samples: u64,
    pub should_quit: bool,
    pub status: Option<String>,
    pub notice: Option<Notice>,
}

impl App {
    pub fn new(interval: Duration) -> Self {
        Self {
            snap: Snapshot::default(),
            duty: vec![],
            hbm: vec![],
            bw: vec![],
            cpu: History::new(),
            net_rx: History::new(),
            net_tx: History::new(),
            selected: 0,
            proc_selected: 0,
            focus: Focus::Devices,
            paused: false,
            show_help: false,
            chart_left: ChartMetric::Duty,
            chart_right: ChartMetric::Hbm,
            chart_all: true,
            interval,
            samples: 0,
            should_quit: false,
            status: None,
            notice: None,
        }
    }

    pub fn apply(&mut self, snap: Snapshot) {
        let n = snap.devices.len();
        while self.duty.len() < n {
            self.duty.push(History::new());
            self.hbm.push(History::new());
            self.bw.push(History::new());
        }
        for (i, d) in snap.devices.iter().enumerate() {
            self.duty[i].push(d.duty_cycle.unwrap_or(0.0));
            self.hbm[i].push(d.hbm_pct().unwrap_or(0.0));
            self.bw[i].push(d.hbm_bw_util.unwrap_or(0.0));
        }
        self.cpu.push(snap.host.cpu_total_pct);
        self.net_rx.push(snap.host.net_rx_bps);
        self.net_tx.push(snap.host.net_tx_bps);
        if n > 0 && self.selected >= n {
            self.selected = n - 1;
        }
        let np = snap.processes.len();
        if np > 0 && self.proc_selected >= np {
            self.proc_selected = np - 1;
        }
        self.samples += 1;
        self.snap = snap;
    }

    pub fn reset_history(&mut self) {
        for h in self.duty.iter_mut().chain(self.hbm.iter_mut()).chain(self.bw.iter_mut()) {
            *h = History::new();
        }
        self.cpu = History::new();
        self.net_rx = History::new();
        self.net_tx = History::new();
    }

    pub fn move_sel(&mut self, delta: i32) {
        match self.focus {
            Focus::Devices => {
                let n = self.snap.devices.len();
                if n == 0 {
                    return;
                }
                self.selected = (self.selected as i32 + delta).rem_euclid(n as i32) as usize;
            }
            Focus::Processes => {
                let n = self.snap.processes.len();
                if n == 0 {
                    return;
                }
                self.proc_selected =
                    (self.proc_selected as i32 + delta).rem_euclid(n as i32) as usize;
            }
        }
    }
}
