use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use system_tray::menu::{MenuItem, MenuType, ToggleState, ToggleType};

/// Build a GTK menu model and its action group from a DBusMenu layout.
/// Activating an entry invokes `on_activate` with the DBusMenu item id.
pub fn build(
    items: &[MenuItem],
    on_activate: &Rc<dyn Fn(i32)>,
) -> (gio::Menu, gio::SimpleActionGroup) {
    let actions = gio::SimpleActionGroup::new();
    let model = build_level(items, &actions, on_activate);
    (model, actions)
}

fn build_level(
    items: &[MenuItem],
    actions: &gio::SimpleActionGroup,
    on_activate: &Rc<dyn Fn(i32)>,
) -> gio::Menu {
    let root = gio::Menu::new();
    let mut section = gio::Menu::new();

    for item in items.iter().filter(|item| item.visible) {
        if item.menu_type == MenuType::Separator {
            if section.n_items() > 0 {
                root.append_section(None, &section);
                section = gio::Menu::new();
            }
            continue;
        }

        let label = clean_label(item.label.as_deref().unwrap_or_default());

        if !item.submenu.is_empty() {
            let submenu = build_level(&item.submenu, actions, on_activate);
            section.append_submenu(Some(&label), &submenu);
            continue;
        }

        let name = format!("item-{}", item.id);
        let action = match item.toggle_type {
            ToggleType::Checkmark | ToggleType::Radio => gio::SimpleAction::new_stateful(
                &name,
                None,
                &(item.toggle_state == ToggleState::On).to_variant(),
            ),
            ToggleType::CannotBeToggled => gio::SimpleAction::new(&name, None),
        };
        action.set_enabled(item.enabled);
        let id = item.id;
        let on_activate = on_activate.clone();
        action.connect_activate(move |_, _| on_activate(id));
        actions.add_action(&action);

        section.append_item(&gio::MenuItem::new(
            Some(&label),
            Some(&format!("tray.{name}")),
        ));
    }

    if section.n_items() > 0 {
        root.append_section(None, &section);
    }
    root
}

/// DBusMenu labels use `_` for mnemonics and `__` for a literal underscore;
/// menu models don't interpret them, so strip the former and unescape the latter.
fn clean_label(label: &str) -> glib::GString {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            if chars.peek() == Some(&'_') {
                chars.next();
                out.push('_');
            }
        } else {
            out.push(c);
        }
    }
    out.into()
}

/// Read the current menu, including protocol defaults for omitted properties.
pub async fn fetch_layout(
    address: &str,
    path: &str,
) -> Result<system_tray::menu::TrayMenu, String> {
    let destination = address.split('/').next().unwrap_or(address);
    let connection = zbus::Connection::session()
        .await
        .map_err(|e| e.to_string())?;
    let proxy = zbus::Proxy::new(&connection, destination, path, "com.canonical.dbusmenu")
        .await
        .map_err(|e| e.to_string())?;
    let reply = proxy
        .call_method("GetLayout", &(0_i32, -1_i32, Vec::<&str>::new()))
        .await
        .map_err(|e| e.to_string())?;
    type Layout = (
        u32,
        (
            i32,
            std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
            Vec<zbus::zvariant::OwnedValue>,
        ),
    );
    let (_, (id, _, children)): Layout = reply.body().deserialize().map_err(|e| e.to_string())?;
    let submenus = children
        .iter()
        .map(MenuItem::try_from)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(system_tray::menu::TrayMenu {
        id: id as u32,
        submenus,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::{OwnedValue, Structure};

    #[test]
    fn fresh_layout_restores_default_enabled_state() {
        let mut props = std::collections::HashMap::new();
        props.insert(
            "label".to_string(),
            OwnedValue::from(zbus::zvariant::Str::from("Install updates")),
        );
        props.insert("enabled".to_string(), OwnedValue::from(false));
        let value = Structure::from((2_i32, props.clone(), Vec::<OwnedValue>::new()));
        assert!(
            !MenuItem::try_from(&OwnedValue::try_from(value).unwrap())
                .unwrap()
                .enabled
        );
        props.remove("enabled");
        let value = Structure::from((2_i32, props, Vec::<OwnedValue>::new()));
        assert!(
            MenuItem::try_from(&OwnedValue::try_from(value).unwrap())
                .unwrap()
                .enabled
        );
    }
}
