use super::*;

pub(super) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

pub fn run() -> Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let (send, requests) = mpsc::channel();
        let (results, receive) = mpsc::channel();
        std::thread::Builder::new()
            .name("document-render".into())
            .spawn(move || worker(requests, results))
            .map_err(|_| Error::from_thread())?;
        let mut paths = Vec::new();
        for path in std::env::args_os()
            .skip(1)
            .map(PathBuf::from)
            .map(|p| std::fs::canonicalize(&p).unwrap_or(p))
        {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
        let path = paths.first().cloned();
        STATE.with(|s| {
            *s.borrow_mut() = Some(State {
                sender: send,
                receiver: receive,
                path,
                page: 0,
                generation: 0,
                frame: None,
                graphics: None,
                status: "Open a PDF or image · Ctrl+O".into(),
                pending: false,
                painted: false,
                due: Some(Instant::now()),
                marked: false,
                sessions: HashMap::new(),
                zoom: 1.0,
                pan: (0.0, 0.0),
                drag: None,
                crop: false,
                selection: None,
                image_rect: D2D_RECT_F::default(),
                exporting: false,
                displayed: None,
                render_failed: false,
                markup: None,
                ink: Vec::new(),
                markup_text: String::new(),
                signature: None,
                cancel: None,
                slideshow: None,
                tabs: Vec::new(),
                password_attempts: HashMap::new(),
                views: HashMap::new(),
            })
        });
        let instance = GetModuleHandleW(None)?;
        let class = w!("PreviewForWindowsSpeedSpike");
        let wc = WNDCLASSW {
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: instance.into(),
            lpszClassName: class,
            lpfnWndProc: Some(wndproc),
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 {
            return Err(Error::from_thread());
        }
        let menu = CreateMenu()?;
        let file = CreatePopupMenu()?;
        let view = CreatePopupMenu()?;
        let edit = CreatePopupMenu()?;
        let markup = CreatePopupMenu()?;
        AppendMenuW(file, MF_STRING, OPEN, w!("&Open...\tCtrl+O"))?;
        for (id, title) in [
            (SAVE, "Save a &copy...\tCtrl+Shift+S"),
            (EXTRACT, "&Extract current PDF page..."),
            (MERGE, "&Combine with another PDF..."),
        ] {
            let text = wide(title);
            AppendMenuW(file, MF_STRING, id, PCWSTR(text.as_ptr()))?;
        }
        AppendMenuW(file, MF_STRING, PRINT, w!("&Print...\tCtrl+P"))?;
        AppendMenuW(file, MF_STRING, BATCH, w!("Batch convert image folder..."))?;
        AppendMenuW(
            file,
            MF_STRING,
            BATCH_SELECTED,
            w!("Batch convert selected images..."),
        )?;
        AppendMenuW(file, MF_STRING, INFO, w!("File information"))?;
        AppendMenuW(file, MF_SEPARATOR, 0, PCWSTR::null())?;
        AppendMenuW(file, MF_STRING, EXIT, w!("E&xit"))?;
        AppendMenuW(view, MF_STRING, PREVIOUS, w!("&Previous\tLeft / Page Up"))?;
        AppendMenuW(view, MF_STRING, NEXT, w!("&Next\tRight / Page Down"))?;
        AppendMenuW(view, MF_STRING, SLIDESHOW, w!("Start / stop slideshow\tF5"))?;
        for (id, title) in [
            (FIT, "&Fit to window\tCtrl+0"),
            (ZOOM_IN, "Zoom &in\tCtrl++"),
            (ZOOM_OUT, "Zoom &out\tCtrl+-"),
            (FIND, "&Find in PDF...\tCtrl+F"),
        ] {
            let text = wide(title);
            AppendMenuW(view, MF_STRING, id, PCWSTR(text.as_ptr()))?;
        }
        for (id, title) in [
            (UNDO, "&Undo\tCtrl+Z"),
            (REVERT, "&Revert to opened"),
            (ROTATE, "&Rotate right\tCtrl+R"),
            (FLIP, "&Flip image horizontally"),
            (CROP, "&Crop image or PDF page"),
            (RESIZE, "Re&size image..."),
            (DELETE, "&Delete PDF page..."),
            (TEXT, "Copy &text\tCtrl+C"),
            (BACKGROUND, "Remove image &background..."),
        ] {
            let text = wide(title);
            AppendMenuW(edit, MF_STRING, id, PCWSTR(text.as_ptr()))?;
        }
        AppendMenuW(edit, MF_STRING, MOVE, w!("Move PDF page..."))?;
        AppendMenuW(
            edit,
            MF_STRING,
            INSERT,
            w!("Insert blank PDF page after current"),
        )?;
        AppendMenuW(menu, MF_POPUP, file.0 as usize, w!("&File"))?;
        AppendMenuW(menu, MF_POPUP, edit.0 as usize, w!("&Edit"))?;
        for (index, title) in [
            "&Ink / draw signature",
            "&Highlight rectangle",
            "&Underline",
            "&Strike through",
            "&Note...",
            "&Rectangle",
            "&Ellipse",
            "&Arrow",
            "&Text box...",
        ]
        .iter()
        .enumerate()
        {
            let label = wide(title);
            AppendMenuW(markup, MF_STRING, 200 + index, PCWSTR(label.as_ptr()))?;
        }
        AppendMenuW(markup, MF_SEPARATOR, 0, PCWSTR::null())?;
        AppendMenuW(
            markup,
            MF_STRING,
            SIGN_SAVE,
            w!("Save last ink as signature"),
        )?;
        AppendMenuW(markup, MF_STRING, SIGN_PLACE, w!("Place saved signature"))?;
        AppendMenuW(markup, MF_STRING, FORM, w!("Fill a form field..."))?;
        AppendMenuW(menu, MF_POPUP, markup.0 as usize, w!("&Markup"))?;
        AppendMenuW(menu, MF_POPUP, view.0 as usize, w!("&View"))?;
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            w!("Preview for Windows"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1100,
            800,
            None,
            Some(menu),
            Some(instance.into()),
            None,
        )?;
        let _ = InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_TAB_CLASSES,
        });
        let tabs = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("SysTabControl32"),
            w!("Open documents"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            8,
            44,
            1060,
            30,
            Some(hwnd),
            Some(HMENU(TABS as *mut _)),
            Some(instance.into()),
            None,
        )?;
        SendMessageW(
            tabs,
            WM_SETFONT,
            Some(WPARAM(GetStockObject(DEFAULT_GUI_FONT).0 as usize)),
            Some(LPARAM(1)),
        );
        add_tabs(hwnd, &paths);
        for (index, (id, label)) in [
            (OPEN, "Open"),
            (PREVIOUS, "Previous"),
            (NEXT, "Next"),
            (FIT, "Fit"),
            (ZOOM_IN, "Zoom +"),
            (ZOOM_OUT, "Zoom -"),
            (ROTATE, "Rotate"),
            (CROP, "Crop"),
            (SAVE, "Save copy"),
        ]
        .iter()
        .enumerate()
        {
            let label = wide(label);
            let button = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                PCWSTR(label.as_ptr()),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                8 + index as i32 * 94,
                6,
                88,
                30,
                Some(hwnd),
                Some(HMENU(*id as *mut _)),
                Some(instance.into()),
                None,
            )?;
            SendMessageW(
                button,
                WM_SETFONT,
                Some(WPARAM(GetStockObject(DEFAULT_GUI_FONT).0 as usize)),
                Some(LPARAM(1)),
            );
        }
        DragAcceptFiles(hwnd, true);
        SetTimer(Some(hwnd), 1, 10, None);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);
        let mut message = MSG::default();
        loop {
            let result = GetMessageW(&mut message, None, 0, 0).0;
            if result == -1 {
                return Err(Error::from_thread());
            }
            if result == 0 {
                break;
            }
            if message.message == WM_KEYDOWN
                && (GetKeyState(VK_CONTROL.0 as i32) < 0
                    || message.wParam.0 == 0x1b
                    || message.wParam.0 == 0x74)
            {
                SendMessageW(hwnd, WM_KEYDOWN, Some(message.wParam), Some(message.lParam));
                continue;
            }
            if !IsDialogMessageW(hwnd, &message).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        STATE.with(|s| *s.borrow_mut() = None);
        CoUninitialize();
        Ok(())
    }
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_COMMAND => {
            command(hwnd, wparam.0 & 0xffff);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let control = GetKeyState(VK_CONTROL.0 as i32) < 0;
            let id = match wparam.0 as u16 {
                0x4f if control => OPEN,
                0x53 if control => SAVE,
                0x5a if control => UNDO,
                0x52 if control => ROTATE,
                0x46 if control => FIND,
                0x43 if control => TEXT,
                0x50 if control => PRINT,
                0x74 => SLIDESHOW,
                0x30 if control => FIT,
                0xbb | 0x6b if control => ZOOM_IN,
                0xbd | 0x6d if control => ZOOM_OUT,
                0x25 | 0x21 => PREVIOUS,
                0x27 | 0x22 => NEXT,
                _ => 0,
            };
            if id != 0 {
                command(hwnd, id);
            }
            if wparam.0 == 0x1b {
                STATE.with(|cell| {
                    if let Some(state) = cell.borrow_mut().as_mut() {
                        state.crop = false;
                        state.slideshow = None;
                        if let Some(cancel) = &state.cancel {
                            cancel.store(true, Ordering::Release);
                        }
                        state.markup = None;
                        state.ink.clear();
                        state.signature = None;
                        state.selection = None;
                        state.drag = None;
                    }
                });
                let _ = ReleaseCapture();
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            LRESULT(0)
        }
        WM_DROPFILES => {
            let drop = HDROP(wparam.0 as *mut _);
            let count = DragQueryFileW(drop, u32::MAX, None);
            let mut paths = Vec::new();
            for index in 0..count {
                let length = DragQueryFileW(drop, index, None);
                let mut path = vec![0u16; length as usize + 1];
                DragQueryFileW(drop, index, Some(&mut path));
                if length > 0 {
                    paths.push(PathBuf::from(String::from_utf16_lossy(
                        &path[..length as usize],
                    )));
                }
            }
            DragFinish(drop);
            add_tabs(hwnd, &paths);
            if let Some(path) = paths.into_iter().next() {
                open(hwnd, path);
            }
            LRESULT(0)
        }
        WM_TIMER => {
            tick(hwnd);
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            BeginPaint(hwnd, &mut ps);
            paint(hwnd);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_SIZE => {
            if let Ok(tabs) = GetDlgItem(Some(hwnd), TABS as i32) {
                let _ = MoveWindow(
                    tabs,
                    8,
                    44,
                    ((lparam.0 as u16) as i32 - 16).max(1),
                    30,
                    true,
                );
            }
            STATE.with(|cell| {
                if let Ok(mut value) = cell.try_borrow_mut() {
                    if let Some(state) = value.as_mut() {
                        state.graphics = None;
                        state.due = Some(
                            Instant::now()
                                + if state.frame.is_some() {
                                    Duration::from_millis(120)
                                } else {
                                    Duration::ZERO
                                },
                        );
                    }
                }
            });
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_NOTIFY => {
            let header = &*(lparam.0 as *const NMHDR);
            if header.idFrom == TABS && header.code == TCN_SELCHANGE {
                let selected = SendMessageW(header.hwndFrom, TCM_GETCURSEL, None, None).0;
                let path = STATE.with(|cell| {
                    cell.try_borrow().ok().and_then(|v| {
                        v.as_ref()
                            .and_then(|s| s.tabs.get(selected as usize).cloned())
                    })
                });
                if let Some(path) = path {
                    open(hwnd, path);
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let point = point(lparam);
            STATE.with(|cell| {
                if let Some(state) = cell.borrow_mut().as_mut() {
                    if !state.pending
                        && !state.render_failed
                        && state.frame.is_some()
                        && point.1 >= 80.0
                    {
                        state.drag = Some(point);
                        state.ink = vec![[point.0, point.1]];
                        if state.crop || state.markup.is_some() {
                            state.selection = Some((point.0, point.1, point.0, point.1));
                        }
                        SetCapture(hwnd);
                    }
                }
            });
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let point = point(lparam);
            STATE.with(|cell| {
                if let Some(state) = cell.borrow_mut().as_mut() {
                    if let Some(start) = state.drag {
                        if state.crop || state.markup.is_some() {
                            state.selection = Some((start.0, start.1, point.0, point.1));
                            if state.ink.len() < 10000 {
                                state.ink.push([point.0, point.1]);
                            }
                        } else {
                            state.pan.0 += point.0 - start.0;
                            state.pan.1 += point.1 - start.1;
                            state.drag = Some(point);
                        }
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
            });
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let _ = ReleaseCapture();
            STATE.with(|cell| {
                if let Some(state) = cell.borrow_mut().as_mut() {
                    state.drag = None;
                    if state.markup.is_some() {
                        if let (Some(kind), Some((x1, y1, x2, y2)), Some(path)) =
                            (state.markup, state.selection.take(), state.path.clone())
                        {
                            let rect = state.image_rect;
                            let width = rect.right - rect.left;
                            let height = rect.bottom - rect.top;
                            if width > 0.0 && height > 0.0 {
                                let normalized = |x: f32, y: f32| {
                                    [
                                        ((x - rect.left) / width).clamp(0.0, 1.0),
                                        ((y - rect.top) / height).clamp(0.0, 1.0),
                                    ]
                                };
                                let first = normalized(x1, y1);
                                let mut last = normalized(x2, y2);
                                if matches!(kind, AnnotationKind::Note | AnnotationKind::Text)
                                    && (x1 - x2).abs() < 3.0
                                {
                                    last = [(first[0] + 0.25).min(1.0), (first[1] + 0.08).min(1.0)];
                                }
                                let points = if let Some(signature) = &state.signature {
                                    signature
                                        .iter()
                                        .map(|p| {
                                            [
                                                first[0].min(last[0])
                                                    + p[0] * (first[0] - last[0]).abs(),
                                                first[1].min(last[1])
                                                    + p[1] * (first[1] - last[1]).abs(),
                                            ]
                                        })
                                        .collect()
                                } else if kind == AnnotationKind::Ink {
                                    state.ink.iter().map(|p| normalized(p[0], p[1])).collect()
                                } else {
                                    vec![first, last]
                                };
                                let pdf = is_pdf(&path);
                                let edits = state.sessions.entry(path).or_default();
                                if pdf {
                                    edits.pdf.push(PdfEdit::Annotate {
                                        page: state.page,
                                        kind,
                                        points,
                                        text: state.markup_text.clone(),
                                    });
                                } else {
                                    edits.image.push(ImageEdit::Annotate {
                                        kind,
                                        points,
                                        text: state.markup_text.clone(),
                                    });
                                }
                                edits.dirty = true;
                                state.ink.clear();
                                schedule(hwnd, state, 0);
                            }
                        }
                    }
                    if state.crop {
                        if let (Some((x1, y1, x2, y2)), Some(path)) =
                            (state.selection.take(), state.path.clone())
                        {
                            let rect = state.image_rect;
                            let width = rect.right - rect.left;
                            let height = rect.bottom - rect.top;
                            if width > 0.0
                                && height > 0.0
                                && (x1 - x2).abs() > 3.0
                                && (y1 - y2).abs() > 3.0
                            {
                                let left = ((x1.min(x2) - rect.left) / width).clamp(0.0, 1.0);
                                let right = ((x1.max(x2) - rect.left) / width).clamp(0.0, 1.0);
                                let top = ((y1.min(y2) - rect.top) / height).clamp(0.0, 1.0);
                                let bottom = ((y1.max(y2) - rect.top) / height).clamp(0.0, 1.0);
                                if right > left && bottom > top {
                                    let pdf = is_pdf(&path);
                                    let edits = state.sessions.entry(path).or_default();
                                    if pdf {
                                        edits.pdf.push(PdfEdit::Crop {
                                            page: state.page,
                                            left,
                                            top,
                                            right,
                                            bottom,
                                        });
                                    } else {
                                        edits.image.push(ImageEdit::Crop {
                                            left,
                                            top,
                                            right,
                                            bottom,
                                        });
                                    }
                                    edits.dirty = true;
                                    state.crop = false;
                                    state.pan = (0.0, 0.0);
                                    state.zoom = 1.0;
                                    schedule(hwnd, state, 0);
                                }
                            }
                        }
                    }
                }
            });
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            if GetKeyState(VK_CONTROL.0 as i32) < 0 {
                command(
                    hwnd,
                    if ((wparam.0 >> 16) as u16 as i16) > 0 {
                        ZOOM_IN
                    } else {
                        ZOOM_OUT
                    },
                );
            } else {
                STATE.with(|cell| {
                    if let Some(state) = cell.borrow_mut().as_mut() {
                        state.pan.1 += ((wparam.0 >> 16) as u16 as i16) as f32;
                    }
                });
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let rect = &*(lparam.0 as *const RECT);
            let _ = SetWindowPos(
                hwnd,
                None,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = KillTimer(Some(hwnd), 1);
            PostQuitMessage(0);
            LRESULT(0)
        }
        WM_CLOSE => {
            let (dirty, saving) = STATE.with(|cell| {
                cell.borrow().as_ref().map_or((false, false), |s| {
                    (s.sessions.values().any(|e| e.dirty), s.exporting)
                })
            });
            if saving {
                MessageBoxW(
                    Some(hwnd),
                    w!("Wait for the save to finish before closing Preview."),
                    w!("Saving"),
                    MB_OK,
                );
                return LRESULT(0);
            }
            if dirty && MessageBoxW(Some(hwnd),w!("Close and discard unsaved edits? Your original files are unchanged. Choose No, then Save copy to keep your edits."),w!("Unsaved edits"),MB_YESNO|MB_ICONQUESTION)!=IDYES { return LRESULT(0); }
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

pub(super) fn point(value: LPARAM) -> (f32, f32) {
    (
        (value.0 as u16 as i16) as f32,
        ((value.0 >> 16) as u16 as i16) as f32,
    )
}

impl Graphics {
    pub(super) unsafe fn new(hwnd: HWND, width: u32, height: u32) -> Result<Self> {
        let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let target = factory.CreateHwndRenderTarget(
            &D2D1_RENDER_TARGET_PROPERTIES {
                dpiX: 96.0,
                dpiY: 96.0,
                ..Default::default()
            },
            &D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd,
                pixelSize: D2D_SIZE_U { width, height },
                ..Default::default()
            },
        )?;
        let brush = target.CreateSolidColorBrush(
            &D2D1_COLOR_F {
                r: 0.24,
                g: 0.25,
                b: 0.28,
                a: 1.0,
            },
            None,
        )?;
        let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let format = dwrite.CreateTextFormat(
            w!("Segoe UI"),
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            14.0 * GetDpiForWindow(hwnd) as f32 / 96.0,
            w!("en-us"),
        )?;
        Ok(Self {
            target,
            brush,
            format,
            bitmap: None,
        })
    }
}

pub(super) unsafe fn paint(hwnd: HWND) {
    STATE.with(|cell| { let Ok(mut value) = cell.try_borrow_mut() else { return; }; let Some(state) = value.as_mut() else { return; };
        state.painted = true;
        let mut rect = RECT::default(); if GetClientRect(hwnd, &mut rect).is_err() || rect.right <= 0 || rect.bottom <= 0 { return; }
        let result = (|| -> Result<bool> {
            if state.graphics.is_none() { state.graphics = Some(Graphics::new(hwnd, rect.right as u32, rect.bottom as u32)?); }
            let Some(graphics) = state.graphics.as_mut() else { return Ok(false); };
            if graphics.bitmap.is_none() { if let Some(frame) = state.frame.as_ref() {
                graphics.bitmap = Some(graphics.target.CreateBitmap(D2D_SIZE_U { width: frame.width, height: frame.height }, Some(frame.pixels.as_ptr().cast()), frame.width * 4,
                    &D2D1_BITMAP_PROPERTIES { pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED }, dpiX: 96.0, dpiY: 96.0 })?);
            }}
            graphics.target.BeginDraw(); graphics.target.Clear(Some(&D2D1_COLOR_F { r: 0.94, g: 0.945, b: 0.95, a: 1.0 }));
            let mut drew = false;
            if let (Some(bitmap), Some(frame)) = (&graphics.bitmap, &state.frame) {
                let scale = if state.zoom>1.0 { 1.0 } else { (rect.right as f32 / frame.width as f32).min((rect.bottom - 128).max(1) as f32 / frame.height as f32).min(1.0) };
                let width = frame.width as f32 * scale; let height = frame.height as f32 * scale;
                let left = (rect.right as f32 - width) / 2.0 + state.pan.0; let top = 80.0 + ((rect.bottom - 128) as f32 - height) / 2.0 + state.pan.1;
                state.image_rect=D2D_RECT_F { left, top, right: left + width, bottom: top + height };
                graphics.target.PushAxisAlignedClip(&D2D_RECT_F { left:0.0,top:80.0,right:rect.right as f32,bottom:(rect.bottom-48) as f32 },D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                graphics.target.DrawBitmap(bitmap, Some(&state.image_rect), 1.0, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None);
                if state.markup==Some(AnnotationKind::Ink) && state.signature.is_none() {
                    for pair in state.ink.windows(2) { graphics.target.DrawLine(windows_numerics::Vector2{X:pair[0][0],Y:pair[0][1]},windows_numerics::Vector2{X:pair[1][0],Y:pair[1][1]},&graphics.brush,2.0,None); }
                } else if let Some((x1,y1,x2,y2))=state.selection { graphics.target.DrawRectangle(&D2D_RECT_F { left:x1.min(x2),top:y1.min(y2),right:x1.max(x2),bottom:y1.max(y2) },&graphics.brush,2.0,None); }
                graphics.target.PopAxisAlignedClip(); drew = true;
            }
            let text: Vec<u16> = state.status.encode_utf16().collect();
            graphics.target.DrawText(&text, &graphics.format, &D2D_RECT_F { left: 18.0, top: (rect.bottom - 38).max(12) as f32, right: (rect.right - 18).max(1) as f32, bottom: rect.bottom as f32 }, &graphics.brush, D2D1_DRAW_TEXT_OPTIONS_CLIP, DWRITE_MEASURING_MODE_NATURAL);
            graphics.target.EndDraw(None, None)?;
            Ok(drew)
        })();
        match result {
            Ok(true) if !state.pending && !state.marked && !state.render_failed => {
                if let (Some(path), Some(frame)) = (std::env::var_os("PFW_BENCH_OUT"), state.frame.as_ref()) {
                    let mut counter = 0; let mut frequency = 0;
                    if DwmFlush().is_ok() && QueryPerformanceCounter(&mut counter).is_ok() && QueryPerformanceFrequency(&mut frequency).is_ok() {
                        let json = format!("{{\"first_content_qpc\":{counter},\"qpc_frequency\":{frequency},\"width\":{},\"height\":{},\"page_count\":{}}}", frame.width, frame.height, frame.page_count);
                        if std::fs::write(path, json).is_ok() { state.marked = true;
                            if std::env::var_os("PFW_BENCH_AUTOCLOSE").is_some_and(|v| v == "1") { let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)); }
                        }
                    }
                }
            }
            Err(error) => { state.graphics = None; state.status = format!("Windows could not draw this view: {error}"); }
            _ => {}
        }
    });
}
