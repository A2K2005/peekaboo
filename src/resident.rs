//! The resident process: it starts at sign-in, watches for Space in Explorer
//! and on the desktop, and keeps one hidden window ready, so Quick view
//! opens at once.
//!
//! The keyboard hook runs on its own thread with a message loop, as
//! WH_KEYBOARD_LL requires, and does only window-class reads. Windows removes
//! a slow hook silently, so it is installed again every 10 minutes and after
//! sleep. https://learn.microsoft.com/windows/win32/winmsg/lowlevelkeyboardproc

use crate::integration::{self, Action, Command};
use std::cell::Cell;
use std::ffi::OsString;
use std::sync::{
    atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering},
    OnceLock,
};
use windows::core::{w, Error, Result, HSTRING, PCWSTR};
use windows::Win32::{
    Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, WPARAM},
    System::{
        LibraryLoader::GetModuleHandleW,
        Threading::{CreateMutexW, GetCurrentProcess, GetCurrentThreadId, SetProcessWorkingSetSize},
    },
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, RegisterHotKey, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
            KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, VIRTUAL_KEY, VK_CONTROL,
            VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT, VK_SPACE,
        },
        WindowsAndMessaging::{
            CallNextHookEx, CreateWindowExW, DefWindowProcW, GetClassNameW, GetForegroundWindow, GetGUIThreadInfo,
            GetMessageW, GetWindowThreadProcessId, KillTimer, PostMessageW, PostThreadMessageW, RegisterClassW,
            SetTimer, SetWindowsHookExW, UnhookWindowsHookEx, GUITHREADINFO, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT,
            LLKHF_INJECTED, MSG, PBT_APMRESUMEAUTOMATIC, WH_KEYBOARD_LL, WM_APP, WM_COPYDATA,
            WM_HOTKEY, WM_KEYDOWN, WM_KEYUP, WM_POWERBROADCAST, WM_SYSKEYUP, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW,
            WS_POPUP,
        },
    },
};

const CLASS: &str = "PreviewForWindowsResident";
/// wParam: the foreground window. lParam: 1 when a swallowed Space caused it.
const WM_APP_PEEK: u32 = WM_APP + 20;
const WM_APP_REINSTALL: u32 = WM_APP + 21;
const HOTKEY: i32 = 1;
const IDLE_TIMER: usize = 1;
const TEN_MINUTES: u32 = 10 * 60 * 1000;

static TARGET: AtomicIsize = AtomicIsize::new(0);
static HOOK_THREAD: AtomicU32 = AtomicU32::new(0);
static SPACE_DOWN: AtomicBool = AtomicBool::new(false);
/// Set when the hook took a Space press; its repeats and key-up are taken too.
static SWALLOW: AtomicBool = AtomicBool::new(false);
/// Event time of the last Space down, in ms.
static LAST_SPACE: AtomicU32 = AtomicU32::new(0);

thread_local! {
    /// Reading the selection makes cross-process COM calls, which dispatch
    /// messages, so a second trigger can arrive during the first.
    static READING: Cell<bool> = const { Cell::new(false) };
}

/// Runs until sign-out. A second resident process exits at once.
pub fn run() -> Result<()> {
    unsafe {
        // The handle lives as long as the process, which marks it as the resident one.
        let _mutex = CreateMutexW(None, false, w!("Local\\PreviewForWindowsResident"))?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return Ok(());
        }
        // Holding the editor's instance mutex makes `integration::hand_off`
        // in a double-click launch forward its files to this process's
        // hidden main window, which `standby` recreates after each release.
        let _instance = CreateMutexW(None, false, &HSTRING::from(format!("Local\\{}", integration::WINDOW_CLASS)))?;
        crate::ui::prepare()?;
        let instance = GetModuleHandleW(None)?;
        let class = HSTRING::from(CLASS);
        let wc = WNDCLASSW {
            hInstance: instance.into(),
            lpszClassName: PCWSTR(class.as_ptr()),
            lpfnWndProc: Some(wndproc),
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 {
            return Err(Error::from_thread());
        }
        // A hidden top-level window, not a message-only one: only top-level
        // windows receive the resume broadcast.
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            &class,
            w!(""),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        TARGET.store(hwnd.0 as isize, Ordering::Release);
        let _ = RegisterHotKey(Some(hwnd), HOTKEY, MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT, VK_SPACE.0 as u32);
        std::thread::Builder::new()
            .name("keyboard-hook".into())
            .spawn(hook_thread)
            .map_err(|_| Error::from_thread())?;
        loop {
            crate::ui::standby()?;
            // The window and its documents are gone; return their pages to Windows.
            let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
        }
    }
}

/// Sends `--peek path...` to the running resident process. Returns false
/// when none answers; the caller then opens the files itself.
pub fn forward(args: &[OsString]) -> bool {
    let command = integration::parse_args(args.iter().cloned());
    !command.paths.is_empty()
        && integration::find_window(CLASS).is_some_and(|hwnd| integration::forward(hwnd, &command).is_ok())
}

unsafe extern "system" fn wndproc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_APP_PEEK => {
            peek(HWND(wparam.0 as *mut _), lparam.0 != 0);
            LRESULT(0)
        }
        WM_HOTKEY => {
            peek(GetForegroundWindow(), false);
            LRESULT(0)
        }
        WM_COPYDATA => match integration::decode_copydata(lparam) {
            Some(Command { action: Action::Peek, paths }) if !paths.is_empty() => {
                show(paths[0].clone(), paths);
                LRESULT(1)
            }
            _ => LRESULT(0),
        },
        WM_TIMER if wparam.0 == IDLE_TIMER => {
            if crate::ui::release() {
                let _ = KillTimer(Some(hwnd), IDLE_TIMER);
            }
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            if wparam.0 == PBT_APMRESUMEAUTOMATIC as usize {
                let _ = PostThreadMessageW(HOOK_THREAD.load(Ordering::Acquire), WM_APP_REINSTALL, WPARAM(0), LPARAM(0));
            }
            LRESULT(1)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

/// Shows the selection of `window` if it is Explorer or the desktop. A Space
/// the hook took goes back to Explorer when nothing supported is selected.
unsafe fn peek(window: HWND, space: bool) {
    if READING.replace(true) {
        return;
    }
    let selection = shell_kind(window).and_then(|desktop| crate::selection::read(window, desktop));
    READING.set(false);
    if logging() {
        log(&format!("peek: space {space}, selection {:?}", selection.as_ref().map(|(path, siblings)| (path, siblings.len()))));
    }
    match selection {
        Some((path, siblings)) => show(path, siblings),
        None if space => resend_space(),
        None => {}
    }
}

/// Documents stay loaded for 10 minutes after the last Quick view.
unsafe fn show(path: std::path::PathBuf, siblings: Vec<std::path::PathBuf>) {
    let target = HWND(TARGET.load(Ordering::Acquire) as *mut _);
    SetTimer(Some(target), IDLE_TIMER, TEN_MINUTES, None);
    crate::ui::peek(path, siblings);
}

/// The hook skips injected keys, so this Space reaches Explorer.
unsafe fn resend_space() {
    let key = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VK_SPACE, dwFlags: flags, ..Default::default() } },
    };
    SendInput(&[key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)], std::mem::size_of::<INPUT>() as i32);
}

fn hook_thread() {
    unsafe {
        HOOK_THREAD.store(GetCurrentThreadId(), Ordering::Release);
        let mut hook = install();
        SetTimer(None, 0, TEN_MINUTES, None);
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            if matches!(message.message, WM_TIMER | WM_APP_REINSTALL) {
                if let Some(new) = install() {
                    if let Some(old) = hook.replace(new) {
                        let _ = UnhookWindowsHookEx(old);
                    }
                }
            }
        }
    }
}

unsafe fn install() -> Option<HHOOK> {
    let module = GetModuleHandleW(None).ok()?;
    SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), Some(module.into()), 0).ok()
}

unsafe extern "system" fn keyboard(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let key = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let injected = key.flags.0 & LLKHF_INJECTED.0 != 0;
        if key.vkCode == VK_SPACE.0 as u32 && logging() {
            log(&format!("hook: message {:#x}, injected {injected}, {}", wparam.0, focus_report()));
        }
        if key.vkCode != VK_SPACE.0 as u32 && !injected {
            // Another key means Space is not held, even if its key-up was missed.
            SPACE_DOWN.store(false, Ordering::Relaxed);
            SWALLOW.store(false, Ordering::Relaxed);
        } else if key.vkCode == VK_SPACE.0 as u32 && !injected {
            let message = wparam.0 as u32;
            if matches!(message, WM_KEYUP | WM_SYSKEYUP) {
                SPACE_DOWN.store(false, Ordering::Relaxed);
                if SWALLOW.swap(false, Ordering::Relaxed) {
                    return LRESULT(1);
                }
            } else {
                // Low-level hooks get no repeat flag; a down while down is a
                // repeat. Repeats come every 33 to 500 ms, so a longer gap
                // means the key-up was missed and this is a fresh press.
                let quiet = key.time.wrapping_sub(LAST_SPACE.swap(key.time, Ordering::Relaxed)) > 1000;
                let repeat = SPACE_DOWN.swap(true, Ordering::Relaxed) && !quiet;
                if !repeat {
                    SWALLOW.store(false, Ordering::Relaxed);
                }
                if SWALLOW.load(Ordering::Relaxed) {
                    return LRESULT(1);
                }
                if !repeat && message == WM_KEYDOWN && no_modifiers() {
                    let window = file_view();
                    let target = HWND(TARGET.load(Ordering::Acquire) as *mut _);
                    let posted = window.is_some_and(|window| {
                        PostMessageW(Some(target), WM_APP_PEEK, WPARAM(window.0 as usize), LPARAM(1)).is_ok()
                    });
                    if logging() {
                        log(&format!("space: {}, posted {posted}", focus_report()));
                    }
                    if posted {
                        SWALLOW.store(true, Ordering::Relaxed);
                        return LRESULT(1);
                    }
                }
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

unsafe fn no_modifiers() -> bool {
    [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN].iter().all(|key: &VIRTUAL_KEY| GetAsyncKeyState(key.0 as i32) >= 0)
}

/// The foreground Explorer window or desktop, when its file list has the
/// keyboard focus and no text box (rename, address, search) has a caret.
unsafe fn file_view() -> Option<HWND> {
    let window = GetForegroundWindow();
    shell_kind(window)?;
    let mut info = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
    GetGUIThreadInfo(GetWindowThreadProcessId(window, None), &mut info).ok()?;
    let list = matches!(class_name(info.hwndFocus).as_str(), "DirectUIHWND" | "SysListView32");
    (list && info.hwndCaret.is_invalid()).then_some(window)
}

/// Foreground class, focus class, and caret, for the diagnostic log.
unsafe fn focus_report() -> String {
    let window = GetForegroundWindow();
    let mut info = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
    let _ = GetGUIThreadInfo(GetWindowThreadProcessId(window, None), &mut info);
    format!("foreground {}, focus {}, caret {}", class_name(window), class_name(info.hwndFocus), !info.hwndCaret.is_invalid())
}

/// True when PFW_RESIDENT_LOG is set.
pub(crate) fn logging() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("PFW_RESIDENT_LOG").is_some())
}

/// Appends one line to %LOCALAPPDATA%\PreviewForWindows\resident.log.
pub(crate) fn log(line: &str) {
    use std::io::Write;
    let Some(folder) = std::env::var_os("LOCALAPPDATA").map(|d| std::path::PathBuf::from(d).join("PreviewForWindows")) else {
        return;
    };
    let _ = std::fs::create_dir_all(&folder);
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(folder.join("resident.log")) {
        let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let _ = writeln!(file, "{time} {line}");
    }
}

/// Some(true) for the desktop, Some(false) for an Explorer window.
unsafe fn shell_kind(window: HWND) -> Option<bool> {
    match class_name(window).as_str() {
        "CabinetWClass" => Some(false),
        "Progman" | "WorkerW" => Some(true),
        _ => None,
    }
}

unsafe fn class_name(window: HWND) -> String {
    let mut buffer = [0u16; 64];
    let length = GetClassNameW(window, &mut buffer).max(0) as usize;
    String::from_utf16_lossy(&buffer[..length])
}
