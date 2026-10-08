//! Quick view's info card, for folders and files with no preview, as in
//! Quick Look: the shell icon or thumbnail, the name, the kind and size (or
//! a folder's item count), and the modified date.
use super::{
    actions::{file_size, local_time},
    app::{display_path, file_name},
    paint::Look,
    render::{measure, Align},
    widgets::Rect,
};
use crate::model::Frame;
use std::path::Path;
use windows::{
    core::HSTRING,
    Win32::{
        Graphics::Direct2D::ID2D1Bitmap,
        Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES,
        UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_TYPENAME},
    },
};

/// The icon's side and the card's content size, in epx.
pub(super) const ICON: f32 = 128.0;
pub(super) const SIZE: (f32, f32) = (520.0, 200.0);

#[derive(Clone)]
pub(super) struct Card {
    name: String,
    detail: String,
    modified: String,
    pub(super) icon: Option<Frame>,
}

impl Card {
    /// Everything but the icon. Call off the window thread: a folder's item
    /// count reads the whole folder.
    pub(super) fn read(path: &Path) -> Self {
        let meta = std::fs::metadata(path).ok();
        let amount = match &meta {
            Some(m) if m.is_dir() => std::fs::read_dir(path).ok().map(|d| match d.count() {
                1 => "1 item".to_string(),
                n => format!("{n} items"),
            }),
            Some(m) => Some(file_size(m.len())),
            None => None,
        };
        let detail = [kind(path), amount.unwrap_or_default()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
        let modified = meta.map(|m| local_time(m.modified())).filter(|t| !t.is_empty()).map(|t| format!("Modified {t}"));
        Self { name: file_name(path), detail, modified: modified.unwrap_or_default(), icon: None }
    }

    /// The icon at the left of `r`, the text beside it, centered vertically.
    pub(super) fn paint(&self, l: &Look, icon: Option<&ID2D1Bitmap>, r: Rect) {
        let s = l.s;
        let side = (r.height() - 32.0 * s).clamp(0.0, ICON * s);
        let frame = Rect::new(r.x0 + 24.0 * s, r.y0 + (r.height() - side) / 2.0, side, side);
        if let (Some(bitmap), Some(image)) = (icon, &self.icon) {
            let k = (side / image.width.max(1) as f32).min(side / image.height.max(1) as f32);
            let (w, h) = (image.width as f32 * k, image.height as f32 * k);
            l.p.draw_bitmap(bitmap, Rect::new(frame.x0 + (side - w) / 2.0, frame.y0 + (side - h) / 2.0, w, h));
        }
        let title = measure("Ag", &l.f.title, 10_000.0).1;
        let body = measure("Ag", &l.f.body, 10_000.0).1;
        let gap = 4.0 * s;
        let x0 = frame.x1 + 20.0 * s;
        let x1 = (r.x1 - 24.0 * s).max(x0);
        let mut y = r.y0 + (r.height() - (title + 2.0 * (body + gap))) / 2.0;
        l.p.text(&self.name, Rect { x0, y0: y, x1, y1: y + title }, &l.f.title, l.t.text, Align::Leading);
        y += title + gap;
        for line in [&self.detail, &self.modified] {
            l.p.text(line, Rect { x0, y0: y, x1, y1: y + body }, &l.f.body, l.t.text_secondary, Align::Leading);
            y += body + gap;
        }
    }
}

/// The type name Explorer shows, such as "Microsoft Word Document".
fn kind(path: &Path) -> String {
    let mut info = SHFILEINFOW::default();
    let size = std::mem::size_of::<SHFILEINFOW>() as u32;
    let path = HSTRING::from(display_path(path));
    if unsafe { SHGetFileInfoW(&path, FILE_FLAGS_AND_ATTRIBUTES(0), Some(&mut info), size, SHGFI_TYPENAME) } == 0 {
        return String::new();
    }
    let length = info.szTypeName.iter().position(|&c| c == 0).unwrap_or(info.szTypeName.len());
    String::from_utf16_lossy(&info.szTypeName[..length])
}
