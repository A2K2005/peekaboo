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

mod app;
mod commands;
mod dialog;
mod files;
mod window;
mod worker;
use app::*;
use commands::*;
use dialog::*;
use files::*;
use window::*;
use worker::*;
use worker::Event;
pub use window::run;

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
