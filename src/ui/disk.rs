//! Save model file steps that run off the window thread: a snapshot of the
//! opened version, crash-safe replacement of the user's file, and copy names.

use std::{
    ffi::{OsStr, OsString},
    fs, io,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};
use windows::{
    core::PCWSTR,
    Win32::Storage::FileSystem::{
        MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MOVE_FILE_FLAGS,
        REPLACEFILE_IGNORE_MERGE_ERRORS,
    },
};

/// Size and times of a file, as the PDF engine compares them, to notice
/// changes made by other apps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Stamp {
    pub(super) len: u64,
    pub(super) modified: Option<SystemTime>,
    pub(super) created: Option<SystemTime>,
}
impl Stamp {
    pub(super) fn of(path: &Path) -> io::Result<Self> {
        let m = fs::metadata(path)?;
        Ok(Self { len: m.len(), modified: m.modified().ok(), created: m.created().ok() })
    }
}

/// The stamp at `path`, or None when no file is there.
fn current(path: &Path) -> Result<Option<Stamp>, String> {
    match Stamp::of(path) {
        Ok(stamp) => Ok(Some(stamp)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Cannot read {}: {e}", path.display())),
    }
}

#[derive(Debug, PartialEq)]
pub(super) enum Failure {
    /// The file changed or disappeared since Peekaboo last read or wrote it.
    Changed,
    Error(String),
}
impl From<String> for Failure {
    fn from(error: String) -> Self {
        Failure::Error(error)
    }
}
impl From<&str> for Failure {
    fn from(error: &str) -> Self {
        Failure::Error(error.into())
    }
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// "name (edited).ext", or "name (edited 2).ext" and up when taken.
/// `extension` replaces the original one, for formats Peekaboo cannot write.
pub(super) fn copy_name(original: &Path, extension: Option<&OsStr>) -> Option<PathBuf> {
    let folder = original.parent()?;
    let stem = original.file_stem()?;
    let extension = extension.or(original.extension());
    (1..10_000)
        .map(|n| {
            let mut name = stem.to_os_string();
            name.push(if n == 1 { " (edited)".to_string() } else { format!(" (edited {n})") });
            if let Some(extension) = extension {
                name.push(".");
                name.push(extension);
            }
            folder.join(name)
        })
        .find(|path| fs::symlink_metadata(path).is_err())
}

/// Prefix of staging files. A crash can leave "~pfw<pid>-<n>.<ext>" beside
/// the user's file; the user's file itself is never partly written.
pub(super) const STAGING: &str = "~pfw";
static NEXT: AtomicU64 = AtomicU64::new(0);

/// An unused staging name in `folder`, so the swap stays on one volume.
fn staging(folder: &Path, extension: Option<&OsStr>) -> PathBuf {
    loop {
        let mut name = OsString::from(format!("{STAGING}{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        if let Some(extension) = extension {
            name.push(".");
            name.push(extension);
        }
        let path = folder.join(name);
        if fs::symlink_metadata(&path).is_err() {
            return path;
        }
    }
}

/// Deletes a staging file that did not reach its target.
struct Staged(PathBuf);
impl Drop for Staged {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Puts `staged` in place of `target`. ReplaceFileW keeps the target's
/// creation time, DACLs, and named streams, such as the downloaded-file mark
/// (https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-replacefilew).
/// Its write-through flag "is not supported", so callers flush `staged`
/// first. In every documented failure, `staged` keeps its own name and the
/// target keeps its name or is no longer under it. MoveFileExW then renames
/// `staged` over the target in one step; with MOVEFILE_WRITE_THROUGH it
/// "does not return until the file is actually moved on the disk"
/// (https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-movefileexw).
pub(super) fn replace(target: &Path, staged: &Path) -> Result<(), String> {
    let (to, from) = (wide(target), wide(staged));
    let replaced = unsafe {
        ReplaceFileW(PCWSTR(to.as_ptr()), PCWSTR(from.as_ptr()), PCWSTR::null(), REPLACEFILE_IGNORE_MERGE_ERRORS, None, None)
    };
    let Err(error) = replaced else {
        return Ok(());
    };
    let flags = MOVE_FILE_FLAGS(MOVEFILE_REPLACE_EXISTING.0 | MOVEFILE_WRITE_THROUGH.0);
    unsafe { MoveFileExW(PCWSTR(from.as_ptr()), PCWSTR(to.as_ptr()), flags) }
        .map_err(|_| format!("Cannot replace {}: {}", target.display(), error.message()))
}

/// Moves `staged` to `target`, which must not exist. Never replaces a file.
pub(super) fn publish(staged: &Path, target: &Path) -> Result<(), String> {
    let (to, from) = (wide(target), wide(staged));
    unsafe { MoveFileExW(PCWSTR(from.as_ptr()), PCWSTR(to.as_ptr()), MOVEFILE_WRITE_THROUGH) }
        .map_err(|e| format!("Cannot create {}: {}", target.display(), e.message()))
}

/// Writes a new version of `target` that is never left truncated. `write`
/// fills and verifies a staging file in the target's folder (ReplaceFileW
/// needs one volume); the file is flushed and then swapped in. `expected`
/// is the target as Peekaboo last saw it, or None for a new file. A target
/// that differs fails with `Changed`, unless `force`, and stays unchanged.
/// Returns the target's new stamp.
pub(super) fn write_verified(
    target: &Path,
    expected: Option<Stamp>,
    force: bool,
    write: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<Stamp, Failure> {
    let target = std::path::absolute(target).map_err(|e| e.to_string())?;
    if current(&target)? != expected && !force {
        return Err(Failure::Changed);
    }
    let folder = target.parent().ok_or("Choose a valid folder.")?;
    let staged = Staged(staging(folder, target.extension()));
    write(&staged.0)?;
    fs::OpenOptions::new()
        .write(true)
        .open(&staged.0)
        .and_then(|file| file.sync_all())
        .map_err(|e| format!("Cannot finish writing {}: {e}", target.display()))?;
    // Rendering a large document can take long enough for another app to
    // replace the destination after the first check. Revalidate immediately
    // before publication so that work is preserved as a conflict instead of
    // overwriting the other writer.
    let found = current(&target)?;
    if found != expected && !force {
        return Err(Failure::Changed);
    }
    match found {
        Some(_) => replace(&target, &staged.0)?,
        None => publish(&staged.0, &target)?,
    }
    Stamp::of(&target).map_err(|e| Failure::Error(e.to_string()))
}

/// Copies `source` to `staged` and checks the length, for a recipe with no
/// edits (revert), so the opened bytes come back exactly.
pub(super) fn copy_verified(source: &Path, staged: &Path) -> Result<(), String> {
    let copied = fs::copy(source, staged).map_err(|e| format!("Cannot copy {}: {e}", source.display()))?;
    match fs::metadata(source) {
        Ok(m) if m.len() == copied => Ok(()),
        _ => Err("The copy did not match the original.".into()),
    }
}

/// Copies the opened version into `folder` (the app's local data) before the
/// first change reaches the user's file, so "Revert to opened" and every
/// later save start from it. Fails with `Changed` when the file is no longer
/// the opened version or changes during the copy.
pub(super) fn snapshot(original: &Path, folder: &Path, opened: Stamp) -> Result<PathBuf, Failure> {
    let read = |path: &Path| Stamp::of(path).map_err(|e| Failure::Error(format!("Cannot read {}: {e}", path.display())));
    if read(original)? != opened {
        return Err(Failure::Changed);
    }
    let io = |e: io::Error| Failure::Error(format!("Cannot keep a copy of the opened file: {e}"));
    fs::create_dir_all(folder).map_err(io)?;
    let copy = folder.join(original.file_name().ok_or("Choose a file.")?);
    fs::copy(original, &copy).map_err(io)?;
    // A read-only original gives a read-only copy; clear it so the copy can be flushed and deleted.
    let mut permissions = fs::metadata(&copy).map_err(io)?.permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    fs::set_permissions(&copy, permissions).map_err(io)?;
    fs::OpenOptions::new().write(true).open(&copy).and_then(|f| f.sync_all()).map_err(io)?;
    if read(original)? != opened || fs::metadata(&copy).map(|m| m.len()).ok() != Some(opened.len) {
        let _ = fs::remove_file(&copy);
        return Err(Failure::Changed);
    }
    Ok(copy)
}

/// Why edits cannot go into this file, so they go to a copy; Ok when they can.
/// A file that another app holds open counts as writable: that save fails
/// and offers to try again.
pub(super) fn can_overwrite(path: &Path) -> Result<(), String> {
    let metadata = fs::metadata(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    if metadata.permissions().readonly() {
        return Err("This file is read-only.".into());
    }
    if let Err(e) = fs::OpenOptions::new().write(true).open(path) {
        if e.kind() == io::ErrorKind::PermissionDenied {
            return Err("You do not have permission to change this file.".into());
        }
    }
    let folder = path.parent().ok_or("Choose a file.")?;
    let probe = staging(folder, None);
    match fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(file) => {
            drop(file);
            let _ = fs::remove_file(&probe);
            Ok(())
        }
        Err(_) => Err("Peekaboo cannot write in this folder.".into()),
    }
}

/// %LOCALAPPDATA%\Peekaboo
pub(super) fn app_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|dir| PathBuf::from(dir).join("Peekaboo"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("pfw-disk-{name}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn replaces_the_file_keeping_its_creation_time_and_leaves_no_staging_file() {
        let dir = temp("replace");
        let target = dir.join("report.pdf");
        fs::write(&target, b"old version").unwrap();
        let before = Stamp::of(&target).unwrap();
        let after = write_verified(&target, Some(before), false, |staged| {
            fs::write(staged, b"new version, longer").map_err(|e| e.to_string())
        })
        .unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new version, longer");
        assert_eq!(after, Stamp::of(&target).unwrap());
        assert_eq!(after.created, before.created, "ReplaceFileW keeps the creation time");
        assert_eq!(names(&dir), vec!["report.pdf"]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn refuses_a_file_changed_by_another_app_unless_forced() {
        let dir = temp("changed");
        let target = dir.join("photo.jpg");
        fs::write(&target, b"opened").unwrap();
        let seen = Stamp::of(&target).unwrap();
        fs::write(&target, b"changed by another app").unwrap();
        let write = |staged: &Path| fs::write(staged, b"edits").map_err(|e| e.to_string());
        assert_eq!(write_verified(&target, Some(seen), false, write), Err(Failure::Changed));
        assert_eq!(fs::read(&target).unwrap(), b"changed by another app");
        assert_eq!(names(&dir), vec!["photo.jpg"]);
        assert!(write_verified(&target, Some(seen), true, write).is_ok(), "the user chose to overwrite");
        assert_eq!(fs::read(&target).unwrap(), b"edits");
        fs::remove_file(&target).unwrap();
        assert_eq!(write_verified(&target, Some(seen), false, write), Err(Failure::Changed), "deleted by another app");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn refuses_a_file_changed_while_the_staged_version_is_being_built() {
        let dir = temp("changed-during-write");
        let target = dir.join("large.pdf");
        fs::write(&target, b"opened").unwrap();
        let seen = Stamp::of(&target).unwrap();
        let result = write_verified(&target, Some(seen), false, |staged| {
            fs::write(staged, b"preview edits").map_err(|e| e.to_string())?;
            fs::write(&target, b"a longer version from another app").map_err(|e| e.to_string())
        });
        assert_eq!(result, Err(Failure::Changed));
        assert_eq!(fs::read(&target).unwrap(), b"a longer version from another app");
        assert_eq!(names(&dir), vec!["large.pdf"], "the rejected staged file is removed");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_write_leaves_the_file_whole_and_no_leftovers() {
        let dir = temp("failed");
        let target = dir.join("scan.pdf");
        fs::write(&target, b"original bytes").unwrap();
        let seen = Stamp::of(&target).unwrap();
        let result = write_verified(&target, Some(seen), false, |staged| {
            fs::write(staged, b"half").unwrap();
            Err("The disk is full.".into())
        });
        assert_eq!(result, Err(Failure::Error("The disk is full.".into())));
        assert_eq!(fs::read(&target).unwrap(), b"original bytes");
        assert_eq!(Stamp::of(&target).unwrap(), seen);
        assert_eq!(names(&dir), vec!["scan.pdf"]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_new_copy_never_replaces_an_existing_file() {
        let dir = temp("copy");
        let original = dir.join("lease.pdf");
        fs::write(&original, b"original").unwrap();
        fs::write(dir.join("lease (edited).pdf"), b"older copy").unwrap();
        let copy = copy_name(&original, None).unwrap();
        assert_eq!(copy, dir.join("lease (edited 2).pdf"));
        assert_eq!(copy_name(&original, Some(OsStr::new("png"))).unwrap(), dir.join("lease (edited).png"));
        let write = |staged: &Path| copy_verified(&original, staged);
        assert!(write_verified(&copy, None, false, write).is_ok());
        assert_eq!(fs::read(&copy).unwrap(), b"original");
        let taken = dir.join("lease (edited).pdf");
        assert_eq!(write_verified(&taken, None, false, write), Err(Failure::Changed), "a file appeared there");
        fs::write(dir.join("stray.tmp"), b"x").unwrap();
        assert!(publish(&dir.join("stray.tmp"), &taken).is_err());
        assert_eq!(fs::read(&taken).unwrap(), b"older copy");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn snapshot_keeps_the_opened_version_and_notices_changes() {
        let dir = temp("snapshot");
        let original = dir.join("form.pdf");
        fs::write(&original, b"opened version").unwrap();
        let mut permissions = fs::metadata(&original).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&original, permissions.clone()).unwrap();
        assert_eq!(can_overwrite(&original), Err("This file is read-only.".into()));
        let opened = Stamp::of(&original).unwrap();
        let copy = snapshot(&original, &dir.join("keep-1"), opened).unwrap();
        assert_eq!(fs::read(&copy).unwrap(), b"opened version");
        assert!(!fs::metadata(&copy).unwrap().permissions().readonly());
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(&original, permissions).unwrap();
        assert_eq!(can_overwrite(&original), Ok(()));
        fs::write(&original, b"changed by another app").unwrap();
        assert_eq!(snapshot(&original, &dir.join("keep-2"), opened), Err(Failure::Changed));
        assert!(!dir.join("keep-2").join("form.pdf").exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
