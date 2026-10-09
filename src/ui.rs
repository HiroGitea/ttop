//! ratatui rendering, btop-style.

use ratatui::layout::Flex;
use ratatui::prelude::*;
use ratatui::symbols::Marker;
use ratatui::widgets::*;

use crate::app::{App, ChartMetric, Focus, History};
use crate::model::*;

const BORDER: Color = Color::Rgb(0x4c, 0x56, 0x6a);
const TITLE: Color = Color::Rgb(0x8b, 0xe9, 0xfd);
const TEXT: Color = Color::Rgb(0xd8, 0xde, 0xe9);
const DIM: Color = Color::Rgb(0x7a, 0x82, 0x96);
const EMPTY: Color = Color::Rgb(0x34, 0x39, 0x46);
const ACCENT: Color = Color::Rgb(0xbd, 0x93, 0xf9);
const SEL_BG: Color = Color::Rgb(0x2b, 0x31, 0x3d);
const OK: Color = Color::Rgb(0x50, 0xfa, 0x7b);
const WARN: Color = Color::Rgb(0xf1, 0xfa, 0x8c);
const BAD: Color = Color::Rgb(0xff, 0x55, 0x55);

const PALETTE: [Color; 8] = [
    Color::Rgb(0x8b, 0xe9, 0xfd),
    Color::Rgb(0xff, 0x79, 0xc6),
    Color::Rgb(0x50, 0xfa, 0x7b),
    Color::Rgb(0xf1, 0xfa, 0x8c),
    Color::Rgb(0xff, 0xb8, 0x6c),
    Color::Rgb(0xbd, 0x93, 0xf9),
    Color::Rgb(0x6c, 0xb6, 0xff),
    Color::Rgb(0xff, 0x55, 0x55),
];

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn lerp(a: (u8, u8, u8), b: (u8, u8, u8), t: f64) -> Color {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t).round() as u8;
    Color::Rgb(f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

/// green -> yellow -> red gradient for a percentage.
pub fn grad(p: f64) -> Color {
    let g = (0x50, 0xfa, 0x7b);
    let y = (0xf1, 0xfa, 0x8c);
    let r = (0xff, 0x55, 0x55);
    if p < 50.0 {
        lerp(g, y, p / 50.0)
    } else {
        lerp(y, r, (p - 50.0) / 50.0)
    }
}

fn bar(pct: Option<f64>, width: usize) -> Vec<Span<'static>> {
    if width == 0 {
        return vec![];
    }
    let Some(p) = pct else {
        return vec![Span::styled("·".repeat(width), Style::new().fg(EMPTY))];
    };
    let p = p.clamp(0.0, 100.0);
    let filled = ((p / 100.0) * width as f64).round() as usize;
    let mut v = Vec::with_capacity(width.min(filled + 1));
    for i in 0..filled.min(width) {
        let c = grad((i as f64 + 0.5) / width as f64 * 100.0);
        v.push(Span::styled("■", Style::new().fg(c)));
    }
    if filled < width {
        v.push(Span::styled("■".repeat(width - filled), Style::new().fg(EMPTY)));
    }
    v
}

fn pct_span(p: Option<f64>, w: usize) -> Span<'static> {
    match p {
        Some(p) => Span::styled(format!("{:>w$.1}%", p, w = w - 1), Style::new().fg(grad(p))),
        None => Span::styled(format!("{:>w$}", "N/A", w = w), Style::new().fg(DIM)),
    }
}

pub fn fmt_bytes(b: f64) -> String {
    const U: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    let mut v = b;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{v:.0}{}", U[i])
    } else if v >= 100.0 {
        format!("{v:.0}{}", U[i])
    } else {
        format!("{v:.1}{}", U[i])
    }
}

pub fn fmt_gib(b: u64) -> String {
    format!("{:.1}", b as f64 / (1u64 << 30) as f64)
}

pub fn fmt_dur(secs: u64) -> String {
    let d = secs / 86400;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if d > 0 {
        format!("{d}d{h:02}h")
    } else if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn fmt_us(us: f64) -> String {
    if us >= 1_000_000.0 {
        format!("{:.2}s", us / 1e6)
    } else if us >= 1000.0 {
        format!("{:.2}ms", us / 1e3)
    } else {
        format!("{us:.0}µs")
    }
}

fn truncate(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        s.to_string()
    } else if w == 0 {
        String::new()
    } else {
        let mut t: String = s.chars().take(w.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

fn panel(title: String, focused: bool) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(if focused { ACCENT } else { BORDER }))
        .title(Line::from(vec![
            Span::raw(" "),
            Span::styled(title, Style::new().fg(TITLE).add_modifier(Modifier::BOLD)),
            Span::raw(" "),
        ]))
}

fn kv(k: &str, v: String, vc: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{k:<10}"), Style::new().fg(DIM)),
        Span::styled(v, Style::new().fg(vc)),
    ])
}

// ---------------------------------------------------------------------------
// top level
// ---------------------------------------------------------------------------

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    f.render_widget(Block::new().style(Style::new().fg(TEXT)), area);
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area);
    draw_header(f, header, app);
    draw_footer(f, footer, app);

    let s = &app.snap;
    let host_w = ((body.width as u32 * 32 / 100) as u16).clamp(30, 64);
    let dev_lines = s.devices.len().max(1) as u16 + 1 + status_lines(app);
    let host_inner_w = host_w.saturating_sub(2);
    let host_h = host_lines(app, host_inner_w).len() as u16 + 2;
    let top_h = (dev_lines + 2)
        .max(host_h)
        .min((body.height as u32 * 50 / 100) as u16)
        .max(6);
    let proc_h = (s.processes.len().max(1) as u16 + 3).clamp(5, 12);

    let [top, mid, bottom] = Layout::vertical([
        Constraint::Length(top_h),
        Constraint::Min(7),
        Constraint::Length(proc_h),
    ])
    .areas(body);

    let [dev_area, host_area] =
        Layout::horizontal([Constraint::Min(40), Constraint::Length(host_w)]).areas(top);
    draw_devices(f, dev_area, app);
    draw_host(f, host_area, app);

    if mid.width >= 120 {
        let [a, b, c] = Layout::horizontal([
            Constraint::Percentage(38),
            Constraint::Percentage(37),
            Constraint::Percentage(25),
        ])
        .areas(mid);
        draw_chart(f, a, app, app.chart_left);
        draw_chart(f, b, app, app.chart_right);
        draw_details(f, c, app);
    } else if mid.width >= 80 {
        let [a, c] =
            Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)]).areas(mid);
        draw_chart(f, a, app, app.chart_left);
        draw_details(f, c, app);
    } else {
        draw_chart(f, mid, app, app.chart_left);
    }

    draw_processes(f, bottom, app);

    if app.show_help {
        draw_help(f, area);
    }
}

fn status_lines(app: &App) -> u16 {
    let s = &app.snap;
    let mut n = 0;
    if s.chip.is_none() {
        n += 1;
    } else if s.runtime_message.is_some() {
        n += 1;
    }
    n
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let s = &app.snap;
    let (rt_txt, rt_col) = match s.runtime {
        RuntimeState::Connected => ("● libtpu connected", OK),
        RuntimeState::Unavailable => ("○ libtpu idle", WARN),
        RuntimeState::Error => ("✖ libtpu error", BAD),
    };
    let mut left = vec![
        Span::styled(" ttop ", Style::new().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(s.host.hostname.clone(), Style::new().fg(TEXT).add_modifier(Modifier::BOLD)),
        Span::styled("  │  ", Style::new().fg(BORDER)),
        Span::styled(
            if app.samples == 0 { "probing…".to_string() } else { s.chip_name.clone() },
            Style::new().fg(TITLE).add_modifier(Modifier::BOLD),
        ),
    ];
    if s.chip.is_some() {
        left.push(Span::styled(
            format!(" ×{}  topo {}", s.chip_count, s.topology),
            Style::new().fg(TEXT),
        ));
    }
    left.push(Span::styled("  │  ", Style::new().fg(BORDER)));
    left.push(Span::styled(rt_txt, Style::new().fg(rt_col)));
    if let Some(v) = &s.libtpu_version {
        left.push(Span::styled(format!("  v{v}"), Style::new().fg(DIM)));
    }
    let mut right = vec![];
    if app.paused {
        right.push(Span::styled(" PAUSED ", Style::new().fg(Color::Black).bg(WARN).add_modifier(Modifier::BOLD)));
        right.push(Span::raw(" "));
    }
    right.push(Span::styled(
        format!("⟳ {:.1}s  ", app.interval.as_secs_f64()),
        Style::new().fg(DIM),
    ));
    right.push(Span::styled(
        s.timestamp.get(11..).unwrap_or("").to_string() + " ",
        Style::new().fg(TEXT),
    ));
    let right = Line::from(right);
    let [l, r] = Layout::horizontal([Constraint::Min(0), Constraint::Length(right.width() as u16 + 1)])
        .areas(area);
    f.render_widget(Paragraph::new(Line::from(left)), l);
    f.render_widget(Paragraph::new(right).alignment(Alignment::Right), r);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let keys: [(&str, &str); 9] = [
        ("q", "quit"),
        ("↑↓", "select"),
        ("tab", "focus"),
        ("c/v", "charts"),
        ("a", "all/one"),
        ("p", if app.paused { "resume" } else { "pause" }),
        ("+/-", "interval"),
        ("r", "reset"),
        ("?", "help"),
    ];
    let mut spans = vec![Span::raw(" ")];
    for (k, d) in keys {
        spans.push(Span::styled(k, Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)));
        spans.push(Span::styled(format!(" {d}  "), Style::new().fg(DIM)));
    }
    let right = match (&app.status, &app.notice) {
        (Some(st), _) => Span::styled(format!(" {st} "), Style::new().fg(WARN)),
        (None, Some(n)) => Span::styled(
            format!(" {} ", n.text),
            Style::new().fg(if n.warn { WARN } else { OK }),
        ),
        (None, None) => Span::styled(format!(" sample {}ms ", app.snap.sample_ms), Style::new().fg(DIM)),
    };
    let [l, r] = Layout::horizontal([Constraint::Min(0), Constraint::Length(right.width() as u16)])
        .areas(area);
    f.render_widget(Paragraph::new(Line::from(spans)), l);
    f.render_widget(Paragraph::new(right).alignment(Alignment::Right), r);
}

// ---------------------------------------------------------------------------
// devices
// ---------------------------------------------------------------------------

fn draw_devices(f: &mut Frame, area: Rect, app: &App) {
    let s = &app.snap;
    let title = match (s.chip, s.total_hbm()) {
        (Some(_), t) if t > 0 => format!(
            "TPU devices ({}) · HBM {}/{} GiB · avg duty {}",
            s.devices.len(),
            fmt_gib(s.total_hbm_used()),
            fmt_gib(t),
            s.avg_duty().map(|v| format!("{v:.0}%")).unwrap_or("N/A".into()),
        ),
        (Some(_), _) => format!("TPU devices ({})", s.devices.len()),
        _ => "TPU devices".to_string(),
    };
    let block = panel(title, app.focus == Focus::Devices);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let w = inner.width as usize;

    let mut lines: Vec<Line> = vec![];
    if app.samples > 0 && s.chip.is_none() {
        lines.push(Line::styled(
            "No TPU found (no PCI device with vendor 0x1ae0). Try `ttop --demo`.",
            Style::new().fg(WARN),
        ));
    }
    if let (Some(msg), Some(_)) = (&s.runtime_message, s.chip) {
        let col = if s.runtime == RuntimeState::Error { BAD } else { WARN };
        lines.push(Line::styled(truncate(&format!("⚠ {msg}"), w), Style::new().fg(col)));
    }

    // column plan
    let show_bw = w >= 84;
    let show_proc = w >= 70;
    let show_q = w >= 100;
    let fixed = 1 + 4 + 7 + 2 + 13 + 2
        + if show_bw { 7 + 2 } else { 0 }
        + if show_q { 6 } else { 0 }
        + if show_proc { 22 } else { 0 };
    let nbars = if show_bw { 3 } else { 2 };
    let bw = (w.saturating_sub(fixed) / nbars).clamp(3, 40);

    let hdr = Style::new().fg(DIM).add_modifier(Modifier::BOLD);
    let mut h = vec![
        Span::styled(format!(" {:>3} ", "ID"), hdr),
        Span::styled(format!("{:<width$}{:>7}", "TensorCore duty", "", width = bw), hdr),
        Span::raw("  "),
        Span::styled(format!("{:<width$}{:>13}", "HBM", "GiB", width = bw), hdr),
    ];
    if show_bw {
        h.push(Span::raw("  "));
        h.push(Span::styled(format!("{:<width$}{:>7}", "HBM bandwidth", "", width = bw), hdr));
    }
    if show_q {
        h.push(Span::styled(format!("{:>6}", "Queue"), hdr));
    }
    if show_proc {
        h.push(Span::styled(format!("  {:>7} {:<12}", "PID", "Process"), hdr));
    }
    if !s.devices.is_empty() {
        lines.push(Line::from(h));
    }

    let avail = (inner.height as usize).saturating_sub(lines.len());
    let offset = if app.selected >= avail && avail > 0 {
        app.selected + 1 - avail
    } else {
        0
    };
    for d in s.devices.iter().skip(offset).take(avail) {
        let sel = d.device_id == app.selected;
        let mut sp = vec![
            Span::styled(
                if sel { "▶" } else { " " },
                Style::new().fg(ACCENT),
            ),
            Span::styled(format!("{:>3} ", d.device_id), Style::new().fg(PALETTE[d.device_id % 8]).add_modifier(Modifier::BOLD)),
        ];
        sp.extend(bar(d.duty_cycle, bw));
        sp.push(pct_span(d.duty_cycle, 7));
        sp.push(Span::raw("  "));
        sp.extend(bar(d.hbm_pct(), bw));
        let mem = match (d.hbm_used, d.hbm_total) {
            (Some(u), Some(t)) => format!("{}/{}", fmt_gib(u), fmt_gib(t)),
            _ => "N/A".into(),
        };
        sp.push(Span::styled(
            format!("{mem:>13}"),
            Style::new().fg(d.hbm_pct().map(grad).unwrap_or(DIM)),
        ));
        if show_bw {
            sp.push(Span::raw("  "));
            sp.extend(bar(d.hbm_bw_util, bw));
            sp.push(pct_span(d.hbm_bw_util, 7));
        }
        if show_q {
            sp.push(Span::styled(
                format!("{:>6}", d.hlo_queue_size.map(|q| q.to_string()).unwrap_or("-".into())),
                Style::new().fg(TEXT),
            ));
        }
        if show_proc {
            let pid = d.pid.map(|p| p.to_string()).unwrap_or("-".into());
            let name = d.process_name.clone().unwrap_or_else(|| "-".into());
            sp.push(Span::styled(format!("  {pid:>7} "), Style::new().fg(TEXT)));
            sp.push(Span::styled(
                format!("{:<12}", truncate(&name, 12)),
                Style::new().fg(if d.pid.is_some() { OK } else { DIM }),
            ));
        }
        let mut line = Line::from(sp);
        if sel && app.focus == Focus::Devices {
            line = line.style(Style::new().bg(SEL_BG));
        }
        lines.push(line);
    }
    if s.devices.is_empty() && app.samples == 0 {
        lines.push(Line::styled("  waiting for first sample…", Style::new().fg(DIM)));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

// ---------------------------------------------------------------------------
// host
// ---------------------------------------------------------------------------

fn host_lines(app: &App, w: u16) -> Vec<Line<'static>> {
    let h = &app.snap.host;
    let w = w as usize;
    let label = |s: &str| Span::styled(format!("{s:<5}"), Style::new().fg(DIM));
    let bw = w.saturating_sub(5 + 7 + 12).max(4);
    let mut lines = vec![];

    let mut cpu = vec![label("CPU")];
    cpu.extend(bar(Some(h.cpu_total_pct), bw));
    cpu.push(pct_span(Some(h.cpu_total_pct), 7));
    cpu.push(Span::styled(
        format!(" {:>3} cores", h.cpu_per_core_pct.len()),
        Style::new().fg(DIM),
    ));
    lines.push(Line::from(cpu));

    // per-core heat strip: one cell per core
    const LV: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let per_line = w.saturating_sub(5).max(1);
    for (ci, chunk) in h.cpu_per_core_pct.chunks(per_line).enumerate() {
        let mut sp = vec![if ci == 0 { label("core") } else { Span::raw("     ") }];
        for p in chunk {
            let idx = ((p / 100.0) * 7.0).round().clamp(0.0, 7.0) as usize;
            sp.push(Span::styled(LV[idx].to_string(), Style::new().fg(grad(*p))));
        }
        lines.push(Line::from(sp));
    }

    let mem_pct = if h.mem_total > 0 {
        h.mem_used as f64 / h.mem_total as f64 * 100.0
    } else {
        0.0
    };
    let mut mem = vec![label("Mem")];
    mem.extend(bar(Some(mem_pct), bw));
    mem.push(pct_span(Some(mem_pct), 7));
    mem.push(Span::styled(
        format!(" {:>5}/{:<5}", fmt_bytes(h.mem_used as f64), fmt_bytes(h.mem_total as f64)),
        Style::new().fg(TEXT),
    ));
    lines.push(Line::from(mem));

    if h.swap_total > 0 {
        let sp_pct = h.swap_used as f64 / h.swap_total as f64 * 100.0;
        let mut sw = vec![label("Swap")];
        sw.extend(bar(Some(sp_pct), bw));
        sw.push(pct_span(Some(sp_pct), 7));
        sw.push(Span::styled(
            format!(" {:>5}/{:<5}", fmt_bytes(h.swap_used as f64), fmt_bytes(h.swap_total as f64)),
            Style::new().fg(TEXT),
        ));
        lines.push(Line::from(sw));
    }

    lines.push(Line::from(vec![
        label("Net"),
        Span::styled("↓ ", Style::new().fg(OK)),
        Span::styled(format!("{:<10}", fmt_bytes(h.net_rx_bps) + "/s"), Style::new().fg(TEXT)),
        Span::styled("↑ ", Style::new().fg(PALETTE[1])),
        Span::styled(fmt_bytes(h.net_tx_bps) + "/s", Style::new().fg(TEXT)),
    ]));
    lines.push(Line::from(vec![
        label("Disk"),
        Span::styled("R ", Style::new().fg(OK)),
        Span::styled(format!("{:<10}", fmt_bytes(h.disk_read_bps) + "/s"), Style::new().fg(TEXT)),
        Span::styled("W ", Style::new().fg(PALETTE[1])),
        Span::styled(fmt_bytes(h.disk_write_bps) + "/s", Style::new().fg(TEXT)),
    ]));
    lines.push(Line::from(vec![
        label("Load"),
        Span::styled(
            format!("{:.2} {:.2} {:.2}", h.load[0], h.load[1], h.load[2]),
            Style::new().fg(TEXT),
        ),
        Span::styled("   up ", Style::new().fg(DIM)),
        Span::styled(fmt_dur(h.uptime_secs), Style::new().fg(TEXT)),
    ]));
    lines
}

fn draw_host(f: &mut Frame, area: Rect, app: &App) {
    let block = panel("Host".to_string(), false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(host_lines(app, inner.width)), inner);
}

// ---------------------------------------------------------------------------
// charts
// ---------------------------------------------------------------------------

fn draw_chart(f: &mut Frame, area: Rect, app: &App, metric: ChartMetric) {
    let hist: &Vec<History> = match metric {
        ChartMetric::Duty => &app.duty,
        ChartMetric::Hbm => &app.hbm,
        ChartMetric::Bandwidth => &app.bw,
    };
    let n = hist.len();
    // braille: 2 samples per cell
    let window = ((area.width.saturating_sub(2 + 5)) as usize * 2).clamp(10, crate::app::HISTORY);

    let ids: Vec<usize> = if app.chart_all && n <= PALETTE.len() {
        (0..n).collect()
    } else if n > 0 {
        vec![app.selected.min(n - 1)]
    } else {
        vec![]
    };
    // average line when there are many devices
    let avg: Option<Vec<(f64, f64)>> = if app.chart_all && n > PALETTE.len() {
        let all: Vec<Vec<(f64, f64)>> = hist.iter().map(|h| h.points(window)).collect();
        let len = all.iter().map(|v| v.len()).min().unwrap_or(0);
        Some(
            (0..len)
                .map(|i| {
                    let x = all[0][all[0].len() - len + i].0;
                    let y = all.iter().map(|v| v[v.len() - len + i].1).sum::<f64>() / n as f64;
                    (x, y)
                })
                .collect(),
        )
    } else {
        None
    };
    let pts: Vec<Vec<(f64, f64)>> = ids.iter().map(|&i| hist[i].points(window)).collect();

    let mut title = vec![
        Span::raw(" "),
        Span::styled(metric.title(), Style::new().fg(TITLE).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
    ];
    for &i in ids.iter().take(8) {
        let cur = hist[i].last().map(|v| format!("{v:.0}")).unwrap_or("-".into());
        title.push(Span::styled(format!("●{i}:{cur} "), Style::new().fg(PALETTE[i % 8])));
    }
    if let Some(a) = &avg {
        let cur = a.last().map(|p| format!("{:.0}", p.1)).unwrap_or("-".into());
        title.push(Span::styled(format!("●avg:{cur} "), Style::new().fg(TEXT)));
    }

    let mut datasets = vec![];
    if let Some(a) = &avg {
        datasets.push(
            Dataset::default()
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::new().fg(TEXT))
                .data(a),
        );
    }
    for (k, &i) in ids.iter().enumerate() {
        datasets.push(
            Dataset::default()
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::new().fg(PALETTE[i % 8]))
                .data(&pts[k]),
        );
    }

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(BORDER))
        .title(Line::from(title));
    let chart = Chart::new(datasets)
        .block(block)
        .legend_position(None)
        .x_axis(
            Axis::default()
                .bounds([0.0, (window - 1) as f64])
                .style(Style::new().fg(BORDER)),
        )
        .y_axis(
            Axis::default()
                .bounds([0.0, 100.0])
                .labels(["0", "50", "100"])
                .style(Style::new().fg(DIM)),
        );
    f.render_widget(chart, area);
}

// ---------------------------------------------------------------------------
// selected-device details
// ---------------------------------------------------------------------------

fn draw_details(f: &mut Frame, area: Rect, app: &App) {
    let s = &app.snap;
    let Some(d) = s.devices.get(app.selected) else {
        let block = panel("Device".to_string(), false);
        let inner = block.inner(area);
        f.render_widget(block, area);
        f.render_widget(Paragraph::new(Line::styled("no device", Style::new().fg(DIM))), inner);
        return;
    };
    let block = panel(format!("Device {}", d.device_id), false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let na = || "N/A".to_string();
    let mut lines = vec![
        kv("Path", d.path.clone(), TEXT),
        kv(
            "Owner",
            match d.pid {
                Some(p) => format!("{} ({p})", d.process_name.clone().unwrap_or("?".into())),
                None => "-".into(),
            },
            if d.pid.is_some() { OK } else { DIM },
        ),
        kv(
            "Duty",
            d.duty_cycle.map(|v| format!("{v:.1} %")).unwrap_or_else(na),
            d.duty_cycle.map(grad).unwrap_or(DIM),
        ),
        kv(
            "HBM",
            match (d.hbm_used, d.hbm_total, d.hbm_pct()) {
                (Some(u), Some(t), Some(p)) => {
                    format!("{} / {} GiB ({p:.0}%)", fmt_gib(u), fmt_gib(t))
                }
                _ => na(),
            },
            d.hbm_pct().map(grad).unwrap_or(DIM),
        ),
        kv(
            "HBM BW",
            d.hbm_bw_util.map(|v| format!("{v:.1} %")).unwrap_or_else(na),
            d.hbm_bw_util.map(grad).unwrap_or(DIM),
        ),
        kv(
            "TC idle",
            d.tc_idle_secs.map(|v| format!("{v:.2} s")).unwrap_or_else(na),
            TEXT,
        ),
        kv(
            "HLO queue",
            d.hlo_queue_size.map(|v| v.to_string()).unwrap_or_else(na),
            TEXT,
        ),
    ];
    match &d.hlo_timing {
        Some(t) => {
            lines.push(kv(
                "HLO exec",
                format!("mean {}  n={}", fmt_us(t.mean_us), t.count),
                TEXT,
            ));
            lines.push(kv(
                "",
                format!("p50 {}  p90 {}", fmt_us(t.p50_us), fmt_us(t.p90_us)),
                TEXT,
            ));
            lines.push(kv(
                "",
                format!("p95 {}  p99.9 {}", fmt_us(t.p95_us), fmt_us(t.p999_us)),
                TEXT,
            ));
        }
        None => lines.push(kv("HLO exec", na(), DIM)),
    }
    // mini sparkline of duty cycle for the selected device
    let spark_h = inner.height.saturating_sub(lines.len() as u16);
    f.render_widget(Paragraph::new(lines.clone()), inner);
    if spark_h >= 2 {
        if let Some(h) = app.duty.get(app.selected) {
            let data = h.tail_u64(inner.width as usize);
            let r = Rect {
                x: inner.x,
                y: inner.y + lines.len() as u16,
                width: inner.width,
                height: spark_h,
            };
            f.render_widget(
                Sparkline::default()
                    .data(&data)
                    .max(100)
                    .style(Style::new().fg(PALETTE[app.selected % 8])),
                r,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// processes
// ---------------------------------------------------------------------------

fn draw_processes(f: &mut Frame, area: Rect, app: &App) {
    let s = &app.snap;
    let block = panel(
        format!("TPU processes ({})", s.processes.len()),
        app.focus == Focus::Processes,
    );
    let hdr = Row::new(["PID", "Name", "Devices", "HBM", "CPU%", "RSS", "Time", "Command"])
        .style(Style::new().fg(DIM).add_modifier(Modifier::BOLD));
    let widths = [
        Constraint::Length(8),
        Constraint::Length(14),
        Constraint::Length(10),
        Constraint::Length(8),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(9),
        Constraint::Fill(1),
    ];
    if s.processes.is_empty() {
        let inner = block.inner(area);
        f.render_widget(block, area);
        let msg = if s.chip.is_some() {
            "  no process holds a TPU device (other users' processes are only visible as root)"
        } else {
            "  no TPU processes"
        };
        f.render_widget(Paragraph::new(Line::styled(msg, Style::new().fg(DIM))), inner);
        return;
    }
    let rows: Vec<Row> = {
        s.processes
            .iter()
            .map(|p| {
                let devs = compact_ids(&p.devices);
                Row::new([
                    Cell::from(Span::styled(p.pid.to_string(), Style::new().fg(TEXT))),
                    Cell::from(Span::styled(truncate(&p.name, 14), Style::new().fg(OK))),
                    Cell::from(Span::styled(devs, Style::new().fg(TITLE))),
                    Cell::from(Span::styled(format!("{}G", fmt_gib(p.hbm_used)), Style::new().fg(TEXT))),
                    Cell::from(Span::styled(
                        format!("{:.0}", p.cpu_pct),
                        Style::new().fg(grad((p.cpu_pct / 4.0).min(100.0))),
                    )),
                    Cell::from(Span::styled(fmt_bytes(p.rss_bytes as f64), Style::new().fg(TEXT))),
                    Cell::from(Span::styled(fmt_dur(p.elapsed_secs), Style::new().fg(TEXT))),
                    Cell::from(Span::styled(p.cmdline.clone(), Style::new().fg(DIM))),
                ])
            })
            .collect()
    };
    let mut state = TableState::default();
    if app.focus == Focus::Processes && !s.processes.is_empty() {
        state.select(Some(app.proc_selected));
    }
    let table = Table::new(rows, widths)
        .header(hdr)
        .block(block)
        .column_spacing(1)
        .row_highlight_style(Style::new().bg(SEL_BG).add_modifier(Modifier::BOLD))
        .highlight_symbol("▶");
    f.render_stateful_widget(table, area, &mut state);
}

/// [0,1,2,3,6] -> "0-3,6"
fn compact_ids(ids: &[usize]) -> String {
    let mut v = ids.to_vec();
    v.sort_unstable();
    v.dedup();
    let mut out = vec![];
    let mut i = 0;
    while i < v.len() {
        let start = v[i];
        let mut end = start;
        while i + 1 < v.len() && v[i + 1] == end + 1 {
            i += 1;
            end = v[i];
        }
        out.push(if end > start { format!("{start}-{end}") } else { start.to_string() });
        i += 1;
    }
    out.join(",")
}

// ---------------------------------------------------------------------------
// help
// ---------------------------------------------------------------------------

fn draw_help(f: &mut Frame, area: Rect) {
    let rows = [
        ("q / Esc / Ctrl-C", "quit (Esc closes this help first)"),
        ("↑ ↓  /  k j", "select device or process"),
        ("Tab", "switch focus: devices ↔ processes"),
        ("c", "cycle left chart: duty / HBM / bandwidth"),
        ("v", "cycle right chart"),
        ("a", "charts: all devices ↔ selected device"),
        ("p / Space", "pause / resume sampling"),
        ("+ / -", "slower / faster refresh"),
        ("r", "reset chart history"),
        ("? / h / F1", "toggle this help"),
    ];
    let mut lines = vec![Line::raw("")];
    for (k, d) in rows {
        lines.push(Line::from(vec![
            Span::styled(format!("  {k:<18}"), Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled(d, Style::new().fg(TEXT)),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "  Data: PCI sysfs + libtpu RuntimeMetricService (gRPC :8431)",
        Style::new().fg(DIM),
    ));
    let w = 64.min(area.width);
    let h = (lines.len() as u16 + 2).min(area.height);
    let [r] = Layout::horizontal([Constraint::Length(w)]).flex(Flex::Center).areas(area);
    let [r] = Layout::vertical([Constraint::Length(h)]).flex(Flex::Center).areas(r);
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines).block(panel("Help".to_string(), true)),
        r,
    );
}
