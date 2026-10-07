//! The sidebar list: page thumbnails, the table of contents, and notes.
//! `widgets::sidebar_rows` places the rows; this module handles clicks,
//! keys, the wheel, and keeping the current page's thumbnail in view.
use super::{
    app::State,
    document,
    theme::size,
    widgets::{self, WidgetId, SHEET_TAB},
};
use windows::Win32::UI::Input::KeyboardAndMouse::*;

fn tab(state: &State) -> usize {
    state.sidebar_tab.min(SHEET_TAB)
}

/// Shows sidebar view `index`, keeping the current page in view.
pub(super) fn show(state: &mut State, index: usize) {
    state.sidebar_tab = index.min(SHEET_TAB);
    if state.sidebar_tab == SHEET_TAB {
        // The sheet hides the page view, so focus moves onto the sheet.
        state.focus = Some(WidgetId::SidebarItem(state.page as usize));
    }
    follow_page(state);
}

/// Click, Enter, or UI Automation Invoke on a row: go to its page. A page
/// picked on the contact sheet opens in the thumbnail view.
pub(super) fn activate(state: &mut State, index: usize) {
    let tab = tab(state);
    state.sidebar_rows[tab] = index;
    state.focus = Some(WidgetId::SidebarItem(index));
    let page = state.pdf.as_ref().and_then(|v| match tab {
        0 | SHEET_TAB => Some(index as u32),
        1 => v.outline.as_ref()?.as_ref().ok()?.get(index)?.page,
        _ => v.notes.get(index).map(|n| n.page),
    });
    if let Some(page) = page {
        document::go_to_page(state, page, true);
    }
    if tab == SHEET_TAB {
        show(state, 0);
        state.focus = Some(WidgetId::Document);
    }
}

/// Arrow keys, Page Up and Down, Home, and End move through the list; on
/// the contact sheet, Left and Right move by one and Up and Down by a row.
/// The document follows thumbnails at once, as in Preview.
pub(super) fn key(state: &mut State, vk: u16, index: usize) -> bool {
    let layout = state.layout();
    let count = state.sidebar_list().len();
    if count == 0 {
        return false;
    }
    let tab = tab(state);
    let row = if tab == SHEET_TAB { widgets::sheet_grid(layout.sidebar_panel.width(), state.scale, state.text_scale).0 } else { 1 };
    let page = layout.widgets.iter().filter(|w| w.role == widgets::Role::ListItem).count().saturating_sub(row).max(row);
    let next = match VIRTUAL_KEY(vk) {
        VK_LEFT if tab == SHEET_TAB => index.saturating_sub(1),
        VK_RIGHT if tab == SHEET_TAB => index + 1,
        VK_UP => index.saturating_sub(row),
        VK_DOWN => index + row,
        VK_PRIOR => index.saturating_sub(page),
        VK_NEXT => index + page,
        VK_HOME => 0,
        VK_END => count - 1,
        _ => return false,
    }
    .min(count - 1);
    state.sidebar_rows[tab] = next;
    state.focus = Some(WidgetId::SidebarItem(next));
    state.focus_visible = true;
    reveal(state, next);
    if matches!(tab, 0 | SHEET_TAB) {
        document::go_to_page(state, next as u32, true);
    }
    true
}

/// Dragging the sidebar's edge sets its width.
pub(super) fn resize(state: &mut State, x: f32) {
    state.sidebar_width = (x / state.scale).clamp(size::SIDEBAR_MIN, size::SIDEBAR_MAX);
}

pub(super) fn wheel(state: &mut State, delta: f32, layout: &widgets::Layout) {
    let max = (layout.sidebar_content - layout.sidebar_panel.height()).max(0.0);
    let tab = tab(state);
    let scroll = state.sidebar_scroll[tab] - delta / 120.0 * 100.0 * state.scale;
    state.sidebar_scroll[tab] = scroll.clamp(0.0, max);
}

/// Keeps the current page's thumbnail in view while the document scrolls.
pub(super) fn follow_page(state: &mut State) {
    if state.quick.is_some() {
        super::quickview::follow_rail(state);
    } else if state.sidebar_open && matches!(state.sidebar_tab, 0 | SHEET_TAB) && state.pdf.is_some() {
        reveal(state, state.page as usize);
    }
}

/// Scrolls the list just enough to show row `index` whole. Scrolling down
/// stops where a row starts, so the top row is never cut to its blank lower half.
fn reveal(state: &mut State, index: usize) {
    let panel = state.layout().sidebar_panel;
    let (s, ts) = (state.scale, state.text_scale);
    let list = state.sidebar_list();
    let (top, height) = widgets::sidebar_row(&list, index, panel.width(), s, ts);
    let current = state.sidebar_scroll[tab(state)];
    let scroll = if top < current {
        top
    } else if top + height > current + panel.height() {
        widgets::row_start_after(&list, top + height - panel.height(), panel.width(), s, ts).min(top)
    } else {
        current
    };
    let tab = tab(state);
    state.sidebar_scroll[tab] = scroll;
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
