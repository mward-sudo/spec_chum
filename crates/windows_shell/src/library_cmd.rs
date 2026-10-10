//! Classify Library window `WM_COMMAND` notifications.

pub const ID_LIBRARY_SEARCH: i32 = 3001;
pub const ID_LIBRARY_CATEGORY: i32 = 3002;
pub const ID_LIBRARY_LIST: i32 = 3003;
pub const ID_LIBRARY_DETAILS: i32 = 3004;
pub const ID_LIBRARY_OPEN_BUTTON: i32 = 3005;
pub const ID_LIBRARY_REMOVE_BUTTON: i32 = 3006;
pub const ID_LIBRARY_CLOSE_BUTTON: i32 = 3007;

/// Action for one Library control notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryCommand {
    Refresh,
    SelectionChanged,
    Open,
    Remove,
    Close,
}

/// Map a Library control ID and notification to an action.
///
/// Button focus notifications share `WM_COMMAND` with clicks, so only
/// `BN_CLICKED` may open, remove, or close a Library entry/window.
#[must_use]
pub fn library_command(control_id: i32, notification: usize) -> Option<LibraryCommand> {
    match (control_id, notification) {
        (ID_LIBRARY_SEARCH, 0x0300) | (ID_LIBRARY_CATEGORY, 1) => Some(LibraryCommand::Refresh),
        (ID_LIBRARY_LIST, 1) => Some(LibraryCommand::SelectionChanged),
        (ID_LIBRARY_OPEN_BUTTON, 0) => Some(LibraryCommand::Open),
        (ID_LIBRARY_REMOVE_BUTTON, 0) => Some(LibraryCommand::Remove),
        (ID_LIBRARY_CLOSE_BUTTON, 0) => Some(LibraryCommand::Close),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{library_command, LibraryCommand};

    const OPEN: i32 = 3005;
    const REMOVE: i32 = 3006;
    const CLOSE: i32 = 3007;

    #[test]
    fn button_focus_notifications_do_not_trigger_actions() {
        for notification in [6, 7] {
            assert_eq!(library_command(OPEN, notification), None);
            assert_eq!(library_command(REMOVE, notification), None);
            assert_eq!(library_command(CLOSE, notification), None);
        }
    }

    #[test]
    fn only_button_click_notifications_trigger_actions() {
        assert_eq!(library_command(OPEN, 0), Some(LibraryCommand::Open));
        assert_eq!(library_command(REMOVE, 0), Some(LibraryCommand::Remove));
        assert_eq!(library_command(CLOSE, 0), Some(LibraryCommand::Close));
        assert_eq!(library_command(REMOVE, 1), None);
    }

    #[test]
    fn search_category_and_selection_notifications_are_preserved() {
        assert_eq!(library_command(3001, 0x0300), Some(LibraryCommand::Refresh));
        assert_eq!(library_command(3002, 1), Some(LibraryCommand::Refresh));
        assert_eq!(
            library_command(3003, 1),
            Some(LibraryCommand::SelectionChanged)
        );
    }
}
