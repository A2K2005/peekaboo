// Form fields: listing, recipe replay (FillField), and the inline form session.
// APIs: fpdf_formfill.h (FORM_*, FPDFDOC_InitFormFillEnvironment) and
// fpdf_annot.h (FPDFAnnot_GetFormField*, FPDFAnnot_GetOption*).
use super::*;
use crate::model::{FormFeedback, FormInput};

// FPDF_FORMFIELD_* values from fpdf_formfill.h.
const CHECKBOX: i32 = 2;
const RADIO: i32 = 3;
const COMBO: i32 = 4;
const LIST: i32 = 5;
const TEXT: i32 = 6;
// FPDF_FORMFLAG_CHOICE_EDIT from fpdf_annot.h.
const CHOICE_EDIT: i32 = 1 << 18;
const WIDGET: i32 = 20;
const NO_ACROFORM: &str =
    "This document does not contain supported AcroForm fields. XFA forms require another reader.";

impl PdfEngine {
    pub fn form_fields(
        &mut self,
        path: &Path,
        page_index: u32,
        edits: &[PdfEdit],
    ) -> Result<Vec<FormField>, String> {
        self.ensure(path, edits)?;
        let document = self.document.as_mut().ok_or("No PDF is open.")?;
        if unsafe { (self.api.form_type)(document.native.handle) } != 1 {
            return Err(NO_ACROFORM.into());
        }
        let form = self.api.form(document)?;
        let page = self.api.open_page(document, page_index)?;
        let mut fields = Vec::new();
        unsafe {
            let count = (self.api.annot_count)(page.handle);
            for index in 0..count.max(0) as u32 {
                let annotation = self
                    .api
                    .annotation(page.handle, index)
                    .map_err(|_| "Cannot read a page annotation.")?;
                let handle = annotation.handle;
                let kind = match (self.api.field_type)(form, handle) {
                    -1 | 0 => continue,
                    TEXT => FormFieldKind::Text,
                    CHECKBOX => FormFieldKind::Checkbox,
                    _ => FormFieldKind::Unsupported,
                };
                let flags = (self.api.field_flags)(form, handle);
                let value = if kind == FormFieldKind::Checkbox {
                    ((self.api.field_checked)(form, handle) != 0).to_string()
                } else {
                    self.api.field_string(form, handle, self.api.field_value)?
                };
                let name = self.api.field_string(form, handle, self.api.field_name)?;
                fields.push(FormField {
                    page: page_index,
                    annotation_index: index,
                    name,
                    value,
                    kind,
                    read_only: flags < 0 || flags & 1 != 0,
                });
            }
        }
        Ok(fields)
    }

    /// Replays a FillField recipe. Values: text as typed; checkboxes and radio
    /// buttons "true" or "false"; combo and list boxes the selected option
    /// labels joined with "\n", or the typed text of an editable combo box.
    pub(super) fn fill_field(
        &self,
        document: &mut Document,
        page_index: u32,
        index: u32,
        value: &str,
    ) -> Result<(), String> {
        if value.len() > 32_000 || value.contains('\0') {
            return Err("Form text is too long or contains invalid characters.".into());
        }
        if unsafe { (self.api.form_type)(document.native.handle) } != 1 {
            return Err(NO_ACROFORM.into());
        }
        let form = self.api.form(document)?;
        let page = self.api.open_page(document, page_index)?;
        let rotation = unsafe { (self.api.rotation)(page.handle) };
        if let Some(state) = document.form.as_mut() {
            state.target(page.handle, page_index, rotation);
        }
        let result = self.api.set_field(form, page.handle, index, value);
        drop(page);
        self.api.retarget(document);
        result
    }

    /// Sends one input event to the inline form session on `page`. Returns
    /// the focused field, whether to redraw, and FillField recipes for values
    /// the user committed. Append those commits to `edits` before the next
    /// call, or the session restarts from the recipe. Call with
    /// `FormInput::Blur` before saving, so a field being typed in commits.
    #[allow(dead_code)]
    pub fn form_event(
        &mut self,
        path: &Path,
        page: u32,
        input: FormInput,
        edits: &[PdfEdit],
    ) -> Result<FormFeedback, String> {
        self.ensure(path, edits)?;
        let api = &self.api;
        let document = self.document.as_mut().ok_or("No PDF is open.")?;
        match unsafe { (api.form_type)(document.native.handle) } {
            0 => return Err("This PDF has no form fields.".into()),
            1 => {}
            _ => return Err(XFA_FORM.into()),
        }
        let form = api.form(document)?;
        let count = unsafe { (api.count)(document.native.handle) }.max(0) as u32;
        if page >= count {
            return Err("This PDF page does not exist.".into());
        }
        let mut commits = Vec::new();
        api.session_page(document, page, &mut commits)?;
        if let Some(state) = document.form.as_mut() {
            state.info.invalidated = false;
        }
        let before = api.focused(document);
        let fill = document.fill.as_ref().ok_or("The form page is not open.")?;
        let handle = fill.page.handle;
        unsafe {
            match input {
                FormInput::PointerDown { x, y }
                | FormInput::PointerUp { x, y }
                | FormInput::PointerMove { x, y } => {
                    let (ux, uy) = api
                        .display(handle)?
                        .user(x as f64, y as f64)
                        .ok_or("Cannot map this point on the page.")?;
                    let event = match input {
                        FormInput::PointerDown { .. } => api.form_down,
                        FormInput::PointerUp { .. } => api.form_up,
                        _ => api.form_move,
                    };
                    event(form, handle, 0, ux, uy);
                }
                FormInput::Char(c) => {
                    // Tab moves focus through `Key`; as a character it would be typed.
                    if c != '\t' {
                        let mut units = [0u16; 2];
                        for unit in c.encode_utf16(&mut units) {
                            (api.form_char)(form, handle, *unit as i32, 0);
                        }
                    }
                }
                FormInput::Key {
                    code,
                    shift,
                    ctrl,
                    alt,
                } => {
                    // FWL_EVENTFLAG_ShiftKey, _ControlKey, _AltKey (fpdf_fwlevent.h).
                    let modifier = shift as i32 | (ctrl as i32) << 1 | (alt as i32) << 2;
                    (api.form_key)(form, handle, code as i32, modifier);
                    // PDFium tabs only within a page and stops at the last field.
                    if code == 0x09 && api.focused(document) == before {
                        api.tab_across_pages(document, page, count, shift, &mut commits)?;
                    }
                }
                FormInput::Blur => {
                    (api.form_blur)(form);
                }
            }
        }
        api.track(document, &mut commits);
        document.edits.extend(commits.iter().cloned());
        let after = api.focused(document);
        let focus = match after {
            Some((page, index)) => api.field_rect(document, index).map(|rect| (page, rect)),
            None => None,
        };
        let invalidated = document.form.as_ref().is_some_and(|f| f.info.invalidated);
        Ok(FormFeedback {
            focus,
            redraw: invalidated || !commits.is_empty() || after != before,
            commits,
        })
    }
}

impl Api {
    fn field_string(
        &self,
        form: Handle,
        annotation: Handle,
        read: FieldRead,
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

    fn options(&self, form: Handle, annotation: Handle) -> Vec<String> {
        let count = unsafe { (self.option_count)(form, annotation) };
        (0..count.max(0))
            .map(|i| {
                read_utf16(|buffer, length| unsafe {
                    (self.option_label)(form, annotation, i, buffer, length)
                })
            })
            .collect()
    }

    /// A field's value in FillField form (see `fill_field`).
    fn field_value(&self, form: Handle, annotation: Handle) -> Result<String, String> {
        unsafe {
            match (self.field_type)(form, annotation) {
                CHECKBOX | RADIO => Ok(((self.field_checked)(form, annotation) != 0).to_string()),
                COMBO | LIST => {
                    let selected: Vec<String> = self
                        .options(form, annotation)
                        .into_iter()
                        .enumerate()
                        .filter(|(i, _)| (self.option_selected)(form, annotation, *i as i32) != 0)
                        .map(|(_, label)| label)
                        .collect();
                    if selected.is_empty() {
                        self.field_string(form, annotation, self.field_value)
                    } else {
                        Ok(selected.join("\n"))
                    }
                }
                _ => self.field_string(form, annotation, self.field_value),
            }
        }
    }

    fn set_field(&self, form: Handle, page: Handle, index: u32, value: &str) -> Result<(), String> {
        let annotation = self
            .annotation(page, index)
            .map_err(|_| "The form field no longer exists.")?;
        let handle = annotation.handle;
        unsafe {
            let flags = (self.field_flags)(form, handle);
            if flags < 0 || flags & 1 != 0 {
                return Err("This form field is read-only.".into());
            }
            let kind = (self.field_type)(form, handle);
            if !(CHECKBOX..=TEXT).contains(&kind) {
                return Err(
                    "This field type is not yet editable. Its original value is preserved.".into(),
                );
            }
            if (self.form_focus)(form, handle) == 0 {
                return Err("Cannot focus this form field.".into());
            }
            let type_text = || -> Result<(), String> {
                if (self.form_select)(form, page) == 0 {
                    return Err("Cannot select the field text.".into());
                }
                let text = wide(value);
                (self.form_replace)(form, page, text.as_ptr());
                Ok(())
            };
            match kind {
                TEXT => type_text()?,
                CHECKBOX | RADIO => {
                    let desired = match value {
                        "true" => true,
                        "false" => false,
                        _ => return Err("Checkbox values must be true or false.".into()),
                    };
                    if desired != ((self.field_checked)(form, handle) != 0)
                        && (self.form_char)(form, page, 32, 0) == 0
                    {
                        return Err("Cannot change this checkbox.".into());
                    }
                }
                _ => {
                    let options = self.options(form, handle);
                    let wanted: Option<Vec<usize>> = value
                        .split('\n')
                        .filter(|label| !value.is_empty() || !label.is_empty())
                        .map(|label| options.iter().position(|o| o == label))
                        .collect();
                    match wanted {
                        Some(indices) if kind == LIST => {
                            for i in 0..options.len() {
                                (self.form_select_index)(
                                    form,
                                    page,
                                    i as i32,
                                    indices.contains(&i) as i32,
                                );
                            }
                        }
                        Some(indices) if indices.len() == 1 => {
                            if (self.form_select_index)(form, page, indices[0] as i32, 1) == 0 {
                                return Err("Cannot select this option.".into());
                            }
                        }
                        _ if kind == COMBO && flags & CHOICE_EDIT != 0 => type_text()?,
                        _ => return Err("Choose one of this field's options.".into()),
                    }
                }
            }
            (self.form_blur)(form);
        }
        if self.field_value(form, handle)? != value {
            return Err(
                "The form rejected or shortened this value. Check its input limits.".into(),
            );
        }
        Ok(())
    }

    /// Points the form callbacks back at the session page, or at nothing.
    fn retarget(&self, document: &mut Document) {
        let page = document.fill.as_ref().map(|f| {
            (f.page.handle, f.index, unsafe {
                (self.rotation)(f.page.handle)
            })
        });
        if let Some(form) = document.form.as_mut() {
            match page {
                Some((handle, index, rotation)) => form.target(handle, index, rotation),
                None => form.target(std::ptr::null_mut(), 0, 0),
            }
        }
    }

    /// Moves the session to `page`. Leaving a page kills focus, which commits
    /// the field being edited.
    fn session_page(
        &self,
        document: &mut Document,
        page: u32,
        commits: &mut Vec<PdfEdit>,
    ) -> Result<(), String> {
        if document.fill.as_ref().is_some_and(|f| f.index == page) {
            return Ok(());
        }
        let form = self.form(document)?;
        if document.fill.is_some() {
            unsafe { (self.form_blur)(form) };
            self.track(document, commits);
            if let Some(fill) = document.fill.take() {
                unsafe { (self.form_before)(fill.page.handle, form) };
            }
            document.focus = None;
        }
        let loaded = self.page(document.native.handle, page)?;
        unsafe { (self.form_after)(loaded.handle, form) };
        document.fill = Some(FillPage {
            index: page,
            page: loaded,
        });
        self.retarget(document);
        Ok(())
    }

    /// The focused field as (page, annotation index) on the session page.
    fn focused(&self, document: &Document) -> Option<(u32, u32)> {
        let form = document.form.as_ref()?.handle;
        let fill = document.fill.as_ref()?;
        let mut page = -1;
        let mut annotation = std::ptr::null_mut();
        unsafe {
            if (self.form_focused)(form, &mut page, &mut annotation) == 0 || annotation.is_null() {
                return None;
            }
            let annotation = NativeHandle {
                handle: annotation,
                close: self.annot_close,
            };
            let index = (self.annot_index)(fill.page.handle, annotation.handle);
            (page == fill.index as i32 && index >= 0).then_some((fill.index, index as u32))
        }
    }

    /// Compares the tracked field with its current value and records a
    /// commit when it changed. Then starts tracking the focused field.
    fn track(&self, document: &mut Document, commits: &mut Vec<PdfEdit>) {
        let now = self.focused(document);
        let current = |index: u32| -> Option<String> {
            let form = document.form.as_ref()?.handle;
            let fill = document.fill.as_ref()?;
            let annotation = self.annotation(fill.page.handle, index).ok()?;
            self.field_value(form, annotation.handle).ok()
        };
        let mut next = None;
        if let Some(previous) = document.focus.as_ref() {
            if let Some(value) = current(previous.index) {
                if value != previous.value {
                    commits.push(PdfEdit::FillField {
                        page: previous.page,
                        annotation_index: previous.index,
                        value: value.clone(),
                    });
                }
                if now == Some((previous.page, previous.index)) {
                    next = Some(FocusedField {
                        value,
                        ..previous.clone()
                    });
                }
            }
        }
        if next.is_none() {
            next = now.and_then(|(page, index)| {
                current(index).map(|value| FocusedField { page, index, value })
            });
        }
        document.focus = next;
    }

    /// Tab past the last field (or Shift+Tab before the first) moves to the
    /// next page that has an editable field, wrapping at the document end.
    fn tab_across_pages(
        &self,
        document: &mut Document,
        page: u32,
        count: u32,
        back: bool,
        commits: &mut Vec<PdfEdit>,
    ) -> Result<(), String> {
        let had_focus = self.focused(document).is_some();
        let form = self.form(document)?;
        for step in (had_focus as u32)..=count {
            let target = if back {
                (page + count * 2 - step) % count
            } else {
                (page + step) % count
            };
            let Some(index) = self.edge_field(document, form, target, back)? else {
                continue;
            };
            self.session_page(document, target, commits)?;
            let fill = document.fill.as_ref().ok_or("The form page is not open.")?;
            let annotation = self.annotation(fill.page.handle, index)?;
            unsafe { (self.form_focus)(form, annotation.handle) };
            return Ok(());
        }
        Ok(())
    }

    /// The first (or last) editable field on a page, in annotation order.
    fn edge_field(
        &self,
        document: &Document,
        form: Handle,
        page: u32,
        last: bool,
    ) -> Result<Option<u32>, String> {
        let loaded = self.page(document.native.handle, page)?;
        let count = unsafe { (self.annot_count)(loaded.handle) }.max(0) as u32;
        let editable = |index: u32| -> bool {
            self.annotation(loaded.handle, index).is_ok_and(|a| unsafe {
                (self.annot_subtype)(a.handle) == WIDGET
                    && (CHECKBOX..=TEXT).contains(&(self.field_type)(form, a.handle))
                    && (self.field_flags)(form, a.handle) & 1 == 0
            })
        };
        Ok(if last {
            (0..count).rev().find(|&i| editable(i))
        } else {
            (0..count).find(|&i| editable(i))
        })
    }

    fn field_rect(&self, document: &Document, index: u32) -> Option<NormRect> {
        let fill = document.fill.as_ref()?;
        let annotation = self.annotation(fill.page.handle, index).ok()?;
        let mut rect = Rect::default();
        if unsafe { (self.annot_get_rect)(annotation.handle, &mut rect) } == 0 {
            return None;
        }
        let display = self.display(fill.page.handle).ok()?;
        Some(display.norm(display.rect(
            rect.left as f64,
            rect.bottom as f64,
            rect.right as f64,
            rect.top as f64,
        )))
    }
}
