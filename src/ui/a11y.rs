//! UI Automation through AccessKit (CLAUDE.md D11). Every widget becomes a
//! node with a role, a name, and actions; keyboard focus is reported.
//! The adapter starts after first content: creating it loads UI Automation
//! (accesskit_windows::Adapter::new calls UiaLookupId).
use super::{
    app::{file_name, with_state, State},
    commands::{self, Command},
    widgets::{Role as WidgetRole, Widget, WidgetId},
};
use accesskit::{Action, ActionHandler, ActionRequest, ActivationHandler, Live, Node, NodeId, Rect, Role, TreeId, TreeInfo, TreeUpdate};
use std::cell::RefCell;
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::{Input::KeyboardAndMouse::GetFocus, WindowsAndMessaging::{PostMessageW, WM_APP}},
};

/// Posted by the action handler: wParam 0 = click, 1 = focus; lParam = node.
pub(super) const WM_APP_A11Y: u32 = WM_APP + 2;

thread_local! {
    static ADAPTER: RefCell<Option<accesskit_windows::Adapter>> = const { RefCell::new(None) };
}

const ROOT: NodeId = NodeId(1);
const TITLE_BAR: NodeId = NodeId(2);
const TAB_LIST: NodeId = NodeId(3);
const TOOLBAR: NodeId = NodeId(4);
const MARKUP_BAR: NodeId = NodeId(5);
const SIDEBAR: NodeId = NodeId(6);
const SIDEBAR_TABS: NodeId = NodeId(7);
const SIDEBAR_PANEL: NodeId = NodeId(8);
const STATUS: NodeId = NodeId(9);
const SHEET: NodeId = NodeId(10);
const SHEET_MESSAGE: NodeId = NodeId(11);
const EMPTY: NodeId = NodeId(12);
const EMPTY_HEADING: NodeId = NodeId(13);
const RECENT: NodeId = NodeId(14);
const FILE_TITLE: NodeId = NodeId(15);

pub(super) fn node_id(id: WidgetId) -> NodeId {
    NodeId(match id {
        WidgetId::Tab(i) => 10_000 + i as u64,
        WidgetId::TabClose(i) => 20_000 + i as u64,
        WidgetId::Command(Command::Tab(n)) => 31_000 + n as u64,
        WidgetId::Command(c) => 30_000 + commands::ALL.iter().position(|x| *x == c).unwrap_or(999) as u64,
        WidgetId::NewTab => 40_000,
        WidgetId::Minimize => 40_001,
        WidgetId::Maximize => 40_002,
        WidgetId::Close => 40_003,
        WidgetId::SidebarTab(i) => 50_000 + i as u64,
        WidgetId::SheetButton(i) => 60_000 + i as u64,
        WidgetId::SheetField(i) => 61_000 + i as u64,
        WidgetId::Document => 70_000,
    })
}

fn bounds(r: super::widgets::Rect) -> Rect {
    Rect { x0: r.x0 as f64, y0: r.y0 as f64, x1: r.x1 as f64, y1: r.y1 as f64 }
}

fn widget_node(w: &Widget) -> Node {
    let mut node = Node::new(match w.role {
        WidgetRole::Tab | WidgetRole::SidebarTab => Role::Tab,
        WidgetRole::Document => Role::Document,
        WidgetRole::Field => Role::TextInput,
        WidgetRole::Button | WidgetRole::Caption => Role::Button,
    });
    node.set_label(w.label.clone());
    node.set_bounds(bounds(w.rect));
    if !w.tooltip.is_empty() && w.tooltip != w.label {
        node.set_description(w.tooltip.clone());
    }
    if let Some(key) = w.access_key {
        node.set_access_key(key.to_string());
    }
    if let WidgetId::Command(c) = w.id {
        if let Some(keys) = commands::shortcut_text(c) {
            node.set_keyboard_shortcut(keys);
        }
    }
    match (w.role, w.checked) {
        (WidgetRole::Tab | WidgetRole::SidebarTab, Some(selected)) => node.set_selected(selected),
        (_, Some(on)) => node.set_toggled(on.into()),
        _ => {}
    }
    if !w.enabled {
        node.set_disabled();
    } else {
        if w.role != WidgetRole::Document {
            node.add_action(Action::Click);
        }
        node.add_action(Action::Focus);
    }
    node
}

fn group(role: Role, label: &str, r: super::widgets::Rect, children: Vec<NodeId>) -> Node {
    let mut node = Node::new(role);
    node.set_label(label);
    node.set_bounds(bounds(r));
    node.set_children(children);
    node
}

fn text(role: Role, label: &str, r: super::widgets::Rect) -> Node {
    let mut node = Node::new(role);
    node.set_label(label);
    node.set_bounds(bounds(r));
    node
}

/// Builds the full tree from the current layout.
pub(super) fn tree(s: &State) -> TreeUpdate {
    let layout = s.layout();
    let mut nodes: Vec<(NodeId, Node)> = Vec::new();
    let ids = |pred: &dyn Fn(&Widget) -> bool| -> Vec<NodeId> {
        layout.widgets.iter().filter(|w| pred(w)).map(|w| node_id(w.id)).collect()
    };
    for w in &layout.widgets {
        if w.role != WidgetRole::Field {
            nodes.push((node_id(w.id), widget_node(w)));
        }
    }
    use super::widgets::Region;
    let tabs = ids(&|w| w.role == WidgetRole::Tab);
    nodes.push((TAB_LIST, group(Role::TabList, "Open files", layout.title_bar, tabs)));
    let mut title_children = vec![TAB_LIST];
    title_children.extend(ids(&|w| w.region == Region::TitleBar && w.role != WidgetRole::Tab));
    nodes.push((TITLE_BAR, group(Role::TitleBar, "Title bar", layout.title_bar, title_children)));
    let mut root_children = vec![TITLE_BAR, TOOLBAR];
    let mut toolbar = ids(&|w| w.region == Region::Toolbar);
    if let Some(path) = &s.path {
        nodes.push((FILE_TITLE, text(Role::Label, &file_name(path), layout.title_text)));
        toolbar.insert(1, FILE_TITLE);
    }
    nodes.push((TOOLBAR, group(Role::Toolbar, "Toolbar", layout.toolbar, toolbar)));
    if let Some(bar) = layout.markup_bar {
        nodes.push((MARKUP_BAR, group(Role::Toolbar, "Markup tools", bar, ids(&|w| w.region == Region::MarkupBar))));
        root_children.push(MARKUP_BAR);
    }
    if let Some(side) = layout.sidebar {
        nodes.push((SIDEBAR_TABS, group(Role::TabList, "Sidebar views", side, ids(&|w| w.role == WidgetRole::SidebarTab))));
        let panel = super::widgets::SIDEBAR_TABS[s.sidebar_tab.min(2)];
        nodes.push((SIDEBAR_PANEL, text(Role::TabPanel, panel, layout.sidebar_panel)));
        nodes.push((SIDEBAR, group(Role::Pane, "Sidebar", side, vec![SIDEBAR_TABS, SIDEBAR_PANEL])));
        root_children.push(SIDEBAR);
    }
    if let Some(empty) = &layout.empty {
        nodes.push((EMPTY_HEADING, text(Role::Heading, "Open a PDF or image", empty.heading)));
        nodes.push((RECENT, text(Role::Label, "Recent files. Files you open will show here.", empty.recent)));
        let mut children = vec![EMPTY_HEADING];
        children.extend(ids(&|w| w.region == Region::Document));
        children.push(RECENT);
        nodes.push((EMPTY, group(Role::Group, "Start", layout.document, children)));
        root_children.push(EMPTY);
    } else {
        root_children.push(node_id(WidgetId::Document));
    }
    let mut status = text(Role::Status, &s.status, layout.status);
    status.set_live(Live::Polite);
    nodes.push((STATUS, status));
    root_children.push(STATUS);
    if let (Some(sheet), Some(card)) = (&s.sheet, &layout.sheet) {
        let mut children = Vec::new();
        if !sheet.message.is_empty() {
            nodes.push((SHEET_MESSAGE, text(Role::Label, &sheet.message, card.message)));
            children.push(SHEET_MESSAGE);
        }
        children.extend(ids(&|w| w.region == Region::Sheet && w.role != WidgetRole::Field));
        let mut dialog = group(Role::Dialog, &sheet.title, card.card, children);
        dialog.set_modal();
        nodes.push((SHEET, dialog));
        root_children.push(SHEET);
    }
    let title = match &s.path {
        Some(path) => format!("{} - Preview for Windows", file_name(path)),
        None => "Preview for Windows".into(),
    };
    nodes.push((ROOT, group(Role::Window, &title, super::widgets::Rect::new(0.0, 0.0, s.size.0, s.size.1), root_children)));
    let focus = s
        .focus
        .filter(|f| !matches!(f, WidgetId::SheetField(_)))
        .filter(|f| layout.widgets.iter().any(|w| w.id == *f))
        .map_or(ROOT, node_id);
    TreeUpdate { nodes, tree: Some(TreeInfo::new(ROOT)), tree_id: TreeId::ROOT, focus }
}

struct Initial;
impl ActivationHandler for Initial {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        // A None here makes AccessKit wait for the next `update`.
        with_state(|s| tree(s))
    }
}

/// Runs on a UI Automation thread, so it only posts to the window.
struct Actions(isize);
impl ActionHandler for Actions {
    fn do_action(&mut self, request: ActionRequest) {
        let kind = match request.action {
            Action::Click => 0,
            Action::Focus => 1,
            _ => return,
        };
        unsafe {
            let _ = PostMessageW(Some(HWND(self.0 as *mut _)), WM_APP_A11Y, WPARAM(kind), LPARAM(request.target_node.0 as isize));
        }
    }
}

pub(super) unsafe fn start(hwnd: HWND) {
    if ADAPTER.with(|a| a.borrow().is_some()) {
        return;
    }
    let adapter = accesskit_windows::Adapter::new(hwnd, GetFocus() == hwnd, Actions(hwnd.0 as isize));
    ADAPTER.with(|a| *a.borrow_mut() = Some(adapter));
}

pub(super) unsafe fn get_object(wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    let result = ADAPTER.with(|a| {
        let mut adapter = a.try_borrow_mut().ok()?;
        adapter.as_mut()?.handle_wm_getobject(wparam, lparam, &mut Initial)
    })?;
    // Converting calls UI Automation, which may send WM_GETOBJECT again, so
    // no borrow is held here.
    Some(result.into())
}

/// Pushes the current tree. Call after state changes, outside any borrow.
pub(super) fn update() {
    // The tree is built only when a UI Automation client is listening.
    let events = ADAPTER.with(|a| {
        let mut adapter = a.try_borrow_mut().ok()?;
        adapter.as_mut()?.update_if_active(|| {
            with_state(|s| tree(s)).unwrap_or_else(|| TreeUpdate {
                nodes: vec![(ROOT, Node::new(Role::Window))],
                tree: Some(TreeInfo::new(ROOT)),
                tree_id: TreeId::ROOT,
                focus: ROOT,
            })
        })
    });
    if let Some(events) = events {
        events.raise();
    }
}

pub(super) fn focus_changed(focused: bool) {
    let events = ADAPTER.with(|a| a.try_borrow_mut().ok()?.as_mut()?.update_window_focus_state(focused));
    if let Some(events) = events {
        events.raise();
    }
}

pub(super) fn widget_for(node: u64) -> Option<WidgetId> {
    with_state(|s| s.layout().widgets.iter().map(|w| w.id).find(|id| node_id(*id).0 == node)).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_ids_are_unique_across_widget_kinds() {
        let mut ids: Vec<WidgetId> =
            commands::ALL.iter().filter(|c| !matches!(c, Command::Tab(_))).map(|c| WidgetId::Command(*c)).collect();
        ids.extend((0..9).map(|n| WidgetId::Command(Command::Tab(n))));
        ids.extend((0..50).flat_map(|i| [WidgetId::Tab(i), WidgetId::TabClose(i), WidgetId::SheetButton(i), WidgetId::SheetField(i)]));
        ids.extend((0..3).map(WidgetId::SidebarTab));
        ids.extend([WidgetId::NewTab, WidgetId::Minimize, WidgetId::Maximize, WidgetId::Close, WidgetId::Document]);
        let mut nodes: Vec<u64> = ids.iter().map(|i| node_id(*i).0).collect();
        nodes.sort_unstable();
        let before = nodes.len();
        nodes.dedup();
        assert_eq!(nodes.len(), before);
        assert!(nodes.iter().all(|n| *n > 15), "widget ids stay clear of the fixed group ids");
    }

    #[test]
    fn every_interactive_widget_node_has_a_name_role_and_action() {
        use super::super::widgets::{layout, Input, SheetView};
        let tabs = vec!["report.pdf".to_string(), "photo.png".to_string()];
        let mut input = Input {
            width: 1100.0,
            height: 800.0,
            scale: 1.0,
            text_scale: 1.0,
            tabs: &tabs,
            active_tab: Some(0),
            maximized: false,
            has_document: true,
            title: "report.pdf, page 1 of 20",
            sidebar_open: true,
            sidebar_tab: 0,
            markup: 1.0,
            ctx: commands::Ctx { has_frame: true, pdf: true, tabs: 2, ..Default::default() },
            sheet: Some(SheetView { message_height: 0.0, fields: vec!["Text to find"], buttons: vec!["OK", "Cancel"] }),
        };
        for with_document in [true, false] {
            input.has_document = with_document;
            for w in layout(&input).widgets.iter().filter(|w| w.role != WidgetRole::Field) {
                let node = widget_node(w);
                assert!(node.label().is_some_and(|l| !l.is_empty()), "{:?} has no name", w.id);
                assert_ne!(node.role(), Role::Unknown);
                if w.enabled {
                    assert!(node.supports_action(Action::Focus), "{:?}", w.id);
                }
            }
        }
    }
}
