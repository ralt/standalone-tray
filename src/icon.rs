use gtk::prelude::*;
use gtk::{gdk, glib};
use system_tray::item::{IconPixmap, Status, StatusNotifierItem};

use crate::ICON_SIZE;

/// Apply an item's current icon, tooltip and status to its widget.
pub fn apply(item: &StatusNotifierItem, image: &gtk::Image) {
    image.set_visible(item.status != Status::Passive);

    let attention = item.status == Status::NeedsAttention;

    let icon_name = if attention {
        non_empty(&item.attention_icon_name).or_else(|| non_empty(&item.icon_name))
    } else {
        non_empty(&item.icon_name)
    };
    let pixmaps = if attention {
        item.attention_icon_pixmap
            .as_ref()
            .or(item.icon_pixmap.as_ref())
    } else {
        item.icon_pixmap.as_ref()
    };

    let theme = gtk::IconTheme::for_display(&image.display());
    if let Some(path) = non_empty(&item.icon_theme_path) {
        if !theme.search_path().iter().any(|p| p.as_os_str() == path) {
            theme.add_search_path(path);
        }
    }

    if let Some(name) = icon_name.filter(|name| theme.has_icon(name)) {
        image.set_icon_name(Some(name));
    } else if let Some(pixmap) = pixmaps.and_then(|p| best_pixmap(p)) {
        image.set_paintable(Some(&texture(pixmap)));
    } else if let Some(name) = icon_name {
        // Not in the theme right now, but the theme may gain it later.
        image.set_icon_name(Some(name));
    } else {
        image.set_icon_name(Some("image-missing"));
    }

    image.set_tooltip_text(tooltip(item).as_deref());
}

fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|s| !s.is_empty())
}

fn tooltip(item: &StatusNotifierItem) -> Option<String> {
    if let Some(tip) = &item.tool_tip {
        let text = if tip.description.is_empty() {
            tip.title.clone()
        } else if tip.title.is_empty() {
            tip.description.clone()
        } else {
            format!("{}\n{}", tip.title, tip.description)
        };
        if !text.is_empty() {
            return Some(text);
        }
    }
    item.title.clone().filter(|t| !t.is_empty())
}

/// Pick the smallest pixmap at least as big as the target size,
/// or the biggest one if none is big enough.
fn best_pixmap(pixmaps: &[IconPixmap]) -> Option<&IconPixmap> {
    pixmaps
        .iter()
        .filter(|p| p.width > 0 && p.height > 0)
        .min_by_key(|p| {
            if p.width >= ICON_SIZE {
                (0, p.width)
            } else {
                (1, -p.width)
            }
        })
}

fn texture(pixmap: &IconPixmap) -> gdk::MemoryTexture {
    // SNI pixmaps are ARGB32 in network byte order; GDK wants RGBA.
    let mut pixels = pixmap.pixels.clone();
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.rotate_left(1);
    }
    gdk::MemoryTexture::new(
        pixmap.width,
        pixmap.height,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_owned(pixels),
        pixmap.width as usize * 4,
    )
}
