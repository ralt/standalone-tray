use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

const WORLD_ZONES: [(&str, &str); 5] = [
    ("UTC", "UTC"),
    ("New York", "America/New_York"),
    ("California", "America/Los_Angeles"),
    ("India", "Asia/Kolkata"),
    ("Australia · Sydney", "Australia/Sydney"),
];

struct WorldClock {
    zone: Option<glib::TimeZone>,
    time: gtk::Label,
    date: gtk::Label,
}

fn named_zone(identifier: &str) -> Option<glib::TimeZone> {
    let zone = glib::TimeZone::new(Some(identifier));
    // Older GLib falls back to UTC for unknown identifiers. Show unavailable
    // instead of silently displaying the wrong time when tzdata is missing.
    (zone.identifier() == identifier).then_some(zone)
}

fn set_text(label: &gtk::Label, text: &str) {
    if label.text() != text {
        label.set_text(text);
    }
}

impl WorldClock {
    fn new(city: &str, identifier: &str) -> (gtk::Box, Self) {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
        row.add_css_class("world-clock");
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let name = gtk::Label::new(Some(city));
        name.set_hexpand(true);
        name.set_xalign(0.0);
        name.add_css_class("world-clock-city");
        let time = gtk::Label::new(Some("—"));
        time.add_css_class("world-clock-time");
        time.set_width_chars(8);
        time.set_xalign(1.0);
        top.append(&name);
        top.append(&time);
        let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let zone = gtk::Label::new(Some(identifier));
        zone.set_hexpand(true);
        zone.set_xalign(0.0);
        let date = gtk::Label::new(None);
        bottom.append(&zone);
        bottom.append(&date);
        bottom.add_css_class("world-clock-detail");
        row.append(&top);
        row.append(&bottom);
        (
            row,
            Self {
                zone: named_zone(identifier),
                time,
                date,
            },
        )
    }

    fn update(&self, now: &glib::DateTime) {
        let Some(local) = self
            .zone
            .as_ref()
            .and_then(|zone| now.to_timezone(zone).ok())
        else {
            set_text(&self.time, "Unavailable");
            return;
        };
        if let Ok(time) = local.format("%H:%M:%S") {
            set_text(&self.time, &time);
        }
        if let Ok(date) = local.format("%a %d %b") {
            set_text(&self.date, &date);
        }
        if let Ok(offset) = local.format("%Z · UTC%:z") {
            if self.time.tooltip_text().as_deref() != Some(offset.as_str()) {
                self.time.set_tooltip_text(Some(&offset));
            }
        }
    }
}

/// Local and world clocks with a browsable month calendar. All clocks share
/// one timer and one instant; GLib's timezone data supplies DST and offsets.
pub fn widget() -> gtk::Box {
    let panel = gtk::Box::new(gtk::Orientation::Vertical, 8);
    panel.add_css_class("calendar-panel");
    panel.set_valign(gtk::Align::Start);
    panel.set_hexpand(false);
    panel.set_size_request(280, -1);

    let title = gtk::Label::new(Some("LOCAL TIME"));
    title.set_xalign(0.0);
    title.add_css_class("metric-title");
    let time = gtk::Label::new(None);
    time.set_xalign(0.0);
    time.add_css_class("clock-time");
    let date = gtk::Label::new(None);
    date.set_xalign(0.0);
    date.add_css_class("clock-date");
    let timezone = gtk::Label::new(None);
    timezone.set_xalign(0.0);
    timezone.add_css_class("metric-detail");
    let calendar = gtk::Calendar::new();
    calendar.set_show_week_numbers(true);
    calendar.set_tooltip_text(Some(
        "Browse months with the arrows; Today returns to the current date.",
    ));
    let today = gtk::Button::with_label("Today");
    let calendar_ref = calendar.downgrade();
    today.connect_clicked(move |_| {
        if let (Some(calendar), Ok(now)) = (calendar_ref.upgrade(), glib::DateTime::now_local()) {
            calendar.select_day(&now);
        }
    });

    panel.append(&title);
    panel.append(&time);
    panel.append(&date);
    panel.append(&timezone);
    panel.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let clocks = gtk::Box::new(gtk::Orientation::Vertical, 8);
    clocks.add_css_class("world-clocks");
    let mut world_clocks = Vec::new();
    for (city, identifier) in WORLD_ZONES {
        let (row, clock) = WorldClock::new(city, identifier);
        clocks.append(&row);
        world_clocks.push(clock);
    }
    panel.append(&clocks);
    panel.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    panel.append(&calendar);
    panel.append(&today);

    let mut marked = None;
    let mut update = move || {
        if let Ok(now) = glib::DateTime::now_local() {
            if let Ok(value) = now.format("%H:%M:%S") {
                set_text(&time, &value);
            }
            if let Ok(value) = now.format("%A\n%e %B %Y") {
                set_text(&date, &value);
            }
            if let Ok(value) = now.format("%Z · UTC%:z") {
                set_text(
                    &timezone,
                    &format!("{} · {value}", now.timezone().identifier()),
                );
            }
            for clock in &world_clocks {
                clock.update(&now);
            }
            // Redecorate the calendar only when the date or browsed month changes.
            let shown = calendar.date();
            let key = (
                now.year(),
                now.month(),
                now.day_of_month(),
                shown.year(),
                shown.month(),
            );
            if marked != Some(key) {
                calendar.clear_marks();
                if shown.year() == now.year() && shown.month() == now.month() {
                    calendar.mark_day(now.day_of_month() as u32);
                }
                marked = Some(key);
            }
        }
    };
    update();
    let weak = panel.downgrade();
    glib::timeout_add_local(Duration::from_secs(1), move || {
        if weak.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        update();
        glib::ControlFlow::Continue
    });
    panel
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall_time(instant: &str, zone: &str) -> String {
        glib::DateTime::from_iso8601(instant, None)
            .unwrap()
            .to_timezone(&named_zone(zone).unwrap())
            .unwrap()
            .format("%Y-%m-%d %H:%M %:z")
            .unwrap()
            .to_string()
    }

    #[test]
    fn world_zones_include_dst_half_hour_offsets_and_next_day() {
        let winter = "2026-01-15T20:00:00Z";
        assert_eq!(wall_time(winter, "UTC"), "2026-01-15 20:00 +00:00");
        assert_eq!(
            wall_time(winter, "America/New_York"),
            "2026-01-15 15:00 -05:00"
        );
        assert_eq!(
            wall_time(winter, "America/Los_Angeles"),
            "2026-01-15 12:00 -08:00"
        );
        assert_eq!(wall_time(winter, "Asia/Kolkata"), "2026-01-16 01:30 +05:30");
        assert_eq!(
            wall_time(winter, "Australia/Sydney"),
            "2026-01-16 07:00 +11:00"
        );
        let summer = "2026-07-15T20:00:00Z";
        assert_eq!(
            wall_time(summer, "America/New_York"),
            "2026-07-15 16:00 -04:00"
        );
        assert_eq!(
            wall_time(summer, "America/Los_Angeles"),
            "2026-07-15 13:00 -07:00"
        );
        assert_eq!(
            wall_time(summer, "Australia/Sydney"),
            "2026-07-16 06:00 +10:00"
        );
        assert_eq!(wall_time(summer, "Europe/Paris"), "2026-07-15 22:00 +02:00");
    }

    #[test]
    fn unknown_timezones_are_not_silently_shown_as_utc() {
        assert!(named_zone("Invalid/Timezone").is_none());
    }
    #[test]
    #[ignore = "requires a graphical display"]
    fn clocks_fit_compact_sidebar() {
        gtk::init().unwrap();
        let panel = widget();
        let win = gtk::Window::new();
        win.add_css_class("dashboard");
        let css = gtk::CssProvider::new();
        css.load_from_data(include_str!("style.css"));
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().unwrap(),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        let fill = gtk::Label::new(None);
        fill.set_hexpand(true);
        body.append(&fill);
        body.append(&panel);
        win.set_child(Some(&body));
        win.present();
        let context = glib::MainContext::default();
        let end = std::time::Instant::now() + Duration::from_millis(500);
        while std::time::Instant::now() < end {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        fn check(widget: &gtk::Widget) -> usize {
            let mut count = 0;
            if widget.has_css_class("world-clock-time") {
                let label = widget.downcast_ref::<gtk::Label>().unwrap();
                assert_eq!(label.text().len(), 8);
                assert!(
                    widget.width() >= widget.measure(gtk::Orientation::Horizontal, -1).1,
                    "full time must fit in the compact column"
                );
                count += 1;
            }
            let mut child = widget.first_child();
            while let Some(current) = child {
                count += check(&current);
                child = current.next_sibling();
            }
            count
        }
        assert_eq!(check(panel.upcast_ref()), 5);
        assert!(panel.width() < 350, "sidebar must leave room for graphs");
        win.close();
    }
}
