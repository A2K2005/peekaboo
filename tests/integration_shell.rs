#![allow(dead_code)]
#[path = "../src/integration.rs"]
mod integration;

use std::path::{Path, PathBuf};
use windows::core::HSTRING;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, HWND};
use windows::Win32::System::Com::{
    CoInitializeEx, COINIT_APARTMENTTHREADED, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL,
};
use windows::Win32::System::Ole::{ReleaseStgMedium, CF_HDROP};
use windows::Win32::System::Registry::{
    RegDeleteTreeW, RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ,
};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

/// The only registry key these tests write. Dropping the guard deletes it.
const TEST_ROOT: &str = r"Software\PreviewForWindows-Test";

struct TestKey;

impl Drop for TestKey {
    fn drop(&mut self) {
        let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(TEST_ROOT)) };
    }
}

fn read(key: &str, name: &str) -> Option<String> {
    let mut buffer = vec![0u16; 1024];
    let mut size = (buffer.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(key),
            &HSTRING::from(name),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    status
        .is_ok()
        .then(|| String::from_utf16_lossy(&buffer[..size as usize / 2 - 1]))
}

/// Every value under a key as sorted "key|name|data" lines, with `root` replaced by "ROOT".
fn dump(root: &str) -> Vec<String> {
    let output = std::process::Command::new("reg")
        .args(["query", &format!(r"HKCU\{root}"), "/s"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let mut key = String::new();
    let mut lines = Vec::new();
    for line in text.lines() {
        if line.starts_with("HKEY_") {
            key = line.replacen(&format!(r"HKEY_CURRENT_USER\{root}"), "ROOT", 1);
        } else if let Some(value) = line.strip_prefix("    ") {
            lines.push(format!("{key}|{}", value.replacen(root, "ROOT", 1)));
        }
    }
    lines.sort();
    lines
}

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("artifacts")
        .join("integration")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn register_writes_per_user_keys_and_unregister_removes_them() {
    let _key = TestKey;
    let base = format!(r"{TEST_ROOT}\rust");
    let exe = Path::new(r"C:\Apps\Preview\preview-for-windows.exe");
    integration::register_at(&base, exe).unwrap();
    let classes = format!(r"{base}\Classes");
    let quoted = r#""C:\Apps\Preview\preview-for-windows.exe""#;

    assert_eq!(
        read(&format!(r"{classes}\PreviewForWindows.Pdf"), "").as_deref(),
        Some("PDF document")
    );
    assert_eq!(
        read(
            &format!(r"{classes}\PreviewForWindows.Jpeg\shell\open\command"),
            ""
        ),
        Some(format!(r#"{quoted} "%1""#))
    );
    assert_eq!(
        read(
            &format!(r"{classes}\.heic\OpenWithProgids"),
            "PreviewForWindows.Heif"
        )
        .as_deref(),
        Some("")
    );
    let convert = format!(r"{classes}\SystemFileAssociations\.png\shell\PreviewForWindows.Convert");
    assert_eq!(read(&convert, "MUIVerb").as_deref(), Some("Convert"));
    assert_eq!(
        read(&convert, "MultiSelectModel").as_deref(),
        Some("Player")
    );
    assert_eq!(
        read(&format!(r"{convert}\command"), ""),
        Some(format!(r#"{quoted} --convert "%1""#))
    );
    let pdf_verbs = format!(r"{classes}\SystemFileAssociations\.pdf\shell");
    assert_eq!(
        read(
            &format!(r"{pdf_verbs}\PreviewForWindows.Combine"),
            "MUIVerb"
        )
        .as_deref(),
        Some("Combine into PDF")
    );
    assert_eq!(
        read(&format!(r"{pdf_verbs}\PreviewForWindows.Resize"), "MUIVerb"),
        None,
        "PDFs get Combine only"
    );
    assert_eq!(
        read(
            &format!(r"{classes}\Applications\preview-for-windows.exe\SupportedTypes"),
            ".tiff"
        )
        .as_deref(),
        Some("")
    );
    let capabilities = format!(r"{base}\PreviewForWindows\Capabilities");
    assert_eq!(
        read(&format!(r"{capabilities}\FileAssociations"), ".webp").as_deref(),
        Some("PreviewForWindows.Webp")
    );
    assert_eq!(
        read(
            &format!(r"{base}\RegisteredApplications"),
            "Preview for Windows"
        ),
        Some(capabilities)
    );
    assert_eq!(dump(&base).len(), 195, "values written");

    integration::unregister_at(&base, exe).unwrap();
    assert_eq!(
        dump(&base),
        Vec::<String>::new(),
        "unregister leaves no values"
    );
    integration::unregister_at(&base, exe).unwrap();
    assert!(
        integration::register_at(&base, Path::new("preview.exe")).is_err(),
        "relative path"
    );
}

#[test]
fn script_writes_the_same_keys_as_rust() {
    let key = TestKey;
    let exe = std::env::current_exe().unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/register-file-associations.ps1");
    let run = |root: &str, extra: &[&str]| {
        let status = std::process::Command::new("powershell")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&script)
            .arg("-Executable")
            .arg(&exe)
            .args(["-Root", root])
            .args(extra)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    };
    let (rust, script_root) = (format!(r"{TEST_ROOT}\rust"), format!(r"{TEST_ROOT}\script"));
    integration::register_at(&rust, &exe).unwrap();
    run(&script_root, &[]);
    let written = dump(&rust);
    assert!(!written.is_empty());
    assert_eq!(dump(&script_root), written);

    run(&script_root, &["-Unregister"]);
    assert_eq!(dump(&script_root), Vec::<String>::new());
    drop(key);
    let status = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(TEST_ROOT)) };
    assert_eq!(status, ERROR_FILE_NOT_FOUND, "the test key is gone");
}

#[test]
fn default_apps_link_matches_windows_version() {
    assert_eq!(
        integration::default_apps_uri(19045),
        "ms-settings:defaultapps"
    );
    assert_eq!(
        integration::default_apps_uri(22631),
        "ms-settings:defaultapps?registeredAppUser=Preview%20for%20Windows"
    );
    assert!(
        integration::windows_build() >= 10240,
        "reads this PC's build"
    );
}

#[test]
fn recent_files_are_newest_first_capped_and_pruned() {
    let dir = scratch("recent");
    let store = dir.join("store");
    assert!(integration::load_recent(&store).is_empty());
    let files: Vec<PathBuf> = (0..25)
        .map(|i| {
            let file = dir.join(format!("file {i}.png"));
            std::fs::write(&file, b"x").unwrap();
            file
        })
        .collect();
    for file in &files {
        integration::add_recent(&store, file).unwrap();
    }
    let list = integration::load_recent(&store);
    assert_eq!(list.len(), integration::RECENT_LIMIT);
    assert_eq!(list[0], files[24]);
    assert_eq!(list[19], files[5]);

    // Adding an existing file again moves it to the top, matched without case.
    let upper = PathBuf::from(files[10].to_str().unwrap().to_uppercase());
    let list = integration::add_recent(&store, &upper).unwrap();
    assert_eq!(list[0], upper);
    assert_eq!(list.len(), 20);
    assert_eq!(
        list.iter()
            .filter(|p| p
                .to_str()
                .unwrap()
                .eq_ignore_ascii_case(files[10].to_str().unwrap()))
            .count(),
        1
    );

    std::fs::remove_file(&files[24]).unwrap();
    // Loading checks no paths, so it cannot stall on an offline share.
    assert!(integration::load_recent(&store).contains(&files[24]));
    let pruned = integration::prune_recent(&store).unwrap();
    assert!(!pruned.contains(&files[24]), "missing files are pruned");
    assert_eq!(pruned.len(), 19);
    assert_eq!(integration::load_recent(&store), pruned);
    assert!(integration::add_recent(&store, Path::new("relative.png")).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn drag_data_object_carries_files_from_two_folders() {
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap() };
    let dir = scratch("drag");
    let files = [
        dir.join("one").join("a.png"),
        dir.join("two").join("b c.pdf"),
    ];
    for file in &files {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, b"x").unwrap();
    }
    let slashes = PathBuf::from(files[1].to_str().unwrap().replace('\\', "/"));
    let data = integration::file_data_object(&[files[0].clone(), slashes]).unwrap();
    let format = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let mut medium = unsafe { data.GetData(&format) }.unwrap();
    let drop = HDROP(unsafe { medium.u.hGlobal.0 });
    let count = unsafe { DragQueryFileW(drop, u32::MAX, None) };
    let paths: Vec<PathBuf> = (0..count)
        .map(|i| {
            let mut name = vec![0u16; unsafe { DragQueryFileW(drop, i, None) } as usize + 1];
            let length = unsafe { DragQueryFileW(drop, i, Some(&mut name)) } as usize;
            PathBuf::from(String::from_utf16_lossy(&name[..length]))
        })
        .collect();
    unsafe { ReleaseStgMedium(&mut medium) };
    assert_eq!(paths, files);

    assert!(integration::file_data_object(&[dir.join("missing.png")]).is_err());
    assert!(integration::file_data_object(&[]).is_err());
    // No mouse button is down in a test, so no drag may start.
    assert!(integration::drag_files(HWND::default(), &files).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}
