use gio::prelude::*;
use gtk::{gio, glib};

// One id=command per line. Commands are parsed as argv, never by a shell.
fn command_for(config: &str, id: &str) -> Option<String> {
    config
        .lines()
        .filter_map(|line| line.split_once('='))
        .find(|(key, value)| key.trim() == id && !value.trim().is_empty())
        .map(|(_, value)| value.trim().to_owned())
}

// Debian's update indicator is menu-only. Reuse its configured updater rather
// than introducing a second command setting just to support primary activation.
fn configured_command(config: &str, id: &str) -> Option<String> {
    if let Some(command) = command_for(config, id) {
        return Some(command);
    }
    if id != "org.guido-berhoerster.code.package-update-indicator" {
        return None;
    }
    let source = gio::SettingsSchemaSource::default()?;
    let schema = source.lookup(id, true)?;
    let settings = gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None);
    let command = settings.string("update-command");
    (!command.trim().is_empty()).then(|| command.to_string())
}

pub async fn activate(address: &str, x: i32, y: i32) -> Result<(), String> {
    let config = std::env::var("STANDALONE_TRAY_ACTIVATE_COMMANDS").unwrap_or_default();
    activate_with_config(address, x, y, &config).await
}

async fn activate_with_config(address: &str, x: i32, y: i32, config: &str) -> Result<(), String> {
    let (destination, path) = address
        .split_once('/')
        .map(|(destination, path)| (destination, format!("/{path}")))
        .unwrap_or((address, "/StatusNotifierItem".to_string()));
    let connection = zbus::Connection::session()
        .await
        .map_err(|e| e.to_string())?;
    let proxy = zbus::Proxy::new(&connection, destination, path, "org.kde.StatusNotifierItem")
        .await
        .map_err(|e| e.to_string())?;
    match proxy.call_method("Activate", &(x, y)).await {
        Ok(_) => Ok(()),
        Err(zbus::Error::MethodError(ref name, _, _))
            if name.as_str() == "org.freedesktop.DBus.Error.UnknownMethod" =>
        {
            // AppIndicator does not expose Activate. Use the configured updater
            // or an explicit fallback, preserving normal SNI behavior.
            let id: String = proxy.get_property("Id").await.map_err(|e| e.to_string())?;
            let command = configured_command(config, &id)
                .ok_or_else(|| format!("{id} has no Activate method and no configured fallback"))?;
            glib::spawn_command_line_async(&command).map_err(|e| e.to_string())
        }
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_is_specific_to_the_configured_icon() {
        let config = "updates=/usr/bin/gpk-update-viewer\nother=some-program --option";
        assert_eq!(
            command_for(config, "updates").as_deref(),
            Some("/usr/bin/gpk-update-viewer")
        );
        assert_eq!(command_for(config, "nm-applet"), None);
        assert_eq!(command_for("updates= ", "updates"), None);
    }

    struct MenuOnly;
    #[zbus::interface(name = "org.kde.StatusNotifierItem")]
    impl MenuOnly {
        #[zbus(property, name = "Id")]
        fn id(&self) -> &str {
            "test-menu-only"
        }
    }

    struct Normal;
    #[zbus::interface(name = "org.kde.StatusNotifierItem")]
    impl Normal {
        fn activate(&self, _x: i32, _y: i32) {}
    }

    struct Failing;
    #[zbus::interface(name = "org.kde.StatusNotifierItem")]
    impl Failing {
        fn activate(&self, _x: i32, _y: i32) -> zbus::fdo::Result<()> {
            Err(zbus::fdo::Error::Failed("activation rejected".to_string()))
        }
    }

    #[test]
    #[ignore = "run inside dbus-run-session"]
    fn activation_uses_fallback_only_when_method_is_missing() {
        crate::runtime().block_on(async {
            let service = zbus::connection::Builder::session()
                .unwrap()
                .name("org.example.StandaloneTrayActivationTest")
                .unwrap()
                .serve_at("/MenuOnly", MenuOnly)
                .unwrap()
                .serve_at("/Normal", Normal)
                .unwrap()
                .serve_at("/Failing", Failing)
                .unwrap()
                .build()
                .await
                .unwrap();
            let address = |path| format!("{}{path}", service.unique_name().unwrap());
            assert!(activate_with_config(&address("/MenuOnly"), 0, 0, "")
                .await
                .is_err());
            assert!(activate_with_config(
                &address("/MenuOnly"),
                0,
                0,
                "test-menu-only=/usr/bin/true"
            )
            .await
            .is_ok());
            // Successful normal activation needs no fallback or Id property.
            assert!(activate_with_config(
                &address("/Normal"),
                0,
                0,
                "test-menu-only=/no/such/program"
            )
            .await
            .is_ok());
            let error =
                activate_with_config(&address("/Failing"), 0, 0, "test-menu-only=/usr/bin/true")
                    .await
                    .unwrap_err();
            assert!(error.contains("activation rejected"));
        });
    }
}
