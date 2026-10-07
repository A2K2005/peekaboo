use super::*;

pub(super) const OPEN: usize = 100;
pub(super) const EXIT: usize = 101;
pub(super) const PREVIOUS: usize = 102;
pub(super) const NEXT: usize = 103;
pub(super) const FIT: usize = 104;
pub(super) const ZOOM_IN: usize = 105;
pub(super) const ZOOM_OUT: usize = 106;
pub(super) const ROTATE: usize = 107;
pub(super) const CROP: usize = 108;
pub(super) const SAVE: usize = 109;
pub(super) const FLIP: usize = 110;
pub(super) const UNDO: usize = 111;
pub(super) const REVERT: usize = 112;
pub(super) const RESIZE: usize = 113;
pub(super) const TEXT: usize = 114;
pub(super) const FIND: usize = 115;
pub(super) const EXTRACT: usize = 116;
pub(super) const MERGE: usize = 117;
pub(super) const DELETE: usize = 118;
pub(super) const FORM: usize = 119;
pub(super) const BACKGROUND: usize = 120;
pub(super) const PRINT: usize = 121;
pub(super) const BATCH: usize = 122;
pub(super) const SLIDESHOW: usize = 123;
pub(super) const INFO: usize = 124;
pub(super) const BATCH_SELECTED: usize = 125;
pub(super) const MOVE: usize = 126;
pub(super) const INSERT: usize = 127;
pub(super) const TABS: usize = 300;
pub(super) const SIGN_SAVE: usize = 219;
pub(super) const SIGN_PLACE: usize = 220;

pub(super) unsafe fn command(hwnd: HWND, id: usize) {
    if id == OPEN {
        if let Some(paths) = choose_many(hwnd, false) {
            add_tabs(hwnd, &paths);
            if let Some(path) = paths.into_iter().next() {
                open(hwnd, path);
            }
        }
        return;
    }
    if id == EXIT {
        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        return;
    }
    if id == PREVIOUS || id == NEXT {
        navigate(hwnd, if id == PREVIOUS { -1 } else { 1 });
        return;
    }
    let snapshot = STATE.with(|cell| {
        cell.borrow().as_ref().and_then(|s| {
            if s.pending || (s.render_failed && !matches!(id, UNDO | REVERT)) || s.frame.is_none() {
                return None;
            }
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
                s.frame.as_ref()?.source_width,
                s.frame.as_ref()?.source_height,
                s.frame.as_ref()?.page_count,
            ))
        })
    });
    let Some((request, width, height, count)) = snapshot else {
        return;
    };
    let pdf = is_pdf(&request.path);
    if id == INFO {
        let bytes = std::fs::metadata(&request.path)
            .map(|m| m.len())
            .unwrap_or(0);
        let text=wide(&format!("{}\n\nSource dimensions: {width} × {height}\nPages: {count}\nFile size: {bytes} bytes\n\nEdits are kept in memory until you save a new copy.",request.path.display()));
        MessageBoxW(
            Some(hwnd),
            PCWSTR(text.as_ptr()),
            w!("File information"),
            MB_OK,
        );
        return;
    }
    if id == SLIDESHOW {
        STATE.with(|cell| {
            if let Some(state) = cell.borrow_mut().as_mut() {
                state.slideshow = if state.slideshow.is_some() {
                    None
                } else {
                    Some(Instant::now() + Duration::from_secs(3))
                };
                state.status = "Slideshow advances every 3 seconds. Escape stops.".into();
            }
        });
        return;
    }
    if id == PRINT {
        match crate::printing::choose(hwnd, count) {
            Ok(Some(job)) => {
                let cancel = job.cancellation();
                STATE.with(|cell| {
                    if let Some(state) = cell.borrow_mut().as_mut() {
                        if state.exporting {
                            return;
                        }
                        state.exporting = true;
                        state.cancel = Some(cancel);
                        state.status = "Printing... Escape cancels.".into();
                        if state.sender.send(Job::Print(request, job)).is_err() {
                            state.exporting = false;
                            state.status = "The print worker stopped.".into();
                        }
                    }
                });
            }
            Ok(None) => {}
            Err(error) => {
                let text = wide(&error);
                MessageBoxW(Some(hwnd), PCWSTR(text.as_ptr()), w!("Print"), MB_OK);
            }
        }
        let _ = InvalidateRect(Some(hwnd), None, false);
        return;
    }
    if id == BATCH || id == BATCH_SELECTED {
        let selected = if id == BATCH_SELECTED {
            let Some(paths) = choose_many(hwnd, true) else {
                return;
            };
            if paths.is_empty() {
                return;
            }
            Some(paths)
        } else {
            None
        };
        if pdf {
            return;
        }
        let Some(values) = input(
            hwnd,
            "Batch convert image folder",
            &[(
                "Output format: png, jpg, tif, bmp, pdf, or webp",
                "png".into(),
            )],
        ) else {
            return;
        };
        let extension = values
            .first()
            .map(|v| v.trim().to_lowercase())
            .unwrap_or_default();
        if !matches!(
            extension.as_str(),
            "png" | "jpg" | "tif" | "bmp" | "pdf" | "webp"
        ) {
            return;
        }
        let Some(output) = folder(hwnd) else {
            return;
        };
        let question = wide(&if let Some(paths) = &selected {
            format!("Apply the current image edits to {} selected images and save new copies? Existing files are not overwritten.",paths.len())
        } else {
            "Apply the current image edits to every supported image in this folder and save new copies? Existing files are not overwritten.".into()
        });
        if MessageBoxW(
            Some(hwnd),
            PCWSTR(question.as_ptr()),
            w!("Batch convert"),
            MB_YESNO | MB_ICONQUESTION,
        ) != IDYES
        {
            return;
        }
        STATE.with(|cell| {
            if let Some(state) = cell.borrow_mut().as_mut() {
                if state.exporting {
                    return;
                }
                let cancel = Arc::new(AtomicBool::new(false));
                state.cancel = Some(cancel.clone());
                state.exporting = true;
                state.status = "Starting batch conversion...".into();
                if state
                    .sender
                    .send(Job::Batch(request, output, extension, cancel, selected))
                    .is_err()
                {
                    state.exporting = false;
                    state.status = "The processing worker stopped.".into();
                }
            }
        });
        let _ = InvalidateRect(Some(hwnd), None, false);
        return;
    }
    if id == BACKGROUND {
        if pdf {
            return;
        }
        let Some(mut output) = destination(hwnd, false) else {
            return;
        };
        output.set_extension("png");
        STATE.with(|cell| {
            if let Some(state) = cell.borrow_mut().as_mut() {
                if state.exporting {
                    return;
                }
                state.exporting = true;
                state.status = "Removing background on this PC...".into();
                if state.sender.send(Job::Background(request, output)).is_err() {
                    state.exporting = false;
                    state.status = "The processing worker stopped.".into();
                }
            }
        });
        let _ = InvalidateRect(Some(hwnd), None, false);
        return;
    }
    if (200..=208).contains(&id) || id == SIGN_SAVE || id == SIGN_PLACE {
        if id == SIGN_SAVE {
            save_signature(hwnd);
            return;
        }
        let signature = if id == SIGN_PLACE {
            match load_signature() {
                Ok(points) => Some(points),
                Err(error) => {
                    let error = wide(&error);
                    MessageBoxW(Some(hwnd), PCWSTR(error.as_ptr()), w!("Signature"), MB_OK);
                    return;
                }
            }
        } else {
            None
        };
        let kind = match id {
            201 => AnnotationKind::Highlight,
            202 => AnnotationKind::Underline,
            203 => AnnotationKind::Strikeout,
            204 => AnnotationKind::Note,
            205 => AnnotationKind::Rectangle,
            206 => AnnotationKind::Ellipse,
            207 => AnnotationKind::Arrow,
            208 => AnnotationKind::Text,
            _ => AnnotationKind::Ink,
        };
        let text = if matches!(kind, AnnotationKind::Text | AnnotationKind::Note) {
            let Some(values) = input(hwnd, "PDF annotation", &[("Text", String::new())]) else {
                return;
            };
            values.first().cloned().unwrap_or_default()
        } else {
            String::new()
        };
        STATE.with(|cell| {
            if let Some(state) = cell.borrow_mut().as_mut() {
                state.crop = false;
                state.markup = Some(kind);
                state.markup_text = text;
                state.signature = signature;
                state.status =
                    "Drag on the PDF to place markup. Escape returns to navigation.".into();
            }
        });
        let _ = InvalidateRect(Some(hwnd), None, false);
        return;
    }
    if matches!(id, SAVE | EXTRACT | MERGE) {
        if id != SAVE && !pdf {
            return;
        }
        let other = if id == MERGE {
            let Some(path) = choose(hwnd) else {
                return;
            };
            Some(path)
        } else {
            None
        };
        let Some(output) = destination(hwnd, pdf) else {
            return;
        };
        STATE.with(|cell| {
            if let Some(state) = cell.borrow_mut().as_mut() {
                if state.exporting {
                    return;
                }
                state.exporting = true;
                state.status = "Saving a new copy...".into();
                if state
                    .sender
                    .send(Job::Save(request, output, id, other))
                    .is_err()
                {
                    state.exporting = false;
                    state.status = "The save worker stopped.".into();
                }
            }
        });
        return;
    }
    if id == TEXT || id == FIND || id == FORM {
        if !pdf && id != TEXT {
            return;
        }
        let job = if id == FORM {
            Job::Fields(request)
        } else if id == TEXT {
            Job::Text(request)
        } else {
            let Some(values) = input(hwnd, "Find text in PDF", &[("Text to find", String::new())])
            else {
                return;
            };
            let Some(query) = values.first().filter(|v| !v.trim().is_empty()) else {
                return;
            };
            Job::Find(request, query.clone())
        };
        STATE.with(|cell| {
            if let Some(state) = cell.borrow_mut().as_mut() {
                let _ = state.sender.send(job);
                state.status = "Reading text on this PC...".into();
            }
        });
        let _ = InvalidateRect(Some(hwnd), None, false);
        return;
    }
    let mut resize = None;
    let mut page_destination = None;
    if id == MOVE {
        if !pdf {
            return;
        }
        let Some(values) = input(
            hwnd,
            "Move PDF page",
            &[("New page number (1-based)", (request.page + 1).to_string())],
        ) else {
            return;
        };
        page_destination = values
            .first()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|v| *v > 0 && *v <= count)
            .map(|v| v - 1);
        if page_destination.is_none() {
            MessageBoxW(
                Some(hwnd),
                w!("Enter a page number within this document."),
                w!("Invalid page number"),
                MB_OK,
            );
            return;
        }
    }
    if id == RESIZE {
        if pdf {
            return;
        }
        let Some(values) = input(
            hwnd,
            "Resize image",
            &[
                ("Width in pixels", width.to_string()),
                ("Height in pixels (blank keeps aspect ratio)", String::new()),
            ],
        ) else {
            return;
        };
        let new_width = values.first().and_then(|v| v.trim().parse::<u32>().ok());
        if let Some(new_width) = new_width.filter(|v| *v > 0 && *v <= 100000) {
            let new_height = if values.get(1).is_none_or(|v| v.trim().is_empty()) {
                ((height as u64 * new_width as u64 + width.max(1) as u64 / 2) / width.max(1) as u64)
                    .max(1) as u32
            } else {
                values[1].trim().parse::<u32>().unwrap_or(0)
            };
            if new_height > 0 && (new_width as u64 * new_height as u64) <= 200_000_000 {
                resize = Some(ImageEdit::Resize {
                    width: new_width,
                    height: new_height,
                });
            }
        }
        if resize.is_none() {
            MessageBoxW(
                Some(hwnd),
                w!("Enter positive pixel dimensions within 200 million pixels."),
                w!("Invalid size"),
                MB_OK | MB_ICONERROR,
            );
            return;
        }
    }
    if id == DELETE && (!pdf || count <= 1 || MessageBoxW(Some(hwnd),w!("Delete this page from the working copy? The original stays unchanged until you save a new copy."),w!("Delete page"),MB_YESNO|MB_ICONQUESTION) != IDYES) { return; }
    STATE.with(|cell| { let mut value = cell.borrow_mut(); let Some(state) = value.as_mut() else { return; };
        if matches!(id,FIT|ZOOM_IN|ZOOM_OUT) { state.zoom = match id { FIT => 1.0, ZOOM_IN => (state.zoom * 1.4).min(8.0), _ => (state.zoom / 1.4).max(0.25) }; state.pan=(0.0,0.0); schedule(hwnd,state,0); return; }
        if id == CROP {state.markup=None;state.crop = !state.crop; state.status = if pdf{"Drag a rectangle to crop the page view. This does not redact hidden content. Escape cancels.".into()}else{"Drag a rectangle over the image to crop. Escape cancels.".into()};let _ = InvalidateRect(Some(hwnd),None,false);return;}
        let edits = state.sessions.entry(request.path).or_default();
        match id {
            ROTATE if pdf => edits.pdf.push(PdfEdit::RotateRight { page:state.page }),
            ROTATE => edits.image.push(ImageEdit::RotateRight),
            FLIP if !pdf => edits.image.push(ImageEdit::FlipHorizontal),
            RESIZE => { if let Some(edit) = resize { edits.image.push(edit); } },
            DELETE => { edits.pdf.push(PdfEdit::Delete { page:state.page }); state.page = state.page.min(count.saturating_sub(2)); },
            MOVE=>{if let Some(to)=page_destination{edits.pdf.push(PdfEdit::Move{from:state.page,to});state.page=to;}},
            INSERT if pdf=>{let at=state.page+1;edits.pdf.push(PdfEdit::InsertBlank{at});state.page=at;},
            UNDO => { if pdf { edits.pdf.pop(); } else { edits.image.pop(); } },
            REVERT => { edits.pdf.clear(); edits.image.clear(); state.page=0; },
            _ => return,
        }
        edits.dirty = !edits.image.is_empty() || !edits.pdf.is_empty(); state.zoom=1.0; state.pan=(0.0,0.0); schedule(hwnd,state,0);
    });
}

pub(super) unsafe fn update_controls(hwnd: HWND) {
    let Some((ready, pdf, saving, previous, next)) = STATE.with(|cell| {
        cell.borrow().as_ref().map(|s| {
            let ready = s.frame.is_some() && !s.pending && !s.render_failed;
            let pdf = s.path.as_ref().is_some_and(|p| is_pdf(p));
            (
                ready,
                pdf,
                s.exporting,
                !pdf || s.page > 0,
                !pdf || s.frame.as_ref().is_some_and(|f| s.page + 1 < f.page_count),
            )
        })
    }) else {
        return;
    };
    let current = (ready, pdf, saving, previous, next);
    if CONTROL_STATE.with(|cell| {
        let mut old = cell.borrow_mut();
        if *old == Some(current) {
            true
        } else {
            *old = Some(current);
            false
        }
    }) {
        return;
    }
    let menu = GetMenu(hwnd);
    for id in [
        PREVIOUS,
        NEXT,
        FIT,
        ZOOM_IN,
        ZOOM_OUT,
        ROTATE,
        CROP,
        SAVE,
        FLIP,
        RESIZE,
        TEXT,
        FIND,
        EXTRACT,
        MERGE,
        DELETE,
        FORM,
        BACKGROUND,
        PRINT,
        BATCH,
        BATCH_SELECTED,
        MOVE,
        INSERT,
        SLIDESHOW,
        INFO,
        SIGN_SAVE,
        SIGN_PLACE,
        200,
        201,
        202,
        203,
        204,
        205,
        206,
        207,
        208,
    ] {
        let enabled = ready
            && match id {
                FLIP | RESIZE | BACKGROUND | BATCH | BATCH_SELECTED => !pdf,
                FIND | EXTRACT | MERGE | DELETE | FORM | MOVE | INSERT => pdf,
                PREVIOUS => previous,
                NEXT => next,
                SAVE | PRINT => !saving,
                _ => true,
            };
        if let Ok(button) = GetDlgItem(Some(hwnd), id as i32) {
            let _ = EnableWindow(button, enabled);
        }
        let _ = EnableMenuItem(
            menu,
            id as u32,
            MF_BYCOMMAND | if enabled { MF_ENABLED } else { MF_GRAYED },
        );
    }
}

pub(super) unsafe fn fill_field(hwnd: HWND, generation: u64, fields: Vec<crate::pdf::FormField>) {
    let fields: Vec<_> = fields
        .into_iter()
        .filter(|f| !f.read_only && !matches!(f.kind, crate::pdf::FormFieldKind::Unsupported))
        .collect();
    if fields.is_empty() {
        MessageBoxW(Some(hwnd),w!("No editable supported fields on this page. Use Markup > Text box for a non-form PDF."),w!("Fill a form"),MB_OK);
        return;
    }
    let Ok(menu) = CreatePopupMenu() else {
        return;
    };
    for (index, field) in fields.iter().enumerate() {
        let text = wide(&format!(
            "{}: {}",
            if field.name.is_empty() {
                "Unnamed field"
            } else {
                &field.name
            },
            field.value
        ));
        let _ = AppendMenuW(menu, MF_STRING, index + 1, PCWSTR(text.as_ptr()));
    }
    let mut point = POINT::default();
    let _ = GetCursorPos(&mut point);
    let chosen = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_NONOTIFY,
        point.x,
        point.y,
        None,
        hwnd,
        None,
    )
    .0;
    let _ = DestroyMenu(menu);
    let Some(field) = fields
        .get(chosen.saturating_sub(1) as usize)
        .filter(|_| chosen > 0)
    else {
        return;
    };
    let label = if matches!(field.kind, crate::pdf::FormFieldKind::Checkbox) {
        "Checked: true or false"
    } else {
        &field.name
    };
    let Some(values) = input(hwnd, "Fill PDF field", &[(label, field.value.clone())]) else {
        return;
    };
    let Some(value) = values.first() else {
        return;
    };
    if matches!(field.kind, crate::pdf::FormFieldKind::Checkbox)
        && value != "true"
        && value != "false"
    {
        MessageBoxW(
            Some(hwnd),
            w!("Enter true or false for this checkbox."),
            w!("Invalid value"),
            MB_OK,
        );
        return;
    }
    STATE.with(|cell| {
        if let Some(state) = cell.borrow_mut().as_mut() {
            if state.generation != generation {
                return;
            }
            if let Some(path) = state.path.clone() {
                let edits = state.sessions.entry(path).or_default();
                edits.pdf.push(PdfEdit::FillField {
                    page: field.page,
                    annotation_index: field.annotation_index,
                    value: value.clone(),
                });
                edits.dirty = true;
                schedule(hwnd, state, 0);
            }
        }
    });
}
