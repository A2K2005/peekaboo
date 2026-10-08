//! System preview handlers in Quick view, as Explorer's preview pane hosts
//! them: Office documents, media, and any other type with a registered
//! IPreviewHandler. https://learn.microsoft.com/windows/win32/shell/preview-handlers
//!
//! Each file gets its own thread, which owns a hidden child window of the
//! Quick view window, so a slow handler never blocks the window thread.
//! PowerToys Peek (MIT) hosts handlers the same way, out of process and
//! initialized with a stream, then an item, then a path:
//! https://github.com/microsoft/PowerToys/tree/main/src/modules/peek
use super::{widgets::Rect, worker::WM_APP_WAKE};
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicIsize, Ordering},
        Arc,
    },
};
use windows::{
    core::{w, Error, Interface, Result, GUID, HSTRING, PCWSTR, PWSTR},
    Win32::{
        Foundation::{E_ABORT, HWND, LPARAM, LRESULT, RECT, WPARAM},
        System::{
            Com::{
                CLSIDFromString, CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED,
                STGM_READ, STGM_SHARE_DENY_NONE,
            },
            LibraryLoader::GetModuleHandleW,
        },
        UI::{
            Shell::{
                AssocQueryStringW, IInitializeWithItem, IPreviewHandler, IShellItem,
                PropertiesSystem::{IInitializeWithFile, IInitializeWithStream},
                SHCreateItemFromParsingName, SHCreateStreamOnFileEx, ASSOCF_INIT_DEFAULTTOSTAR, ASSOCSTR_SHELLEXTENSION,
            },
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW, IsWindow,
                PostMessageW, PostQuitMessage, RegisterClassW, SetWindowPos, TranslateMessage, MSG, SWP_ASYNCWINDOWPOS,
                SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, WM_CLOSE, WM_DESTROY, WM_SIZE, WNDCLASSW,
                WS_CHILD, WS_CLIPCHILDREN, WS_EX_NOPARENTNOTIFY,
            },
        },
    },
};

const CLASS: PCWSTR = w!("PeekabooHandlerHost");

thread_local! {
    static HANDLER: RefCell<Option<IPreviewHandler>> = const { RefCell::new(None) };
    /// Set while the handler loads. A close then waits until it returns,
    /// because Unload during DoPreview reenters the handler.
    static BUSY: Cell<bool> = const { Cell::new(false) };
    static CLOSING: Cell<bool> = const { Cell::new(false) };
}

/// The CLSID of the preview handler registered for `path`'s type.
pub(super) fn find(path: &Path) -> Option<GUID> {
    let extension = HSTRING::from(format!(".{}", path.extension()?.to_string_lossy()));
    let mut buffer = [0u16; 64];
    let mut length = buffer.len() as u32;
    unsafe {
        AssocQueryStringW(
            ASSOCF_INIT_DEFAULTTOSTAR,
            ASSOCSTR_SHELLEXTENSION,
            &extension,
            w!("{8895b1c6-b41f-4c1c-a562-0d564250836f}"),
            Some(PWSTR(buffer.as_mut_ptr())),
            &mut length,
        )
        .ok()
        .ok()?;
        CLSIDFromString(PCWSTR(buffer.as_ptr())).ok()
    }
}

#[derive(Default)]
struct Shared {
    window: AtomicIsize,
    ready: AtomicBool,
    closed: AtomicBool,
}

/// A preview handler running for one file. Dropping it unloads the handler.
pub(super) struct Host {
    shared: Arc<Shared>,
    placed: Option<(Rect, bool)>,
}

impl Host {
    /// Starts `clsid` for `path` in a hidden child of `parent` at `rect`.
    pub(super) fn start(parent: HWND, clsid: GUID, path: PathBuf, rect: Rect) -> Self {
        let shared = Arc::new(Shared::default());
        let thread = shared.clone();
        let parent = parent.0 as isize;
        let rect = RECT { left: rect.x0 as i32, top: rect.y0 as i32, right: rect.x1 as i32, bottom: rect.y1 as i32 };
        std::thread::spawn(move || unsafe { run(HWND(parent as *mut _), clsid, &path, rect, &thread) });
        Self { shared, placed: None }
    }

    /// True once the handler has drawn its preview.
    pub(super) fn ready(&self) -> bool {
        self.shared.ready.load(Ordering::SeqCst)
    }

    /// Moves the preview to `rect`, shown when `visible` and ready.
    pub(super) unsafe fn place(&mut self, rect: Rect, visible: bool) {
        let window = self.shared.window.load(Ordering::SeqCst);
        let visible = visible && self.ready();
        if window == 0 || self.placed == Some((rect, visible)) {
            return;
        }
        self.placed = Some((rect, visible));
        let show = if visible { SWP_SHOWWINDOW } else { SWP_HIDEWINDOW };
        // Async: the handler thread may be inside a slow call into the handler.
        let _ = SetWindowPos(
            HWND(window as *mut _),
            None,
            rect.x0.round() as i32,
            rect.y0.round() as i32,
            rect.width().round() as i32,
            rect.height().round() as i32,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS | show,
        );
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::SeqCst);
        let window = self.shared.window.load(Ordering::SeqCst);
        if window != 0 {
            unsafe {
                let _ = PostMessageW(Some(HWND(window as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }
}

unsafe fn run(parent: HWND, clsid: GUID, path: &Path, rect: RECT, shared: &Shared) {
    let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
    let instance = GetModuleHandleW(None).unwrap_or_default();
    // A second registration fails, and the first one serves.
    RegisterClassW(&WNDCLASSW { hInstance: instance.into(), lpszClassName: CLASS, lpfnWndProc: Some(host_proc), ..Default::default() });
    let host = CreateWindowExW(
        WS_EX_NOPARENTNOTIFY,
        CLASS,
        None,
        WS_CHILD | WS_CLIPCHILDREN,
        rect.left,
        rect.top,
        rect.right - rect.left,
        rect.bottom - rect.top,
        Some(parent),
        None,
        Some(instance.into()),
        None,
    );
    if let Ok(host) = host {
        shared.window.store(host.0 as isize, Ordering::SeqCst);
        BUSY.set(true);
        let handler = if shared.closed.load(Ordering::SeqCst) { Err(Error::from(E_ABORT)) } else { preview(clsid, path, host) };
        BUSY.set(false);
        let alive = || IsWindow(Some(host)).as_bool();
        if (CLOSING.get() || shared.closed.load(Ordering::SeqCst) || handler.is_err()) && alive() {
            let _ = DestroyWindow(host);
        }
        match handler {
            Ok(handler) if alive() => {
                HANDLER.set(Some(handler.clone()));
                fit(host, &handler);
                shared.ready.store(true, Ordering::SeqCst);
                let _ = PostMessageW(Some(parent), WM_APP_WAKE, WPARAM(0), LPARAM(0));
                let mut message = MSG::default();
                while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            Ok(handler) => {
                let _ = handler.Unload();
            }
            Err(_) => {}
        }
    }
    if com {
        CoUninitialize();
    }
}

/// Loads and draws the preview into `host`.
unsafe fn preview(clsid: GUID, path: &Path, host: HWND) -> Result<IPreviewHandler> {
    // Out of process (prevhost.exe), as Explorer runs them: a crash in a
    // third-party handler then cannot end the resident process.
    let handler: IPreviewHandler = CoCreateInstance(&clsid, None, CLSCTX_LOCAL_SERVER)?;
    let file = HSTRING::from(path);
    if let Ok(init) = handler.cast::<IInitializeWithStream>() {
        // Share-deny-none: the file stays editable in its own app meanwhile.
        let stream = SHCreateStreamOnFileEx(&file, (STGM_READ | STGM_SHARE_DENY_NONE).0, 0, false, None)?;
        init.Initialize(&stream, STGM_READ.0)?;
    } else if let Ok(init) = handler.cast::<IInitializeWithItem>() {
        let item: IShellItem = SHCreateItemFromParsingName(&file, None)?;
        init.Initialize(&item, STGM_READ.0)?;
    } else {
        handler.cast::<IInitializeWithFile>()?.Initialize(&file, STGM_READ.0)?;
    }
    let mut rect = RECT::default();
    GetClientRect(host, &mut rect)?;
    if let Err(error) = handler.SetWindow(host, &rect).and_then(|()| handler.DoPreview()) {
        let _ = handler.Unload();
        return Err(error);
    }
    Ok(handler)
}

unsafe fn fit(host: HWND, handler: &IPreviewHandler) {
    let mut rect = RECT::default();
    if GetClientRect(host, &mut rect).is_ok() {
        let _ = handler.SetRect(&rect);
    }
}

unsafe extern "system" fn host_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_SIZE => {
            // A clone, so no borrow is held while the call pumps messages.
            if let Some(handler) = HANDLER.with_borrow(Clone::clone) {
                fit(hwnd, &handler);
            }
            LRESULT(0)
        }
        WM_CLOSE if BUSY.get() => {
            CLOSING.set(true);
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            if let Some(handler) = HANDLER.take() {
                let _ = handler.Unload();
            }
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}
