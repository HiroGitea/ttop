//! ttop: an interactive top-like monitor for Google Cloud TPU.

mod app;
mod device;
mod grpc;
mod host;
mod model;
mod proto;
mod sampler;
mod ui;
mod update;

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use clap::Parser;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

use crate::app::{App, Focus, Notice};
use crate::device::TpuChip;
use crate::model::{RuntimeState, Snapshot};
use crate::sampler::{Backend, MockBackend, RealBackend};
use crate::update::Outcome;

#[derive(Parser, Debug)]
#[command(
    name = "ttop",
    version,
    about = "Interactive top-like monitor for Google Cloud TPU (like jtop / btop)",
    after_help = "Data sources: PCI sysfs (chip discovery), /proc (owning processes, host stats) and\n\
                  the libtpu RuntimeMetricService gRPC server on localhost:8431, which is only up\n\
                  while a framework (JAX / PyTorch-XLA) holds the TPU."
)]
struct Cli {
    /// Refresh interval in seconds.
    #[arg(short, long, default_value_t = 1.0)]
    interval: f64,

    /// libtpu metrics ports, comma separated
    /// [default: $TPU_RUNTIME_METRICS_PORTS or 8431].
    #[arg(short, long, value_delimiter = ',')]
    ports: Vec<u16>,

    /// gRPC timeout in milliseconds.
    #[arg(long, default_value_t = 800)]
    timeout_ms: u64,

    /// Use simulated data (no TPU required).
    #[arg(long)]
    demo: bool,

    /// Chip type for --demo, or override detection (v4, v5e, v5p, v6e, 7x ...).
    #[arg(long)]
    chip: Option<String>,

    /// Number of logical devices for --demo.
    #[arg(long, default_value_t = 8)]
    demo_devices: usize,

    /// Print one text snapshot and exit (no TUI).
    #[arg(long)]
    once: bool,

    /// Print snapshots as JSON lines (no TUI). Combine with -n.
    #[arg(long)]
    json: bool,

    /// Number of snapshots for --json (0 = forever).
    #[arg(short = 'n', long, default_value_t = 1)]
    count: u64,

    /// List metric names supported by the running libtpu and exit.
    #[arg(long)]
    list_metrics: bool,

    /// Update ttop to the latest GitHub release and exit.
    #[arg(long)]
    update: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if !(cli.interval > 0.0) {
        bail!("--interval must be positive");
    }
    let interval = Duration::from_secs_f64(cli.interval.max(0.1));
    let chip_override = match &cli.chip {
        Some(s) => match TpuChip::from_name(s) {
            Some(c) => Some(c),
            None => bail!("unknown chip type `{s}` (try v4, v5e, v5p, v6e, 7x)"),
        },
        None => None,
    };
    let ports = if cli.ports.is_empty() {
        device::metrics_ports_from_env()
    } else {
        cli.ports.clone()
    };
    let timeout = Duration::from_millis(cli.timeout_ms);

    if cli.list_metrics {
        let b = RealBackend::new(ports, timeout, chip_override)?;
        let mut names = b.list_metrics()?;
        names.sort();
        for n in names {
            println!("{n}");
        }
        return Ok(());
    }

    if cli.update {
        match update::update()? {
            Outcome::UpToDate => println!("ttop {} is up to date", update::CURRENT),
            Outcome::Updated { version } => println!("updated ttop {} -> {version}", update::CURRENT),
            Outcome::NeedsPermission { dir, .. } => {
                bail!("{} is not writable; rerun with sudo", dir.display())
            }
        }
        return Ok(());
    }

    let mut backend: Box<dyn Backend> = if cli.demo {
        Box::new(MockBackend::new(
            chip_override.unwrap_or(TpuChip::V6E),
            cli.demo_devices.clamp(1, 64),
        ))
    } else {
        Box::new(RealBackend::new(ports, timeout, chip_override)?)
    };

    if cli.once || cli.json {
        // A short warm-up sample so CPU / rate figures have a baseline.
        let _ = backend.sample();
        thread::sleep(Duration::from_millis(300));
        let mut i = 0u64;
        loop {
            let snap = backend.sample();
            if cli.json {
                println!("{}", serde_json::to_string(&snap)?);
                std::io::stdout().flush()?;
            } else {
                print_text(&snap);
            }
            i += 1;
            if cli.once || (cli.count != 0 && i >= cli.count) {
                break;
            }
            thread::sleep(interval);
        }
        return Ok(());
    }

    run_tui(backend, interval)
}

fn run_tui(backend: Box<dyn Backend>, interval: Duration) -> Result<()> {
    let interval_ms = Arc::new(AtomicU64::new(interval.as_millis() as u64));
    let paused = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel::<Snapshot>();
    {
        let interval_ms = interval_ms.clone();
        let paused = paused.clone();
        let mut backend = backend;
        thread::spawn(move || loop {
            if !paused.load(Ordering::Relaxed) {
                let snap = backend.sample();
                if tx.send(snap).is_err() {
                    break;
                }
            }
            let start = Instant::now();
            while start.elapsed() < Duration::from_millis(interval_ms.load(Ordering::Relaxed)) {
                thread::sleep(Duration::from_millis(25));
            }
        });
    }

    // Background update check: silent unless an update was installed or needs sudo.
    let (upd_tx, upd_rx) = mpsc::channel::<Outcome>();
    if update::auto_update_enabled() {
        thread::spawn(move || {
            if let Ok(outcome) = update::update() {
                let _ = upd_tx.send(outcome);
            }
        });
    }
    let mut exit_message = None;

    let mut terminal = ratatui::init();
    let mut app = App::new(interval);
    let res = (|| -> Result<()> {
        let mut dirty = true;
        let mut status_until: Option<Instant> = None;
        while !app.should_quit {
            while let Ok(s) = rx.try_recv() {
                app.apply(s);
                dirty = true;
            }
            if let Ok(outcome) = upd_rx.try_recv() {
                match outcome {
                    Outcome::UpToDate => {}
                    Outcome::Updated { version } => {
                        app.notice = Some(Notice {
                            text: format!("✓ updated to v{version} · restart to apply"),
                            warn: false,
                        });
                        exit_message = Some(format!(
                            "ttop was updated to v{version}; the new version runs next time."
                        ));
                    }
                    Outcome::NeedsPermission { version, .. } => {
                        app.notice = Some(Notice {
                            text: format!("v{version} available · sudo ttop --update"),
                            warn: true,
                        });
                        exit_message = Some(format!(
                            "ttop v{version} is available; run `sudo ttop --update` to install it."
                        ));
                    }
                }
                dirty = true;
            }
            if let Some(t) = status_until {
                if Instant::now() > t {
                    app.status = None;
                    status_until = None;
                    dirty = true;
                }
            }
            if dirty {
                terminal.draw(|f| ui::draw(f, &app))?;
                dirty = false;
            }
            if event::poll(Duration::from_millis(50))? {
                match event::read()? {
                    Event::Key(k) if k.kind == KeyEventKind::Press => {
                        dirty = true;
                        if handle_key(&mut app, k.code, k.modifiers, &interval_ms, &paused) {
                            status_until = Some(Instant::now() + Duration::from_secs(2));
                        }
                    }
                    Event::Resize(_, _) => dirty = true,
                    _ => {}
                }
            }
        }
        Ok(())
    })();
    ratatui::restore();
    if let Some(msg) = exit_message {
        eprintln!("{msg}");
    }
    res
}

/// Returns true if a transient status message was set.
fn handle_key(
    app: &mut App,
    code: KeyCode,
    mods: KeyModifiers,
    interval_ms: &AtomicU64,
    paused: &AtomicBool,
) -> bool {
    if mods.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
        app.should_quit = true;
        return false;
    }
    match code {
        KeyCode::Esc if app.show_help => app.show_help = false,
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('?') | KeyCode::Char('h') | KeyCode::F(1) => app.show_help = !app.show_help,
        KeyCode::Up | KeyCode::Char('k') => app.move_sel(-1),
        KeyCode::Down | KeyCode::Char('j') => app.move_sel(1),
        KeyCode::Tab | KeyCode::BackTab => {
            app.focus = match app.focus {
                Focus::Devices => Focus::Processes,
                Focus::Processes => Focus::Devices,
            }
        }
        KeyCode::Char('c') => app.chart_left = app.chart_left.next(),
        KeyCode::Char('v') => app.chart_right = app.chart_right.next(),
        KeyCode::Char('a') => app.chart_all = !app.chart_all,
        KeyCode::Char('p') | KeyCode::Char(' ') => {
            app.paused = !app.paused;
            paused.store(app.paused, Ordering::Relaxed);
        }
        KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Char('-') | KeyCode::Char('_') => {
            let cur = interval_ms.load(Ordering::Relaxed);
            let slower = matches!(code, KeyCode::Char('+') | KeyCode::Char('='));
            let step = if cur >= 2000 || (cur == 1000 && slower) { 500 } else { 100 };
            let new = if slower { cur + step } else { cur.saturating_sub(step) };
            let new = new.clamp(100, 10_000);
            interval_ms.store(new, Ordering::Relaxed);
            app.interval = Duration::from_millis(new);
            app.status = Some(format!("refresh interval {:.1}s", new as f64 / 1000.0));
            return true;
        }
        KeyCode::Char('r') => {
            app.reset_history();
            app.status = Some("chart history cleared".into());
            return true;
        }
        _ => {}
    }
    false
}

fn print_text(s: &Snapshot) {
    let rt = match s.runtime {
        RuntimeState::Connected => "connected",
        RuntimeState::Unavailable => "unavailable",
        RuntimeState::Error => "error",
    };
    println!(
        "ttop  {}  |  {}  x{}  topology {}  |  libtpu {}  |  runtime {}  |  {}",
        s.host.hostname,
        s.chip_name,
        s.chip_count,
        s.topology,
        s.libtpu_version.as_deref().unwrap_or("?"),
        rt,
        s.timestamp
    );
    if let Some(m) = &s.runtime_message {
        println!("WARNING: {m}");
    }
    println!();
    println!(
        "{:>3}  {:<14} {:>8} {:<14} {:>7} {:>17} {:>6} {:>7} {:>6}",
        "DEV", "PATH", "PID", "PROCESS", "DUTY%", "HBM USED/TOTAL", "HBM%", "BW%", "QUEUE"
    );
    let f1 = |v: Option<f64>| v.map(|x| format!("{x:.1}")).unwrap_or("N/A".into());
    for d in &s.devices {
        let mem = match (d.hbm_used, d.hbm_total) {
            (Some(u), Some(t)) => format!("{}/{} GiB", ui::fmt_gib(u), ui::fmt_gib(t)),
            _ => "N/A".into(),
        };
        println!(
            "{:>3}  {:<14} {:>8} {:<14} {:>7} {:>17} {:>6} {:>7} {:>6}",
            d.device_id,
            d.path,
            d.pid.map(|p| p.to_string()).unwrap_or("-".into()),
            d.process_name.clone().unwrap_or("-".into()),
            f1(d.duty_cycle),
            mem,
            f1(d.hbm_pct()),
            f1(d.hbm_bw_util),
            d.hlo_queue_size.map(|q| q.to_string()).unwrap_or("-".into()),
        );
    }
    if s.devices.is_empty() {
        println!("  (no TPU devices found)");
    }
    println!();
    println!(
        "host: cpu {:.1}% ({} cores)  mem {}/{}  load {:.2} {:.2} {:.2}",
        s.host.cpu_total_pct,
        s.host.cpu_per_core_pct.len(),
        ui::fmt_bytes(s.host.mem_used as f64),
        ui::fmt_bytes(s.host.mem_total as f64),
        s.host.load[0],
        s.host.load[1],
        s.host.load[2]
    );
    if !s.processes.is_empty() {
        println!();
        println!("{:>8}  {:<16} {:<10} {:>8} {:>6}  COMMAND", "PID", "NAME", "DEVICES", "HBM", "CPU%");
        for p in &s.processes {
            let devs: Vec<String> = p.devices.iter().map(|d| d.to_string()).collect();
            println!(
                "{:>8}  {:<16} {:<10} {:>7}G {:>6.0}  {}",
                p.pid,
                p.name,
                devs.join(","),
                ui::fmt_gib(p.hbm_used),
                p.cpu_pct,
                p.cmdline
            );
        }
    }
}
