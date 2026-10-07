//! The native UI: one window thread, Direct2D drawing, custom chrome, and
//! two workers (document and task).
//!
//! | Module | Role |
//! | --- | --- |
//! | window | Window class, message loop, custom title bar, input routing |
//! | app | Window-thread state, tabs, render scheduling, worker results |
//! | worker | Document worker (PDFium, WIC viewing) and task worker |
//! | commands | Command table: labels, icons, shortcuts, access keys, menus |
//! | actions | Runs commands |
//! | widgets | Widget list, layout, hit testing, focus order, access keys |
//! | paint | Draws the chrome, sheets, focus, keytips, tooltips |
//! | document | Document view drawing and pointer input |
//! | render | Direct2D device context, fonts, drawing helpers |
//! | theme | Light, dark, and contrast colors; DWM attributes |
//! | menu | Popup menus |
//! | sheet | In-window dialogs with EDIT text fields |
//! | a11y | UI Automation through AccessKit |
//! | files | File dialogs, clipboard, saved signature |
mod a11y;
mod actions;
mod app;
mod commands;
mod document;
mod files;
mod menu;
mod paint;
mod render;
mod sheet;
mod theme;
mod widgets;
mod window;
mod worker;

pub use window::run;
