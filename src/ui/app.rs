use super::*;

pub(super) struct Graphics {
    pub(super) target: ID2D1HwndRenderTarget,
    pub(super) brush: ID2D1SolidColorBrush,
    pub(super) format: IDWriteTextFormat,
    pub(super) bitmap: Option<ID2D1Bitmap>,
}
pub(super) struct State {
    pub(super) sender: mpsc::Sender<Job>,
    pub(super) receiver: mpsc::Receiver<Event>,
    pub(super) path: Option<PathBuf>,
    pub(super) page: u32,
    pub(super) generation: u64,
    pub(super) frame: Option<Frame>,
    pub(super) graphics: Option<Graphics>,
    pub(super) status: String,
    pub(super) pending: bool,
    pub(super) painted: bool,
    pub(super) due: Option<Instant>,
    pub(super) marked: bool,
    pub(super) sessions: HashMap<PathBuf, Edits>,
    pub(super) zoom: f32,
    pub(super) pan: (f32, f32),
    pub(super) drag: Option<(f32, f32)>,
    pub(super) crop: bool,
    pub(super) selection: Option<(f32, f32, f32, f32)>,
    pub(super) image_rect: D2D_RECT_F,
    pub(super) exporting: bool,
    pub(super) displayed: Option<(PathBuf, u32)>,
    pub(super) render_failed: bool,
    pub(super) markup: Option<AnnotationKind>,
    pub(super) ink: Vec<[f32; 2]>,
    pub(super) markup_text: String,
    pub(super) signature: Option<Vec<[f32; 2]>>,
    pub(super) cancel: Option<Arc<AtomicBool>>,
    pub(super) slideshow: Option<Instant>,
    pub(super) tabs: Vec<PathBuf>,
    pub(super) password_attempts: HashMap<PathBuf, u32>,
    pub(super) views: HashMap<PathBuf, (u32, f32, (f32, f32))>,
}
thread_local! { pub(super) static STATE: RefCell<Option<State>> = const { RefCell::new(None) }; }
thread_local! { pub(super) static CONTROL_STATE: RefCell<Option<(bool,bool,bool,bool,bool)>> = const { RefCell::new(None) }; }

pub(super) unsafe fn add_tabs(hwnd: HWND, paths: &[PathBuf]) {
    let Ok(tabs) = GetDlgItem(Some(hwnd), TABS as i32) else {
        return;
    };
    STATE.with(|cell| {
        if let Some(state) = cell.borrow_mut().as_mut() {
            for path in paths {
                let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
                if state.tabs.contains(&path) {
                    continue;
                }
                let mut text = wide(&path.file_name().unwrap_or_default().to_string_lossy());
                let item = TCITEMW {
                    mask: TCIF_TEXT,
                    pszText: PWSTR(text.as_mut_ptr()),
                    ..Default::default()
                };
                let index = state.tabs.len();
                SendMessageW(
                    tabs,
                    TCM_INSERTITEMW,
                    Some(WPARAM(index)),
                    Some(LPARAM((&item as *const TCITEMW) as isize)),
                );
                state.tabs.push(path);
            }
        }
    });
}

pub(super) unsafe fn schedule(hwnd: HWND, state: &mut State, delta: i32) {
    let Some(path) = state.path.clone() else {
        return;
    };
    let mut rect = RECT::default();
    if GetClientRect(hwnd, &mut rect).is_err() {
        return;
    }
    if rect.right <= 0 || rect.bottom <= 100 {
        return;
    }
    state.generation = state.generation.wrapping_add(1);
    state.pending = true;
    state.render_failed = false;
    state.status = "Opening...".into();
    let request = Request {
        generation: state.generation,
        path,
        page: state.page,
        delta,
        width: ((rect.right as f32 * state.zoom) as u32).clamp(1, 4096),
        height: (((rect.bottom - 128) as f32 * state.zoom) as u32).clamp(1, 4096),
        sessions: state.sessions.clone(),
    };
    if state.sender.send(Job::Render(request)).is_err() {
        state.pending = false;
        state.status = "The rendering worker stopped. Close Preview and reopen the file.".into();
    }
    let _ = InvalidateRect(Some(hwnd), None, false);
}

pub(super) unsafe fn open(hwnd: HWND, path: PathBuf) {
    STATE.with(|cell| {
        if let Ok(mut value) = cell.try_borrow_mut() {
            if let Some(state) = value.as_mut() {
                if let Some((path, page)) = &state.displayed {
                    if !state.pending && !state.render_failed {
                        state
                            .views
                            .insert(path.clone(), (*page, state.zoom, state.pan));
                    }
                }
                let path = std::fs::canonicalize(&path).unwrap_or(path);
                let view = state
                    .views
                    .get(&path)
                    .copied()
                    .unwrap_or((0, 1.0, (0.0, 0.0)));
                state.path = Some(path);
                state.page = view.0;
                state.due = None;
                state.zoom = view.1;
                state.pan = view.2;
                state.crop = false;
                state.markup = None;
                state.selection = None;
                schedule(hwnd, state, 0);
            }
        }
    });
}

pub(super) unsafe fn navigate(hwnd: HWND, delta: i32) {
    STATE.with(|cell| {
        if let Ok(mut value) = cell.try_borrow_mut() {
            if let Some(state) = value.as_mut() {
                if state.pending {
                    return;
                }
                if state.path.as_ref().is_some_and(|p| is_pdf(p)) {
                    let count = state.frame.as_ref().map_or(0, |f| f.page_count);
                    let page = state.page as i64 + delta as i64;
                    if page < 0 || page >= count as i64 {
                        return;
                    }
                    state.page = page as u32;
                    schedule(hwnd, state, 0);
                } else {
                    schedule(hwnd, state, delta);
                }
            }
        }
    });
}

pub(super) unsafe fn tick(hwnd: HWND) {
    let mut fields_to_show = None;
    let mut output_to_open = None;
    let mut advance = false;
    let mut password_to_show = None;
    STATE.with(|cell| {
        let Ok(mut value) = cell.try_borrow_mut() else {
            return;
        };
        let Some(state) = value.as_mut() else {
            return;
        };
        while let Ok(event) = state.receiver.try_recv() {
            let completed = match event {
                Event::Render(completed) => completed,
                Event::Saved(path, edits, whole_document, result) => {
                    state.exporting = false;
                    state.status = match result {
                        Ok(()) => {
                            if let Some(current) = state.sessions.get_mut(&path) {
                                if whole_document
                                    && current.image == edits.image
                                    && current.pdf == edits.pdf
                                {
                                    current.dirty = false;
                                }
                            }
                            "Saved a new copy. The original file is unchanged.".into()
                        }
                        Err(error) => error,
                    };
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    continue;
                }
                Event::Text(generation, path, result) => {
                    if generation != state.generation
                        || state.path.as_ref() != Some(&path)
                        || state.pending
                    {
                        continue;
                    }
                    state.status = match result {
                        Ok(text) if text.is_empty() => "No text found.".into(),
                        Ok(text) => match clipboard(hwnd, &text) {
                            Ok(()) => {
                                "Text copied. Check recognition and reading order before pasting."
                                    .into()
                            }
                            Err(error) => error,
                        },
                        Err(error) => error,
                    };
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    continue;
                }
                Event::Found(generation, result) => {
                    if generation == state.generation {
                        match result {
                            Ok(Some(page)) => {
                                state.page = page;
                                schedule(hwnd, state, 0);
                            }
                            Ok(None) => state.status = "No matching text found.".into(),
                            Err(error) => state.status = error,
                        };
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                    continue;
                }
                Event::Fields(generation, result) => {
                    if generation == state.generation {
                        match result {
                            Ok(fields) => fields_to_show = Some((generation, fields)),
                            Err(error) => state.status = error,
                        }
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                    continue;
                }
                Event::Background(output, result) => {
                    state.exporting = false;
                    match result {
                        Ok(detail) => {
                            state.status = detail;
                            output_to_open = Some(output);
                        }
                        Err(error) => state.status = error,
                    }
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    continue;
                }
                Event::Finished(result) => {
                    state.exporting = false;
                    state.cancel = None;
                    state.status = match result {
                        Ok(detail) => detail,
                        Err(error) => error,
                    };
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    continue;
                }
                Event::Progress(text) => {
                    state.status = text;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    continue;
                }
            };
            if completed.generation != state.generation {
                continue;
            }
            state.pending = false;
            match completed.result {
                Ok(frame) => {
                    if crate::model::frame_bytes(frame.width, frame.height).ok()
                        != Some(frame.pixels.len())
                    {
                        state.status = "The decoder returned an invalid image.".into();
                        continue;
                    }
                    let previous_dirty = state
                        .displayed
                        .as_ref()
                        .and_then(|(path, _)| state.sessions.get(path))
                        .is_some_and(|edits| edits.dirty);
                    if completed.navigation {
                        if let Some((path, page)) = &state.displayed {
                            if path != &completed.path {
                                state
                                    .views
                                    .insert(path.clone(), (*page, state.zoom, state.pan));
                            }
                        }
                    }
                    state.password_attempts.remove(&completed.path);
                    state.displayed = Some((completed.path.clone(), completed.page));
                    state.path = Some(completed.path);
                    state.page = completed.page;
                    state.render_failed = false;
                    if let (Some(path), Ok(tabs)) =
                        (&state.path, GetDlgItem(Some(hwnd), TABS as i32))
                    {
                        if completed.navigation && !previous_dirty && !state.tabs.contains(path) {
                            let selected = SendMessageW(tabs, TCM_GETCURSEL, None, None).0;
                            if let Some(tab) = state.tabs.get_mut(selected as usize) {
                                *tab = path.clone();
                                let mut text =
                                    wide(&path.file_name().unwrap_or_default().to_string_lossy());
                                let item = TCITEMW {
                                    mask: TCIF_TEXT,
                                    pszText: PWSTR(text.as_mut_ptr()),
                                    ..Default::default()
                                };
                                SendMessageW(
                                    tabs,
                                    TCM_SETITEMW,
                                    Some(WPARAM(selected as usize)),
                                    Some(LPARAM((&item as *const TCITEMW) as isize)),
                                );
                            }
                        }
                        let index = match state.tabs.iter().position(|p| p == path) {
                            Some(index) => index,
                            None => {
                                let mut text =
                                    wide(&path.file_name().unwrap_or_default().to_string_lossy());
                                let item = TCITEMW {
                                    mask: TCIF_TEXT,
                                    pszText: PWSTR(text.as_mut_ptr()),
                                    ..Default::default()
                                };
                                let index = state.tabs.len();
                                SendMessageW(
                                    tabs,
                                    TCM_INSERTITEMW,
                                    Some(WPARAM(index)),
                                    Some(LPARAM((&item as *const TCITEMW) as isize)),
                                );
                                state.tabs.push(path.clone());
                                index
                            }
                        };
                        SendMessageW(tabs, TCM_SETCURSEL, Some(WPARAM(index)), None);
                    }
                    let name = state
                        .path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    state.status = if state.path.as_ref().is_some_and(|p| is_pdf(p)) {
                        format!(
                            "{name}   ·   Page {} of {}",
                            state.page + 1,
                            frame.page_count
                        )
                    } else {
                        format!(
                            "{name}   ·   {} × {}",
                            frame.source_width, frame.source_height
                        )
                    };
                    let title = wide(&format!("{name} - Preview for Windows"));
                    let _ = SetWindowTextW(hwnd, PCWSTR(title.as_ptr()));
                    state.frame = Some(frame);
                    if let Some(graphics) = state.graphics.as_mut() {
                        graphics.bitmap = None;
                    }
                }
                Err(error) => {
                    state.render_failed = error != "No more images in this direction.";
                    state.status = format!("{error} Previous view retained.");
                    state.slideshow = None;
                    if error == crate::pdf::PASSWORD_REQUIRED {
                        password_to_show =
                            Some((completed.generation, completed.path, completed.page));
                        state.due = None;
                    }
                    if let Some((path, page)) = &state.displayed {
                        if state.path.as_ref() != Some(path) {
                            if let Some((_, zoom, pan)) = state.views.get(path) {
                                state.zoom = *zoom;
                                state.pan = *pan;
                            }
                        }
                        state.path = Some(path.clone());
                        state.page = *page;
                    }
                }
            }
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        if state.painted && state.due.is_some_and(|due| Instant::now() >= due) {
            state.due = None;
            schedule(hwnd, state, 0);
        }
        if !state.pending && state.slideshow.is_some_and(|due| Instant::now() >= due) {
            if state.path.as_ref().is_some_and(|p| is_pdf(p))
                && state
                    .frame
                    .as_ref()
                    .is_some_and(|f| state.page + 1 >= f.page_count)
            {
                state.slideshow = None;
                state.status = "End of slideshow.".into();
            } else {
                state.slideshow = Some(Instant::now() + Duration::from_secs(3));
                advance = true;
            }
        }
    });
    if let Some((generation, fields)) = fields_to_show {
        fill_field(hwnd, generation, fields);
    }
    if let Some(output) = output_to_open {
        open(hwnd, output);
    }
    if let Some((generation, path, page)) = password_to_show {
        password_prompt(hwnd, generation, path, page);
    }
    if advance {
        navigate(hwnd, 1);
    }
    update_controls(hwnd);
}

pub(super) unsafe fn password_prompt(hwnd: HWND, generation: u64, path: PathBuf, page: u32) {
    let retry = STATE.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_some_and(|s| s.password_attempts.get(&path).copied().unwrap_or(0) > 0)
    });
    let answer = input_with_password(
        hwnd,
        if retry {
            "That password did not unlock this PDF"
        } else {
            "Open password-protected PDF"
        },
        &[("Password", String::new())],
        true,
    );
    let Some(mut values) = answer else {
        STATE.with(|cell| {
            if let Some(state) = cell.borrow_mut().as_mut() {
                state.password_attempts.remove(&path);
                state.render_failed = state.frame.is_none();
                state.status = "PDF opening cancelled. The previous document is unchanged.".into();
            }
        });
        let _ = InvalidateRect(Some(hwnd), None, false);
        return;
    };
    if values.is_empty() {
        return;
    }
    let password = values.remove(0);
    let mut rect = RECT::default();
    if GetClientRect(hwnd, &mut rect).is_err() {
        return;
    }
    STATE.with(|cell| {
        if let Some(state) = cell.borrow_mut().as_mut() {
            if state.generation != generation {
                return;
            }
            *state.password_attempts.entry(path.clone()).or_default() += 1;
            state.generation = state.generation.wrapping_add(1);
            state.pending = true;
            state.render_failed = false;
            state.path = Some(path.clone());
            state.page = page;
            state.status = "Unlocking PDF...".into();
            let request = Request {
                generation: state.generation,
                path,
                page,
                delta: 0,
                width: (rect.right.max(1) as u32).min(4096),
                height: ((rect.bottom - 128).max(1) as u32).min(4096),
                sessions: state.sessions.clone(),
            };
            if state.sender.send(Job::Password(request, password)).is_err() {
                state.pending = false;
                state.status = "The PDF worker stopped.".into();
            }
        }
    });
    let _ = InvalidateRect(Some(hwnd), None, false);
}
