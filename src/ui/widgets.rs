//! Retained widget list: layout, hit testing, Tab order, F6 panes, and
//! access keys. Pure code, so all of it runs in unit tests.
use super::{
    commands::{self, Command, Ctx, enabled, info},
    theme::size,
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
    SheetControl(usize),
    Document,
    /// A row of the open sidebar list: a page thumbnail, an outline entry,
    /// or a note.
    SidebarItem(usize),
    FindField,
    FindCase,
    FindClose,
    InfoButton(usize),
    InfoClose,
    /// A row of the recent files list on the empty window.
    Recent(usize),
    /// A file in the Quick view index sheet.
    IndexItem(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Role {
    Tab,
    Button,
    Caption,
    SidebarTab,
    Field,
    Slider,
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
    FindBar,
    InfoBar,
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
    pub(super) controls: &'a [super::sheet::Control],
}

/// What the sidebar panel lists. Only rows in view become widgets.
#[derive(Clone, Copy)]
pub(super) enum SidebarList<'a> {
    /// Page sizes in points.
    Thumbnails(&'a [[f32; 2]]),
    /// The contact sheet: thumbnails in a grid that fills the window.
    Sheet(&'a [[f32; 2]]),
    Contents(&'a [OutlineItem]),
    Notes(&'a [Note]),
    /// "No contents", "Looking for notes", and similar.
    Message(&'a str),
}

impl SidebarList<'_> {
    pub(super) fn len(&self) -> usize {
        match self {
            SidebarList::Thumbnails(s) | SidebarList::Sheet(s) => s.len(),
            SidebarList::Contents(s) => s.len(),
            SidebarList::Notes(s) => s.len(),
            SidebarList::Message(_) => 0,
        }
    }
    /// Accessible name and visible text of a row.
    pub(super) fn label(&self, index: usize) -> String {
        match self {
            SidebarList::Thumbnails(_) | SidebarList::Sheet(_) => format!("Thumbnail, page {}", index + 1),
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

/// Contact sheet grid in a panel `width` pixels wide: columns, cell width,
/// and cell height. Every cell holds the tallest thumbnail box.
pub(super) fn sheet_grid(width: f32, s: f32, ts: f32) -> (usize, f32, f32) {
    let columns = ((width / ((THUMB_WIDTH + 24.0) * s)).floor() as usize).max(1);
    (columns, width / columns as f32, (1.4 * THUMB_WIDTH + 12.0 + 20.0 * ts) * s)
}

/// Top and height of a list row in content pixels, in a panel `width`
/// pixels wide.
pub(super) fn sidebar_row(list: &SidebarList, index: usize, width: f32, s: f32, ts: f32) -> (f32, f32) {
    match list {
        SidebarList::Thumbnails(_) => {
            let top = (0..index).map(|i| list_row_height(list, i, s, ts)).sum();
            (top, list_row_height(list, index, s, ts))
        }
        SidebarList::Sheet(_) => {
            let (columns, _, h) = sheet_grid(width, s, ts);
            ((index / columns) as f32 * h, h)
        }
        _ => {
            let h = list_row_height(list, index, s, ts);
            (index as f32 * h, h)
        }
    }
}

/// The scroll offset at or after `y` where a row starts, so revealing a row
/// below never leaves the top row cut off with only its blank lower part showing.
pub(super) fn row_start_after(list: &SidebarList, y: f32, width: f32, s: f32, ts: f32) -> f32 {
    match list {
        SidebarList::Thumbnails(_) => {
            let mut top = 0.0;
            for index in 0..list.len() {
                if top >= y {
                    break;
                }
                top += list_row_height(list, index, s, ts);
            }
            top
        }
        _ => {
            let h = sidebar_row(list, 0, width, s, ts).1.max(1.0);
            (y / h).ceil() * h
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
    if let SidebarList::Sheet(_) = list {
        let (columns, w, h) = sheet_grid(panel.width(), s, ts);
        let first = (scroll / h).floor().max(0.0) as usize;
        let last = ((scroll + panel.height()) / h).ceil().max(0.0) as usize;
        for index in (first * columns..last * columns).take_while(|i| *i < count) {
            let (row, column) = (index / columns, index % columns);
            rows.push((index, Rect::new(panel.x0 + column as f32 * w, panel.y0 + row as f32 * h - scroll, w, h)));
        }
        return (rows, count.div_ceil(columns) as f32 * h);
    }
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
    /// The editor bar and tab strip show. Full screen hides them until the
    /// pointer reaches the top edge.
    pub(super) chrome: bool,
    pub(super) has_document: bool,
    pub(super) title: &'a str,
    pub(super) sidebar_open: bool,
    /// Sidebar width in epx; the user can drag its edge.
    pub(super) sidebar_width: f32,
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
    /// The one editor bar: sidebar button, title, tools, caption buttons.
    pub(super) title_bar: Rect,
    /// File name and page count.
    pub(super) title_text: Rect,
    /// Shown only with 2 or more documents.
    pub(super) tab_strip: Option<Rect>,
    pub(super) markup_bar: Option<Rect>,
    pub(super) sidebar: Option<Rect>,
    /// The strip at the sidebar's edge that resizes it.
    pub(super) sidebar_edge: Option<Rect>,
    pub(super) sidebar_panel: Rect,
    /// Height of the whole sidebar list, for scrolling.
    pub(super) sidebar_content: f32,
    /// The contact sheet covers the document.
    pub(super) contact_sheet: bool,
    pub(super) document: Rect,
    /// The document area before the info bar pushed it down.
    pub(super) info_area: Rect,
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
const PAD: f32 = 8.0;
/// Sidebar view switcher: icon buttons, so the four modes fit 200 epx.
const SIDEBAR_TAB: f32 = 32.0;
pub(super) const SIDEBAR_TABS: [&str; 4] = ["Thumbnails", "Table of contents", "Highlights and notes", "Contact sheet"];
/// The contact sheet's index in `SIDEBAR_TABS`.
pub(super) const SHEET_TAB: usize = 3;

/// Heights in epx. Rows that hold text grow with the text size setting.
/// The editor bar holds the file name over the page count.
pub(super) fn bar_height(text_scale: f32) -> f32 {
    size::BAR.max(26.0 * text_scale + 12.0)
}
pub(super) fn control_height(text_scale: f32) -> f32 {
    32f32.max(20.0 * text_scale + 12.0)
}
fn tab_strip_height(text_scale: f32) -> f32 {
    control_height(text_scale) + 8.0
}

/// Width of a row of buttons with the gaps between them.
fn row_width(commands: &[Command], button: f32, s: f32) -> f32 {
    commands.iter().enumerate().map(|(i, c)| button + if i == 0 { 0.0 } else { gap_before(*c, s) }).sum()
}
fn gap_before(command: Command, s: f32) -> f32 {
    if commands::starts_group(command) { size::GROUP_GAP * s } else { size::BAR_GAP * s }
}

/// Places a row of command buttons from `x`, `button` wide, centered on `mid_y`.
#[allow(clippy::too_many_arguments)]
fn place_row(w: &mut Vec<Widget>, commands: &[Command], region: Region, x: f32, mid_y: f32, button: f32, s: f32, ctx: &Ctx) {
    let mut x = x;
    for (i, command) in commands.iter().enumerate() {
        if i > 0 {
            x += gap_before(*command, s);
        }
        w.push(command_widget(*command, region, Rect::new(x, mid_y - button / 2.0, button, button), ctx));
        x += button;
    }
}

/// The most items that fit in `room` with the `more` button after them,
/// hiding `drop` items first. The row to place is returned first.
fn fit_row(items: &[Command], drop: &[Command], more: Command, always_more: bool, room: f32, button: f32, s: f32) -> (Vec<Command>, Vec<Command>) {
    let mut fits = items.len();
    loop {
        let (mut row, hidden) = overflow(items, drop, fits);
        if always_more || !hidden.is_empty() {
            row.push(more);
        }
        if fits == 0 || row_width(&row, button, s) <= room {
            return (row, hidden);
        }
        fits -= 1;
    }
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

pub(super) fn command_widget(command: Command, region: Region, rect: Rect, ctx: &Ctx) -> Widget {
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

pub(super) fn plain_widget(id: WidgetId, role: Role, region: Region, rect: Rect, label: String) -> Widget {
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

    // The editor bar: sidebar, title, tools, More, and caption buttons in
    // one row, as Preview's unified title bar and toolbar.
    let mut top = 0.0;
    let (pad, button) = (size::BAR_PAD * s, size::BAR_BUTTON * s);
    if input.chrome {
        let bar_h = bar_height(ts) * s;
        out.title_bar = Rect::new(0.0, 0.0, width, bar_h);
        let caption_w = CAPTION_WIDTH * s;
        let caption_x = width - 3.0 * caption_w;
        let mid = bar_h / 2.0;
        let mut sidebar = command_widget(Command::ToggleSidebar, Region::Toolbar, Rect::new(pad, mid - button / 2.0, button, button), &input.ctx);
        sidebar.enabled = input.ctx.pdf;
        w.push(sidebar);
        let title_x = pad + button + size::GROUP_GAP * s;
        let right = caption_x - pad;
        let room = (right - title_x - 96.0 * s).max(0.0);
        let (row, hidden) = fit_row(commands::TOOLBAR, commands::TOOLBAR_DROP, Command::AppMenu, true, room, button, s);
        out.toolbar_overflow = hidden;
        let first_x = right - row_width(&row, button, s);
        place_row(w, &row, Region::Toolbar, first_x, mid, button, s, &input.ctx);
        out.title_text = Rect { x0: title_x, y0: 0.0, x1: (first_x - size::GROUP_GAP * s).max(title_x), y1: bar_h };
        for (index, (id, label)) in
            [(WidgetId::Minimize, "Minimize"), (WidgetId::Maximize, if input.maximized { "Restore" } else { "Maximize" }), (WidgetId::Close, "Close")]
                .into_iter()
                .enumerate()
        {
            let rect = Rect::new(caption_x + index as f32 * caption_w, 0.0, caption_w, bar_h);
            let mut button = plain_widget(id, Role::Caption, Region::TitleBar, rect, label.into());
            button.tooltip = label.into();
            // Caption buttons are not in Tab order, as in standard windows.
            button.focusable = false;
            w.push(button);
        }
        top = bar_h;
    }

    // Tab strip under the bar, only with 2 or more documents, as Preview
    // shows tabs only when windows merge.
    let count = input.tabs.len();
    if input.chrome && count >= 2 {
        let strip = Rect::new(0.0, top, width, tab_strip_height(ts) * s);
        out.tab_strip = Some(strip);
        let tab_h = strip.height() - 8.0 * s;
        let tab_top = top + 4.0 * s;
        let new_tab_w = tab_h;
        let space = (width - 2.0 * pad - new_tab_w - size::BAR_GAP * s).max(0.0);
        let tab_w = (space / count as f32).max(96.0 * s);
        let mut x = pad;
        for (index, title) in input.tabs.iter().enumerate() {
            if x + tab_w > pad + space + 0.5 {
                // Tabs that do not fit stay reachable with Ctrl+Tab and Ctrl+1 to Ctrl+8.
                break;
            }
            let rect = Rect::new(x, tab_top, tab_w, tab_h);
            let mut tab = plain_widget(WidgetId::Tab(index), Role::Tab, Region::TitleBar, rect, title.clone());
            tab.checked = Some(input.active_tab == Some(index));
            tab.tooltip = title.clone();
            w.push(tab);
            let close = 24.0 * s;
            let close_rect = Rect::new(rect.x1 - close - 4.0 * s, rect.y0 + (rect.height() - close) / 2.0, close, close);
            let mut close_button = plain_widget(WidgetId::TabClose(index), Role::Button, Region::TitleBar, close_rect, format!("Close {title}"));
            close_button.glyph = Some(commands::glyph::CANCEL);
            close_button.tooltip = "Close tab (Ctrl+W)".into();
            // Ctrl+W closes the focused tab, so the close button stays out of Tab order.
            close_button.focusable = false;
            w.push(close_button);
            x += tab_w;
        }
        let mut new_tab = command_widget(Command::Open, Region::TitleBar, Rect::new(x + size::BAR_GAP * s, tab_top, new_tab_w, tab_h), &input.ctx);
        new_tab.id = WidgetId::NewTab;
        new_tab.glyph = Some(commands::glyph::ADD);
        new_tab.label = "Open in new tab".into();
        new_tab.tooltip = "Open in new tab (Ctrl+T)".into();
        new_tab.access_key = None;
        w.push(new_tab);
        top = strip.y1;
    }

    // Markup bar, under the bar when open, with only this file type's tools.
    if input.markup > 0.0 {
        let bar_h = size::MARKUP_BAR * s;
        let bar = Rect::new(0.0, top, width, bar_h * input.markup.min(1.0));
        out.markup_bar = Some(bar);
        let small = size::MARKUP_BUTTON * s;
        let tools: Vec<Command> =
            commands::MARKUP_TOOLS.iter().copied().filter(|c| !input.has_document || commands::for_type(*c, input.ctx.pdf)).collect();
        let drop: Vec<Command> = tools.iter().rev().copied().collect();
        // Tools appear once the bar has slid fully open, so a half-open bar
        // never takes clicks meant for the bar above.
        if input.markup >= 1.0 {
            let (row, hidden) = fit_row(&tools, &drop, Command::MoreTools, false, width - 2.0 * pad, small, s);
            let x = ((width - row_width(&row, small, s)) / 2.0).max(pad);
            place_row(w, &row, Region::MarkupBar, x, top + bar_h / 2.0, small, s, &input.ctx);
            out.markup_overflow = hidden;
        }
        top = bar.y1;
    }

    let bottom = height.max(top);

    // Sidebar: four view buttons over the panel list. The contact sheet
    // covers the whole content area.
    let mut left = 0.0;
    if input.sidebar_open {
        out.contact_sheet = input.sidebar_tab == SHEET_TAB;
        let side_w = if out.contact_sheet { width } else { (input.sidebar_width * s).min(width * 0.5) };
        let side = Rect { x0: 0.0, y0: top, x1: side_w, y1: bottom };
        out.sidebar = Some(side);
        if !out.contact_sheet {
            out.sidebar_edge = Some(Rect { x0: side_w - 3.0 * s, x1: side_w + 3.0 * s, ..side });
        }
        let tab = SIDEBAR_TAB * s;
        let tabs_w = SIDEBAR_TABS.len() as f32 * (tab + size::BAR_GAP * s) - size::BAR_GAP * s;
        let tabs_x = ((side_w - tabs_w) / 2.0).max(PAD * s);
        for (index, label) in SIDEBAR_TABS.iter().enumerate() {
            let rect = Rect::new(tabs_x + index as f32 * (tab + size::BAR_GAP * s), top + PAD * s, tab, tab);
            let mut view = plain_widget(WidgetId::SidebarTab(index), Role::SidebarTab, Region::Sidebar, rect, (*label).into());
            view.glyph = info(commands::SIDEBAR_MODES[index]).glyph;
            view.tooltip = (*label).into();
            view.checked = Some(input.sidebar_tab == index);
            w.push(view);
        }
        let panel = Rect { x0: PAD * s, y0: top + PAD * s + tab + PAD * s, x1: side_w - PAD * s, y1: bottom };
        out.sidebar_panel = panel;
        let list = &input.sidebar_list;
        let (rows, total) = sidebar_rows(list, panel, input.sidebar_scroll, s, ts);
        out.sidebar_content = total;
        let active = if rows.iter().any(|(i, _)| *i == input.sidebar_active) { Some(input.sidebar_active) } else { rows.first().map(|r| r.0) };
        for (index, row) in rows {
            // Clipped to the panel, so a half-hidden row never takes clicks
            // meant for the sidebar view buttons.
            let rect = Rect { y0: row.y0.max(panel.y0), y1: row.y1.min(panel.y1), ..row };
            let mut item = plain_widget(WidgetId::SidebarItem(index), Role::ListItem, Region::Sidebar, rect, list.label(index));
            item.focusable = active == Some(index);
            match list {
                SidebarList::Thumbnails(_) | SidebarList::Sheet(_) => item.checked = Some(index == input.sidebar_active),
                SidebarList::Contents(items) => item.enabled = items[index].page.is_some(),
                _ => {}
            }
            w.push(item);
        }
        if !out.contact_sheet {
            left = side_w;
        }
    }

    out.document = Rect { x0: left, y0: top, x1: width.max(left), y1: bottom };
    out.info_area = out.document;
    let doc = out.document;
    if input.has_document && !out.contact_sheet {
        let mut view = plain_widget(WidgetId::Document, Role::Document, Region::Document, doc, input.title.into());
        view.tooltip.clear();
        w.push(view);
    } else if !input.has_document {
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
        let fields_h: f32 = sheet.fields.len() as f32 * (label_h + 4.0 * s + control + 12.0 * s)
            + sheet.controls.len() as f32 * (control + 12.0 * s);
        let card_h = pad + title_line + 12.0 * s + message_h + if message_h > 0.0 { 16.0 * s } else { 0.0 } + fields_h + 12.0 * s + control + pad;
        let card = Rect::new((width - card_w) / 2.0, ((height - card_h) / 2.0).max(out.title_bar.y1), card_w, card_h);
        let mut y = card.y0 + pad;
        let title = Rect::new(card.x0 + pad, y, inner, title_line);
        y += title_line + 12.0 * s;
        let message = Rect::new(card.x0 + pad, y, inner, message_h);
        y += message_h + if message_h > 0.0 { 16.0 * s } else { 0.0 };
        let mut layout = SheetLayout { card, title, message, ..Default::default() };
        for (index, c) in sheet.controls.iter().enumerate() {
            let rect = Rect::new(card.x0 + pad, y, inner, control);
            y += control + 12.0 * s;
            let role = if c.kind == super::sheet::Kind::Slider { Role::Slider } else { Role::Button };
            let mut widget = plain_widget(WidgetId::SheetControl(index), role, Region::Sheet, rect, c.text());
            widget.enabled = c.enabled;
            if c.kind == super::sheet::Kind::Toggle {
                widget.checked = Some(c.value == 1);
            }
            w.push(widget);
        }
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
            chrome: true,
            has_document: !tabs.is_empty(),
            title: "a.pdf",
            sidebar_open: false,
            sidebar_width: size::SIDEBAR,
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
    fn one_bar_holds_the_title_and_tools_in_preview_order() {
        let tabs = vec!["a.pdf".to_string()];
        let layout = layout(&input(1100.0, &tabs));
        assert_eq!(layout.title_bar.height(), 48.0);
        assert!(layout.tab_strip.is_none(), "one document has no tab strip");
        let mut expected = vec![cmd(Command::ToggleSidebar)];
        expected.extend(commands::TOOLBAR.iter().map(|c| cmd(*c)));
        expected.push(cmd(Command::AppMenu));
        assert_eq!(ids(&layout, Region::Toolbar), expected);
        assert!(layout.toolbar_overflow.is_empty());
        let rects: Vec<Rect> = layout.widgets.iter().filter(|w| w.region == Region::Toolbar).map(|w| w.rect).collect();
        assert!(rects.windows(2).all(|p| p[0].x1 <= p[1].x0), "buttons overlap");
        assert!(rects.iter().all(|r| r.width() == 36.0));
        let caption = layout.widgets.iter().find(|w| w.id == WidgetId::Minimize).unwrap().rect;
        assert!(rects.last().unwrap().x1 <= caption.x0);
        assert!(layout.title_text.x0 >= rects[0].x1 && layout.title_text.x1 <= rects[1].x0);
    }

    #[test]
    fn narrow_bar_overflows_share_first_into_the_menu() {
        let tabs = vec!["a.pdf".to_string()];
        let narrow = layout(&input(700.0, &tabs));
        assert_eq!(narrow.toolbar_overflow.first(), Some(&Command::Share));
        assert!(ids(&narrow, Region::Toolbar).contains(&cmd(Command::AppMenu)));
        let tiny = layout(&input(60.0, &tabs));
        assert_eq!(tiny.toolbar_overflow.len(), commands::TOOLBAR.len());
    }

    #[test]
    fn tab_strip_shows_only_with_two_documents() {
        let tabs = vec!["a.pdf".to_string(), "b.png".to_string()];
        let two = layout(&input(1100.0, &tabs));
        let strip = two.tab_strip.unwrap();
        assert_eq!(strip.y0, two.title_bar.y1);
        assert_eq!(two.document.y0, strip.y1);
        assert_eq!(two.widgets.iter().filter(|w| w.role == Role::Tab).count(), 2);
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
        assert_eq!(bar.y0, base.title_bar.y1);
        assert_eq!(bar.height(), 40.0);
        assert_eq!(open.document.y0, bar.y1);
        assert_eq!(open.document.x0, size::SIDEBAR);
        assert_eq!(ids(&open, Region::Sidebar).len(), SIDEBAR_TABS.len());
        let pdf_tools = commands::MARKUP_TOOLS.iter().filter(|c| commands::for_type(**c, true)).count();
        assert_eq!(ids(&open, Region::MarkupBar).len(), pdf_tools);
        assert!(open.document.height() > 0.0 && open.document.y1 == 800.0, "no status bar");
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
        assert_eq!(half.markup_bar.unwrap().height(), 20.0);
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
        assert!(last_tab.rect.x1 <= 1000.0 && last_tab.rect.y0 >= layout.title_bar.y1);
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
        assert_eq!(normal.title_bar.height(), 48.0);
        assert!(big.title_bar.height() > normal.title_bar.height());
        let button = |l: &Layout| l.widgets.iter().find(|w| w.id == cmd(Command::Rotate)).unwrap().rect.width();
        assert_eq!(button(&normal), 36.0);
        assert_eq!(button(&big), button(&normal), "225% text keeps 36 DIP icon targets");
        assert!(big.title_bar.height() >= 26.0 * 2.25 + 12.0, "the bar grows only enough for two lines of large text");
        let tabs: Vec<_> = big.widgets.iter().filter(|w| w.role == Role::SidebarTab).collect();
        assert!(tabs.iter().all(|t| t.glyph.is_some()), "icon view buttons fit at any text size");
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
        let tabs = vec!["a.pdf".to_string(), "b.pdf".to_string()];
        let mut i = input(1100.0, &tabs);
        let plain = layout(&i);
        let close_tab = plain.widgets.iter().find(|w| w.id == WidgetId::TabClose(0)).unwrap();
        let (x, y) = (close_tab.rect.x0 + 2.0, close_tab.rect.y0 + 2.0);
        assert_eq!(hit(&plain.widgets, x, y).unwrap().id, WidgetId::TabClose(0));
        assert_eq!(hit(&plain.widgets, 600.0, 400.0).unwrap().id, WidgetId::Document);
        i.sheet = Some(SheetView { message_height: 0.0, fields: vec!["Text"], buttons: vec!["OK", "Cancel"], controls: &[] });
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
        let mut i = input(1100.0, &tabs);
        i.ctx.pending = true;
        let layout = layout(&i);
        let first = next_focus(&layout.widgets, None, false).unwrap();
        assert_eq!(first, cmd(Command::ToggleSidebar));
        assert!(layout.widgets.iter().any(|w| w.id == WidgetId::Tab(0) && w.focusable));
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
        i.sheet = Some(SheetView { message_height: 20.0, fields: vec!["Width", "Height"], buttons: vec!["OK", "Cancel"], controls: &[] });
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
        let start = cmd(Command::ToggleSidebar);
        let mut current = Some(start);
        let mut regions = vec![];
        for _ in 0..4 {
            current = next_region(&layout.widgets, current, false);
            regions.push(layout.widgets.iter().find(|w| Some(w.id) == current).unwrap().region);
        }
        assert_eq!(regions, vec![Region::MarkupBar, Region::Sidebar, Region::Document, Region::Toolbar]);
        assert_eq!(next_region(&layout.widgets, Some(start), true), Some(WidgetId::Document));
    }

    #[test]
    fn access_keys_resolve_per_scope() {
        let tabs = vec!["a.pdf".to_string()];
        let mut i = input(1100.0, &tabs);
        i.markup = 1.0;
        let layout = layout(&i);
        assert_eq!(access_key_target(&layout.widgets, Scope::Root, 'm'), Some(cmd(Command::ToggleMarkup)));
        assert_eq!(access_key_target(&layout.widgets, Scope::Root, 'O'), Some(cmd(Command::AppMenu)));
        assert_eq!(access_key_target(&layout.widgets, Scope::Markup, 'H'), Some(cmd(Command::HighlightMenu)));
        assert_eq!(access_key_target(&layout.widgets, Scope::Root, 'H'), Some(cmd(Command::Share)));
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
        let row = sidebar_row(&i.sidebar_list, 1, l.sidebar_panel.width(), 1.0, 1.0).0;
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
    fn contact_sheet_fills_the_window_with_a_grid_and_reveal_snaps_to_rows() {
        let tabs = vec!["a.pdf".to_string()];
        let sizes = vec![[612.0, 792.0]; 40];
        let mut i = input(1100.0, &tabs);
        i.sidebar_open = true;
        i.sidebar_tab = SHEET_TAB;
        i.sidebar_list = SidebarList::Sheet(&sizes);
        let l = layout(&i);
        assert!(l.contact_sheet && l.sidebar.unwrap().width() == 1100.0);
        assert!(!l.widgets.iter().any(|w| w.id == WidgetId::Document), "the sheet covers the page view");
        let (columns, _, h) = sheet_grid(l.sidebar_panel.width(), 1.0, 1.0);
        let cells: Vec<&Widget> = l.widgets.iter().filter(|w| w.role == Role::ListItem).collect();
        assert!(columns > 1 && cells[1].rect.y0 == cells[0].rect.y0, "pages sit side by side");
        assert_eq!(l.sidebar_content, (40usize.div_ceil(columns)) as f32 * h);
        let list = SidebarList::Thumbnails(&sizes);
        let row = sidebar_row(&list, 1, 200.0, 1.0, 1.0).0;
        assert_eq!(row_start_after(&list, row * 2.0 - 30.0, 200.0, 1.0, 1.0), row * 2.0, "never a half row at the top");
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
