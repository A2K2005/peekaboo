//! Retained widget list: layout, hit testing, Tab order, F6 panes, and
//! access keys. Pure code, so all of it runs in unit tests.
use super::{
    commands::{self, Command, Ctx, enabled, info},
    worker::Note,
};
use crate::model::OutlineItem;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Rect {
    pub(super) x0: f32,
    pub(super) y0: f32,
    pub(super) x1: f32,
    pub(super) y1: f32,
}

impl Rect {
    pub(super) fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x0: x, y0: y, x1: x + width.max(0.0), y1: y + height.max(0.0) }
    }
    pub(super) fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }
    pub(super) fn width(&self) -> f32 {
        self.x1 - self.x0
    }
    pub(super) fn height(&self) -> f32 {
        self.y1 - self.y0
    }
    pub(super) fn inset(&self, d: f32) -> Self {
        Self { x0: self.x0 + d, y0: self.y0 + d, x1: (self.x1 - d).max(self.x0 + d), y1: (self.y1 - d).max(self.y0 + d) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum WidgetId {
    Tab(usize),
    TabClose(usize),
    Command(Command),
    NewTab,
    Minimize,
    Maximize,
    Close,
    SidebarTab(usize),
    SheetButton(usize),
    SheetField(usize),
    Document,
    /// A row of the open sidebar list: a page thumbnail, an outline entry,
    /// or a note.
    SidebarItem(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Role {
    Tab,
    Button,
    Caption,
    SidebarTab,
    Field,
    Document,
    ListItem,
}

/// F6 cycles these in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Region {
    TitleBar,
    Toolbar,
    MarkupBar,
    Sidebar,
    Document,
    Sheet,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Widget {
    pub(super) id: WidgetId,
    pub(super) role: Role,
    pub(super) region: Region,
    pub(super) rect: Rect,
    pub(super) label: String,
    pub(super) glyph: Option<u16>,
    pub(super) tooltip: String,
    pub(super) access_key: Option<char>,
    pub(super) enabled: bool,
    pub(super) checked: Option<bool>,
    pub(super) focusable: bool,
    /// Accent-filled primary button.
    pub(super) primary: bool,
}

/// Keytip scope. Alt shows the root keys; M enters the markup bar scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    Root,
    Markup,
}

pub(super) struct SheetView<'a> {
    /// Wrapped message height in epx at the current text scale.
    pub(super) message_height: f32,
    pub(super) fields: Vec<&'a str>,
    /// The first button is the primary (accent) button.
    pub(super) buttons: Vec<&'a str>,
}

/// What the sidebar panel lists. Only rows in view become widgets.
#[derive(Clone, Copy)]
pub(super) enum SidebarList<'a> {
    /// Page sizes in points.
    Thumbnails(&'a [[f32; 2]]),
    Contents(&'a [OutlineItem]),
    Notes(&'a [Note]),
    /// "No contents", "Looking for notes", and similar.
    Message(&'a str),
}

impl SidebarList<'_> {
    pub(super) fn len(&self) -> usize {
        match self {
            SidebarList::Thumbnails(s) => s.len(),
            SidebarList::Contents(s) => s.len(),
            SidebarList::Notes(s) => s.len(),
            SidebarList::Message(_) => 0,
        }
    }
    /// Accessible name and visible text of a row.
    pub(super) fn label(&self, index: usize) -> String {
        match self {
            SidebarList::Thumbnails(_) => format!("Thumbnail, page {}", index + 1),
            SidebarList::Contents(items) => items[index].title.clone(),
            SidebarList::Notes(notes) => {
                let n = &notes[index];
                format!("Page {}, {}: {}", n.page + 1, n.kind.to_lowercase(), n.text)
            }
            SidebarList::Message(_) => String::new(),
        }
    }
}

/// Thumbnail width in epx. Pages fit a box this wide and 1.4 times as tall.
pub(super) const THUMB_WIDTH: f32 = 120.0;

/// A page thumbnail's size in physical pixels.
pub(super) fn thumb_size(size: [f32; 2], scale: f32) -> (f32, f32) {
    let (w, h) = (size[0].max(1.0), size[1].max(1.0));
    let fit = (THUMB_WIDTH / w).min(1.4 * THUMB_WIDTH / h) * scale;
    ((w * fit).round().max(1.0), (h * fit).round().max(1.0))
}

fn list_row_height(list: &SidebarList, index: usize, s: f32, ts: f32) -> f32 {
    match list {
        SidebarList::Thumbnails(sizes) => thumb_size(sizes[index], s).1 + (12.0 + 20.0 * ts) * s,
        _ => control_height(ts) * s,
    }
}

/// Top and height of a list row in content pixels.
pub(super) fn sidebar_row(list: &SidebarList, index: usize, s: f32, ts: f32) -> (f32, f32) {
    match list {
        SidebarList::Thumbnails(_) => {
            let top = (0..index).map(|i| list_row_height(list, i, s, ts)).sum();
            (top, list_row_height(list, index, s, ts))
        }
        _ => {
            let h = list_row_height(list, index, s, ts);
            (index as f32 * h, h)
        }
    }
}

/// Rows that overlap the panel at this scroll offset, as (index, full row
/// rectangle), and the height of the whole list.
pub(super) fn sidebar_rows(list: &SidebarList, panel: Rect, scroll: f32, s: f32, ts: f32) -> (Vec<(usize, Rect)>, f32) {
    let count = list.len();
    let mut rows = Vec::new();
    let mut place = |index: usize, top: f32, h: f32| {
        if top + h > scroll && top < scroll + panel.height() {
            rows.push((index, Rect::new(panel.x0, panel.y0 + top - scroll, panel.width(), h)));
        }
    };
    let total = match list {
        SidebarList::Thumbnails(_) => {
            let mut top = 0.0;
            for index in 0..count {
                let h = list_row_height(list, index, s, ts);
                place(index, top, h);
                top += h;
            }
            top
        }
        _ if count == 0 => 0.0,
        _ => {
            let h = list_row_height(list, 0, s, ts);
            let first = (scroll / h).floor().max(0.0) as usize;
            let last = (((scroll + panel.height()) / h).ceil().max(0.0) as usize).min(count);
            for index in first..last {
                place(index, index as f32 * h, h);
            }
            count as f32 * h
        }
    };
    (rows, total)
}

pub(super) struct Input<'a> {
    /// Client size in physical pixels.
    pub(super) width: f32,
    pub(super) height: f32,
    /// Pixels per effective pixel (DPI / 96).
    pub(super) scale: f32,
    /// Windows text size setting, 1.0 to 2.25. Icons do not scale with it.
    pub(super) text_scale: f32,
    pub(super) tabs: &'a [String],
    pub(super) active_tab: Option<usize>,
    pub(super) maximized: bool,
    pub(super) has_document: bool,
    pub(super) title: &'a str,
    pub(super) sidebar_open: bool,
    pub(super) sidebar_tab: usize,
    pub(super) sidebar_list: SidebarList<'a>,
    pub(super) sidebar_scroll: f32,
    /// The list's one Tab stop: the current page, or the last row used.
    pub(super) sidebar_active: usize,
    /// Visible fraction of the markup bar, 0 to 1, while it slides.
    pub(super) markup: f32,
    pub(super) ctx: Ctx,
    pub(super) sheet: Option<SheetView<'a>>,
}

#[derive(Debug, Default)]
pub(super) struct Layout {
    pub(super) widgets: Vec<Widget>,
    pub(super) title_bar: Rect,
    pub(super) toolbar: Rect,
    pub(super) title_text: Rect,
    pub(super) markup_bar: Option<Rect>,
    pub(super) sidebar: Option<Rect>,
    pub(super) sidebar_panel: Rect,
    /// Height of the whole sidebar list, for scrolling.
    pub(super) sidebar_content: f32,
    pub(super) document: Rect,
    pub(super) status: Rect,
    pub(super) empty: Option<Empty>,
    pub(super) sheet: Option<SheetLayout>,
    pub(super) toolbar_overflow: Vec<Command>,
    pub(super) markup_overflow: Vec<Command>,
}

#[derive(Debug, Default)]
pub(super) struct Empty {
    pub(super) heading: Rect,
    pub(super) drop_hint: Rect,
    pub(super) recent_heading: Rect,
    pub(super) recent: Rect,
}

#[derive(Debug, Default)]
pub(super) struct SheetLayout {
    pub(super) card: Rect,
    pub(super) title: Rect,
    pub(super) message: Rect,
    pub(super) labels: Vec<Rect>,
    /// Text boxes. The EDIT child sits inside, inset by the border.
    pub(super) fields: Vec<Rect>,
}

pub(super) const CAPTION_WIDTH: f32 = 46.0;
const BUTTON: f32 = 40.0;
const GAP: f32 = 4.0;
const PAD: f32 = 8.0;
pub(super) const SIDEBAR_WIDTH: f32 = 240.0;
pub(super) const SIDEBAR_TABS: [&str; 3] = ["Thumbnails", "Contents", "Notes"];

/// Heights in epx. Rows that hold text grow with the text size setting.
pub(super) fn title_height(text_scale: f32) -> f32 {
    40f32.max(20.0 * text_scale + 16.0)
}
fn row_height(text_scale: f32) -> f32 {
    44f32.max(20.0 * text_scale + 12.0)
}
pub(super) fn control_height(text_scale: f32) -> f32 {
    32f32.max(20.0 * text_scale + 12.0)
}
fn status_height(text_scale: f32) -> f32 {
    28f32.max(16.0 * text_scale + 10.0)
}

/// Keeps the items in order and hides `drop` items, first to last, until
/// `fits` items remain.
pub(super) fn overflow<T: Copy + PartialEq>(items: &[T], drop: &[T], fits: usize) -> (Vec<T>, Vec<T>) {
    let mut hidden = Vec::new();
    for item in drop {
        if items.len() - hidden.len() <= fits {
            break;
        }
        if items.contains(item) {
            hidden.push(*item);
        }
    }
    for item in items.iter().rev() {
        if items.len() - hidden.len() <= fits {
            break;
        }
        if !hidden.contains(item) {
            hidden.push(*item);
        }
    }
    let visible = items.iter().copied().filter(|i| !hidden.contains(i)).collect();
    let hidden = items.iter().copied().filter(|i| hidden.contains(i)).collect();
    (visible, hidden)
}

fn command_widget(command: Command, region: Region, rect: Rect, ctx: &Ctx) -> Widget {
    let i = info(command);
    let tooltip = match commands::shortcut_text(command) {
        Some(keys) => format!("{} ({keys})", i.label),
        None => i.label.to_string(),
    };
    Widget {
        id: WidgetId::Command(command),
        role: Role::Button,
        region,
        rect,
        label: i.label.into(),
        glyph: i.glyph,
        tooltip,
        access_key: i.key,
        enabled: enabled(command, ctx),
        checked: commands::checked(command, ctx),
        focusable: true,
        primary: false,
    }
}

fn plain_widget(id: WidgetId, role: Role, region: Region, rect: Rect, label: String) -> Widget {
    Widget {
        id,
        role,
        region,
        rect,
        tooltip: String::new(),
        label,
        glyph: None,
        access_key: None,
        enabled: true,
        checked: None,
        focusable: true,
        primary: false,
    }
}

pub(super) fn layout(input: &Input) -> Layout {
    let s = input.scale;
    let ts = input.text_scale;
    let (width, height) = (input.width.max(1.0), input.height.max(1.0));
    let mut out = Layout::default();
    let w = &mut out.widgets;

    // Title bar: tabs, new tab, drag space, caption buttons.
    let title_h = title_height(ts) * s;
    out.title_bar = Rect::new(0.0, 0.0, width, title_h);
    let caption_w = CAPTION_WIDTH * s;
    let caption_x = width - 3.0 * caption_w;
    let tab_top = 6.0 * s;
    let new_tab_w = BUTTON * s;
    let tab_space = (caption_x - PAD * s - new_tab_w - 48.0 * s).max(0.0);
    let count = input.tabs.len();
    let tab_w = if count == 0 { 0.0 } else { (tab_space / count as f32).clamp(96.0 * s, 240.0 * s) };
    let mut x = PAD * s;
    for (index, title) in input.tabs.iter().enumerate() {
        if x + tab_w > PAD * s + tab_space {
            // Tabs that do not fit stay reachable with Ctrl+Tab and Ctrl+1 to Ctrl+9.
            break;
        }
        let rect = Rect::new(x, tab_top, tab_w, title_h - tab_top);
        let mut tab = plain_widget(WidgetId::Tab(index), Role::Tab, Region::TitleBar, rect, title.clone());
        tab.checked = Some(input.active_tab == Some(index));
        tab.tooltip = title.clone();
        w.push(tab);
        let close = 28.0 * s;
        let close_rect = Rect::new(rect.x1 - close - 6.0 * s, rect.y0 + (rect.height() - close) / 2.0, close, close);
        let mut close_button = plain_widget(WidgetId::TabClose(index), Role::Button, Region::TitleBar, close_rect, format!("Close {title}"));
        close_button.glyph = Some(commands::glyph::CANCEL);
        close_button.tooltip = "Close tab (Ctrl+W)".into();
        // Ctrl+W closes the focused tab, so the close button stays out of Tab order.
        close_button.focusable = false;
        w.push(close_button);
        x += tab_w;
    }
    let mut new_tab = command_widget(Command::Open, Region::TitleBar, Rect::new(x + GAP * s, tab_top, new_tab_w, title_h - tab_top - 2.0 * s), &input.ctx);
    new_tab.id = WidgetId::NewTab;
    new_tab.glyph = Some(commands::glyph::ADD);
    new_tab.label = "Open in new tab".into();
    new_tab.tooltip = "Open in new tab (Ctrl+T)".into();
    new_tab.access_key = None;
    w.push(new_tab);
    for (index, (id, label)) in
        [(WidgetId::Minimize, "Minimize"), (WidgetId::Maximize, if input.maximized { "Restore" } else { "Maximize" }), (WidgetId::Close, "Close")]
            .into_iter()
            .enumerate()
    {
        let rect = Rect::new(caption_x + index as f32 * caption_w, 0.0, caption_w, title_h);
        let mut button = plain_widget(id, Role::Caption, Region::TitleBar, rect, label.into());
        button.tooltip = label.into();
        // Caption buttons are not in Tab order, as in standard windows.
        button.focusable = false;
        w.push(button);
    }

    // Toolbar: sidebar at the left, then the file name, the PRD buttons, and More.
    let row_h = row_height(ts) * s;
    out.toolbar = Rect::new(0.0, title_h, width, row_h);
    let button = BUTTON * s;
    let button_y = title_h + (row_h - button) / 2.0;
    let mut sidebar = command_widget(Command::ToggleSidebar, Region::Toolbar, Rect::new(PAD * s, button_y, button, button), &input.ctx);
    sidebar.enabled = !input.has_document || input.ctx.pdf;
    w.push(sidebar);
    let step = button + GAP * s;
    let room = width - 2.0 * PAD * s - step - step;
    let fits = (room / step).floor().max(0.0) as usize;
    let (visible, hidden) = overflow(commands::TOOLBAR, commands::TOOLBAR_DROP, fits);
    out.toolbar_overflow = hidden;
    let more_x = width - PAD * s - button;
    let first_x = more_x - step * visible.len() as f32;
    for (index, command) in visible.iter().enumerate() {
        let rect = Rect::new(first_x + step * index as f32, button_y, button, button);
        w.push(command_widget(*command, Region::Toolbar, rect, &input.ctx));
    }
    w.push(command_widget(Command::AppMenu, Region::Toolbar, Rect::new(more_x, button_y, button, button), &input.ctx));
    let x = first_x;
    out.title_text = Rect { x0: PAD * s + step + 8.0 * s, y0: title_h, x1: (x - 8.0 * s).max(PAD * s + step), y1: title_h + row_h };

    // Markup bar, under the toolbar when open.
    let mut top = title_h + row_h;
    if input.markup > 0.0 {
        let bar = Rect::new(0.0, top, width, row_h * input.markup.min(1.0));
        out.markup_bar = Some(bar);
        // Tools appear once the bar has slid fully open, so a half-open bar
        // never takes clicks meant for the toolbar.
        let fits = if input.markup < 1.0 { 0 } else { ((width - 2.0 * PAD * s - step) / step).floor().max(0.0) as usize };
        let drop: Vec<Command> = commands::MARKUP_TOOLS.iter().rev().copied().collect();
        let (tools, hidden) = overflow(commands::MARKUP_TOOLS, &drop, fits);
        let total = (tools.len() + usize::from(!hidden.is_empty())) as f32 * step - GAP * s;
        let mut x = ((width - total) / 2.0).max(PAD * s);
        for command in &tools {
            w.push(command_widget(*command, Region::MarkupBar, Rect::new(x, top + (row_h - button) / 2.0, button, button), &input.ctx));
            x += step;
        }
        if !hidden.is_empty() && input.markup >= 1.0 {
            w.push(command_widget(Command::MoreTools, Region::MarkupBar, Rect::new(x, top + (row_h - button) / 2.0, button, button), &input.ctx));
        }
        out.markup_overflow = hidden;
        top = bar.y1;
    }

    let status_h = status_height(ts) * s;
    out.status = Rect::new(0.0, (height - status_h).max(top), width, status_h);
    let bottom = out.status.y0;

    // Sidebar frame with three tabs. Wave 2 fills the panels.
    let mut left = 0.0;
    if input.sidebar_open {
        // Wider with large text, so the three tab names still fit.
        let side_w = (SIDEBAR_WIDTH * s * (0.8 * ts).max(1.0)).min(width * 0.5);
        let side = Rect { x0: 0.0, y0: top, x1: side_w, y1: bottom };
        out.sidebar = Some(side);
        let tab_h = control_height(ts) * s;
        let each = (side_w - 2.0 * PAD * s) / SIDEBAR_TABS.len() as f32;
        for (index, label) in SIDEBAR_TABS.iter().enumerate() {
            let rect = Rect::new(PAD * s + index as f32 * each, top + PAD * s, each, tab_h);
            // The full "Thumbnails" label clips at Windows' 225% text size.
            let label = if ts >= 2.0 && index == 0 { "Pages" } else { label };
            let mut tab = plain_widget(WidgetId::SidebarTab(index), Role::SidebarTab, Region::Sidebar, rect, (*label).into());
            tab.checked = Some(input.sidebar_tab == index);
            w.push(tab);
        }
        let panel = Rect { x0: PAD * s, y0: top + PAD * s + tab_h + PAD * s, x1: side_w - PAD * s, y1: bottom };
        out.sidebar_panel = panel;
        let list = &input.sidebar_list;
        let (rows, total) = sidebar_rows(list, panel, input.sidebar_scroll, s, ts);
        out.sidebar_content = total;
        let active = if rows.iter().any(|(i, _)| *i == input.sidebar_active) { Some(input.sidebar_active) } else { rows.first().map(|r| r.0) };
        for (index, row) in rows {
            // Clipped to the panel, so a half-hidden row never takes clicks
            // meant for the sidebar tabs or the status bar.
            let rect = Rect { y0: row.y0.max(panel.y0), y1: row.y1.min(panel.y1), ..row };
            let mut item = plain_widget(WidgetId::SidebarItem(index), Role::ListItem, Region::Sidebar, rect, list.label(index));
            item.focusable = active == Some(index);
            match list {
                SidebarList::Thumbnails(_) => item.checked = Some(index == input.sidebar_active),
                SidebarList::Contents(items) => item.enabled = items[index].page.is_some(),
                _ => {}
            }
            w.push(item);
        }
        left = side_w;
    }

    out.document = Rect { x0: left, y0: top, x1: width.max(left), y1: bottom.max(top) };
    let doc = out.document;
    if input.has_document {
        let mut view = plain_widget(WidgetId::Document, Role::Document, Region::Document, doc, input.title.into());
        view.tooltip.clear();
        w.push(view);
    } else {
        // Empty state: an Open button and the recent files area. No tool tiles.
        let line = 28.0 * ts * s;
        let control = control_height(ts) * s;
        let hint = 20.0 * ts * s;
        let block = line + 16.0 * s + control + 8.0 * s + hint + 32.0 * s + 20.0 * ts * s + 8.0 * s + 20.0 * ts * s;
        let y = doc.y0 + ((doc.height() - block) / 2.0).max(16.0 * s);
        let cx = (doc.x0 + doc.x1) / 2.0;
        let text_w = (doc.width() - 32.0 * s).max(0.0);
        let heading = Rect::new(cx - text_w / 2.0, y, text_w, line);
        let open_w = 120f32.max(48.0 + 40.0 * ts) * s;
        let mut open = command_widget(Command::Open, Region::Document, Rect::new(cx - open_w / 2.0, heading.y1 + 16.0 * s, open_w, control), &input.ctx);
        open.primary = true;
        open.label = "Open".into();
        open.tooltip = "Open a file (Ctrl+O)".into();
        let open_bottom = open.rect.y1;
        w.push(open);
        let drop_hint = Rect::new(heading.x0, open_bottom + 8.0 * s, text_w, hint);
        let recent_heading = Rect::new(heading.x0, drop_hint.y1 + 32.0 * s, text_w, 20.0 * ts * s);
        let recent = Rect::new(heading.x0, recent_heading.y1 + 8.0 * s, text_w, 20.0 * ts * s);
        out.empty = Some(Empty { heading, drop_hint, recent_heading, recent });
    }

    if let Some(sheet) = &input.sheet {
        // Modal sheet: only its widgets take input while it is open.
        for widget in w.iter_mut() {
            widget.focusable = false;
        }
        let card_w = (440.0 * s).min(width - 32.0 * s).max(160.0 * s);
        let pad = 24.0 * s;
        let inner = card_w - 2.0 * pad;
        let control = control_height(ts) * s;
        let label_h = 20.0 * ts * s;
        let title_line = 28.0 * ts * s;
        let message_h = sheet.message_height * s;
        let fields_h: f32 = sheet.fields.len() as f32 * (label_h + 4.0 * s + control + 12.0 * s);
        let card_h = pad + title_line + 12.0 * s + message_h + if message_h > 0.0 { 16.0 * s } else { 0.0 } + fields_h + 12.0 * s + control + pad;
        let card = Rect::new((width - card_w) / 2.0, ((height - card_h) / 2.0).max(title_h), card_w, card_h);
        let mut y = card.y0 + pad;
        let title = Rect::new(card.x0 + pad, y, inner, title_line);
        y += title_line + 12.0 * s;
        let message = Rect::new(card.x0 + pad, y, inner, message_h);
        y += message_h + if message_h > 0.0 { 16.0 * s } else { 0.0 };
        let mut layout = SheetLayout { card, title, message, ..Default::default() };
        for (index, label) in sheet.fields.iter().enumerate() {
            let label_rect = Rect::new(card.x0 + pad, y, inner, label_h);
            y += label_h + 4.0 * s;
            let field = Rect::new(card.x0 + pad, y, inner, control);
            y += control + 12.0 * s;
            layout.labels.push(label_rect);
            layout.fields.push(field);
            let mut widget = plain_widget(WidgetId::SheetField(index), Role::Field, Region::Sheet, field, (*label).into());
            widget.focusable = true;
            w.push(widget);
        }
        y += 12.0 * s;
        let count = sheet.buttons.len().max(1) as f32;
        let button_w = (inner - (count - 1.0) * 8.0 * s) / count;
        for (index, label) in sheet.buttons.iter().enumerate() {
            let rect = Rect::new(card.x0 + pad + index as f32 * (button_w + 8.0 * s), y, button_w, control);
            let mut widget = plain_widget(WidgetId::SheetButton(index), Role::Button, Region::Sheet, rect, (*label).into());
            widget.primary = index == 0;
            widget.access_key = None;
            w.push(widget);
        }
        out.sheet = Some(layout);
    }
    out
}

/// Topmost widget under the point. A sheet blocks everything outside it.
pub(super) fn hit(widgets: &[Widget], x: f32, y: f32) -> Option<&Widget> {
    let modal = widgets.iter().any(|w| w.region == Region::Sheet);
    widgets.iter().rev().filter(|w| !modal || w.region == Region::Sheet || w.role == Role::Caption).find(|w| w.rect.contains(x, y))
}

fn can_focus(w: &Widget) -> bool {
    w.focusable && w.enabled
}

/// Tab and Shift+Tab order is the widget order.
pub(super) fn next_focus(widgets: &[Widget], current: Option<WidgetId>, back: bool) -> Option<WidgetId> {
    let order: Vec<WidgetId> = widgets.iter().filter(|w| can_focus(w)).map(|w| w.id).collect();
    if order.is_empty() {
        return None;
    }
    let position = current.and_then(|c| order.iter().position(|id| *id == c));
    let index = match (position, back) {
        (None, false) => 0,
        (None, true) => order.len() - 1,
        (Some(p), false) => (p + 1) % order.len(),
        (Some(p), true) => (p + order.len() - 1) % order.len(),
    };
    Some(order[index])
}

/// F6 and Shift+F6: the first focusable widget of the next pane.
pub(super) fn next_region(widgets: &[Widget], current: Option<WidgetId>, back: bool) -> Option<WidgetId> {
    let mut regions: Vec<Region> = Vec::new();
    for w in widgets.iter().filter(|w| can_focus(w)) {
        if regions.last() != Some(&w.region) {
            regions.push(w.region);
        }
    }
    if regions.is_empty() {
        return None;
    }
    let region = current.and_then(|c| widgets.iter().find(|w| w.id == c)).map(|w| w.region);
    let position = region.and_then(|r| regions.iter().position(|x| *x == r));
    let target = match (position, back) {
        (None, false) => regions[0],
        (None, true) => regions[regions.len() - 1],
        (Some(p), false) => regions[(p + 1) % regions.len()],
        (Some(p), true) => regions[(p + regions.len() - 1) % regions.len()],
    };
    widgets.iter().find(|w| can_focus(w) && w.region == target).map(|w| w.id)
}

pub(super) fn in_scope(widget: &Widget, scope: Scope) -> bool {
    match scope {
        Scope::Root => matches!(widget.region, Region::Toolbar | Region::Document | Region::TitleBar),
        Scope::Markup => widget.region == Region::MarkupBar,
    }
}

/// The enabled widget whose access key is `key` in this scope.
pub(super) fn access_key_target(widgets: &[Widget], scope: Scope, key: char) -> Option<WidgetId> {
    let key = key.to_ascii_uppercase();
    widgets.iter().find(|w| w.enabled && w.access_key == Some(key) && in_scope(w, scope)).map(|w| w.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(width: f32, tabs: &'a [String]) -> Input<'a> {
        Input {
            width,
            height: 800.0,
            scale: 1.0,
            text_scale: 1.0,
            tabs,
            active_tab: tabs.first().map(|_| 0),
            maximized: false,
            has_document: !tabs.is_empty(),
            title: "a.pdf",
            sidebar_open: false,
            sidebar_tab: 0,
            sidebar_list: SidebarList::Message(""),
            sidebar_scroll: 0.0,
            sidebar_active: 0,
            markup: 0.0,
            ctx: Ctx { has_frame: !tabs.is_empty(), pdf: true, tabs: tabs.len(), ..Default::default() },
            sheet: None,
        }
    }
    fn ids(layout: &Layout, region: Region) -> Vec<WidgetId> {
        layout.widgets.iter().filter(|w| w.region == region).map(|w| w.id).collect()
    }
    fn cmd(c: Command) -> WidgetId {
        WidgetId::Command(c)
    }

    #[test]
    fn toolbar_shows_the_six_prd_buttons_at_normal_width() {
        let tabs = vec!["a.pdf".to_string()];
        let layout = layout(&input(1100.0, &tabs));
        assert_eq!(
            ids(&layout, Region::Toolbar),
            vec![
                cmd(Command::ToggleSidebar),
                cmd(Command::ZoomMenu),
                cmd(Command::ToggleMarkup),
                cmd(Command::Rotate),
                cmd(Command::Share),
                cmd(Command::Find),
                cmd(Command::AppMenu)
            ]
        );
        assert!(layout.toolbar_overflow.is_empty());
        let rects: Vec<Rect> = layout.widgets.iter().filter(|w| w.region == Region::Toolbar).map(|w| w.rect).collect();
        assert!(rects.windows(2).all(|p| p[0].x1 <= p[1].x0), "buttons overlap");
        assert!(rects.last().unwrap().x1 <= 1100.0);
    }

    #[test]
    fn narrow_toolbar_overflows_share_then_rotate_into_the_menu() {
        let tabs = vec!["a.pdf".to_string()];
        let five = layout(&input(2.0 * PAD + 6.0 * 44.0 + 40.0, &tabs));
        assert_eq!(five.toolbar_overflow, vec![Command::Share]);
        let four = layout(&input(2.0 * PAD + 5.0 * 44.0 + 40.0, &tabs));
        assert_eq!(four.toolbar_overflow, vec![Command::Rotate, Command::Share]);
        assert!(ids(&four, Region::Toolbar).contains(&cmd(Command::AppMenu)));
        let tiny = layout(&input(60.0, &tabs));
        assert_eq!(tiny.toolbar_overflow.len(), commands::TOOLBAR.len());
    }

    #[test]
    fn overflow_keeps_order_and_falls_back_to_the_end() {
        assert_eq!(overflow(&[1, 2, 3, 4], &[4, 2], 3), (vec![1, 2, 3], vec![4]));
        assert_eq!(overflow(&[1, 2, 3, 4], &[4, 2], 2), (vec![1, 3], vec![2, 4]));
        assert_eq!(overflow(&[1, 2, 3, 4], &[4], 1), (vec![1], vec![2, 3, 4]));
        assert_eq!(overflow(&[1, 2], &[2], 5), (vec![1, 2], vec![]));
    }

    #[test]
    fn markup_bar_and_sidebar_take_space_from_the_document() {
        let tabs = vec!["a.pdf".to_string()];
        let base = layout(&input(1100.0, &tabs));
        let mut i = input(1100.0, &tabs);
        i.markup = 1.0;
        i.sidebar_open = true;
        let open = layout(&i);
        let bar = open.markup_bar.unwrap();
        assert_eq!(bar.y0, base.toolbar.y1);
        assert_eq!(open.document.y0, bar.y1);
        assert_eq!(open.document.x0, SIDEBAR_WIDTH);
        assert_eq!(ids(&open, Region::Sidebar).len(), 3);
        assert_eq!(ids(&open, Region::MarkupBar).len(), commands::MARKUP_TOOLS.len());
        assert!(open.document.height() > 0.0 && open.document.y1 == open.status.y0);
        let mut narrow = input(400.0, &tabs);
        narrow.markup = 1.0;
        let narrow = layout(&narrow);
        assert!(!narrow.markup_overflow.is_empty());
        assert!(ids(&narrow, Region::MarkupBar).contains(&cmd(Command::MoreTools)));
        assert!(narrow.widgets.iter().all(|w| w.rect.x1 <= 400.0 + 0.01));
    }

    #[test]
    fn half_open_markup_bar_has_no_tools_yet() {
        let tabs = vec!["a.pdf".to_string()];
        let mut i = input(1100.0, &tabs);
        i.markup = 0.5;
        let half = layout(&i);
        assert_eq!(half.markup_bar.unwrap().height(), 22.0);
        assert_eq!(half.document.y0, half.markup_bar.unwrap().y1);
        assert!(ids(&half, Region::MarkupBar).is_empty());
    }

    #[test]
    fn caption_buttons_sit_at_the_top_right_and_tabs_fit_before_them() {
        let tabs: Vec<String> = (0..30).map(|n| format!("{n}.png")).collect();
        let layout = layout(&input(1000.0, &tabs));
        let close = layout.widgets.iter().find(|w| w.id == WidgetId::Close).unwrap();
        assert_eq!((close.rect.x1, close.rect.y0), (1000.0, 0.0));
        let max = layout.widgets.iter().find(|w| w.id == WidgetId::Maximize).unwrap();
        assert_eq!(max.rect.x1, close.rect.x0);
        let last_tab = layout.widgets.iter().filter(|w| w.role == Role::Tab).last().unwrap();
        assert!(last_tab.rect.x1 + 40.0 <= max.rect.x0 - CAPTION_WIDTH);
        assert!(last_tab.rect.width() >= 96.0);
        assert!(layout.widgets.iter().filter(|w| w.role == Role::Tab).count() < 30);
    }

    #[test]
    fn text_scale_grows_text_rows_but_not_icon_buttons() {
        let tabs = vec!["a.pdf".to_string()];
        let mut big = input(1100.0, &tabs);
        big.text_scale = 2.25;
        big.sidebar_open = true;
        let big = layout(&big);
        let normal = layout(&input(1100.0, &tabs));
        assert_eq!(normal.toolbar.height(), 44.0);
        assert!(big.title_bar.height() > normal.title_bar.height());
        assert!(big.status.height() > normal.status.height());
        let button = |l: &Layout| l.widgets.iter().find(|w| w.id == cmd(Command::Rotate)).unwrap().rect.width();
        assert_eq!(button(&normal), 40.0);
        assert_eq!(button(&big), button(&normal), "225% text keeps 40 DIP icon targets");
        assert!(big.toolbar.height() >= 20.0 * 2.25 + 12.0, "the row grows only enough to keep large text readable");
        let tabs: Vec<_> = big.widgets.iter().filter(|w| w.role == Role::SidebarTab).collect();
        assert_eq!(tabs[0].label, "Pages", "the compact selector label stays readable at 225% text");
        assert!(tabs.windows(2).all(|pair| pair[0].rect.x1 <= pair[1].rect.x0));
    }

    #[test]
    fn dpi_scale_multiplies_every_dimension() {
        let tabs = vec!["a.pdf".to_string()];
        let mut double = input(2200.0, &tabs);
        double.height = 1600.0;
        double.scale = 2.0;
        let double = layout(&double);
        let normal = layout(&input(1100.0, &tabs));
        assert_eq!(double.title_bar.height(), 2.0 * normal.title_bar.height());
        assert_eq!(double.document.y0, 2.0 * normal.document.y0);
    }

    #[test]
    fn hit_test_prefers_topmost_and_sheet_is_modal() {
        let tabs = vec!["a.pdf".to_string()];
        let mut i = input(1100.0, &tabs);
        let plain = layout(&i);
        let close_tab = plain.widgets.iter().find(|w| w.id == WidgetId::TabClose(0)).unwrap();
        let (x, y) = (close_tab.rect.x0 + 2.0, close_tab.rect.y0 + 2.0);
        assert_eq!(hit(&plain.widgets, x, y).unwrap().id, WidgetId::TabClose(0));
        assert_eq!(hit(&plain.widgets, 600.0, 400.0).unwrap().id, WidgetId::Document);
        i.sheet = Some(SheetView { message_height: 0.0, fields: vec!["Text"], buttons: vec!["OK", "Cancel"] });
        let modal = layout(&i);
        assert!(hit(&modal.widgets, x, y).is_none());
        let ok = modal.widgets.iter().find(|w| w.id == WidgetId::SheetButton(0)).unwrap();
        assert!(ok.primary);
        assert_eq!(hit(&modal.widgets, ok.rect.x0 + 1.0, ok.rect.y0 + 1.0).unwrap().id, WidgetId::SheetButton(0));
        let close = modal.widgets.iter().find(|w| w.id == WidgetId::Close).unwrap();
        assert_eq!(hit(&modal.widgets, close.rect.x0 + 1.0, 1.0).unwrap().id, WidgetId::Close);
        let card = modal.sheet.as_ref().unwrap().card;
        assert!(card.contains(ok.rect.x0, ok.rect.y0) && card.y1 <= 800.0);
    }

    #[test]
    fn tab_order_skips_disabled_and_wraps() {
        let tabs = vec!["a.pdf".to_string(), "b.pdf".to_string()];
        let layout = layout(&input(1100.0, &tabs));
        let first = next_focus(&layout.widgets, None, false).unwrap();
        assert_eq!(first, WidgetId::Tab(0));
        let mut seen = vec![first];
        let mut current = first;
        loop {
            current = next_focus(&layout.widgets, Some(current), false).unwrap();
            if current == first {
                break;
            }
            seen.push(current);
        }
        assert!(seen.contains(&WidgetId::Document));
        assert!(!seen.contains(&cmd(Command::Share)), "disabled Share is not a Tab stop");
        assert!(!seen.contains(&WidgetId::Close) && !seen.contains(&WidgetId::TabClose(0)));
        assert_eq!(next_focus(&layout.widgets, Some(first), true), seen.last().copied());
        let toolbar = seen.iter().position(|id| *id == cmd(Command::ToggleSidebar)).unwrap();
        let document = seen.iter().position(|id| *id == WidgetId::Document).unwrap();
        assert!(toolbar < document);
    }

    #[test]
    fn sheet_limits_tab_order_to_its_fields_and_buttons() {
        let tabs = vec!["a.png".to_string()];
        let mut i = input(1100.0, &tabs);
        i.sheet = Some(SheetView { message_height: 20.0, fields: vec!["Width", "Height"], buttons: vec!["OK", "Cancel"] });
        let layout = layout(&i);
        let mut order = vec![];
        let mut current = None;
        for _ in 0..4 {
            current = next_focus(&layout.widgets, current, false);
            order.push(current.unwrap());
        }
        assert_eq!(order, vec![WidgetId::SheetField(0), WidgetId::SheetField(1), WidgetId::SheetButton(0), WidgetId::SheetButton(1)]);
        assert_eq!(next_focus(&layout.widgets, current, false), Some(WidgetId::SheetField(0)));
        let sheet = layout.sheet.unwrap();
        assert!(sheet.fields[0].y1 <= sheet.fields[1].y0 && sheet.message.y1 <= sheet.labels[0].y0);
    }

    #[test]
    fn f6_moves_between_panes() {
        let tabs = vec!["a.pdf".to_string()];
        let mut i = input(1100.0, &tabs);
        i.markup = 1.0;
        i.sidebar_open = true;
        let layout = layout(&i);
        let mut current = Some(WidgetId::Tab(0));
        let mut regions = vec![];
        for _ in 0..5 {
            current = next_region(&layout.widgets, current, false);
            regions.push(layout.widgets.iter().find(|w| Some(w.id) == current).unwrap().region);
        }
        assert_eq!(regions, vec![Region::Toolbar, Region::MarkupBar, Region::Sidebar, Region::Document, Region::TitleBar]);
        assert_eq!(next_region(&layout.widgets, Some(WidgetId::Tab(0)), true), Some(WidgetId::Document));
    }

    #[test]
    fn access_keys_resolve_per_scope() {
        let tabs = vec!["a.pdf".to_string()];
        let mut i = input(1100.0, &tabs);
        i.markup = 1.0;
        let layout = layout(&i);
        assert_eq!(access_key_target(&layout.widgets, Scope::Root, 'm'), Some(cmd(Command::ToggleMarkup)));
        assert_eq!(access_key_target(&layout.widgets, Scope::Root, 'O'), Some(cmd(Command::AppMenu)));
        assert_eq!(access_key_target(&layout.widgets, Scope::Markup, 'H'), Some(cmd(Command::Highlight)));
        assert_eq!(access_key_target(&layout.widgets, Scope::Root, 'H'), None, "Share is disabled");
        assert_eq!(access_key_target(&layout.widgets, Scope::Markup, 'S'), None);
    }

    #[test]
    fn sidebar_lists_only_rows_in_view_with_one_tab_stop() {
        let tabs = vec!["a.pdf".to_string()];
        let sizes = vec![[612.0, 792.0]; 500];
        let mut i = input(1100.0, &tabs);
        i.sidebar_open = true;
        i.sidebar_list = SidebarList::Thumbnails(&sizes);
        i.sidebar_active = 3;
        let l = layout(&i);
        let items: Vec<&Widget> = l.widgets.iter().filter(|w| w.role == Role::ListItem).collect();
        let row = sidebar_row(&i.sidebar_list, 1, 1.0, 1.0).0;
        assert!(items.len() < 10 && items.len() as f32 >= l.sidebar_panel.height() / row);
        assert_eq!(l.sidebar_content, 500.0 * row);
        assert_eq!(items[0].label, "Thumbnail, page 1");
        assert_eq!(items.iter().filter(|w| w.focusable).map(|w| w.id).collect::<Vec<_>>(), vec![WidgetId::SidebarItem(3)]);
        assert_eq!(items[3].checked, Some(true), "the current page is marked");
        assert!(items.iter().all(|w| w.rect.y0 >= l.sidebar_panel.y0 && w.rect.y1 <= l.sidebar_panel.y1));
        i.sidebar_scroll = row * 250.0 + 5.0;
        let far = layout(&i);
        let first = far.widgets.iter().find(|w| w.role == Role::ListItem).unwrap();
        assert_eq!(first.id, WidgetId::SidebarItem(250));
        assert!(first.focusable, "page 4 is out of view, so the first row in view takes the Tab stop");
        let outline: Vec<OutlineItem> = (0..100_000).map(|n| OutlineItem { title: format!("Part {n}"), page: (n % 2 == 0).then_some(n), level: 0 }).collect();
        i.sidebar_list = SidebarList::Contents(&outline);
        i.sidebar_scroll = 32.0 * 5000.0;
        let contents = layout(&i);
        let rows: Vec<&Widget> = contents.widgets.iter().filter(|w| w.role == Role::ListItem).collect();
        assert_eq!((rows[0].id, rows[0].label.as_str()), (WidgetId::SidebarItem(5000), "Part 5000"));
        assert!(rows[0].enabled && !rows[1].enabled, "entries without a page are disabled");
        assert!(rows.len() <= 30);
        assert_eq!(contents.sidebar_content, 32.0 * 100_000.0);
        i.sidebar_list = SidebarList::Message("No contents");
        assert!(!layout(&i).widgets.iter().any(|w| w.role == Role::ListItem));
    }

    #[test]
    fn thumbnails_fit_a_portrait_box() {
        assert_eq!(thumb_size([612.0, 792.0], 1.0), (120.0, 155.0));
        assert_eq!(thumb_size([792.0, 612.0], 2.0), (240.0, 185.0));
        assert_eq!(thumb_size([100.0, 1000.0], 1.0), (17.0, 168.0));
    }

    #[test]
    fn empty_state_has_an_open_button_and_recent_files_area() {
        let layout = layout(&input(1100.0, &[]));
        let open = layout.widgets.iter().find(|w| w.region == Region::Document).unwrap();
        assert_eq!(open.id, cmd(Command::Open));
        assert!(open.primary && open.enabled);
        let empty = layout.empty.as_ref().unwrap();
        assert!(empty.heading.y1 <= open.rect.y0 && open.rect.y1 <= empty.drop_hint.y0);
        assert!(empty.drop_hint.y1 <= empty.recent_heading.y0);
        assert!(layout.document.contains(open.rect.x0, open.rect.y0));
        assert!(!layout.widgets.iter().any(|w| w.id == WidgetId::Document));
    }
}
