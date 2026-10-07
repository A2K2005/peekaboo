//! Space+drag pans the document, like the hand tool in other viewers. A
//! Space press with no drag still scrolls a screen, when Space comes up.
use super::{app::State, document::Phase, widgets::WidgetId};
use std::cell::Cell;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_SPACE};

thread_local! {
    /// Some while Space is held for the document; true once it panned.
    static HELD: Cell<Option<bool>> = const { Cell::new(None) };
    /// The pointer gesture in progress pans.
    static DRAG: Cell<bool> = const { Cell::new(false) };
}

/// Space went down. Returns true when the document takes it, including
/// auto-repeats of a hold.
pub(super) fn press(state: &State) -> bool {
    if HELD.get().is_some() {
        return true;
    }
    let document = matches!(state.focus, Some(WidgetId::Document) | None)
        && state.sheet.is_none()
        && (state.pdf.is_some() || state.frame.is_some());
    if document {
        HELD.set(Some(false));
    }
    document
}

/// Space came up. Returns true when the hold never panned, so the press
/// scrolls a screen.
pub(super) fn release() -> bool {
    HELD.take() == Some(false)
}

/// The key state is read again, so a release lost to another window never
/// leaves the hand tool on.
pub(super) fn held() -> bool {
    HELD.get().is_some() && unsafe { GetKeyState(VK_SPACE.0 as i32) } < 0
}

/// True while this pointer gesture pans. Decided when the button goes down.
pub(super) fn gesture(phase: Phase) -> bool {
    match phase {
        Phase::Down => {
            let pan = held();
            if pan {
                HELD.set(Some(true));
            }
            DRAG.set(pan);
            pan
        }
        Phase::Move => DRAG.get(),
        Phase::Up | Phase::Cancel => DRAG.replace(false),
    }
}
