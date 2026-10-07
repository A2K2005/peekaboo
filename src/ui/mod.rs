//! The native UI: one window thread, Direct2D drawing, custom chrome, and
//! two workers (document and task).
//!
//! | Module | Role |
//! | --- | --- |
//! | window | Window class, message loop, custom title bar, input routing |
//! | app | Window-thread state, tabs, render scheduling, worker results |
//! | worker | Document worker (PDFium, WIC viewing), task worker, render queue |
//! | commands | Command table: labels, icons, shortcuts, access keys, menus |
//! | actions | Runs commands |
//! | widgets | Widget list, layout, hit testing, focus order, access keys |
//! | paint | Draws the chrome, sidebar, sheets, focus, keytips, tooltips |
//! | document | Document view: PDF tiles or an image, scrolling, zoom, pointer input |
//! | view | Page layout, tile grid, and zoom math |
//! | cache | Byte-budgeted LRU cache for tiles and pre-decoded images |
//! | sidebar | Thumbnails, contents, and notes lists |
//! | organize | Thumbnail drag to reorder or drag out, page moves, page inserts |
//! | drop | Files dropped on the window (OLE drop target) |
//! | bench | Benchmark scenarios, only when their variables are set |
//! | render | Direct2D device context, fonts, drawing helpers |
//! | theme | Light, dark, and contrast colors; DWM attributes |
//! | menu | Popup menus |
//! | sheet | In-window dialogs with EDIT text fields |
//! | a11y | UI Automation through AccessKit |
//! | files | File dialogs, clipboard, saved signature |
//! | imagetools | Image crop handles, resize, export, batch, and background removal results |
//! | forms | Form fields, signature pad and placement, text markup, text boxes |
//! | text | Text selection, copy, and image search over text layers |
//! | findbar | Find bar with live search |
//! | infobar | Info bar and toasts, the live regions for messages |
//! | empty | Empty window: New from clipboard and recent files |
//! | pan | Space+drag panning |
mod a11y;
mod actions;
mod app;
mod bench;
mod cache;
mod commands;
mod disk;
mod document;
mod drop;
mod empty;
mod files;
mod findbar;
mod forms;
mod imagetools;
mod infobar;
mod menu;
mod organize;
mod paint;
mod pan;
mod render;
mod sheet;
mod sidebar;
mod text;
mod theme;
mod view;
mod widgets;
mod window;
mod worker;

pub use window::{prepare, run, standby};

use std::{cell::Cell, path::PathBuf};
use windows::Win32::{
    Foundation::HWND,
    System::Threading::{AttachThreadInput, GetCurrentThreadId},
    UI::WindowsAndMessaging::{
        DestroyWindow, GetForegroundWindow, GetWindowThreadProcessId, IsWindowVisible, KillTimer,
        SetForegroundWindow, SetTimer, ShowWindow, SW_HIDE, SW_SHOW,
    },
};

thread_local! {
    /// This thread's main window, while it exists.
    static WINDOW: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    /// The window shows a quick view, so Space and Esc hide it.
    static PEEKING: Cell<bool> = const { Cell::new(false) };
}

/// Shows `path` as a quick view in this thread's main window and brings it
/// forward. `siblings` are the files to step through, in Explorer view order.
pub fn peek(path: PathBuf, _siblings: Vec<PathBuf>) {
    let hwnd = WINDOW.get();
    if hwnd.is_invalid() {
        return;
    }
    unsafe {
        app::with_state(|s| app::add_tabs(s, std::slice::from_ref(&path)));
        app::open(hwnd, path);
        PEEKING.set(true);
        SetTimer(Some(hwnd), 1, 10, None);
        let _ = ShowWindow(hwnd, SW_SHOW);
        // A background process may not take the foreground; a thread that
        // shares the foreground thread's input state may.
        let current = GetCurrentThreadId();
        let foreground = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let attached = foreground != current && AttachThreadInput(current, foreground, true).as_bool();
        let _ = SetForegroundWindow(hwnd);
        if attached {
            let _ = AttachThreadInput(current, foreground, false);
        }
    }
}

/// Hides the window when it shows a quick view and no sheet waits for an answer.
unsafe fn dismiss(hwnd: HWND) -> bool {
    if !PEEKING.get() || app::with_state(|s| s.sheet.is_none()) != Some(true) {
        return false;
    }
    PEEKING.set(false);
    let _ = KillTimer(Some(hwnd), 1);
    let _ = ShowWindow(hwnd, SW_HIDE);
    true
}

/// Destroys the hidden main window when it holds documents and no unsaved
/// work, which frees them; `standby` then returns. Returns false while the
/// window is visible or busy, so the caller tries again later.
pub fn release() -> bool {
    let hwnd = WINDOW.get();
    unsafe {
        if hwnd.is_invalid() {
            return true;
        }
        if IsWindowVisible(hwnd).as_bool() {
            return false;
        }
        match app::with_state(|s| (s.path.is_none() && s.tabs.is_empty(), s.window_close_state())) {
            Some((true, _)) => true,
            Some((false, (false, false))) => {
                app::with_state(|s| s.cleanup_snapshots());
                let _ = DestroyWindow(hwnd);
                true
            }
            _ => false,
        }
    }
}
