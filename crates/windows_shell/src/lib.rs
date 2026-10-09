//! Library surface for the Windows native shell (keymap unit tests + shared audio).

pub mod keymap;
mod library_cmd;

pub use library_cmd::{
    library_command, LibraryCommand, ID_LIBRARY_CATEGORY, ID_LIBRARY_CLOSE_BUTTON,
    ID_LIBRARY_DETAILS, ID_LIBRARY_LIST, ID_LIBRARY_OPEN_BUTTON, ID_LIBRARY_REMOVE_BUTTON,
    ID_LIBRARY_SEARCH,
};
