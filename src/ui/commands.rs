//! Every user command, its label, icon, shortcut, access key, and when it is
//! available. The app menu, context menus, toolbar, markup bar, keyboard,
//! and UI Automation all read this one table.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Command {
    Open,
    NewFromClipboard,
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
    FlipVertical,
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
    SidebarHide,
    SidebarThumbnails,
    SidebarContents,
    SidebarNotes,
    SidebarSheet,
    FullScreen,
    ToggleMarkup,
    NextTab,
    PreviousTab,
    /// Tabs 1 to 8.
    Tab(u8),
    NextPane,
    PreviousPane,
    /// Turns the markup tool off, so a drag selects text again.
    SelectText,
    /// Markup bar buttons that open a menu of related tools.
    ShapesMenu,
    HighlightMenu,
    SignMenu,
    Draw,
    Highlight,
    Underline,
    Strikethrough,
    Note,
    TextBox,
    Rectangle,
    Ellipse,
    Arrow,
    Line,
    RoundedRectangle,
    SpeechBubble,
    Star,
    Polygon,
    Magnifier,
    DimOutside,
    /// Image selections. Crop keeps the selected area; Delete clears it.
    SelectRectangle,
    SelectEllipse,
    /// Markup bar menus that set the look of the selected mark and the next ones.
    LineWidthMenu,
    BorderColorMenu,
    FillColorMenu,
    TextStyleMenu,
    SaveSignature,
    PlaceSignature,
    FillForm,
    AppMenu,
    MoreTools,
    /// Quick view becomes the editor in the same window.
    OpenInEditor,
    IndexSheet,
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
    pub(in crate::ui) const ZOOM_IN: u16 = 0xE8A3;
    pub(in crate::ui) const ZOOM_OUT: u16 = 0xE71F;
    pub(in crate::ui) const INFO: u16 = 0xE946;
    pub(in crate::ui) const PAGE: u16 = 0xE7C3;
    pub(in crate::ui) const LIST: u16 = 0xE8FD;
    pub(in crate::ui) const GRID: u16 = 0xE8A9;
    pub(in crate::ui) const CHARACTERS: u16 = 0xE8C1;
    pub(in crate::ui) const SELECT_ALL: u16 = 0xE8B3;
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
    pub(in crate::ui) const FLIP: u16 = 0xE8AB;
    pub(in crate::ui) const FOLDER_OPEN: u16 = 0xE838;
    pub(in crate::ui) const PRINT: u16 = 0xE749;
    pub(in crate::ui) const CHECK: u16 = 0xE73E;
    pub(in crate::ui) const CHEVRON_RIGHT: u16 = 0xE76C;
    pub(in crate::ui) const CHEVRON_DOWN: u16 = 0xE70D;
    pub(in crate::ui) const MINIMIZE: u16 = 0xE921;
    pub(in crate::ui) const MAXIMIZE: u16 = 0xE922;
    pub(in crate::ui) const RESTORE: u16 = 0xE923;
    pub(in crate::ui) const CLOSE: u16 = 0xE8BB;
    pub(in crate::ui) const ADD: u16 = 0xE710;
    pub(in crate::ui) const CANCEL: u16 = 0xE711;
    pub(in crate::ui) const FULL_SCREEN: u16 = 0xE740;
    pub(in crate::ui) const VIEW_ALL: u16 = 0xE8A9;
    pub(in crate::ui) const SELECT_RECTANGLE: u16 = 0xF407;
    pub(in crate::ui) const LINES: u16 = 0xE700;
    pub(in crate::ui) const FILLED_SQUARE: u16 = 0xE73B;
    pub(in crate::ui) const FONT: u16 = 0xE8D2;
    #[cfg(test)]
    pub(in crate::ui) const ALL: &[u16] = &[
        OPEN_PANE, ZOOM_IN, ZOOM_OUT, INFO, PAGE, LIST, GRID, CHARACTERS, SELECT_ALL, EDIT, ROTATE, SHARE, SEARCH, MORE,
        HIGHLIGHT, UNDERLINE, STRIKETHROUGH, COMMENT, INKING, CROP, RESIZE, SIGNATURE, FONT_SIZE, SQUARE, CIRCLE, ARROW, SAVE,
        DOCUMENT, FLIP,
        FOLDER_OPEN, PRINT, CHECK, CHEVRON_RIGHT, CHEVRON_DOWN, MINIMIZE, MAXIMIZE, RESTORE, CLOSE, ADD, CANCEL,
        FULL_SCREEN, VIEW_ALL,
    ];
}

pub(super) fn info(command: Command) -> Info {
    use glyph::*;
    match command {
        Open => i("Open", "&Open...", Some(FOLDER_OPEN), Some('P')),
        NewFromClipboard => i("New from clipboard", "New from clip&board", None, None),
        SaveCopy => i("Save a copy", "Save a &copy...", Some(SAVE), None),
        ExtractPage => i("Extract this page", "Extract this pa&ge...", None, None),
        Combine => i("Combine with another PDF", "Co&mbine with another PDF...", None, None),
        Print => i("Print", "&Print...", Some(PRINT), None),
        BatchFolder => i("Convert all images in this folder", "Convert &folder...", None, None),
        BatchSelected => i("Convert selected images", "Convert &selected images...", None, None),
        FileInfo => i("File information", "File &information", Some(INFO), Some('I')),
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
        FlipVertical => i("Flip vertically", "Flip verticall&y", None, None),
        Crop => i("Crop", "Cr&op", Some(CROP), Some('C')),
        Resize => i("Resize", "Re&size...", Some(RESIZE), Some('Z')),
        RemoveBackground => i("Remove background", "Remove &background...", Some(SELECT_ALL), Some('B')),
        DeletePage => i("Delete page", "&Delete page...", None, None),
        MovePage => i("Move page", "&Move page...", None, None),
        MovePageUp => i("Move page up", "Move page u&p", None, None),
        MovePageDown => i("Move page down", "Move page do&wn", None, None),
        InsertPage => i("Insert blank page", "&Insert blank page", None, None),
        InsertImagePage => i("Insert image as page", "Insert image as pag&e...", None, None),
        Previous => i("Previous", "&Previous", None, None),
        Next => i("Next", "&Next", None, None),
        ZoomIn => i("Zoom in", "Zoom &in", Some(ZOOM_IN), Some('Z')),
        ZoomOut => i("Zoom out", "Zoom &out", Some(ZOOM_OUT), Some('X')),
        Fit => i("Fit to window", "&Fit to window", None, None),
        FitWidth => i("Fit width", "Fit &width", None, None),
        ActualSize => i("Actual size", "Actual si&ze", None, None),
        ZoomToSelection => i("Zoom to selection", "Zoom to se&lection", None, None),
        ViewContinuous => i("Continuous scroll", "&Continuous scroll", None, None),
        ViewSingle => i("Single page", "Single pa&ge", None, None),
        ViewTwoPages => i("Two pages side by side", "Two pages side &by side", None, None),
        Slideshow => i("Slideshow", "Slide&show", None, None),
        ToggleSidebar => i("Sidebar", "Si&debar", Some(OPEN_PANE), Some('S')),
        SidebarHide => i("Hide sidebar", "&Hide sidebar", None, None),
        SidebarThumbnails => i("Thumbnails", "&Thumbnails", Some(PAGE), None),
        SidebarContents => i("Table of contents", "Table of &contents", Some(LIST), None),
        SidebarNotes => i("Highlights and notes", "Highlights and &notes", Some(COMMENT), None),
        SidebarSheet => i("Contact sheet", "Contact &sheet", Some(GRID), None),
        FullScreen => i("Full screen", "F&ull screen", Some(FULL_SCREEN), None),
        ToggleMarkup => i("Markup", "&Markup bar", Some(EDIT), Some('M')),
        NextTab => i("Next tab", "Next &tab", None, None),
        PreviousTab => i("Previous tab", "Previous t&ab", None, None),
        Tab(_) => i("Go to tab", "Go to tab", None, None),
        NextPane => i("Next pane", "Next pane", None, None),
        PreviousPane => i("Previous pane", "Previous pane", None, None),
        SelectText => i("Select text", "Select te&xt", Some(CHARACTERS), Some('X')),
        ShapesMenu => i("Shapes", "Sha&pes", Some(SQUARE), Some('P')),
        HighlightMenu => i("Highlight and strike", "&Highlight", Some(HIGHLIGHT), Some('H')),
        SignMenu => i("Sign", "Si&gn", Some(SIGNATURE), Some('G')),
        Draw => i("Draw", "&Draw", Some(INKING), Some('D')),
        Highlight => i("Highlight", "&Highlight", Some(HIGHLIGHT), Some('G')),
        Underline => i("Underline", "&Underline", Some(UNDERLINE), Some('U')),
        Strikethrough => i("Strikethrough", "Stri&kethrough", Some(STRIKETHROUGH), Some('K')),
        Note => i("Note", "&Note...", Some(COMMENT), Some('N')),
        TextBox => i("Text box", "&Text box", Some(FONT_SIZE), Some('T')),
        Rectangle => i("Rectangle", "&Rectangle", Some(SQUARE), Some('P')),
        Ellipse => i("Ellipse", "&Ellipse", Some(CIRCLE), Some('E')),
        Arrow => i("Arrow", "&Arrow", Some(ARROW), Some('A')),
        Line => i("Line", "&Line", None, None),
        RoundedRectangle => i("Rounded rectangle", "R&ounded rectangle", None, None),
        SpeechBubble => i("Speech bubble", "Speech &bubble", None, None),
        Star => i("Star", "S&tar", None, None),
        Polygon => i("Polygon", "&Polygon", None, None),
        Magnifier => i("Magnifier", "Ma&gnifier", None, None),
        DimOutside => i("Dim outside", "&Dim outside", None, None),
        SelectRectangle => i("Rectangular selection", "&Rectangular selection", Some(SELECT_RECTANGLE), Some('S')),
        SelectEllipse => i("Elliptical selection", "&Elliptical selection", Some(CIRCLE), Some('E')),
        LineWidthMenu => i("Line width", "&Line width", Some(LINES), Some('W')),
        BorderColorMenu => i("Border color", "&Border color", Some(SQUARE), Some('K')),
        FillColorMenu => i("Fill color", "F&ill color", Some(FILLED_SQUARE), Some('I')),
        TextStyleMenu => i("Text style", "Text st&yle", Some(FONT), Some('A')),
        SaveSignature => i("Create signature", "&Create signature", Some(ADD), Some('V')),
        PlaceSignature => i("Sign", "&Sign", Some(SIGNATURE), Some('G')),
        FillForm => i("Fill form", "Fill f&orm", Some(DOCUMENT), Some('F')),
        AppMenu => i("More", "More", Some(MORE), Some('O')),
        MoreTools => i("More tools", "More tools", Some(MORE), Some('O')),
        OpenInEditor => i("Open", "Open in &editor", None, None),
        IndexSheet => i("Index sheet", "&Index sheet", Some(VIEW_ALL), None),
    }
}

/// Editor bar buttons after the title, in Preview's order. Overflow hides
/// them in `TOOLBAR_DROP` order.
pub(super) const TOOLBAR: &[Command] = &[FileInfo, ZoomOut, ZoomIn, Share, Highlight, Rotate, ToggleMarkup, Find];
pub(super) const TOOLBAR_DROP: &[Command] = &[Share, Highlight, Rotate, FileInfo, ZoomOut, ZoomIn, ToggleMarkup, Find];
/// Markup bar tools in Preview's order. Commands for the other document
/// type are left out (`for_type`).
pub(super) const MARKUP_TOOLS: &[Command] = &[
    SelectText, SelectRectangle, SelectEllipse, RemoveBackground, Draw, ShapesMenu, TextBox, HighlightMenu, SignMenu, Note,
    Resize, Flip, Rotate, Crop, FillForm, LineWidthMenu, BorderColorMenu, FillColorMenu, TextStyleMenu,
];

/// Buttons that start a new group get a wider gap before them.
pub(super) fn starts_group(command: Command) -> bool {
    matches!(command, ZoomOut | Share | Highlight | Find | AppMenu | Draw | Note | Resize | LineWidthMenu)
}

/// The tools a menu button holds.
pub(super) fn tool_group(command: Command) -> &'static [Command] {
    match command {
        ShapesMenu => &[Line, Arrow, Rectangle, RoundedRectangle, Ellipse, SpeechBubble, Star, Polygon, DimOutside, Magnifier],
        HighlightMenu => &[Highlight, Underline, Strikethrough],
        SignMenu => &[PlaceSignature, SaveSignature],
        _ => &[],
    }
}

/// False for commands that never apply to this document type.
pub(super) fn for_type(command: Command, pdf: bool) -> bool {
    match command {
        Flip | Resize | RemoveBackground | SelectRectangle | SelectEllipse => !pdf,
        FillForm => pdf,
        _ => true,
    }
}

/// Menu buttons that set the look of marks (`marks::style_menu`).
pub(super) fn style_menu(command: Command) -> bool {
    matches!(command, LineWidthMenu | BorderColorMenu | FillColorMenu | TextStyleMenu)
}

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
        Line => K::Line,
        RoundedRectangle => K::RoundedRectangle,
        SpeechBubble => K::Bubble,
        Star => K::Star,
        Polygon => K::Polygon,
        Magnifier => K::Loupe,
        DimOutside => K::Mask,
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
const fn alt(key: u16) -> Chord {
    Chord { key, ctrl: false, shift: false, alt: true }
}

/// Preview's shortcuts with Cmd as Ctrl and Option as Alt
/// (docs/quicklook-spec.md section 4), then docs/research/ux-teardown.md.
/// Preview's Option-Cmd chords use Ctrl+Shift here: Ctrl+Alt is AltGr on
/// many layouts. The first chord per command is the one menus show.
pub(super) const SHORTCUTS: &[(Chord, Command)] = &[
    (ctrl_shift(0x31), SidebarHide),
    (ctrl_shift(0x32), SidebarThumbnails),
    (ctrl_shift(0x33), SidebarContents),
    (ctrl_shift(0x34), SidebarNotes),
    (ctrl_shift(0x35), SidebarSheet),
    (plain(0x7A), FullScreen),
    (ctrl(0x49), FileInfo),
    (alt(0x28), Next),
    (alt(0x26), Previous),
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
    (ctrl(0x30), ActualSize),
    (ctrl(0x60), ActualSize),
    (ctrl(0x39), Fit),
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
    (ctrl(0x4E), NewFromClipboard),
    (plain(0x0D), OpenInEditor),
    (ctrl(0x0D), IndexSheet),
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
    pub(super) sidebar_tab: usize,
    pub(super) markup_open: bool,
    pub(super) full_screen: bool,
    pub(super) crop: bool,
    pub(super) tool: Option<Command>,
    pub(super) zoom: super::view::Zoom,
    pub(super) view: super::view::ViewMode,
    pub(super) zoom_select: bool,
    pub(super) quick: bool,
    /// The open file has edits in its recipe.
    pub(super) can_undo: bool,
}

/// Sidebar modes in sidebar tab order.
pub(super) const SIDEBAR_MODES: [Command; 4] = [SidebarThumbnails, SidebarContents, SidebarNotes, SidebarSheet];

/// Whether `command` can run in the state `x`.
pub(super) fn enabled(command: Command, x: &Ctx) -> bool {
    let ready = x.has_frame && !x.pending && !x.failed;
    match command {
        Open | NewFromClipboard | Exit | ToggleMarkup | FullScreen | AppMenu | MoreTools | NextPane | PreviousPane | BatchSelected => true,
        OpenInEditor | IndexSheet => x.quick,
        ToggleSidebar | SidebarHide | SidebarThumbnails | SidebarContents | SidebarNotes | SidebarSheet => x.pdf,
        NextTab | PreviousTab => x.tabs > 1,
        Tab(_) | CloseTab => x.tabs > 0,
        Undo | Revert => x.has_frame && !x.pending && x.can_undo,
        _ if !ready => false,
        Flip | FlipVertical | Resize | RemoveBackground | BatchFolder | SelectRectangle | SelectEllipse | Magnifier => !x.pdf,
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
        SidebarHide => Some(!x.sidebar_open),
        SidebarThumbnails | SidebarContents | SidebarNotes | SidebarSheet => {
            Some(x.sidebar_open && SIDEBAR_MODES.get(x.sidebar_tab) == Some(&command))
        }
        FullScreen => Some(x.full_screen),
        ToggleMarkup => Some(x.markup_open),
        SelectText => Some(x.tool.is_none() && !x.crop),
        ShapesMenu | HighlightMenu | SignMenu => Some(x.tool.is_some_and(|tool| tool_group(command).contains(&tool))),
        Crop => Some(x.crop && x.tool.is_none()),
        ZoomToSelection => Some(x.zoom_select),
        Fit => Some(x.zoom == Zoom::Fit),
        FitWidth => Some(x.zoom == Zoom::FitWidth),
        ActualSize => Some(x.zoom == Zoom::Ratio(1.0)),
        ViewContinuous => Some(x.view == ViewMode::Continuous),
        ViewSingle => Some(x.view == ViewMode::Single),
        ViewTwoPages => Some(x.view == ViewMode::TwoPages),
        c if annotation(c).is_some() || matches!(c, PlaceSignature | SelectRectangle | SelectEllipse) => Some(x.tool == Some(c)),
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
    pub(super) preview: Option<Preview>,
}

/// A picture before a menu item's label.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Preview {
    /// A swatch of a `0xRRGGBBAA` color; alpha 0 shows "none".
    Color(u32),
    /// A line this many points wide.
    Width(f32),
    /// A saved signature: strokes in 0..1 of its box, and the box height over its width.
    Ink(Vec<Vec<[f32; 3]>>, f32),
}

impl MenuItem {
    pub(super) fn separator() -> Self {
        Self {
            label: String::new(),
            shortcut: String::new(),
            pick: None,
            enabled: false,
            checked: None,
            children: Vec::new(),
            preview: None,
        }
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
            preview: None,
        }
    }
    pub(super) fn for_command(command: Command, x: &Ctx) -> Self {
        Self {
            label: info(command).menu.into(),
            shortcut: shortcut_text(command).unwrap_or_default(),
            pick: Some(Pick::Command(command)),
            enabled: enabled(command, x),
            checked: checked(command, x),
            children: Vec::new(),
            preview: None,
        }
    }
    pub(super) fn submenu(label: &str, children: Vec<MenuItem>) -> Self {
        Self { label: label.into(), shortcut: String::new(), pick: None, enabled: true, checked: None, children, preview: None }
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
                Some(Open), Some(NewFromClipboard), Some(SaveCopy), Some(ExtractPage), Some(Combine), None, Some(Print), Some(Share), None,
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
                Some(FlipVertical), Some(Crop), Some(Resize), Some(RemoveBackground), None, Some(DeletePage), Some(MovePage),
                Some(MovePageUp), Some(MovePageDown), Some(InsertPage), Some(InsertImagePage),
            ],
            x,
        ),
    ));
    let mut view = items(
        &[
            Some(Previous), Some(Next), None, Some(ZoomIn), Some(ZoomOut), Some(ActualSize), Some(Fit),
            Some(FitWidth), Some(ZoomToSelection), None, Some(ViewContinuous), Some(ViewSingle), Some(ViewTwoPages), None,
        ],
        x,
    );
    view.push(MenuItem::submenu("Si&debar", items(&[Some(SidebarHide), None, Some(SidebarThumbnails), Some(SidebarContents), Some(SidebarNotes), Some(SidebarSheet)], x)));
    view.extend(items(&[Some(ToggleMarkup), Some(FullScreen), Some(Slideshow), None, Some(NextTab), Some(PreviousTab)], x));
    menu.push(MenuItem::submenu("&View", view));
    let mut markup = items(&[Some(SelectText), Some(SelectRectangle), Some(SelectEllipse), None, Some(Draw)], x);
    markup.push(MenuItem::submenu("Sha&pes", tool_menu(ShapesMenu, x)));
    markup.extend(items(
        &[
            Some(Highlight), Some(Underline), Some(Strikethrough), Some(Note), Some(TextBox), None, Some(LineWidthMenu),
            Some(BorderColorMenu), Some(FillColorMenu), Some(TextStyleMenu), None, Some(SaveSignature), Some(PlaceSignature),
            Some(FillForm),
        ],
        x,
    ));
    menu.push(MenuItem::submenu("Mar&kup", markup));
    menu
}

/// Right-click on the document. Only commands that apply to this file show.
pub(super) fn document_menu(x: &Ctx) -> Vec<MenuItem> {
    let list = [
        Some(CopyText), Some(Find), Some(FindNext), Some(FindPrevious), None, Some(Highlight), Some(Note), Some(TextBox), Some(PlaceSignature),
        Some(FillForm), None, Some(Rotate), Some(Crop), Some(Flip), Some(FlipVertical), Some(Resize), Some(RemoveBackground), None,
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

/// A markup bar menu button's tools.
pub(super) fn tool_menu(command: Command, x: &Ctx) -> Vec<MenuItem> {
    items(&tool_group(command).iter().copied().map(Some).collect::<Vec<_>>(), x)
}

/// The Sign button's menu: each saved signature, then Create signature.
pub(super) fn sign_menu(x: &Ctx, saved: &[super::files::Signature]) -> Vec<MenuItem> {
    let mut menu: Vec<MenuItem> = saved
        .iter()
        .enumerate()
        .map(|(index, s)| MenuItem {
            preview: Some(Preview::Ink(s.strokes.clone(), s.aspect)),
            enabled: enabled(PlaceSignature, x),
            ..MenuItem::choice(&format!("Signature {}", index + 1), index)
        })
        .collect();
    if !menu.is_empty() {
        menu.push(MenuItem::separator());
    }
    menu.extend(items(&[Some(SaveSignature)], x));
    menu
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
    overflow
        .iter()
        .map(|c| match tool_group(*c) {
            [] => MenuItem::for_command(*c, x),
            _ => MenuItem::submenu(info(*c).menu, tool_menu(*c, x)),
        })
        .collect()
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
    Open, NewFromClipboard, SaveCopy, ExtractPage, Combine, Print, BatchFolder, BatchSelected, FileInfo, Share, CloseTab, Exit, Undo,
    Revert, CopyText, Find, FindNext, FindPrevious, Rotate, Flip, Crop, Resize, RemoveBackground, DeletePage, MovePage, MovePageUp,
    MovePageDown, InsertPage, InsertImagePage, Previous, Next, ZoomIn, ZoomOut, Fit, FitWidth, ActualSize, ZoomToSelection, ViewContinuous, ViewSingle,
    ViewTwoPages, Slideshow, ToggleSidebar, SidebarHide, SidebarThumbnails, SidebarContents, SidebarNotes, SidebarSheet,
    FullScreen, ToggleMarkup, NextTab, PreviousTab, Tab(0), NextPane, PreviousPane, SelectText, ShapesMenu, HighlightMenu,
    SignMenu, Draw, Highlight, Underline, Strikethrough, Note, TextBox, Rectangle, Ellipse, Arrow, SaveSignature,
    PlaceSignature, FillForm, AppMenu, MoreTools, OpenInEditor, IndexSheet, FlipVertical, Line, RoundedRectangle,
    SpeechBubble, Star, Polygon, Magnifier, DimOutside, SelectRectangle, SelectEllipse, LineWidthMenu, BorderColorMenu,
    FillColorMenu, TextStyleMenu,
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
        assert_eq!(lookup(ctrl(0x39)), Some(Fit));
        assert_eq!(lookup(ctrl(0x30)), Some(ActualSize));
        assert_eq!(lookup(ctrl_shift(0x42)), Some(ToggleSidebar));
        assert_eq!(lookup(ctrl_shift(0x31)), Some(SidebarHide));
        assert_eq!(lookup(ctrl_shift(0x35)), Some(SidebarSheet));
        assert_eq!(lookup(alt(0x28)), Some(Next));
        assert_eq!(lookup(plain(0x25)), Some(Previous));
        assert_eq!(lookup(ctrl(0x41)), None);
        assert_eq!(lookup(plain(0x4E)), None, "typing a letter never picks a tool");
        assert_eq!(shortcut_text(SaveCopy).as_deref(), Some("Ctrl+Shift+S"));
        assert_eq!(shortcut_text(ZoomIn).as_deref(), Some("Ctrl+="));
        assert_eq!(shortcut_text(Slideshow).as_deref(), Some("F5"));
        assert_eq!(shortcut_text(FullScreen).as_deref(), Some("F11"));
        assert_eq!(shortcut_text(FileInfo).as_deref(), Some("Ctrl+I"));
        assert_eq!(lookup(ctrl(0xDC)), Some(FitWidth));
        assert_eq!(shortcut_text(FitWidth).as_deref(), Some("Ctrl+\\"));
    }

    #[test]
    fn tool_menus_mark_the_active_tool_and_its_button() {
        let pdf = Ctx { has_frame: true, pdf: true, tabs: 1, tool: Some(Ellipse), ..Default::default() };
        assert_eq!(flatten(&tool_menu(ShapesMenu, &pdf)), vec![Rectangle, Ellipse, Arrow]);
        assert!(tool_menu(ShapesMenu, &pdf).iter().any(|m| m.pick == Some(Pick::Command(Ellipse)) && m.checked == Some(true)));
        assert_eq!(checked(ShapesMenu, &pdf), Some(true));
        assert_eq!(checked(HighlightMenu, &pdf), Some(false));
        assert_eq!(checked(SelectText, &pdf), Some(false));
        let overflow = markup_overflow_menu(&[ShapesMenu, Crop], &pdf);
        assert_eq!(overflow[0].children.len(), 3, "a hidden menu button becomes a submenu");
    }

    #[test]
    fn app_menu_exposes_every_command() {
        let everything = Ctx { has_frame: true, tabs: 2, ..Default::default() };
        let listed = flatten(&app_menu(&everything, &[]));
        // Chrome commands open menus or move focus; tab numbers are keyboard-only.
        let not_listed =
            [AppMenu, MoreTools, ShapesMenu, HighlightMenu, SignMenu, ToggleSidebar, NextPane, PreviousPane, Tab(0), OpenInEditor, IndexSheet];
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
        let menus = app_menu(&x, &[]);
        let nested = menus.iter().flat_map(|m| m.children.iter().filter(|c| !c.children.is_empty()));
        for menu in menus.iter().chain(nested) {
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
        let failed = Ctx { failed: true, can_undo: true, ..pdf };
        assert!(!enabled(Rotate, &failed) && enabled(Undo, &failed));
        assert!(!enabled(Undo, &pdf) && !enabled(Revert, &pdf), "nothing to undo without edits");
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
