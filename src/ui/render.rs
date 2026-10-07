//! Direct2D drawing through an ID2D1HwndRenderTarget, used as an
//! ID2D1DeviceContext.
//!
//! Why an HWND render target and not a flip-model composition swap chain with
//! DirectComposition: a 15-run proof on this PC (artifacts/renderer-choice)
//! measured 84 ms median to the first frame for the HWND target and 96 ms for
//! D3D11 + DXGI + DirectComposition, and the composition path hid the GDI EDIT
//! children that sheets need for IME text input. The HWND target keeps a GDI
//! redirection surface, and with a premultiplied pixel format its alpha shows
//! the DWM backdrop inside the extended frame, which Mica needs.
//! https://learn.microsoft.com/windows/win32/direct2d/supported-pixel-formats-and-alpha-modes
//! https://learn.microsoft.com/windows/win32/api/d2d1_1/nn-d2d1_1-id2d1devicecontext
use super::{theme::Rgba, widgets::Rect};
use std::{cell::RefCell, rc::Rc};
use windows::{
    core::*,
    Win32::{
        Foundation::HWND,
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Dxgi::Common::*,
        },
    },
};

thread_local! {
    static DWRITE: RefCell<Option<IDWriteFactory>> = const { RefCell::new(None) };
    static FAMILIES: RefCell<Option<(&'static str, &'static str)>> = const { RefCell::new(None) };
    static FONTS: RefCell<Option<Rc<Fonts>>> = const { RefCell::new(None) };
}

pub(super) fn dwrite() -> Result<IDWriteFactory> {
    DWRITE.with(|cell| {
        if let Some(factory) = cell.borrow().as_ref() {
            return Ok(factory.clone());
        }
        let factory: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        *cell.borrow_mut() = Some(factory.clone());
        Ok(factory)
    })
}

fn installed(collection: &IDWriteFontCollection, name: &str) -> bool {
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let mut index = 0;
    let mut exists = BOOL::default();
    unsafe { collection.FindFamilyName(PCWSTR(name.as_ptr()), &mut index, &mut exists) }.is_ok() && exists.as_bool()
}

/// Segoe UI Variable and Segoe Fluent Icons ship with Windows 11 only and may
/// not be bundled, so Windows 10 falls back to Segoe UI and Segoe MDL2 Assets.
/// https://learn.microsoft.com/windows/apps/design/style/typography
/// https://learn.microsoft.com/windows/apps/design/style/segoe-fluent-icons-font
pub(super) fn families() -> (&'static str, &'static str) {
    FAMILIES.with(|cell| {
        if let Some(found) = *cell.borrow() {
            return found;
        }
        let mut found = ("Segoe UI", "Segoe MDL2 Assets");
        if let Ok(factory) = dwrite() {
            let mut collection = None;
            if unsafe { factory.GetSystemFontCollection(&mut collection, false) }.is_ok() {
                if let Some(collection) = collection {
                    if installed(&collection, "Segoe UI Variable Text") {
                        found.0 = "Segoe UI Variable Text";
                    }
                    if installed(&collection, "Segoe Fluent Icons") {
                        found.1 = "Segoe Fluent Icons";
                    }
                }
            }
        }
        *cell.borrow_mut() = Some(found);
        found
    })
}

/// Text styles from the Windows type ramp (Caption 12, Body 14, Subtitle 20).
/// https://learn.microsoft.com/windows/apps/design/style/typography
pub(super) struct Fonts {
    key: (f32, f32),
    pub(super) body: IDWriteTextFormat,
    pub(super) strong: IDWriteTextFormat,
    pub(super) caption: IDWriteTextFormat,
    pub(super) title: IDWriteTextFormat,
    pub(super) wrap: IDWriteTextFormat,
    pub(super) icon: IDWriteTextFormat,
    pub(super) caption_icon: IDWriteTextFormat,
}

fn format(family: &str, size: f32, weight: DWRITE_FONT_WEIGHT, single_line: bool) -> Result<IDWriteTextFormat> {
    let factory = dwrite()?;
    let family: Vec<u16> = family.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let format = factory.CreateTextFormat(
            PCWSTR(family.as_ptr()),
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("en-us"),
        )?;
        if single_line {
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            let sign = factory.CreateEllipsisTrimmingSign(&format)?;
            format.SetTrimming(
                &DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 },
                &sign,
            )?;
        }
        Ok(format)
    }
}

/// `scale` is pixels per epx. Text grows with `text_scale`; icons do not, as
/// the text scaling guidance says for font icons.
/// https://learn.microsoft.com/windows/apps/design/input/text-scaling
pub(super) fn fonts(scale: f32, text_scale: f32) -> Result<Rc<Fonts>> {
    FONTS.with(|cell| {
        if let Some(fonts) = cell.borrow().as_ref().filter(|f| f.key == (scale, text_scale)) {
            return Ok(fonts.clone());
        }
        let (text, icons) = families();
        let t = scale * text_scale;
        let fonts = Rc::new(Fonts {
            key: (scale, text_scale),
            body: format(text, 14.0 * t, DWRITE_FONT_WEIGHT_NORMAL, true)?,
            strong: format(text, 14.0 * t, DWRITE_FONT_WEIGHT_SEMI_BOLD, true)?,
            caption: format(text, 12.0 * t, DWRITE_FONT_WEIGHT_NORMAL, true)?,
            title: format(text, 20.0 * t, DWRITE_FONT_WEIGHT_SEMI_BOLD, true)?,
            wrap: format(text, 14.0 * t, DWRITE_FONT_WEIGHT_NORMAL, false)?,
            icon: format(icons, 16.0 * scale, DWRITE_FONT_WEIGHT_NORMAL, true)?,
            caption_icon: format(icons, 10.0 * scale, DWRITE_FONT_WEIGHT_NORMAL, true)?,
        });
        *cell.borrow_mut() = Some(fonts.clone());
        Ok(fonts)
    })
}

/// Width and height of `text` laid out in `max_width`.
pub(super) fn measure(text: &str, format: &IDWriteTextFormat, max_width: f32) -> (f32, f32) {
    let wide: Vec<u16> = text.encode_utf16().collect();
    let result = (|| -> Result<(f32, f32)> {
        let layout = unsafe { dwrite()?.CreateTextLayout(&wide, format, max_width.max(1.0), 100_000.0)? };
        let mut metrics = DWRITE_TEXT_METRICS::default();
        unsafe { layout.GetMetrics(&mut metrics)? };
        Ok((metrics.widthIncludingTrailingWhitespace, metrics.height))
    })();
    result.unwrap_or((0.0, 0.0))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Align {
    Leading,
    Center,
    Trailing,
}

pub(super) fn d2d_rect(r: Rect) -> D2D_RECT_F {
    D2D_RECT_F { left: r.x0, top: r.y0, right: r.x1, bottom: r.y1 }
}

/// Drawing helpers shared by the main window and popup menus.
pub(super) struct Painter {
    pub(super) target: ID2D1RenderTarget,
    brush: ID2D1SolidColorBrush,
}

impl Painter {
    pub(super) fn new(target: ID2D1RenderTarget) -> Result<Self> {
        let brush = unsafe { target.CreateSolidColorBrush(&Rgba(0.0, 0.0, 0.0, 1.0).d2d(), None)? };
        Ok(Self { target, brush })
    }
    fn brush(&self, color: Rgba) -> &ID2D1SolidColorBrush {
        unsafe { self.brush.SetColor(&color.d2d()) };
        &self.brush
    }
    pub(super) fn clear(&self, color: Rgba) {
        unsafe { self.target.Clear(Some(&color.d2d())) };
    }
    pub(super) fn fill(&self, r: Rect, color: Rgba) {
        unsafe { self.target.FillRectangle(&d2d_rect(r), self.brush(color)) };
    }
    pub(super) fn fill_round(&self, r: Rect, radius: f32, color: Rgba) {
        let rounded = D2D1_ROUNDED_RECT { rect: d2d_rect(r), radiusX: radius, radiusY: radius };
        unsafe { self.target.FillRoundedRectangle(&rounded, self.brush(color)) };
    }
    pub(super) fn stroke_round(&self, r: Rect, radius: f32, color: Rgba, width: f32) {
        // Strokes center on the edge; inset by half the width to stay inside.
        let r = r.inset(width / 2.0);
        let rounded = D2D1_ROUNDED_RECT { rect: d2d_rect(r), radiusX: radius, radiusY: radius };
        unsafe { self.target.DrawRoundedRectangle(&rounded, self.brush(color), width, None) };
    }
    pub(super) fn line(&self, from: (f32, f32), to: (f32, f32), color: Rgba, width: f32) {
        unsafe {
            self.target.DrawLine(
                windows_numerics::Vector2 { X: from.0, Y: from.1 },
                windows_numerics::Vector2 { X: to.0, Y: to.1 },
                self.brush(color),
                width,
                None,
            )
        };
    }
    pub(super) fn text(&self, text: &str, r: Rect, format: &IDWriteTextFormat, color: Rgba, align: Align) {
        if text.is_empty() || r.width() <= 0.0 {
            return;
        }
        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            let _ = format.SetTextAlignment(match align {
                Align::Leading => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::Trailing => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
            self.target.DrawText(
                &wide,
                format,
                &d2d_rect(r),
                self.brush(color),
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }
    pub(super) fn glyph(&self, glyph: u16, r: Rect, format: &IDWriteTextFormat, color: Rgba) {
        if let Some(c) = char::from_u32(glyph as u32) {
            self.text(&c.to_string(), r, format, color, Align::Center);
        }
    }
    /// Text with one character underlined, for access keys in menus.
    pub(super) fn text_underlined(&self, text: &str, underline: Option<usize>, r: Rect, format: &IDWriteTextFormat, color: Rgba) {
        let wide: Vec<u16> = text.encode_utf16().collect();
        let draw = || -> Result<()> {
            unsafe {
                format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING)?;
                let layout = dwrite()?.CreateTextLayout(&wide, format, r.width().max(1.0), r.height().max(1.0))?;
                if let Some(position) = underline {
                    layout.SetUnderline(true, DWRITE_TEXT_RANGE { startPosition: position as u32, length: 1 })?;
                }
                self.target.DrawTextLayout(
                    windows_numerics::Vector2 { X: r.x0, Y: r.y0 },
                    &layout,
                    self.brush(color),
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
            }
            Ok(())
        };
        let _ = draw();
    }
    /// Uploads a frame as a bitmap for this target.
    pub(super) fn upload(&self, frame: &crate::model::Frame) -> Result<ID2D1Bitmap> {
        unsafe {
            self.target.CreateBitmap(
                D2D_SIZE_U { width: frame.width, height: frame.height },
                Some(frame.pixels.as_ptr().cast()),
                frame.width * 4,
                &D2D1_BITMAP_PROPERTIES {
                    pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                    dpiX: 96.0,
                    dpiY: 96.0,
                },
            )
        }
    }
    pub(super) fn draw_bitmap(&self, bitmap: &ID2D1Bitmap, r: Rect) {
        unsafe { self.target.DrawBitmap(bitmap, Some(&d2d_rect(r)), 1.0, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None) };
    }
    pub(super) fn push_clip(&self, r: Rect) {
        unsafe { self.target.PushAxisAlignedClip(&d2d_rect(r), D2D1_ANTIALIAS_MODE_ALIASED) };
    }
    pub(super) fn pop_clip(&self) {
        unsafe { self.target.PopAxisAlignedClip() };
    }
}

pub(super) struct Renderer {
    target: ID2D1HwndRenderTarget,
    pub(super) painter: Painter,
    /// The current document frame, uploaded once per frame.
    pub(super) bitmap: Option<ID2D1Bitmap>,
}

impl Renderer {
    pub(super) unsafe fn new(hwnd: HWND, width: u32, height: u32) -> Result<Self> {
        let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let target = factory.CreateHwndRenderTarget(
            &D2D1_RENDER_TARGET_PROPERTIES {
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                // Layout works in physical pixels, so the target uses 96 DPI.
                dpiX: 96.0,
                dpiY: 96.0,
                ..Default::default()
            },
            &D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd,
                pixelSize: D2D_SIZE_U { width: width.max(1), height: height.max(1) },
                ..Default::default()
            },
        )?;
        let context: ID2D1DeviceContext = target.cast()?;
        // ClearType needs an opaque target; transparent chrome needs grayscale.
        context.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        let painter = Painter::new(context.cast()?)?;
        Ok(Self { target, painter, bitmap: None })
    }
    pub(super) fn resize(&self, width: u32, height: u32) -> Result<()> {
        unsafe { self.target.Resize(&D2D_SIZE_U { width: width.max(1), height: height.max(1) }) }
    }
    pub(super) fn begin(&self) {
        unsafe { self.target.BeginDraw() };
    }
    pub(super) fn end(&self) -> Result<()> {
        unsafe { self.target.EndDraw(None, None) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every icon the UI draws exists in the installed symbol font. Runs
    /// headless through DirectWrite.
    #[test]
    fn icons_exist_in_the_installed_symbol_font() {
        let factory = dwrite().unwrap();
        let mut collection = None;
        unsafe { factory.GetSystemFontCollection(&mut collection, false).unwrap() };
        let collection = collection.unwrap();
        let name: Vec<u16> = families().1.encode_utf16().chain(Some(0)).collect();
        let (mut index, mut exists) = (0, BOOL::default());
        unsafe { collection.FindFamilyName(PCWSTR(name.as_ptr()), &mut index, &mut exists).unwrap() };
        assert!(exists.as_bool(), "{} is missing", families().1);
        let family = unsafe { collection.GetFontFamily(index).unwrap() };
        let font = unsafe {
            family
                .GetFirstMatchingFont(DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL)
                .unwrap()
        };
        for glyph in super::super::commands::glyph::ALL {
            let has = unsafe { font.HasCharacter(*glyph as u32).unwrap() };
            assert!(has.as_bool(), "{glyph:04X} missing from {}", families().1);
        }
    }

    #[test]
    fn text_measurement_grows_with_text_scale() {
        let small = fonts(1.0, 1.0).unwrap();
        let (w1, h1) = measure("Open a PDF or image", &small.body, 1000.0);
        let big = fonts(1.0, 2.0).unwrap();
        let (w2, h2) = measure("Open a PDF or image", &big.body, 1000.0);
        assert!(w1 > 50.0 && w2 > w1 * 1.8 && h2 > h1 * 1.8);
        let (_, wrapped) = measure("word ".repeat(40).trim(), &small.wrap, 100.0);
        assert!(wrapped > h1 * 3.0);
    }
}
