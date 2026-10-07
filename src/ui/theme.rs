//! Light, dark, and high-contrast colors, system theme detection, and the
//! DWM window attributes that follow the theme.
use windows::{
    core::w,
    Win32::{
        Foundation::{COLORREF, HWND},
        Graphics::{
            Direct2D::Common::D2D1_COLOR_F,
            Dwm::*,
            Gdi::{GetSysColor, SYS_COLOR_INDEX, COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT},
        },
        System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD},
        UI::{
            Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
            Controls::MARGINS,
            WindowsAndMessaging::{SystemParametersInfoW, SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS},
        },
    },
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Rgba(pub f32, pub f32, pub f32, pub f32);

impl Rgba {
    pub(super) const fn hex(rgb: u32) -> Self {
        Self(
            ((rgb >> 16) & 255) as f32 / 255.0,
            ((rgb >> 8) & 255) as f32 / 255.0,
            (rgb & 255) as f32 / 255.0,
            1.0,
        )
    }
    pub(super) const fn alpha(self, a: f32) -> Self {
        Self(self.0, self.1, self.2, a)
    }
    fn from_colorref(c: u32) -> Self {
        Self::hex(((c & 255) << 16) | (c & 0xff00) | ((c >> 16) & 255))
    }
    pub(super) fn colorref(self) -> COLORREF {
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
        COLORREF(c(self.0) | (c(self.1) << 8) | (c(self.2) << 16))
    }
    pub(super) fn d2d(self) -> D2D1_COLOR_F {
        D2D1_COLOR_F { r: self.0, g: self.1, b: self.2, a: self.3 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Light,
    Dark,
    Contrast,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Theme {
    pub(super) mode: Mode,
    /// Mica shows behind the chrome, so the chrome is painted transparent.
    pub(super) mica: bool,
    pub(super) chrome: Rgba,
    pub(super) surface: Rgba,
    /// Behind the document. Always opaque.
    pub(super) canvas: Rgba,
    pub(super) text: Rgba,
    pub(super) text_secondary: Rgba,
    pub(super) text_disabled: Rgba,
    pub(super) hover: Rgba,
    pub(super) hover_text: Rgba,
    pub(super) pressed: Rgba,
    pub(super) selected: Rgba,
    pub(super) selected_text: Rgba,
    pub(super) accent: Rgba,
    pub(super) on_accent: Rgba,
    pub(super) border: Rgba,
    pub(super) field: Rgba,
    pub(super) scrim: Rgba,
    pub(super) focus_outer: Rgba,
    pub(super) focus_inner: Rgba,
    pub(super) close_hover: Rgba,
    pub(super) border_width: f32,
}

/// Values follow the Fluent 2 Windows color tokens (SolidBackgroundFillColorBase,
/// TextFillColorPrimary, SubtleFillColorSecondary, AccentFillColorDefault).
/// https://learn.microsoft.com/windows/apps/design/style/color
pub(super) fn palette(mode: Mode) -> Theme {
    match mode {
        Mode::Light => Theme {
            mode,
            mica: false,
            chrome: Rgba::hex(0xf3f3f3),
            surface: Rgba::hex(0xf9f9f9),
            canvas: Rgba::hex(0xe4e4e4),
            text: Rgba::hex(0x1b1b1b),
            text_secondary: Rgba::hex(0x5f5f5f),
            text_disabled: Rgba::hex(0xa0a0a0),
            hover: Rgba(0.0, 0.0, 0.0, 0.06),
            hover_text: Rgba::hex(0x1b1b1b),
            pressed: Rgba(0.0, 0.0, 0.0, 0.035),
            selected: Rgba::hex(0xffffff),
            selected_text: Rgba::hex(0x1b1b1b),
            accent: Rgba::hex(0x005fb8),
            on_accent: Rgba::hex(0xffffff),
            border: Rgba::hex(0xd9d9d9),
            field: Rgba::hex(0xffffff),
            scrim: Rgba(0.0, 0.0, 0.0, 0.3),
            focus_outer: Rgba::hex(0x000000),
            focus_inner: Rgba::hex(0xffffff),
            close_hover: Rgba::hex(0xc42b1c),
            border_width: 1.0,
        },
        Mode::Dark => Theme {
            mode,
            mica: false,
            chrome: Rgba::hex(0x202020),
            surface: Rgba::hex(0x2c2c2c),
            canvas: Rgba::hex(0x181818),
            text: Rgba::hex(0xffffff),
            text_secondary: Rgba::hex(0xc8c8c8),
            text_disabled: Rgba::hex(0x6e6e6e),
            hover: Rgba(1.0, 1.0, 1.0, 0.07),
            hover_text: Rgba::hex(0xffffff),
            pressed: Rgba(1.0, 1.0, 1.0, 0.045),
            selected: Rgba::hex(0x2e2e2e),
            selected_text: Rgba::hex(0xffffff),
            accent: Rgba::hex(0x60cdff),
            on_accent: Rgba::hex(0x000000),
            border: Rgba::hex(0x3d3d3d),
            field: Rgba::hex(0x2d2d2d),
            scrim: Rgba(0.0, 0.0, 0.0, 0.5),
            focus_outer: Rgba::hex(0xffffff),
            focus_inner: Rgba::hex(0x000000),
            close_hover: Rgba::hex(0xc42b1c),
            border_width: 1.0,
        },
        // Windows "High Contrast Black". Only used when PFW_THEME=contrast forces
        // contrast while the system contrast theme is off.
        Mode::Contrast => contrast([0x000000, 0xffffff, 0x1aebff, 0x000000, 0x3ff23f].map(Rgba::hex)),
    }
}

/// Contrast themes must use the system colors.
/// https://learn.microsoft.com/windows/apps/design/accessibility/high-contrast-themes
fn contrast([window, text, highlight, highlight_text, gray]: [Rgba; 5]) -> Theme {
    Theme {
        mode: Mode::Contrast,
        mica: false,
        chrome: window,
        surface: window,
        canvas: window,
        text,
        text_secondary: text,
        text_disabled: gray,
        hover: highlight,
        hover_text: highlight_text,
        pressed: highlight,
        selected: highlight,
        selected_text: highlight_text,
        accent: highlight,
        on_accent: highlight_text,
        border: text,
        field: window,
        scrim: window.alpha(0.0),
        focus_outer: text,
        focus_inner: window,
        close_hover: highlight,
        border_width: 2.0,
    }
}

/// PFW_THEME is a test hook. It lets a run check each theme without
/// changing system settings.
pub(super) fn select_mode(forced: Option<&str>, high_contrast: bool, apps_light: bool) -> Mode {
    match forced {
        Some("light") => Mode::Light,
        Some("dark") => Mode::Dark,
        Some("contrast") => Mode::Contrast,
        _ if high_contrast => Mode::Contrast,
        _ if apps_light => Mode::Light,
        _ => Mode::Dark,
    }
}

fn high_contrast_on() -> bool {
    let mut value = HIGHCONTRASTW {
        cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32,
        ..Default::default()
    };
    unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            value.cbSize,
            Some(&mut value as *mut _ as _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok()
            && value.dwFlags.contains(HCF_HIGHCONTRASTON)
    }
}

fn read_dword(key: windows::core::PCWSTR, name: windows::core::PCWSTR) -> Option<u32> {
    let mut value = 0u32;
    let mut size = 4u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            name,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as _),
            Some(&mut size),
        )
    }
    .is_ok()
    .then_some(value)
}

/// Reads the system theme. The registry value is the one Windows Settings
/// writes for "Choose your app mode"; it costs about 0.1 ms, where creating
/// UISettings costs 12 to 23 ms on this PC (artifacts/startup-probes).
pub(super) fn current() -> Theme {
    let apps_light = read_dword(
        w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
        w!("AppsUseLightTheme"),
    )
    .is_none_or(|v| v != 0);
    let forced = std::env::var("PFW_THEME").ok();
    let system_contrast = high_contrast_on();
    let mode = select_mode(forced.as_deref(), system_contrast, apps_light);
    if mode == Mode::Contrast && system_contrast {
        let sys = |i: SYS_COLOR_INDEX| unsafe { Rgba::from_colorref(GetSysColor(i)) };
        return contrast([
            sys(COLOR_WINDOW),
            sys(COLOR_WINDOWTEXT),
            sys(COLOR_HIGHLIGHT),
            sys(COLOR_HIGHLIGHTTEXT),
            sys(COLOR_GRAYTEXT),
        ]);
    }
    palette(mode)
}

/// Startup value of the Windows text size setting (100 to 225 percent).
/// UISettings.TextScaleFactor is authoritative and replaces this after the
/// first frame; this registry value avoids its startup cost.
pub(super) fn text_scale_from_registry() -> f32 {
    read_dword(w!(r"Software\Microsoft\Accessibility"), w!("TextScaleFactor"))
        .map_or(1.0, |v| (v as f32 / 100.0).clamp(1.0, 2.25))
}

/// Applies the dark title bar and the backdrop, and returns the theme with
/// `mica` set when the system accepted Mica.
/// - DWMWA_USE_IMMERSIVE_DARK_MODE and DWMWA_SYSTEMBACKDROP_TYPE:
///   https://learn.microsoft.com/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute
/// - Mica Alt (DWMSBT_TABBEDWINDOW) suits a tabbed title bar:
///   https://learn.microsoft.com/windows/apps/design/style/mica
/// - Windows 10 and Windows 11 before 22H2 reject the backdrop attribute, so
///   the chrome stays a solid color there.
pub(super) fn apply(hwnd: HWND, mut theme: Theme) -> Theme {
    unsafe {
        let dark = windows::core::BOOL::from(theme.mode == Mode::Dark);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const _ as _,
            std::mem::size_of_val(&dark) as u32,
        );
        let backdrop = if theme.mode == Mode::Contrast { DWMSBT_NONE } else { DWMSBT_TABBEDWINDOW };
        let accepted = DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &backdrop as *const _ as _,
            std::mem::size_of_val(&backdrop) as u32,
        )
        .is_ok();
        theme.mica = accepted && theme.mode != Mode::Contrast;
        if theme.mica {
            theme.chrome = theme.chrome.alpha(0.0);
        }
        // The backdrop shows only through client pixels inside the extended frame.
        // https://learn.microsoft.com/windows/win32/dwm/customframe
        let inset = if theme.mica { -1 } else { 0 };
        let _ = DwmExtendFrameIntoClientArea(
            hwnd,
            &MARGINS { cxLeftWidth: inset, cxRightWidth: inset, cyTopHeight: inset, cyBottomHeight: inset },
        );
    }
    theme
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_selection_follows_override_then_contrast_then_app_mode() {
        assert_eq!(select_mode(Some("light"), true, false), Mode::Light);
        assert_eq!(select_mode(Some("dark"), false, true), Mode::Dark);
        assert_eq!(select_mode(Some("contrast"), false, true), Mode::Contrast);
        assert_eq!(select_mode(Some("bogus"), true, true), Mode::Contrast);
        assert_eq!(select_mode(None, true, true), Mode::Contrast);
        assert_eq!(select_mode(None, false, true), Mode::Light);
        assert_eq!(select_mode(None, false, false), Mode::Dark);
    }

    #[test]
    fn document_canvas_is_opaque_and_text_contrasts_in_every_theme() {
        for mode in [Mode::Light, Mode::Dark, Mode::Contrast] {
            let t = palette(mode);
            assert_eq!(t.canvas.3, 1.0);
            assert_eq!(t.surface.3, 1.0);
            let luma = |c: Rgba| 0.2126 * c.0 + 0.7152 * c.1 + 0.0722 * c.2;
            assert!((luma(t.text) - luma(t.surface)).abs() > 0.6, "{mode:?}");
            assert!((luma(t.on_accent) - luma(t.accent)).abs() > 0.4, "{mode:?}");
        }
        assert_eq!(palette(Mode::Contrast).border_width, 2.0);
    }

    #[test]
    fn colorref_round_trips_byte_order() {
        let c = Rgba::hex(0x123456);
        assert_eq!(c.colorref().0, 0x563412);
        assert_eq!(Rgba::from_colorref(0x563412), c);
    }
}
