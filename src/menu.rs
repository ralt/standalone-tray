use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use system_tray::menu::{MenuItem, MenuType, ToggleState, ToggleType};

/// Build a GTK menu model and its action group from a DBusMenu layout.
/// Activating an entry invokes `on_activate` with the DBusMenu item id.
pub fn build(items: &[MenuItem], on_activate: &Rc<dyn Fn(i32)>) -> (gio::Menu, gio::SimpleActionGroup) {
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
