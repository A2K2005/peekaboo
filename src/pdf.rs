use crate::model::{frame_bytes, AnnotationKind, Frame, PdfEdit};
use std::{
    collections::HashMap,
    ffi::c_void,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    marker::PhantomData,
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt},
    path::{Path, PathBuf},
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};
use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
use windows::{
    core::{PCSTR, PCWSTR},
    Win32::{
        Foundation::{FreeLibrary, HMODULE},
        System::LibraryLoader::{
            GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        },
    },
};

static ENGINE_ACTIVE: AtomicBool = AtomicBool::new(false);
pub const PASSWORD_REQUIRED: &str = "This PDF requires a password.";
struct Password(Vec<u8>);
impl Drop for Password {
    fn drop(&mut self) {
        for byte in &mut self.0 {
            unsafe {
                std::ptr::write_volatile(byte, 0);
            }
        }
        std::sync::atomic::compiler_fence(Ordering::SeqCst);
    }
}
#[derive(Clone, Debug, PartialEq)]
struct FileStamp {
    length: u64,
    modified: Option<std::time::SystemTime>,
    created: Option<std::time::SystemTime>,
}
fn stamp(metadata: &std::fs::Metadata) -> FileStamp {
    FileStamp {
        length: metadata.len(),
        modified: metadata.modified().ok(),
        created: metadata.created().ok(),
    }
}
type Handle = *mut c_void;
#[repr(C)]
#[derive(Clone, Copy)]
struct Point {
    x: f32,
    y: f32,
}
#[repr(C)]
struct Rect {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}
#[repr(C)]
struct Quad {
    values: [f32; 8],
}
#[repr(C)]
struct PdfSystemTime {
    year: u16,
    month: u16,
    day_of_week: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    millisecond: u16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FormFieldKind {
    Text,
    Checkbox,
    Unsupported,
}
#[derive(Clone, Debug)]
pub struct FormField {
    pub page: u32,
    pub annotation_index: u32,
    pub name: String,
    pub value: String,
    pub kind: FormFieldKind,
    pub read_only: bool,
}

// Exact version-1 prefix from the pinned fpdf_formfill.h. Version 2 callbacks are not accessed.
#[repr(C)]
struct FormInfo {
    version: i32,
    release: Option<unsafe extern "C" fn(*mut FormInfo)>,
    invalidate: Option<unsafe extern "C" fn(*mut FormInfo, Handle, f64, f64, f64, f64)>,
    selected: Option<unsafe extern "C" fn(*mut FormInfo, Handle, f64, f64, f64, f64)>,
    cursor: Option<unsafe extern "C" fn(*mut FormInfo, i32)>,
    timer:
        Option<unsafe extern "C" fn(*mut FormInfo, i32, Option<unsafe extern "C" fn(i32)>) -> i32>,
    kill_timer: Option<unsafe extern "C" fn(*mut FormInfo, i32)>,
    local_time: Option<unsafe extern "C" fn(*mut FormInfo) -> PdfSystemTime>,
    change: Option<unsafe extern "C" fn(*mut FormInfo)>,
    get_page: Option<unsafe extern "C" fn(*mut FormInfo, Handle, i32) -> Handle>,
    current_page: Option<unsafe extern "C" fn(*mut FormInfo, Handle) -> Handle>,
    rotation: Option<unsafe extern "C" fn(*mut FormInfo, Handle) -> i32>,
    named_action: Option<unsafe extern "C" fn(*mut FormInfo, *const u8)>,
    focus: Option<unsafe extern "C" fn(*mut FormInfo, *const u16, u32, i32)>,
    uri: Option<unsafe extern "C" fn(*mut FormInfo, *const u8)>,
    goto: Option<unsafe extern "C" fn(*mut FormInfo, i32, i32, *mut f32, i32)>,
    javascript: *mut c_void,
    // Host-owned context follows the complete v1 prefix.
    page: Handle,
    index: i32,
    page_rotation: i32,
}
unsafe extern "C" fn form_invalidate(_: *mut FormInfo, _: Handle, _: f64, _: f64, _: f64, _: f64) {}
unsafe extern "C" fn form_cursor(_: *mut FormInfo, _: i32) {}
unsafe extern "C" fn form_timer(
    _: *mut FormInfo,
    _: i32,
    _: Option<unsafe extern "C" fn(i32)>,
) -> i32 {
    0
}
unsafe extern "C" fn form_page(info: *mut FormInfo, _: Handle, index: i32) -> Handle {
    if index == (*info).index {
        (*info).page
    } else {
        std::ptr::null_mut()
    }
}
unsafe extern "C" fn form_current(info: *mut FormInfo, _: Handle) -> Handle {
    (*info).page
}
unsafe extern "C" fn form_rotation(info: *mut FormInfo, _: Handle) -> i32 {
    (*info).page_rotation
}

struct FormSession<'a> {
    handle: Handle,
    info: Box<FormInfo>,
    api: &'a Api,
}
impl Drop for FormSession<'_> {
    fn drop(&mut self) {
        unsafe {
            (self.api.form_blur)(self.handle);
            (self.api.form_before)(self.info.page, self.handle);
            (self.api.form_exit)(self.handle);
        }
    }
}

#[repr(C)]
struct FileAccess {
    length: u32,
    read: unsafe extern "C" fn(*mut c_void, u32, *mut u8, u32) -> i32,
    context: *mut c_void,
}

// The callback owns no memory and never unwinds through the native ABI.
unsafe extern "C" fn read_block(
    context: *mut c_void,
    position: u32,
    buffer: *mut u8,
    size: u32,
) -> i32 {
    if context.is_null() || (buffer.is_null() && size != 0) {
        return 0;
    }
    if size == 0 {
        return 1;
    }
    let file = &mut *(context as *mut File);
    let bytes = std::slice::from_raw_parts_mut(buffer, size as usize);
    (file.seek(SeekFrom::Start(position as u64)).is_ok() && file.read_exact(bytes).is_ok()) as i32
}

struct Library(HMODULE);
impl Drop for Library {
    fn drop(&mut self) {
        unsafe {
            let _ = FreeLibrary(self.0);
        }
    }
}

struct Api {
    init: unsafe extern "system" fn(),
    destroy: unsafe extern "system" fn(),
    load: unsafe extern "system" fn(*mut FileAccess, *const u8) -> Handle,
    close_doc: unsafe extern "system" fn(Handle),
    count: unsafe extern "system" fn(Handle) -> i32,
    load_page: unsafe extern "system" fn(Handle, i32) -> Handle,
    close_page: unsafe extern "system" fn(Handle),
    width: unsafe extern "system" fn(Handle) -> f32,
    height: unsafe extern "system" fn(Handle) -> f32,
    bitmap: unsafe extern "system" fn(i32, i32, i32, *mut c_void, i32) -> Handle,
    close_bitmap: unsafe extern "system" fn(Handle),
    render: unsafe extern "system" fn(Handle, Handle, i32, i32, i32, i32, i32, i32),
    error: unsafe extern "system" fn() -> u32,
    rotation: unsafe extern "system" fn(Handle) -> i32,
    set_rotation: unsafe extern "system" fn(Handle, i32),
    delete: unsafe extern "system" fn(Handle, i32),
    create: unsafe extern "system" fn() -> Handle,
    import: unsafe extern "system" fn(Handle, Handle, *const i32, u32, i32) -> i32,
    form_type: unsafe extern "system" fn(Handle) -> i32,
    save: unsafe extern "system" fn(Handle, *mut FileWrite, u32) -> i32,
    text_load: unsafe extern "system" fn(Handle) -> Handle,
    text_close: unsafe extern "system" fn(Handle),
    text_count: unsafe extern "system" fn(Handle) -> i32,
    text_get: unsafe extern "system" fn(Handle, i32, i32, *mut u16) -> i32,
    device_to_page: unsafe extern "system" fn(
        Handle,
        i32,
        i32,
        i32,
        i32,
        i32,
        i32,
        i32,
        *mut f64,
        *mut f64,
    ) -> i32,
    annot_create: unsafe extern "system" fn(Handle, i32) -> Handle,
    annot_close: unsafe extern "system" fn(Handle),
    annot_rect: unsafe extern "system" fn(Handle, *const Rect) -> i32,
    annot_color: unsafe extern "system" fn(Handle, i32, u32, u32, u32, u32) -> i32,
    annot_flags: unsafe extern "system" fn(Handle, i32) -> i32,
    annot_border: unsafe extern "system" fn(Handle, f32, f32, f32) -> i32,
    annot_string: unsafe extern "system" fn(Handle, *const u8, *const u16) -> i32,
    annot_ink: unsafe extern "system" fn(Handle, *const Point, usize) -> i32,
    annot_quad: unsafe extern "system" fn(Handle, *const Quad) -> i32,
    annot_ap: unsafe extern "system" fn(Handle, i32, *mut u16, u32) -> u32,
    annot_count: unsafe extern "system" fn(Handle) -> i32,
    annot_get: unsafe extern "system" fn(Handle, i32) -> Handle,
    form_init: unsafe extern "system" fn(Handle, *mut FormInfo) -> Handle,
    form_exit: unsafe extern "system" fn(Handle),
    form_after: unsafe extern "system" fn(Handle, Handle),
    form_before: unsafe extern "system" fn(Handle, Handle),
    form_focus: unsafe extern "system" fn(Handle, Handle) -> i32,
    form_blur: unsafe extern "system" fn(Handle) -> i32,
    form_select: unsafe extern "system" fn(Handle, Handle) -> i32,
    form_replace: unsafe extern "system" fn(Handle, Handle, *const u16),
    form_char: unsafe extern "system" fn(Handle, Handle, i32, i32) -> i32,
    field_type: unsafe extern "system" fn(Handle, Handle) -> i32,
    field_flags: unsafe extern "system" fn(Handle, Handle) -> i32,
    field_name: unsafe extern "system" fn(Handle, Handle, *mut u16, u32) -> u32,
    field_value: unsafe extern "system" fn(Handle, Handle, *mut u16, u32) -> u32,
    field_checked: unsafe extern "system" fn(Handle, Handle) -> i32,
    form_draw: unsafe extern "system" fn(Handle, Handle, Handle, i32, i32, i32, i32, i32, i32),
    page_new: unsafe extern "system" fn(Handle, i32, f64, f64) -> Handle,
    image_new: unsafe extern "system" fn(Handle) -> Handle,
    object_destroy: unsafe extern "system" fn(Handle),
    image_bitmap: unsafe extern "system" fn(*mut Handle, i32, Handle, Handle) -> i32,
    image_matrix: unsafe extern "system" fn(Handle, f64, f64, f64, f64, f64, f64) -> i32,
    page_insert: unsafe extern "system" fn(Handle, Handle) -> i32,
    page_generate: unsafe extern "system" fn(Handle) -> i32,
    security_revision: unsafe extern "system" fn(Handle) -> i32,
    permissions: unsafe extern "system" fn(Handle) -> u32,
    move_pages: unsafe extern "system" fn(Handle, *const i32, u32, i32) -> i32,
    crop_box: unsafe extern "system" fn(Handle, f32, f32, f32, f32),
}

#[repr(C)]
struct FileWrite {
    version: i32,
    write: unsafe extern "C" fn(*mut FileWrite, *const u8, u32) -> i32,
    file: File,
}
unsafe extern "C" fn write_block(writer: *mut FileWrite, data: *const u8, length: u32) -> i32 {
    if writer.is_null() || (data.is_null() && length != 0) {
        return 0;
    }
    if length == 0 {
        return 1;
    }
    (*writer)
        .file
        .write_all(std::slice::from_raw_parts(data, length as usize))
        .is_ok() as i32
}
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct NativeHandle {
    handle: Handle,
    close: unsafe extern "system" fn(Handle),
}
impl Drop for NativeHandle {
    fn drop(&mut self) {
        unsafe {
            (self.close)(self.handle);
        }
    }
}

struct Document {
    native: NativeHandle,
    // Both allocations must outlive the native document and keep stable addresses.
    _access: Box<FileAccess>,
    _file: Box<File>,
    path: PathBuf,
    edits: Vec<PdfEdit>,
    stamp: FileStamp,
}

pub struct PdfEngine {
    document: Option<Document>,
    passwords: HashMap<PathBuf, Password>,
    revisions: HashMap<PathBuf, FileStamp>,
    api: Api,
    _library: Library,
    // PDFium is process-global and not thread-safe. The engine cannot leave its worker.
    _thread: PhantomData<Rc<()>>,
}

impl PdfEngine {
    pub fn new() -> Result<Self, String> {
        if ENGINE_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("Only one PDF worker can run at a time.".into());
        }
        let result = Self::load();
        if result.is_err() {
            ENGINE_ACTIVE.store(false, Ordering::Release);
        }
        result
    }

    fn load() -> Result<Self, String> {
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        let directory = executable
            .parent()
            .ok_or("Cannot locate the app directory.")?;
        let mut candidates = vec![directory.join("pdfium.dll")];
        // Test binaries run from target/<profile>/deps.
        let profile = if directory.file_name().is_some_and(|name| name == "deps") {
            directory.parent()
        } else {
            Some(directory)
        };
        if let Some(profile) = profile {
            candidates.push(profile.join("pdfium.dll"));
            if let Some(project) = profile.parent().and_then(Path::parent) {
                candidates.push(project.join("runtime/pdfium.dll"));
            }
        }
        let path = candidates
            .into_iter()
            .find(|p| p.is_file())
            .ok_or("PDF support is missing. Run tools/fetch-pdfium.ps1 and restart the app.")?;
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            let library = Library(
                LoadLibraryExW(
                    PCWSTR(wide.as_ptr()),
                    None,
                    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
                )
                .map_err(|e| format!("Cannot load PDF support: {e}"))?,
            );
            macro_rules! symbol {
                ($name:literal, $type:ty) => {{
                    let address = GetProcAddress(library.0, PCSTR(concat!($name, "\0").as_ptr()))
                        .ok_or(concat!("PDF runtime is missing ", $name))?;
                    std::mem::transmute::<unsafe extern "system" fn() -> isize, $type>(address)
                }};
            }
            let api = Api {
                init: symbol!("FPDF_InitLibrary", unsafe extern "system" fn()),
                destroy: symbol!("FPDF_DestroyLibrary", unsafe extern "system" fn()),
                load: symbol!(
                    "FPDF_LoadCustomDocument",
                    unsafe extern "system" fn(*mut FileAccess, *const u8) -> Handle
                ),
                close_doc: symbol!("FPDF_CloseDocument", unsafe extern "system" fn(Handle)),
                count: symbol!(
                    "FPDF_GetPageCount",
                    unsafe extern "system" fn(Handle) -> i32
                ),
                load_page: symbol!(
                    "FPDF_LoadPage",
                    unsafe extern "system" fn(Handle, i32) -> Handle
                ),
                close_page: symbol!("FPDF_ClosePage", unsafe extern "system" fn(Handle)),
                width: symbol!(
                    "FPDF_GetPageWidthF",
                    unsafe extern "system" fn(Handle) -> f32
                ),
                height: symbol!(
                    "FPDF_GetPageHeightF",
                    unsafe extern "system" fn(Handle) -> f32
                ),
                bitmap: symbol!(
                    "FPDFBitmap_CreateEx",
                    unsafe extern "system" fn(i32, i32, i32, *mut c_void, i32) -> Handle
                ),
                close_bitmap: symbol!("FPDFBitmap_Destroy", unsafe extern "system" fn(Handle)),
                render: symbol!(
                    "FPDF_RenderPageBitmap",
                    unsafe extern "system" fn(Handle, Handle, i32, i32, i32, i32, i32, i32)
                ),
                error: symbol!("FPDF_GetLastError", unsafe extern "system" fn() -> u32),
                rotation: symbol!(
                    "FPDFPage_GetRotation",
                    unsafe extern "system" fn(Handle) -> i32
                ),
                set_rotation: symbol!(
                    "FPDFPage_SetRotation",
                    unsafe extern "system" fn(Handle, i32)
                ),
                delete: symbol!("FPDFPage_Delete", unsafe extern "system" fn(Handle, i32)),
                create: symbol!(
                    "FPDF_CreateNewDocument",
                    unsafe extern "system" fn() -> Handle
                ),
                import: symbol!(
                    "FPDF_ImportPagesByIndex",
                    unsafe extern "system" fn(Handle, Handle, *const i32, u32, i32) -> i32
                ),
                form_type: symbol!("FPDF_GetFormType", unsafe extern "system" fn(Handle) -> i32),
                save: symbol!(
                    "FPDF_SaveAsCopy",
                    unsafe extern "system" fn(Handle, *mut FileWrite, u32) -> i32
                ),
                text_load: symbol!(
                    "FPDFText_LoadPage",
                    unsafe extern "system" fn(Handle) -> Handle
                ),
                text_close: symbol!("FPDFText_ClosePage", unsafe extern "system" fn(Handle)),
                text_count: symbol!(
                    "FPDFText_CountChars",
                    unsafe extern "system" fn(Handle) -> i32
                ),
                text_get: symbol!(
                    "FPDFText_GetText",
                    unsafe extern "system" fn(Handle, i32, i32, *mut u16) -> i32
                ),
                device_to_page: symbol!(
                    "FPDF_DeviceToPage",
                    unsafe extern "system" fn(
                        Handle,
                        i32,
                        i32,
                        i32,
                        i32,
                        i32,
                        i32,
                        i32,
                        *mut f64,
                        *mut f64,
                    ) -> i32
                ),
                annot_create: symbol!(
                    "FPDFPage_CreateAnnot",
                    unsafe extern "system" fn(Handle, i32) -> Handle
                ),
                annot_close: symbol!("FPDFPage_CloseAnnot", unsafe extern "system" fn(Handle)),
                annot_rect: symbol!(
                    "FPDFAnnot_SetRect",
                    unsafe extern "system" fn(Handle, *const Rect) -> i32
                ),
                annot_color: symbol!(
                    "FPDFAnnot_SetColor",
                    unsafe extern "system" fn(Handle, i32, u32, u32, u32, u32) -> i32
                ),
                annot_flags: symbol!(
                    "FPDFAnnot_SetFlags",
                    unsafe extern "system" fn(Handle, i32) -> i32
                ),
                annot_border: symbol!(
                    "FPDFAnnot_SetBorder",
                    unsafe extern "system" fn(Handle, f32, f32, f32) -> i32
                ),
                annot_string: symbol!(
                    "FPDFAnnot_SetStringValue",
                    unsafe extern "system" fn(Handle, *const u8, *const u16) -> i32
                ),
                annot_ink: symbol!(
                    "FPDFAnnot_AddInkStroke",
                    unsafe extern "system" fn(Handle, *const Point, usize) -> i32
                ),
                annot_quad: symbol!(
                    "FPDFAnnot_AppendAttachmentPoints",
                    unsafe extern "system" fn(Handle, *const Quad) -> i32
                ),
                annot_ap: symbol!(
                    "FPDFAnnot_GetAP",
                    unsafe extern "system" fn(Handle, i32, *mut u16, u32) -> u32
                ),
                annot_count: symbol!(
                    "FPDFPage_GetAnnotCount",
                    unsafe extern "system" fn(Handle) -> i32
                ),
                annot_get: symbol!(
                    "FPDFPage_GetAnnot",
                    unsafe extern "system" fn(Handle, i32) -> Handle
                ),
                form_init: symbol!(
                    "FPDFDOC_InitFormFillEnvironment",
                    unsafe extern "system" fn(Handle, *mut FormInfo) -> Handle
                ),
                form_exit: symbol!(
                    "FPDFDOC_ExitFormFillEnvironment",
                    unsafe extern "system" fn(Handle)
                ),
                form_after: symbol!(
                    "FORM_OnAfterLoadPage",
                    unsafe extern "system" fn(Handle, Handle)
                ),
                form_before: symbol!(
                    "FORM_OnBeforeClosePage",
                    unsafe extern "system" fn(Handle, Handle)
                ),
                form_focus: symbol!(
                    "FORM_SetFocusedAnnot",
                    unsafe extern "system" fn(Handle, Handle) -> i32
                ),
                form_blur: symbol!(
                    "FORM_ForceToKillFocus",
                    unsafe extern "system" fn(Handle) -> i32
                ),
                form_select: symbol!(
                    "FORM_SelectAllText",
                    unsafe extern "system" fn(Handle, Handle) -> i32
                ),
                form_replace: symbol!(
                    "FORM_ReplaceSelection",
                    unsafe extern "system" fn(Handle, Handle, *const u16)
                ),
                form_char: symbol!(
                    "FORM_OnChar",
                    unsafe extern "system" fn(Handle, Handle, i32, i32) -> i32
                ),
                field_type: symbol!(
                    "FPDFAnnot_GetFormFieldType",
                    unsafe extern "system" fn(Handle, Handle) -> i32
                ),
                field_flags: symbol!(
                    "FPDFAnnot_GetFormFieldFlags",
                    unsafe extern "system" fn(Handle, Handle) -> i32
                ),
                field_name: symbol!(
                    "FPDFAnnot_GetFormFieldName",
                    unsafe extern "system" fn(Handle, Handle, *mut u16, u32) -> u32
                ),
                field_value: symbol!(
                    "FPDFAnnot_GetFormFieldValue",
                    unsafe extern "system" fn(Handle, Handle, *mut u16, u32) -> u32
                ),
                field_checked: symbol!(
                    "FPDFAnnot_IsChecked",
                    unsafe extern "system" fn(Handle, Handle) -> i32
                ),
                form_draw: symbol!(
                    "FPDF_FFLDraw",
                    unsafe extern "system" fn(Handle, Handle, Handle, i32, i32, i32, i32, i32, i32)
                ),
                page_new: symbol!(
                    "FPDFPage_New",
                    unsafe extern "system" fn(Handle, i32, f64, f64) -> Handle
                ),
                image_new: symbol!(
                    "FPDFPageObj_NewImageObj",
                    unsafe extern "system" fn(Handle) -> Handle
                ),
                object_destroy: symbol!("FPDFPageObj_Destroy", unsafe extern "system" fn(Handle)),
                image_bitmap: symbol!(
                    "FPDFImageObj_SetBitmap",
                    unsafe extern "system" fn(*mut Handle, i32, Handle, Handle) -> i32
                ),
                image_matrix: symbol!(
                    "FPDFImageObj_SetMatrix",
                    unsafe extern "system" fn(Handle, f64, f64, f64, f64, f64, f64) -> i32
                ),
                page_insert: symbol!(
                    "FPDFPage_InsertObject",
                    unsafe extern "system" fn(Handle, Handle) -> i32
                ),
                page_generate: symbol!(
                    "FPDFPage_GenerateContent",
                    unsafe extern "system" fn(Handle) -> i32
                ),
                security_revision: symbol!(
                    "FPDF_GetSecurityHandlerRevision",
                    unsafe extern "system" fn(Handle) -> i32
                ),
                permissions: symbol!(
                    "FPDF_GetDocPermissions",
                    unsafe extern "system" fn(Handle) -> u32
                ),
                move_pages: symbol!(
                    "FPDF_MovePages",
                    unsafe extern "system" fn(Handle, *const i32, u32, i32) -> i32
                ),
                crop_box: symbol!(
                    "FPDFPage_SetCropBox",
                    unsafe extern "system" fn(Handle, f32, f32, f32, f32)
                ),
            };
            (api.init)();
            Ok(Self {
                document: None,
                passwords: HashMap::new(),
                revisions: HashMap::new(),
                api,
                _library: library,
                _thread: PhantomData,
            })
        }
    }

    fn open(&self, path: &Path, edits: &[PdfEdit]) -> Result<Document, String> {
        let path = path
            .canonicalize()
            .map_err(|e| format!("Cannot open PDF: {e}"))?;
        self.open_using(
            &path,
            edits,
            self.passwords
                .get(&path)
                .map(|password| password.0.as_slice()),
        )
    }

    fn open_using(
        &self,
        path: &Path,
        edits: &[PdfEdit],
        password: Option<&[u8]>,
    ) -> Result<Document, String> {
        let path = path.to_path_buf();
        unsafe {
            // Keep this revision stable against in-place writes; atomic replacement remains possible.
            let mut file = Box::new(
                OpenOptions::new()
                    .read(true)
                    .share_mode(1 | 4)
                    .open(&path)
                    .map_err(|e| format!("Cannot open PDF: {e}"))?,
            );
            let metadata = file.metadata().map_err(|e| e.to_string())?;
            let source_stamp = stamp(&metadata);
            if !edits.is_empty()
                && self
                    .revisions
                    .get(&path)
                    .is_some_and(|known| known != &source_stamp)
            {
                return Err(
                    "This PDF changed outside Preview. Reopen it before applying or saving edits."
                        .into(),
                );
            }
            let length = metadata.len();
            if length == 0 || length > u32::MAX as u64 {
                return Err("This PDF is empty or exceeds the 4 GiB preview limit.".into());
            }
            let mut access = Box::new(FileAccess {
                length: length as u32,
                read: read_block,
                context: (&mut *file as *mut File).cast(),
            });
            let handle = (self.api.load)(
                &mut *access,
                password.map_or(std::ptr::null(), |p| p.as_ptr()),
            );
            if handle.is_null() {
                return Err(match (self.api.error)() {
                    4 => PASSWORD_REQUIRED,
                    5 => "This PDF uses unsupported security settings.",
                    _ => "This PDF is damaged or uses an unsupported format.",
                }
                .into());
            }
            let document = Document {
                native: NativeHandle {
                    handle,
                    close: self.api.close_doc,
                },
                _access: access,
                _file: file,
                path,
                edits: edits.to_vec(),
                stamp: source_stamp,
            };
            let permissions = (self.api.permissions)(handle);
            for edit in edits {
                let allowed = match edit {
                    PdfEdit::FillField { .. } => {
                        permissions & (1 << 8) != 0 || permissions & (1 << 5) != 0
                    }
                    PdfEdit::Annotate { .. } => permissions & (1 << 5) != 0,
                    _ => permissions & (1 << 3) != 0 || permissions & (1 << 10) != 0,
                };
                if !allowed {
                    return Err("This PDF's permissions do not allow this edit.".into());
                }
                let count = (self.api.count)(handle);
                if count <= 0 {
                    return Err("This PDF has no editable pages.".into());
                }
                let page_index = match *edit {
                    PdfEdit::RotateRight { page }
                    | PdfEdit::Delete { page }
                    | PdfEdit::Annotate { page, .. }
                    | PdfEdit::FillField { page, .. }
                    | PdfEdit::Crop { page, .. } => page,
                    PdfEdit::Move { from, to } => {
                        if to >= count as u32 {
                            return Err("Choose a page position within this PDF.".into());
                        }
                        from
                    }
                    PdfEdit::InsertBlank { at } => {
                        if at > count as u32 {
                            return Err("Choose an insertion position within this PDF.".into());
                        }
                        at.min(count as u32 - 1)
                    }
                };
                if count <= 0 || page_index >= count as u32 {
                    return Err("This PDF page does not exist.".into());
                }
                match *edit {
                    PdfEdit::Move { from, to } => {
                        let index = from as i32;
                        if from != to && (self.api.move_pages)(handle, &index, 1, to as i32) == 0 {
                            return Err(
                                "Could not move this page. The original document is unchanged."
                                    .into(),
                            );
                        }
                    }
                    PdfEdit::InsertBlank { at } => {
                        let neighbor = self.page(handle, page_index)?;
                        let width = dimension((self.api.width)(neighbor.handle))?;
                        let height = dimension((self.api.height)(neighbor.handle))?;
                        drop(neighbor);
                        let inserted =
                            (self.api.page_new)(handle, at as i32, width as f64, height as f64);
                        if inserted.is_null() {
                            return Err("Cannot insert a blank page.".into());
                        }
                        let _page = NativeHandle {
                            handle: inserted,
                            close: self.api.close_page,
                        };
                        if (self.api.count)(handle) != count + 1 {
                            return Err("The blank page was not inserted.".into());
                        }
                    }
                    PdfEdit::Crop {
                        left,
                        top,
                        right,
                        bottom,
                        ..
                    } => {
                        if [left, top, right, bottom]
                            .iter()
                            .any(|n| !n.is_finite() || !(0.0..=1.0).contains(n))
                            || left >= right
                            || top >= bottom
                        {
                            return Err("Select a crop rectangle inside this PDF page.".into());
                        }
                        let page = self.page(handle, page_index)?;
                        let mut points = Vec::with_capacity(4);
                        for (x, y) in [(left, top), (right, top), (left, bottom), (right, bottom)] {
                            let (mut px, mut py) = (0.0, 0.0);
                            if (self.api.device_to_page)(
                                page.handle,
                                0,
                                0,
                                100_000,
                                100_000,
                                0,
                                (x * 100_000.0).round() as i32,
                                (y * 100_000.0).round() as i32,
                                &mut px,
                                &mut py,
                            ) == 0
                                || !px.is_finite()
                                || !py.is_finite()
                            {
                                return Err("Cannot map the crop to this page.".into());
                            }
                            points.push((px as f32, py as f32));
                        }
                        let l = points.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
                        let r = points.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max);
                        let b = points.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
                        let t = points.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max);
                        if r - l < 1.0 || t - b < 1.0 {
                            return Err("The PDF crop selection is too small.".into());
                        }
                        (self.api.crop_box)(page.handle, l, b, r, t);
                    }
                    PdfEdit::RotateRight { .. } => {
                        let page = self.page(handle, page_index)?;
                        let rotation = (self.api.rotation)(page.handle);
                        if rotation < 0 {
                            return Err("Cannot read page rotation.".into());
                        }
                        (self.api.set_rotation)(page.handle, (rotation + 1) % 4);
                    }
                    PdfEdit::Delete { .. } => {
                        if count <= 1 {
                            return Err("A PDF must keep at least one page.".into());
                        }
                        (self.api.delete)(handle, page_index as i32);
                        if (self.api.count)(handle) != count - 1 {
                            return Err("Cannot delete this PDF page.".into());
                        }
                    }
                    PdfEdit::FillField {
                        annotation_index,
                        ref value,
                        ..
                    } => {
                        let page = self.page(handle, page_index)?;
                        self.fill_field(handle, page.handle, page_index, annotation_index, value)?;
                    }
                    PdfEdit::Annotate {
                        kind,
                        ref points,
                        ref text,
                        ..
                    } => {
                        let page = self.page(handle, page_index)?;
                        self.annotate(page.handle, kind, points, text)?;
                    }
                }
            }
            Ok(document)
        }
    }

    fn page(&self, document: Handle, index: u32) -> Result<NativeHandle, String> {
        unsafe {
            let count = (self.api.count)(document);
            if count <= 0 || index >= count as u32 {
                return Err("This PDF page does not exist.".into());
            }
            let handle = (self.api.load_page)(document, index as i32);
            if handle.is_null() {
                return Err("This PDF page could not be read.".into());
            }
            Ok(NativeHandle {
                handle,
                close: self.api.close_page,
            })
        }
    }

    fn form_session(
        &self,
        document: Handle,
        page: Handle,
        index: u32,
    ) -> Result<FormSession<'_>, String> {
        unsafe {
            if (self.api.form_type)(document) != 1 {
                return Err("This document does not contain supported AcroForm fields. XFA forms require another reader.".into());
            }
            let mut info = Box::new(FormInfo {
                version: 1,
                release: None,
                invalidate: Some(form_invalidate),
                selected: None,
                cursor: Some(form_cursor),
                timer: Some(form_timer),
                kill_timer: Some(form_cursor),
                local_time: None,
                change: None,
                get_page: Some(form_page),
                current_page: Some(form_current),
                rotation: Some(form_rotation),
                named_action: None,
                focus: None,
                uri: None,
                goto: None,
                javascript: std::ptr::null_mut(),
                page,
                index: index as i32,
                page_rotation: (self.api.rotation)(page),
            });
            let handle = (self.api.form_init)(document, &mut *info);
            if handle.is_null() {
                return Err("Cannot initialize PDF form fields.".into());
            }
            (self.api.form_after)(page, handle);
            Ok(FormSession {
                handle,
                info,
                api: &self.api,
            })
        }
    }

    fn field_string(
        &self,
        form: Handle,
        annotation: Handle,
        read: unsafe extern "system" fn(Handle, Handle, *mut u16, u32) -> u32,
    ) -> Result<String, String> {
        unsafe {
            let size = read(form, annotation, std::ptr::null_mut(), 0);
            if size < 2 || size > 2_000_000 || size % 2 != 0 {
                return Err("Cannot read this form field.".into());
            }
            let mut buffer = vec![0u16; size as usize / 2];
            if read(form, annotation, buffer.as_mut_ptr(), size) != size {
                return Err("Cannot read this form field.".into());
            }
            Ok(String::from_utf16_lossy(&buffer[..buffer.len() - 1]))
        }
    }

    pub fn form_fields(
        &mut self,
        path: &Path,
        page_index: u32,
        edits: &[PdfEdit],
    ) -> Result<Vec<FormField>, String> {
        self.ensure(path, edits)?;
        let doc = self
            .document
            .as_ref()
            .ok_or("No PDF is open.")?
            .native
            .handle;
        let page = self.page(doc, page_index)?;
        let form = self.form_session(doc, page.handle, page_index)?;
        let mut fields = Vec::new();
        unsafe {
            let count = (self.api.annot_count)(page.handle);
            for index in 0..count {
                let handle = (self.api.annot_get)(page.handle, index);
                if handle.is_null() {
                    return Err("Cannot read a page annotation.".into());
                }
                let annotation = NativeHandle {
                    handle,
                    close: self.api.annot_close,
                };
                let kind = match (self.api.field_type)(form.handle, handle) {
                    -1 | 0 => continue,
                    6 => FormFieldKind::Text,
                    2 => FormFieldKind::Checkbox,
                    _ => FormFieldKind::Unsupported,
                };
                let flags = (self.api.field_flags)(form.handle, handle);
                let value = if kind == FormFieldKind::Checkbox {
                    ((self.api.field_checked)(form.handle, handle) != 0).to_string()
                } else {
                    self.field_string(form.handle, handle, self.api.field_value)?
                };
                let name =
                    self.field_string(form.handle, annotation.handle, self.api.field_name)?;
                fields.push(FormField {
                    page: page_index,
                    annotation_index: index as u32,
                    name,
                    value,
                    kind,
                    read_only: flags < 0 || flags & 1 != 0,
                });
            }
        }
        Ok(fields)
    }

    fn fill_field(
        &self,
        document: Handle,
        page: Handle,
        page_index: u32,
        index: u32,
        value: &str,
    ) -> Result<(), String> {
        if value.len() > 32_000 || value.contains('\0') {
            return Err("Form text is too long or contains invalid characters.".into());
        }
        let form = self.form_session(document, page, page_index)?;
        unsafe {
            let count = (self.api.annot_count)(page);
            if count <= 0 || index >= count as u32 {
                return Err("The form field no longer exists.".into());
            }
            let handle = (self.api.annot_get)(page, index as i32);
            if handle.is_null() {
                return Err("Cannot read the form field.".into());
            }
            let annotation = NativeHandle {
                handle,
                close: self.api.annot_close,
            };
            let flags = (self.api.field_flags)(form.handle, handle);
            if flags < 0 || flags & 1 != 0 {
                return Err("This form field is read-only.".into());
            }
            let kind = (self.api.field_type)(form.handle, handle);
            if kind != 6 && kind != 2 {
                return Err(
                    "This field type is not yet editable. Its original value is preserved.".into(),
                );
            }
            if (self.api.form_focus)(form.handle, annotation.handle) == 0 {
                return Err("Cannot focus this form field.".into());
            }
            if kind == 6 {
                if (self.api.form_select)(form.handle, page) == 0 {
                    return Err("Cannot select the field text.".into());
                }
                let text: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
                (self.api.form_replace)(form.handle, page, text.as_ptr());
            } else {
                let desired = match value {
                    "true" => true,
                    "false" => false,
                    _ => return Err("Checkbox values must be true or false.".into()),
                };
                if desired != ((self.api.field_checked)(form.handle, handle) != 0)
                    && (self.api.form_char)(form.handle, page, 32, 0) == 0
                {
                    return Err("Cannot change this checkbox.".into());
                }
            }
            (self.api.form_blur)(form.handle);
            let actual = if kind == 2 {
                ((self.api.field_checked)(form.handle, handle) != 0).to_string()
            } else {
                self.field_string(form.handle, handle, self.api.field_value)?
            };
            if actual != value {
                return Err(
                    "The form rejected or shortened this value. Check its input limits.".into(),
                );
            }
        }
        Ok(())
    }

    fn ensure(&mut self, path: &Path, edits: &[PdfEdit]) -> Result<(), String> {
        let path = path
            .canonicalize()
            .map_err(|e| format!("Cannot open PDF: {e}"))?;
        let current = stamp(&std::fs::metadata(&path).map_err(|e| e.to_string())?);
        if !edits.is_empty()
            && self
                .revisions
                .get(&path)
                .is_some_and(|known| known != &current)
        {
            return Err(
                "This PDF changed outside Preview. Reopen it before applying or saving edits."
                    .into(),
            );
        }
        if !self
            .document
            .as_ref()
            .is_some_and(|d| d.path == path && d.edits == edits && d.stamp == current)
        {
            self.document = Some(self.open(&path, edits)?);
        }
        if let Some(document) = self.document.as_ref() {
            self.revisions.insert(path, document.stamp.clone());
        }
        Ok(())
    }

    pub fn set_password(&mut self, path: &Path, password: String) -> Result<(), String> {
        let path = path
            .canonicalize()
            .map_err(|e| format!("Cannot open PDF: {e}"))?;
        let mut secret = Password(password.into_bytes());
        if secret.0.len() > 32_000 || secret.0.contains(&0) {
            return Err("The password is too long or contains invalid characters.".into());
        }
        secret.0.push(0);
        // Validate before replacing a previously accepted password or current document.
        let document = self.open_using(&path, &[], Some(&secret.0))?;
        self.revisions
            .entry(path.clone())
            .or_insert_with(|| document.stamp.clone());
        self.passwords.insert(path, secret);
        self.document = Some(document);
        Ok(())
    }

    pub fn render(
        &mut self,
        path: &Path,
        page_index: u32,
        max_width: u32,
        max_height: u32,
    ) -> Result<Frame, String> {
        self.render_edited(path, page_index, max_width, max_height, &[])
    }

    pub fn render_edited(
        &mut self,
        path: &Path,
        page_index: u32,
        max_width: u32,
        max_height: u32,
        edits: &[PdfEdit],
    ) -> Result<Frame, String> {
        self.render_with_flags(path, page_index, max_width, max_height, edits, 1)
    }

    pub fn render_for_print(
        &mut self,
        path: &Path,
        page_index: u32,
        max_width: u32,
        max_height: u32,
        edits: &[PdfEdit],
    ) -> Result<Frame, String> {
        self.render_with_flags(path, page_index, max_width, max_height, edits, 1 | 0x800)
    }

    fn render_with_flags(
        &mut self,
        path: &Path,
        page_index: u32,
        max_width: u32,
        max_height: u32,
        edits: &[PdfEdit],
        flags: i32,
    ) -> Result<Frame, String> {
        self.ensure(path, edits)?;
        unsafe {
            let document = self.document.as_ref().ok_or("No PDF is open.")?;
            let count = (self.api.count)(document.native.handle);
            if count <= 0 || page_index >= count as u32 {
                return Err("This PDF page does not exist.".into());
            }
            let handle = (self.api.load_page)(document.native.handle, page_index as i32);
            if handle.is_null() {
                return Err("This PDF page could not be read.".into());
            }
            let page = NativeHandle {
                handle,
                close: self.api.close_page,
            };
            let source_width = dimension((self.api.width)(page.handle))?;
            let source_height = dimension((self.api.height)(page.handle))?;
            let permissions = (self.api.permissions)(document.native.handle);
            if flags & 0x800 != 0 && permissions & 4 == 0 {
                return Err("This PDF's permissions do not allow printing.".into());
            }
            let (max_width, max_height) = if flags & 0x800 != 0 && permissions & 0x800 == 0 {
                (
                    max_width.min(source_width.saturating_mul(150) / 72),
                    max_height.min(source_height.saturating_mul(150) / 72),
                )
            } else {
                (max_width, max_height)
            };
            let (width, height) = pdf_size(source_width, source_height, max_width, max_height)?;
            let mut pixels = vec![255; frame_bytes(width, height)?];
            // BGRx on an opaque white surface is also valid premultiplied BGRA.
            let handle = (self.api.bitmap)(
                width as i32,
                height as i32,
                3,
                pixels.as_mut_ptr().cast(),
                (width * 4) as i32,
            );
            if handle.is_null() {
                return Err("There is not enough memory to render this PDF page.".into());
            }
            let bitmap = NativeHandle {
                handle,
                close: self.api.close_bitmap,
            };
            (self.api.render)(
                bitmap.handle,
                page.handle,
                0,
                0,
                width as i32,
                height as i32,
                0,
                flags,
            );
            if (self.api.form_type)(document.native.handle) == 1 {
                let form = self.form_session(document.native.handle, page.handle, page_index)?;
                (self.api.form_draw)(
                    form.handle,
                    bitmap.handle,
                    page.handle,
                    0,
                    0,
                    width as i32,
                    height as i32,
                    0,
                    flags,
                );
            }
            drop(bitmap);
            for pixel in pixels.chunks_exact_mut(4) {
                pixel[3] = 255;
            }
            Ok(Frame {
                width,
                height,
                pixels,
                page_count: count as u32,
                source_width,
                source_height,
            })
        }
    }

    fn annotate(
        &self,
        page: Handle,
        kind: AnnotationKind,
        input: &[[f32; 2]],
        text: &str,
    ) -> Result<(), String> {
        use AnnotationKind::*;
        if input.is_empty()
            || input.len() > 100_000
            || input
                .iter()
                .flatten()
                .any(|n| !n.is_finite() || !(0.0..=1.0).contains(n))
        {
            return Err("Annotation coordinates are invalid.".into());
        }
        if text.len() > 32_000 || text.contains('\0') {
            return Err("Annotation text is too long or contains invalid characters.".into());
        }
        if matches!(kind, Text | Note) && text.trim().is_empty() {
            return Err("Enter annotation text.".into());
        }
        if input.len() < 2 && !matches!(kind, Text | Note) {
            return Err("Drag to draw this annotation.".into());
        }
        unsafe {
            let convert = |p: [f32; 2]| -> Result<Point, String> {
                let (mut x, mut y) = (0.0, 0.0);
                if (self.api.device_to_page)(
                    page,
                    0,
                    0,
                    100_000,
                    100_000,
                    0,
                    (p[0] * 100_000.0).round() as i32,
                    (p[1] * 100_000.0).round() as i32,
                    &mut x,
                    &mut y,
                ) == 0
                    || !x.is_finite()
                    || !y.is_finite()
                {
                    return Err("Cannot position this annotation.".into());
                }
                Ok(Point {
                    x: x as f32,
                    y: y as f32,
                })
            };
            let first = input[0];
            let last = if input.len() > 1 {
                input[input.len() - 1]
            } else {
                [(first[0] + 0.25).min(1.0), (first[1] + 0.08).min(1.0)]
            };
            let l = first[0].min(last[0]);
            let r = first[0].max(last[0]);
            let t = first[1].min(last[1]);
            let b = first[1].max(last[1]);
            let corners = [
                convert([l, t])?,
                convert([r, t])?,
                convert([l, b])?,
                convert([r, b])?,
            ];
            let mut points: Vec<Point> = input
                .iter()
                .map(|p| convert(*p))
                .collect::<Result<_, _>>()?;
            // PDFium cannot create Line annotations. A standard Ink arrow keeps its visible strokes portable.
            if kind == Arrow {
                let a = points[0];
                let z = *points.last().unwrap();
                let dx = z.x - a.x;
                let dy = z.y - a.y;
                let length = dx.hypot(dy);
                if length < 1.0 {
                    return Err("Draw a longer arrow.".into());
                }
                let head = 12.0f32.min(length * 0.3);
                let ux = dx / length;
                let uy = dy / length;
                points = vec![
                    a,
                    z,
                    Point {
                        x: z.x - head * ux + head * 0.5 * uy,
                        y: z.y - head * uy - head * 0.5 * ux,
                    },
                    z,
                    Point {
                        x: z.x - head * ux - head * 0.5 * uy,
                        y: z.y - head * uy + head * 0.5 * ux,
                    },
                ];
            }
            let bounds = if matches!(kind, Ink | Arrow) {
                &points[..]
            } else {
                &corners[..]
            };
            let pad = if matches!(kind, Ink | Arrow) {
                2.0
            } else {
                0.0
            };
            let rect = Rect {
                left: bounds.iter().map(|p| p.x).fold(f32::INFINITY, f32::min) - pad,
                right: bounds.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max) + pad,
                bottom: bounds.iter().map(|p| p.y).fold(f32::INFINITY, f32::min) - pad,
                top: bounds.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max) + pad,
            };
            if rect.right - rect.left < 0.2 || rect.top - rect.bottom < 0.2 {
                return Err("Draw a larger annotation.".into());
            }
            let subtype = match kind {
                Ink | Arrow => 15,
                Highlight => 9,
                Underline => 10,
                Strikeout => 12,
                Note => 1,
                Rectangle => 5,
                Ellipse => 6,
                Text => 3,
            };
            let handle = (self.api.annot_create)(page, subtype);
            if handle.is_null() {
                return Err("This annotation type is unavailable.".into());
            }
            let annotation = NativeHandle {
                handle,
                close: self.api.annot_close,
            };
            let check = |result| {
                if result != 0 {
                    Ok(())
                } else {
                    Err("Could not create the PDF annotation.".to_string())
                }
            };
            check((self.api.annot_rect)(handle, &rect))?;
            check((self.api.annot_flags)(handle, 4))?;
            let (red, green, blue, alpha) = if kind == Highlight {
                (255, 220, 0, 96)
            } else {
                (20, 70, 180, 255)
            };
            check((self.api.annot_color)(handle, 0, red, green, blue, alpha))?;
            if matches!(kind, Ink | Arrow | Rectangle | Ellipse) {
                check((self.api.annot_border)(handle, 0.0, 0.0, 2.0))?;
            }
            if matches!(kind, Ink | Arrow)
                && (self.api.annot_ink)(handle, points.as_ptr(), points.len()) < 0
            {
                return Err("Cannot save the drawn stroke.".into());
            }
            if matches!(kind, Highlight | Underline | Strikeout) {
                let quad = Quad {
                    values: [
                        corners[0].x,
                        corners[0].y,
                        corners[1].x,
                        corners[1].y,
                        corners[2].x,
                        corners[2].y,
                        corners[3].x,
                        corners[3].y,
                    ],
                };
                check((self.api.annot_quad)(handle, &quad))?;
            }
            let content: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            check((self.api.annot_string)(
                handle,
                b"Contents\0".as_ptr(),
                content.as_ptr(),
            ))?;
            if kind == Text {
                let appearance: Vec<u16> = "/Helv 12 Tf 0.08 0.27 0.7 rg"
                    .encode_utf16()
                    .chain(Some(0))
                    .collect();
                check((self.api.annot_string)(
                    handle,
                    b"DA\0".as_ptr(),
                    appearance.as_ptr(),
                ))?;
            }
            // Rendering generates font resources and standard appearance streams before a save.
            let mut scratch = vec![255u8; 64 * 64 * 4];
            let bitmap_handle = (self.api.bitmap)(64, 64, 3, scratch.as_mut_ptr().cast(), 64 * 4);
            if bitmap_handle.is_null() {
                return Err("Cannot generate an annotation appearance.".into());
            }
            let bitmap = NativeHandle {
                handle: bitmap_handle,
                close: self.api.close_bitmap,
            };
            (self.api.render)(bitmap.handle, page, 0, 0, 64, 64, 0, 1);
            if (self.api.annot_ap)(annotation.handle, 0, std::ptr::null_mut(), 0) <= 2 {
                return Err("The annotation has no portable appearance. It was not saved.".into());
            }
            Ok(())
        }
    }

    pub fn page_text(&mut self, path: &Path, page: u32) -> Result<String, String> {
        self.page_text_edited(path, page, &[])
    }

    pub fn page_text_edited(
        &mut self,
        path: &Path,
        page: u32,
        edits: &[PdfEdit],
    ) -> Result<String, String> {
        self.ensure(path, edits)?;
        self.text(
            self.document
                .as_ref()
                .ok_or("No PDF is open.")?
                .native
                .handle,
            page,
        )
    }

    fn text(&self, document: Handle, index: u32) -> Result<String, String> {
        if unsafe { (self.api.permissions)(document) } & 16 == 0 {
            return Err("This PDF's permissions do not allow copying or extracting text.".into());
        }
        let page = self.page(document, index)?;
        unsafe {
            let handle = (self.api.text_load)(page.handle);
            if handle.is_null() {
                return Err("Cannot read text on this PDF page.".into());
            }
            let text = NativeHandle {
                handle,
                close: self.api.text_close,
            };
            let count = (self.api.text_count)(text.handle);
            if !(0..=16_000_000).contains(&count) {
                return Err("The page contains too much text to copy safely.".into());
            }
            let mut buffer = vec![0u16; count as usize + 1];
            let written = (self.api.text_get)(text.handle, 0, count, buffer.as_mut_ptr());
            if written < 0 || written as usize > buffer.len() {
                return Err("Cannot extract PDF text.".into());
            }
            Ok(String::from_utf16_lossy(
                &buffer[..(written as usize).saturating_sub(1)],
            ))
        }
    }

    pub fn find(
        &mut self,
        path: &Path,
        query: &str,
        start_page: u32,
    ) -> Result<Option<u32>, String> {
        self.find_edited(path, query, start_page, &[])
    }

    pub fn find_edited(
        &mut self,
        path: &Path,
        query: &str,
        start_page: u32,
        edits: &[PdfEdit],
    ) -> Result<Option<u32>, String> {
        if query.is_empty() {
            return Err("Enter text to find.".into());
        }
        self.ensure(path, edits)?;
        let handle = self
            .document
            .as_ref()
            .ok_or("No PDF is open.")?
            .native
            .handle;
        let count = unsafe { (self.api.count)(handle) };
        if count <= 0 {
            return Ok(None);
        }
        let needle = query.to_lowercase();
        for offset in 0..count as u32 {
            let index = ((start_page as u64 + offset as u64) % count as u64) as u32;
            if self.text(handle, index)?.to_lowercase().contains(&needle) {
                return Ok(Some(index));
            }
        }
        Ok(None)
    }

    pub fn save_copy(
        &mut self,
        path: &Path,
        output: &Path,
        edits: &[PdfEdit],
    ) -> Result<(), String> {
        let document = self.open(path, edits)?;
        self.reject_protected_export(document.native.handle)?;
        self.write_new(document.native.handle, output)
    }

    pub fn create_from_image(&mut self, frame: &Frame, output: &Path) -> Result<(), String> {
        if frame_bytes(frame.width, frame.height)? != frame.pixels.len() {
            return Err("The image pixel buffer is invalid.".into());
        }
        let document = self.new_document()?;
        unsafe {
            // Use 96 DPI for the physical page while retaining every source pixel.
            let width = frame.width as f64 * 0.75;
            let height = frame.height as f64 * 0.75;
            let page_handle = (self.api.page_new)(document.handle, 0, width, height);
            if page_handle.is_null() {
                return Err("Cannot create an image PDF page.".into());
            }
            let page = NativeHandle {
                handle: page_handle,
                close: self.api.close_page,
            };
            let mut pixels = frame.pixels.clone();
            for p in pixels.chunks_exact_mut(4) {
                let alpha = p[3] as u32;
                if alpha > 0 {
                    for channel in &mut p[..3] {
                        *channel = ((*channel as u32 * 255 + alpha / 2) / alpha).min(255) as u8;
                    }
                }
            }
            let bitmap_handle = (self.api.bitmap)(
                frame.width as i32,
                frame.height as i32,
                4,
                pixels.as_mut_ptr().cast(),
                (frame.width * 4) as i32,
            );
            if bitmap_handle.is_null() {
                return Err("Cannot prepare the image for PDF export.".into());
            }
            let bitmap = NativeHandle {
                handle: bitmap_handle,
                close: self.api.close_bitmap,
            };
            let image_handle = (self.api.image_new)(document.handle);
            if image_handle.is_null() {
                return Err("Cannot create a PDF image object.".into());
            }
            let image = NativeHandle {
                handle: image_handle,
                close: self.api.object_destroy,
            };
            if (self.api.image_bitmap)(std::ptr::null_mut(), 0, image.handle, bitmap.handle) == 0
                || (self.api.image_matrix)(image.handle, width, 0.0, 0.0, height, 0.0, 0.0) == 0
            {
                return Err("Cannot encode the image in PDF.".into());
            }
            let image_handle = image.handle;
            std::mem::forget(image); // Insert takes ownership even on failure in the pinned ABI.
            if (self.api.page_insert)(page.handle, image_handle) == 0
                || (self.api.page_generate)(page.handle) == 0
            {
                return Err("Cannot generate the image PDF page.".into());
            }
            self.write_new(document.handle, output)
        }
    }

    pub fn extract_page(
        &mut self,
        path: &Path,
        output: &Path,
        page: u32,
        edits: &[PdfEdit],
    ) -> Result<(), String> {
        let source = self.open(path, edits)?;
        self.reject_protected_export(source.native.handle)?;
        self.reject_forms(source.native.handle)?;
        let checked = self.page(source.native.handle, page)?;
        drop(checked);
        let destination = self.new_document()?;
        let index = page as i32;
        if unsafe { (self.api.import)(destination.handle, source.native.handle, &index, 1, 0) } == 0
        {
            return Err("Could not extract this PDF page.".into());
        }
        self.write_new(destination.handle, output)
    }

    pub fn merge(
        &mut self,
        path: &Path,
        other: &Path,
        output: &Path,
        edits: &[PdfEdit],
    ) -> Result<(), String> {
        let first = self.open(path, edits)?;
        let second = self.open(other, &[])?;
        self.reject_protected_export(first.native.handle)?;
        self.reject_protected_export(second.native.handle)?;
        self.reject_forms(first.native.handle)?;
        self.reject_forms(second.native.handle)?;
        let destination = self.new_document()?;
        unsafe {
            for source in [first.native.handle, second.native.handle] {
                let count = (self.api.count)(source);
                if count <= 0 {
                    return Err("Cannot merge a PDF without pages.".into());
                }
                let index = (self.api.count)(destination.handle);
                if (self.api.import)(destination.handle, source, std::ptr::null(), 0, index) == 0 {
                    return Err("Could not merge these PDF pages.".into());
                }
            }
        }
        self.write_new(destination.handle, output)
    }

    fn reject_forms(&self, document: Handle) -> Result<(), String> {
        if unsafe { (self.api.form_type)(document) } != 0 {
            return Err(
                "This PDF contains form fields. Page import is disabled to prevent losing them."
                    .into(),
            );
        }
        Ok(())
    }

    fn reject_protected_export(&self, document: Handle) -> Result<(), String> {
        if unsafe { (self.api.security_revision)(document) } >= 0 {
            return Err("Saving or extracting a protected PDF is not available until encryption preservation is verified. The protected original is unchanged.".into());
        }
        Ok(())
    }

    fn new_document(&self) -> Result<NativeHandle, String> {
        let handle = unsafe { (self.api.create)() };
        if handle.is_null() {
            return Err("Cannot create a PDF document.".into());
        }
        Ok(NativeHandle {
            handle,
            close: self.api.close_doc,
        })
    }

    fn write_new(&self, document: Handle, output: &Path) -> Result<(), String> {
        let output = std::path::absolute(output).map_err(|e| e.to_string())?;
        if output.try_exists().map_err(|e| e.to_string())? {
            return Err("Choose a new file name. Existing files are never overwritten.".into());
        }
        let parent = output.parent().ok_or("Choose a valid output folder.")?;
        let mut selected = None;
        for suffix in 0..100 {
            let path = parent.join(format!(".preview-{}-{suffix}.tmp", std::process::id()));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    selected = Some((Temporary(path), file));
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("Cannot create the PDF copy: {e}")),
            }
        }
        let (temporary, file) = selected.ok_or("Cannot create a temporary PDF file.")?;
        let mut writer = FileWrite {
            version: 1,
            write: write_block,
            file,
        };
        if unsafe { (self.api.save)(document, &mut writer, 2) } == 0 {
            return Err("The PDF could not be saved. Your original is unchanged.".into());
        }
        writer
            .file
            .sync_all()
            .map_err(|e| format!("Cannot finish the PDF copy: {e}"))?;
        drop(writer);
        // Reopen before committing so a failed writer never leaves a visible PDF.
        let verification = self.open(&temporary.0, &[])?;
        if unsafe { (self.api.count)(verification.native.handle) != (self.api.count)(document) } {
            return Err("The saved PDF did not pass validation.".into());
        }
        drop(verification);
        let source: Vec<u16> = temporary
            .0
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let target: Vec<u16> = output.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            MoveFileExW(
                PCWSTR(source.as_ptr()),
                PCWSTR(target.as_ptr()),
                MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(|e| format!("Cannot commit the PDF copy. Choose a new filename: {e}"))?;
        Ok(())
    }
}

fn pdf_size(
    width: u32,
    height: u32,
    max_width: u32,
    max_height: u32,
) -> Result<(u32, u32), String> {
    if width == 0 || height == 0 || max_width == 0 || max_height == 0 {
        return Err("Invalid PDF display size.".into());
    }
    let scale = (max_width as f64 / width as f64).min(max_height as f64 / height as f64);
    let size = (
        ((width as f64 * scale).round() as u32).max(1),
        ((height as f64 * scale).round() as u32).max(1),
    );
    frame_bytes(size.0, size.1)?;
    Ok(size)
}

fn dimension(value: f32) -> Result<u32, String> {
    if !value.is_finite() || value <= 0.0 || value > 1_000_000.0 {
        return Err("This PDF has invalid or unsupported page dimensions.".into());
    }
    Ok(value.ceil() as u32)
}

impl Drop for PdfEngine {
    fn drop(&mut self) {
        self.document.take();
        unsafe {
            (self.api.destroy)();
        }
        ENGINE_ACTIVE.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_pdf_dimensions_do_not_allocate() {
        for value in [0.0, -1.0, f32::NAN, f32::INFINITY, 1_000_001.0] {
            assert!(dimension(value).is_err());
        }
        assert_eq!(dimension(612.5).unwrap(), 613);
        assert_eq!(pdf_size(612, 792, 1200, 1600).unwrap(), (1200, 1553));
    }

    #[test]
    #[ignore = "requires fetched PDFium and generated fixtures; run with --ignored --test-threads=1"]
    fn pdf_edits_round_trip_without_overwriting() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let fixture = root.join("fixtures/20-pages.pdf");
        let original = std::fs::read(&fixture).unwrap();
        let output = root.join(format!("target/pdf-test-{}", std::process::id()));
        std::fs::create_dir(&output).unwrap();
        let mut engine = PdfEngine::new().unwrap();
        let edits = [
            PdfEdit::Delete { page: 0 },
            PdfEdit::RotateRight { page: 0 },
        ];
        assert!(engine
            .page_text_edited(&fixture, 0, &edits)
            .unwrap()
            .contains("page 2 of 20"));
        assert_eq!(
            engine
                .find_edited(&fixture, "page 3 of 20", 18, &edits)
                .unwrap(),
            Some(1)
        );
        assert!(engine.save_copy(&fixture, &fixture, &edits).is_err());
        let saved = output.join("edited.pdf");
        engine.save_copy(&fixture, &saved, &edits).unwrap();
        assert!(engine.save_copy(&fixture, &saved, &[]).is_err());
        let frame = engine.render(&saved, 0, 1200, 1600).unwrap();
        assert_eq!(frame.page_count, 19);
        assert!(frame.width > frame.height);
        assert!(frame.pixels.chunks_exact(4).any(|p| p[0] < 200));
        let extracted = output.join("extracted.pdf");
        engine
            .extract_page(&fixture, &extracted, 0, &edits)
            .unwrap();
        assert_eq!(
            engine.render(&extracted, 0, 400, 400).unwrap().page_count,
            1
        );
        assert!(engine
            .page_text(&extracted, 0)
            .unwrap()
            .contains("page 2 of 20"));
        let merged = output.join("merged.pdf");
        engine.merge(&extracted, &fixture, &merged, &[]).unwrap();
        assert_eq!(engine.render(&merged, 0, 400, 400).unwrap().page_count, 21);
        assert!(engine
            .page_text(&merged, 1)
            .unwrap()
            .contains("page 1 of 20"));
        let base = engine.render(&fixture, 0, 612, 792).unwrap();
        let kinds = [
            AnnotationKind::Ink,
            AnnotationKind::Highlight,
            AnnotationKind::Underline,
            AnnotationKind::Strikeout,
            AnnotationKind::Note,
            AnnotationKind::Rectangle,
            AnnotationKind::Ellipse,
            AnnotationKind::Arrow,
            AnnotationKind::Text,
        ];
        let mut annotations = Vec::new();
        for (index, kind) in kinds.into_iter().enumerate() {
            let y = 0.2 + index as f32 * 0.07;
            annotations.push(PdfEdit::Annotate {
                page: 0,
                kind,
                points: vec![[0.1, y], [0.5, y + 0.04]],
                text: "Visible test text".into(),
            });
            let frame = engine
                .render_edited(&fixture, 0, 612, 792, &annotations[index..])
                .unwrap();
            assert_ne!(
                base.pixels, frame.pixels,
                "{kind:?} must draw visible pixels"
            );
        }
        let annotated = output.join("annotated.pdf");
        engine
            .save_copy(&fixture, &annotated, &annotations)
            .unwrap();
        assert_ne!(
            engine.render(&annotated, 0, 612, 792).unwrap().pixels,
            base.pixels
        );
        let serialized = std::fs::read(&annotated).unwrap();
        for subtype in [
            "/Ink",
            "/Highlight",
            "/Underline",
            "/StrikeOut",
            "/Text",
            "/Square",
            "/Circle",
            "/FreeText",
        ] {
            assert!(
                serialized
                    .windows(subtype.len())
                    .any(|part| part == subtype.as_bytes()),
                "{subtype} missing"
            );
        }
        assert!(engine
            .render_edited(
                &fixture,
                0,
                612,
                792,
                &[PdfEdit::Annotate {
                    page: 0,
                    kind: AnnotationKind::Ink,
                    points: vec![[f32::NAN, 0.0], [0.5, 0.5]],
                    text: String::new()
                }]
            )
            .is_err());
        let form_path = output.join("form.pdf");
        let objects=[
            "<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>",
            "<< /Type /Pages /Count 1 /Kids [3 0 R] >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [6 0 R 7 0 R] /Resources << /Font << /Helv 4 0 R >> >> >>",
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
            "<< /Fields [6 0 R 7 0 R] /DA (/Helv 12 Tf 0 g) /DR << /Font << /Helv 4 0 R >> >> >>",
            "<< /Type /Annot /Subtype /Widget /FT /Tx /T (Name) /V () /Rect [60 650 350 685] /F 4 /P 3 0 R /DA (/Helv 12 Tf 0 g) >>",
            "<< /Type /Annot /Subtype /Widget /FT /Btn /T (Agree) /V /Off /AS /Off /Rect [60 600 80 620] /F 4 /P 3 0 R /AP << /N << /Yes 8 0 R /Off 9 0 R >> >> >>",
            "<< /Type /XObject /Subtype /Form /BBox [0 0 20 20] /Length 27 >>\nstream\n0 0 m 20 20 l 2 w 0 0 0 RG S\nendstream",
            "<< /Type /XObject /Subtype /Form /BBox [0 0 20 20] /Length 0 >>\nstream\n\nendstream",
        ];
        let mut data = String::from("%PDF-1.4\n");
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(data.len());
            data.push_str(&format!("{} 0 obj\n{}\nendobj\n", index + 1, object));
        }
        let xref = data.len();
        data.push_str(&format!(
            "xref\n0 {}\n0000000000 65535 f \n",
            objects.len() + 1
        ));
        for offset in offsets {
            data.push_str(&format!("{offset:010} 00000 n \n"));
        }
        data.push_str(&format!(
            "trailer\n<< /Root 1 0 R /Size {} >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        ));
        std::fs::write(&form_path, data).unwrap();
        let fields = engine.form_fields(&form_path, 0, &[]).unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].kind, FormFieldKind::Text);
        assert_eq!(fields[1].kind, FormFieldKind::Checkbox);
        assert_eq!(fields[1].value, "false");
        let empty = engine.render(&form_path, 0, 612, 792).unwrap();
        let fills = [
            PdfEdit::FillField {
                page: 0,
                annotation_index: 0,
                value: "Ada Lovelace".into(),
            },
            PdfEdit::FillField {
                page: 0,
                annotation_index: 1,
                value: "true".into(),
            },
        ];
        let filled_path = output.join("filled.pdf");
        engine.save_copy(&form_path, &filled_path, &fills).unwrap();
        let fields = engine.form_fields(&filled_path, 0, &[]).unwrap();
        assert_eq!(fields[0].value, "Ada Lovelace");
        assert_eq!(fields[1].value, "true");
        assert_ne!(
            engine.render(&filled_path, 0, 612, 792).unwrap().pixels,
            empty.pixels
        );
        assert!(engine
            .merge(&form_path, &fixture, &output.join("rejected.pdf"), &[])
            .is_err());
        assert!(!output.join("rejected.pdf").exists());
        let image_pdf = output.join("image.pdf");
        let image = Frame {
            width: 2,
            height: 2,
            pixels: vec![0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255, 0, 0, 0, 0],
            page_count: 1,
            source_width: 2,
            source_height: 2,
        };
        engine.create_from_image(&image, &image_pdf).unwrap();
        let frame = engine.render(&image_pdf, 0, 200, 200).unwrap();
        assert_eq!(frame.page_count, 1);
        let pixel = |x: usize, y: usize| &frame.pixels[(y * frame.width as usize + x) * 4..][..4];
        assert!(pixel(30, 30)[2] > 200 && pixel(30, 30)[0] < 30);
        assert!(pixel(170, 170)[0] > 230 && pixel(170, 170)[1] > 230);
        let encrypted = root.join("fixtures/password.pdf");
        let restricted = root.join("fixtures/password-restricted.pdf");
        assert_eq!(
            engine.render(&encrypted, 0, 400, 400).err().unwrap(),
            PASSWORD_REQUIRED
        );
        assert_eq!(
            engine
                .set_password(&encrypted, "incorrect".into())
                .err()
                .unwrap(),
            PASSWORD_REQUIRED
        );
        engine
            .set_password(&encrypted, "preview-test".into())
            .unwrap();
        assert_eq!(
            engine.render(&encrypted, 0, 400, 400).unwrap().page_count,
            1
        );
        assert!(engine
            .page_text(&encrypted, 0)
            .unwrap()
            .contains("Synthetic page"));
        assert!(engine
            .save_copy(&encrypted, &output.join("decrypted.pdf"), &[])
            .is_err());
        assert!(!output.join("decrypted.pdf").exists());
        engine
            .set_password(&restricted, "preview-test".into())
            .unwrap();
        assert!(engine.render(&restricted, 0, 400, 400).is_ok());
        assert!(engine.page_text(&restricted, 0).is_err());
        assert!(engine
            .render_for_print(&restricted, 0, 400, 400, &[])
            .is_err());
        let replaced = output.join("replaced.pdf");
        std::fs::copy(&fixture, &replaced).unwrap();
        engine.render(&replaced, 0, 400, 400).unwrap();
        let replacement = output.join("replacement.pdf");
        std::fs::copy(&extracted, &replacement).unwrap();
        std::fs::rename(&replacement, &replaced).unwrap();
        assert!(engine
            .render_edited(&replaced, 0, 400, 400, &[PdfEdit::RotateRight { page: 0 }])
            .is_err());
        assert!(engine
            .save_copy(
                &replaced,
                &output.join("stale.pdf"),
                &[PdfEdit::RotateRight { page: 0 }]
            )
            .is_err());
        assert_eq!(engine.render(&replaced, 0, 400, 400).unwrap().page_count, 1);
        let organized = output.join("organized.pdf");
        let organize = [
            PdfEdit::Move { from: 0, to: 19 },
            PdfEdit::InsertBlank { at: 1 },
            PdfEdit::RotateRight { page: 0 },
            PdfEdit::Crop {
                page: 0,
                left: 0.0,
                top: 0.0,
                right: 0.5,
                bottom: 1.0,
            },
        ];
        engine.save_copy(&fixture, &organized, &organize).unwrap();
        let frame = engine.render(&organized, 0, 1200, 1600).unwrap();
        assert_eq!(frame.page_count, 21);
        assert_eq!((frame.source_width, frame.source_height), (396, 612));
        assert!(engine.page_text(&organized, 1).unwrap().trim().is_empty());
        assert!(engine
            .page_text(&organized, 20)
            .unwrap()
            .contains("page 1 of 20"));
        assert!(engine
            .save_copy(
                &fixture,
                &output.join("invalidmove.pdf"),
                &[PdfEdit::Move { from: 20, to: 0 }]
            )
            .is_err());
        assert!(engine
            .save_copy(
                &fixture,
                &output.join("invalidcrop.pdf"),
                &[PdfEdit::Crop {
                    page: 0,
                    left: 0.8,
                    top: 0.0,
                    right: 0.2,
                    bottom: 1.0
                }]
            )
            .is_err());
        assert_eq!(std::fs::read(&fixture).unwrap(), original);
        drop(engine);
        for name in [
            "edited.pdf",
            "extracted.pdf",
            "merged.pdf",
            "annotated.pdf",
            "form.pdf",
            "filled.pdf",
            "image.pdf",
            "replaced.pdf",
            "organized.pdf",
        ] {
            std::fs::remove_file(output.join(name)).unwrap();
        }
        std::fs::remove_dir(output).unwrap();
    }
}
