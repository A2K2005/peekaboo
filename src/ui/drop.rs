//! Files dragged onto the window from Explorer or another app. Over the
//! page thumbnails, PDFs and images go in at the insertion line; anywhere
//! else, the files open as tabs.
use super::{
    app::{add_tabs, invalidate, open, with_state},
    organize,
    worker::WM_APP_WAKE,
};
use std::{cell::Cell, path::PathBuf};
use windows::{
    core::{implement, Ref, Result},
    Win32::{
        Foundation::{HWND, LPARAM, POINT, POINTL, WPARAM},
        Graphics::Gdi::ScreenToClient,
        System::{
            Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL},
            Ole::{
                IDropTarget, IDropTarget_Impl, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop, CF_HDROP, DROPEFFECT,
                DROPEFFECT_COPY, DROPEFFECT_NONE,
            },
            SystemServices::MODIFIERKEYS_FLAGS,
        },
        UI::{
            Shell::{DragAcceptFiles, DragQueryFileW, HDROP},
            WindowsAndMessaging::PostMessageW,
        },
    },
};

#[implement(IDropTarget)]
struct Target {
    hwnd: HWND,
    /// The drag carries files, and it is not this window's own drag out.
    accept: Cell<bool>,
    /// Some of the files can become pages.
    pages: Cell<bool>,
}

impl Target {
    /// Sets the insertion line and the drop effect. Returns the gap the
    /// files would go into, or None when they would open as tabs.
    fn over(&self, pt: &POINTL, effect: *mut DROPEFFECT) -> Option<u32> {
        let mut point = POINT { x: pt.x, y: pt.y };
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut point);
        }
        let point = (point.x as f32, point.y as f32);
        // A sheet waits for an answer; opening or editing files under it
        // would change what it asks about.
        let accept = self.accept.get() && with_state(|s| s.sheet.is_none()) == Some(true);
        let gap = if accept && self.pages.get() { with_state(|s| organize::drop_gap(s, point)).flatten() } else { None };
        self.show(gap.map(|_| point));
        if !effect.is_null() {
            unsafe {
                let allowed = (*effect).0 & DROPEFFECT_COPY.0 != 0;
                *effect = if accept && allowed { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
            }
        }
        gap
    }

    fn show(&self, at: Option<(f32, f32)>) {
        if with_state(|s| std::mem::replace(&mut s.organize.drop_over, at) != at) == Some(true) {
            unsafe { invalidate(self.hwnd) };
        }
    }
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(&self, data: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let paths = data.as_ref().map(|d| unsafe { paths(d) }).unwrap_or_default();
        self.accept.set(!paths.is_empty() && !organize::dragging_out());
        self.pages.set(paths.iter().any(|p| organize::can_insert(p)));
        self.over(pt, effect);
        Ok(())
    }

    fn DragOver(&self, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        self.over(pt, effect);
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        self.show(None);
        Ok(())
    }

    fn Drop(&self, data: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let accept = self.accept.get();
        let gap = self.over(pt, effect);
        self.show(None);
        let paths = data.as_ref().map(|d| unsafe { paths(d) }).unwrap_or_default();
        if accept && !paths.is_empty() && with_state(|s| s.organize.dropped = Some((paths, gap))).is_some() {
            unsafe {
                let _ = PostMessageW(Some(self.hwnd), WM_APP_WAKE, WPARAM(0), LPARAM(0));
            }
        }
        Ok(())
    }
}

unsafe fn paths(data: &IDataObject) -> Vec<PathBuf> {
    let format = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let Ok(mut medium) = data.GetData(&format) else {
        return Vec::new();
    };
    let paths = hdrop_paths(HDROP(medium.u.hGlobal.0));
    ReleaseStgMedium(&mut medium);
    paths
}

/// The file paths in a CF_HDROP block.
pub(super) unsafe fn hdrop_paths(drop: HDROP) -> Vec<PathBuf> {
    (0..DragQueryFileW(drop, u32::MAX, None))
        .filter_map(|index| {
            let length = DragQueryFileW(drop, index, None) as usize;
            let mut path = vec![0u16; length + 1];
            DragQueryFileW(drop, index, Some(&mut path));
            (length > 0).then(|| PathBuf::from(String::from_utf16_lossy(&path[..length])))
        })
        .collect()
}

/// Inserts the dropped files as pages at `gap`, or opens them as tabs.
pub(super) unsafe fn finish(hwnd: HWND, paths: Vec<PathBuf>, gap: Option<u32>) {
    match gap {
        Some(gap) => organize::insert_files(hwnd, &paths, gap),
        None => {
            with_state(|s| add_tabs(s, &paths));
            if let Some(path) = paths.into_iter().next() {
                open(hwnd, path);
            }
        }
    }
}

/// An OLE drop target reports the pointer during the drag, which
/// WM_DROPFILES does not. If registration fails, WM_DROPFILES still opens
/// dropped files.
pub(super) unsafe fn register(hwnd: HWND) {
    let target: IDropTarget = Target { hwnd, accept: Cell::new(false), pages: Cell::new(false) }.into();
    if RegisterDragDrop(hwnd, &target).is_err() {
        DragAcceptFiles(hwnd, true);
    }
}

pub(super) unsafe fn revoke(hwnd: HWND) {
    let _ = RevokeDragDrop(hwnd);
}
