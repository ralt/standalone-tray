use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

/// Local wall clock and a browsable month calendar, with no external service.
pub fn widget() -> gtk::Box {
    let panel = gtk::Box::new(gtk::Orientation::Vertical, 12);
    panel.add_css_class("calendar-panel");
    panel.set_valign(gtk::Align::Start);
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
    panel.append(&calendar);
    panel.append(&today);

    let update = move || {
        if let Ok(now) = glib::DateTime::now_local() {
            if let Ok(value) = now.format("%H:%M:%S") {
                time.set_text(&value);
            }
            if let Ok(value) = now.format("%A\n%e %B %Y") {
                date.set_text(&value);
            }
            if let Ok(value) = now.format("%Z · UTC%:z") {
                timezone.set_text(&value);
            }
            // Mark today's date without moving the month the user is browsing.
            calendar.clear_marks();
            let shown = calendar.date();
            if shown.year() == now.year() && shown.month() == now.month() {
                calendar.mark_day(now.day_of_month() as u32);
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
