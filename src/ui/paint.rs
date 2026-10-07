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
        Graphics::Direct2D::ID2D1Bitmap,
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
    l.p.fill(layout.toolbar, l.t.bar);
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
    l.p.fill(status, l.t.bar);
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

/// Draws everything between BeginDraw and EndDraw. Returns true when the
/// document frame was drawn.
pub(super) fn draw(p: &Painter, bitmap: Option<&ID2D1Bitmap>, fonts: &Fonts, state: &mut State) -> bool {
    let layout = state.layout();
    let look = Look { p, t: state.theme, f: fonts, s: state.scale };
    p.clear(state.theme.chrome);
    title_bar(&look, state, &layout);
    bars(&look, state, &layout);
    let drew = if layout.empty.is_some() {
        p.fill(layout.document, state.theme.canvas);
        empty_state(&look, state, &layout);
        false
    } else {
        document::paint(p, bitmap, state, layout.document)
    };
    sheet(&look, state, &layout);
    focus_ring(&look, state, &layout);
    keytips(&look, state, &layout);
    tooltip(&look, state, &layout);
    drew
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
                    renderer.bitmap = Some(renderer.painter.upload(frame)?);
                }
            }
            let fonts = fonts(state.scale, state.text_scale)?;
            let renderer = state.renderer.take().unwrap();
            renderer.begin();
            let drew = draw(&renderer.painter, renderer.bitmap.as_ref(), &fonts, state);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::Frame, ui::theme::{palette, Mode, Rgba}, ui::widgets::Scope, ui::worker::Workers};
    use std::path::PathBuf;
    use windows::{
        core::Interface,
        Win32::{
            Graphics::{Direct2D::{Common::*, *}, Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM, Imaging::*},
            System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED},
        },
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

    /// Draws one scene into a WIC bitmap with the window's own drawing code
    /// and returns its premultiplied BGRA pixels. No window is created.
    unsafe fn render(mode: Mode, scene: &str, size: (u32, u32), scale: f32, text_scale: f32) -> Frame {
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
        let painter = Painter::new(target.cast().unwrap()).unwrap();
        let paths: Vec<PathBuf> = if scene == "empty" {
            Vec::new()
        } else {
            vec![PathBuf::from("Quarterly report.pdf"), PathBuf::from("Beach photo.jpg"), PathBuf::from("Lease agreement.pdf")]
        };
        let workers = Workers::start(HWND::default()).unwrap();
        let mut state = State::new(workers, &paths, (size.0 as f32, size.1 as f32), scale, text_scale, palette(mode));
        state.animations = false;
        let mut frame_bitmap = None;
        if !paths.is_empty() {
            let frame = page();
            frame_bitmap = Some(painter.upload(&frame).unwrap());
            state.frame = Some(frame);
            state.status = "Page 1 of 20".into();
        }
        match scene {
            "document" => {
                state.sidebar_open = true;
                state.set_markup(true);
                state.markup = Some(crate::model::AnnotationKind::Highlight);
                state.hover = Some(WidgetId::Command(crate::ui::commands::Command::Rotate));
                state.tooltip = state.hover;
                state.focus = Some(WidgetId::Command(crate::ui::commands::Command::ZoomMenu));
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
                    buttons: vec!["OK".into(), "Cancel".into()],
                    cancel: 1,
                    result: None,
                });
                state.focus = Some(WidgetId::SheetButton(1));
                state.focus_visible = true;
            }
            _ => {}
        }
        let fonts = fonts(scale, text_scale).unwrap();
        painter.target.BeginDraw();
        let drew = draw(&painter, frame_bitmap.as_ref(), &fonts, &mut state);
        painter.target.EndDraw(None, None).unwrap();
        assert_eq!(drew, !paths.is_empty());
        let mut pixels = vec![0u8; (size.0 * size.1 * 4) as usize];
        bitmap.CopyPixels(std::ptr::null(), size.0 * 4, &mut pixels).unwrap();
        Frame { width: size.0, height: size.1, pixels, page_count: 1, source_width: size.0, source_height: size.1 }
    }

    fn pixel(frame: &Frame, x: u32, y: u32) -> Rgba {
        let i = ((y * frame.width + x) * 4) as usize;
        let p = &frame.pixels[i..i + 4];
        Rgba(p[2] as f32 / 255.0, p[1] as f32 / 255.0, p[0] as f32 / 255.0, p[3] as f32 / 255.0)
    }

    fn close(a: Rgba, b: Rgba) -> bool {
        (a.0 - b.0).abs() < 0.02 && (a.1 - b.1).abs() < 0.02 && (a.2 - b.2).abs() < 0.02
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
                for scene in ["document", "empty", "sheet", "keytips"] {
                    let size = (1100, 760);
                    let shot = render(mode, scene, size, 1.0, 1.0);
                    // Title bar drag space and the bottom-right canvas corner.
                    assert!(close(pixel(&shot, 700, 10), theme.chrome), "{name} {scene} title bar");
                    assert!(close(pixel(&shot, 700, 45), theme.bar) || scene == "sheet", "{name} {scene} toolbar");
                    if scene != "sheet" {
                        assert!(close(pixel(&shot, size.0 - 4, size.1 - 40), theme.canvas), "{name} {scene} canvas");
                    }
                    let path = out.join(format!("{name}-{scene}.png"));
                    let _ = std::fs::remove_file(&path);
                    crate::imaging::export_frame(&shot, &path).unwrap();
                }
            }
            // 150% display scale and 225% text size, the largest Windows allows.
            for (scale, text_scale, file) in [(1.5, 1.0, "light-document-150dpi.png"), (1.0, 2.25, "light-document-text225.png")] {
                let size = ((1100.0 * scale) as u32, (760.0 * scale) as u32);
                let shot = render(Mode::Light, "document", size, scale, text_scale);
                assert!(close(pixel(&shot, size.0 - 4, size.1 - 60), palette(Mode::Light).canvas), "{file}");
                let path = out.join(file);
                let _ = std::fs::remove_file(&path);
                crate::imaging::export_frame(&shot, &path).unwrap();
            }
        }
    }
}
