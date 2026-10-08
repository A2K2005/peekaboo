//! Windows file dialogs, the clipboard, and saved signatures.
use super::sheet;
use std::path::PathBuf;
use windows::{
    core::*,
    Win32::{
        Foundation::*,
        System::Com::CoTaskMemFree,
        UI::{Controls::Dialogs::*, Shell::*},
    },
};

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

pub(super) unsafe fn choose(hwnd: HWND) -> Option<PathBuf> {
    choose_paths(hwnd, false, false).and_then(|paths| paths.into_iter().next())
}
pub(super) unsafe fn choose_many(hwnd: HWND, images_only: bool) -> Option<Vec<PathBuf>> {
    choose_paths(hwnd, images_only, true)
}
pub(super) unsafe fn choose_paths(hwnd: HWND, images_only: bool, multiple: bool) -> Option<Vec<PathBuf>> {
    let mut buffer = vec![0u16; 1024 * 1024];
    let filter = wide(if images_only {
        "Images\0*.jpg;*.jpeg;*.png;*.gif;*.tif;*.tiff;*.bmp;*.webp;*.heic;*.heif\0"
    } else {
        "PDF and images\0*.pdf;*.jpg;*.jpeg;*.png;*.gif;*.tif;*.tiff;*.bmp;*.webp;*.heic;*.heif\0All files\0*.*\0"
    });
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: hwnd,
        lpstrFile: PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        Flags: OFN_FILEMUSTEXIST
            | OFN_PATHMUSTEXIST
            | OFN_NOCHANGEDIR
            | OFN_EXPLORER
            | if multiple {
                OFN_ALLOWMULTISELECT
            } else {
                OPEN_FILENAME_FLAGS(0)
            },
        ..Default::default()
    };
    if !GetOpenFileNameW(&mut dialog).as_bool() {
        return None;
    }
    Some(picker_paths(&buffer))
}
pub(super) fn picker_paths(buffer: &[u16]) -> Vec<PathBuf> {
    let parts: Vec<_> = buffer
        .split(|c| *c == 0)
        .take_while(|s| !s.is_empty())
        .map(|s| PathBuf::from(String::from_utf16_lossy(s)))
        .collect();
    if parts.len() <= 1 {
        return parts;
    }
    parts[1..].iter().map(|name| parts[0].join(name)).collect()
}

pub(super) unsafe fn destination(hwnd: HWND, pdf: bool) -> Option<PathBuf> {
    let mut buffer = vec![0u16; 32768];
    let name = wide(if pdf {
        "Edited copy.pdf"
    } else {
        "Edited copy.png"
    });
    buffer[..name.len()].copy_from_slice(&name);
    let filter = wide(if pdf {
        "PDF\0*.pdf\0"
    } else {
        "PNG image\0*.png\0JPEG image\0*.jpg\0TIFF image\0*.tif\0Bitmap\0*.bmp\0PDF document\0*.pdf\0WebP image\0*.webp\0"
    });
    let extension = wide(if pdf { "pdf" } else { "png" });
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: hwnd,
        lpstrFile: PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrDefExt: PCWSTR(extension.as_ptr()),
        Flags: OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    if !GetSaveFileNameW(&mut dialog).as_bool() {
        return None;
    }
    let length = buffer.iter().position(|v| *v == 0).unwrap_or(0);
    let mut path = PathBuf::from(String::from_utf16_lossy(&buffer[..length]));
    if !pdf {
        path.set_extension(match dialog.nFilterIndex {
            2 => "jpg",
            3 => "tif",
            4 => "bmp",
            5 => "pdf",
            6 => "webp",
            _ => "png",
        });
    }
    if path.exists() {
        sheet::alert(hwnd, "Choose a new name", "Peekaboo saves a copy and never replaces an existing file.");
        None
    } else {
        Some(path)
    }
}

/// A save dialog for one file type. The name always gets `extension`.
/// None when cancelled or when the file exists.
pub(super) unsafe fn save_as(hwnd: HWND, name: &str, label: &str, extension: &str) -> Option<PathBuf> {
    let mut buffer = vec![0u16; 32768];
    let name: Vec<u16> = name.encode_utf16().take(buffer.len() - 1).collect();
    buffer[..name.len()].copy_from_slice(&name);
    let filter = wide(&format!("{label}\0*.{extension}\0"));
    let default = wide(extension);
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: hwnd,
        lpstrFile: PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrDefExt: PCWSTR(default.as_ptr()),
        Flags: OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    if !GetSaveFileNameW(&mut dialog).as_bool() {
        return None;
    }
    let length = buffer.iter().position(|v| *v == 0).unwrap_or(0);
    let mut path = PathBuf::from(String::from_utf16_lossy(&buffer[..length]));
    let jpeg = extension == "jpg" && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("jpeg"));
    if !jpeg && !path.extension().is_some_and(|e| e.eq_ignore_ascii_case(extension)) {
        path.set_extension(extension);
    }
    if path.exists() {
        sheet::alert(hwnd, "Choose a new name", "Peekaboo saves a copy and never replaces an existing file.");
        None
    } else {
        Some(path)
    }
}

pub(super) unsafe fn folder(hwnd: HWND) -> Option<PathBuf> {
    let mut display = vec![0u16; 260];
    let info = BROWSEINFOW {
        hwndOwner: hwnd,
        pszDisplayName: PWSTR(display.as_mut_ptr()),
        lpszTitle: w!("Choose a folder for new copies. Existing files will not be overwritten."),
        ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
        ..Default::default()
    };
    let pidl = SHBrowseForFolderW(&info);
    if pidl.is_null() {
        return None;
    }
    let mut buffer = vec![0u16; 32768];
    let ok = SHGetPathFromIDListEx(pidl, &mut buffer, GPFIDL_DEFAULT).as_bool();
    CoTaskMemFree(Some(pidl.cast()));
    if !ok {
        return None;
    }
    let length = buffer.iter().position(|v| *v == 0).unwrap_or(0);
    Some(PathBuf::from(String::from_utf16_lossy(&buffer[..length])))
}

fn app_data() -> std::result::Result<PathBuf, String> {
    std::env::var_os("LOCALAPPDATA")
        .map(|p| PathBuf::from(p).join("Peekaboo"))
        .ok_or("Windows local app storage is unavailable.".into())
}
/// A saved signature: strokes of `[x, y, pressure]` in 0..1 of its bounding
/// box, and the box height divided by its width.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Signature {
    pub(super) strokes: Vec<Vec<[f32; 3]>>,
    pub(super) aspect: f32,
}

const SIGNATURE_HEADER: &str = "signature 2 ";

/// Saved signatures, newest first. Each is a file in the signatures
/// folder; `signature.txt` beside it is the one earlier versions saved.
/// Files that cannot be read are left out.
pub(super) fn signatures() -> Vec<Signature> {
    let Ok(base) = app_data() else {
        return Vec::new();
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(base.join("signatures"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|x| x == "txt"))
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files
        .into_iter()
        .map(|(_, path)| path)
        .chain(Some(base.join("signature.txt")))
        .filter_map(|path| parse_signature(&std::fs::read_to_string(path).ok()?))
        .collect()
}

/// Version 2 starts with a header and separates strokes with blank lines.
/// The first version was one stroke of "x,y" lines with no shape stored.
fn parse_signature(text: &str) -> Option<Signature> {
    if text.len() > 4_000_000 {
        return None;
    }
    let mut lines = text.lines().peekable();
    let aspect = match lines.peek()?.strip_prefix(SIGNATURE_HEADER) {
        Some(value) => {
            let aspect = value.trim().parse::<f32>().ok().filter(|a| a.is_finite() && (0.01..=100.0).contains(a))?;
            lines.next();
            aspect
        }
        None => 0.35,
    };
    let valid = |n: &f32| n.is_finite() && (0.0..=1.0).contains(n);
    let mut strokes: Vec<Vec<[f32; 3]>> = vec![Vec::new()];
    for line in lines {
        if line.trim().is_empty() {
            if strokes.last().is_some_and(|s| !s.is_empty()) {
                strokes.push(Vec::new());
            }
            continue;
        }
        let values: Vec<f32> = line.split(',').map(|v| v.trim().parse::<f32>().ok()).collect::<Option<_>>()?;
        let point = match values[..] {
            [x, y] => [x, y, 0.5],
            [x, y, p] => [x, y, p],
            _ => return None,
        };
        if !point.iter().all(valid) {
            return None;
        }
        strokes.last_mut()?.push(point);
    }
    strokes.retain(|s| !s.is_empty());
    (strokes.iter().map(Vec::len).sum::<usize>() >= 2).then_some(Signature { strokes, aspect })
}

/// Saves strokes drawn in any units, fitted to their bounding box.
pub(super) fn store_signature(strokes: &[Vec<[f32; 3]>]) -> std::result::Result<Signature, String> {
    let points = || strokes.iter().flatten();
    if points().count() < 2 {
        return Err("Draw your signature first.".into());
    }
    let left = points().map(|p| p[0]).fold(f32::INFINITY, f32::min);
    let top = points().map(|p| p[1]).fold(f32::INFINITY, f32::min);
    let width = (points().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max) - left).max(1e-4);
    let height = (points().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max) - top).max(1e-4);
    let signature = Signature {
        strokes: strokes
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.iter().map(|p| [(p[0] - left) / width, (p[1] - top) / height, p[2].clamp(0.0, 1.0)]).collect())
            .collect(),
        aspect: (height / width).clamp(0.01, 100.0),
    };
    let mut text = format!("{SIGNATURE_HEADER}{}\n", signature.aspect);
    let body: Vec<String> = signature
        .strokes
        .iter()
        .map(|s| s.iter().map(|p| format!("{},{},{}", p[0], p[1], p[2])).collect::<Vec<_>>().join("\n"))
        .collect();
    text.push_str(&body.join("\n\n"));
    let folder = app_data()?.join("signatures");
    std::fs::create_dir_all(&folder).map_err(|e| format!("Cannot save the signature: {e}"))?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
    let path = folder.join(format!("{stamp}.txt"));
    let staged = path.with_extension("tmp");
    std::fs::write(&staged, text)
        .and_then(|_| std::fs::rename(&staged, &path))
        .map_err(|e| format!("Cannot save the signature: {e}"))?;
    Ok(signature)
}
pub(super) unsafe fn read_clipboard(hwnd: HWND) -> Option<String> {
    use windows::Win32::System::{DataExchange::*, Memory::*};
    OpenClipboard(Some(hwnd)).ok()?;
    let text = (|| {
        let memory = HGLOBAL(GetClipboardData(13).ok()?.0);
        let pointer = GlobalLock(memory) as *const u16;
        if pointer.is_null() {
            return None;
        }
        let units = std::slice::from_raw_parts(pointer, GlobalSize(memory) / 2);
        let length = units.iter().position(|c| *c == 0).unwrap_or(units.len());
        let text = String::from_utf16_lossy(&units[..length]);
        let _ = GlobalUnlock(memory);
        Some(text)
    })();
    let _ = CloseClipboard();
    text
}

pub(super) unsafe fn clipboard(hwnd: HWND, text: &str) -> std::result::Result<(), String> {
    use windows::Win32::System::{DataExchange::*, Memory::*};
    let text = wide(text);
    OpenClipboard(Some(hwnd)).map_err(|_| "Clipboard is busy. Try again.".to_string())?;
    let result = (|| -> Result<()> {
        let memory = GlobalAlloc(GMEM_MOVEABLE, text.len() * 2)?;
        let destination = GlobalLock(memory);
        if destination.is_null() {
            let _ = GlobalFree(Some(memory));
            return Err(Error::from_thread());
        }
        std::ptr::copy_nonoverlapping(text.as_ptr(), destination.cast::<u16>(), text.len());
        let _ = GlobalUnlock(memory);
        if let Err(error) =
            EmptyClipboard().and_then(|_| SetClipboardData(13, Some(HANDLE(memory.0))).map(|_| ()))
        {
            let _ = GlobalFree(Some(memory));
            return Err(error);
        }
        Ok(())
    })();
    let _ = CloseClipboard();
    result.map_err(|e| format!("Could not copy text: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_picker_handles_single_and_multiple_files() {
        assert_eq!(picker_paths(&wide(r"C:\photos\one.png")), vec![PathBuf::from(r"C:\photos\one.png")]);
        let many: Vec<u16> = r"C:\photos|one.png|two.jpg|".encode_utf16().map(|c| if c == '|' as u16 { 0 } else { c }).collect();
        assert_eq!(
            picker_paths(&many),
            vec![PathBuf::from(r"C:\photos").join("one.png"), PathBuf::from(r"C:\photos").join("two.jpg")]
        );
    }
}
