//! Runs commands.
use super::{
    app::{add_tabs, close_tab, invalidate, navigate, open, schedule, select_tab, with_state, SaveStatus},
    commands::{self, Command, MenuItem, Pick},
    document,
    files::{choose, choose_many, destination},
    disk, menu, sheet,
    view::{ViewMode, Zoom},
    widgets::{self, WidgetId},
    worker::{Job, Request, SaveKind},
};
use crate::model::{AnnotationKind, ImageEdit, PdfEdit, PdfFormType, PdfMetadata};
use std::time::{Duration, Instant};
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
    if state.frame.is_some() || state.pdf.is_some() {
        state.due = Some(Instant::now() + Duration::from_millis(120));
    }
}

fn toggle_sidebar(state: &mut super::app::State) {
    set_sidebar(state, !state.sidebar_open);
}

/// Shows or hides the PDF sidebar and remembers the choice for this file.
fn set_sidebar(state: &mut super::app::State, open: bool) {
    if state.path.is_some() && !state.is_pdf() {
        return;
    }
    state.sidebar_open = open;
    if let Some(path) = state.path.clone() {
        state.sidebars.insert(path, open);
    }
    if !state.sidebar_open && matches!(state.focus, Some(WidgetId::SidebarTab(_) | WidgetId::SidebarItem(_))) {
        let layout = state.layout();
        state.focus = [WidgetId::Document, WidgetId::Command(Command::Open), WidgetId::Command(Command::ToggleSidebar)]
            .into_iter()
            .find(|id| layout.widgets.iter().any(|w| w.id == *id && w.enabled && w.focusable));
    }
    relayout(state);
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
        NewFromClipboard => {
            super::empty::new_from_clipboard(hwnd);
            return;
        }
        Exit => {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            return;
        }
        BatchSelected => {
            if let Some(paths) = choose_many(hwnd, true).filter(|paths| !paths.is_empty()) {
                super::imagetools::batch(hwnd, paths, false);
            }
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
        ToggleSidebar | ToggleMarkup | SidebarHide | SidebarThumbnails | SidebarContents | SidebarNotes | SidebarSheet => {
            with_state(|s| match command {
                ToggleSidebar => toggle_sidebar(s),
                SidebarHide => set_sidebar(s, false),
                // The document renders again when the slide ends (tick).
                ToggleMarkup => s.set_markup(!s.markup_open),
                mode => {
                    let index = commands::SIDEBAR_MODES.iter().position(|m| *m == mode).unwrap_or(0);
                    set_sidebar(s, true);
                    super::sidebar::show(s, index);
                }
            });
            invalidate(hwnd);
            return;
        }
        FullScreen => {
            super::window::toggle_full_screen(hwnd);
            return;
        }
        NextPane | PreviousPane => {
            with_state(|s| {
                let layout = s.layout();
                s.focus = widgets::next_region(&layout.widgets, s.focus, command == PreviousPane).or(s.focus);
                s.focus_visible = true;
            });
            super::findbar::follow_focus();
            invalidate(hwnd);
            return;
        }
        AppMenu | MoreTools | ShapesMenu | HighlightMenu | SignMenu => {
            let overflow = with_state(|s| {
                let layout = s.layout();
                (layout.toolbar_overflow, layout.markup_overflow)
            })
            .unwrap_or_default();
            let items = match command {
                AppMenu => commands::app_menu(&ctx, &overflow.0),
                MoreTools => commands::markup_overflow_menu(&overflow.1, &ctx),
                _ => commands::tool_menu(command, &ctx),
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
        // PDF pages give their size in points; images in pixels.
        let (width, height, count) = match (&s.pdf, &s.frame) {
            (Some(v), _) => {
                let [w, h] = *v.sizes.get(s.page as usize)?;
                (w.round() as u32, h.round() as u32, v.sizes.len() as u32)
            }
            (None, Some(frame)) => (frame.source_width, frame.source_height, frame.page_count),
            _ => return None,
        };
        Some((
            Request {
                generation: s.generation,
                path: s.path.clone()?,
                page: s.page,
                delta: 0,
                width: 1,
                height: 1,
                sessions: s.sessions.clone(),
                sources: s.opened_sources(),
            },
            width,
            height,
            count,
            s.text_selection,
        ))
    })
    .flatten();
    let Some((request, width, height, count, text_selection)) = snapshot else {
        return;
    };
    let pdf = super::worker::is_pdf(&request.path);
    match command {
        FileInfo if pdf => {
            // PDF metadata comes from the document worker; tick shows it.
            with_state(|s| {
                if !s.send(Job::Metadata(request)) {
                    s.status = "The PDF worker stopped.".into();
                }
            });
        }
        FileInfo => sheet::alert(hwnd, "File information", &image_info(&request.path, width, height)),
        Share => {
            if let Err(error) = crate::integration::share_files(hwnd, std::slice::from_ref(&request.path)) {
                sheet::alert(hwnd, "Share", &error);
            }
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
        BatchFolder => super::imagetools::batch_folder(hwnd, &request.path),
        RemoveBackground => super::imagetools::remove_background(hwnd, request),
        SaveCopy if !pdf => super::imagetools::export(hwnd, request, width, height),
        SaveSignature => super::forms::new_signature(hwnd),
        FillForm => super::forms::start_form(hwnd),
        PlaceSignature | Draw | Highlight | Underline | Strikethrough | Note | TextBox | Rectangle | Ellipse | Arrow => {
            choose_tool(hwnd, command)
        }
        SelectText => {
            with_state(|s| {
                s.markup = None;
                s.signature = None;
                s.crop = false;
                s.tools.crop = None;
                s.zoom_select = false;
            });
            invalidate(hwnd);
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
        FindNext | FindPrevious => {
            with_state(|s| {
                if s.find_hits.is_empty() {
                    s.status = "Press Ctrl+F to search.".into();
                    return;
                }
                s.find_index = if command == FindNext {
                    (s.find_index + 1) % s.find_hits.len()
                } else {
                    (s.find_index + s.find_hits.len() - 1) % s.find_hits.len()
                };
                let page = s.find_hits[s.find_index].page;
                if s.pdf.is_some() {
                    document::go_to_page(s, page, true);
                } else {
                    s.page = page;
                }
                s.status = format!("Match {} of {}", s.find_index + 1, s.find_hits.len());
            });
            invalidate(hwnd);
        }
        Find => super::findbar::open(hwnd),
        CopyText => {
            let job = Job::Text(request, text_selection.filter(|selection| !selection.is_empty()));
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
        Resize => super::imagetools::resize(hwnd, request, width, height),
        DeletePage => {
            if count <= 1
                || !sheet::confirm(
                    hwnd,
                    "Delete this page?",
                    "The page is removed and the file saves automatically. Revert to opened brings it back.",
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
        Fit | FitWidth | ActualSize | ZoomIn | ZoomOut => {
            with_state(|s| match command {
                ZoomIn | ZoomOut => document::zoom_step(s, command == ZoomIn),
                // Ctrl+\ goes back to fit to window when the width already fits.
                FitWidth if s.zoom == Zoom::FitWidth => document::set_zoom(s, Zoom::Fit, None),
                FitWidth => document::set_zoom(s, Zoom::FitWidth, None),
                ActualSize => document::set_zoom(s, Zoom::Ratio(1.0), None),
                _ => document::set_zoom(s, Zoom::Fit, None),
            });
            invalidate(hwnd);
        }
        ZoomToSelection => {
            with_state(|s| {
                s.markup = None;
                s.signature = None;
                s.crop = false;
                s.zoom_select = !s.zoom_select;
                s.status = if s.zoom_select { "Drag over the area to zoom to. Escape cancels." } else { "Zoom to selection is off." }.into();
            });
            invalidate(hwnd);
        }
        ViewContinuous | ViewSingle | ViewTwoPages => {
            with_state(|s| {
                s.view_mode = match command {
                    ViewSingle => ViewMode::Single,
                    ViewTwoPages => ViewMode::TwoPages,
                    _ => ViewMode::Continuous,
                };
                let page = s.page;
                document::go_to_page(s, page, false);
            });
            invalidate(hwnd);
        }
        Crop => {
            with_state(|s| {
                s.markup = None;
                s.signature = None;
                s.zoom_select = false;
                s.crop = !s.crop;
                s.tools.crop = None;
                s.status = if !s.crop {
                    "Crop is off.".into()
                } else if pdf {
                    "Drag a rectangle to crop the page view. Hidden content stays in the file. Escape cancels.".into()
                } else {
                    super::imagetools::CROP_HINT.into()
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
        MovePageUp | MovePageDown => super::organize::step(hwnd, &request, count, command == MovePageDown),
        InsertImagePage => super::organize::insert_images(hwnd, &request),
        Undo => edit(hwnd, |s, edits| {
            if pdf {
                let undone = edits.pdf.pop();
                s.page = page_after_undo(s.page, count, undone.as_ref());
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

/// The page count after undoing `undone`, and `page` kept inside it. An
/// undone insert removes a page; an undone delete brings one back.
pub(super) fn page_after_undo(page: u32, count: u32, undone: Option<&PdfEdit>) -> u32 {
    let count = match undone {
        Some(PdfEdit::InsertBlank { .. } | PdfEdit::InsertImage { .. }) => count.saturating_sub(1),
        Some(PdfEdit::Delete { .. }) => count + 1,
        _ => count,
    };
    page.min(count.saturating_sub(1))
}

/// True while the file, page, and render generation still match a request
/// captured before a sheet opened. A Find result or a slideshow step can
/// move to another page while the sheet waits.
pub(super) fn same_target(s: &super::app::State, request: &Request) -> bool {
    s.generation == request.generation && s.page == request.page && s.path.as_ref() == Some(&request.path)
}

/// Applies a page edit chosen in a sheet, only if the page did not change
/// while the sheet was open.
pub(super) unsafe fn edit_same_page(hwnd: HWND, request: &Request, change: impl FnOnce(&mut super::app::State, &mut super::worker::Edits)) {
    if with_state(|s| same_target(s, request)) != Some(true) {
        with_state(|s| s.status = "The page changed while the dialog was open. Nothing was edited.".into());
        invalidate(hwnd);
        return;
    }
    edit(hwnd, change);
}

/// Points autosave at the file itself on the first edit. A file Preview
/// cannot write asks where to save a copy instead.
pub(super) unsafe fn prepare_save(hwnd: HWND, path: &std::path::Path) -> bool {
    let Some((session_id, opened, configured)) = with_state(|s| {
        s.saves.get(path).map(|save| (save.id, save.opened, save.target.is_some()))
    })
    .flatten()
    else {
        sheet::alert(hwnd, "Cannot edit this file", "Preview has not finished opening this file. Try again in a moment.");
        return false;
    };
    if configured {
        return true;
    }
    let (target, expected) = match disk::can_overwrite(path) {
        Ok(()) => (path.to_path_buf(), Some(opened)),
        Err(error) => {
            let message = format!("{error} Choose where to save a copy with your edits.");
            if !sheet::confirm(hwnd, "Save edits to a copy", &message, "Choose location") {
                return false;
            }
            let Some(target) = destination(hwnd, super::worker::is_pdf(path)) else {
                return false;
            };
            (target, None)
        }
    };
    with_state(|s| {
        let Some(save) = s.saves.get_mut(path).filter(|save| save.id == session_id) else {
            return false;
        };
        save.configure(target, expected);
        true
    }) == Some(true)
}

pub(super) unsafe fn resolve_save_conflict(hwnd: HWND, path: std::path::PathBuf) {
    let conflict = with_state(|s| {
        s.saves
            .get(&path)
            .filter(|save| matches!(save.status, SaveStatus::Conflict))
            .and_then(|save| save.target.clone().map(|target| (save.id, target)))
    })
    .flatten();
    let Some((session_id, current_target)) = conflict else {
        return;
    };
    let message = format!(
        "{} changed outside Preview while edits were being saved. Overwrite that version, save your edits to a new copy, or keep autosave paused.",
        super::app::file_name(&current_target)
    );
    let Some((button, _)) = sheet::ask(
        hwnd,
        "File changed",
        &message,
        &[],
        false,
        &["Overwrite changed file", "Save a copy", "Keep paused"],
        2,
        Some(2),
    ) else {
        return;
    };
    let resolution = match button {
        0 => Some((current_target, None, true)),
        1 => disk::copy_name(&path, None).map(|target| (target, None, false)),
        _ => None,
    };
    let Some((target, expected, force)) = resolution else {
        return;
    };
    with_state(|s| {
        if let Some(save) = s.saves.get_mut(&path).filter(|save| save.id == session_id) {
            save.resolve_conflict(target, expected, force);
        }
    });
    invalidate(hwnd);
}

/// Applies one recipe change to the open file and renders again.
pub(super) unsafe fn edit(hwnd: HWND, change: impl FnOnce(&mut super::app::State, &mut super::worker::Edits)) {
    let Some(path) = with_state(|s| s.path.clone()).flatten() else {
        return;
    };
    if !prepare_save(hwnd, &path) {
        return;
    }
    with_state(|s| {
        if s.path.as_ref() != Some(&path) {
            return;
        }
        let mut edits = s.sessions.remove(&path).unwrap_or_default();
        let before = (edits.image.clone(), edits.pdf.clone());
        change(s, &mut edits);
        let changed = before.0 != edits.image || before.1 != edits.pdf;
        if changed {
            edits.dirty = true;
            s.edited_for_save(&path, Instant::now());
        }
        s.sessions.insert(path, edits);
        if !changed {
            return;
        }
        if s.pdf.is_none() {
            // An edited image may change shape, so it shows whole again.
            s.zoom = Zoom::Fit;
            s.pan = (0.0, 0.0);
        }
        schedule(hwnd, s, 0);
    });
}

pub(super) unsafe fn start_export(hwnd: HWND, job: Job, status: &str) {
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
    if super::forms::tool(hwnd, command) {
        return;
    }
    let kind = commands::annotation(command).unwrap_or(AnnotationKind::Ink);
    let text = if kind == AnnotationKind::Note {
        let Some(values) = sheet::input(hwnd, "Add a note", &[("Text", String::new())]) else {
            return;
        };
        values.into_iter().next().unwrap_or_default()
    } else {
        String::new()
    };
    with_state(|s| {
        s.crop = false;
        s.zoom_select = false;
        s.markup = Some(kind);
        s.markup_text = text;
        s.signature = None;
        // The bar's Highlight button works without the markup bar, as in Preview.
        if command != Command::Highlight {
            s.set_markup(true);
        }
        s.status = if kind == AnnotationKind::Text {
            "Click where the text goes, or click a text box to change it. Escape returns to navigation."
        } else {
            "Drag on the page to place the mark. Escape returns to navigation."
        }
        .into();
    });
    invalidate(hwnd);
}

/// "2.4 MB" style sizes, in decimal units as File Explorer's details pane.
pub(super) fn file_size(bytes: u64) -> String {
    match bytes {
        0..=999 => format!("{bytes} bytes"),
        1_000..=999_999 => format!("{:.0} KB", bytes as f64 / 1e3),
        _ => format!("{:.1} MB", bytes as f64 / 1e6),
    }
}

/// Seconds since 1970 as "2026-10-07 13:19" (days to civil date from
/// Howard Hinnant's algorithm, http://howardhinnant.github.io/date_algorithms.html).
fn civil(seconds: i64) -> String {
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}", rest / 3600, rest % 3600 / 60)
}

/// A file time in the PC's time zone.
fn local_time(time: std::io::Result<std::time::SystemTime>) -> String {
    use windows::Win32::{Foundation::FILETIME, Storage::FileSystem::FileTimeToLocalFileTime};
    let Some(since) = time.ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()) else {
        return String::new();
    };
    // FILETIME counts 100 ns steps from 1601; Unix time starts 11,644,473,600 s later.
    let ticks = (since.as_nanos() / 100) as u64 + 116_444_736_000_000_000;
    let utc = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let mut local = FILETIME::default();
    if unsafe { FileTimeToLocalFileTime(&utc, &mut local) }.is_err() {
        return String::new();
    }
    let ticks = ((local.dwHighDateTime as u64) << 32) | local.dwLowDateTime as u64;
    civil((ticks / 10_000_000) as i64 - 11_644_473_600)
}

/// A PDF date ("D:20261006120000Z") as "2026-10-06 12:00", in the time
/// zone the file states.
fn pdf_date(raw: &str) -> String {
    let digits: String = raw.trim_start_matches("D:").chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.len() < 8 {
        return raw.to_string();
    }
    let part = |range: std::ops::Range<usize>| digits.get(range).unwrap_or("00");
    format!("{}-{}-{} {}:{}", &digits[0..4], &digits[4..6], &digits[6..8], part(8..10), part(10..12))
}

/// Lines for the info pane, skipping empty values.
fn info_lines(path: &std::path::Path, lines: &[(&str, String)]) -> String {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut text = name + "\n";
    for (label, value) in lines.iter().filter(|(_, v)| !v.trim().is_empty()) {
        text.push_str(&format!("\n{label}: {}", value.trim()));
    }
    text
}

fn image_info(path: &std::path::Path, width: u32, height: u32) -> String {
    let meta = std::fs::metadata(path);
    let format = path.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default();
    let format = match format.as_str() {
        "JPG" | "JPEG" => "JPEG".to_string(),
        "TIF" | "TIFF" => "TIFF".to_string(),
        other => other.to_string(),
    };
    info_lines(
        path,
        &[
            ("Dimensions", format!("{width} × {height} pixels")),
            ("Format", format),
            ("File size", meta.as_ref().map(|m| file_size(m.len())).unwrap_or_default()),
            ("Created", meta.as_ref().map(|m| local_time(m.created())).unwrap_or_default()),
            ("Modified", meta.as_ref().map(|m| local_time(m.modified())).unwrap_or_default()),
            ("Folder", path.parent().map(super::app::display_path).unwrap_or_default()),
        ],
    )
}

fn pdf_text(path: &std::path::Path, m: &PdfMetadata, page: Option<[f32; 2]>) -> String {
    let file = std::fs::metadata(path);
    let form = match m.form {
        PdfFormType::None => "",
        PdfFormType::AcroForm => "Fillable form",
        PdfFormType::XfaFull | PdfFormType::XfaForeground => "XFA form (needs Adobe Acrobat Reader)",
    };
    let size = page.map(|[w, h]| format!("{:.1} × {:.1} in ({w:.0} × {h:.0} points)", w / 72.0, h / 72.0)).unwrap_or_default();
    info_lines(
        path,
        &[
            ("Title", m.title.clone()),
            ("Author", m.author.clone()),
            ("Subject", m.subject.clone()),
            ("Keywords", m.keywords.clone()),
            ("Created", if m.created.is_empty() { String::new() } else { pdf_date(&m.created) }),
            ("Modified", if m.modified.is_empty() { String::new() } else { pdf_date(&m.modified) }),
            ("Creator", m.creator.clone()),
            ("Producer", m.producer.clone()),
            ("PDF version", m.version.clone()),
            ("Pages", m.page_count.to_string()),
            ("Page size", size),
            ("Form", form.to_string()),
            ("Password protected", if m.encrypted { "Yes".to_string() } else { String::new() }),
            ("File size", file.as_ref().map(|f| file_size(f.len())).unwrap_or_default()),
            ("Modified on disk", file.as_ref().map(|f| local_time(f.modified())).unwrap_or_default()),
        ],
    )
}

/// The info pane for the open PDF, once the worker read its metadata.
pub(super) unsafe fn pdf_info(hwnd: HWND, result: Result<PdfMetadata, String>) {
    let Some((path, page)) =
        with_state(|s| Some((s.path.clone()?, s.pdf.as_ref().and_then(|v| v.sizes.get(s.page as usize).copied())))).flatten()
    else {
        return;
    };
    match result {
        Ok(metadata) => sheet::alert(hwnd, "File information", &pdf_text(&path, &metadata, page)),
        Err(error) => sheet::alert(hwnd, "File information", &error),
    }
}

/// Right-click or the context menu key. `at` is in client pixels; None
/// means the keyboard opened it, so the menu goes at the focused widget.
pub(super) unsafe fn context_menu(hwnd: HWND, at: Option<(f32, f32)>) {
    let Some((target, ctx, thumbnails)) = with_state(|s| {
        let layout = s.layout();
        let target = match at {
            Some((x, y)) => widgets::hit(&layout.widgets, x, y).map(|w| w.id),
            None => s.focus,
        };
        (target, s.ctx(), s.sidebar_tab == 0 && s.pdf.is_some())
    }) else {
        return;
    };
    let (items, anchor) = match target {
        Some(WidgetId::Tab(index)) | Some(WidgetId::TabClose(index)) => {
            select_tab(hwnd, index);
            (commands::tab_menu(&ctx), Some(WidgetId::Tab(index)))
        }
        Some(WidgetId::SidebarItem(index)) if thumbnails => {
            let ctx = with_state(|s| {
                super::sidebar::activate(s, index);
                s.ctx()
            })
            .unwrap_or(ctx);
            (commands::page_menu(&ctx), Some(WidgetId::SidebarItem(index)))
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
    fn closing_sidebar_rehomes_focus_without_stealing_other_focus() {
        use super::super::{app::State, theme, worker::Workers};
        let workers = Workers::start(HWND::default()).unwrap();
        let mut s = State::new(workers, &[], (800.0, 600.0), 1.0, 1.0, theme::palette(theme::Mode::Light));
        for has_document in [false, true] {
            s.path = has_document.then(|| std::path::PathBuf::from("report.pdf"));
            for id in [WidgetId::SidebarTab(1), WidgetId::SidebarItem(20)] {
                s.sidebar_open = true;
                s.focus = Some(id);
                s.focus_visible = true;
                toggle_sidebar(&mut s);
                let expected = if has_document { WidgetId::Document } else { WidgetId::Command(Command::Open) };
                assert_eq!(s.focus, Some(expected));
                assert!(s.layout().widgets.iter().any(|w| Some(w.id) == s.focus && w.enabled && w.focusable));
                assert!(s.focus_visible);
                assert!(!s.sidebar_open);
            }
        }
        s.sidebar_open = true;
        s.focus = Some(WidgetId::Command(Command::ToggleSidebar));
        toggle_sidebar(&mut s);
        assert_eq!(s.focus, Some(WidgetId::Command(Command::ToggleSidebar)));
        assert!(s.layout().widgets.iter().any(|w| Some(w.id) == s.focus && w.enabled && w.focusable));
        s.path = Some(std::path::PathBuf::from("photo.png"));
        s.sidebar_open = true;
        toggle_sidebar(&mut s);
        assert!(s.sidebar_open && s.layout().sidebar.is_none(), "an image cannot change the saved PDF sidebar preference");
    }

    #[test]
    fn info_pane_formats_sizes_and_dates() {
        assert_eq!(file_size(512), "512 bytes");
        assert_eq!(file_size(379_281), "379 KB");
        assert_eq!(file_size(50_145_534), "50.1 MB");
        assert_eq!(civil(0), "1970-01-01 00:00");
        assert_eq!(civil(951_782_400), "2000-02-29 00:00");
        assert_eq!(civil(1_000_000_000), "2001-09-09 01:46");
        assert_eq!(pdf_date("D:20261006120000Z"), "2026-10-06 12:00");
        assert_eq!(pdf_date("D:2026"), "D:2026", "too short to read is shown as is");
        assert_eq!(pdf_date("20240229"), "2024-02-29 00:00");
        assert!(!local_time(Ok(std::time::SystemTime::now())).is_empty());
        let m = PdfMetadata {
            title: "Lease".into(),
            author: String::new(),
            subject: String::new(),
            keywords: String::new(),
            creator: String::new(),
            producer: "Preview".into(),
            created: "D:20261006120000Z".into(),
            modified: String::new(),
            version: "1.7".into(),
            page_count: 20,
            encrypted: false,
            form: PdfFormType::AcroForm,
        };
        let text = pdf_text(std::path::Path::new("missing/lease.pdf"), &m, Some([612.0, 792.0]));
        assert_eq!(
            text,
            "lease.pdf\n\nTitle: Lease\nCreated: 2026-10-06 12:00\nProducer: Preview\nPDF version: 1.7\nPages: 20\nPage size: 8.5 × 11.0 in (612 × 792 points)\nForm: Fillable form"
        );
        let image = image_info(std::path::Path::new("missing/photo.jpg"), 6000, 4000);
        assert!(image.starts_with("photo.jpg\n\nDimensions: 6000 × 4000 pixels\nFormat: JPEG"), "{image}");
    }

    #[test]
    fn undo_keeps_the_page_inside_the_document() {
        let insert = PdfEdit::InsertBlank { at: 5 };
        assert_eq!(page_after_undo(5, 6, Some(&insert)), 4, "undoing an insert on the last page");
        assert_eq!(page_after_undo(2, 6, Some(&insert)), 2);
        assert_eq!(page_after_undo(4, 5, Some(&PdfEdit::Delete { page: 4 })), 4);
        assert_eq!(page_after_undo(3, 4, Some(&PdfEdit::RotateRight { page: 3 })), 3);
        assert_eq!(page_after_undo(0, 1, None), 0);
    }

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
        let request = Request {
            generation: 9,
            path,
            page: 3,
            delta: 0,
            width: 1,
            height: 1,
            sessions: HashMap::new(),
            sources: HashMap::new(),
        };
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
}
