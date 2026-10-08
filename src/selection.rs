//! The files selected in an Explorer window or on the desktop, read from the
//! shell's own view objects. PowerToys Peek reads the selection the same way:
//! https://github.com/microsoft/PowerToys/blob/main/src/modules/peek/Peek.UI/Helpers/FileExplorerHelper.cs

use std::path::PathBuf;
use windows::core::Interface;
use windows::Win32::{
    Foundation::HWND,
    System::{
        Com::{CoCreateInstance, CoTaskMemFree, IDispatch, IServiceProvider, CLSCTX_ALL},
        Variant::{VARIANT, VT_I4},
    },
    UI::{
        Shell::{
            IFolderView2, IShellBrowser, IShellItemArray, IShellWindows, IWebBrowserApp, ShellWindows,
            CSIDL_DESKTOP, SID_STopLevelBrowser, SIGDN_FILESYSPATH, SVGIO_ALLVIEW, SVGIO_FLAG_VIEWORDER,
            SVGIO_SELECTION, SWC_DESKTOP, SWFO_NEEDDISPATCH, _SVGIO,
        },
        WindowsAndMessaging::IsWindowVisible,
    },
};

/// Returns the item to show and the items to step through, in view order.
/// With 2 or more items selected, those are the items to step through; with
/// 1, every item in the view. Returns None when no file system item is
/// selected or the shell does not answer, for example an elevated
/// Explorer. Call on a COM apartment thread.
pub unsafe fn read(window: HWND, desktop: bool) -> Option<(PathBuf, Vec<PathBuf>)> {
    let windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL).ok()?;
    let browser = if desktop { desktop_browser(&windows)? } else { explorer_browser(&windows, window)? };
    let view: IFolderView2 = browser.QueryActiveShellView().ok()?.cast().ok()?;
    let selected = paths(&view, SVGIO_SELECTION);
    let path = selected.first()?.clone();
    // ponytail: one display-name call per file in the view; read siblings lazily if huge folders delay the first frame.
    let siblings = if selected.len() > 1 { selected } else { paths(&view, SVGIO_ALLVIEW) };
    Some((path, siblings))
}

unsafe fn explorer_browser(windows: &IShellWindows, window: HWND) -> Option<IShellBrowser> {
    let mut first = None;
    for index in 0..windows.Count().ok()? {
        let Ok(item) = windows.Item(&int_variant(index)) else {
            continue;
        };
        let frame = item.cast::<IWebBrowserApp>().and_then(|app| app.HWND());
        if frame.map(|h| h.0) != Ok(window.0 as isize) {
            continue;
        }
        let Some(browser) = top_browser(&item) else {
            continue;
        };
        // Windows 11 tabs share one frame window; only the active tab's window is visible (unverified).
        if browser.GetWindow().is_ok_and(|tab| IsWindowVisible(tab).as_bool()) {
            return Some(browser);
        }
        first.get_or_insert(browser);
    }
    first
}

unsafe fn desktop_browser(windows: &IShellWindows) -> Option<IShellBrowser> {
    let mut hwnd = 0;
    let desktop = windows
        .FindWindowSW(&int_variant(CSIDL_DESKTOP as i32), &VARIANT::default(), SWC_DESKTOP, &mut hwnd, SWFO_NEEDDISPATCH)
        .ok()?;
    top_browser(&desktop)
}

unsafe fn top_browser(object: &IDispatch) -> Option<IShellBrowser> {
    object.cast::<IServiceProvider>().ok()?.QueryService(&SID_STopLevelBrowser).ok()
}

/// Files and folders; virtual items such as This PC have no path and are skipped.
unsafe fn paths(view: &IFolderView2, which: _SVGIO) -> Vec<PathBuf> {
    let Ok(items) = view.Items::<IShellItemArray>(_SVGIO(which.0 | SVGIO_FLAG_VIEWORDER.0)) else {
        return Vec::new();
    };
    (0..items.GetCount().unwrap_or(0))
        .filter_map(|index| {
            let name = items.GetItemAt(index).ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
            let path = PathBuf::from(name.to_hstring().to_os_string());
            CoTaskMemFree(Some(name.0 as _));
            Some(path)
        })
        .collect()
}

fn int_variant(value: i32) -> VARIANT {
    let mut variant = VARIANT::default();
    unsafe {
        let inner = &mut *variant.Anonymous.Anonymous;
        inner.vt = VT_I4;
        inner.Anonymous.lVal = value;
    }
    variant
}
