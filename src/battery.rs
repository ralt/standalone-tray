use futures_util::StreamExt;
use gtk::glib;
use gtk::prelude::*;

use crate::ICON_SIZE;

const UPOWER: &str = "org.freedesktop.UPower";
const DISPLAY_DEVICE: &str = "/org/freedesktop/UPower/devices/DisplayDevice";

// org.freedesktop.UPower.Device.State values.
const STATE_CHARGING: u32 = 1;
const STATE_DISCHARGING: u32 = 2;
const STATE_FULLY_CHARGED: u32 = 4;

struct Snapshot {
    present: bool,
    percentage: f64,
    state: u32,
    icon_name: String,
    time_to_empty: i64,
    time_to_full: i64,
}

/// A battery widget (icon + percentage) backed by UPower's aggregate display
/// device. Hidden when UPower is unavailable or reports no battery.
pub fn widget() -> gtk::Box {
    let container = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(2)
        .visible(false)
        .build();
    let image = gtk::Image::new();
    image.set_pixel_size(ICON_SIZE);
    let label = gtk::Label::new(None);
    container.append(&image);
    container.append(&label);

    let (tx, rx) = async_channel::unbounded::<Snapshot>();
    crate::runtime().spawn(watch(tx));

    let widget = container.clone();
    glib::spawn_future_local(async move {
        while let Ok(snapshot) = rx.recv().await {
            apply(&widget, &image, &label, &snapshot);
        }
    });

    container
}

fn apply(container: &gtk::Box, image: &gtk::Image, label: &gtk::Label, snapshot: &Snapshot) {
    if std::env::var_os("TRAY_DEBUG").is_some() {
        eprintln!(
            "standalone-tray: battery: present={} {:.0}% state={} icon={}",
            snapshot.present, snapshot.percentage, snapshot.state, snapshot.icon_name
        );
    }
    container.set_visible(snapshot.present);
    if !snapshot.present {
        return;
    }

    if snapshot.icon_name.is_empty() {
        image.set_icon_name(Some("battery-symbolic"));
    } else {
        image.set_icon_name(Some(&snapshot.icon_name));
    }

    let percent = snapshot.percentage.round() as i32;
    label.set_text(&format!("{percent}%"));

    let tooltip = match snapshot.state {
        STATE_CHARGING if snapshot.time_to_full > 0 => format!(
            "Battery: {percent}% (charging, {} until full)",
            duration(snapshot.time_to_full)
        ),
        STATE_CHARGING => format!("Battery: {percent}% (charging)"),
        STATE_DISCHARGING if snapshot.time_to_empty > 0 => format!(
            "Battery: {percent}% ({} remaining)",
            duration(snapshot.time_to_empty)
        ),
        STATE_FULLY_CHARGED => format!("Battery: {percent}% (full)"),
        _ => format!("Battery: {percent}%"),
    };
    container.set_tooltip_text(Some(&tooltip));
}

fn duration(seconds: i64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    if hours > 0 {
        format!("{hours}h{minutes:02}m")
    } else {
        format!("{minutes}m")
    }
}

async fn read(device: &zbus::Proxy<'_>) -> zbus::Result<Snapshot> {
    Ok(Snapshot {
        present: device.get_property("IsPresent").await?,
        percentage: device.get_property("Percentage").await?,
        state: device.get_property("State").await?,
        icon_name: device.get_property("IconName").await?,
        time_to_empty: device.get_property("TimeToEmpty").await?,
        time_to_full: device.get_property("TimeToFull").await?,
    })
}

async fn watch(tx: async_channel::Sender<Snapshot>) {
    let result = async {
        let connection = zbus::Connection::system().await?;
        let device = zbus::Proxy::new(
            &connection,
            UPOWER,
            DISPLAY_DEVICE,
            "org.freedesktop.UPower.Device",
        )
        .await?;
        let properties = zbus::fdo::PropertiesProxy::builder(&connection)
            .destination(UPOWER)?
            .path(DISPLAY_DEVICE)?
            .build()
            .await?;
        let mut changes = properties.receive_properties_changed().await?;

        if tx.send(read(&device).await?).await.is_err() {
            return Ok(());
        }
        while changes.next().await.is_some() {
            if tx.send(read(&device).await?).await.is_err() {
                break;
            }
        }
        Ok::<_, zbus::Error>(())
    }
    .await;
    if let Err(e) = result {
        eprintln!("standalone-tray: battery widget disabled: {e}");
    }
}
