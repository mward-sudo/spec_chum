//! Classify Library window `WM_COMMAND` notifications.
//!
//! Win32 delivers `BN_SETFOCUS` and `BN_KILLFOCUS` through the same
//! `WM_COMMAND` as `BN_CLICKED`. A click on an unfocused button sends focus
//! first. The Library remove handler rebuilds the list during that focus
//! notification, so treating focus as a click deletes a second recent entry.

/// Search edit. `EN_CHANGE` is `0x0300`.
pub const ID_LIBRARY_SEARCH: i32 = 3001;
/// Category combo. `CBN_SELCHANGE` is `1`.
pub const ID_LIBRARY_CATEGORY: i32 = 3002;
/// Recent-item list. `LBN_SELCHANGE` is `1`.
pub const ID_LIBRARY_LIST: i32 = 3003;
/// Details static text. It does not send action notifications.
pub const ID_LIBRARY_DETAILS: i32 = 3004;
pub const ID_LIBRARY_OPEN_BUTTON: i32 = 3005;
pub const ID_LIBRARY_REMOVE_BUTTON: i32 = 3006;
pub const ID_LIBRARY_CLOSE_BUTTON: i32 = 3007;

/// `BN_CLICKED` from `Winuser.h`.
const BN_CLICKED: usize = 0;
/// `EN_CHANGE`.
const EN_CHANGE: usize = 0x0300;
/// `CBN_SELCHANGE` and `LBN_SELCHANGE`.
const SELCHANGE: usize = 1;

/// Action the Library window should take for one control notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryCommand {
    /// Search text or category changed.
    Refresh,
    /// The list selection changed.
    SelectionChanged,
    Open,
    Remove,
    Close,
}

/// Map a Library control id and `WM_COMMAND` notification to an action.
///
/// Button focus notifications return [`None`] so they cannot open media or
/// remove a recent entry.
#[must_use]
pub fn library_command(control_id: i32, notification: usize) -> Option<LibraryCommand> {
    match (control_id, notification) {
        (ID_LIBRARY_SEARCH, EN_CHANGE) | (ID_LIBRARY_CATEGORY, SELCHANGE) => {
            Some(LibraryCommand::Refresh)
        }
        (ID_LIBRARY_LIST, SELCHANGE) => Some(LibraryCommand::SelectionChanged),
        (ID_LIBRARY_OPEN_BUTTON, BN_CLICKED) => Some(LibraryCommand::Open),
        (ID_LIBRARY_REMOVE_BUTTON, BN_CLICKED) => Some(LibraryCommand::Remove),
        (ID_LIBRARY_CLOSE_BUTTON, BN_CLICKED) => Some(LibraryCommand::Close),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `BN_SETFOCUS` / `BN_KILLFOCUS`.
    const BN_SETFOCUS: usize = 6;
    const BN_KILLFOCUS: usize = 7;

    #[test]
    fn button_focus_does_not_open_remove_or_close() {
        for notification in [BN_SETFOCUS, BN_KILLFOCUS] {
            assert_eq!(library_command(ID_LIBRARY_OPEN_BUTTON, notification), None);
            assert_eq!(
                library_command(ID_LIBRARY_REMOVE_BUTTON, notification),
                None
            );
            assert_eq!(library_command(ID_LIBRARY_CLOSE_BUTTON, notification), None);
        }
    }

    #[test]
    fn button_click_is_the_only_button_action() {
        assert_eq!(
            library_command(ID_LIBRARY_OPEN_BUTTON, BN_CLICKED),
            Some(LibraryCommand::Open)
        );
        assert_eq!(
            library_command(ID_LIBRARY_REMOVE_BUTTON, BN_CLICKED),
            Some(LibraryCommand::Remove)
        );
        assert_eq!(
            library_command(ID_LIBRARY_CLOSE_BUTTON, BN_CLICKED),
            Some(LibraryCommand::Close)
        );
    }

    #[test]
    fn search_category_and_selection_notifications_still_update() {
        assert_eq!(
            library_command(ID_LIBRARY_SEARCH, EN_CHANGE),
            Some(LibraryCommand::Refresh)
        );
        assert_eq!(
            library_command(ID_LIBRARY_CATEGORY, SELCHANGE),
            Some(LibraryCommand::Refresh)
        );
        assert_eq!(
            library_command(ID_LIBRARY_LIST, SELCHANGE),
            Some(LibraryCommand::SelectionChanged)
        );
        assert_eq!(library_command(ID_LIBRARY_SEARCH, BN_CLICKED), None);
        assert_eq!(library_command(ID_LIBRARY_LIST, BN_CLICKED), None);
    }
}
