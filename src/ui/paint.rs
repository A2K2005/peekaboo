//! Draws the window: title bar, toolbar, markup bar, sidebar, status,
//! empty state, sheets, focus rectangles, keytips, and tooltips. The
//! document itself is drawn by document.rs.
use super::{
    app::{with_state, State},
    commands::glyph,
    document,
    render::{fonts, measure, Align, Fonts, Painter, Renderer},
    theme::Theme,
    widgets::{in_scope, Layout, Rect, Region, Role, Widget, WidgetId},
};
use windows::{
    core::Result,
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        Graphics::Dwm::DwmFlush,
        System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
        UI::{Input::KeyboardAndMouse::GetFocus, WindowsAndMessaging::*},
    },
};

const SIDEBAR_PLACEHOLDERS: [&str; 3] = [
    "Page thumbnails will show here.",
    "The table of contents will show here.",
    "Notes and highlights will show here.",
];

struct Look<'a> {
    p: &'a Painter,
    t: Theme,
    f: &'a Fonts,
    s: f32,
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

fn icon_button(l: &Look, state: &State, w: &Widget, solid_when_checked: bool) {
    let fill = background(state, w);
    let checked = w.checked == Some(true);
    let mut color = if w.enabled { l.t.text } else { l.t.text_disabled };
    let r = w.rect;
    if checked && solid_when_checked {
        l.p.fill_round(r, 4.0 * l.s, l.t.accent);
        color = l.t.on_accent;
    } else if let Some(fill) = fill {
        l.p.fill_round(r, 4.0 * l.s, if fill == Fill::Hover { l.t.hover } else { l.t.pressed });
        color = l.t.hover_text;
    } else if checked {
        l.p.fill_round(r, 4.0 * l.s, l.t.hover);
        color = l.t.hover_text;
    }
    if checked && !solid_when_checked {
        let bar = Rect { x0: r.x0 + 12.0 * l.s, y0: r.y1 - 3.0 * l.s, x1: r.x1 - 12.0 * l.s, y1: r.y1 };
        l.p.fill_round(bar, 1.5 * l.s, l.t.accent);
    }
    if let Some(g) = w.glyph {
        l.p.glyph(g, r, &l.f.icon, color);
    }
}

fn text_button(l: &Look, state: &State, w: &Widget) {
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
        l.p.fill_round(r, 4.0 * l.s, l.t.field);
    }
    l.p.fill_round(r, 4.0 * l.s, back);
    if !w.primary || l.t.mode == super::theme::Mode::Contrast {
        l.p.stroke_round(r, 4.0 * l.s, l.t.border, l.t.border_width * l.s.floor().max(1.0));
    }
    l.p.text(&w.label, r.inset(4.0 * l.s), &l.f.body, color, Align::Center);
}

fn title_bar(l: &Look, state: &State, layout: &Layout) {
    let s = l.s;
    let bar = layout.title_bar;
    l.p.push_clip(bar);
    for w in layout.widgets.iter().filter(|w| w.region == Region::TitleBar) {
        match w.id {
            WidgetId::Tab(_) => {
                let selected = w.checked == Some(true);
                let r = w.rect;
                if selected {
                    // Round the top corners only; the bottom joins the toolbar.
                    l.p.fill_round(Rect { y1: r.y1 + 8.0 * s, ..r }, 8.0 * s, l.t.selected);
                } else if let Some(fill) = background(state, w) {
                    let inner = Rect { x0: r.x0 + 2.0 * s, y0: r.y0 + 2.0 * s, x1: r.x1 - 2.0 * s, y1: r.y1 - 4.0 * s };
                    l.p.fill_round(inner, 4.0 * s, if fill == Fill::Hover { l.t.hover } else { l.t.pressed });
                }
                let color = if selected { l.t.selected_text } else if state.hover == Some(w.id) { l.t.hover_text } else { l.t.text_secondary };
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
            WidgetId::NewTab => icon_button(l, state, w, false),
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
                    color = if w.id == WidgetId::Close && l.t.mode != super::theme::Mode::Contrast {
                        super::theme::Rgba::hex(0xffffff)
                    } else {
                        l.t.hover_text
                    };
                }
                l.p.glyph(g, w.rect, &l.f.caption_icon, color);
            }
            _ => {}
        }
    }
    if state.tabs.is_empty() {
        if let Some(plus) = layout.widgets.iter().find(|w| w.id == WidgetId::NewTab) {
            let r = Rect { x0: plus.rect.x1 + 12.0 * s, y0: bar.y0, x1: bar.x1 - 3.0 * 46.0 * s, y1: bar.y1 };
            l.p.text("Preview for Windows", r, &l.f.body, l.t.text_secondary, Align::Leading);
        }
    }
    l.p.pop_clip();
}

fn bars(l: &Look, state: &State, layout: &Layout) {
    let s = l.s;
    l.p.fill(layout.toolbar, l.t.selected);
    if let Some(path) = &state.path {
        let name = super::app::file_name(path);
        l.p.text(&name, layout.title_text, &l.f.strong, l.t.text, Align::Leading);
    }
    for w in layout.widgets.iter().filter(|w| w.region == Region::Toolbar) {
        icon_button(l, state, w, false);
    }
    if let Some(bar) = layout.markup_bar {
        l.p.fill(bar, l.t.surface);
        l.p.fill(Rect { y0: bar.y1 - s, ..bar }, l.t.border);
        for w in layout.widgets.iter().filter(|w| w.region == Region::MarkupBar) {
            icon_button(l, state, w, true);
        }
    } else {
        l.p.fill(Rect { y0: layout.toolbar.y1 - s, ..layout.toolbar }, l.t.border);
    }
    if let Some(side) = layout.sidebar {
        l.p.fill(side, l.t.surface);
        l.p.fill(Rect { x0: side.x1 - s, ..side }, l.t.border);
        for w in layout.widgets.iter().filter(|w| w.role == Role::SidebarTab) {
            let selected = w.checked == Some(true);
            if let Some(fill) = background(state, w) {
                l.p.fill_round(w.rect, 4.0 * s, if fill == Fill::Hover { l.t.hover } else { l.t.pressed });
            }
            let font = if selected { &l.f.strong } else { &l.f.body };
            let color = if selected { l.t.text } else { l.t.text_secondary };
            l.p.text(&w.label, w.rect, font, color, Align::Center);
            if selected {
                let bar = Rect { x0: w.rect.x0 + 16.0 * s, y0: w.rect.y1 - 3.0 * s, x1: w.rect.x1 - 16.0 * s, y1: w.rect.y1 };
                l.p.fill_round(bar, 1.5 * s, l.t.accent);
            }
        }
        let text = SIDEBAR_PLACEHOLDERS[state.sidebar_tab.min(2)];
        l.p.text(text, layout.sidebar_panel, &l.f.wrap, l.t.text_secondary, Align::Leading);
    }
    let status = layout.status;
    l.p.fill(status, l.t.selected);
    l.p.fill(Rect { y1: status.y0 + s, ..status }, l.t.border);
    let text = Rect { x0: status.x0 + 12.0 * s, y0: status.y0, x1: status.x1 - 12.0 * s, y1: status.y1 };
    l.p.text(&state.status, text, &l.f.caption, l.t.text_secondary, Align::Leading);
}

fn empty_state(l: &Look, state: &State, layout: &Layout) {
    let Some(empty) = &layout.empty else {
        return;
    };
    l.p.text("Open a PDF or image", empty.heading, &l.f.title, l.t.text, Align::Center);
    for w in layout.widgets.iter().filter(|w| w.region == Region::Document) {
        text_button(l, state, w);
    }
    l.p.text("Recent files", empty.recent_heading, &l.f.strong, l.t.text, Align::Center);
    l.p.text("Files you open will show here.", empty.recent, &l.f.body, l.t.text_secondary, Align::Center);
}

fn sheet(l: &Look, state: &State, layout: &Layout) {
    let (Some(sheet), Some(card)) = (&state.sheet, &layout.sheet) else {
        return;
    };
    let s = l.s;
    l.p.fill(Rect { y0: layout.title_bar.y1, ..Rect::new(0.0, 0.0, state.size.0, state.size.1) }, l.t.scrim);
    l.p.fill_round(card.card, 8.0 * s, l.t.surface);
    l.p.stroke_round(card.card, 8.0 * s, l.t.border, l.t.border_width * s.floor().max(1.0));
    l.p.text(&sheet.title, card.title, &l.f.title, l.t.text, Align::Leading);
    l.p.text(&sheet.message, card.message, &l.f.wrap, l.t.text, Align::Leading);
    let focused = unsafe { GetFocus() };
    for ((field, label), rect) in sheet.fields.iter().zip(&card.labels).zip(&card.fields) {
        l.p.text(&field.label, *label, &l.f.body, l.t.text, Align::Leading);
        l.p.fill_round(*rect, 4.0 * s, l.t.field);
        l.p.stroke_round(*rect, 4.0 * s, l.t.border, l.t.border_width * s.floor().max(1.0));
        // Fluent text boxes mark focus with an accent line at the bottom.
        let line = if field.edit == focused { 2.0 * s } else { 0.0 };
        if line > 0.0 {
            l.p.fill_round(Rect { y0: rect.y1 - line, ..*rect }, s, l.t.accent);
        }
    }
    for w in layout.widgets.iter().filter(|w| w.region == Region::Sheet && w.role == Role::Button) {
        text_button(l, state, w);
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
    l.p.fill_round(r, 4.0 * s, l.t.surface);
    l.p.stroke_round(r, 4.0 * s, l.t.border, l.t.border_width * s.floor().max(1.0));
    l.p.text(&w.tooltip, r, &l.f.caption, l.t.text, Align::Center);
}

/// WM_PAINT body. Paints before any decode on launch; decode is scheduled
/// from tick once `painted` is set.
pub(super) unsafe fn paint(hwnd: HWND) {
    with_state(|state| {
        state.painted = true;
        let (width, height) = (state.size.0 as u32, state.size.1 as u32);
        if width == 0 || height == 0 {
            return;
        }
        let result = (|| -> Result<bool> {
            if state.renderer.is_none() {
                state.renderer = Some(Renderer::new(hwnd, width, height)?);
            }
            if let (Some(renderer), Some(frame)) = (state.renderer.as_mut(), state.frame.as_ref()) {
                if renderer.bitmap.is_none() {
                    renderer.upload(frame)?;
                }
            }
            let fonts = fonts(state.scale, state.text_scale)?;
            let layout = state.layout();
            let renderer = state.renderer.take().unwrap();
            renderer.begin();
            let look = Look { p: &renderer.painter, t: state.theme, f: &fonts, s: state.scale };
            look.p.clear(state.theme.chrome);
            title_bar(&look, state, &layout);
            bars(&look, state, &layout);
            let drew = if layout.empty.is_some() {
                look.p.fill(layout.document, state.theme.canvas);
                empty_state(&look, state, &layout);
                false
            } else {
                document::paint(&renderer, state, layout.document)
            };
            sheet(&look, state, &layout);
            focus_ring(&look, state, &layout);
            keytips(&look, state, &layout);
            tooltip(&look, state, &layout);
            let ended = renderer.end();
            state.renderer = Some(renderer);
            ended?;
            Ok(drew)
        })();
        match result {
            Ok(true) if !state.pending && !state.marked && !state.render_failed => benchmark_marker(hwnd, state),
            Err(error) => {
                // D2DERR_RECREATE_TARGET and other device loss: rebuild on the next paint.
                state.renderer = None;
                state.status = format!("Windows could not draw this view: {error}");
            }
            _ => {}
        }
    });
}

/// PFW_BENCH_OUT and PFW_BENCH_AUTOCLOSE, exactly as docs/contracts.md defines.
unsafe fn benchmark_marker(hwnd: HWND, state: &mut State) {
    let (Some(path), Some(frame)) = (std::env::var_os("PFW_BENCH_OUT"), state.frame.as_ref()) else {
        return;
    };
    let mut counter = 0;
    let mut frequency = 0;
    if DwmFlush().is_ok() && QueryPerformanceCounter(&mut counter).is_ok() && QueryPerformanceFrequency(&mut frequency).is_ok() {
        let json = format!(
            "{{\"first_content_qpc\":{counter},\"qpc_frequency\":{frequency},\"width\":{},\"height\":{},\"page_count\":{}}}",
            frame.width, frame.height, frame.page_count
        );
        if std::fs::write(path, json).is_ok() {
            state.marked = true;
            if std::env::var_os("PFW_BENCH_AUTOCLOSE").is_some_and(|v| v == "1") {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }
}
