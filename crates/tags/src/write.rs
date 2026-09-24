//! Tag writing — full implementation lands with the tag editor in M4
//! (including backup/rollback). What exists now is only enough to prove
//! read/write round-trips on copied fixtures.

use lofty::config::WriteOptions;
use lofty::prelude::*;
use lofty::tag::{Tag, TagType};
use std::path::Path;

use crate::TagError;

/// Write `title`/`artist` onto a file's primary tag (creating an ID3v2 tag
/// when the container has none). Returns an error for containers lofty
/// cannot tag or files it cannot write back.
pub fn write_test_tags(path: &Path, title: &str, artist: &str) -> Result<(), TagError> {
    use lofty::tag::Accessor;

    let mut tagged = lofty::read_from_path(path)?;
    let tag = match tagged.primary_tag_mut() {
        Some(t) => t,
        None => {
            tagged.insert_tag(Tag::new(TagType::Id3v2));
            tagged.primary_tag_mut().ok_or(TagError::NoTagSlot)?
        }
    };
    tag.set_title(title.to_string());
    tag.set_artist(artist.to_string());
    tagged.save_to_path(path, WriteOptions::default())?;
    Ok(())
}
