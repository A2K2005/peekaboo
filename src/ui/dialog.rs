use super::*;

pub(super) struct InputDialog {
    pub(super) edits: Vec<HWND>,
    pub(super) answer: Option<Vec<String>>,
    pub(super) finished: bool,
}
thread_local! { pub(super) static INPUT: RefCell<Option<InputDialog>> = const { RefCell::new(None) }; }
unsafe extern "system" fn input_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_COMMAND if wparam.0 & 0xffff == 1 => {
            INPUT.with(|cell| {
                if let Ok(mut value) = cell.try_borrow_mut() {
                    if let Some(dialog) = value.as_mut() {
                        dialog.answer = Some(
                            dialog
                                .edits
                                .iter()
                                .map(|handle| {
                                    let mut text =
                                        vec![
                                            0u16;
                                            GetWindowTextLengthW(*handle).max(0) as usize + 1
                                        ];
                                    let n = GetWindowTextW(*handle, &mut text);
                                    String::from_utf16_lossy(&text[..n.max(0) as usize])
                                })
                                .collect(),
                        );
                        dialog.finished = true;
                    }
                }
            });
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_COMMAND if wparam.0 & 0xffff == 2 => {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            LRESULT(0)
        }
        WM_CLOSE => {
            INPUT.with(|cell| {
                if let Ok(mut value) = cell.try_borrow_mut() {
                    if let Some(dialog) = value.as_mut() {
                        dialog.finished = true;
                    }
                }
            });
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

pub(super) unsafe fn input(hwnd: HWND, title: &str, fields: &[(&str, String)]) -> Option<Vec<String>> {
    input_with_password(hwnd, title, fields, false)
}
pub(super) unsafe fn input_with_password(
    hwnd: HWND,
    title: &str,
    fields: &[(&str, String)],
    password: bool,
) -> Option<Vec<String>> {
    let instance = GetModuleHandleW(None).ok()?;
    let class = w!("PreviewInputDialog");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(input_proc),
        hInstance: instance.into(),
        lpszClassName: class,
        hCursor: LoadCursorW(None, IDC_ARROW).ok()?,
        hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as *mut _),
        ..Default::default()
    };
    RegisterClassW(&wc);
    let title = wide(title);
    let height = 135 + fields.len() as i32 * 62;
    let dialog = CreateWindowExW(
        WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
        class,
        PCWSTR(title.as_ptr()),
        WS_CAPTION | WS_SYSMENU | WS_POPUP,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        430,
        height,
        Some(hwnd),
        None,
        Some(instance.into()),
        None,
    )
    .ok()?;
    let mut edits = Vec::new();
    for (index, (label, value)) in fields.iter().enumerate() {
        let label = wide(label);
        let value = wide(value);
        let top = 16 + index as i32 * 62;
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            PCWSTR(label.as_ptr()),
            WS_CHILD | WS_VISIBLE,
            18,
            top,
            380,
            22,
            Some(dialog),
            None,
            Some(instance.into()),
            None,
        );
        if let Ok(edit) = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            w!("EDIT"),
            PCWSTR(value.as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(if password { 0xa0 } else { 0x80 }),
            18,
            top + 24,
            380,
            26,
            Some(dialog),
            Some(HMENU((20 + index) as *mut _)),
            Some(instance.into()),
            None,
        ) {
            edits.push(edit);
        }
    }
    let _ = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("BUTTON"),
        w!("OK"),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(1),
        216,
        height - 78,
        86,
        30,
        Some(dialog),
        Some(HMENU(1 as *mut _)),
        Some(instance.into()),
        None,
    );
    let _ = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("BUTTON"),
        w!("Cancel"),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        312,
        height - 78,
        86,
        30,
        Some(dialog),
        Some(HMENU(2 as *mut _)),
        Some(instance.into()),
        None,
    );
    let first = edits.first().copied();
    INPUT.with(|cell| {
        *cell.borrow_mut() = Some(InputDialog {
            edits,
            answer: None,
            finished: false,
        })
    });
    let _ = EnableWindow(hwnd, false);
    let _ = ShowWindow(dialog, SW_SHOW);
    if let Some(first) = first {
        let _ = SetFocus(Some(first));
    }
    let mut message = MSG::default();
    while !INPUT.with(|cell| cell.borrow().as_ref().is_none_or(|d| d.finished)) {
        if GetMessageW(&mut message, None, 0, 0).0 <= 0 {
            break;
        }
        if !IsDialogMessageW(dialog, &message).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    let _ = EnableWindow(hwnd, true);
    let _ = SetForegroundWindow(hwnd);
    INPUT.with(|cell| cell.borrow_mut().take().and_then(|d| d.answer))
}
