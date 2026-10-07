use super::*;

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
        MessageBoxW(Some(hwnd), w!("Choose a new file name. Preview saves a copy and does not overwrite an existing file."), w!("Save a copy"), MB_OK | MB_ICONINFORMATION);
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

pub(super) fn signature_path() -> std::result::Result<PathBuf, String> {
    std::env::var_os("LOCALAPPDATA")
        .map(|p| {
            PathBuf::from(p)
                .join("PreviewForWindows")
                .join("signature.txt")
        })
        .ok_or("Windows local app storage is unavailable.".into())
}
pub(super) fn load_signature() -> std::result::Result<Vec<[f32; 2]>, String> {
    let text = std::fs::read_to_string(signature_path()?).map_err(|_| {
        "Draw a signature with Markup > Ink, then choose Save last ink as signature.".to_string()
    })?;
    if text.len() > 300000 {
        return Err("The saved signature is invalid.".into());
    }
    let mut points = Vec::new();
    for line in text.lines() {
        let Some((x, y)) = line.split_once(',') else {
            return Err("The saved signature is invalid.".into());
        };
        let x = x
            .parse::<f32>()
            .map_err(|_| "The saved signature is invalid.")?;
        let y = y
            .parse::<f32>()
            .map_err(|_| "The saved signature is invalid.")?;
        if !x.is_finite()
            || !y.is_finite()
            || !(0.0..=1.0).contains(&x)
            || !(0.0..=1.0).contains(&y)
        {
            return Err("The saved signature is invalid.".into());
        }
        points.push([x, y]);
    }
    if points.len() < 2 {
        return Err("The saved signature is empty.".into());
    }
    Ok(points)
}
pub(super) unsafe fn save_signature(hwnd: HWND) {
    let points = STATE.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|s| {
                s.path
                    .as_ref()
                    .and_then(|p| s.sessions.get(p).map(|e| (p, e)))
            })
            .and_then(|(path, edits)| {
                if is_pdf(path) {
                    edits.pdf.iter().rev().find_map(|e| match e {
                        PdfEdit::Annotate {
                            kind: AnnotationKind::Ink,
                            points,
                            ..
                        } => Some(points.clone()),
                        _ => None,
                    })
                } else {
                    edits.image.iter().rev().find_map(|e| match e {
                        ImageEdit::Annotate {
                            kind: AnnotationKind::Ink,
                            points,
                            ..
                        } => Some(points.clone()),
                        _ => None,
                    })
                }
            })
    });
    let result = (|| -> std::result::Result<(), String> {
        let points = points.ok_or("Draw a signature with Markup > Ink first.")?;
        let left = points.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min);
        let top = points.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min);
        let width = points
            .iter()
            .map(|p| p[0])
            .fold(f32::NEG_INFINITY, f32::max)
            - left;
        let height = points
            .iter()
            .map(|p| p[1])
            .fold(f32::NEG_INFINITY, f32::max)
            - top;
        if width <= 0.0 || height <= 0.0 {
            return Err("Draw a signature with both width and height.".into());
        }
        let text = points
            .iter()
            .map(|p| format!("{},{}", (p[0] - left) / width, (p[1] - top) / height))
            .collect::<Vec<_>>()
            .join("\n");
        let path = signature_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, text).map_err(|e| e.to_string())
    })();
    let text=wide(&match result {Ok(())=>"Signature saved on this PC. Use Markup > Place saved signature, then drag its size and position.".into(),Err(e)=>e});
    MessageBoxW(Some(hwnd), PCWSTR(text.as_ptr()), w!("Signature"), MB_OK);
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
