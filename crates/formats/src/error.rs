//! Format parse / I/O errors (kept out of `lib.rs` to avoid import cycles).

#[derive(Debug)]
pub enum FormatError {
    Io(std::io::Error),
    Format(String),
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Format(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for FormatError {}
