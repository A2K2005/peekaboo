//! In-window sheets replace the old dialog windows and message boxes.
//! Text fields are Win32 EDIT children, so IME and text services work
//! (CLAUDE.md D11). `ask` runs a nested message loop, like DialogBox, so
//! callers keep a simple call-and-return flow.
use super::{app::with_state, widgets::WidgetId};
use std::cell::RefCell;
use windows::{
    core::*,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::{Com::*, LibraryLoader::GetModuleHandleW},
        UI::{
            Accessibility::{CAccPropServices, IAccPropServices, PROPID_ACC_NAME},
            Controls::EM_SETSEL,
            Input::KeyboardAndMouse::*,
            WindowsAndMessaging::*,
        },
    },
};

pub(super) struct Sheet {
    pub(super) title: String,
    pub(super) message: String,
    pub(super) fields: Vec<Field>,
    /// The first button is the primary action.
    pub(super) buttons: Vec<String>,
    pub(super) cancel: usize,
    /// None while open; Some(None) when cancelled.
    pub(super) result: Option<Option<usize>>,
}

pub(super) struct Field {
    pub(super) label: String,
    pub(super) edit: HWND,
}

thread_local! {
    static FONT: RefCell<Option<(i32, HFONT)>> = const { RefCell::new(None) };
    static BRUSH: RefCell<Option<(u32, HBRUSH)>> = const { RefCell::new(None) };
}

/// A GDI font for EDIT children that matches the Body style at this scale.
pub(super) unsafe fn edit_font(scale: f32, text_scale: f32) -> HFONT {
    let height = -(14.0 * scale * text_scale).round() as i32;
    FONT.with(|cell| {
        let mut cell = cell.borrow_mut();
        if let Some((h, font)) = *cell {
            if h == height {
                return font;
            }
            let _ = DeleteObject(font.into());
        }
        // GDI may not map Segoe UI Variable by name, so EDITs use Segoe UI.
        let font = CreateFontW(
            height,
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            w!("Segoe UI"),
        );
        *cell = Some((height, font));
        font
    })
}

/// WM_CTLCOLOREDIT: themed EDIT text and background.
/// https://learn.microsoft.com/windows/win32/controls/wm-ctlcoloredit
pub(super) unsafe fn color_edit(hdc: HDC, text: COLORREF, field: COLORREF) -> LRESULT {
    SetTextColor(hdc, text);
    SetBkColor(hdc, field);
    let brush = BRUSH.with(|cell| {
        let mut cell = cell.borrow_mut();
        match *cell {
            Some((color, brush)) if color == field.0 => brush,
            old => {
                if let Some((_, brush)) = old {
                    let _ = DeleteObject(brush.into());
                }
                let brush = CreateSolidBrush(field);
                *cell = Some((field.0, brush));
                brush
            }
        }
    });
    LRESULT(brush.0 as isize)
}

/// Names the EDIT for UI Automation, because its visible label is drawn by
/// Direct2D rather than a STATIC control. Dynamic annotation:
/// https://learn.microsoft.com/windows/win32/winauto/dynamic-annotation-api
pub(super) unsafe fn name_field(edit: HWND, name: &str) {
    if let Ok(services) = CoCreateInstance::<_, IAccPropServices>(&CAccPropServices, None, CLSCTX_INPROC_SERVER) {
        let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let _ = services.SetHwndPropStr(edit, OBJID_CLIENT.0 as u32, CHILDID_SELF, PROPID_ACC_NAME, PCWSTR(name.as_ptr()));
    }
}

fn is_open() -> bool {
    with_state(|s| s.sheet.is_some()).unwrap_or(false)
}

/// Shows a sheet and waits. Returns the chosen button and field values, or
/// None when the user cancels (the cancel button, Escape, or closing).
pub(super) unsafe fn ask(
    hwnd: HWND,
    title: &str,
    message: &str,
    fields: &[(&str, String)],
    password: bool,
    buttons: &[&str],
    cancel: usize,
    focus_button: Option<usize>,
) -> Option<(usize, Vec<String>)> {
    if is_open() || buttons.is_empty() {
        return None;
    }
    let (scale, text_scale) = with_state(|s| (s.scale, s.text_scale)).unwrap_or((1.0, 1.0));
    let font = edit_font(scale, text_scale);
    let instance = GetModuleHandleW(None).ok();
    let mut created = Vec::new();
    for (index, (label, value)) in fields.iter().enumerate() {
        let value: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
        let style = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE((ES_AUTOHSCROLL | if password { ES_PASSWORD } else { 0 }) as u32);
        if let Ok(edit) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("EDIT"),
            PCWSTR(value.as_ptr()),
            style,
            0,
            0,
            0,
            0,
            Some(hwnd),
            Some(HMENU((1000 + index) as *mut _)),
            instance.map(Into::into),
            None,
        ) {
            SendMessageW(edit, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
            SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
            name_field(edit, label);
            created.push(Field { label: (*label).into(), edit });
        }
    }
    let first_edit = created.first().map(|f| f.edit);
    let focus = match (first_edit, focus_button) {
        (Some(_), _) => WidgetId::SheetField(0),
        (None, Some(b)) => WidgetId::SheetButton(b),
        (None, None) => WidgetId::SheetButton(0),
    };
    with_state(|s| {
        s.sheet = Some(Sheet {
            title: title.into(),
            message: message.into(),
            fields: created,
            buttons: buttons.iter().map(|b| (*b).into()).collect(),
            cancel,
            result: None,
        });
        s.focus = Some(focus);
        s.focus_visible = first_edit.is_none();
        s.keytips = None;
        s.tooltip = None;
        s.pressed = None;
    });
    position_fields(hwnd);
    let _ = SetFocus(Some(first_edit.unwrap_or(hwnd)));
    let _ = InvalidateRect(Some(hwnd), None, false);
    let mut message = MSG::default();
    loop {
        if with_state(|s| s.sheet.as_ref().is_none_or(|x| x.result.is_some())).unwrap_or(false) {
            break;
        }
        if GetMessageW(&mut message, None, 0, 0).0 <= 0 {
            PostQuitMessage(message.wParam.0 as i32);
            break;
        }
        if message.message == WM_KEYDOWN && is_field(message.hwnd) {
            let key = VIRTUAL_KEY(message.wParam.0 as u16);
            if key == VK_TAB {
                move_focus(hwnd, GetKeyState(VK_SHIFT.0 as i32) < 0);
                continue;
            }
            if key == VK_RETURN {
                finish(Some(0));
                continue;
            }
            if key == VK_ESCAPE {
                finish(None);
                continue;
            }
        }
        let _ = TranslateMessage(&message);
        DispatchMessageW(&message);
    }
    let sheet = with_state(|s| {
        s.focus = None;
        s.focus_visible = false;
        s.sheet.take()
    })
    .flatten()?;
    let values: Vec<String> = sheet
        .fields
        .iter()
        .map(|f| {
            let mut text = vec![0u16; GetWindowTextLengthW(f.edit).max(0) as usize + 1];
            let n = GetWindowTextW(f.edit, &mut text);
            String::from_utf16_lossy(&text[..n.max(0) as usize])
        })
        .collect();
    for field in &sheet.fields {
        let _ = DestroyWindow(field.edit);
    }
    let _ = SetFocus(Some(hwnd));
    let _ = InvalidateRect(Some(hwnd), None, false);
    match sheet.result.flatten() {
        Some(index) if index != sheet.cancel => Some((index, values)),
        _ => None,
    }
}

/// Text fields with OK and Cancel.
pub(super) unsafe fn input(hwnd: HWND, title: &str, fields: &[(&str, String)]) -> Option<Vec<String>> {
    input_action(hwnd, title, fields, "OK")
}

pub(super) unsafe fn input_action(hwnd: HWND, title: &str, fields: &[(&str, String)], action: &str) -> Option<Vec<String>> {
    ask(hwnd, title, "", fields, false, &[action, "Cancel"], 1, None).map(|(_, v)| v)
}

pub(super) unsafe fn password(hwnd: HWND, title: &str, message: &str) -> Option<String> {
    ask(hwnd, title, message, &[("Password", String::new())], true, &["Open", "Cancel"], 1, None)
        .and_then(|(_, mut v)| v.pop())
}

/// Focus starts on Cancel, so Enter never runs a destructive action by
/// accident. https://learn.microsoft.com/windows/apps/design/controls/dialogs-and-flyouts/dialogs
pub(super) unsafe fn confirm(hwnd: HWND, title: &str, message: &str, action: &str) -> bool {
    ask(hwnd, title, message, &[], false, &[action, "Cancel"], 1, Some(1)).is_some()
}

pub(super) unsafe fn alert(hwnd: HWND, title: &str, message: &str) {
    let _ = ask(hwnd, title, message, &[], false, &["OK"], usize::MAX, Some(0));
}

/// Ends the open sheet. `None` cancels.
pub(super) fn finish(button: Option<usize>) {
    with_state(|s| {
        if let Some(sheet) = s.sheet.as_mut() {
            sheet.result = Some(button.filter(|b| *b < sheet.buttons.len()));
        }
    });
}

pub(super) fn is_field(hwnd: HWND) -> bool {
    with_state(|s| s.sheet.as_ref().is_some_and(|x| x.fields.iter().any(|f| f.edit == hwnd))).unwrap_or(false)
}

pub(super) fn field_index(hwnd: HWND) -> Option<usize> {
    with_state(|s| s.sheet.as_ref().and_then(|x| x.fields.iter().position(|f| f.edit == hwnd))).flatten()
}

/// Tab and Shift+Tab inside the sheet, across EDIT children and buttons.
pub(super) unsafe fn move_focus(hwnd: HWND, back: bool) {
    let current = field_index(GetFocus()).map(WidgetId::SheetField);
    let target = with_state(|s| {
        let layout = s.layout();
        let from = current.or(s.focus);
        let next = super::widgets::next_focus(&layout.widgets, from, back)?;
        s.focus = Some(next);
        s.focus_visible = true;
        let edit = match next {
            WidgetId::SheetField(i) => s.sheet.as_ref().and_then(|x| x.fields.get(i)).map(|f| f.edit),
            _ => None,
        };
        Some(edit.unwrap_or(hwnd))
    })
    .flatten();
    if let Some(target) = target {
        let _ = SetFocus(Some(target));
        if target != hwnd {
            SendMessageW(target, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
        }
    }
    let _ = InvalidateRect(Some(hwnd), None, false);
}

/// Moves EDIT children onto their drawn text boxes after layout changes.
pub(super) unsafe fn position_fields(hwnd: HWND) {
    let placements = with_state(|s| {
        let layout = s.layout();
        let (Some(sheet), Some(fields)) = (s.sheet.as_ref(), layout.sheet.as_ref()) else {
            return Vec::new();
        };
        let line = 20.0 * s.text_scale * s.scale;
        let pad = 10.0 * s.scale;
        sheet
            .fields
            .iter()
            .zip(&fields.fields)
            .map(|(f, r)| {
                let y = r.y0 + (r.height() - line) / 2.0;
                (f.edit, (r.x0 + pad) as i32, y as i32, (r.width() - 2.0 * pad) as i32, line.ceil() as i32)
            })
            .collect()
    })
    .unwrap_or_default();
    let (scale, text_scale) = with_state(|s| (s.scale, s.text_scale)).unwrap_or((1.0, 1.0));
    let font = edit_font(scale, text_scale);
    for (edit, x, y, w, h) in placements {
        SendMessageW(edit, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(0)));
        let _ = MoveWindow(edit, x, y, w.max(1), h.max(1), true);
    }
    let _ = InvalidateRect(Some(hwnd), None, false);
}
