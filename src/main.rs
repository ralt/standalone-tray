mod battery;
mod icon;
mod menu;
mod volume;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};

use gtk::prelude::*;
use gtk::{gdk, glib};
use system_tray::client::{ActivateRequest, Client, Event, UpdateEvent};
use system_tray::data::BaseMap;

pub const ICON_SIZE: i32 = 22;

pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Runtime::new().expect("failed to start tokio runtime"))
}

enum Msg {
    Ready(Arc<Client>, Arc<Mutex<BaseMap>>),
    Event(Event),
}

fn main() -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id("com.github.ralt.StandaloneTray")
        .build();
    app.connect_activate(build_ui);
    app.run()
}

struct Tray {
    container: gtk::Box,
    placeholder: gtk::Label,
    widgets: HashMap<String, gtk::Image>,
    client: Option<Arc<Client>>,
    items: Option<Arc<Mutex<BaseMap>>>,
}

impl Tray {
    fn item(&self, address: &str) -> Option<system_tray::item::StatusNotifierItem> {
        let items = self.items.as_ref()?.lock().ok()?;
        items.get(address).map(|(item, _)| item.clone())
    }

    fn menu(&self, address: &str) -> Option<system_tray::menu::TrayMenu> {
        let items = self.items.as_ref()?.lock().ok()?;
        items.get(address).and_then(|(_, menu)| menu.clone())
    }
}

fn build_ui(app: &gtk::Application) {
    let container = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(4)
        .build();

    let placeholder = gtk::Label::new(Some("no tray items"));
    placeholder.add_css_class("dim-label");
    container.append(&placeholder);

    // Tray items live in their own box so items added later never end up
    // after the fixed widgets.
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(4)
        .margin_end(4)
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Center)
        .build();
    row.append(&container);
    row.append(&volume::widget());
    row.append(&battery::widget());

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Tray")
        .child(&row)
        .build();
    window.present();

    let tray = Rc::new(RefCell::new(Tray {
        container,
        placeholder,
        widgets: HashMap::new(),
        client: None,
        items: None,
    }));

    let (tx, rx) = async_channel::unbounded::<Msg>();

    runtime().spawn(async move {
        let client = match Client::new().await {
            Ok(client) => Arc::new(client),
            Err(e) => {
                eprintln!("standalone-tray: failed to connect to session bus: {e}");
                return;
            }
        };

        let mut events = client.subscribe();
        let items = client.items();

        if tx
            .send(Msg::Ready(client.clone(), items.clone()))
            .await
            .is_err()
        {
            return;
        }

        // Replay items that registered before we subscribed.
        let snapshot: Vec<_> = items
            .lock()
            .expect("items mutex should not be poisoned")
            .iter()
            .map(|(address, (item, _))| (address.clone(), item.clone()))
            .collect();
        for (address, item) in snapshot {
            if tx
                .send(Msg::Event(Event::Add(address, Box::new(item))))
                .await
                .is_err()
            {
                return;
            }
        }

        // The library misses removals of items registered by well-known name
        // (it compares the D-Bus unique name against the well-known name), so
        // also listen to the watcher's own unregistered signal.
        runtime().spawn(watch_unregistered(tx.clone()));

        loop {
            match events.recv().await {
                Ok(event) => {
                    if tx.send(Msg::Event(event)).await.is_err() {
                        return;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    });

    glib::spawn_future_local(async move {
        while let Ok(msg) = rx.recv().await {
            match msg {
                Msg::Ready(client, items) => {
                    let mut tray = tray.borrow_mut();
                    tray.client = Some(client);
                    tray.items = Some(items);
                }
                Msg::Event(event) => handle_event(&tray, event),
            }
        }
    });
}

fn handle_event(tray: &Rc<RefCell<Tray>>, event: Event) {
    if std::env::var_os("TRAY_DEBUG").is_some() {
        eprintln!("standalone-tray: event: {event:?}");
    }
    match event {
        Event::Add(address, _) => {
            if !tray.borrow().widgets.contains_key(&address) {
                add_item(tray, &address);
            }
            refresh_item(tray, &address);
        }
        Event::Update(address, update) => match update {
            // Menus are read from the state map when opened; nothing to redraw.
            UpdateEvent::Menu(_) | UpdateEvent::MenuDiff(_) | UpdateEvent::MenuConnect(_) => {}
            _ => refresh_item(tray, &address),
        },
        Event::Remove(address) => {
            let mut tray = tray.borrow_mut();
            if let Some(widget) = tray.widgets.remove(&address) {
                tray.container.remove(&widget);
            }
            // Drop library state too: removals we detect ourselves (see
            // watch_unregistered) don't go through the library.
            if let Some(items) = &tray.items {
                if let Ok(mut items) = items.lock() {
                    items.remove(&address);
                }
            }
            let empty = tray.widgets.is_empty();
            tray.placeholder.set_visible(empty);
        }
    }
}

fn add_item(tray: &Rc<RefCell<Tray>>, address: &str) {
    let image = gtk::Image::new();
    image.set_pixel_size(ICON_SIZE);

    let gesture = gtk::GestureClick::new();
    gesture.set_button(0);
    let tray_ref = Rc::downgrade(tray);
    let addr = address.to_string();
    let widget = image.clone();
    gesture.connect_pressed(move |gesture, _, x, y| {
        let Some(tray) = tray_ref.upgrade() else {
            return;
        };
        match gesture.current_button() {
            gdk::BUTTON_PRIMARY => primary_activate(&tray, &addr, &widget, x, y),
            gdk::BUTTON_MIDDLE => send_activate(
                &tray,
                ActivateRequest::Secondary {
                    address: addr.clone(),
                    x: x as i32,
                    y: y as i32,
                },
            ),
            gdk::BUTTON_SECONDARY => show_menu(&tray, &addr, &widget, x, y),
            _ => {}
        }
    });
    image.add_controller(gesture);

    let mut tray = tray.borrow_mut();
    tray.placeholder.set_visible(false);
    tray.container.append(&image);
    tray.widgets.insert(address.to_string(), image);
}

fn refresh_item(tray: &Rc<RefCell<Tray>>, address: &str) {
    let tray = tray.borrow();
    let (Some(widget), Some(item)) = (tray.widgets.get(address), tray.item(address)) else {
        return;
    };
    icon::apply(&item, widget);
}

fn primary_activate(tray: &Rc<RefCell<Tray>>, address: &str, widget: &gtk::Image, x: f64, y: f64) {
    let item_is_menu = tray
        .borrow()
        .item(address)
        .map(|item| item.item_is_menu)
        .unwrap_or(false);
    if item_is_menu {
        show_menu(tray, address, widget, x, y);
    } else {
        send_activate(
            tray,
            ActivateRequest::Default {
                address: address.to_string(),
                x: x as i32,
                y: y as i32,
            },
        );
    }
}

fn send_activate(tray: &Rc<RefCell<Tray>>, request: ActivateRequest) {
    let Some(client) = tray.borrow().client.clone() else {
        return;
    };
    runtime().spawn(async move {
        if let Err(e) = client.activate(request).await {
            eprintln!("standalone-tray: activate request failed: {e}");
        }
    });
}

fn show_menu(tray: &Rc<RefCell<Tray>>, address: &str, widget: &gtk::Image, x: f64, y: f64) {
    let (client, menu_path) = {
        let tray = tray.borrow();
        let Some(client) = tray.client.clone() else {
            return;
        };
        (client, tray.item(address).and_then(|item| item.menu))
    };

    let Some(menu_path) = menu_path else {
        // No DBusMenu: fall back to asking the item to show its own menu.
        context_menu_fallback(address.to_string(), x as i32, y as i32);
        return;
    };

    // Let lazy items (e.g. nm-applet) populate their menu before we render it.
    let about_to_show = runtime().spawn({
        let client = client.clone();
        let address = address.to_string();
        let menu_path = menu_path.clone();
        async move { client.about_to_show_menuitem(address, menu_path, 0).await }
    });

    let tray_ref = Rc::downgrade(tray);
    let address = address.to_string();
    let widget = widget.clone();
    glib::spawn_future_local(async move {
        let _ = about_to_show.await;
        let Some(tray) = tray_ref.upgrade() else {
            return;
        };
        let Some(tray_menu) = tray.borrow().menu(&address) else {
            return;
        };

        let on_activate: Rc<dyn Fn(i32)> = Rc::new({
            let tray = tray.clone();
            let address = address.clone();
            let menu_path = menu_path.clone();
            move |submenu_id| {
                send_activate(
                    &tray,
                    ActivateRequest::MenuItem {
                        address: address.clone(),
                        menu_path: menu_path.clone(),
                        submenu_id,
                    },
                );
            }
        });

        let (model, actions) = menu::build(&tray_menu.submenus, &on_activate);
        let popover = gtk::PopoverMenu::from_model(Some(&model));
        popover.insert_action_group("tray", Some(&actions));
        popover.set_parent(&widget);
        popover.set_has_arrow(false);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.connect_closed(|popover| {
            let popover = popover.clone();
            glib::idle_add_local_once(move || popover.unparent());
        });
        popover.popup();
    });
}

async fn watch_unregistered(tx: async_channel::Sender<Msg>) {
    let result = async {
        let connection = zbus::Connection::session().await?;
        let proxy = zbus::Proxy::new(
            &connection,
            "org.kde.StatusNotifierWatcher",
            "/StatusNotifierWatcher",
            "org.kde.StatusNotifierWatcher",
        )
        .await?;
        let mut stream = proxy
            .receive_signal("StatusNotifierItemUnregistered")
            .await?;
        use futures_util::StreamExt;
        while let Some(signal) = stream.next().await {
            if let Ok(service) = signal.body().deserialize::<String>() {
                let destination = service
                    .split_once('/')
                    .map_or(service.as_str(), |(dest, _)| dest)
                    .to_string();
                if tx
                    .send(Msg::Event(Event::Remove(destination)))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
        Ok::<_, zbus::Error>(())
    }
    .await;
    if let Err(e) = result {
        eprintln!("standalone-tray: failed to watch item unregistrations: {e}");
    }
}

/// Ask the item to show its own context menu (`ContextMenu(x, y)`); only used
/// when the item exposes no DBusMenu. The library client doesn't wrap this
/// call, so it is made directly.
fn context_menu_fallback(address: String, x: i32, y: i32) {
    runtime().spawn(async move {
        let (destination, path) = match address.split_once('/') {
            Some((dest, path)) => (dest.to_string(), format!("/{path}")),
            None => (address.clone(), String::from("/StatusNotifierItem")),
        };
        let result = async {
            let connection = zbus::Connection::session().await?;
            let proxy =
                zbus::Proxy::new(&connection, destination, path, "org.kde.StatusNotifierItem")
                    .await?;
            proxy.call_method("ContextMenu", &(x, y)).await?;
            Ok::<_, zbus::Error>(())
        }
        .await;
        if let Err(e) = result {
            eprintln!("standalone-tray: ContextMenu call failed: {e}");
        }
    });
}
