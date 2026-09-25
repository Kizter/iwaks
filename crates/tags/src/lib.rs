//! Audio tag reading/writing via `lofty` (reading implemented in M1;
//! writing + backup lands with the tag editor in M4).

pub mod cover;
pub mod lyrics;
pub mod read;
pub mod write;

/// Errors produced while reading audio metadata.
#[derive(Debug, thiserror::Error)]
pub enum TagError {
    #[error("unreadable file")]
    Io(#[from] std::io::Error),
    #[error("lofty error: {0}")]
    Lofty(#[from] lofty::error::LoftyError),
    #[error("no audio properties available")]
    NoProperties,
    #[error("file has no tag slot for writing")]
    NoTagSlot,
    #[error("unsupported operation: {0}")]
    Unsupported(String),
}
