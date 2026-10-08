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
//! | files | File dialogs, clipboard, saved signatures |
//! | imagetools | Image crop handles, resize, export, batch, and background removal results |
//! | forms | Form fields, signature pad and placement, text markup, text boxes |
//! | marks | Mark style menus; selecting, moving, resizing, and deleting marks |
//! | text | Text selection, copy, and image search over text layers |
//! | findbar | Find bar with live search |
//! | infobar | Info bar and toasts, the live regions for messages |
//! | empty | Empty window: New from clipboard and recent files |
//! | pan | Space+drag panning |
//! | quickview | Quick view: peek window, hover strip, index sheet, handoff to the editor |
//! | textview | Quick view of text files |
//! | handler | Quick view through system preview handlers |
//! | infocard | Quick view's info card for folders and files with no preview |
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
mod handler;
mod imagetools;
mod infobar;
mod infocard;
mod marks;
mod menu;
mod organize;
mod paint;
mod pan;
mod quickview;
mod render;
mod sheet;
mod sidebar;
mod text;
mod textview;
mod theme;
mod view;
mod widgets;
mod window;
mod worker;

pub use window::{prepare, run, standby};

use std::{cell::Cell, path::PathBuf};
use windows::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{DestroyWindow, IsWindowVisible},
};

thread_local! {
    /// This thread's main window, while it exists.
    static WINDOW: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
}

/// Shows `path` in Quick view. Arrows move through `siblings`: the
/// selection when 2 or more files are selected, else the folder's files in
/// Explorer view order. An empty list means the folder's files by name.
/// Callable from any thread; the window thread does the work. A call before
/// the window exists waits for it.
pub fn peek(path: PathBuf, siblings: Vec<PathBuf>) {
    quickview::post(path, siblings);
}

/// Destroys the hidden main window when it has no unsaved work, which frees
/// its documents, tiles, and Direct2D device; `standby` then returns.
/// Returns false while the window is visible or busy, so the caller tries
/// again later.
pub fn release() -> bool {
    let hwnd = WINDOW.get();
    unsafe {
        if hwnd.is_invalid() {
            return true;
        }
        if IsWindowVisible(hwnd).as_bool() {
            return false;
        }
        match app::with_state(|s| s.window_close_state()) {
            Some((false, false)) => {
                app::with_state(|s| s.cleanup_snapshots());
                let _ = DestroyWindow(hwnd);
                true
            }
            _ => false,
        }
    }
}
