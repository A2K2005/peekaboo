use crate::model::{AnnotationKind, Frame, ImageEdit, PdfEdit};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};
use windows::Win32::UI::Controls::{
    InitCommonControlsEx, ICC_TAB_CLASSES, INITCOMMONCONTROLSEX, NMHDR, TCIF_TEXT, TCITEMW,
    TCM_GETCURSEL, TCM_INSERTITEMW, TCM_SETCURSEL, TCM_SETITEMW, TCN_SELCHANGE,
};
use windows::{
    core::*,
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::Common::*, Direct2D::*, DirectWrite::*, Dwm::DwmFlush, Dxgi::Common::*,
            Gdi::*,
        },
        System::{Com::*, LibraryLoader::GetModuleHandleW, Performance::*},
        UI::{
            Controls::Dialogs::*, HiDpi::*, Input::KeyboardAndMouse::*, Shell::*,
            WindowsAndMessaging::*,
        },
    },
};

const OPEN: usize = 100;
const EXIT: usize = 101;
const PREVIOUS: usize = 102;
const NEXT: usize = 103;
const FIT: usize = 104;
const ZOOM_IN: usize = 105;
const ZOOM_OUT: usize = 106;
const ROTATE: usize = 107;
const CROP: usize = 108;
const SAVE: usize = 109;
const FLIP: usize = 110;
const UNDO: usize = 111;
const REVERT: usize = 112;
const RESIZE: usize = 113;
const TEXT: usize = 114;
const FIND: usize = 115;
const EXTRACT: usize = 116;
const MERGE: usize = 117;
const DELETE: usize = 118;
const FORM: usize = 119;
const BACKGROUND: usize = 120;
const PRINT: usize = 121;
const BATCH: usize = 122;
const SLIDESHOW: usize = 123;
const INFO: usize = 124;
const BATCH_SELECTED: usize = 125;
const MOVE: usize = 126;
const INSERT: usize = 127;
const TABS: usize = 300;
const SIGN_SAVE: usize = 219;
const SIGN_PLACE: usize = 220;
#[derive(Clone, Default)]
struct Edits {
    image: Vec<ImageEdit>,
    pdf: Vec<PdfEdit>,
    dirty: bool,
}
#[derive(Clone)]
struct Request {
    generation: u64,
    path: PathBuf,
    page: u32,
    delta: i32,
    width: u32,
    height: u32,
    sessions: HashMap<PathBuf, Edits>,
}
enum Job {
    Render(Request),
    Save(Request, PathBuf, usize, Option<PathBuf>),
    Text(Request),
    Find(Request, String),
    Fields(Request),
    Background(Request, PathBuf),
    Print(Request, crate::printing::PrintJob),
    Batch(
        Request,
        PathBuf,
        String,
        Arc<AtomicBool>,
        Option<Vec<PathBuf>>,
    ),
    Password(Request, String),
}
enum Event {
    Render(Completed),
    Saved(PathBuf, Edits, bool, std::result::Result<(), String>),
    Text(u64, PathBuf, std::result::Result<String, String>),
    Found(u64, std::result::Result<Option<u32>, String>),
    Fields(u64, std::result::Result<Vec<crate::pdf::FormField>, String>),
    Background(PathBuf, std::result::Result<String, String>),
    Finished(std::result::Result<String, String>),
    Progress(String),
}
struct Completed {
    generation: u64,
    path: PathBuf,
    page: u32,
    navigation: bool,
    result: std::result::Result<Frame, String>,
}
struct Graphics {
    target: ID2D1HwndRenderTarget,
    brush: ID2D1SolidColorBrush,
    format: IDWriteTextFormat,
    bitmap: Option<ID2D1Bitmap>,
}
struct State {
    sender: mpsc::Sender<Job>,
    receiver: mpsc::Receiver<Event>,
    path: Option<PathBuf>,
    page: u32,
    generation: u64,
    frame: Option<Frame>,
    graphics: Option<Graphics>,
    status: String,
    pending: bool,
    painted: bool,
    due: Option<Instant>,
    marked: bool,
    sessions: HashMap<PathBuf, Edits>,
    zoom: f32,
    pan: (f32, f32),
    drag: Option<(f32, f32)>,
    crop: bool,
    selection: Option<(f32, f32, f32, f32)>,
    image_rect: D2D_RECT_F,
    exporting: bool,
    displayed: Option<(PathBuf, u32)>,
    render_failed: bool,
    markup: Option<AnnotationKind>,
    ink: Vec<[f32; 2]>,
    markup_text: String,
    signature: Option<Vec<[f32; 2]>>,
    cancel: Option<Arc<AtomicBool>>,
    slideshow: Option<Instant>,
    tabs: Vec<PathBuf>,
    password_attempts: HashMap<PathBuf, u32>,
    views: HashMap<PathBuf, (u32, f32, (f32, f32))>,
}
thread_local! { static STATE: RefCell<Option<State>> = const { RefCell::new(None) }; }
thread_local! { static CONTROL_STATE: RefCell<Option<(bool,bool,bool,bool,bool)>> = const { RefCell::new(None) }; }

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
fn is_pdf(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

fn sibling(path: &Path, delta: i32) -> std::result::Result<PathBuf, String> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("Cannot read this folder: {e}"))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|e| {
                matches!(
                    e.to_string_lossy().to_lowercase().as_str(),
                    "jpg"
                        | "jpeg"
                        | "png"
                        | "gif"
                        | "tif"
                        | "tiff"
                        | "bmp"
                        | "webp"
                        | "heic"
                        | "heif"
                )
            })
        })
        .collect();
    files.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    let index = files
        .iter()
        .position(|p| p == path)
        .ok_or("This image is no longer in its folder.")?;
    let next = index as i64 + delta as i64;
    files
        .get(usize::try_from(next).unwrap_or(usize::MAX))
        .cloned()
        .ok_or("No more images in this direction.".into())
}

fn worker(receiver: mpsc::Receiver<Job>, sender: mpsc::Sender<Event>) {
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_MULTITHREADED).ok();
        let mut engine = None;
        while let Ok(job) = receiver.recv() {
            let job = match job {
                Job::Password(request, password) => {
                    if engine.is_none() {
                        engine = crate::pdf::PdfEngine::new().ok();
                    }
                    let result = match engine.as_mut() {
                        Some(engine) => engine.set_password(&request.path, password),
                        None => Err("PDF support could not load.".into()),
                    };
                    if let Err(error) = result {
                        let _ = sender.send(Event::Render(Completed {
                            generation: request.generation,
                            path: request.path,
                            page: request.page,
                            navigation: false,
                            result: Err(error),
                        }));
                        continue;
                    }
                    Job::Render(request)
                }
                other => other,
            };
            let mut request = match &job {
                Job::Render(r)
                | Job::Save(r, _, _, _)
                | Job::Text(r)
                | Job::Find(r, _)
                | Job::Fields(r)
                | Job::Background(r, _)
                | Job::Print(r, _)
                | Job::Batch(r, _, _, _, _)
                | Job::Password(r, _) => r.clone(),
            };
            if !matches!(job, Job::Render(_)) {
                let edits = request
                    .sessions
                    .get(&request.path)
                    .cloned()
                    .unwrap_or_default();
                if is_pdf(&request.path) && engine.is_none() {
                    engine = crate::pdf::PdfEngine::new().ok();
                }
                let event = match job {
                    Job::Save(_, output, mode, other) => {
                        let result = if is_pdf(&request.path) {
                            match engine.as_mut() {
                                None => Err("PDF support could not load.".into()),
                                Some(engine) => match mode {
                                    EXTRACT => engine.extract_page(
                                        &request.path,
                                        &output,
                                        request.page,
                                        &edits.pdf,
                                    ),
                                    MERGE => match other {
                                        Some(other) => {
                                            engine.merge(&request.path, &other, &output, &edits.pdf)
                                        }
                                        None => Err("Choose a PDF to combine.".into()),
                                    },
                                    _ => engine.save_copy(&request.path, &output, &edits.pdf),
                                },
                            }
                        } else if is_pdf(&output) {
                            if engine.is_none() {
                                engine = crate::pdf::PdfEngine::new().ok();
                            }
                            match engine.as_mut() {
                                Some(engine) => crate::imaging::decode_edited(
                                    &request.path,
                                    u32::MAX,
                                    u32::MAX,
                                    &edits.image,
                                )
                                .and_then(|frame| engine.create_from_image(&frame, &output)),
                                None => Err("PDF support could not load.".into()),
                            }
                        } else {
                            crate::imaging::export(&request.path, &output, &edits.image)
                        };
                        Event::Saved(
                            request.path,
                            edits,
                            mode != EXTRACT,
                            result.map_err(|e| format!("Could not save {}: {e}", output.display())),
                        )
                    }
                    Job::Text(_) => Event::Text(
                        request.generation,
                        request.path.clone(),
                        if is_pdf(&request.path) {
                            match engine.as_mut() {
                                Some(e) => {
                                    e.page_text_edited(&request.path, request.page, &edits.pdf)
                                }
                                None => Err("Open a PDF first.".into()),
                            }
                        } else {
                            crate::ocr::recognize(&request.path, &edits.image)
                        },
                    ),
                    Job::Find(_, query) => Event::Found(
                        request.generation,
                        match engine.as_mut() {
                            Some(e) => e.find_edited(
                                &request.path,
                                &query,
                                request.page.saturating_add(1),
                                &edits.pdf,
                            ),
                            None => Err("Open a PDF first.".into()),
                        },
                    ),
                    Job::Fields(_) => Event::Fields(
                        request.generation,
                        match engine.as_mut() {
                            Some(e) => e.form_fields(&request.path, request.page, &edits.pdf),
                            None => Err("Open a PDF first.".into()),
                        },
                    ),
                    Job::Background(_, output) => {
                        let result =
                            crate::background::remove(&request.path, &output, &edits.image);
                        Event::Background(output, result)
                    }
                    Job::Print(_, job) => {
                        if engine.is_none() {
                            engine = crate::pdf::PdfEngine::new().ok();
                        }
                        Event::Finished(match engine.as_mut() {
                            Some(engine) => crate::printing::print(
                                job,
                                engine,
                                &request.path,
                                &edits.pdf,
                                &edits.image,
                                is_pdf(&request.path),
                            )
                            .map(|_| "Print job sent to Windows.".into()),
                            None => Err("PDF support could not load.".into()),
                        })
                    }
                    Job::Batch(_, folder, extension, cancel, selected) => {
                        let mut good = 0;
                        let mut errors = Vec::new();
                        let files = if let Some(selected) = selected {
                            Ok(selected)
                        } else {
                            std::fs::read_dir(request.path.parent().unwrap_or(Path::new("."))).map(
                                |entries| {
                                    entries
                                        .filter_map(|e| e.ok())
                                        .map(|e| e.path())
                                        .filter(|p| {
                                            p.is_file()
                                                && p.extension().is_some_and(|e| {
                                                    matches!(
                                                        e.to_string_lossy().to_lowercase().as_str(),
                                                        "png"
                                                            | "jpg"
                                                            | "jpeg"
                                                            | "bmp"
                                                            | "tif"
                                                            | "tiff"
                                                            | "gif"
                                                            | "webp"
                                                            | "heic"
                                                            | "heif"
                                                    )
                                                })
                                        })
                                        .collect::<Vec<_>>()
                                },
                            )
                        };
                        match files {
                            Err(error) => {
                                Event::Finished(Err(format!("Cannot read this folder: {error}")))
                            }
                            Ok(mut files) => {
                                files.sort();
                                let total = files.len();
                                for (index, path) in files.iter().enumerate() {
                                    if cancel.load(Ordering::Acquire) {
                                        break;
                                    }
                                    let _=sender.send(Event::Progress(format!("Converting image {} of {total}. Escape cancels after the current file.",index+1)));
                                    let mut output =
                                        folder.join(path.file_name().unwrap_or_default());
                                    output.set_extension(&extension);
                                    let result = if extension == "pdf" {
                                        if engine.is_none() {
                                            engine = crate::pdf::PdfEngine::new().ok();
                                        }
                                        match engine.as_mut() {
                                            Some(engine) => crate::imaging::decode_edited(
                                                path,
                                                u32::MAX,
                                                u32::MAX,
                                                &edits.image,
                                            )
                                            .and_then(|frame| {
                                                engine.create_from_image(&frame, &output)
                                            }),
                                            None => Err("PDF support could not load.".into()),
                                        }
                                    } else {
                                        crate::imaging::export(path, &output, &edits.image)
                                    };
                                    match result {
                                        Ok(()) => good += 1,
                                        Err(error) => errors.push(format!(
                                            "{}: {error}",
                                            path.file_name().unwrap_or_default().to_string_lossy()
                                        )),
                                    }
                                }
                                Event::Finished(Ok(format!(
                                    "Batch finished: {good} saved, {} failed, {} not processed.{}",
                                    errors.len(),
                                    total - good - errors.len(),
                                    if errors.is_empty() {
                                        String::new()
                                    } else {
                                        format!(" First error: {}", errors[0])
                                    }
                                )))
                            }
                        }
                    }
                    Job::Render(_) | Job::Password(_, _) => continue,
                };
                if sender.send(event).is_err() {
                    break;
                }
                continue;
            }
            let result = if let Err(error) = &initialized {
                Err(format!("Cannot initialize Windows imaging: {error}"))
            } else {
                let resolved = if request.delta != 0 {
                    sibling(&request.path, request.delta)
                } else {
                    Ok(request.path.clone())
                };
                match resolved {
                    Err(error) => Err(error),
                    Ok(path) => {
                        request.path = path;
                        if is_pdf(&request.path) {
                            if engine.is_none() {
                                engine = crate::pdf::PdfEngine::new().ok();
                            }
                            match engine.as_mut() {
                                Some(engine) => engine.render_edited(&request.path, request.page, request.width, request.height, &request.sessions.get(&request.path).cloned().unwrap_or_default().pdf),
                                None => Err("PDFium could not load. Install the verified pdfium.dll beside Preview and reopen the PDF.".into()),
                            }
                        } else {
                            crate::imaging::decode_edited(
                                &request.path,
                                request.width,
                                request.height,
                                &request
                                    .sessions
                                    .get(&request.path)
                                    .cloned()
                                    .unwrap_or_default()
                                    .image,
                            )
                        }
                    }
                }
            };
            if sender
                .send(Event::Render(Completed {
                    generation: request.generation,
                    path: request.path,
                    page: request.page,
                    navigation: request.delta != 0,
                    result,
                }))
                .is_err()
            {
                break;
            }
        }
        drop(engine);
        if initialized.is_ok() {
            CoUninitialize();
        }
    }
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

unsafe fn choose(hwnd: HWND) -> Option<PathBuf> {
    choose_paths(hwnd, false, false).and_then(|paths| paths.into_iter().next())
}
unsafe fn choose_many(hwnd: HWND, images_only: bool) -> Option<Vec<PathBuf>> {
    choose_paths(hwnd, images_only, true)
}
unsafe fn choose_paths(hwnd: HWND, images_only: bool, multiple: bool) -> Option<Vec<PathBuf>> {
    let mut buffer = vec![0u16; 1024 * 1024];
    let filter = wide(if images_only {
        "Images\0*.jpg;*.jpeg;*.png;*.gif;*.tif;*.tiff;*.bmp;*.webp;*.heic;*.heif\0"
    } else {
        "PDF and images\0*.pdf;*.jpg;*.jpeg;*.png;*.gif;*.tif;*.tiff;*.bmp;*.webp;*.heic;*.heif\0All files\0*.*\0"
    });
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: hwnd,
        lpstrFile: PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        Flags: OFN_FILEMUSTEXIST
            | OFN_PATHMUSTEXIST
            | OFN_NOCHANGEDIR
            | OFN_EXPLORER
            | if multiple {
                OFN_ALLOWMULTISELECT
            } else {
                OPEN_FILENAME_FLAGS(0)
            },
        ..Default::default()
    };
    if !GetOpenFileNameW(&mut dialog).as_bool() {
        return None;
    }
    Some(picker_paths(&buffer))
}
fn picker_paths(buffer: &[u16]) -> Vec<PathBuf> {
    let parts: Vec<_> = buffer
        .split(|c| *c == 0)
        .take_while(|s| !s.is_empty())
        .map(|s| PathBuf::from(String::from_utf16_lossy(s)))
        .collect();
    if parts.len() <= 1 {
        return parts;
    }
    parts[1..].iter().map(|name| parts[0].join(name)).collect()
}
unsafe fn add_tabs(hwnd: HWND, paths: &[PathBuf]) {
    let Ok(tabs) = GetDlgItem(Some(hwnd), TABS as i32) else {
        return;
    };
    STATE.with(|cell| {
        if let Some(state) = cell.borrow_mut().as_mut() {
            for path in paths {
                let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
                if state.tabs.contains(&path) {
                    continue;
                }
                let mut text = wide(&path.file_name().unwrap_or_default().to_string_lossy());
                let item = TCITEMW {
                    mask: TCIF_TEXT,
                    pszText: PWSTR(text.as_mut_ptr()),
                    ..Default::default()
                };
                let index = state.tabs.len();
                SendMessageW(
                    tabs,
                    TCM_INSERTITEMW,
                    Some(WPARAM(index)),
                    Some(LPARAM((&item as *const TCITEMW) as isize)),
                );
                state.tabs.push(path);
            }
        }
    });
}

unsafe fn destination(hwnd: HWND, pdf: bool) -> Option<PathBuf> {
    let mut buffer = vec![0u16; 32768];
    let name = wide(if pdf {
        "Edited copy.pdf"
    } else {
        "Edited copy.png"
    });
    buffer[..name.len()].copy_from_slice(&name);
    let filter = wide(if pdf {
        "PDF\0*.pdf\0"
    } else {
        "PNG image\0*.png\0JPEG image\0*.jpg\0TIFF image\0*.tif\0Bitmap\0*.bmp\0PDF document\0*.pdf\0WebP image\0*.webp\0"
    });
    let extension = wide(if pdf { "pdf" } else { "png" });
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: hwnd,
        lpstrFile: PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrDefExt: PCWSTR(extension.as_ptr()),
        Flags: OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    if !GetSaveFileNameW(&mut dialog).as_bool() {
        return None;
    }
    let length = buffer.iter().position(|v| *v == 0).unwrap_or(0);
    let mut path = PathBuf::from(String::from_utf16_lossy(&buffer[..length]));
    if !pdf {
        path.set_extension(match dialog.nFilterIndex {
            2 => "jpg",
            3 => "tif",
            4 => "bmp",
            5 => "pdf",
            6 => "webp",
            _ => "png",
        });
    }
    if path.exists() {
        MessageBoxW(Some(hwnd), w!("Choose a new file name. Preview saves a copy and does not overwrite an existing file."), w!("Save a copy"), MB_OK | MB_ICONINFORMATION);
        None
    } else {
        Some(path)
    }
}

unsafe fn folder(hwnd: HWND) -> Option<PathBuf> {
    let mut display = vec![0u16; 260];
    let info = BROWSEINFOW {
        hwndOwner: hwnd,
        pszDisplayName: PWSTR(display.as_mut_ptr()),
        lpszTitle: w!("Choose a folder for new copies. Existing files will not be overwritten."),
        ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
        ..Default::default()
    };
    let pidl = SHBrowseForFolderW(&info);
    if pidl.is_null() {
        return None;
    }
    let mut buffer = vec![0u16; 32768];
    let ok = SHGetPathFromIDListEx(pidl, &mut buffer, GPFIDL_DEFAULT).as_bool();
    CoTaskMemFree(Some(pidl.cast()));
    if !ok {
        return None;
    }
    let length = buffer.iter().position(|v| *v == 0).unwrap_or(0);
    Some(PathBuf::from(String::from_utf16_lossy(&buffer[..length])))
}

struct InputDialog {
    edits: Vec<HWND>,
    answer: Option<Vec<String>>,
    finished: bool,
}
thread_local! { static INPUT: RefCell<Option<InputDialog>> = const { RefCell::new(None) }; }
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

unsafe fn input(hwnd: HWND, title: &str, fields: &[(&str, String)]) -> Option<Vec<String>> {
    input_with_password(hwnd, title, fields, false)
}
unsafe fn input_with_password(
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

unsafe fn command(hwnd: HWND, id: usize) {
    if id == OPEN {
        if let Some(paths) = choose_many(hwnd, false) {
            add_tabs(hwnd, &paths);
            if let Some(path) = paths.into_iter().next() {
                open(hwnd, path);
            }
        }
        return;
    }
    if id == EXIT {
        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        return;
    }
    if id == PREVIOUS || id == NEXT {
        navigate(hwnd, if id == PREVIOUS { -1 } else { 1 });
        return;
    }
    let snapshot = STATE.with(|cell| {
        cell.borrow().as_ref().and_then(|s| {
            if s.pending || (s.render_failed && !matches!(id, UNDO | REVERT)) || s.frame.is_none() {
                return None;
            }
            Some((
                Request {
                    generation: s.generation,
                    path: s.path.clone()?,
                    page: s.page,
                    delta: 0,
                    width: 1,
                    height: 1,
                    sessions: s.sessions.clone(),
                },
                s.frame.as_ref()?.source_width,
                s.frame.as_ref()?.source_height,
                s.frame.as_ref()?.page_count,
            ))
        })
    });
    let Some((request, width, height, count)) = snapshot else {
        return;
    };
    let pdf = is_pdf(&request.path);
    if id == INFO {
        let bytes = std::fs::metadata(&request.path)
            .map(|m| m.len())
            .unwrap_or(0);
        let text=wide(&format!("{}\n\nSource dimensions: {width} × {height}\nPages: {count}\nFile size: {bytes} bytes\n\nEdits are kept in memory until you save a new copy.",request.path.display()));
        MessageBoxW(
            Some(hwnd),
            PCWSTR(text.as_ptr()),
            w!("File information"),
            MB_OK,
        );
        return;
    }
    if id == SLIDESHOW {
        STATE.with(|cell| {
            if let Some(state) = cell.borrow_mut().as_mut() {
                state.slideshow = if state.slideshow.is_some() {
                    None
                } else {
                    Some(Instant::now() + Duration::from_secs(3))
                };
                state.status = "Slideshow advances every 3 seconds. Escape stops.".into();
            }
        });
        return;
    }
    if id == PRINT {
        match crate::printing::choose(hwnd, count) {
            Ok(Some(job)) => {
                let cancel = job.cancellation();
                STATE.with(|cell| {
                    if let Some(state) = cell.borrow_mut().as_mut() {
                        if state.exporting {
                            return;
                        }
                        state.exporting = true;
                        state.cancel = Some(cancel);
                        state.status = "Printing... Escape cancels.".into();
                        if state.sender.send(Job::Print(request, job)).is_err() {
                            state.exporting = false;
                            state.status = "The print worker stopped.".into();
                        }
                    }
                });
            }
            Ok(None) => {}
            Err(error) => {
                let text = wide(&error);
                MessageBoxW(Some(hwnd), PCWSTR(text.as_ptr()), w!("Print"), MB_OK);
            }
        }
        let _ = InvalidateRect(Some(hwnd), None, false);
        return;
    }
    if id == BATCH || id == BATCH_SELECTED {
        let selected = if id == BATCH_SELECTED {
            let Some(paths) = choose_many(hwnd, true) else {
                return;
            };
            if paths.is_empty() {
                return;
            }
            Some(paths)
        } else {
            None
        };
        if pdf {
            return;
        }
        let Some(values) = input(
            hwnd,
            "Batch convert image folder",
            &[(
                "Output format: png, jpg, tif, bmp, pdf, or webp",
                "png".into(),
            )],
        ) else {
            return;
        };
        let extension = values
            .first()
            .map(|v| v.trim().to_lowercase())
            .unwrap_or_default();
        if !matches!(
            extension.as_str(),
            "png" | "jpg" | "tif" | "bmp" | "pdf" | "webp"
        ) {
            return;
        }
        let Some(output) = folder(hwnd) else {
            return;
        };
        let question = wide(&if let Some(paths) = &selected {
            format!("Apply the current image edits to {} selected images and save new copies? Existing files are not overwritten.",paths.len())
        } else {
            "Apply the current image edits to every supported image in this folder and save new copies? Existing files are not overwritten.".into()
        });
        if MessageBoxW(
            Some(hwnd),
            PCWSTR(question.as_ptr()),
            w!("Batch convert"),
            MB_YESNO | MB_ICONQUESTION,
        ) != IDYES
        {
            return;
        }
        STATE.with(|cell| {
            if let Some(state) = cell.borrow_mut().as_mut() {
                if state.exporting {
                    return;
                }
                let cancel = Arc::new(AtomicBool::new(false));
                state.cancel = Some(cancel.clone());
                state.exporting = true;
                state.status = "Starting batch conversion...".into();
                if state
                    .sender
                    .send(Job::Batch(request, output, extension, cancel, selected))
                    .is_err()
                {
                    state.exporting = false;
                    state.status = "The processing worker stopped.".into();
                }
            }
        });
        let _ = InvalidateRect(Some(hwnd), None, false);
        return;
    }
    if id == BACKGROUND {
        if pdf {
            return;
        }
        let Some(mut output) = destination(hwnd, false) else {
            return;
        };
        output.set_extension("png");
        STATE.with(|cell| {
            if let Some(state) = cell.borrow_mut().as_mut() {
                if state.exporting {
                    return;
                }
                state.exporting = true;
                state.status = "Removing background on this PC...".into();
                if state.sender.send(Job::Background(request, output)).is_err() {
                    state.exporting = false;
                    state.status = "The processing worker stopped.".into();
                }
            }
        });
        let _ = InvalidateRect(Some(hwnd), None, false);
        return;
    }
    if (200..=208).contains(&id) || id == SIGN_SAVE || id == SIGN_PLACE {
        if id == SIGN_SAVE {
            save_signature(hwnd);
            return;
        }
        let signature = if id == SIGN_PLACE {
            match load_signature() {
                Ok(points) => Some(points),
                Err(error) => {
                    let error = wide(&error);
                    MessageBoxW(Some(hwnd), PCWSTR(error.as_ptr()), w!("Signature"), MB_OK);
                    return;
                }
            }
        } else {
            None
        };
        let kind = match id {
            201 => AnnotationKind::Highlight,
            202 => AnnotationKind::Underline,
            203 => AnnotationKind::Strikeout,
            204 => AnnotationKind::Note,
            205 => AnnotationKind::Rectangle,
            206 => AnnotationKind::Ellipse,
            207 => AnnotationKind::Arrow,
            208 => AnnotationKind::Text,
            _ => AnnotationKind::Ink,
        };
        let text = if matches!(kind, AnnotationKind::Text | AnnotationKind::Note) {
            let Some(values) = input(hwnd, "PDF annotation", &[("Text", String::new())]) else {
                return;
            };
            values.first().cloned().unwrap_or_default()
        } else {
            String::new()
        };
        STATE.with(|cell| {
            if let Some(state) = cell.borrow_mut().as_mut() {
                state.crop = false;
                state.markup = Some(kind);
                state.markup_text = text;
                state.signature = signature;
                state.status =
                    "Drag on the PDF to place markup. Escape returns to navigation.".into();
            }
        });
        let _ = InvalidateRect(Some(hwnd), None, false);
        return;
    }
    if matches!(id, SAVE | EXTRACT | MERGE) {
        if id != SAVE && !pdf {
            return;
        }
        let other = if id == MERGE {
            let Some(path) = choose(hwnd) else {
                return;
            };
            Some(path)
        } else {
            None
        };
        let Some(output) = destination(hwnd, pdf) else {
            return;
        };
        STATE.with(|cell| {
            if let Some(state) = cell.borrow_mut().as_mut() {
                if state.exporting {
                    return;
                }
                state.exporting = true;
                state.status = "Saving a new copy...".into();
                if state
                    .sender
                    .send(Job::Save(request, output, id, other))
                    .is_err()
                {
                    state.exporting = false;
                    state.status = "The save worker stopped.".into();
                }
            }
        });
        return;
    }
    if id == TEXT || id == FIND || id == FORM {
        if !pdf && id != TEXT {
            return;
        }
        let job = if id == FORM {
            Job::Fields(request)
        } else if id == TEXT {
            Job::Text(request)
        } else {
            let Some(values) = input(hwnd, "Find text in PDF", &[("Text to find", String::new())])
            else {
                return;
            };
            let Some(query) = values.first().filter(|v| !v.trim().is_empty()) else {
                return;
            };
            Job::Find(request, query.clone())
        };
        STATE.with(|cell| {
            if let Some(state) = cell.borrow_mut().as_mut() {
                let _ = state.sender.send(job);
                state.status = "Reading text on this PC...".into();
            }
        });
        let _ = InvalidateRect(Some(hwnd), None, false);
        return;
    }
    let mut resize = None;
    let mut page_destination = None;
    if id == MOVE {
        if !pdf {
            return;
        }
        let Some(values) = input(
            hwnd,
            "Move PDF page",
            &[("New page number (1-based)", (request.page + 1).to_string())],
        ) else {
            return;
        };
        page_destination = values
            .first()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|v| *v > 0 && *v <= count)
            .map(|v| v - 1);
        if page_destination.is_none() {
            MessageBoxW(
                Some(hwnd),
                w!("Enter a page number within this document."),
                w!("Invalid page number"),
                MB_OK,
            );
            return;
        }
    }
    if id == RESIZE {
        if pdf {
            return;
        }
        let Some(values) = input(
            hwnd,
            "Resize image",
            &[
                ("Width in pixels", width.to_string()),
                ("Height in pixels (blank keeps aspect ratio)", String::new()),
            ],
        ) else {
            return;
        };
        let new_width = values.first().and_then(|v| v.trim().parse::<u32>().ok());
        if let Some(new_width) = new_width.filter(|v| *v > 0 && *v <= 100000) {
            let new_height = if values.get(1).is_none_or(|v| v.trim().is_empty()) {
                ((height as u64 * new_width as u64 + width.max(1) as u64 / 2) / width.max(1) as u64)
                    .max(1) as u32
            } else {
                values[1].trim().parse::<u32>().unwrap_or(0)
            };
            if new_height > 0 && (new_width as u64 * new_height as u64) <= 200_000_000 {
                resize = Some(ImageEdit::Resize {
                    width: new_width,
                    height: new_height,
                });
            }
        }
        if resize.is_none() {
            MessageBoxW(
                Some(hwnd),
                w!("Enter positive pixel dimensions within 200 million pixels."),
                w!("Invalid size"),
                MB_OK | MB_ICONERROR,
            );
            return;
        }
    }
    if id == DELETE && (!pdf || count <= 1 || MessageBoxW(Some(hwnd),w!("Delete this page from the working copy? The original stays unchanged until you save a new copy."),w!("Delete page"),MB_YESNO|MB_ICONQUESTION) != IDYES) { return; }
    STATE.with(|cell| { let mut value = cell.borrow_mut(); let Some(state) = value.as_mut() else { return; };
        if matches!(id,FIT|ZOOM_IN|ZOOM_OUT) { state.zoom = match id { FIT => 1.0, ZOOM_IN => (state.zoom * 1.4).min(8.0), _ => (state.zoom / 1.4).max(0.25) }; state.pan=(0.0,0.0); schedule(hwnd,state,0); return; }
        if id == CROP {state.markup=None;state.crop = !state.crop; state.status = if pdf{"Drag a rectangle to crop the page view. This does not redact hidden content. Escape cancels.".into()}else{"Drag a rectangle over the image to crop. Escape cancels.".into()};let _ = InvalidateRect(Some(hwnd),None,false);return;}
        let edits = state.sessions.entry(request.path).or_default();
        match id {
            ROTATE if pdf => edits.pdf.push(PdfEdit::RotateRight { page:state.page }),
            ROTATE => edits.image.push(ImageEdit::RotateRight),
            FLIP if !pdf => edits.image.push(ImageEdit::FlipHorizontal),
            RESIZE => { if let Some(edit) = resize { edits.image.push(edit); } },
            DELETE => { edits.pdf.push(PdfEdit::Delete { page:state.page }); state.page = state.page.min(count.saturating_sub(2)); },
            MOVE=>{if let Some(to)=page_destination{edits.pdf.push(PdfEdit::Move{from:state.page,to});state.page=to;}},
            INSERT if pdf=>{let at=state.page+1;edits.pdf.push(PdfEdit::InsertBlank{at});state.page=at;},
            UNDO => { if pdf { edits.pdf.pop(); } else { edits.image.pop(); } },
            REVERT => { edits.pdf.clear(); edits.image.clear(); state.page=0; },
            _ => return,
        }
        edits.dirty = !edits.image.is_empty() || !edits.pdf.is_empty(); state.zoom=1.0; state.pan=(0.0,0.0); schedule(hwnd,state,0);
    });
}

fn signature_path() -> std::result::Result<PathBuf, String> {
    std::env::var_os("LOCALAPPDATA")
        .map(|p| {
            PathBuf::from(p)
                .join("PreviewForWindows")
                .join("signature.txt")
        })
        .ok_or("Windows local app storage is unavailable.".into())
}
fn load_signature() -> std::result::Result<Vec<[f32; 2]>, String> {
    let text = std::fs::read_to_string(signature_path()?).map_err(|_| {
        "Draw a signature with Markup > Ink, then choose Save last ink as signature.".to_string()
    })?;
    if text.len() > 300000 {
        return Err("The saved signature is invalid.".into());
    }
    let mut points = Vec::new();
    for line in text.lines() {
        let Some((x, y)) = line.split_once(',') else {
            return Err("The saved signature is invalid.".into());
        };
        let x = x
            .parse::<f32>()
            .map_err(|_| "The saved signature is invalid.")?;
        let y = y
            .parse::<f32>()
            .map_err(|_| "The saved signature is invalid.")?;
        if !x.is_finite()
            || !y.is_finite()
            || !(0.0..=1.0).contains(&x)
            || !(0.0..=1.0).contains(&y)
        {
            return Err("The saved signature is invalid.".into());
        }
        points.push([x, y]);
    }
    if points.len() < 2 {
        return Err("The saved signature is empty.".into());
    }
    Ok(points)
}
unsafe fn save_signature(hwnd: HWND) {
    let points = STATE.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|s| {
                s.path
                    .as_ref()
                    .and_then(|p| s.sessions.get(p).map(|e| (p, e)))
            })
            .and_then(|(path, edits)| {
                if is_pdf(path) {
                    edits.pdf.iter().rev().find_map(|e| match e {
                        PdfEdit::Annotate {
                            kind: AnnotationKind::Ink,
                            points,
                            ..
                        } => Some(points.clone()),
                        _ => None,
                    })
                } else {
                    edits.image.iter().rev().find_map(|e| match e {
                        ImageEdit::Annotate {
                            kind: AnnotationKind::Ink,
                            points,
                            ..
                        } => Some(points.clone()),
                        _ => None,
                    })
                }
            })
    });
    let result = (|| -> std::result::Result<(), String> {
        let points = points.ok_or("Draw a signature with Markup > Ink first.")?;
        let left = points.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min);
        let top = points.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min);
        let width = points
            .iter()
            .map(|p| p[0])
            .fold(f32::NEG_INFINITY, f32::max)
            - left;
        let height = points
            .iter()
            .map(|p| p[1])
            .fold(f32::NEG_INFINITY, f32::max)
            - top;
        if width <= 0.0 || height <= 0.0 {
            return Err("Draw a signature with both width and height.".into());
        }
        let text = points
            .iter()
            .map(|p| format!("{},{}", (p[0] - left) / width, (p[1] - top) / height))
            .collect::<Vec<_>>()
            .join("\n");
        let path = signature_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, text).map_err(|e| e.to_string())
    })();
    let text=wide(&match result {Ok(())=>"Signature saved on this PC. Use Markup > Place saved signature, then drag its size and position.".into(),Err(e)=>e});
    MessageBoxW(Some(hwnd), PCWSTR(text.as_ptr()), w!("Signature"), MB_OK);
}

unsafe fn schedule(hwnd: HWND, state: &mut State, delta: i32) {
    let Some(path) = state.path.clone() else {
        return;
    };
    let mut rect = RECT::default();
    if GetClientRect(hwnd, &mut rect).is_err() {
        return;
    }
    if rect.right <= 0 || rect.bottom <= 100 {
        return;
    }
    state.generation = state.generation.wrapping_add(1);
    state.pending = true;
    state.render_failed = false;
    state.status = "Opening...".into();
    let request = Request {
        generation: state.generation,
        path,
        page: state.page,
        delta,
        width: ((rect.right as f32 * state.zoom) as u32).clamp(1, 4096),
        height: (((rect.bottom - 128) as f32 * state.zoom) as u32).clamp(1, 4096),
        sessions: state.sessions.clone(),
    };
    if state.sender.send(Job::Render(request)).is_err() {
        state.pending = false;
        state.status = "The rendering worker stopped. Close Preview and reopen the file.".into();
    }
    let _ = InvalidateRect(Some(hwnd), None, false);
}

unsafe fn open(hwnd: HWND, path: PathBuf) {
    STATE.with(|cell| {
        if let Ok(mut value) = cell.try_borrow_mut() {
            if let Some(state) = value.as_mut() {
                if let Some((path, page)) = &state.displayed {
                    if !state.pending && !state.render_failed {
                        state
                            .views
                            .insert(path.clone(), (*page, state.zoom, state.pan));
                    }
                }
                let path = std::fs::canonicalize(&path).unwrap_or(path);
                let view = state
                    .views
                    .get(&path)
                    .copied()
                    .unwrap_or((0, 1.0, (0.0, 0.0)));
                state.path = Some(path);
                state.page = view.0;
                state.due = None;
                state.zoom = view.1;
                state.pan = view.2;
                state.crop = false;
                state.markup = None;
                state.selection = None;
                schedule(hwnd, state, 0);
            }
        }
    });
}

unsafe fn navigate(hwnd: HWND, delta: i32) {
    STATE.with(|cell| {
        if let Ok(mut value) = cell.try_borrow_mut() {
            if let Some(state) = value.as_mut() {
                if state.pending {
                    return;
                }
                if state.path.as_ref().is_some_and(|p| is_pdf(p)) {
                    let count = state.frame.as_ref().map_or(0, |f| f.page_count);
                    let page = state.page as i64 + delta as i64;
                    if page < 0 || page >= count as i64 {
                        return;
                    }
                    state.page = page as u32;
                    schedule(hwnd, state, 0);
                } else {
                    schedule(hwnd, state, delta);
                }
            }
        }
    });
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

fn point(value: LPARAM) -> (f32, f32) {
    (
        (value.0 as u16 as i16) as f32,
        ((value.0 >> 16) as u16 as i16) as f32,
    )
}

unsafe fn tick(hwnd: HWND) {
    let mut fields_to_show = None;
    let mut output_to_open = None;
    let mut advance = false;
    let mut password_to_show = None;
    STATE.with(|cell| {
        let Ok(mut value) = cell.try_borrow_mut() else {
            return;
        };
        let Some(state) = value.as_mut() else {
            return;
        };
        while let Ok(event) = state.receiver.try_recv() {
            let completed = match event {
                Event::Render(completed) => completed,
                Event::Saved(path, edits, whole_document, result) => {
                    state.exporting = false;
                    state.status = match result {
                        Ok(()) => {
                            if let Some(current) = state.sessions.get_mut(&path) {
                                if whole_document
                                    && current.image == edits.image
                                    && current.pdf == edits.pdf
                                {
                                    current.dirty = false;
                                }
                            }
                            "Saved a new copy. The original file is unchanged.".into()
                        }
                        Err(error) => error,
                    };
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    continue;
                }
                Event::Text(generation, path, result) => {
                    if generation != state.generation
                        || state.path.as_ref() != Some(&path)
                        || state.pending
                    {
                        continue;
                    }
                    state.status = match result {
                        Ok(text) if text.is_empty() => "No text found.".into(),
                        Ok(text) => match clipboard(hwnd, &text) {
                            Ok(()) => {
                                "Text copied. Check recognition and reading order before pasting."
                                    .into()
                            }
                            Err(error) => error,
                        },
                        Err(error) => error,
                    };
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    continue;
                }
                Event::Found(generation, result) => {
                    if generation == state.generation {
                        match result {
                            Ok(Some(page)) => {
                                state.page = page;
                                schedule(hwnd, state, 0);
                            }
                            Ok(None) => state.status = "No matching text found.".into(),
                            Err(error) => state.status = error,
                        };
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                    continue;
                }
                Event::Fields(generation, result) => {
                    if generation == state.generation {
                        match result {
                            Ok(fields) => fields_to_show = Some((generation, fields)),
                            Err(error) => state.status = error,
                        }
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                    continue;
                }
                Event::Background(output, result) => {
                    state.exporting = false;
                    match result {
                        Ok(detail) => {
                            state.status = detail;
                            output_to_open = Some(output);
                        }
                        Err(error) => state.status = error,
                    }
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    continue;
                }
                Event::Finished(result) => {
                    state.exporting = false;
                    state.cancel = None;
                    state.status = match result {
                        Ok(detail) => detail,
                        Err(error) => error,
                    };
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    continue;
                }
                Event::Progress(text) => {
                    state.status = text;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    continue;
                }
            };
            if completed.generation != state.generation {
                continue;
            }
            state.pending = false;
            match completed.result {
                Ok(frame) => {
                    if crate::model::frame_bytes(frame.width, frame.height).ok()
                        != Some(frame.pixels.len())
                    {
                        state.status = "The decoder returned an invalid image.".into();
                        continue;
                    }
                    let previous_dirty = state
                        .displayed
                        .as_ref()
                        .and_then(|(path, _)| state.sessions.get(path))
                        .is_some_and(|edits| edits.dirty);
                    if completed.navigation {
                        if let Some((path, page)) = &state.displayed {
                            if path != &completed.path {
                                state
                                    .views
                                    .insert(path.clone(), (*page, state.zoom, state.pan));
                            }
                        }
                    }
                    state.password_attempts.remove(&completed.path);
                    state.displayed = Some((completed.path.clone(), completed.page));
                    state.path = Some(completed.path);
                    state.page = completed.page;
                    state.render_failed = false;
                    if let (Some(path), Ok(tabs)) =
                        (&state.path, GetDlgItem(Some(hwnd), TABS as i32))
                    {
                        if completed.navigation && !previous_dirty && !state.tabs.contains(path) {
                            let selected = SendMessageW(tabs, TCM_GETCURSEL, None, None).0;
                            if let Some(tab) = state.tabs.get_mut(selected as usize) {
                                *tab = path.clone();
                                let mut text =
                                    wide(&path.file_name().unwrap_or_default().to_string_lossy());
                                let item = TCITEMW {
                                    mask: TCIF_TEXT,
                                    pszText: PWSTR(text.as_mut_ptr()),
                                    ..Default::default()
                                };
                                SendMessageW(
                                    tabs,
                                    TCM_SETITEMW,
                                    Some(WPARAM(selected as usize)),
                                    Some(LPARAM((&item as *const TCITEMW) as isize)),
                                );
                            }
                        }
                        let index = match state.tabs.iter().position(|p| p == path) {
                            Some(index) => index,
                            None => {
                                let mut text =
                                    wide(&path.file_name().unwrap_or_default().to_string_lossy());
                                let item = TCITEMW {
                                    mask: TCIF_TEXT,
                                    pszText: PWSTR(text.as_mut_ptr()),
                                    ..Default::default()
                                };
                                let index = state.tabs.len();
                                SendMessageW(
                                    tabs,
                                    TCM_INSERTITEMW,
                                    Some(WPARAM(index)),
                                    Some(LPARAM((&item as *const TCITEMW) as isize)),
                                );
                                state.tabs.push(path.clone());
                                index
                            }
                        };
                        SendMessageW(tabs, TCM_SETCURSEL, Some(WPARAM(index)), None);
                    }
                    let name = state
                        .path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    state.status = if state.path.as_ref().is_some_and(|p| is_pdf(p)) {
                        format!(
                            "{name}   ·   Page {} of {}",
                            state.page + 1,
                            frame.page_count
                        )
                    } else {
                        format!(
                            "{name}   ·   {} × {}",
                            frame.source_width, frame.source_height
                        )
                    };
                    let title = wide(&format!("{name} - Preview for Windows"));
                    let _ = SetWindowTextW(hwnd, PCWSTR(title.as_ptr()));
                    state.frame = Some(frame);
                    if let Some(graphics) = state.graphics.as_mut() {
                        graphics.bitmap = None;
                    }
                }
                Err(error) => {
                    state.render_failed = error != "No more images in this direction.";
                    state.status = format!("{error} Previous view retained.");
                    state.slideshow = None;
                    if error == crate::pdf::PASSWORD_REQUIRED {
                        password_to_show =
                            Some((completed.generation, completed.path, completed.page));
                        state.due = None;
                    }
                    if let Some((path, page)) = &state.displayed {
                        if state.path.as_ref() != Some(path) {
                            if let Some((_, zoom, pan)) = state.views.get(path) {
                                state.zoom = *zoom;
                                state.pan = *pan;
                            }
                        }
                        state.path = Some(path.clone());
                        state.page = *page;
                    }
                }
            }
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        if state.painted && state.due.is_some_and(|due| Instant::now() >= due) {
            state.due = None;
            schedule(hwnd, state, 0);
        }
        if !state.pending && state.slideshow.is_some_and(|due| Instant::now() >= due) {
            if state.path.as_ref().is_some_and(|p| is_pdf(p))
                && state
                    .frame
                    .as_ref()
                    .is_some_and(|f| state.page + 1 >= f.page_count)
            {
                state.slideshow = None;
                state.status = "End of slideshow.".into();
            } else {
                state.slideshow = Some(Instant::now() + Duration::from_secs(3));
                advance = true;
            }
        }
    });
    if let Some((generation, fields)) = fields_to_show {
        fill_field(hwnd, generation, fields);
    }
    if let Some(output) = output_to_open {
        open(hwnd, output);
    }
    if let Some((generation, path, page)) = password_to_show {
        password_prompt(hwnd, generation, path, page);
    }
    if advance {
        navigate(hwnd, 1);
    }
    update_controls(hwnd);
}

unsafe fn password_prompt(hwnd: HWND, generation: u64, path: PathBuf, page: u32) {
    let retry = STATE.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_some_and(|s| s.password_attempts.get(&path).copied().unwrap_or(0) > 0)
    });
    let answer = input_with_password(
        hwnd,
        if retry {
            "That password did not unlock this PDF"
        } else {
            "Open password-protected PDF"
        },
        &[("Password", String::new())],
        true,
    );
    let Some(mut values) = answer else {
        STATE.with(|cell| {
            if let Some(state) = cell.borrow_mut().as_mut() {
                state.password_attempts.remove(&path);
                state.render_failed = state.frame.is_none();
                state.status = "PDF opening cancelled. The previous document is unchanged.".into();
            }
        });
        let _ = InvalidateRect(Some(hwnd), None, false);
        return;
    };
    if values.is_empty() {
        return;
    }
    let password = values.remove(0);
    let mut rect = RECT::default();
    if GetClientRect(hwnd, &mut rect).is_err() {
        return;
    }
    STATE.with(|cell| {
        if let Some(state) = cell.borrow_mut().as_mut() {
            if state.generation != generation {
                return;
            }
            *state.password_attempts.entry(path.clone()).or_default() += 1;
            state.generation = state.generation.wrapping_add(1);
            state.pending = true;
            state.render_failed = false;
            state.path = Some(path.clone());
            state.page = page;
            state.status = "Unlocking PDF...".into();
            let request = Request {
                generation: state.generation,
                path,
                page,
                delta: 0,
                width: (rect.right.max(1) as u32).min(4096),
                height: ((rect.bottom - 128).max(1) as u32).min(4096),
                sessions: state.sessions.clone(),
            };
            if state.sender.send(Job::Password(request, password)).is_err() {
                state.pending = false;
                state.status = "The PDF worker stopped.".into();
            }
        }
    });
    let _ = InvalidateRect(Some(hwnd), None, false);
}

unsafe fn update_controls(hwnd: HWND) {
    let Some((ready, pdf, saving, previous, next)) = STATE.with(|cell| {
        cell.borrow().as_ref().map(|s| {
            let ready = s.frame.is_some() && !s.pending && !s.render_failed;
            let pdf = s.path.as_ref().is_some_and(|p| is_pdf(p));
            (
                ready,
                pdf,
                s.exporting,
                !pdf || s.page > 0,
                !pdf || s.frame.as_ref().is_some_and(|f| s.page + 1 < f.page_count),
            )
        })
    }) else {
        return;
    };
    let current = (ready, pdf, saving, previous, next);
    if CONTROL_STATE.with(|cell| {
        let mut old = cell.borrow_mut();
        if *old == Some(current) {
            true
        } else {
            *old = Some(current);
            false
        }
    }) {
        return;
    }
    let menu = GetMenu(hwnd);
    for id in [
        PREVIOUS,
        NEXT,
        FIT,
        ZOOM_IN,
        ZOOM_OUT,
        ROTATE,
        CROP,
        SAVE,
        FLIP,
        RESIZE,
        TEXT,
        FIND,
        EXTRACT,
        MERGE,
        DELETE,
        FORM,
        BACKGROUND,
        PRINT,
        BATCH,
        BATCH_SELECTED,
        MOVE,
        INSERT,
        SLIDESHOW,
        INFO,
        SIGN_SAVE,
        SIGN_PLACE,
        200,
        201,
        202,
        203,
        204,
        205,
        206,
        207,
        208,
    ] {
        let enabled = ready
            && match id {
                FLIP | RESIZE | BACKGROUND | BATCH | BATCH_SELECTED => !pdf,
                FIND | EXTRACT | MERGE | DELETE | FORM | MOVE | INSERT => pdf,
                PREVIOUS => previous,
                NEXT => next,
                SAVE | PRINT => !saving,
                _ => true,
            };
        if let Ok(button) = GetDlgItem(Some(hwnd), id as i32) {
            let _ = EnableWindow(button, enabled);
        }
        let _ = EnableMenuItem(
            menu,
            id as u32,
            MF_BYCOMMAND | if enabled { MF_ENABLED } else { MF_GRAYED },
        );
    }
}

unsafe fn fill_field(hwnd: HWND, generation: u64, fields: Vec<crate::pdf::FormField>) {
    let fields: Vec<_> = fields
        .into_iter()
        .filter(|f| !f.read_only && !matches!(f.kind, crate::pdf::FormFieldKind::Unsupported))
        .collect();
    if fields.is_empty() {
        MessageBoxW(Some(hwnd),w!("No editable supported fields on this page. Use Markup > Text box for a non-form PDF."),w!("Fill a form"),MB_OK);
        return;
    }
    let Ok(menu) = CreatePopupMenu() else {
        return;
    };
    for (index, field) in fields.iter().enumerate() {
        let text = wide(&format!(
            "{}: {}",
            if field.name.is_empty() {
                "Unnamed field"
            } else {
                &field.name
            },
            field.value
        ));
        let _ = AppendMenuW(menu, MF_STRING, index + 1, PCWSTR(text.as_ptr()));
    }
    let mut point = POINT::default();
    let _ = GetCursorPos(&mut point);
    let chosen = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_NONOTIFY,
        point.x,
        point.y,
        None,
        hwnd,
        None,
    )
    .0;
    let _ = DestroyMenu(menu);
    let Some(field) = fields
        .get(chosen.saturating_sub(1) as usize)
        .filter(|_| chosen > 0)
    else {
        return;
    };
    let label = if matches!(field.kind, crate::pdf::FormFieldKind::Checkbox) {
        "Checked: true or false"
    } else {
        &field.name
    };
    let Some(values) = input(hwnd, "Fill PDF field", &[(label, field.value.clone())]) else {
        return;
    };
    let Some(value) = values.first() else {
        return;
    };
    if matches!(field.kind, crate::pdf::FormFieldKind::Checkbox)
        && value != "true"
        && value != "false"
    {
        MessageBoxW(
            Some(hwnd),
            w!("Enter true or false for this checkbox."),
            w!("Invalid value"),
            MB_OK,
        );
        return;
    }
    STATE.with(|cell| {
        if let Some(state) = cell.borrow_mut().as_mut() {
            if state.generation != generation {
                return;
            }
            if let Some(path) = state.path.clone() {
                let edits = state.sessions.entry(path).or_default();
                edits.pdf.push(PdfEdit::FillField {
                    page: field.page,
                    annotation_index: field.annotation_index,
                    value: value.clone(),
                });
                edits.dirty = true;
                schedule(hwnd, state, 0);
            }
        }
    });
}

unsafe fn clipboard(hwnd: HWND, text: &str) -> std::result::Result<(), String> {
    use windows::Win32::System::{DataExchange::*, Memory::*};
    let text = wide(text);
    OpenClipboard(Some(hwnd)).map_err(|_| "Clipboard is busy. Try again.".to_string())?;
    let result = (|| -> Result<()> {
        let memory = GlobalAlloc(GMEM_MOVEABLE, text.len() * 2)?;
        let destination = GlobalLock(memory);
        if destination.is_null() {
            let _ = GlobalFree(Some(memory));
            return Err(Error::from_thread());
        }
        std::ptr::copy_nonoverlapping(text.as_ptr(), destination.cast::<u16>(), text.len());
        let _ = GlobalUnlock(memory);
        if let Err(error) =
            EmptyClipboard().and_then(|_| SetClipboardData(13, Some(HANDLE(memory.0))).map(|_| ()))
        {
            let _ = GlobalFree(Some(memory));
            return Err(error);
        }
        Ok(())
    })();
    let _ = CloseClipboard();
    result.map_err(|e| format!("Could not copy text: {e}"))
}

impl Graphics {
    unsafe fn new(hwnd: HWND, width: u32, height: u32) -> Result<Self> {
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

unsafe fn paint(hwnd: HWND) {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pdf_extension_is_case_insensitive() {
        assert!(is_pdf(Path::new("sample.PDF")));
        assert!(!is_pdf(Path::new("sample.png")));
    }
    #[test]
    fn file_picker_handles_single_and_multiple_files() {
        assert_eq!(
            picker_paths(&wide("C:\\photos\\one.png\0")),
            vec![PathBuf::from("C:\\photos\\one.png")]
        );
        assert_eq!(
            picker_paths(&wide("C:\\photos\0one.png\0two.jpg\0")),
            vec![
                PathBuf::from("C:\\photos").join("one.png"),
                PathBuf::from("C:\\photos").join("two.jpg")
            ]
        );
    }
    #[test]
    fn signed_mouse_coordinates_survive_negative_positions() {
        assert_eq!(
            point(LPARAM(((20u32 << 16) | (-12i16 as u16 as u32)) as isize)),
            (-12.0, 20.0)
        );
    }
    #[test]
    fn sibling_navigation_filters_and_orders_images() {
        let folder = std::env::temp_dir().join(format!(
            "preview-shell-nav-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&folder).unwrap();
        for name in ["B.jpg", "a.PNG", "ignore.txt"] {
            std::fs::write(folder.join(name), b"fixture").unwrap();
        }
        assert_eq!(
            sibling(&folder.join("a.PNG"), 1).unwrap(),
            folder.join("B.jpg")
        );
        assert!(sibling(&folder.join("a.PNG"), -1).is_err());
        std::fs::remove_dir_all(folder).unwrap();
    }
}
