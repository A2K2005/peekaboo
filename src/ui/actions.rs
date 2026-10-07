//! Runs commands. Ported from the pre-split shell's command handler, with
//! sheets in place of dialog windows and message boxes.
use super::{
    app::{add_tabs, close_tab, invalidate, navigate, open, schedule, select_tab, with_state},
    commands::{self, Command, MenuItem, Pick},
    document::{MAX_ZOOM, MIN_ZOOM},
    files::{choose, choose_many, destination, folder, load_signature, save_signature},
    menu, sheet,
    widgets::{self, WidgetId},
    worker::{Job, Request, SaveKind},
};
use crate::model::{AnnotationKind, ImageEdit, PdfEdit};
use std::{
    sync::{atomic::AtomicBool, Arc},
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::{HWND, LPARAM, POINT, WPARAM},
    Graphics::Gdi::ClientToScreen,
    UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE},
};

/// Shows a popup menu under the widget for `anchor`, or at `at` (client
/// pixels), and returns the pick.
pub(super) unsafe fn popup(hwnd: HWND, items: Vec<MenuItem>, anchor: Option<WidgetId>, at: Option<(f32, f32)>, keyboard: bool) -> Option<Pick> {
    let (point, theme, scale, text_scale) = with_state(|s| {
        let layout = s.layout();
        let rect = anchor.and_then(|id| layout.widgets.iter().find(|w| w.id == id)).map(|w| w.rect);
        let point = match (rect, at) {
            (Some(r), _) => (r.x0, r.y1 + 2.0 * s.scale),
            (None, Some(p)) => p,
            (None, None) => (layout.document.x0 + 16.0 * s.scale, layout.document.y0 + 8.0 * s.scale),
        };
        (point, s.theme, s.scale, s.text_scale)
    })?;
    let mut screen = POINT { x: point.0 as i32, y: point.1 as i32 };
    let _ = ClientToScreen(hwnd, &mut screen);
    let pick = menu::track(hwnd, items, (screen.x as f32, screen.y as f32), keyboard, theme, scale, text_scale);
    with_state(|s| {
        s.pressed = None;
        s.hover = None;
    });
    invalidate(hwnd);
    pick
}

/// Re-renders after the document area changes size, as WM_SIZE does.
fn relayout(state: &mut super::app::State) {
    if state.frame.is_some() {
        state.due = Some(Instant::now() + Duration::from_millis(120));
    }
}

pub(super) unsafe fn execute(hwnd: HWND, command: Command, keyboard: bool) {
    use Command::*;
    let Some(ctx) = with_state(|s| s.ctx()) else {
        return;
    };
    if !commands::enabled(command, &ctx) {
        return;
    }
    match command {
        Open => {
            if let Some(paths) = choose_many(hwnd, false) {
                with_state(|s| add_tabs(s, &paths));
                if let Some(path) = paths.into_iter().next() {
                    open(hwnd, path);
                }
            }
            return;
        }
        Exit => {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            return;
        }
        CloseTab => {
            if let Some(index) = with_state(|s| s.active_tab()).flatten() {
                close_tab(hwnd, index);
            }
            return;
        }
        NextTab | PreviousTab | Tab(_) => {
            let target = with_state(|s| {
                let count = s.tabs.len();
                let current = s.active_tab().unwrap_or(0);
                match command {
                    NextTab => (current + 1) % count,
                    PreviousTab => (current + count - 1) % count,
                    Tab(8) => count - 1,
                    Tab(n) => n as usize,
                    _ => current,
                }
            });
            if let Some(index) = target {
                select_tab(hwnd, index);
            }
            return;
        }
        Previous | Next => {
            navigate(hwnd, if command == Previous { -1 } else { 1 });
            return;
        }
        ToggleSidebar | ToggleMarkup => {
            with_state(|s| {
                if command == ToggleSidebar {
                    s.sidebar_open = !s.sidebar_open;
                    relayout(s);
                } else {
                    // The document renders again when the slide ends (tick).
                    s.set_markup(!s.markup_open);
                }
            });
            invalidate(hwnd);
            return;
        }
        NextPane | PreviousPane => {
            with_state(|s| {
                let layout = s.layout();
                s.focus = widgets::next_region(&layout.widgets, s.focus, command == PreviousPane).or(s.focus);
                s.focus_visible = true;
            });
            invalidate(hwnd);
            return;
        }
        ZoomMenu | AppMenu | MoreTools => {
            let overflow = with_state(|s| {
                let layout = s.layout();
                (layout.toolbar_overflow, layout.markup_overflow)
            })
            .unwrap_or_default();
            let items = match command {
                ZoomMenu => commands::zoom_menu(&ctx),
                AppMenu => commands::app_menu(&ctx, &overflow.0),
                _ => commands::markup_overflow_menu(&overflow.1, &ctx),
            };
            if let Some(Pick::Command(chosen)) = popup(hwnd, items, Some(WidgetId::Command(command)), None, keyboard) {
                execute(hwnd, chosen, keyboard);
            }
            return;
        }
        _ => {}
    }
    document_command(hwnd, command);
}

/// Commands that act on the open document.
unsafe fn document_command(hwnd: HWND, command: Command) {
    use Command::*;
    let snapshot = with_state(|s| {
        if s.pending || (s.render_failed && !matches!(command, Undo | Revert)) {
            return None;
        }
        let frame = s.frame.as_ref()?;
        Some((
            Request {
                generation: s.generation,
                path: s.path.clone()?,
                page: s.page,
                delta: 0,
                width: 1,
                height: 1,
                sessions: s.sessions.clone(),
            },
            frame.source_width,
            frame.source_height,
            frame.page_count,
        ))
    })
    .flatten();
    let Some((request, width, height, count)) = snapshot else {
        return;
    };
    let pdf = super::worker::is_pdf(&request.path);
    match command {
        FileInfo => {
            let bytes = std::fs::metadata(&request.path).map(|m| m.len()).unwrap_or(0);
            let text = format!(
                "{}\n\nSize: {width} × {height}\nPages: {count}\nFile size: {bytes} bytes\n\nEdits stay in memory until you save a copy.",
                request.path.display()
            );
            sheet::alert(hwnd, "File information", &text);
        }
        Slideshow => {
            with_state(|s| {
                s.slideshow = if s.slideshow.is_some() { None } else { Some(Instant::now() + Duration::from_secs(3)) };
                s.status = "Slideshow advances every 3 seconds. Escape stops.".into();
            });
            invalidate(hwnd);
        }
        Print => {
            match crate::printing::choose(hwnd, count) {
                Ok(Some(job)) => {
                    let cancel = job.cancellation();
                    with_state(|s| {
                        if s.exporting {
                            return;
                        }
                        s.exporting = true;
                        s.cancel = Some(cancel);
                        s.status = "Printing... Escape cancels.".into();
                        if !s.send(Job::Print(request, job)) {
                            s.exporting = false;
                            s.status = "The print worker stopped.".into();
                        }
                    });
                }
                Ok(None) => {}
                Err(error) => sheet::alert(hwnd, "Print", &error),
            }
            invalidate(hwnd);
        }
        BatchFolder | BatchSelected => batch(hwnd, command, request),
        RemoveBackground => {
            let Some(mut output) = destination(hwnd, false) else {
                return;
            };
            output.set_extension("png");
            start_export(hwnd, Job::Background(request, output), "Removing background on this PC...");
        }
        SaveSignature => save_signature(hwnd),
        PlaceSignature | Draw | Highlight | Underline | Strikethrough | Note | TextBox | Rectangle | Ellipse | Arrow => {
            choose_tool(hwnd, command)
        }
        SaveCopy | ExtractPage | Combine => {
            let kind = match command {
                ExtractPage => SaveKind::ExtractPage,
                Combine => match choose(hwnd) {
                    Some(other) => SaveKind::Merge(other),
                    None => return,
                },
                _ => SaveKind::Copy,
            };
            let Some(output) = destination(hwnd, pdf) else {
                return;
            };
            start_export(hwnd, Job::Save(request, output, kind), "Saving a new copy...");
        }
        CopyText | Find | FillForm => {
            let job = match command {
                FillForm => Job::Fields(request),
                CopyText => Job::Text(request),
                _ => {
                    let Some(values) = sheet::input(hwnd, "Find", &[("Text to find", String::new())]) else {
                        return;
                    };
                    let Some(query) = values.into_iter().next().filter(|v| !v.trim().is_empty()) else {
                        return;
                    };
                    Job::Find(request, query)
                }
            };
            with_state(|s| {
                if !s.send(job) {
                    s.status = "The processing worker stopped.".into();
                } else {
                    s.status = "Reading text on this PC...".into();
                }
            });
            invalidate(hwnd);
        }
        MovePage => {
            let Some(values) = sheet::input(hwnd, "Move page", &[("New page number", (request.page + 1).to_string())]) else {
                return;
            };
            let to = values.first().and_then(|v| v.trim().parse::<u32>().ok()).filter(|v| *v > 0 && *v <= count);
            match to {
                Some(to) => edit_same_page(hwnd, &request, |s, edits| {
                    edits.pdf.push(PdfEdit::Move { from: request.page, to: to - 1 });
                    s.page = to - 1;
                }),
                None => sheet::alert(hwnd, "Move page", &format!("Enter a page number from 1 to {count}.")),
            }
        }
        Resize => {
            let Some(values) = sheet::input(
                hwnd,
                "Resize image",
                &[("Width in pixels", width.to_string()), ("Height in pixels (leave empty to keep the shape)", String::new())],
            ) else {
                return;
            };
            match resize_edit(width, height, &values) {
                Some(resize) => edit(hwnd, |_, edits| edits.image.push(resize)),
                None => sheet::alert(hwnd, "Resize image", "Enter a width and height above 0, with at most 200 million pixels in total."),
            }
        }
        DeletePage => {
            if count <= 1
                || !sheet::confirm(
                    hwnd,
                    "Delete this page?",
                    "The page is removed from your working copy. The original file stays unchanged until you save a copy.",
                    "Delete page",
                )
            {
                return;
            }
            edit_same_page(hwnd, &request, |s, edits| {
                edits.pdf.push(PdfEdit::Delete { page: request.page });
                s.page = request.page.min(count.saturating_sub(2));
            });
        }
        Fit | ZoomIn | ZoomOut => {
            with_state(|s| {
                s.zoom = match command {
                    Fit => 1.0,
                    ZoomIn => (s.zoom * 1.4).min(MAX_ZOOM),
                    _ => (s.zoom / 1.4).max(MIN_ZOOM),
                };
                s.pan = (0.0, 0.0);
                schedule(hwnd, s, 0);
            });
        }
        Crop => {
            with_state(|s| {
                s.markup = None;
                s.signature = None;
                s.crop = !s.crop;
                s.status = if !s.crop {
                    "Crop is off.".into()
                } else if pdf {
                    "Drag a rectangle to crop the page view. Hidden content stays in the file. Escape cancels.".into()
                } else {
                    "Drag a rectangle over the image to crop. Escape cancels.".into()
                };
            });
            invalidate(hwnd);
        }
        Rotate => edit(hwnd, |s, edits| {
            if pdf {
                edits.pdf.push(PdfEdit::RotateRight { page: s.page });
            } else {
                edits.image.push(ImageEdit::RotateRight);
            }
        }),
        Flip => edit(hwnd, |_, edits| edits.image.push(ImageEdit::FlipHorizontal)),
        InsertPage => edit(hwnd, |s, edits| {
            let at = s.page + 1;
            edits.pdf.push(PdfEdit::InsertBlank { at });
            s.page = at;
        }),
        Undo => edit(hwnd, |_, edits| {
            if pdf {
                edits.pdf.pop();
            } else {
                edits.image.pop();
            }
        }),
        Revert => edit(hwnd, |s, edits| {
            edits.pdf.clear();
            edits.image.clear();
            s.page = 0;
        }),
        _ => {}
    }
}

/// True while the file, page, and render generation still match a request
/// captured before a sheet opened. A Find result or a slideshow step can
/// move to another page while the sheet waits.
pub(super) fn same_target(s: &super::app::State, request: &Request) -> bool {
    s.generation == request.generation && s.page == request.page && s.path.as_ref() == Some(&request.path)
}

/// Applies a page edit chosen in a sheet, only if the page did not change
/// while the sheet was open.
unsafe fn edit_same_page(hwnd: HWND, request: &Request, change: impl FnOnce(&mut super::app::State, &mut super::worker::Edits)) {
    if with_state(|s| same_target(s, request)) != Some(true) {
        with_state(|s| s.status = "The page changed while the dialog was open. Nothing was edited.".into());
        invalidate(hwnd);
        return;
    }
    edit(hwnd, change);
}

/// Applies one recipe change to the open file and renders again.
unsafe fn edit(hwnd: HWND, change: impl FnOnce(&mut super::app::State, &mut super::worker::Edits)) {
    with_state(|s| {
        let Some(path) = s.path.clone() else {
            return;
        };
        let mut edits = s.sessions.remove(&path).unwrap_or_default();
        change(s, &mut edits);
        edits.dirty = !edits.image.is_empty() || !edits.pdf.is_empty();
        s.sessions.insert(path, edits);
        s.zoom = 1.0;
        s.pan = (0.0, 0.0);
        schedule(hwnd, s, 0);
    });
}

/// Width and height from the resize sheet. An empty height keeps the shape.
pub(super) fn resize_edit(width: u32, height: u32, values: &[String]) -> Option<ImageEdit> {
    let new_width = values.first()?.trim().parse::<u32>().ok().filter(|v| *v > 0 && *v <= 100_000)?;
    let new_height = match values.get(1).map(|v| v.trim()) {
        None | Some("") => {
            ((height as u64 * new_width as u64 + width.max(1) as u64 / 2) / width.max(1) as u64).max(1) as u32
        }
        Some(text) => text.parse::<u32>().ok()?,
    };
    (new_height > 0 && new_width as u64 * new_height as u64 <= 200_000_000)
        .then_some(ImageEdit::Resize { width: new_width, height: new_height })
}

unsafe fn start_export(hwnd: HWND, job: Job, status: &str) {
    with_state(|s| {
        if s.exporting {
            return;
        }
        s.exporting = true;
        s.status = status.into();
        if !s.send(job) {
            s.exporting = false;
            s.status = "The processing worker stopped.".into();
        }
    });
    invalidate(hwnd);
}

unsafe fn batch(hwnd: HWND, command: Command, request: Request) {
    let selected = if command == Command::BatchSelected {
        match choose_many(hwnd, true) {
            Some(paths) if !paths.is_empty() => Some(paths),
            _ => return,
        }
    } else {
        None
    };
    let Some(values) =
        sheet::input(hwnd, "Convert images", &[("Format: png, jpg, tif, bmp, pdf, or webp", "png".into())])
    else {
        return;
    };
    let extension = values.first().map(|v| v.trim().trim_start_matches('.').to_lowercase()).unwrap_or_default();
    if !matches!(extension.as_str(), "png" | "jpg" | "tif" | "bmp" | "pdf" | "webp") {
        sheet::alert(hwnd, "Convert images", "Choose one of these formats: png, jpg, tif, bmp, pdf, or webp.");
        return;
    }
    let Some(output) = folder(hwnd) else {
        return;
    };
    let question = match &selected {
        Some(paths) => format!("Apply the current image edits to {} images and save new copies? Existing files are not replaced.", paths.len()),
        None => "Apply the current image edits to every image in this folder and save new copies? Existing files are not replaced.".into(),
    };
    if !sheet::confirm(hwnd, "Convert images?", &question, "Convert") {
        return;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    with_state(|s| s.cancel = Some(cancel.clone()));
    start_export(hwnd, Job::Batch(request, output, extension, cancel, selected), "Starting batch conversion...");
}

unsafe fn choose_tool(hwnd: HWND, command: Command) {
    if with_state(|s| s.tool() == Some(command)).unwrap_or(false) {
        // Choosing the active tool again turns it off.
        with_state(|s| {
            s.markup = None;
            s.signature = None;
            s.status = "Markup is off.".into();
        });
        invalidate(hwnd);
        return;
    }
    let signature = if command == Command::PlaceSignature {
        match load_signature() {
            Ok(points) => Some(points),
            Err(error) => {
                sheet::alert(hwnd, "Signature", &error);
                return;
            }
        }
    } else {
        None
    };
    let kind = commands::annotation(command).unwrap_or(AnnotationKind::Ink);
    let text = if matches!(kind, AnnotationKind::Text | AnnotationKind::Note) {
        let title = if kind == AnnotationKind::Note { "Add a note" } else { "Add a text box" };
        let Some(values) = sheet::input(hwnd, title, &[("Text", String::new())]) else {
            return;
        };
        values.into_iter().next().unwrap_or_default()
    } else {
        String::new()
    };
    with_state(|s| {
        s.crop = false;
        s.markup = Some(kind);
        s.markup_text = text;
        s.signature = signature;
        s.set_markup(true);
        s.status = "Drag on the page to place the mark. Escape returns to navigation.".into();
    });
    invalidate(hwnd);
}

/// Lists the editable form fields on this page, then asks for a value.
pub(super) unsafe fn fill_field(hwnd: HWND, generation: u64, fields: Vec<crate::pdf::FormField>) {
    let fields: Vec<_> = fields
        .into_iter()
        .filter(|f| !f.read_only && !matches!(f.kind, crate::pdf::FormFieldKind::Unsupported))
        .collect();
    if fields.is_empty() {
        sheet::alert(hwnd, "Fill a form", "This page has no fields Preview can fill. For a PDF without a form, use Text box.");
        return;
    }
    let items = fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let name = if field.name.is_empty() { "Unnamed field" } else { &field.name };
            MenuItem::choice(&format!("{name}: {}", field.value), index)
        })
        .collect();
    let Some(Pick::Index(chosen)) = popup(hwnd, items, None, None, true) else {
        return;
    };
    let Some(field) = fields.get(chosen) else {
        return;
    };
    let checkbox = matches!(field.kind, crate::pdf::FormFieldKind::Checkbox);
    let label = if checkbox { "Checked: true or false" } else { field.name.as_str() };
    let Some(values) = sheet::input(hwnd, "Fill a form field", &[(label, field.value.clone())]) else {
        return;
    };
    let Some(value) = values.into_iter().next() else {
        return;
    };
    if checkbox && value != "true" && value != "false" {
        sheet::alert(hwnd, "Fill a form field", "Enter true or false for this checkbox.");
        return;
    }
    with_state(|s| {
        if s.generation != generation {
            return;
        }
        if let Some(path) = s.path.clone() {
            let edits = s.sessions.entry(path).or_default();
            edits.pdf.push(PdfEdit::FillField { page: field.page, annotation_index: field.annotation_index, value });
            edits.dirty = true;
            schedule(hwnd, s, 0);
        }
    });
}

/// Right-click or the context menu key. `at` is in client pixels; None
/// means the keyboard opened it, so the menu goes at the focused widget.
pub(super) unsafe fn context_menu(hwnd: HWND, at: Option<(f32, f32)>) {
    let Some((target, ctx)) = with_state(|s| {
        let layout = s.layout();
        let target = match at {
            Some((x, y)) => widgets::hit(&layout.widgets, x, y).map(|w| w.id),
            None => s.focus,
        };
        (target, s.ctx())
    }) else {
        return;
    };
    let (items, anchor) = match target {
        Some(WidgetId::Tab(index)) | Some(WidgetId::TabClose(index)) => {
            select_tab(hwnd, index);
            (commands::tab_menu(&ctx), Some(WidgetId::Tab(index)))
        }
        Some(WidgetId::Document) => (commands::document_menu(&ctx), None),
        _ if at.is_none() => (commands::document_menu(&ctx), None),
        _ => return,
    };
    let anchor = if at.is_some() { None } else { anchor };
    if let Some(Pick::Command(command)) = popup(hwnd, items, anchor, at, at.is_none()) {
        execute(hwnd, command, at.is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_edits_from_a_sheet_need_the_same_page_and_generation() {
        use super::super::{app::State, theme, worker::Workers};
        use std::{collections::HashMap, path::PathBuf};
        let path = PathBuf::from("report.pdf");
        let workers = Workers::start(HWND::default()).unwrap();
        let mut s = State::new(workers, &[], (800.0, 600.0), 1.0, 1.0, theme::palette(theme::Mode::Light));
        s.path = Some(path.clone());
        s.page = 3;
        s.generation = 9;
        let request = Request { generation: 9, path, page: 3, delta: 0, width: 1, height: 1, sessions: HashMap::new() };
        assert!(same_target(&s, &request));
        s.page = 7;
        assert!(!same_target(&s, &request), "a Find result moved to page 8");
        s.page = 3;
        s.generation = 10;
        assert!(!same_target(&s, &request), "a slideshow step rendered again");
        s.generation = 9;
        s.path = Some(PathBuf::from("other.pdf"));
        assert!(!same_target(&s, &request), "another tab");
    }

    #[test]
    fn resize_keeps_shape_when_height_is_empty_and_rejects_bad_sizes() {
        let v = |a: &str, b: &str| vec![a.to_string(), b.to_string()];
        assert_eq!(resize_edit(400, 300, &v("200", "")), Some(ImageEdit::Resize { width: 200, height: 150 }));
        assert_eq!(resize_edit(400, 300, &v(" 200 ", "50")), Some(ImageEdit::Resize { width: 200, height: 50 }));
        assert_eq!(resize_edit(400, 300, &v("0", "")), None);
        assert_eq!(resize_edit(400, 300, &v("abc", "")), None);
        assert_eq!(resize_edit(400, 300, &v("100000", "100000")), None);
        assert_eq!(resize_edit(3, 1, &v("1", "")), Some(ImageEdit::Resize { width: 1, height: 1 }));
    }
}
