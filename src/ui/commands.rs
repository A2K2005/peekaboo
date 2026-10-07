//! Every user command, its label, icon, shortcut, access key, and when it is
//! available. The app menu, context menus, toolbar, markup bar, keyboard,
//! and UI Automation all read this one table.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Command {
    Open,
    SaveCopy,
    ExtractPage,
    Combine,
    Print,
    BatchFolder,
    BatchSelected,
    FileInfo,
    Share,
    CloseTab,
    Exit,
    Undo,
    Revert,
    CopyText,
    Find,
    FindNext,
    FindPrevious,
    Rotate,
    Flip,
    Crop,
    Resize,
    RemoveBackground,
    DeletePage,
    MovePage,
    MovePageUp,
    MovePageDown,
    InsertPage,
    InsertImagePage,
    Previous,
    Next,
    ZoomIn,
    ZoomOut,
    Fit,
    FitWidth,
    ActualSize,
    ZoomToSelection,
    ViewContinuous,
    ViewSingle,
    ViewTwoPages,
    Slideshow,
    ToggleSidebar,
    ToggleMarkup,
    NextTab,
    PreviousTab,
    /// Tabs 1 to 8; 8 means the last tab.
    Tab(u8),
    NextPane,
    PreviousPane,
    Draw,
    Highlight,
    Underline,
    Strikethrough,
    Note,
    TextBox,
    Rectangle,
    Ellipse,
    Arrow,
    SaveSignature,
    PlaceSignature,
    FillForm,
    ZoomMenu,
    AppMenu,
    MoreTools,
}
use Command::*;

pub(super) struct Info {
    /// Accessible name and tooltip text.
    pub(super) label: &'static str,
    /// Menu text. `&` marks the access key.
    pub(super) menu: &'static str,
    pub(super) glyph: Option<u16>,
    /// Access key on the toolbar (scope root) or markup bar (scope markup).
    pub(super) key: Option<char>,
}

const fn i(label: &'static str, menu: &'static str, glyph: Option<u16>, key: Option<char>) -> Info {
    Info { label, menu, glyph, key }
}

/// Glyphs exist in both Segoe MDL2 Assets and Segoe Fluent Icons.
/// https://learn.microsoft.com/windows/apps/design/style/segoe-ui-symbol-font
pub(super) mod glyph {
    pub(in crate::ui) const OPEN_PANE: u16 = 0xE8A0;
    /// ZoomIn, so the Zoom button does not look like Search.
    pub(in crate::ui) const ZOOM: u16 = 0xE8A3;
    pub(in crate::ui) const EDIT: u16 = 0xE70F;
    pub(in crate::ui) const ROTATE: u16 = 0xE7AD;
    pub(in crate::ui) const SHARE: u16 = 0xE72D;
    pub(in crate::ui) const SEARCH: u16 = 0xE721;
    pub(in crate::ui) const MORE: u16 = 0xE712;
    pub(in crate::ui) const HIGHLIGHT: u16 = 0xE7E6;
    pub(in crate::ui) const UNDERLINE: u16 = 0xE8DC;
    pub(in crate::ui) const STRIKETHROUGH: u16 = 0xEDE0;
    pub(in crate::ui) const COMMENT: u16 = 0xE90A;
    pub(in crate::ui) const INKING: u16 = 0xE76D;
    pub(in crate::ui) const CROP: u16 = 0xE7A8;
    pub(in crate::ui) const RESIZE: u16 = 0xE740;
    pub(in crate::ui) const SIGNATURE: u16 = 0xEF3F;
    pub(in crate::ui) const FONT_SIZE: u16 = 0xE8E9;
    pub(in crate::ui) const SQUARE: u16 = 0xE739;
    pub(in crate::ui) const CIRCLE: u16 = 0xEA3A;
    pub(in crate::ui) const ARROW: u16 = 0xE72A;
    pub(in crate::ui) const SAVE: u16 = 0xE74E;
    pub(in crate::ui) const DOCUMENT: u16 = 0xE8A5;
    pub(in crate::ui) const ERASE: u16 = 0xE75C;
    pub(in crate::ui) const FLIP: u16 = 0xE8AB;
    pub(in crate::ui) const FOLDER_OPEN: u16 = 0xE838;
    pub(in crate::ui) const PRINT: u16 = 0xE749;
    pub(in crate::ui) const CHECK: u16 = 0xE73E;
    pub(in crate::ui) const CHEVRON_RIGHT: u16 = 0xE76C;
    pub(in crate::ui) const MINIMIZE: u16 = 0xE921;
    pub(in crate::ui) const MAXIMIZE: u16 = 0xE922;
    pub(in crate::ui) const RESTORE: u16 = 0xE923;
    pub(in crate::ui) const CLOSE: u16 = 0xE8BB;
    pub(in crate::ui) const ADD: u16 = 0xE710;
    pub(in crate::ui) const CANCEL: u16 = 0xE711;
    #[cfg(test)]
    pub(in crate::ui) const ALL: &[u16] = &[
        OPEN_PANE, ZOOM, EDIT, ROTATE, SHARE, SEARCH, MORE, HIGHLIGHT, UNDERLINE, STRIKETHROUGH, COMMENT,
        INKING, CROP, RESIZE, SIGNATURE, FONT_SIZE, SQUARE, CIRCLE, ARROW, SAVE, DOCUMENT, ERASE, FLIP,
        FOLDER_OPEN, PRINT, CHECK, CHEVRON_RIGHT, MINIMIZE, MAXIMIZE, RESTORE, CLOSE, ADD, CANCEL,
    ];
}

pub(super) fn info(command: Command) -> Info {
    use glyph::*;
    match command {
        Open => i("Open", "&Open...", Some(FOLDER_OPEN), Some('P')),
        SaveCopy => i("Save a copy", "Save a &copy...", Some(SAVE), None),
        ExtractPage => i("Extract this page", "Extract this pa&ge...", None, None),
        Combine => i("Combine with another PDF", "Co&mbine with another PDF...", None, None),
        Print => i("Print", "&Print...", Some(PRINT), None),
        BatchFolder => i("Convert all images in this folder", "Convert &folder...", None, None),
        BatchSelected => i("Convert selected images", "Convert &selected images...", None, None),
        FileInfo => i("File information", "File &information", None, None),
        Share => i("Share", "S&hare", Some(SHARE), Some('H')),
        CloseTab => i("Close tab", "Close &tab", Some(CANCEL), None),
        Exit => i("Exit", "E&xit", None, None),
        Undo => i("Undo", "&Undo", None, None),
        Revert => i("Revert to opened", "&Revert to opened", None, None),
        CopyText => i("Copy text", "&Copy text", None, None),
        Find => i("Search", "&Find...", Some(SEARCH), Some('F')),
        FindNext => i("Next search result", "Find &next", None, None),
        FindPrevious => i("Previous search result", "Find pre&vious", None, None),
        Rotate => i("Rotate right", "Rotate righ&t", Some(ROTATE), Some('R')),
        Flip => i("Flip horizontally", "F&lip horizontally", Some(FLIP), Some('L')),
        Crop => i("Crop", "Cr&op", Some(CROP), Some('C')),
        Resize => i("Resize", "Re&size...", Some(RESIZE), Some('Z')),
        RemoveBackground => i("Remove background", "Remove &background...", Some(ERASE), Some('B')),
        DeletePage => i("Delete page", "&Delete page...", None, None),
        MovePage => i("Move page", "&Move page...", None, None),
        MovePageUp => i("Move page up", "Move page u&p", None, None),
        MovePageDown => i("Move page down", "Move page do&wn", None, None),
        InsertPage => i("Insert blank page", "&Insert blank page", None, None),
        InsertImagePage => i("Insert image as page", "Insert image as pag&e...", None, None),
        Previous => i("Previous", "&Previous", None, None),
        Next => i("Next", "&Next", None, None),
        ZoomIn => i("Zoom in", "Zoom &in", None, None),
        ZoomOut => i("Zoom out", "Zoom &out", None, None),
        Fit => i("Fit to window", "&Fit to window", None, None),
        FitWidth => i("Fit width", "Fit &width", None, None),
        ActualSize => i("Actual size", "Actual si&ze", None, None),
        ZoomToSelection => i("Zoom to selection", "Zoom to se&lection", None, None),
        ViewContinuous => i("Continuous scroll", "&Continuous scroll", None, None),
        ViewSingle => i("Single page", "Single pa&ge", None, None),
        ViewTwoPages => i("Two pages side by side", "Two pages side &by side", None, None),
        Slideshow => i("Slideshow", "Slide&show", None, None),
        ToggleSidebar => i("Sidebar", "Si&debar", Some(OPEN_PANE), Some('S')),
        ToggleMarkup => i("Markup", "&Markup bar", Some(EDIT), Some('M')),
        NextTab => i("Next tab", "Next &tab", None, None),
        PreviousTab => i("Previous tab", "Previous t&ab", None, None),
        Tab(_) => i("Go to tab", "Go to tab", None, None),
        NextPane => i("Next pane", "Next pane", None, None),
        PreviousPane => i("Previous pane", "Previous pane", None, None),
        Draw => i("Draw", "&Draw", Some(INKING), Some('D')),
        Highlight => i("Highlight", "&Highlight", Some(HIGHLIGHT), Some('H')),
        Underline => i("Underline", "&Underline", Some(UNDERLINE), Some('U')),
        Strikethrough => i("Strikethrough", "Stri&kethrough", Some(STRIKETHROUGH), Some('K')),
        Note => i("Note", "&Note...", Some(COMMENT), Some('N')),
        TextBox => i("Text box", "&Text box...", Some(FONT_SIZE), Some('T')),
        Rectangle => i("Rectangle", "&Rectangle", Some(SQUARE), Some('P')),
        Ellipse => i("Ellipse", "&Ellipse", Some(CIRCLE), Some('E')),
        Arrow => i("Arrow", "&Arrow", Some(ARROW), Some('A')),
        SaveSignature => i("Save drawing as signature", "Sa&ve drawing as signature", Some(SAVE), Some('V')),
        PlaceSignature => i("Sign", "Place &signature", Some(SIGNATURE), Some('G')),
        FillForm => i("Fill a form field", "Fill a f&orm field...", Some(DOCUMENT), Some('F')),
        ZoomMenu => i("Zoom", "&Zoom", Some(ZOOM), Some('Z')),
        AppMenu => i("More", "More", Some(MORE), Some('O')),
        MoreTools => i("More tools", "More tools", Some(MORE), Some('O')),
    }
}

/// Toolbar buttons, PRD order. Overflow hides them in `TOOLBAR_DROP` order.
pub(super) const TOOLBAR: &[Command] = &[ZoomMenu, ToggleMarkup, Rotate, Share, Find];
/// Overflow order from the UX spec: Share first, then Rotate.
pub(super) const TOOLBAR_DROP: &[Command] = &[Share, Rotate, ZoomMenu, ToggleMarkup, Find];
pub(super) const MARKUP_TOOLS: &[Command] = &[
    Draw, Highlight, Underline, Strikethrough, Note, TextBox, Rectangle, Ellipse, Arrow, PlaceSignature,
    SaveSignature, FillForm, Crop, Resize, Flip, RemoveBackground,
];

pub(super) fn annotation(command: Command) -> Option<crate::model::AnnotationKind> {
    use crate::model::AnnotationKind as K;
    Some(match command {
        Draw => K::Ink,
        Highlight => K::Highlight,
        Underline => K::Underline,
        Strikethrough => K::Strikeout,
        Note => K::Note,
        TextBox => K::Text,
        Rectangle => K::Rectangle,
        Ellipse => K::Ellipse,
        Arrow => K::Arrow,
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Chord {
    pub(super) key: u16,
    pub(super) ctrl: bool,
    pub(super) shift: bool,
    pub(super) alt: bool,
}
const fn ctrl(key: u16) -> Chord {
    Chord { key, ctrl: true, shift: false, alt: false }
}
const fn ctrl_shift(key: u16) -> Chord {
    Chord { key, ctrl: true, shift: true, alt: false }
}
const fn plain(key: u16) -> Chord {
    Chord { key, ctrl: false, shift: false, alt: false }
}
const fn shift(key: u16) -> Chord {
    Chord { key, ctrl: false, shift: true, alt: false }
}

/// Keyboard shortcuts from docs/research/ux-teardown.md, "Consolidated
/// keyboard shortcuts". The first chord per command is the one menus show.
pub(super) const SHORTCUTS: &[(Chord, Command)] = &[
    (ctrl(0x4F), Open),
    (ctrl(0x54), Open),
    (ctrl(0x57), CloseTab),
    (ctrl(0x73), CloseTab),
    (ctrl(0x09), NextTab),
    (ctrl_shift(0x09), PreviousTab),
    (ctrl(0x31), Tab(0)),
    (ctrl(0x32), Tab(1)),
    (ctrl(0x33), Tab(2)),
    (ctrl(0x34), Tab(3)),
    (ctrl(0x35), Tab(4)),
    (ctrl(0x36), Tab(5)),
    (ctrl(0x37), Tab(6)),
    (ctrl(0x38), Tab(7)),
    (ctrl(0x39), Tab(8)),
    (ctrl_shift(0x53), SaveCopy),
    (ctrl(0x53), SaveCopy),
    (ctrl(0x50), Print),
    (ctrl(0x46), Find),
    (plain(0x72), FindNext),
    (shift(0x72), FindPrevious),
    (ctrl(0x43), CopyText),
    (ctrl_shift(0x43), CopyText),
    (ctrl(0x5A), Undo),
    (ctrl(0xBB), ZoomIn),
    (ctrl(0x6B), ZoomIn),
    (ctrl(0xBD), ZoomOut),
    (ctrl(0x6D), ZoomOut),
    (ctrl(0x30), Fit),
    (ctrl(0x60), Fit),
    (ctrl(0xDC), FitWidth),
    (plain(0x74), Slideshow),
    (ctrl_shift(0x42), ToggleSidebar),
    (ctrl_shift(0x41), ToggleMarkup),
    (ctrl(0x45), ToggleMarkup),
    (plain(0x75), NextPane),
    (shift(0x75), PreviousPane),
    (ctrl_shift(0x48), Highlight),
    (ctrl(0x55), Underline),
    (ctrl(0x52), Rotate),
    (ctrl(0xDD), Rotate),
    (ctrl(0x4B), Crop),
    (ctrl_shift(0x52), Resize),
    (ctrl_shift(0x4B), RemoveBackground),
    (plain(0x2E), DeletePage),
    (ctrl_shift(0x4E), InsertPage),
    (ctrl_shift(0x26), MovePageUp),
    (ctrl_shift(0x28), MovePageDown),
    (Chord { key: 0x0D, ctrl: false, shift: false, alt: true }, FileInfo),
    (plain(0x25), Previous),
    (plain(0x21), Previous),
    (plain(0x27), Next),
    (plain(0x22), Next),
];

pub(super) fn lookup(chord: Chord) -> Option<Command> {
    SHORTCUTS.iter().find(|(c, _)| *c == chord).map(|(_, command)| *command)
}

pub(super) fn shortcut_text(command: Command) -> Option<String> {
    let (chord, _) = SHORTCUTS.iter().find(|(_, c)| *c == command)?;
    let key = match chord.key {
        0x09 => "Tab".to_string(),
        0x0D => "Enter".into(),
        0x21 => "Page Up".into(),
        0x22 => "Page Down".into(),
        0x25 => "Left".into(),
        0x26 => "Up".into(),
        0x27 => "Right".into(),
        0x28 => "Down".into(),
        0x2E => "Delete".into(),
        0x60 => "Num 0".into(),
        0x6B => "Num +".into(),
        0x6D => "Num -".into(),
        0x70..=0x7B => format!("F{}", chord.key - 0x6F),
        0xBB => "=".into(),
        0xBD => "-".into(),
        0xDC => "\\".into(),
        0xDD => "]".into(),
        key => char::from(key as u8).to_string(),
    };
    let mut text = String::new();
    for (on, name) in [(chord.ctrl, "Ctrl+"), (chord.shift, "Shift+"), (chord.alt, "Alt+")] {
        if on {
            text.push_str(name);
        }
    }
    Some(text + &key)
}

/// Snapshot of app state that decides which commands are available.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Ctx {
    pub(super) has_frame: bool,
    pub(super) pending: bool,
    pub(super) failed: bool,
    pub(super) pdf: bool,
    pub(super) saving: bool,
    pub(super) can_previous: bool,
    pub(super) can_next: bool,
    pub(super) tabs: usize,
    pub(super) sidebar_open: bool,
    pub(super) markup_open: bool,
    pub(super) crop: bool,
    pub(super) tool: Option<Command>,
    pub(super) zoom: super::view::Zoom,
    pub(super) view: super::view::ViewMode,
    pub(super) zoom_select: bool,
}

/// Rules match the pre-split shell (`update_controls`), plus the new UI commands.
pub(super) fn enabled(command: Command, x: &Ctx) -> bool {
    let ready = x.has_frame && !x.pending && !x.failed;
    match command {
        Open | Exit | ToggleSidebar | ToggleMarkup | AppMenu | MoreTools | NextPane | PreviousPane => true,
        NextTab | PreviousTab => x.tabs > 1,
        Tab(_) | CloseTab => x.tabs > 0,
        Undo | Revert => x.has_frame && !x.pending,
        // The Windows share sheet arrives with W1-D in wave 2.
        Share => false,
        _ if !ready => false,
        Flip | Resize | RemoveBackground | BatchFolder | BatchSelected => !x.pdf,
        ExtractPage | Combine | DeletePage | FillForm | MovePage | InsertPage | InsertImagePage | ViewContinuous | ViewSingle | ViewTwoPages => x.pdf,
        MovePageUp => x.pdf && x.can_previous,
        MovePageDown => x.pdf && x.can_next,
        Previous => x.can_previous,
        Next => x.can_next,
        SaveCopy | Print => !x.saving,
        _ => true,
    }
}

pub(super) fn checked(command: Command, x: &Ctx) -> Option<bool> {
    use super::view::{ViewMode, Zoom};
    match command {
        ToggleSidebar => Some(x.sidebar_open),
        ToggleMarkup => Some(x.markup_open),
        Crop => Some(x.crop),
        ZoomToSelection => Some(x.zoom_select),
        Fit => Some(x.zoom == Zoom::Fit),
        FitWidth => Some(x.zoom == Zoom::FitWidth),
        ActualSize => Some(x.zoom == Zoom::Ratio(1.0)),
        ViewContinuous => Some(x.view == ViewMode::Continuous),
        ViewSingle => Some(x.view == ViewMode::Single),
        ViewTwoPages => Some(x.view == ViewMode::TwoPages),
        c if annotation(c).is_some() || c == PlaceSignature => Some(x.tool == Some(c)),
        _ => None,
    }
}

/// What a menu item returns: a command, or the index of a choice list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Pick {
    Command(Command),
    Index(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct MenuItem {
    pub(super) label: String,
    pub(super) shortcut: String,
    pub(super) pick: Option<Pick>,
    pub(super) enabled: bool,
    pub(super) checked: Option<bool>,
    pub(super) children: Vec<MenuItem>,
}

impl MenuItem {
    pub(super) fn separator() -> Self {
        Self { label: String::new(), shortcut: String::new(), pick: None, enabled: false, checked: None, children: Vec::new() }
    }
    pub(super) fn is_separator(&self) -> bool {
        self.pick.is_none() && self.children.is_empty()
    }
    /// A plain choice. `label` is shown as is, so `&` is doubled.
    pub(super) fn choice(label: &str, index: usize) -> Self {
        Self {
            label: label.replace('&', "&&"),
            shortcut: String::new(),
            pick: Some(Pick::Index(index)),
            enabled: true,
            checked: None,
            children: Vec::new(),
        }
    }
    fn for_command(command: Command, x: &Ctx) -> Self {
        Self {
            label: info(command).menu.into(),
            shortcut: shortcut_text(command).unwrap_or_default(),
            pick: Some(Pick::Command(command)),
            enabled: enabled(command, x),
            checked: checked(command, x),
            children: Vec::new(),
        }
    }
    fn submenu(label: &str, children: Vec<MenuItem>) -> Self {
        Self { label: label.into(), shortcut: String::new(), pick: None, enabled: true, checked: None, children }
    }
}

/// `None` in a list is a separator.
fn items(list: &[Option<Command>], x: &Ctx) -> Vec<MenuItem> {
    list.iter()
        .map(|c| c.map_or_else(MenuItem::separator, |c| MenuItem::for_command(c, x)))
        .collect()
}

/// The "More" menu exposes every command. Toolbar buttons hidden by
/// overflow come first.
pub(super) fn app_menu(x: &Ctx, overflow: &[Command]) -> Vec<MenuItem> {
    let mut menu = items(&overflow.iter().copied().map(Some).collect::<Vec<_>>(), x);
    if !menu.is_empty() {
        menu.push(MenuItem::separator());
    }
    menu.push(MenuItem::submenu(
        "&File",
        items(
            &[
                Some(Open), Some(SaveCopy), Some(ExtractPage), Some(Combine), None, Some(Print), Some(Share), None,
                Some(BatchFolder), Some(BatchSelected), None, Some(FileInfo), Some(CloseTab), Some(Exit),
            ],
            x,
        ),
    ));
    menu.push(MenuItem::submenu(
        "&Edit",
        items(
            &[
                Some(Undo), Some(Revert), None, Some(CopyText), Some(Find), Some(FindNext), Some(FindPrevious), None, Some(Rotate), Some(Flip),
                Some(Crop), Some(Resize), Some(RemoveBackground), None, Some(DeletePage), Some(MovePage),
                Some(MovePageUp), Some(MovePageDown), Some(InsertPage), Some(InsertImagePage),
            ],
            x,
        ),
    ));
    menu.push(MenuItem::submenu(
        "&View",
        items(
            &[
                Some(Previous), Some(Next), None, Some(ZoomIn), Some(ZoomOut), Some(ActualSize), Some(Fit),
                Some(FitWidth), Some(ZoomToSelection), None, Some(ViewContinuous), Some(ViewSingle), Some(ViewTwoPages),
                None, Some(ToggleSidebar), Some(ToggleMarkup), Some(Slideshow), None, Some(NextTab), Some(PreviousTab),
            ],
            x,
        ),
    ));
    menu.push(MenuItem::submenu(
        "Mar&kup",
        items(
            &[
                Some(Draw), Some(Highlight), Some(Underline), Some(Strikethrough), Some(Note), Some(TextBox),
                Some(Rectangle), Some(Ellipse), Some(Arrow), None, Some(SaveSignature), Some(PlaceSignature),
                Some(FillForm),
            ],
            x,
        ),
    ));
    menu
}

/// Right-click on the document. Only commands that apply to this file show.
pub(super) fn document_menu(x: &Ctx) -> Vec<MenuItem> {
    let list = [
        Some(CopyText), Some(Find), Some(FindNext), Some(FindPrevious), None, Some(Highlight), Some(Note), Some(TextBox), Some(PlaceSignature),
        Some(FillForm), None, Some(Rotate), Some(Crop), Some(Flip), Some(Resize), Some(RemoveBackground), None,
        Some(DeletePage), Some(InsertPage), Some(MovePage), None, Some(ZoomIn), Some(ZoomOut), Some(Fit),
        Some(ZoomToSelection), None,
        Some(SaveCopy), Some(Print), Some(FileInfo),
    ];
    let mut menu: Vec<MenuItem> = Vec::new();
    for item in items(&list, x) {
        if item.is_separator() {
            if menu.last().is_some_and(|m| !m.is_separator()) {
                menu.push(item);
            }
        } else if item.enabled {
            menu.push(item);
        }
    }
    if menu.last().is_some_and(MenuItem::is_separator) {
        menu.pop();
    }
    menu
}

pub(super) fn zoom_menu(x: &Ctx) -> Vec<MenuItem> {
    let mut list = vec![Some(ZoomIn), Some(ZoomOut), None, Some(ActualSize), Some(Fit), Some(FitWidth), Some(ZoomToSelection)];
    if x.pdf {
        list.extend([None, Some(ViewContinuous), Some(ViewSingle), Some(ViewTwoPages)]);
    }
    items(&list, x)
}

/// Right-click on a page thumbnail.
pub(super) fn page_menu(x: &Ctx) -> Vec<MenuItem> {
    items(
        &[
            Some(MovePageUp), Some(MovePageDown), Some(MovePage), None, Some(Rotate), Some(DeletePage), None,
            Some(InsertPage), Some(InsertImagePage), None, Some(ExtractPage),
        ],
        x,
    )
}

pub(super) fn tab_menu(x: &Ctx) -> Vec<MenuItem> {
    items(&[Some(CloseTab), None, Some(NextTab), Some(PreviousTab)], x)
}

pub(super) fn markup_overflow_menu(overflow: &[Command], x: &Ctx) -> Vec<MenuItem> {
    items(&overflow.iter().copied().map(Some).collect::<Vec<_>>(), x)
}

/// The access key marked with `&`, upper-cased.
pub(super) fn access_key(label: &str) -> Option<char> {
    let mut chars = label.chars();
    while let Some(c) = chars.next() {
        if c == '&' {
            return chars.next().map(|c| c.to_ascii_uppercase());
        }
    }
    None
}

pub(super) const ALL: &[Command] = &[
    Open, SaveCopy, ExtractPage, Combine, Print, BatchFolder, BatchSelected, FileInfo, Share, CloseTab, Exit, Undo,
    Revert, CopyText, Find, FindNext, FindPrevious, Rotate, Flip, Crop, Resize, RemoveBackground, DeletePage, MovePage, MovePageUp,
    MovePageDown, InsertPage, InsertImagePage, Previous, Next, ZoomIn, ZoomOut, Fit, FitWidth, ActualSize, ZoomToSelection, ViewContinuous, ViewSingle,
    ViewTwoPages, Slideshow, ToggleSidebar, ToggleMarkup, NextTab, PreviousTab, Tab(0),
    NextPane, PreviousPane, Draw, Highlight, Underline, Strikethrough, Note, TextBox, Rectangle, Ellipse, Arrow,
    SaveSignature, PlaceSignature, FillForm, ZoomMenu, AppMenu, MoreTools,
];

#[cfg(test)]
mod tests {
    use super::*;

    fn flatten(menu: &[MenuItem]) -> Vec<Command> {
        let command = |m: &MenuItem| match m.pick {
            Some(Pick::Command(c)) => Some(c),
            _ => None,
        };
        menu.iter().flat_map(|m| command(m).into_iter().chain(flatten(&m.children))).collect()
    }

    #[test]
    fn shortcuts_are_unique_and_avoid_windows_reserved_keys() {
        for (index, (chord, _)) in SHORTCUTS.iter().enumerate() {
            assert!(!SHORTCUTS[index + 1..].iter().any(|(c, _)| c == chord), "duplicate {chord:?}");
            assert!(!(chord.ctrl && chord.alt), "Ctrl+Alt acts as AltGr: {chord:?}");
            assert_ne!(chord.key, 0x79, "F10 belongs to Windows");
            assert!(!(chord.alt && matches!(chord.key, 0x73 | 0x20 | 0x09)), "{chord:?}");
            assert!(!(chord.ctrl && chord.key == 0x1B), "Ctrl+Esc belongs to Windows");
        }
    }

    #[test]
    fn shortcut_table_matches_the_ux_spec() {
        assert_eq!(lookup(ctrl_shift(0x41)), Some(ToggleMarkup));
        assert_eq!(lookup(ctrl(0x45)), Some(ToggleMarkup));
        assert_eq!(lookup(ctrl(0x57)), Some(CloseTab));
        assert_eq!(lookup(ctrl(0x09)), Some(NextTab));
        assert_eq!(lookup(ctrl(0x31)), Some(Tab(0)));
        assert_eq!(lookup(ctrl(0x38)), Some(Tab(7)));
        assert_eq!(lookup(ctrl(0x39)), Some(Tab(8)));
        assert_eq!(lookup(ctrl_shift(0x42)), Some(ToggleSidebar));
        assert_eq!(lookup(plain(0x25)), Some(Previous));
        assert_eq!(lookup(ctrl(0x41)), None);
        assert_eq!(shortcut_text(SaveCopy).as_deref(), Some("Ctrl+Shift+S"));
        assert_eq!(shortcut_text(ZoomIn).as_deref(), Some("Ctrl+="));
        assert_eq!(shortcut_text(Slideshow).as_deref(), Some("F5"));
        assert_eq!(shortcut_text(FileInfo).as_deref(), Some("Alt+Enter"));
        assert_eq!(lookup(ctrl(0xDC)), Some(FitWidth));
        assert_eq!(shortcut_text(FitWidth).as_deref(), Some("Ctrl+\\"));
    }

    #[test]
    fn zoom_menu_marks_the_current_zoom_and_view() {
        use super::super::view::{ViewMode, Zoom};
        let pdf = Ctx { has_frame: true, pdf: true, tabs: 1, zoom: Zoom::FitWidth, view: ViewMode::TwoPages, ..Default::default() };
        let menu = zoom_menu(&pdf);
        let checked: Vec<Command> = menu
            .iter()
            .filter(|m| m.checked == Some(true))
            .filter_map(|m| match m.pick {
                Some(Pick::Command(c)) => Some(c),
                _ => None,
            })
            .collect();
        assert_eq!(checked, vec![FitWidth, ViewTwoPages]);
        let image = Ctx { pdf: false, zoom: Zoom::Ratio(1.0), ..pdf };
        let menu = zoom_menu(&image);
        assert!(!flatten(&menu).contains(&ViewSingle), "view modes are for PDFs");
        assert!(menu.iter().any(|m| m.pick == Some(Pick::Command(ActualSize)) && m.checked == Some(true)));
        let keys: Vec<char> = zoom_menu(&pdf).iter().filter_map(|m| access_key(&m.label)).collect();
        for (n, key) in keys.iter().enumerate() {
            assert!(!keys[n + 1..].contains(key), "zoom menu reuses {key}");
        }
    }

    #[test]
    fn app_menu_exposes_every_command() {
        let everything = Ctx { has_frame: true, tabs: 2, ..Default::default() };
        let listed = flatten(&app_menu(&everything, &[]));
        // Chrome commands open menus or move focus; tab numbers are keyboard-only.
        let not_listed = [ZoomMenu, AppMenu, MoreTools, NextPane, PreviousPane, Tab(0)];
        for command in ALL {
            assert!(listed.contains(command) || not_listed.contains(command), "{command:?} missing");
        }
    }

    #[test]
    fn every_command_has_a_keyboard_path() {
        for command in ALL {
            let i = info(*command);
            let keyboard = shortcut_text(*command).is_some() || access_key(i.menu).is_some() || i.key.is_some();
            assert!(keyboard, "{command:?} has no shortcut or access key");
            assert!(!i.label.is_empty() && !i.label.contains('\u{2014}'));
        }
    }

    #[test]
    fn access_keys_are_unique_within_each_submenu() {
        let x = Ctx::default();
        for menu in app_menu(&x, &[]) {
            let keys: Vec<_> = menu.children.iter().filter_map(|m| access_key(&m.label)).collect();
            for (n, key) in keys.iter().enumerate() {
                assert!(!keys[n + 1..].contains(key), "{} reuses {key}", menu.label);
            }
        }
        let top: Vec<_> = app_menu(&x, &[]).iter().filter_map(|m| access_key(&m.label)).collect();
        assert_eq!(top, vec!['F', 'E', 'V', 'K']);
    }

    #[test]
    fn toolbar_and_markup_access_keys_do_not_collide_in_their_scope() {
        let root: Vec<_> = [ToggleSidebar, AppMenu].iter().chain(TOOLBAR).filter_map(|c| info(*c).key).collect();
        let markup: Vec<_> = MARKUP_TOOLS.iter().chain([&MoreTools]).filter_map(|c| info(*c).key).collect();
        for keys in [root, markup] {
            for (n, key) in keys.iter().enumerate() {
                assert!(!keys[n + 1..].contains(key), "{key} repeats");
            }
        }
    }

    #[test]
    fn availability_follows_document_type_and_state() {
        let pdf = Ctx { has_frame: true, pdf: true, tabs: 1, can_next: true, ..Default::default() };
        assert!(enabled(DeletePage, &pdf) && !enabled(Resize, &pdf) && enabled(Next, &pdf) && !enabled(Previous, &pdf));
        let image = Ctx { pdf: false, ..pdf };
        assert!(!enabled(DeletePage, &image) && enabled(Resize, &image));
        let busy = Ctx { pending: true, ..pdf };
        assert!(!enabled(Rotate, &busy) && !enabled(Undo, &busy) && enabled(Open, &busy));
        let failed = Ctx { failed: true, ..pdf };
        assert!(!enabled(Rotate, &failed) && enabled(Undo, &failed));
        assert!(!enabled(NextTab, &pdf) && enabled(CloseTab, &pdf));
        let empty = Ctx::default();
        assert!(enabled(Open, &empty) && !enabled(SaveCopy, &empty) && !enabled(CloseTab, &empty));
    }

    #[test]
    fn document_menu_lists_only_applicable_commands_without_stray_separators() {
        let image = Ctx { has_frame: true, tabs: 1, ..Default::default() };
        let menu = document_menu(&image);
        let commands = flatten(&menu);
        assert!(commands.contains(&Resize) && !commands.contains(&DeletePage));
        assert!(!menu.first().unwrap().is_separator() && !menu.last().unwrap().is_separator());
        assert!(menu.windows(2).all(|w| !(w[0].is_separator() && w[1].is_separator())));
        assert!(document_menu(&Ctx::default()).is_empty());
    }
}
