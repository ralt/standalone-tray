use gtk::glib;
use gtk::prelude::*;
use tokio::io::AsyncBufReadExt;

use crate::ICON_SIZE;

const SINK: &str = "@DEFAULT_AUDIO_SINK@";

#[derive(Clone, Copy, PartialEq)]
struct State {
    volume: f64,
    muted: bool,
}

/// A volume widget (icon + percentage) for the default PipeWire/PulseAudio
/// sink. Left click toggles mute, scrolling adjusts the volume.
/// Hidden when no audio server is available.
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

    let gesture = gtk::GestureClick::new();
    gesture.connect_pressed(|_, _, _, _| run_wpctl(&["set-mute", SINK, "toggle"]));
    container.add_controller(gesture);

    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    scroll.connect_scroll(|_, _, dy| {
        let step = if dy < 0.0 { "5%+" } else { "5%-" };
        run_wpctl(&["set-volume", "-l", "1.0", SINK, step]);
        glib::Propagation::Stop
    });
    container.add_controller(scroll);

    let (tx, rx) = async_channel::unbounded::<Option<State>>();
    crate::runtime().spawn(watch(tx));

    let widget = container.clone();
    glib::spawn_future_local(async move {
        while let Ok(state) = rx.recv().await {
            apply(&widget, &image, &label, state);
        }
    });

    container
}

fn apply(container: &gtk::Box, image: &gtk::Image, label: &gtk::Label, state: Option<State>) {
    if std::env::var_os("TRAY_DEBUG").is_some() {
        match state {
            Some(s) => eprintln!(
                "standalone-tray: volume: {:.0}% muted={}",
                s.volume * 100.0,
                s.muted
            ),
            None => eprintln!("standalone-tray: volume: unavailable"),
        }
    }
    let Some(state) = state else {
        container.set_visible(false);
        return;
    };
    container.set_visible(true);

    let percent = (state.volume * 100.0).round() as i32;
    let icon = if state.muted || percent == 0 {
        "audio-volume-muted-symbolic"
    } else if percent < 34 {
        "audio-volume-low-symbolic"
    } else if percent < 67 {
        "audio-volume-medium-symbolic"
    } else {
        "audio-volume-high-symbolic"
    };
    image.set_icon_name(Some(icon));
    label.set_text(&format!("{percent}%"));
    if state.muted {
        label.add_css_class("dim-label");
    } else {
        label.remove_css_class("dim-label");
    }
    container.set_tooltip_text(Some(&if state.muted {
        format!("Volume: {percent}% (muted)")
    } else {
        format!("Volume: {percent}%")
    }));
}

fn run_wpctl(args: &[&str]) {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    crate::runtime().spawn(async move {
        if let Err(e) = tokio::process::Command::new("wpctl")
            .args(&args)
            .status()
            .await
        {
            eprintln!("standalone-tray: wpctl failed: {e}");
        }
    });
}

/// Parse `wpctl get-volume` output, e.g. `Volume: 0.45 [MUTED]`.
async fn read_state() -> Option<State> {
    let output = tokio::process::Command::new("wpctl")
        .args(["get-volume", SINK])
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let volume = stdout.split_whitespace().nth(1)?.parse().ok()?;
    Some(State {
        volume,
        muted: stdout.contains("[MUTED]"),
    })
}

async fn watch(tx: async_channel::Sender<Option<State>>) {
    let mut last = read_state().await;
    if tx.send(last).await.is_err() {
        return;
    }

    let subscribe = tokio::process::Command::new("pactl")
        .arg("subscribe")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn();

    match subscribe {
        Ok(mut child) => {
            let Some(stdout) = child.stdout.take() else {
                return;
            };
            let mut lines = tokio::io::BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                // Volume/mute changes show up on the sink; default-sink
                // switches show up on the server.
                if !line.contains("on sink") && !line.contains("on server") {
                    continue;
                }
                let state = read_state().await;
                if state != last {
                    last = state;
                    if tx.send(state).await.is_err() {
                        break;
                    }
                }
            }
            let _ = child.kill().await;
        }
        Err(e) => {
            eprintln!("standalone-tray: pactl unavailable ({e}), polling volume instead");
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                let state = read_state().await;
                if state != last {
                    last = state;
                    if tx.send(state).await.is_err() {
                        break;
                    }
                }
            }
        }
    }
}
