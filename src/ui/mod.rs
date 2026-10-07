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
//! | bench | Benchmark scenarios, only when their variables are set |
//! | render | Direct2D device context, fonts, drawing helpers |
//! | theme | Light, dark, and contrast colors; DWM attributes |
//! | menu | Popup menus |
//! | sheet | In-window dialogs with EDIT text fields |
//! | a11y | UI Automation through AccessKit |
//! | files | File dialogs, clipboard, saved signature |
//! | text | Text selection, copy, and image search over text layers |
mod a11y;
mod actions;
mod app;
mod bench;
mod cache;
mod commands;
mod disk;
mod document;
mod files;
mod menu;
mod paint;
mod render;
mod sheet;
mod sidebar;
// W2-2: tested core only; the shell does not call it yet.
#[allow(dead_code)]
mod text;
mod theme;
mod view;
mod widgets;
mod window;
mod worker;

pub use window::run;
