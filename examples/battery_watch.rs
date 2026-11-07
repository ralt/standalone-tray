use futures_util::StreamExt;

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    if let Err(e) = rt.block_on(run()) {
        eprintln!("error: {e}");
    }
}

async fn run() -> zbus::Result<()> {
    let connection = zbus::Connection::system().await?;
    let device = zbus::Proxy::new(
        &connection,
        "org.freedesktop.UPower",
        "/org/freedesktop/UPower/devices/DisplayDevice",
        "org.freedesktop.UPower.Device",
    )
    .await?;
    let properties = zbus::fdo::PropertiesProxy::builder(&connection)
        .destination("org.freedesktop.UPower")?
        .path("/org/freedesktop/UPower/devices/DisplayDevice")?
        .build()
        .await?;
    let mut changes = properties.receive_properties_changed().await?;

    dump("initial", &device, None).await?;
    while let Some(signal) = changes.next().await {
        let args = signal.args()?;
        let changed: Vec<String> = args
            .changed_properties()
            .iter()
            .map(|(k, v)| format!("{k}={v:?}"))
            .collect();
        dump("signal", &device, Some(changed)).await?;
    }
    Ok(())
}

async fn dump(
    tag: &str,
    device: &zbus::Proxy<'_>,
    changed: Option<Vec<String>>,
) -> zbus::Result<()> {
    let icon: String = device.get_property("IconName").await?;
    let state: u32 = device.get_property("State").await?;
    let pct: f64 = device.get_property("Percentage").await?;
    println!("[{tag}] cache: icon={icon} state={state} pct={pct:.0}");
    if let Some(changed) = changed {
        println!("        signal body: {changed:?}");
    }
    Ok(())
}
