//! Linux system counters. Sampling runs off the GTK thread; histories are bounded.
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::time::{Duration, Instant};
use std::{cell::RefCell, rc::Rc};

use gtk::prelude::*;
use gtk::{cairo, glib};

const HISTORY: usize = 120;
type Counters = HashMap<String, [u64; 2]>;
type Color = (f64, f64, f64);
const COLORS: [Color; 4] = [
    (0.42, 0.78, 0.72),
    (0.66, 0.60, 0.88),
    (0.48, 0.69, 0.92),
    (0.90, 0.69, 0.43),
];
const SECONDARY: Color = (0.74, 0.79, 0.86);

#[derive(Default)]
struct Snapshot {
    cpu: Option<(u64, u64)>,
    memory: Option<(u64, u64)>,
    swap: Option<(u64, u64)>,
    network: Option<Counters>,
    disk: Option<Counters>,
}

fn cpu(text: &str) -> Option<(u64, u64)> {
    let line = text.lines().find(|line| line.starts_with("cpu "))?;
    // Guest time is already included in user/nice, so only sum the first eight fields.
    let ticks: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    (ticks.len() == 8).then(|| (ticks.iter().sum(), ticks[3] + ticks[4]))
}

fn memory(text: &str, total: &str, available: &str) -> Option<(u64, u64)> {
    let field = |name: &str| {
        text.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            (key == name)
                .then(|| value.split_whitespace().next()?.parse::<u64>().ok())
                .flatten()
        })
    };
    let total = field(total)? * 1024;
    Some((total.saturating_sub(field(available)? * 1024), total))
}

fn network(text: &str) -> Counters {
    text.lines()
        .filter_map(|line| {
            let (name, values) = line.split_once(':')?;
            let name = name.trim();
            if name == "lo" {
                return None;
            }
            let values: Vec<_> = values.split_whitespace().collect();
            Some((
                name.to_string(),
                [values.first()?.parse().ok()?, values.get(8)?.parse().ok()?],
            ))
        })
        .collect()
}

fn disks(text: &str, devices: &[String]) -> Counters {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            let name = *fields.get(2)?;
            if !devices.iter().any(|device| device == name) {
                return None;
            }
            // Linux diskstats always reports sectors in units of 512 bytes.
            Some((
                name.to_string(),
                [
                    fields.get(5)?.parse::<u64>().ok()? * 512,
                    fields.get(9)?.parse::<u64>().ok()? * 512,
                ],
            ))
        })
        .collect()
}

impl Snapshot {
    fn read() -> Self {
        let meminfo = fs::read_to_string("/proc/meminfo").ok();
        // Only whole physical devices: exclude partitions and device-mapper layers
        // to avoid counting the same I/O more than once.
        let devices: Vec<_> = fs::read_dir("/sys/block")
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().join("device").exists())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        Self {
            cpu: fs::read_to_string("/proc/stat")
                .ok()
                .and_then(|text| cpu(&text)),
            memory: meminfo
                .as_deref()
                .and_then(|text| memory(text, "MemTotal", "MemAvailable")),
            swap: meminfo
                .as_deref()
                .and_then(|text| memory(text, "SwapTotal", "SwapFree")),
            network: fs::read_to_string("/proc/net/dev")
                .ok()
                .map(|text| network(&text)),
            disk: fs::read_to_string("/proc/diskstats")
                .ok()
                .map(|text| disks(&text, &devices)),
        }
    }
}

fn cpu_usage(previous: (u64, u64), current: (u64, u64)) -> Option<f64> {
    let total = current.0.checked_sub(previous.0)?;
    let idle = current.1.checked_sub(previous.1)?;
    (total > 0).then(|| 100.0 * total.saturating_sub(idle) as f64 / total as f64)
}

fn rates(previous: &Counters, current: &Counters, seconds: f64) -> [f64; 2] {
    let mut result = [0.0; 2];
    for (name, values) in current {
        if let Some(old) = previous.get(name) {
            for i in 0..2 {
                result[i] += values[i].saturating_sub(old[i]) as f64 / seconds;
            }
        }
    }
    result
}

fn bytes(value: f64) -> String {
    if value >= 1024.0_f64.powi(3) {
        format!("{:.1} GiB", value / 1024.0_f64.powi(3))
    } else if value >= 1024.0_f64.powi(2) {
        format!("{:.1} MiB", value / 1024.0_f64.powi(2))
    } else if value >= 1024.0 {
        format!("{:.1} KiB", value / 1024.0)
    } else {
        format!("{value:.0} B")
    }
}

struct Chart {
    area: gtk::DrawingArea,
    value: gtk::Label,
    detail: gtk::Label,
    scale: gtk::Label,
    history: Rc<RefCell<VecDeque<Option<[f64; 2]>>>>,
}

impl Chart {
    fn new(
        title: &str,
        legend: &str,
        color: Color,
        percent: bool,
        paired: bool,
    ) -> (gtk::Box, Self) {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
        card.add_css_class("monitor-card");
        card.set_hexpand(true);
        card.set_vexpand(true);
        let heading = gtk::Label::new(Some(title));
        heading.set_xalign(0.0);
        heading.add_css_class("metric-title");
        let value = gtk::Label::new(Some("Collecting…"));
        value.set_xalign(0.0);
        value.add_css_class("metric-value");
        let detail = gtk::Label::new(Some(legend));
        detail.set_xalign(0.0);
        detail.add_css_class("metric-detail");
        let area = gtk::DrawingArea::builder()
            .height_request(60)
            .hexpand(true)
            .vexpand(true)
            .build();
        let history = Rc::new(RefCell::new(VecDeque::<Option<[f64; 2]>>::new()));
        let samples = history.clone();
        area.set_draw_func(move |_, cr, width, height| {
            let samples = samples.borrow();
            let ceiling = ceiling(&samples, percent);
            let w = f64::from(width);
            let h = f64::from(height) - 4.0;
            cr.set_source_rgb(0.19, 0.23, 0.28);
            cr.set_line_width(1.0);
            for fraction in [0.0, 0.5, 1.0] {
                let y = 2.0 + fraction * h;
                cr.move_to(0.0, y);
                cr.line_to(w, y);
            }
            let _ = cr.stroke();
            for series in 0..if paired { 2 } else { 1 } {
                let (r, g, b) = if series == 0 { color } else { SECONDARY };
                cr.set_source_rgb(r, g, b);
                cr.set_line_width(2.0);
                if series == 1 {
                    cr.set_dash(&[5.0, 4.0], 0.0);
                }
                cr.set_line_join(cairo::LineJoin::Round);
                let mut connected = false;
                for (i, sample) in samples.iter().enumerate() {
                    let Some(values) = sample else {
                        connected = false;
                        continue;
                    };
                    let x = (HISTORY - samples.len() + i) as f64 / (HISTORY - 1) as f64 * w;
                    let y = 2.0 + h * (1.0 - (values[series] / ceiling).clamp(0.0, 1.0));
                    if connected {
                        cr.line_to(x, y);
                    } else {
                        cr.move_to(x, y);
                    }
                    connected = true;
                }
                let _ = cr.stroke();
            }
        });
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let time = gtk::Label::new(Some("2 minutes ago"));
        time.set_hexpand(true);
        time.set_xalign(0.0);
        let scale = gtk::Label::new(None);
        footer.append(&time);
        footer.append(&scale);
        footer.append(&gtk::Label::new(Some("now")));
        footer.add_css_class("chart-footer");
        card.append(&heading);
        card.append(&value);
        card.append(&detail);
        card.append(&area);
        card.append(&footer);
        (
            card,
            Self {
                area,
                value,
                detail,
                scale,
                history,
            },
        )
    }

    fn update(&self, sample: Option<[f64; 2]>, value: String, detail: String, percent: bool) {
        let mut history = self.history.borrow_mut();
        if history.len() == HISTORY {
            history.pop_front();
        }
        history.push_back(sample);
        let top = ceiling(&history, percent);
        self.scale.set_text(&if percent {
            "0–100%".into()
        } else {
            format!("max {}/s", bytes(top))
        });
        self.value.set_text(&value);
        self.detail.set_text(&detail);
        self.area.queue_draw();
    }
}

fn ceiling(history: &VecDeque<Option<[f64; 2]>>, percent: bool) -> f64 {
    if percent {
        100.0
    } else {
        history
            .iter()
            .flatten()
            .flat_map(|values| values.iter())
            .copied()
            .fold(1024.0, f64::max)
            * 1.1
    }
}

pub fn widget() -> gtk::Grid {
    let grid = gtk::Grid::builder()
        .row_spacing(16)
        .column_spacing(16)
        .column_homogeneous(true)
        .row_homogeneous(true)
        .hexpand(true)
        .vexpand(true)
        .build();
    let specs = [
        ("CPU", "All cores · busy time", false),
        (
            "MEMORY",
            "Used RAM · cache available to apps excluded",
            false,
        ),
        (
            "NETWORK",
            "Solid: receive · dashed: send · excludes loopback",
            true,
        ),
        (
            "DISK I/O",
            "Solid: read · dashed: write · physical disks",
            true,
        ),
    ];
    let mut charts = Vec::new();
    for (i, (title, legend, paired)) in specs.iter().enumerate() {
        let (card, chart) = Chart::new(title, legend, COLORS[i], i < 2, *paired);
        grid.attach(&card, (i % 2) as i32, (i / 2) as i32, 1, 1);
        charts.push(chart);
    }
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let mut previous = Snapshot::read();
        let mut last = Instant::now();
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let current = Snapshot::read();
            let now = Instant::now();
            let elapsed = now.duration_since(last).as_secs_f64();
            let usage = previous
                .cpu
                .zip(current.cpu)
                .and_then(|(old, new)| cpu_usage(old, new));
            let net = previous
                .network
                .as_ref()
                .zip(current.network.as_ref())
                .map(|(old, new)| rates(old, new, elapsed));
            let disk = previous
                .disk
                .as_ref()
                .zip(current.disk.as_ref())
                .map(|(old, new)| rates(old, new, elapsed));
            if tx
                .send_blocking((usage, current.memory, current.swap, net, disk))
                .is_err()
            {
                break;
            }
            previous = current;
            last = now;
        }
    });
    let weak = grid.downgrade();
    glib::spawn_future_local(async move {
        while let Ok((cpu, mem, swap, net, disk)) = rx.recv().await {
            if weak.upgrade().is_none() {
                break;
            }
            charts[0].update(
                cpu.map(|v| [v, 0.0]),
                cpu.map_or("Unavailable".into(), |v| format!("{v:.0}%")),
                specs[0].1.into(),
                true,
            );
            let mem_sample = mem.filter(|(_, total)| *total > 0);
            charts[1].update(
                mem_sample.map(|(used, total)| [100.0 * used as f64 / total as f64, 0.0]),
                mem_sample.map_or("Unavailable".into(), |(used, total)| {
                    format!("{} / {}", bytes(used as f64), bytes(total as f64))
                }),
                format!(
                    "RAM excludes available cache · {}",
                    swap.map_or("swap unavailable".into(), |(used, total)| if total == 0 {
                        "no swap".into()
                    } else {
                        format!("swap {} / {}", bytes(used as f64), bytes(total as f64))
                    })
                ),
                true,
            );
            for (i, sample, first, second) in [(2, net, "↓", "↑"), (3, disk, "R", "W")] {
                charts[i].update(
                    sample,
                    sample.map_or("Unavailable".into(), |v| {
                        format!("{first} {}/s   {second} {}/s", bytes(v[0]), bytes(v[1]))
                    }),
                    specs[i].1.into(),
                    false,
                );
            }
        }
    });
    grid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_excludes_guest_and_counts_iowait_as_idle() {
        assert_eq!(
            cpu("cpu  100 20 30 400 50 6 7 8 10 2\ncpu0 1 2 3"),
            Some((621, 450))
        );
        assert_eq!(cpu_usage((100, 50), (200, 125)), Some(25.0));
        assert_eq!(cpu_usage((100, 50), (100, 50)), None);
        assert_eq!(cpu_usage((100, 50), (90, 40)), None);
    }

    #[test]
    fn memory_uses_available_instead_of_free() {
        let text = "MemTotal: 1000 kB\nMemFree: 10 kB\nMemAvailable: 400 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n";
        assert_eq!(
            memory(text, "MemTotal", "MemAvailable"),
            Some((600 * 1024, 1000 * 1024))
        );
        assert_eq!(memory(text, "SwapTotal", "SwapFree"), Some((0, 0)));
        assert_eq!(memory("", "MemTotal", "MemAvailable"), None);
    }

    #[test]
    fn rates_handle_hotplug_resets_and_actual_elapsed_time() {
        let old = Counters::from([("eth0".into(), [100, 200]), ("gone".into(), [999, 999])]);
        let new = Counters::from([("eth0".into(), [200, 20]), ("new".into(), [999, 999])]);
        assert_eq!(rates(&old, &new, 2.0), [50.0, 0.0]);
    }

    #[test]
    fn network_excludes_loopback_and_disk_counts_only_selected_devices() {
        assert_eq!(
            network("lo: 999 0 0 0 0 0 0 0 999\nwlan0: 100 0 0 0 0 0 0 0 200").len(),
            1
        );
        let text = "259 0 nvme0n1 1 0 10 0 2 0 20 0\n259 1 nvme0n1p1 1 0 10 0 2 0 20 0\n254 0 dm-0 1 0 10 0 2 0 20 0";
        assert_eq!(
            disks(text, &["nvme0n1".into()]),
            Counters::from([("nvme0n1".into(), [5120, 10240])])
        );
    }
}
