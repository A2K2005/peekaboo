//! Draws the window: title bar, toolbar, markup bar, sidebar, status,
//! empty state, sheets, focus rectangles, keytips, and tooltips. The
//! document itself is drawn by document.rs.
use super::{
    app::{State, file_name, invalidate, with_state},
    commands::glyph,
    document,
    render::{Align, Fonts, Painter, Renderer, fonts, measure},
    theme::{Mode, Rgba, Theme},
    widgets::{self, Layout, Rect, Region, Role, SidebarList, Widget, WidgetId, in_scope},
    worker::{Key, Work},
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        Graphics::Direct2D::ID2D1Bitmap,
        Graphics::Dwm::DwmFlush,
        System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
        UI::{Input::KeyboardAndMouse::GetFocus, WindowsAndMessaging::*},
    },
    core::Result,
};

pub(super) struct Look<'a> {
    pub(super) p: &'a Painter,
    pub(super) t: Theme,
    pub(super) f: &'a Fonts,
    pub(super) s: f32,
}

#[derive(Clone, Copy, PartialEq)]
enum Fill {
    Hover,
    Pressed,
}

/// Hover or pressed fill for an enabled widget.
fn background(state: &State, w: &Widget) -> Option<Fill> {
    if !w.enabled {
        return None;
    }
    if state.pressed == Some(w.id) || state.caption_pressed == Some(w.id) {
        Some(Fill::Pressed)
    } else if state.hover == Some(w.id) || state.caption_hover == Some(w.id) {
        Some(Fill::Hover)
    } else {
        None
    }
}

pub(super) fn icon_button(l: &Look, state: &State, w: &Widget) {
    let fill = background(state, w);
    let checked = w.checked == Some(true);
    let mut color = if w.enabled { l.t.text } else { l.t.text_disabled };
    let r = w.rect;
    if checked && l.t.mode == Mode::Contrast {
        l.p.fill_round(r, 6.0 * l.s, l.t.accent);
        color = l.t.on_accent;
    } else if let Some(fill) = fill {
        l.p.fill_round(r, 6.0 * l.s, if fill == Fill::Hover { l.t.hover } else { l.t.pressed });
        color = l.t.hover_text;
    } else if checked {
        l.p.fill_round(r, 6.0 * l.s, l.t.hover);
        color = l.t.hover_text;
    }
    if checked && l.t.mode != Mode::Contrast {
        let bar = Rect { x0: r.x0 + 12.0 * l.s, y0: r.y1 - 3.0 * l.s, x1: r.x1 - 12.0 * l.s, y1: r.y1 };
        l.p.fill_round(bar, 1.5 * l.s, l.t.accent);
    }
    if let Some(g) = w.glyph {
        l.p.glyph(g, r, &l.f.icon, color);
    }
}

pub(super) fn text_button(l: &Look, state: &State, w: &Widget) {
    let r = w.rect;
    let fill = background(state, w);
    let (mut back, mut color) = if w.primary { (l.t.accent, l.t.on_accent) } else { (l.t.field, l.t.text) };
    if let Some(fill) = fill {
        if !w.primary || l.t.mode == super::theme::Mode::Contrast {
            back = if fill == Fill::Hover { l.t.hover } else { l.t.pressed };
            color = l.t.hover_text;
        }
    }
    if !w.enabled {
        color = l.t.text_disabled;
    }
    if !w.primary {
        l.p.fill_round(r, 6.0 * l.s, l.t.field);
    }
    l.p.fill_round(r, 6.0 * l.s, back);
    if !w.primary || l.t.mode == super::theme::Mode::Contrast {
        l.p.stroke_round(r, 6.0 * l.s, l.t.control_border, l.t.border_width * l.s.floor().max(1.0));
    }
    l.p.text(&w.label, r.inset(4.0 * l.s), &l.f.body, color, Align::Center);
}

/// The editor bar: the title and page count, the tools, and the caption
/// buttons; then the tab strip when 2 or more files are open.
fn title_bar(l: &Look, state: &State, layout: &Layout) {
    let s = l.s;
    let bar = layout.title_bar;
    if bar.height() > 0.0 {
        l.p.fill(bar, l.t.bar);
        l.p.fill(Rect { y0: bar.y1 - s, ..bar }, l.t.divider);
        let t = layout.title_text;
        let name = state.path.as_deref().map(file_name).unwrap_or_else(|| "Preview for Windows".into());
        let detail = state.title_detail();
        if detail.is_empty() {
            l.p.text(&name, t, &l.f.strong, l.t.text, Align::Leading);
        } else {
            let (line, small) = (20.0 * state.text_scale * s, 16.0 * state.text_scale * s);
            let y = (t.y0 + t.y1 - line - small) / 2.0;
            l.p.text(&name, Rect { y0: y, y1: y + line, ..t }, &l.f.strong, l.t.text, Align::Leading);
            l.p.text(&detail, Rect { y0: y + line, y1: y + line + small, ..t }, &l.f.caption, l.t.text_secondary, Align::Leading);
        }
        for w in layout.widgets.iter().filter(|w| w.region == Region::Toolbar) {
            icon_button(l, state, w);
        }
    }
    if let Some(strip) = layout.tab_strip {
        l.p.fill(Rect { y0: strip.y1 - s, ..strip }, l.t.divider);
    }
    for w in layout.widgets.iter().filter(|w| w.region == Region::TitleBar) {
        match w.id {
            WidgetId::Tab(_) => {
                let selected = w.checked == Some(true);
                let r = w.rect;
                if selected {
                    l.p.fill_round(r, 6.0 * s, l.t.selected);
                    l.p.stroke_round(r, 6.0 * s, l.t.divider, s);
                } else if let Some(fill) = background(state, w) {
                    l.p.fill_round(r, 6.0 * s, if fill == Fill::Hover { l.t.hover } else { l.t.pressed });
                }
                let color = if selected {
                    l.t.selected_text
                } else if state.hover == Some(w.id) {
                    l.t.hover_text
                } else {
                    l.t.text_secondary
                };
                let text = Rect { x0: r.x0 + 12.0 * s, y0: r.y0, x1: r.x1 - 36.0 * s, y1: r.y1 };
                l.p.text(&w.label, text, &l.f.body, color, Align::Leading);
            }
            WidgetId::TabClose(index) => {
                let tab_selected = layout.widgets.iter().any(|t| t.id == WidgetId::Tab(index) && t.checked == Some(true));
                let mut color = if tab_selected { l.t.selected_text } else { l.t.text_secondary };
                if let Some(fill) = background(state, w) {
                    l.p.fill_round(w.rect, 4.0 * s, if fill == Fill::Hover { l.t.hover } else { l.t.pressed });
                    color = l.t.hover_text;
                }
                l.p.glyph(glyph::CANCEL, w.rect, &l.f.caption_icon, color);
            }
            WidgetId::NewTab => icon_button(l, state, w),
            WidgetId::Minimize | WidgetId::Maximize | WidgetId::Close => {
                let g = match w.id {
                    WidgetId::Minimize => glyph::MINIMIZE,
                    WidgetId::Maximize if state.maximized => glyph::RESTORE,
                    WidgetId::Maximize => glyph::MAXIMIZE,
                    _ => glyph::CLOSE,
                };
                let mut color = l.t.text;
                if let Some(fill) = background(state, w) {
                    let back = match (w.id, fill) {
                        (WidgetId::Close, _) => l.t.close_hover,
                        (_, Fill::Hover) => l.t.hover,
                        _ => l.t.pressed,
                    };
                    l.p.fill(w.rect, back);
                    color =
                        if w.id == WidgetId::Close && l.t.mode != super::theme::Mode::Contrast { super::theme::Rgba::hex(0xffffff) } else { l.t.hover_text };
                }
                l.p.glyph(g, w.rect, &l.f.caption_icon, color);
            }
            _ => {}
        }
    }
}

fn bars(l: &Look, state: &State, layout: &Layout) {
    let s = l.s;
    if let Some(bar) = layout.markup_bar {
        l.p.fill(bar, l.t.bar);
        l.p.fill(Rect { y0: bar.y1 - s, ..bar }, l.t.divider);
        for w in layout.widgets.iter().filter(|w| w.region == Region::MarkupBar) {
            icon_button(l, state, w);
        }
    }
    if let Some(side) = layout.sidebar {
        l.p.fill(side, l.t.content_layer);
        l.p.fill(Rect { y1: layout.sidebar_panel.y0, ..side }, l.t.bar);
        if !layout.contact_sheet {
            let edge = if state.sidebar_resize.is_some() { l.t.accent } else { l.t.divider };
            l.p.fill(Rect { x0: side.x1 - s, ..side }, edge);
        }
        for w in layout.widgets.iter().filter(|w| w.role == Role::SidebarTab) {
            icon_button(l, state, w);
        }
    }
}

/// The sidebar list: thumbnails with page numbers, outline entries indented
/// by level, or notes. Only rows in view are drawn.
fn sidebar_panel(l: &Look, state: &mut State, layout: &Layout) {
    if layout.sidebar.is_none() {
        return;
    }
    let (s, ts, panel) = (l.s, state.text_scale, layout.sidebar_panel);
    let list = state.sidebar_list();
    if let SidebarList::Message(message) = list {
        let line = Rect { y1: panel.y0 + 3.0 * 20.0 * ts * s, ..panel };
        l.p.text(message, line, &l.f.wrap, l.t.text_secondary, Align::Leading);
        return;
    }
    let scroll = state.sidebar_scroll[state.sidebar_tab.min(3)];
    let (rows, _) = widgets::sidebar_rows(&list, panel, scroll, s, ts);
    let rows: Vec<(usize, Rect, String, u32)> = rows
        .into_iter()
        .map(|(i, r)| {
            let level = if let SidebarList::Contents(items) = list { items[i].level.min(8) } else { 0 };
            (i, r, list.label(i), level)
        })
        .collect();
    let thumbnails = matches!(list, SidebarList::Thumbnails(_) | SidebarList::Sheet(_));
    l.p.push_clip(panel);
    for (index, row, label, level) in rows {
        if state.hover == Some(WidgetId::SidebarItem(index)) {
            l.p.fill_round(row.inset(2.0 * s), 4.0 * s, l.t.hover);
        }
        if !thumbnails {
            let enabled = layout.widgets.iter().find(|w| w.id == WidgetId::SidebarItem(index)).is_none_or(|w| w.enabled);
            let text = Rect { x0: row.x0 + (8.0 + 12.0 * level as f32) * s, x1: row.x1 - 8.0 * s, ..row };
            l.p.text(&label, text, &l.f.body, if enabled { l.t.text } else { l.t.text_disabled }, Align::Leading);
            continue;
        }
        let Some(v) = state.pdf.as_ref() else {
            break;
        };
        let (w, h) = widgets::thumb_size(v.sizes[index], s);
        let thumb = Rect::new((row.x0 + (row.width() - w) / 2.0).round(), (row.y0 + 6.0 * s).round(), w, h);
        let current = index == state.page as usize;
        l.p.fill(thumb, super::theme::Rgba(1.0, 1.0, 1.0, 1.0));
        if let Some(tile) = state.cache.get(&Key { doc: v.doc, work: Work::Thumb { page: index as u32 } }) {
            l.p.draw_bitmap(&tile.bitmap, thumb);
        }
        // The current page gets the accent frame, as Preview does.
        let (frame, width) = if current { (l.t.accent, 2.0 * s) } else { (l.t.border, s) };
        l.p.stroke_round(thumb.inset(-width), 0.0, frame, width);
        let number = Rect { y0: thumb.y1 + 4.0 * s, y1: row.y1, ..row };
        let font = if current { &l.f.strong } else { &l.f.caption };
        l.p.text(&(index + 1).to_string(), number, font, if current { l.t.text } else { l.t.text_secondary }, Align::Center);
    }
    l.p.pop_clip();
}

fn empty_state(l: &Look, state: &State, layout: &Layout) {
    let Some(empty) = &layout.empty else {
        return;
    };
    l.p.text("Open a PDF or image", empty.heading, &l.f.title, l.t.text, Align::Center);
    for w in layout.widgets.iter().filter(|w| w.region == Region::Document && w.role == Role::Button) {
        text_button(l, state, w);
    }
    l.p.text("Or drop a PDF or image here", empty.drop_hint, &l.f.body, l.t.text_secondary, Align::Center);
    l.p.text("Recent files", empty.recent_heading, &l.f.strong, l.t.text, Align::Center);
    super::empty::paint_recent(l, state, layout, empty.recent);
}

fn sheet(l: &Look, state: &State, layout: &Layout) {
    let (Some(sheet), Some(card)) = (&state.sheet, &layout.sheet) else {
        return;
    };
    let s = l.s;
    l.p.fill(Rect { y0: layout.title_bar.y1, ..Rect::new(0.0, 0.0, state.size.0, state.size.1) }, l.t.scrim);
    if l.t.mode != Mode::Contrast {
        for (spread, alpha) in [(12.0, 0.04), (6.0, 0.08), (2.0, 0.16)] {
            l.p.fill_round(card.card.inset(-spread * s), (10.0 + spread) * s, Rgba(0.0, 0.0, 0.0, alpha));
        }
    }
    l.p.fill_round(card.card, 10.0 * s, l.t.surface);
    l.p.stroke_round(card.card, 10.0 * s, l.t.control_border, l.t.border_width * s.floor().max(1.0));
    l.p.text(&sheet.title, card.title, &l.f.title, l.t.text, Align::Leading);
    l.p.text(&sheet.message, card.message, &l.f.wrap, l.t.text, Align::Leading);
    let focused = unsafe { GetFocus() };
    for ((field, label), rect) in sheet.fields.iter().zip(&card.labels).zip(&card.fields) {
        l.p.text(&field.label, *label, &l.f.body, l.t.text, Align::Leading);
        l.p.fill_round(*rect, 6.0 * s, l.t.field);
        l.p.stroke_round(*rect, 6.0 * s, l.t.control_border, l.t.border_width * s.floor().max(1.0));
        // Fluent text boxes mark focus with an accent line at the bottom.
        let line = if field.edit == focused { 2.0 * s } else { 0.0 };
        if line > 0.0 {
            l.p.fill_round(Rect { y0: rect.y1 - line, ..*rect }, s, l.t.accent);
        }
    }
    for w in layout.widgets.iter().filter(|w| w.region == Region::Sheet) {
        match w.id {
            WidgetId::SheetControl(i) => {
                if let Some(c) = sheet.controls.get(i) {
                    sheet_control(l, state, w, c);
                }
            }
            WidgetId::SheetButton(_) => text_button(l, state, w),
            _ => {}
        }
    }
}

fn sheet_control(l: &Look, state: &State, w: &Widget, c: &super::sheet::Control) {
    use super::sheet::Kind;
    let (s, r) = (l.s, w.rect);
    let color = if w.enabled { l.t.text } else { l.t.text_disabled };
    let border = l.t.border_width * s.floor().max(1.0);
    let hover = background(state, w).map(|fill| if fill == Fill::Hover { l.t.hover } else { l.t.pressed });
    match c.kind {
        Kind::Choice => {
            l.p.fill_round(r, 6.0 * s, hover.unwrap_or(l.t.field));
            l.p.stroke_round(r, 6.0 * s, l.t.control_border, border);
            let text = Rect { x0: r.x0 + 12.0 * s, x1: r.x1 - 36.0 * s, ..r };
            l.p.text(&c.text(), text, &l.f.body, color, Align::Leading);
            l.p.glyph(glyph::CHEVRON_DOWN, Rect { x0: r.x1 - 36.0 * s, ..r }, &l.f.caption_icon, color);
        }
        Kind::Toggle => {
            if let Some(fill) = hover {
                l.p.fill_round(r, 6.0 * s, fill);
            }
            let side = 20.0 * s;
            let check = Rect::new(r.x0 + 6.0 * s, r.y0 + (r.height() - side) / 2.0, side, side);
            if c.value == 1 {
                l.p.fill_round(check, 4.0 * s, if w.enabled { l.t.accent } else { l.t.text_disabled });
                l.p.glyph(glyph::CHECK, check, &l.f.caption_icon, l.t.on_accent);
            } else {
                l.p.fill_round(check, 4.0 * s, l.t.field);
                l.p.stroke_round(check, 4.0 * s, l.t.control_border, border);
            }
            l.p.text(&c.text(), Rect { x0: check.x1 + 10.0 * s, ..r }, &l.f.body, color, Align::Leading);
        }
        Kind::Slider => {
            let track = super::sheet::track(r, s);
            l.p.text(&c.text(), Rect { x1: track.x0 - 8.0 * s, ..r }, &l.f.body, color, Align::Leading);
            let mid = (r.y0 + r.y1) / 2.0;
            let rail = Rect { y0: mid - 2.0 * s, y1: mid + 2.0 * s, ..track };
            let x = track.x0 + track.width() * (c.value.clamp(1, 100) - 1) as f32 / 99.0;
            let filled = if w.enabled { l.t.accent } else { l.t.text_disabled };
            l.p.fill_round(rail, 2.0 * s, l.t.control_border);
            l.p.fill_round(Rect { x1: x, ..rail }, 2.0 * s, filled);
            let thumb = Rect::new(x - 10.0 * s, mid - 10.0 * s, 20.0 * s, 20.0 * s);
            l.p.fill_round(thumb, 10.0 * s, l.t.surface);
            l.p.stroke_round(thumb, 10.0 * s, l.t.control_border, border);
            l.p.fill_round(thumb.inset(5.0 * s), 5.0 * s, filled);
        }
    }
}

fn focus_ring(l: &Look, state: &State, layout: &Layout) {
    if !state.focus_visible {
        return;
    }
    let Some(w) = state.focus.and_then(|f| layout.widgets.iter().find(|w| w.id == f)) else {
        return;
    };
    if w.role == Role::Field {
        return;
    }
    // Fluent focus visual: 2 px outer stroke, 1 px inner stroke.
    // https://learn.microsoft.com/windows/apps/design/input/guidelines-for-visualfocus
    let s = l.s.floor().max(1.0);
    let r = if w.role == Role::Document { w.rect.inset(2.0 * s) } else { w.rect.inset(-2.0 * s) };
    l.p.stroke_round(r, 6.0 * l.s, l.t.focus_outer, 2.0 * s);
    l.p.stroke_round(r.inset(2.0 * s), 4.0 * l.s, l.t.focus_inner, s);
}

fn keytips(l: &Look, state: &State, layout: &Layout) {
    let Some(scope) = state.keytips else {
        return;
    };
    let s = l.s;
    for w in layout.widgets.iter().filter(|w| w.enabled && w.access_key.is_some() && in_scope(w, scope)) {
        let key = w.access_key.unwrap().to_string();
        let size = 20.0 * s * state.text_scale.max(1.0);
        let x = (w.rect.x0 + w.rect.x1 - size) / 2.0;
        let badge = Rect::new(x, w.rect.y1 - size / 2.0, size, size);
        l.p.fill_round(badge, 4.0 * s, l.t.text);
        l.p.text(&key, badge, &l.f.caption, l.t.surface, Align::Center);
    }
}

fn tooltip(l: &Look, state: &State, layout: &Layout) {
    let Some(w) = state.tooltip.and_then(|id| layout.widgets.iter().find(|w| w.id == id)) else {
        return;
    };
    if w.tooltip.is_empty() {
        return;
    }
    let s = l.s;
    let (tw, th) = measure(&w.tooltip, &l.f.caption, 10_000.0);
    let (bw, bh) = (tw + 16.0 * s, th + 10.0 * s);
    let x = ((w.rect.x0 + w.rect.x1 - bw) / 2.0).clamp(4.0 * s, (state.size.0 - bw - 4.0 * s).max(0.0));
    let below = w.rect.y1 + 4.0 * s;
    let y = if below + bh > state.size.1 { w.rect.y0 - bh - 4.0 * s } else { below };
    let r = Rect::new(x, y, bw, bh);
    l.p.fill_round(r, 6.0 * s, l.t.surface);
    l.p.stroke_round(r, 6.0 * s, l.t.control_border, l.t.border_width * s.floor().max(1.0));
    l.p.text(&w.tooltip, r, &l.f.caption, l.t.text, Align::Center);
}

/// Draws everything between BeginDraw and EndDraw. Returns true when the
/// document shows complete content.
pub(super) fn draw(p: &Painter, bitmap: Option<&ID2D1Bitmap>, fonts: &Fonts, state: &mut State, layout: &Layout) -> bool {
    let look = Look { p, t: state.theme, f: fonts, s: state.scale };
    p.clear(state.theme.chrome);
    if state.quick.is_none() {
        title_bar(&look, state, layout);
        bars(&look, state, layout);
        sidebar_panel(&look, state, layout);
        super::organize::paint(p, state, layout);
    }
    let drew = if layout.empty.is_some() {
        p.fill(layout.document, state.theme.canvas);
        empty_state(&look, state, layout);
        false
    } else if state.quick.is_some() {
        super::quickview::paint(&look, bitmap, state, layout)
    } else if layout.contact_sheet {
        false
    } else {
        // The band an info bar pushed the document out of.
        p.fill(Rect { y1: layout.document.y0, ..layout.info_area }, state.theme.canvas);
        document::paint(p, bitmap, state, layout.document)
    };
    super::infobar::paint(&look, state, layout);
    super::findbar::paint(&look, state, layout);
    sheet(&look, state, layout);
    focus_ring(&look, state, layout);
    keytips(&look, state, layout);
    tooltip(&look, state, layout);
    drew
}

/// WM_PAINT body. Paints before any decode on launch; decode is scheduled
/// from tick once `painted` is set. After the frame, it asks the document
/// worker for what the view still needs.
pub(super) unsafe fn paint(hwnd: HWND) {
    with_state(|state| {
        state.painted = true;
        let (width, height) = (state.size.0 as u32, state.size.1 as u32);
        if width == 0 || height == 0 {
            return;
        }
        let document = state.layout().document;
        let gliding = document::prepare(state, document);
        super::bench::before_draw(state, document);
        let layout = state.layout();
        let result = (|| -> Result<bool> {
            if state.renderer.is_none() {
                state.renderer = Some(Renderer::new(hwnd, width, height)?);
            }
            if let (Some(renderer), Some(frame)) = (state.renderer.as_mut(), state.frame.as_ref()) {
                if renderer.bitmap.is_none() {
                    renderer.bitmap = Some(renderer.painter.upload(frame)?);
                }
            }
            let fonts = fonts(state.scale, state.text_scale)?;
            state.cache.new_frame();
            let renderer = state.renderer.take().unwrap();
            let uploaded = document::upload(state, &renderer.painter);
            // Ask for missing tiles before drawing: EndDraw waits for the
            // display, and the worker can render meanwhile.
            document::request(state, &layout);
            if uploaded.is_ok() {
                renderer.begin();
            }
            let drew = uploaded.is_ok() && draw(&renderer.painter, renderer.bitmap.as_ref(), &fonts, state, &layout);
            let ended = if uploaded.is_ok() { renderer.end() } else { Ok(()) };
            state.renderer = Some(renderer);
            uploaded?;
            ended?;
            Ok(drew)
        })();
        let drew = match result {
            Ok(drew) => drew,
            Err(error) => {
                // D2DERR_RECREATE_TARGET and other device loss: rebuild on the
                // next paint. Cached bitmaps belong to the lost target.
                state.renderer = None;
                state.cache.clear();
                state.sent.clear();
                if let Some(quick) = state.quick.as_mut() {
                    quick.forget_bitmaps();
                }
                state.status = format!("Windows could not draw this view: {error}");
                false
            }
        };
        if drew && !state.pending && !state.render_failed {
            state.content_drawn = true;
            if !state.marked {
                benchmark_marker(hwnd, state);
            }
        }
        let more = super::bench::after_present(hwnd, state, layout.document, drew);
        if gliding || more {
            invalidate(hwnd);
        }
    });
}

/// PFW_BENCH_OUT and PFW_BENCH_AUTOCLOSE, exactly as docs/contracts.md
/// defines. For a PDF, width and height are the first visible page's pixels.
unsafe fn benchmark_marker(hwnd: HWND, state: &mut State) {
    let Some(path) = std::env::var_os("PFW_BENCH_OUT") else {
        return;
    };
    let (width, height, pages) = match (&state.pdf, &state.frame) {
        (Some(v), _) => (state.stats.page_px.0, state.stats.page_px.1, v.sizes.len() as u32),
        (None, Some(frame)) => (frame.width, frame.height, frame.page_count),
        _ => return,
    };
    let mut counter = 0;
    let mut frequency = 0;
    if DwmFlush().is_ok() && QueryPerformanceCounter(&mut counter).is_ok() && QueryPerformanceFrequency(&mut frequency).is_ok() {
        let json = format!("{{\"first_content_qpc\":{counter},\"qpc_frequency\":{frequency},\"width\":{width},\"height\":{height},\"page_count\":{pages}}}");
        if std::fs::write(path, json).is_ok() {
            state.marked = true;
            if std::env::var_os("PFW_BENCH_AUTOCLOSE").is_some_and(|v| v == "1") {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::Frame,
        ui::{
            document::{PdfView, Tile},
            theme::{Mode, Rgba, palette},
            view::Zoom,
            widgets::Scope,
            worker::Workers,
        },
    };
    use std::{path::PathBuf, sync::Arc};
    use windows::{
        Win32::{
            Graphics::{
                Direct2D::{Common::*, *},
                Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
                Imaging::*,
            },
            System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx},
        },
        core::Interface,
    };

    /// A white page with grey text lines.
    fn page() -> Frame {
        let (width, height) = (612u32, 792u32);
        let mut pixels = vec![255u8; (width * height * 4) as usize];
        for line in 0..24u32 {
            let y0 = 80 + line * 26;
            for y in y0..y0 + 8 {
                let right = if line % 5 == 4 { 330 } else { 540 };
                for x in 72..right {
                    let i = ((y * width + x) * 4) as usize;
                    pixels[i..i + 3].copy_from_slice(&[96, 96, 96]);
                }
            }
        }
        Frame { width, height, pixels, page_count: 20, source_width: width, source_height: height }
    }

    /// A page tile cut from `page` as the renderer would draw it at `scale`.
    fn tile(page: &Frame, scale: f32, [x, y, w, h]: [u32; 4]) -> Frame {
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for row in y..y + h {
            for col in x..x + w {
                let sx = ((col as f32 / scale) as u32).min(page.width - 1);
                let sy = ((row as f32 / scale) as u32).min(page.height - 1);
                let i = ((sy * page.width + sx) * 4) as usize;
                pixels.extend_from_slice(&page.pixels[i..i + 4]);
            }
        }
        Frame { width: w, height: h, pixels, page_count: 1, source_width: w, source_height: h }
    }

    /// The thumbnail stand-in color, BGRA.
    const THUMB: [u8; 4] = [255, 200, 150, 255];

    /// Opens a synthetic 20-page PDF in `state` and fills the cache the way
    /// the document worker would: full tiles for `tiled` pages, thumbnails
    /// for `thumbs` pages.
    fn open_pdf(painter: &Painter, state: &mut State, tiled: &[u32], thumbs: &[u32]) {
        let path = PathBuf::from("Quarterly report.pdf");
        state.pdf = Some(PdfView::new(path, vec![[612.0, 792.0]; 20], Arc::default(), 0));
        state.status = "Page 1 of 20".into();
        let document = state.layout().document;
        document::prepare(state, document);
        let source = page();
        for item in document::wanted(state, &state.layout()) {
            let frame = match item.key.work {
                Work::Tile { page, .. } if tiled.contains(&page) => tile(&source, item.scale, item.region),
                Work::Thumb { page } if thumbs.contains(&page) => {
                    let (w, h) = (item.region[2], item.region[3]);
                    Frame { width: w, height: h, pixels: THUMB.repeat((w * h) as usize), page_count: 1, source_width: w, source_height: h }
                }
                _ => continue,
            };
            let bitmap = painter.upload(&frame).unwrap();
            state.cache.insert(item.key, Tile { bitmap, fresh: true }, frame.pixels.len());
        }
    }

    unsafe fn target(size: (u32, u32)) -> (IWICBitmap, Painter) {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).unwrap();
        let bitmap = wic.CreateBitmap(size.0, size.1, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad).unwrap();
        let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).unwrap();
        let target = factory
            .CreateWicBitmapRenderTarget(
                &bitmap,
                &D2D1_RENDER_TARGET_PROPERTIES {
                    pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                    dpiX: 96.0,
                    dpiY: 96.0,
                    ..Default::default()
                },
            )
            .unwrap();
        target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        (bitmap, Painter::new(target.cast().unwrap()).unwrap())
    }

    fn pixels(bitmap: &IWICBitmap, size: (u32, u32)) -> Frame {
        let mut pixels = vec![0u8; (size.0 * size.1 * 4) as usize];
        unsafe { bitmap.CopyPixels(std::ptr::null(), size.0 * 4, &mut pixels).unwrap() };
        Frame { width: size.0, height: size.1, pixels, page_count: 1, source_width: size.0, source_height: size.1 }
    }

    /// Draws one scene into a WIC bitmap with the window's own drawing code
    /// and returns its premultiplied BGRA pixels. No window is created.
    unsafe fn render(mode: Mode, scene: &str, size: (u32, u32), scale: f32, text_scale: f32) -> (Frame, Layout) {
        let (bitmap, painter) = target(size);
        let paths: Vec<PathBuf> = match scene {
            "empty" => Vec::new(),
            "image" => vec![PathBuf::from("Beach photo.jpg")],
            _ => vec![PathBuf::from("Quarterly report.pdf"), PathBuf::from("Beach photo.jpg"), PathBuf::from("Lease agreement.pdf")],
        };
        let workers = Workers::start(HWND::default()).unwrap();
        let mut state = State::new(workers, &paths, (size.0 as f32, size.1 as f32), scale, text_scale, palette(mode));
        state.animations = false;
        match scene {
            "document" | "image" => {
                state.sidebar_open = true;
                state.set_markup(true);
                state.markup = Some(crate::model::AnnotationKind::Highlight);
                state.hover = Some(WidgetId::Command(crate::ui::commands::Command::Rotate));
                state.tooltip = state.hover;
                state.focus = Some(WidgetId::Command(crate::ui::commands::Command::ZoomIn));
                state.focus_visible = true;
            }
            "keytips" => {
                state.set_markup(true);
                state.keytips = Some(Scope::Root);
            }
            "sheet" => {
                state.sheet = Some(crate::ui::sheet::Sheet {
                    title: "Resize image".into(),
                    message: "The new size applies when you save a copy.".into(),
                    fields: vec![
                        crate::ui::sheet::Field { label: "Width in pixels".into(), edit: HWND::default() },
                        crate::ui::sheet::Field { label: "Height in pixels".into(), edit: HWND::default() },
                    ],
                    buttons: vec!["Resize".into(), "Cancel".into()],
                    cancel: 1,
                    result: None,
                    controls: Vec::new(),
                    live: None,
                });
                state.focus = Some(WidgetId::SheetField(0));
                state.focus_visible = false;
            }
            _ => {}
        }
        let mut frame_bitmap = None;
        if scene == "image" {
            let frame = page();
            frame_bitmap = Some(painter.upload(&frame).unwrap());
            state.frame = Some(frame);
            state.status = state.subtitle();
        } else if !paths.is_empty() {
            open_pdf(&painter, &mut state, &[0, 1], &[0, 1, 2]);
        }
        let fonts = fonts(scale, text_scale).unwrap();
        painter.target.BeginDraw();
        let layout = state.layout();
        let drew = draw(&painter, frame_bitmap.as_ref(), &fonts, &mut state, &layout);
        painter.target.EndDraw(None, None).unwrap();
        assert_eq!(drew, !paths.is_empty(), "{scene}");
        (pixels(&bitmap, size), layout)
    }

    /// Tiles draw where they arrived; a page without tiles shows its
    /// thumbnail stretched, so it is not blank; a page with neither counts
    /// as blank for the scroll benchmark.
    #[test]
    fn pdf_view_draws_tiles_and_thumbnail_placeholders_headless() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let size = (1000, 900);
            let (bitmap, painter) = target(size);
            let workers = Workers::start(HWND::default()).unwrap();
            let paths = vec![PathBuf::from("Quarterly report.pdf")];
            let mut state = State::new(workers, &paths, (size.0 as f32, size.1 as f32), 1.0, 1.0, palette(Mode::Light));
            // Pages of 408 by 528 pixels: page 1 whole, the top of page 2.
            state.zoom = Zoom::Ratio(0.5);
            open_pdf(&painter, &mut state, &[0], &[1]);
            let fonts = fonts(1.0, 1.0).unwrap();
            let layout = state.layout();
            painter.target.BeginDraw();
            let drew = draw(&painter, None, &fonts, &mut state, &layout);
            painter.target.EndDraw(None, None).unwrap();
            assert!(!drew, "page 2 has no tiles yet");
            let stats = state.stats.clone();
            assert!(stats.visible > 2 && stats.missing > 0, "{stats:?}");
            assert!(stats.blank.is_empty(), "page 2 shows its thumbnail");
            let shot = pixels(&bitmap, size);
            let g = document::geometry_in(&state, layout.document).unwrap();
            let rect = |i: usize| {
                let r = g.layout.pages[i].1;
                (layout.document.x0 + r.x0 - g.left, layout.document.y0 + r.y0 - g.top, r.width() / 612.0)
            };
            let (x, y, k) = rect(0);
            let text = pixel(&shot, (x + 100.0 * k) as u32, (y + 83.0 * k) as u32);
            assert!(close(text, Rgba(96.0 / 255.0, 96.0 / 255.0, 96.0 / 255.0, 1.0)), "page 1 text line from its tile: {text:?}");
            let halo = pixel(&shot, (x + 30.0) as u32, (y - 2.0) as u32);
            let far_canvas = pixel(&shot, (x + 30.0) as u32, (y - 6.0) as u32);
            assert!(!close(halo, palette(Mode::Light).canvas), "a page has visible depth outside its edge");
            assert!(close(far_canvas, palette(Mode::Light).canvas), "the halo remains restrained");
            let (x, y, _) = rect(1);
            let thumb = pixel(&shot, (x + 30.0) as u32, (y + 10.0).min(size.1 as f32 - 40.0) as u32);
            assert!(close(thumb, Rgba(150.0 / 255.0, 200.0 / 255.0, 1.0, 1.0)), "page 2 placeholder: {thumb:?}");
            let gap = pixel(&shot, (x + 30.0) as u32, (y - 6.0) as u32);
            assert!(close(gap, palette(Mode::Light).canvas), "the gap between pages");
            // Without a thumbnail, the missing tiles count as blank.
            state.cache.clear();
            painter.target.BeginDraw();
            draw(&painter, None, &fonts, &mut state, &layout);
            painter.target.EndDraw(None, None).unwrap();
            assert_eq!(state.stats.blank.len() as u32, state.stats.visible);
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("artifacts/screenshots/light-pdf-placeholder.png");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let _ = std::fs::remove_file(&path);
            crate::imaging::export_frame(&shot, &path).unwrap();
        }
    }

    /// Before the first page is up, its tiles go first; after that, a
    /// page's thumbnail goes before its tiles, so scrolling never shows a
    /// blank page for long. Sidebar thumbnails come after the view's work.
    #[test]
    fn work_list_puts_the_first_page_then_placeholders_first() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let (_, painter) = target((10, 10));
            let workers = Workers::start(HWND::default()).unwrap();
            let paths = vec![PathBuf::from("Quarterly report.pdf")];
            let mut state = State::new(workers, &paths, (1000.0, 900.0), 1.0, 1.0, palette(Mode::Light));
            state.zoom = Zoom::Ratio(0.5);
            open_pdf(&painter, &mut state, &[], &[]);
            let layout = state.layout();
            let kinds = |state: &State| -> Vec<&'static str> {
                document::wanted(state, &state.layout())
                    .iter()
                    .map(|i| match i.key.work {
                        Work::Tile { .. } => "tile",
                        Work::Thumb { .. } => "thumb",
                        _ => "other",
                    })
                    .collect()
            };
            let first = kinds(&state);
            assert_eq!(&first[..5], &["tile", "tile", "tile", "thumb", "thumb"], "three tiles in view, then placeholders");
            assert!(first[5..].contains(&"tile"), "then the margin");
            state.content_drawn = true;
            let later = kinds(&state);
            assert_eq!(&later[..2], &["thumb", "thumb"], "pages 1 and 2 in view get placeholders first");
            let items = document::wanted(&state, &layout);
            let tiles: Vec<u32> = items.iter().filter_map(|i| if let Work::Tile { page, .. } = i.key.work { Some(page) } else { None }).collect();
            assert!(tiles.iter().all(|p| *p <= 2), "only pages in view and the margin: {tiles:?}");
            open_pdf(&painter, &mut state, &[], &[0, 1]);
            let sizes = state.pdf.as_ref().unwrap().sizes.clone();
            state.pdf.as_mut().unwrap().update(sizes, Arc::new(vec![crate::model::PdfEdit::RotateRight { page: 0 }]), &mut state.cache);
            assert!(
                document::wanted(&state, &state.layout()).iter().any(|item| item.key.work == Work::Thumb { page: 0 }),
                "an edited visible page replaces its stale thumbnail"
            );
            state.sidebar_open = true;
            let with_sidebar = kinds(&state);
            assert!(with_sidebar.len() > later.len(), "sidebar thumbnails come last");
            // An image asks for its neighbors, never for tiles.
            state.pdf = None;
            state.frame = Some(page());
            state.displayed = Some((paths[0].clone(), 0));
            state.path = Some(paths[0].clone());
            let neighbors = document::wanted(&state, &state.layout());
            assert_eq!(neighbors.len(), 2);
            assert!(matches!(neighbors[0].key.work, Work::Predecode { delta: 1, .. }));
            assert!(matches!(neighbors[1].key.work, Work::Predecode { delta: -1, .. }));
        }
    }

    #[test]
    fn four_k_view_and_sidebar_work_converges_in_the_bitmap_cache() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let (_, painter) = target((10, 10));
            let workers = Workers::start(HWND::default()).unwrap();
            let path = PathBuf::from("large.pdf");
            let mut state = State::new(workers, &[path.clone()], (3840.0, 2160.0), 1.0, 1.0, palette(Mode::Light));
            state.animations = false;
            state.zoom = Zoom::Ratio(4.0);
            state.sidebar_open = true;
            state.content_drawn = true;
            state.pdf = Some(PdfView::new(path, vec![[612.0, 792.0]; 20], Arc::default(), 0));
            let document = state.layout().document;
            document::prepare(&mut state, document);

            let source = page();
            let current = document::wanted(&state, &state.layout());
            let first_tile = current.iter().find(|item| matches!(item.key.work, Work::Tile { .. })).unwrap();
            let mut old_key = first_tile.key;
            if let Work::Tile { page, col, row, .. } = old_key.work {
                old_key.work = Work::Tile { page, scale: 0.5f32.to_bits(), col, row };
            }
            let old_frame = tile(&source, first_tile.scale, first_tile.region);
            state.cache.insert(old_key, Tile { bitmap: painter.upload(&old_frame).unwrap(), fresh: true }, old_frame.pixels.len());

            let mut active: Vec<Key> = Vec::new();
            let mut drained = false;
            for round in 0..3 {
                state.cache.new_frame();
                for key in &active {
                    let drawn = match key.work {
                        Work::Tile { page, .. } => page == state.page,
                        Work::Thumb { .. } => true,
                        _ => false,
                    };
                    if drawn {
                        state.cache.get(key);
                    }
                }
                let items = document::wanted(&state, &state.layout());
                let frames: Vec<_> = items.into_iter().filter(|item| matches!(item.key.work, Work::Tile { .. } | Work::Thumb { .. })).collect();
                if frames.is_empty() {
                    assert!(round > 0);
                    drained = true;
                    continue;
                }
                assert!(!drained, "a later frame must not re-request evicted margin work");
                if round == 0 {
                    active = frames.iter().map(|item| item.key).collect();
                    assert!(frames.iter().any(|item| matches!(item.key.work, Work::Thumb { page } if page > 1)), "visible sidebar thumbnails are reserved");
                }
                for item in frames {
                    let frame = match item.key.work {
                        Work::Tile { .. } => tile(&source, item.scale, item.region),
                        Work::Thumb { .. } => {
                            let (w, h) = (item.region[2], item.region[3]);
                            Frame { width: w, height: h, pixels: THUMB.repeat((w * h) as usize), page_count: 1, source_width: w, source_height: h }
                        }
                        _ => unreachable!(),
                    };
                    state.cache.insert(item.key, Tile { bitmap: painter.upload(&frame).unwrap(), fresh: true }, frame.pixels.len());
                }
            }
            assert!(drained, "the bitmap work queue must converge");
            assert!(
                document::wanted(&state, &state.layout()).iter().all(|item| !matches!(item.key.work, Work::Tile { .. } | Work::Thumb { .. })),
                "retained view work must drain the queue"
            );
            assert!(active.iter().all(|key| state.cache.peek(key).is_some()), "visible and sidebar entries remain resident");
            assert!(state.cache.used() <= super::super::cache::TILE_BUDGET);
        }
    }

    fn pixel(frame: &Frame, x: u32, y: u32) -> Rgba {
        let i = ((y * frame.width + x) * 4) as usize;
        let p = &frame.pixels[i..i + 4];
        Rgba(p[2] as f32 / 255.0, p[1] as f32 / 255.0, p[0] as f32 / 255.0, p[3] as f32 / 255.0)
    }

    fn close(a: Rgba, b: Rgba) -> bool {
        (a.0 - b.0).abs() < 0.02 && (a.1 - b.1).abs() < 0.02 && (a.2 - b.2).abs() < 0.02
    }

    fn fixture_ink(frame: &Frame) -> usize {
        let (x0, x1) = (frame.width / 4, frame.width * 3 / 4);
        let (y0, y1) = (frame.height / 4, frame.height * 3 / 4);
        (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| ((y * frame.width + x) * 4) as usize))
            .filter(|i| frame.pixels[*i..*i + 3].iter().all(|channel| (88..=104).contains(channel)))
            .count()
    }

    /// Times the first chrome draw in a fresh process (font loading, text
    /// layout, glyph rasterization) on a software WIC target. Run alone:
    /// cargo test --release --bin preview-for-windows first_frame_cost -- --ignored --nocapture
    #[test]
    #[ignore = "timing probe; run alone in a fresh process"]
    fn first_frame_cost() {
        let t = std::time::Instant::now();
        let fonts = fonts(1.0, 1.0).unwrap();
        let formats = t.elapsed();
        let t = std::time::Instant::now();
        let _ = measure("Quarterly report.pdf", &fonts.body, 1000.0);
        let body = t.elapsed();
        let t = std::time::Instant::now();
        let _ = measure("Quarterly report.pdf", &fonts.strong, 1000.0);
        let strong = t.elapsed();
        let t = std::time::Instant::now();
        let _ = measure("\u{E8A3}\u{E70F}\u{E7AD}", &fonts.icon, 1000.0);
        let icons = t.elapsed();
        println!("text formats {formats:?}; first Segoe UI layout {body:?}; first semibold layout {strong:?}; first icon font layout {icons:?}");
    }

    /// Headless screenshots of every theme, written to artifacts/screenshots.
    /// The pixel checks prove each theme reaches the chrome and the canvas.
    #[test]
    fn every_theme_draws_chrome_document_and_sheets_headless() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("artifacts/screenshots");
            std::fs::create_dir_all(&out).unwrap();
            for (mode, name) in [(Mode::Light, "light"), (Mode::Dark, "dark"), (Mode::Contrast, "contrast")] {
                let theme = palette(mode);
                for scene in ["document", "image", "empty", "sheet", "keytips"] {
                    let size = (1100, 760);
                    let (shot, layout) = render(mode, scene, size, 1.0, 1.0);
                    // Title bar drag space and the bottom-right canvas corner.
                    assert!(close(pixel(&shot, 700, 10), theme.chrome), "{name} {scene} title bar");
                    assert!(close(pixel(&shot, 700, 45), theme.bar) || scene == "sheet", "{name} {scene} toolbar");
                    if scene != "sheet" {
                        assert!(close(pixel(&shot, size.0 - 4, size.1 - 40), theme.canvas), "{name} {scene} canvas");
                    }
                    if scene == "image" {
                        assert!(!close(pixel(&shot, size.0 / 2, size.1 / 2), theme.canvas), "{name} image content");
                    }
                    if matches!(scene, "document" | "image") {
                        assert!(fixture_ink(&shot) > 100, "{name} {scene} must contain the synthetic gray document ink");
                    }
                    if scene == "document" {
                        let side = layout.sidebar.expect("PDF fixture has a sidebar");
                        let header = pixel(&shot, (side.x0 + 4.0) as u32, ((side.y0 + layout.sidebar_panel.y0) / 2.0) as u32);
                        let body = pixel(&shot, (side.x0 + 4.0) as u32, (layout.sidebar_panel.y0 + 8.0) as u32);
                        assert!(close(header, theme.bar), "{name} sidebar header uses the commanding layer");
                        assert!(close(body, theme.content_layer), "{name} sidebar body uses the content layer");

                        let selected = layout.widgets.iter().find(|w| w.id == WidgetId::Command(crate::ui::commands::Command::Highlight)).unwrap();
                        let indicator = pixel(&shot, ((selected.rect.x0 + selected.rect.x1) / 2.0) as u32, (selected.rect.y1 - 1.0) as u32);
                        assert!(close(indicator, theme.accent), "{name} checked markup uses an accent indicator");
                        if mode != Mode::Contrast {
                            let center =
                                pixel(&shot, ((selected.rect.x0 + selected.rect.x1) / 2.0) as u32, ((selected.rect.y0 + selected.rect.y1) / 2.0) as u32);
                            assert!(!close(center, theme.accent), "{name} checked markup is not a solid accent tile");
                        }
                    }
                    if scene == "sheet" && mode != Mode::Contrast {
                        let card = layout.sheet.as_ref().unwrap().card;
                        let near = pixel(&shot, (card.x0 - 1.0) as u32, ((card.y0 + card.y1) / 2.0) as u32);
                        let far = pixel(&shot, (card.x0 - 14.0) as u32, ((card.y0 + card.y1) / 2.0) as u32);
                        let luma = |c: Rgba| 0.2126 * c.0 + 0.7152 * c.1 + 0.0722 * c.2;
                        assert!(luma(near) < luma(far), "{name} sheet has an opaque-card halo");
                    }
                    let path = out.join(format!("{name}-{scene}.png"));
                    let _ = std::fs::remove_file(&path);
                    crate::imaging::export_frame(&shot, &path).unwrap();
                }
            }
            // 150% display scale and 225% text size, the largest Windows allows.
            for (scale, text_scale, file) in [(1.5, 1.0, "light-document-150dpi.png"), (1.0, 2.25, "light-document-text225.png")] {
                let size = ((1100.0 * scale) as u32, (760.0 * scale) as u32);
                let (shot, layout) = render(Mode::Light, "document", size, scale, text_scale);
                assert!(close(pixel(&shot, size.0 - 4, size.1 - 60), palette(Mode::Light).canvas), "{file}");
                if text_scale == 2.25 {
                    let rotate = layout.widgets.iter().find(|w| w.id == WidgetId::Command(crate::ui::commands::Command::Rotate)).unwrap();
                    assert_eq!(rotate.rect.width(), 36.0, "225% text preserves 36 DIP icon targets");
                    assert!(layout.title_bar.height() >= 26.0 * text_scale + 12.0);
                }
                let path = out.join(file);
                let _ = std::fs::remove_file(&path);
                crate::imaging::export_frame(&shot, &path).unwrap();
            }
        }
    }
}
