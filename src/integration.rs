//! Windows integration: single instance, file associations, Explorer verbs,
//! share, drag out, and recent files. Sources for each behavior: docs/packaging.md.

use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::core::{w, AgileReference, Interface, HSTRING, PCWSTR, PWSTR};
use windows::ApplicationModel::DataTransfer::{DataRequestedEventArgs, DataTransferManager};
use windows::Foundation::TypedEventHandler;
use windows::Storage::{IStorageItem, StorageFile};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, HANDLE, HWND, LPARAM,
    WPARAM,
};
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::Ole::{IDropSource, DROPEFFECT, DROPEFFECT_COPY};
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegDeleteTreeW, RegGetValueW, RegSetKeyValueW, HKEY,
    HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_SZ, RRF_RT_REG_SZ,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_LBUTTON, VK_RBUTTON};
use windows::Win32::UI::Shell::{
    AssocQueryStringW, ASSOCF_NONE, ASSOCSTR, ASSOCSTR_EXECUTABLE, ASSOCSTR_PROGID,
    BHID_DataObject, IDataTransferManagerInterop, ILCreateFromPathW, ILFree, SHAddToRecentDocs,
    SHChangeNotify, SHCreateShellItemArrayFromIDLists, SHDoDragDrop, ShellExecuteW, SHARD_PATHW,
    SHCNE_ASSOCCHANGED, SHCNF_IDLIST,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, FindWindowExW, GetWindowThreadProcessId, SendMessageTimeoutW,
    SMTO_ABORTIFHUNG, SW_SHOWNORMAL, WM_COPYDATA,
};

pub const APP_NAME: &str = "Preview for Windows";
/// Class name of the main window. A second launch finds the running instance by it.
pub const WINDOW_CLASS: &str = "PreviewForWindowsMain";
/// dwData of every WM_COPYDATA this app sends ("PFW1").
pub const COPYDATA_TAG: usize = 0x5046_5731;
/// Command lines are at most 32,767 characters, so 1 MiB is ample.
const MAX_PAYLOAD: u32 = 1 << 20;
const HANDOFF_WAIT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Open,
    Convert,
    Resize,
    Combine,
    /// Quick view: the first path, then the files that arrows move through.
    Peek,
}

impl Action {
    pub fn flag(self) -> &'static str {
        match self {
            Action::Open => "--open",
            Action::Convert => "--convert",
            Action::Resize => "--resize",
            Action::Combine => "--combine",
            Action::Peek => "--peek",
        }
    }

    fn from_flag(flag: &str) -> Option<Self> {
        [
            Action::Open,
            Action::Convert,
            Action::Resize,
            Action::Combine,
            Action::Peek,
        ]
        .into_iter()
        .find(|a| a.flag() == flag)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub action: Action,
    pub paths: Vec<PathBuf>,
}

/// Reads `[--convert | --resize | --combine] path...` (arguments after the
/// program name). Relative paths resolve against the current folder.
/// Other flags are ignored.
pub fn parse_args<I: IntoIterator<Item = OsString>>(args: I) -> Command {
    let mut command = Command {
        action: Action::Open,
        paths: Vec::new(),
    };
    for (index, arg) in args.into_iter().enumerate() {
        if let Some(action) = arg.to_str().and_then(Action::from_flag) {
            if index == 0 {
                command.action = action;
            }
            continue;
        }
        if arg.to_string_lossy().starts_with("--") {
            continue;
        }
        if let Ok(path) = std::path::absolute(&arg) {
            if !command.paths.contains(&path) {
                command.paths.push(path);
            }
        }
    }
    command
}

/// Explorer can start one process per selected file for a verb, and each one
/// forwards its part. The shell collects same-action commands for a moment
/// and then runs one job. Returns false, and changes nothing, when the actions differ.
pub fn merge(pending: &mut Command, next: Command) -> bool {
    if pending.action != next.action {
        return false;
    }
    for path in next.paths {
        if !pending.paths.contains(&path) {
            pending.paths.push(path);
        }
    }
    true
}

/// Payload format: UTF-16 strings, each ending in NUL. The first is the action flag.
pub fn encode(command: &Command) -> Vec<u16> {
    let mut data: Vec<u16> = command.action.flag().encode_utf16().chain([0]).collect();
    for path in &command.paths {
        data.extend(path.as_os_str().encode_wide().chain([0]));
    }
    data
}

/// Inverse of `encode`. Drops every path that is not an absolute drive or UNC
/// path, so device paths such as `\\.\PhysicalDrive0` never reach the app.
pub fn decode(data: &[u16]) -> Option<Command> {
    let (&last, body) = data.split_last()?;
    if last != 0 {
        return None;
    }
    let mut parts = body.split(|&c| c == 0);
    let action = Action::from_flag(&String::from_utf16(parts.next()?).ok()?)?;
    let paths = parts
        .map(|p| PathBuf::from(OsString::from_wide(p)))
        .filter(|p| is_file_path(p))
        .collect();
    Some(Command { action, paths })
}

fn is_file_path(path: &Path) -> bool {
    let drive_or_unc = match path.components().next() {
        Some(Component::Prefix(prefix)) => matches!(
            prefix.kind(),
            Prefix::Disk(_) | Prefix::VerbatimDisk(_) | Prefix::UNC(..) | Prefix::VerbatimUNC(..)
        ),
        _ => false,
    };
    drive_or_unc && path.is_absolute()
}

/// Reads a WM_COPYDATA payload sent by `forward`. Returns `None` for any
/// other sender or a malformed payload; the window procedure then returns 0.
///
/// # Safety
/// `lparam` must be the lParam of a WM_COPYDATA message being handled now.
pub unsafe fn decode_copydata(lparam: LPARAM) -> Option<Command> {
    let data = unsafe { (lparam.0 as *const COPYDATASTRUCT).as_ref() }?;
    if data.dwData != COPYDATA_TAG
        || data.lpData.is_null()
        || data.cbData == 0
        || data.cbData % 2 != 0
        || data.cbData > MAX_PAYLOAD
    {
        return None;
    }
    // The system copy may be unaligned for u16, so read bytes.
    let bytes =
        unsafe { std::slice::from_raw_parts(data.lpData as *const u8, data.cbData as usize) };
    let wide: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    decode(&wide)
}

/// Finds the running instance's top-level window, or its message-only window
/// if it keeps one to receive files while no window is open.
pub fn find_window(class: &str) -> Option<HWND> {
    // With no parent, FindWindowEx searches top-level and message-only windows.
    unsafe { FindWindowExW(None, None, &HSTRING::from(class), None) }.ok()
}

/// Sends `command` to a running window. Succeeds only when that window
/// returns TRUE from WM_COPYDATA.
pub fn forward(hwnd: HWND, command: &Command) -> Result<(), String> {
    let payload = encode(command);
    let data = COPYDATASTRUCT {
        dwData: COPYDATA_TAG,
        cbData: (payload.len() * 2) as u32,
        lpData: payload.as_ptr() as *mut _,
    };
    if data.cbData > MAX_PAYLOAD {
        return Err("Too many files to pass to the open Preview window.".into());
    }
    let mut reply = 0usize;
    let sent = unsafe {
        let mut process = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut process));
        // This process got foreground rights from the launch; pass them on.
        let _ = AllowSetForegroundWindow(process);
        SendMessageTimeoutW(
            hwnd,
            WM_COPYDATA,
            WPARAM(0),
            LPARAM(&data as *const COPYDATASTRUCT as isize),
            SMTO_ABORTIFHUNG,
            HANDOFF_WAIT.as_millis() as u32,
            Some(&mut reply),
        )
    };
    if sent.0 == 0 || reply != 1 {
        return Err("The open Preview window did not respond.".into());
    }
    Ok(())
}

/// Keeps this process registered as the running instance until dropped.
pub struct InstanceGuard(Option<HANDLE>);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            unsafe {
                let _ = CloseHandle(handle);
            }
        }
    }
}

/// Call before creating any window. Returns `None` when a running instance
/// took `command`; exit then. Returns a guard when this process must open
/// its own window; keep the guard until exit. If the running instance does
/// not answer within 5 s, this process opens the files itself.
pub fn hand_off(class: &str, command: &Command) -> Option<InstanceGuard> {
    let name = HSTRING::from(format!("Local\\{class}"));
    let Ok(mutex) = (unsafe { CreateMutexW(None, false, &name) }) else {
        return Some(InstanceGuard(None));
    };
    let guard = InstanceGuard(Some(mutex));
    if unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
        return Some(guard);
    }
    // The other instance may still be creating its window.
    let deadline = Instant::now() + HANDOFF_WAIT;
    loop {
        if let Some(hwnd) = find_window(class) {
            return forward(hwnd, command).err().map(|_| guard);
        }
        if Instant::now() >= deadline {
            return Some(guard);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

const APP_KEY: &str = "PreviewForWindows";
const THUMBNAIL_HANDLER: &str = "{e357fccd-a995-4576-b01f-234630154e96}";
const PREVIEW_HANDLER: &str = "{8895b1c6-b41f-4c1c-a562-0d564250836f}";
const PRESERVED_KEY: &str = "AssociationPreservation";
/// Extension, ProgID, and type name. Default Programs asks for app-specific ProgIDs.
pub const FILE_TYPES: [(&str, &str, &str); 11] = [
    (".pdf", "PreviewForWindows.Pdf", "PDF document"),
    (".jpg", "PreviewForWindows.Jpeg", "JPEG image"),
    (".jpeg", "PreviewForWindows.Jpeg", "JPEG image"),
    (".png", "PreviewForWindows.Png", "PNG image"),
    (".webp", "PreviewForWindows.Webp", "WebP image"),
    (".heic", "PreviewForWindows.Heif", "HEIF image"),
    (".heif", "PreviewForWindows.Heif", "HEIF image"),
    (".gif", "PreviewForWindows.Gif", "GIF image"),
    (".tif", "PreviewForWindows.Tiff", "TIFF image"),
    (".tiff", "PreviewForWindows.Tiff", "TIFF image"),
    (".bmp", "PreviewForWindows.Bmp", "BMP image"),
];
/// Explorer verb key, menu text, action, and whether PDFs get it.
pub const VERBS: [(&str, &str, Action, bool); 3] = [
    (
        "PreviewForWindows.Convert",
        "Convert",
        Action::Convert,
        false,
    ),
    ("PreviewForWindows.Resize", "Resize", Action::Resize, false),
    (
        "PreviewForWindows.Combine",
        "Combine into PDF",
        Action::Combine,
        true,
    ),
];

/// Registers the app for this Windows user: ProgIDs, "Open with", Default
/// apps, and the Explorer verbs. It never changes a default app.
pub fn register(exe: &Path) -> Result<(), String> {
    register_at("Software", exe)?;
    notify_associations_changed();
    Ok(())
}

pub fn unregister(exe: &Path) -> Result<(), String> {
    let result = unregister_at("Software", exe);
    notify_associations_changed();
    result
}

/// `base` is a key under HKEY_CURRENT_USER: "Software" for real use, a test key in tests.
pub fn register_at(base: &str, exe: &Path) -> Result<(), String> {
    let (exe, exe_name) = exe_parts(exe)?;
    let classes = format!("{base}\\Classes");
    let capabilities = format!("{base}\\{APP_KEY}\\Capabilities");
    let application = format!("{classes}\\Applications\\{exe_name}");
    let open = format!("\"{exe}\" \"%1\"");
    let icon = format!("\"{exe}\",0");
    for (extension, progid, type_name) in FILE_TYPES {
        preserve_shell_metadata(base, &classes, extension)?;
        let key = format!("{classes}\\{progid}");
        set(&key, "", type_name)?;
        set(&format!("{key}\\DefaultIcon"), "", &icon)?;
        set(&format!("{key}\\shell\\open"), "MultiSelectModel", "Player")?;
        set(&format!("{key}\\shell\\open\\command"), "", &open)?;
        set(
            &format!("{classes}\\{extension}\\OpenWithProgids"),
            progid,
            "",
        )?;
        set(&format!("{application}\\SupportedTypes"), extension, "")?;
        set(
            &format!("{capabilities}\\FileAssociations"),
            extension,
            progid,
        )?;
        for (verb, text, action, pdf) in VERBS {
            if extension == ".pdf" && !pdf {
                continue;
            }
            let key = format!("{classes}\\SystemFileAssociations\\{extension}\\shell\\{verb}");
            set(&key, "MUIVerb", text)?;
            set(&key, "MultiSelectModel", "Player")?;
            set(&key, "Icon", &icon)?;
            set(
                &format!("{key}\\command"),
                "",
                &format!("\"{exe}\" {} \"%1\"", action.flag()),
            )?;
        }
    }
    set(&application, "FriendlyAppName", APP_NAME)?;
    set(
        &format!("{application}\\shell\\open"),
        "MultiSelectModel",
        "Player",
    )?;
    set(&format!("{application}\\shell\\open\\command"), "", &open)?;
    set(&capabilities, "ApplicationName", APP_NAME)?;
    set(
        &capabilities,
        "ApplicationDescription",
        "View and edit PDFs and images.",
    )?;
    set(
        &format!("{base}\\RegisteredApplications"),
        APP_NAME,
        &capabilities,
    )
}

/// Removes only what `register_at` wrote. Extension keys stay; other apps share them.
pub fn unregister_at(base: &str, exe: &Path) -> Result<(), String> {
    let (_, exe_name) = exe_parts(exe)?;
    let classes = format!("{base}\\Classes");
    let mut result = Ok(());
    let mut keep_first_error = |r: Result<(), String>| {
        if result.is_ok() {
            result = r;
        }
    };
    for (extension, progid, _) in FILE_TYPES {
        keep_first_error(delete_tree(&format!("{classes}\\{progid}")));
        keep_first_error(delete_value(
            &format!("{classes}\\{extension}\\OpenWithProgids"),
            progid,
        ));
        for (verb, ..) in VERBS {
            keep_first_error(delete_tree(&format!(
                "{classes}\\SystemFileAssociations\\{extension}\\shell\\{verb}"
            )));
        }
    }
    keep_first_error(delete_tree(&format!("{classes}\\Applications\\{exe_name}")));
    keep_first_error(delete_tree(&format!("{base}\\{APP_KEY}\\Capabilities")));
    keep_first_error(delete_tree(&format!("{base}\\{APP_KEY}\\{PRESERVED_KEY}")));
    keep_first_error(delete_value(
        &format!("{base}\\RegisteredApplications"),
        APP_NAME,
    ));
    result
}

/// A per-user extension key hides machine-wide shell metadata. Copy only
/// missing values from the effective association, then record each copy.
/// Uninstall leaves these copied Explorer values in place because deleting a
/// shared extension key cannot be made conditional on another installer not
/// writing it at the same time.
fn preserve_shell_metadata(base: &str, classes: &str, extension: &str) -> Result<(), String> {
    let extension_key = format!("{classes}\\{extension}");
    let marker_key = preservation_key(base, extension);
    // Read the merged association before creating the per-user extension key.
    let perceived_source = source_value(base, extension, "PerceivedType")?;
    let previous_progid = previous_progid(base, extension)?;
    let mut handlers = Vec::new();
    for (handler, marker) in [
        (THUMBNAIL_HANDLER, "ThumbnailHandler"),
        (PREVIEW_HANDLER, "PreviewHandler"),
    ] {
        let direct = source_value(base, &format!("{extension}\\ShellEx\\{handler}"), "")?;
        let inherited = match previous_progid.as_deref() {
            Some(progid) if !progid.is_empty() => {
                source_value(base, &format!("{progid}\\ShellEx\\{handler}"), "")?
            }
            _ => None,
        };
        handlers.push((handler, marker, direct.or(inherited)));
    }

    if get(HKEY_CURRENT_USER, &extension_key, "PerceivedType")?.is_none() {
        if let Some(value) = perceived_source.filter(|value| valid_perceived_type(value)) {
            set(&extension_key, "PerceivedType", &value)?;
            set(&marker_key, "PerceivedType", &value)?;
        }
    }

    for (handler, marker, source) in handlers {
        let target = format!("{extension_key}\\ShellEx\\{handler}");
        if get(HKEY_CURRENT_USER, &target, "")?.is_some() {
            continue;
        }
        if let Some(value) = source.filter(|value| valid_handler_clsid(value)) {
            set(&target, "", &value)?;
            set(&marker_key, marker, &value)?;
        }
    }
    Ok(())
}

fn preservation_key(base: &str, extension: &str) -> String {
    format!(
        "{base}\\{APP_KEY}\\{PRESERVED_KEY}\\{}",
        extension.trim_start_matches('.')
    )
}

/// Tests use a scratch MachineClasses key as the effective source. Real
/// registration reads HKCR before the per-user extension key is created.
fn source_value(base: &str, relative_key: &str, name: &str) -> Result<Option<String>, String> {
    if base.eq_ignore_ascii_case("Software") {
        get(HKEY_CLASSES_ROOT, relative_key, name)
    } else {
        get(
            HKEY_CURRENT_USER,
            &format!("{base}\\MachineClasses\\{relative_key}"),
            name,
        )
    }
}

fn previous_progid(base: &str, extension: &str) -> Result<Option<String>, String> {
    let user_choice = if base.eq_ignore_ascii_case("Software") {
        get(
            HKEY_CURRENT_USER,
            &format!(
                "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FileExts\\{extension}\\UserChoice"
            ),
            "ProgId",
        )?
    } else {
        get(
            HKEY_CURRENT_USER,
            &format!("{base}\\UserChoice\\{extension}"),
            "ProgId",
        )?
    };
    match user_choice.filter(|value| !value.is_empty()) {
        Some(value) => Ok(Some(value)),
        None => source_value(base, extension, ""),
    }
}

fn valid_handler_clsid(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 38
        && bytes[0] == b'{'
        && bytes[37] == b'}'
        && bytes[9] == b'-'
        && bytes[14] == b'-'
        && bytes[19] == b'-'
        && bytes[24] == b'-'
        && bytes[1..37]
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 8 | 13 | 18 | 23) || byte.is_ascii_hexdigit())
}

fn valid_perceived_type(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn exe_parts(exe: &Path) -> Result<(&str, &str), String> {
    let text = exe.to_str().filter(|_| exe.is_absolute());
    let name = exe.file_name().and_then(|n| n.to_str());
    match (text, name) {
        (Some(text), Some(name)) => Ok((text, name)),
        _ => Err("Choose the Preview program file by its full path.".into()),
    }
}

fn set(key: &str, name: &str, value: &str) -> Result<(), String> {
    let data: Vec<u16> = value.encode_utf16().chain([0]).collect();
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(key),
            &HSTRING::from(name),
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            (data.len() * 2) as u32,
        )
    }
    .ok()
    .map_err(|e| format!("Could not write the registry key {key}: {e}"))
}

fn get(root: HKEY, key: &str, name: &str) -> Result<Option<String>, String> {
    let mut size = 0u32;
    let status = unsafe {
        RegGetValueW(
            root,
            &HSTRING::from(key),
            &HSTRING::from(name),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    status
        .ok()
        .map_err(|e| format!("Could not read the registry key {key}: {e}"))?;
    let mut data = vec![0u16; (size as usize).div_ceil(2)];
    unsafe {
        RegGetValueW(
            root,
            &HSTRING::from(key),
            &HSTRING::from(name),
            RRF_RT_REG_SZ,
            None,
            Some(data.as_mut_ptr().cast()),
            Some(&mut size),
        )
    }
    .ok()
    .map_err(|e| format!("Could not read the registry key {key}: {e}"))?;
    data.truncate(
        data.iter()
            .position(|&unit| unit == 0)
            .unwrap_or(data.len()),
    );
    Ok(Some(String::from_utf16_lossy(&data)))
}

fn delete_tree(key: &str) -> Result<(), String> {
    let status = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(key)) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    status
        .ok()
        .map_err(|e| format!("Could not remove the registry key {key}: {e}"))
}

fn delete_value(key: &str, name: &str) -> Result<(), String> {
    let status =
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(key), &HSTRING::from(name)) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    status
        .ok()
        .map_err(|e| format!("Could not remove {name} from {key}: {e}"))
}

fn notify_associations_changed() {
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

/// Windows 11 can open the app's own page; Windows 10 opens the general page.
pub fn default_apps_uri(build: u32) -> String {
    if build >= 22000 {
        // APP_NAME has only letters and spaces, so this is its URI escape.
        format!(
            "ms-settings:defaultapps?registeredAppUser={}",
            APP_NAME.replace(' ', "%20")
        )
    } else {
        "ms-settings:defaultapps".into()
    }
}

/// Opens Default apps in Settings, where the user picks Preview. Never sets a default itself.
pub fn open_default_apps() -> Result<(), String> {
    let uri = HSTRING::from(default_apps_uri(windows_build()));
    let result = unsafe { ShellExecuteW(None, w!("open"), &uri, None, None, SW_SHOWNORMAL) };
    if result.0 as isize <= 32 {
        return Err(
            "Windows could not open Default apps settings. Open Settings > Apps > Default apps."
                .into(),
        );
    }
    Ok(())
}

/// True when Windows opens `extension` (".pdf") with this app, by its ProgID
/// or by this program's path ("Open with" choices use the path).
pub fn is_default_for(extension: &str) -> bool {
    let extension = HSTRING::from(extension);
    let query = |what: ASSOCSTR| -> Option<String> {
        let mut buffer = [0u16; 1024];
        let mut length = buffer.len() as u32;
        unsafe {
            AssocQueryStringW(
                ASSOCF_NONE,
                what,
                &extension,
                PCWSTR::null(),
                Some(PWSTR(buffer.as_mut_ptr())),
                &mut length,
            )
        }
        .ok()
        .ok()?;
        Some(String::from_utf16_lossy(
            &buffer[..(length as usize).saturating_sub(1).min(buffer.len())],
        ))
    };
    if query(ASSOCSTR_PROGID).is_some_and(|progid| progid.starts_with("PreviewForWindows.")) {
        return true;
    }
    let exe = std::env::current_exe().ok();
    match (query(ASSOCSTR_EXECUTABLE), exe.as_deref().and_then(Path::to_str)) {
        (Some(found), Some(exe)) => found.eq_ignore_ascii_case(exe),
        _ => false,
    }
}

/// True when this user's Default apps list has the app.
pub fn is_registered() -> bool {
    matches!(
        get(HKEY_CURRENT_USER, "Software\\RegisteredApplications", APP_NAME),
        Ok(Some(_))
    )
}

pub fn windows_build() -> u32 {
    let mut buffer = [0u16; 16];
    let mut size = (buffer.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"),
            w!("CurrentBuildNumber"),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status.is_err() {
        return 0;
    }
    let text = String::from_utf16_lossy(&buffer[..(size as usize / 2).saturating_sub(1)]);
    text.trim().parse().unwrap_or(0)
}

/// Opens the Windows share sheet for files. Call on the UI thread that owns `hwnd`.
pub fn share_files(hwnd: HWND, paths: &[PathBuf]) -> Result<(), String> {
    let title = match paths {
        [] => return Err("Open a file to share.".into()),
        [one] => one
            .file_name()
            .unwrap_or(one.as_os_str())
            .to_string_lossy()
            .into_owned(),
        many => format!("{} files", many.len()),
    };
    // ponytail: resolves files on the UI thread; move to a worker if 100-file shares feel slow.
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path.as_os_str()))
            .and_then(|operation| operation.join())
            .and_then(|file| AgileReference::new(&file))
            .map_err(|e| format!("Could not share {}: {}", path.display(), e.message()))?;
        files.push(file);
    }
    let fail = |e: windows::core::Error| {
        format!("Windows could not open the share sheet: {}", e.message())
    };
    let interop = windows::core::factory::<DataTransferManager, IDataTransferManagerInterop>()
        .map_err(fail)?;
    let manager: DataTransferManager = unsafe { interop.GetForWindow(hwnd) }.map_err(fail)?;
    let token = Arc::new(AtomicI64::new(0));
    let handler_token = token.clone();
    let handler = TypedEventHandler::<DataTransferManager, DataRequestedEventArgs>::new(
        move |sender, args| {
            // One share per call; the next call registers fresh items. Remove
            // the handler first, so a failure below cannot leave it registered.
            sender
                .ok()?
                .RemoveDataRequested(handler_token.load(Ordering::SeqCst))?;
            let data = args.ok()?.Request()?.Data()?;
            data.Properties()?.SetTitle(&HSTRING::from(&title))?;
            let items = files
                .iter()
                .map(|f| f.resolve().and_then(|f| f.cast::<IStorageItem>()).map(Some))
                .collect::<Result<Vec<_>, _>>()?;
            data.SetStorageItemsReadOnly(&windows_collections::IIterable::<IStorageItem>::from(
                items,
            ))
        },
    );
    token.store(
        manager.DataRequested(&handler).map_err(fail)?,
        Ordering::SeqCst,
    );
    if let Err(e) = unsafe { interop.ShowShareUIForWindow(hwnd) } {
        let _ = manager.RemoveDataRequested(token.load(Ordering::SeqCst));
        return Err(fail(e));
    }
    Ok(())
}

/// A shell data object (CF_HDROP plus shell ID lists) for existing files in any folders.
pub fn file_data_object(paths: &[PathBuf]) -> Result<IDataObject, String> {
    if paths.is_empty() {
        return Err("Choose a file to drag.".into());
    }
    let mut ids = Vec::with_capacity(paths.len());
    let mut missing = None;
    for path in paths {
        // The shell parser rejects forward slashes; absolute() rewrites them.
        let full = std::path::absolute(path).unwrap_or_else(|_| path.clone());
        let id = unsafe { ILCreateFromPathW(&HSTRING::from(full.as_os_str())) };
        if id.is_null() {
            missing = Some(path);
            break;
        }
        ids.push(id as *const _);
    }
    let result = match missing {
        Some(path) => Err(format!("Could not find {}.", path.display())),
        None => unsafe { SHCreateShellItemArrayFromIDLists(&ids) }
            .and_then(|items| unsafe { items.BindToHandler(None, &BHID_DataObject) })
            .map_err(|e| format!("Could not prepare the files for dragging: {}", e.message())),
    };
    for id in ids {
        unsafe { ILFree(Some(id)) };
    }
    result
}

/// Drags files out of the window as copies; the originals never move.
/// Call on the UI thread after OleInitialize, while a mouse button is down.
pub fn drag_files(hwnd: HWND, paths: &[PathBuf]) -> Result<DROPEFFECT, String> {
    // Without a pressed button, OLE would drop at once wherever the pointer is.
    let pressed =
        unsafe { GetKeyState(VK_LBUTTON.0 as i32) < 0 || GetKeyState(VK_RBUTTON.0 as i32) < 0 };
    if !pressed {
        return Err("Hold the mouse button to drag.".into());
    }
    let data = file_data_object(paths)?;
    // With no drop source, the shell supplies one: Esc cancels, releasing the button drops.
    unsafe { SHDoDragDrop(Some(hwnd), &data, None::<&IDropSource>, DROPEFFECT_COPY) }
        .map_err(|e| format!("Could not drag the files: {}", e.message()))
}

pub const RECENT_LIMIT: usize = 20;
const RECENT_FILE: &str = "recent.txt";

/// %LOCALAPPDATA%\PreviewForWindows
pub fn recent_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|dir| PathBuf::from(dir).join(APP_KEY))
}

/// Newest first, as saved. It reads one small file and checks no paths, so it
/// is safe on the launch path. `prune_recent` drops files that no longer exist.
pub fn load_recent(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(dir.join(RECENT_FILE))
        .unwrap_or_default()
        .lines()
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .take(RECENT_LIMIT)
        .collect()
}

/// Removes files that no longer exist from the list in `dir` and returns the
/// new list. A check on an offline network share can take seconds, so call
/// this on a worker thread, never on the UI thread.
pub fn prune_recent(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let saved = load_recent(dir);
    let missing: Vec<PathBuf> = saved.iter().filter(|p| !p.exists()).cloned().collect();
    if missing.is_empty() {
        return Ok(saved);
    }
    update_recent(dir, |list| list.retain(|p| !missing.contains(p)))
}

/// Moves `path` to the top of the list in `dir` and saves it. Returns the new list.
pub fn add_recent(dir: &Path, path: &Path) -> io::Result<Vec<PathBuf>> {
    let Some(text) = path.to_str().filter(|_| path.is_absolute()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Recent files need a full path.",
        ));
    };
    let key = text.to_lowercase();
    update_recent(dir, |list| {
        list.retain(|p| p.to_string_lossy().to_lowercase() != key);
        list.insert(0, path.to_path_buf());
        list.truncate(RECENT_LIMIT);
    })
}

/// Changes the saved list and saves it. The lock keeps a worker's prune and
/// the UI thread's add from writing the same temporary file.
fn update_recent(dir: &Path, change: impl FnOnce(&mut Vec<PathBuf>)) -> io::Result<Vec<PathBuf>> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut list = load_recent(dir);
    change(&mut list);
    let lines: String = list
        .iter()
        .filter_map(|p| p.to_str())
        .map(|p| format!("{p}\n"))
        .collect();
    std::fs::create_dir_all(dir)?;
    let temp = dir.join("recent.tmp");
    std::fs::write(&temp, lines)?;
    std::fs::rename(&temp, dir.join(RECENT_FILE))?;
    Ok(list)
}

/// Records an opened file in the app's list and in Windows Recent, which feeds the jump list.
pub fn note_recent(path: &Path) -> io::Result<Vec<PathBuf>> {
    let dir = recent_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA is not set."))?;
    let list = add_recent(&dir, path)?;
    let wide = HSTRING::from(path.as_os_str());
    unsafe { SHAddToRecentDocs(SHARD_PATHW.0 as u32, Some(wide.as_ptr().cast())) };
    Ok(list)
}
