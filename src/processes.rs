//! A read-only process view. CPU percentages follow htop: 100% is one busy core.
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs;
use std::rc::Rc;
use std::time::Duration;

use glib::subclass::prelude::*;
use gtk::prelude::*;
use gtk::{gio, glib};

#[derive(Clone, Debug, PartialEq)]
struct Process {
    pid: u32,
    user: String,
    state: String,
    command: String,
    ticks: u64,
    started: u64,
    cpu: Option<f64>,
    rss: u64,
    memory: f64,
    threads: u32,
    nice: i32,
}

fn parse_stat(text: &str) -> Option<Process> {
    // comm may itself contain spaces and parentheses; the last ')' ends it.
    let (pid, rest) = text.split_once('(')?;
    let end = rest.rfind(')')?;
    let fields: Vec<_> = rest[end + 1..].split_whitespace().collect();
    Some(Process {
        pid: pid.trim().parse().ok()?,
        command: rest[..end].to_string(),
        state: fields.first()?.to_string(),
        ticks: fields
            .get(11)?
            .parse::<u64>()
            .ok()?
            .checked_add(fields.get(12)?.parse().ok()?)?,
        nice: fields.get(16)?.parse().ok()?,
        threads: fields.get(17)?.parse().ok()?,
        started: fields.get(19)?.parse().ok()?,
        user: String::new(),
        cpu: None,
        rss: 0,
        memory: 0.0,
    })
}

fn status_value(text: &str, key: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name != key {
            return None;
        }
        value.split_whitespace().next()?.parse().ok()
    })
}

fn cpu_clock(text: &str) -> Option<(u64, usize)> {
    let total = text
        .lines()
        .find(|line| line.starts_with("cpu "))?
        .split_whitespace()
        .skip(1)
        .take(8)
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if total.len() != 8 {
        return None;
    }
    let cores = text
        .lines()
        .filter(|line| {
            line.strip_prefix("cpu")
                .and_then(|rest| rest.chars().next())
                .is_some_and(|c| c.is_ascii_digit())
        })
        .count();
    (cores > 0).then(|| (total.iter().sum(), cores))
}

fn cpu_percent(old: &Process, current: &Process, elapsed_ticks: u64, cores: usize) -> Option<f64> {
    if old.started != current.started || elapsed_ticks == 0 {
        return None;
    }
    Some(100.0 * current.ticks.checked_sub(old.ticks)? as f64 * cores as f64 / elapsed_ticks as f64)
}

struct Sampler {
    previous: HashMap<u32, Process>,
    clock: Option<(u64, usize)>,
    users: HashMap<u64, String>,
}

impl Sampler {
    fn new() -> Self {
        let users = fs::read_to_string("/etc/passwd")
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                let fields: Vec<_> = line.split(':').collect();
                Some((fields.get(2)?.parse().ok()?, fields.first()?.to_string()))
            })
            .collect();
        Self {
            previous: HashMap::new(),
            clock: None,
            users,
        }
    }

    fn sample(&mut self) -> std::io::Result<Vec<Process>> {
        let clock = fs::read_to_string("/proc/stat")
            .ok()
            .and_then(|text| cpu_clock(&text));
        let elapsed = self.clock.zip(clock).and_then(|(old, new)| {
            // A CPU hotplug changes the denominator; wait for a stable interval.
            (old.1 == new.1).then(|| new.0.checked_sub(old.0)).flatten()
        });
        let total_memory = fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|text| status_value(&text, "MemTotal"))
            .unwrap_or(0)
            * 1024;
        let mut current = HashMap::new();
        for entry in fs::read_dir("/proc")?.filter_map(Result::ok) {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
                continue;
            };
            let path = entry.path();
            let Some(mut process) = fs::read_to_string(path.join("stat"))
                .ok()
                .and_then(|text| parse_stat(&text))
            else {
                continue;
            };
            let Ok(status) = fs::read_to_string(path.join("status")) else {
                continue;
            };
            let Some(uid) = status_value(&status, "Uid") else {
                continue;
            };
            process.user = self
                .users
                .get(&uid)
                .cloned()
                .unwrap_or_else(|| uid.to_string());
            process.rss = status_value(&status, "VmRSS").unwrap_or(0) * 1024;
            if total_memory > 0 {
                process.memory = 100.0 * process.rss as f64 / total_memory as f64;
            }
            if let Ok(command) = fs::read(path.join("cmdline")) {
                let command = String::from_utf8_lossy(&command).replace('\0', " ");
                if !command.trim().is_empty() {
                    process.command = command.trim().to_string();
                }
            }
            if let (Some(old), Some(ticks), Some((_, cores))) =
                (self.previous.get(&pid), elapsed, clock)
            {
                process.cpu = cpu_percent(old, &process, ticks, cores);
            }
            // Verify identity after the multi-file read: never mix a reused PID's data.
            let identity = fs::read_to_string(path.join("stat"))
                .ok()
                .and_then(|text| parse_stat(&text));
            if identity.is_some_and(|check| check.started == process.started) {
                current.insert(pid, process);
            }
        }
        self.previous = current;
        self.clock = clock;
        Ok(self.previous.values().cloned().collect())
    }
}

mod row {
    use super::*;
    use std::cell::Cell;

    #[derive(Default, glib::Properties)]
    #[properties(wrapper_type = super::ProcessRow)]
    pub struct ProcessRow {
        #[property(get)]
        pub(super) revision: Cell<u64>,
        pub(super) process: RefCell<Option<Process>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ProcessRow {
        const NAME: &'static str = "StandaloneTrayProcessRow";
        type Type = super::ProcessRow;
    }

    #[glib::derived_properties]
    impl ObjectImpl for ProcessRow {}
}

glib::wrapper! {
    pub struct ProcessRow(ObjectSubclass<row::ProcessRow>);
}

impl ProcessRow {
    fn new(process: Process) -> Self {
        let row: Self = glib::Object::new();
        row.imp().process.replace(Some(process));
        row
    }

    fn data(&self) -> std::cell::Ref<'_, Process> {
        std::cell::Ref::map(self.imp().process.borrow(), |p| p.as_ref().unwrap())
    }

    fn update(&self, process: Process) {
        let changed = {
            let old = self.data();
            let tenth = |v: f64| (v * 10.0).round() as i64;
            old.pid != process.pid
                || old.user != process.user
                || old.state != process.state
                || old.command != process.command
                || old.cpu.map(tenth) != process.cpu.map(tenth)
                || tenth(old.memory) != tenth(process.memory)
                || old.rss != process.rss
                || old.threads != process.threads
                || old.nice != process.nice
        };
        self.imp().process.replace(Some(process));
        if changed {
            self.imp().revision.set(self.revision().wrapping_add(1));
            self.notify_revision();
        }
    }
}

struct Table {
    snapshot: Vec<Process>,
    query: String,
    store: gio::ListStore,
    sorter: gtk::Sorter,
    count: gtk::Label,
}

impl Table {
    fn refresh(&self) {
        let mut rows: Vec<_> = self
            .snapshot
            .iter()
            .filter(|p| {
                self.query.is_empty()
                    || p.command.to_lowercase().contains(&self.query)
                    || p.user.to_lowercase().contains(&self.query)
                    || p.pid.to_string().contains(&self.query)
            })
            .cloned()
            .map(ProcessRow::new)
            .collect();
        rows.sort_unstable_by(|a, b| self.sorter.compare(a, b).into());
        let length = rows.len() as u32;
        for (i, process) in rows.into_iter().enumerate() {
            if let Some(slot) = self.store.item(i as u32).and_downcast::<ProcessRow>() {
                slot.update(process.data().clone());
            } else {
                self.store.append(&process);
            }
        }
        if length < self.store.n_items() {
            self.store
                .splice(length, self.store.n_items() - length, &[] as &[ProcessRow]);
        }
        self.count
            .set_text(&format!("{length} / {}", self.snapshot.len()));
    }
}

#[derive(Clone, Copy)]
enum Field {
    Pid,
    User,
    State,
    Cpu,
    Memory,
    Rss,
    Threads,
    Nice,
    Command,
}

impl Field {
    fn text(self, p: &Process) -> String {
        match self {
            Self::Pid => p.pid.to_string(),
            Self::User => p.user.clone(),
            Self::State => p.state.clone(),
            Self::Cpu => p.cpu.map_or("—".into(), |value| format!("{value:.1}")),
            Self::Memory => format!("{:.1}", p.memory),
            Self::Rss => {
                if p.rss >= 1024 * 1024 * 1024 {
                    format!("{:.2} GiB", p.rss as f64 / 1073741824.0)
                } else {
                    format!("{:.1} MiB", p.rss as f64 / 1048576.0)
                }
            }
            Self::Threads => p.threads.to_string(),
            Self::Nice => p.nice.to_string(),
            Self::Command => p.command.clone(),
        }
    }

    fn compare(self, a: &Process, b: &Process) -> Ordering {
        let order = match self {
            Self::Pid => a.pid.cmp(&b.pid),
            Self::User => a.user.cmp(&b.user),
            Self::State => a.state.cmp(&b.state),
            Self::Cpu => a.cpu.unwrap_or(-1.0).total_cmp(&b.cpu.unwrap_or(-1.0)),
            Self::Memory | Self::Rss => a.rss.cmp(&b.rss),
            Self::Threads => a.threads.cmp(&b.threads),
            Self::Nice => a.nice.cmp(&b.nice),
            Self::Command => a.command.cmp(&b.command),
        };
        order.then_with(|| a.pid.cmp(&b.pid))
    }
}

fn column(title: &str, field: Field, width: i32) -> gtk::ColumnViewColumn {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, item| {
        let label = gtk::Label::new(None);
        let textual = matches!(field, Field::User | Field::State | Field::Command);
        label.set_xalign(if textual { 0.0 } else { 1.0 });
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.add_css_class("process-cell");
        if matches!(field, Field::Cpu) {
            label.add_css_class("process-cpu");
        }
        if matches!(field, Field::Memory | Field::Rss) {
            label.add_css_class("process-memory");
        }
        if matches!(field, Field::Command) {
            label.connect_label_notify(|label| label.set_tooltip_text(Some(&label.text())));
        }
        item.set_child(Some(&label));
    });
    let bindings = Rc::new(RefCell::new(HashMap::<usize, glib::Binding>::new()));
    let active = bindings.clone();
    factory.connect_bind(move |_, item| {
        let object = item.item().and_downcast::<ProcessRow>().unwrap();
        let label = item.child().and_downcast::<gtk::Label>().unwrap();
        let binding = object
            .bind_property("revision", &label, "label")
            .transform_to(move |binding, _: u64| {
                let row = binding.source()?.downcast::<ProcessRow>().ok()?;
                let text = field.text(&row.data());
                Some(text)
            })
            .sync_create()
            .build();
        active.borrow_mut().insert(item.as_ptr() as usize, binding);
    });
    factory.connect_unbind(move |_, item| {
        if let Some(binding) = bindings.borrow_mut().remove(&(item.as_ptr() as usize)) {
            binding.unbind();
        }
    });
    let column = gtk::ColumnViewColumn::new(Some(title), Some(factory));
    column.set_fixed_width(width);
    column.set_resizable(true);
    column.set_expand(matches!(field, Field::Command));
    column.set_sorter(Some(&gtk::CustomSorter::new(move |a, b| {
        let a = a.downcast_ref::<ProcessRow>().unwrap().data();
        let b = b.downcast_ref::<ProcessRow>().unwrap().data();
        field.compare(&a, &b).into()
    })));
    column
}

pub fn widget() -> gtk::Box {
    let panel = gtk::Box::new(gtk::Orientation::Vertical, 10);
    panel.add_css_class("process-panel");
    panel.set_vexpand(true);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    let title = gtk::Label::new(Some("PROCESSES"));
    title.add_css_class("metric-title");
    let count = gtk::Label::new(Some("Collecting…"));
    count.add_css_class("metric-detail");
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Filter by name, user or PID"));
    search.set_hexpand(true);
    header.append(&title);
    header.append(&count);
    header.append(&search);
    panel.append(&header);
    let hint = gtk::Label::new(Some(
        "2s refresh · 100% CPU = one core · RAM = resident memory · click a heading to sort",
    ));
    hint.set_xalign(0.0);
    hint.add_css_class("dashboard-subtitle");
    panel.append(&hint);

    // Slots represent row positions, not process identities. Updating their data
    // preserves native cell widgets and the numeric scroll position as ranks change.
    let store = gio::ListStore::new::<ProcessRow>();
    let view = gtk::ColumnView::new(Some(gtk::NoSelection::new(Some(store.clone()))));
    view.add_css_class("process-table");
    view.set_show_row_separators(false);
    for (title, field, width) in [
        ("PID", Field::Pid, 75),
        ("USER", Field::User, 95),
        ("S", Field::State, 35),
        ("CPU %", Field::Cpu, 80),
        ("RAM %", Field::Memory, 80),
        ("RES", Field::Rss, 100),
        ("THR", Field::Threads, 55),
        ("NI", Field::Nice, 45),
        ("COMMAND", Field::Command, 240),
    ] {
        let col = column(title, field, width);
        view.append_column(&col);
        if matches!(field, Field::Cpu) {
            view.sort_by_column(Some(&col), gtk::SortType::Descending);
        }
    }
    let table = Rc::new(RefCell::new(Table {
        snapshot: Vec::new(),
        query: String::new(),
        store,
        sorter: view.sorter().unwrap(),
        count,
    }));
    let state = table.clone();
    search.connect_search_changed(move |entry| {
        let mut state = state.borrow_mut();
        state.query = entry.text().to_lowercase();
        state.refresh();
    });
    let weak = Rc::downgrade(&table);
    table.borrow().sorter.connect_changed(move |_, _| {
        if let Some(state) = weak.upgrade() {
            state.borrow().refresh();
        }
    });
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(190)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&view)
        .build();
    panel.append(&scroll);

    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let mut sampler = Sampler::new();
        loop {
            if tx.send_blocking(sampler.sample()).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
    let weak = panel.downgrade();
    glib::spawn_future_local(async move {
        while let Ok(result) = rx.recv().await {
            if weak.upgrade().is_none() {
                break;
            }
            match result {
                Ok(processes) => {
                    let mut state = table.borrow_mut();
                    state.snapshot = processes;
                    state.refresh();
                }
                Err(_) => {
                    let state = table.borrow();
                    state.store.remove_all();
                    state.count.set_text("/proc unavailable");
                }
            }
        }
    });
    panel
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(pid: u32, name: &str, user: u64, system: u64, started: u64) -> String {
        let mut fields = vec!["0".to_string(); 22];
        fields[0] = "R".into();
        fields[11] = user.to_string();
        fields[12] = system.to_string();
        fields[16] = "-5".into();
        fields[17] = "7".into();
        fields[19] = started.to_string();
        format!("{pid} ({name}) {}", fields.join(" "))
    }

    #[test]
    fn parses_names_with_parentheses_and_spaces_and_stat_fields() {
        let process = parse_stat(&stat(42, "a (busy) worker)", 120, 30, 999)).unwrap();
        assert_eq!(process.command, "a (busy) worker)");
        assert_eq!(
            (
                process.pid,
                process.ticks,
                process.started,
                process.threads,
                process.nice
            ),
            (42, 150, 999, 7, -5)
        );
        assert!(parse_stat("42 (truncated) R").is_none());
    }

    #[test]
    fn cpu_matches_htop_and_rejects_pid_reuse_and_counter_resets() {
        let old = parse_stat(&stat(42, "worker", 100, 0, 9)).unwrap();
        let mut current = parse_stat(&stat(42, "worker", 250, 0, 9)).unwrap();
        assert_eq!(cpu_percent(&old, &current, 400, 4), Some(150.0));
        assert_eq!(cpu_percent(&old, &current, 0, 4), None);
        current.started = 10;
        assert_eq!(cpu_percent(&old, &current, 400, 4), None);
        current.started = 9;
        current.ticks = 1;
        assert_eq!(cpu_percent(&old, &current, 400, 4), None);
    }

    #[test]
    fn clock_excludes_guest_ticks_and_status_uses_correct_units() {
        assert_eq!(
            cpu_clock("cpu 100 20 30 400 50 6 7 8 10 2\ncpu0 1\ncpu1 2"),
            Some((621, 2))
        );
        assert_eq!(
            status_value("Uid:\t1000 1000 1000 1000\nVmRSS: 2048 kB", "VmRSS"),
            Some(2048)
        );
        assert_eq!(status_value("Threads: 3", "VmRSS"), None);
    }

    #[test]
    #[ignore = "requires a graphical display"]
    fn live_view_sorts_and_filters_across_refreshes() {
        gtk::init().unwrap();
        let panel = widget();
        let scroll = panel
            .last_child()
            .unwrap()
            .downcast::<gtk::ScrolledWindow>()
            .unwrap();
        let view = scroll
            .child()
            .unwrap()
            .downcast::<gtk::ColumnView>()
            .unwrap();
        let search = panel
            .first_child()
            .unwrap()
            .last_child()
            .unwrap()
            .downcast::<gtk::SearchEntry>()
            .unwrap();
        let model = view.model().unwrap();
        let window = gtk::Window::new();
        window.set_default_size(1100, 400);
        window.set_child(Some(&panel));
        window.present();
        let pump = |milliseconds| {
            let end = std::time::Instant::now() + Duration::from_millis(milliseconds);
            let context = glib::MainContext::default();
            while std::time::Instant::now() < end {
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        pump(2400);
        assert!(model.n_items() > 0, "must show real processes");
        assert!(
            scroll.vadjustment().value() < 1.0,
            "must open at busiest rows"
        );
        let first_slot = model.item(0).unwrap();
        let cpu: Vec<_> = (0..model.n_items())
            .map(|i| {
                model
                    .item(i)
                    .unwrap()
                    .downcast::<ProcessRow>()
                    .unwrap()
                    .data()
                    .cpu
                    .unwrap_or(-1.0)
            })
            .collect();
        assert!(cpu.windows(2).all(|pair| pair[0] >= pair[1]));
        let column = view
            .columns()
            .item(5)
            .unwrap()
            .downcast::<gtk::ColumnViewColumn>()
            .unwrap();
        view.sort_by_column(Some(&column), gtk::SortType::Descending);
        pump(2200);
        assert_eq!(
            model.item(0).unwrap(),
            first_slot,
            "sort and refresh must reuse row slots"
        );
        assert!(
            scroll.vadjustment().value() < 1.0,
            "refresh must keep the viewport at the top"
        );
        let memory: Vec<_> = (0..model.n_items())
            .map(|i| {
                model
                    .item(i)
                    .unwrap()
                    .downcast::<ProcessRow>()
                    .unwrap()
                    .data()
                    .rss
            })
            .collect();
        assert!(memory.windows(2).all(|pair| pair[0] >= pair[1]));
        search.set_text("no-process-matches-this-filter-xyz");
        pump(2300);
        assert_eq!(model.n_items(), 0, "filter must survive live refresh");
        search.set_text("");
        pump(300);
        assert!(model.n_items() > 0);
        window.close();
    }
}
