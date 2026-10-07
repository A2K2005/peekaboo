//! The sidebar list: page thumbnails, the table of contents, and notes.
//! `widgets::sidebar_rows` places the rows; this module handles clicks,
//! keys, the wheel, and keeping the current page's thumbnail in view.
use super::{
    app::State,
    document,
    widgets::{self, WidgetId},
};
use windows::Win32::UI::Input::KeyboardAndMouse::*;

fn tab(state: &State) -> usize {
    state.sidebar_tab.min(2)
}

/// Click, Enter, or UI Automation Invoke on a row: go to its page.
pub(super) fn activate(state: &mut State, index: usize) {
    let tab = tab(state);
    state.sidebar_rows[tab] = index;
    state.focus = Some(WidgetId::SidebarItem(index));
    let page = state.pdf.as_ref().and_then(|v| match tab {
        0 => Some(index as u32),
        1 => v.outline.as_ref()?.as_ref().ok()?.get(index)?.page,
        _ => v.notes.get(index).map(|n| n.page),
    });
    if let Some(page) = page {
        document::go_to_page(state, page, true);
    }
}

/// Arrow keys, Page Up and Down, Home, and End move through the list.
/// Thumbnails follow at once, as in Preview.
pub(super) fn key(state: &mut State, vk: u16, index: usize) -> bool {
    let layout = state.layout();
    let count = state.sidebar_list().len();
    if count == 0 {
        return false;
    }
    let page = layout.widgets.iter().filter(|w| w.role == widgets::Role::ListItem).count().saturating_sub(1).max(1);
    let next = match VIRTUAL_KEY(vk) {
        VK_UP => index.saturating_sub(1),
        VK_DOWN => index + 1,
        VK_PRIOR => index.saturating_sub(page),
        VK_NEXT => index + page,
        VK_HOME => 0,
        VK_END => count - 1,
        _ => return false,
    }
    .min(count - 1);
    let tab = tab(state);
    state.sidebar_rows[tab] = next;
    state.focus = Some(WidgetId::SidebarItem(next));
    state.focus_visible = true;
    reveal(state, next);
    if tab == 0 {
        document::go_to_page(state, next as u32, true);
    }
    true
}

pub(super) fn wheel(state: &mut State, delta: f32, layout: &widgets::Layout) {
    let max = (layout.sidebar_content - layout.sidebar_panel.height()).max(0.0);
    let tab = tab(state);
    let scroll = state.sidebar_scroll[tab] - delta / 120.0 * 100.0 * state.scale;
    state.sidebar_scroll[tab] = scroll.clamp(0.0, max);
}

/// Keeps the current page's thumbnail in view while the document scrolls.
pub(super) fn follow_page(state: &mut State) {
    if state.sidebar_open && state.sidebar_tab == 0 && state.pdf.is_some() {
        reveal(state, state.page as usize);
    }
}

/// Scrolls the list just enough to show row `index` whole.
fn reveal(state: &mut State, index: usize) {
    let panel = state.layout().sidebar_panel.height();
    let (top, height) = widgets::sidebar_row(&state.sidebar_list(), index, state.scale, state.text_scale);
    let tab = tab(state);
    let scroll = &mut state.sidebar_scroll[tab];
    if top < *scroll {
        *scroll = top;
    } else if top + height > *scroll + panel {
        *scroll = top + height - panel;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{document::PdfView, theme, worker::Workers};
    use std::{path::PathBuf, sync::Arc};
    use windows::Win32::Foundation::HWND;

    fn state(pages: usize) -> State {
        let workers = Workers::start(HWND::default()).unwrap();
        let path = PathBuf::from("a.pdf");
        let mut s = State::new(workers, &[path.clone()], (1100.0, 800.0), 1.0, 1.0, theme::palette(theme::Mode::Light));
        s.animations = false;
        s.sidebar_open = true;
        s.pdf = Some(PdfView::new(path, vec![[612.0, 792.0]; pages], Arc::default(), 0));
        s
    }

    #[test]
    fn keys_move_through_thumbnails_and_keep_the_row_in_view() {
        let mut s = state(500);
        assert!(key(&mut s, VK_DOWN.0, 0));
        assert_eq!((s.page, s.focus), (1, Some(WidgetId::SidebarItem(1))), "the document follows the thumbnail");
        assert!(key(&mut s, VK_END.0, 1));
        assert_eq!(s.page, 499);
        let layout = s.layout();
        let row = layout.widgets.iter().find(|w| w.id == WidgetId::SidebarItem(499)).expect("the last row is in view");
        assert!(row.rect.y1 <= layout.sidebar_panel.y1 + 0.5);
        assert!(key(&mut s, VK_HOME.0, 499));
        assert_eq!((s.page, s.sidebar_scroll[0]), (0, 0.0));
        assert!(!key(&mut s, VK_LEFT.0, 0), "other keys go to the window");
    }

    #[test]
    fn wheel_scrolls_the_list_within_its_length() {
        let mut s = state(3);
        let layout = s.layout();
        wheel(&mut s, -1200.0, &layout);
        assert_eq!(s.sidebar_scroll[0], (layout.sidebar_content - layout.sidebar_panel.height()).max(0.0));
        wheel(&mut s, 240.0, &layout);
        assert_eq!(s.sidebar_scroll[0], 0.0);
    }

    #[test]
    fn clicking_an_outline_entry_or_a_note_goes_to_its_page() {
        let mut s = state(20);
        s.view_mode = crate::ui::view::ViewMode::Single;
        s.sidebar_tab = 1;
        let v = s.pdf.as_mut().unwrap();
        v.outline = Some(Ok(vec![crate::model::OutlineItem { title: "Intro".into(), page: Some(7), level: 0 }]));
        v.notes = vec![crate::ui::worker::Note { page: 12, kind: "Text".into(), text: "Check".into() }];
        activate(&mut s, 0);
        assert_eq!(s.page, 7);
        s.sidebar_tab = 2;
        activate(&mut s, 0);
        assert_eq!((s.page, s.sidebar_rows[2]), (12, 0));
    }
}
