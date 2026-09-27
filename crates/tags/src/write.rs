//! Tag writing (M4 slice 4 — tag editor): validate the container, back the
//! file up to `path.bak`, then write the core tag fields via `lofty`.
//! `write_test_tags` is kept for the read-side round-trip test; the editor
//! path is `write_metadata`.

use lofty::config::WriteOptions;
use lofty::file::FileType;
use lofty::prelude::*;
use lofty::tag::{Accessor, ItemKey, Tag, TagType};
use std::path::{Path, PathBuf};

use iwaks_core::track::{AudioMetadata, TagEdits};

use crate::read::read_metadata;
use crate::TagError;

/// Write `title`/`artist` onto a file's primary tag (creating an ID3v2 tag
/// when the container has none). Returns an error for containers lofty
/// cannot tag or files it cannot write back.
pub fn write_test_tags(path: &Path, title: &str, artist: &str) -> Result<(), TagError> {
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

/// `path.bak` — the pre-edit backup created right before writing tags.
pub fn backup_path(path: &Path) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(".bak");
    PathBuf::from(os)
}

/// The tag type each container saves natively. Used only when a file has no
/// primary tag yet — forcing ID3v2 onto FLAC/OGG/MP4 would be wrong; lofty
/// would then have to convert on save. Matches `FileType::primary_tag_type`.
fn primary_tag_type(file_type: FileType) -> TagType {
    match file_type {
        FileType::Flac | FileType::Vorbis | FileType::Opus => TagType::VorbisComments,
        FileType::Mp4 => TagType::Mp4Ilst,
        FileType::WavPack | FileType::Ape => TagType::Ape,
        // Mpeg (mp3), Wav, Aiff, Dsf
        _ => TagType::Id3v2,
    }
}

/// Write the user-editable tag fields of `meta` onto the file's tags.
///
/// Order matters: the container is opened and a writable primary tag is
/// ensured *before* anything touches disk, so unreadable/untaggable files
/// never leave a stray `.bak`. Then the current file is copied to
/// `path.bak` (an older backup is overwritten) and the 8 core fields are
/// applied. `None` fields remove that tag from the file.
pub fn write_metadata(path: &Path, meta: &AudioMetadata) -> Result<(), TagError> {
    let mut tagged = lofty::read_from_path(path)?;
    if tagged.primary_tag().is_none() {
        tagged.insert_tag(Tag::new(primary_tag_type(tagged.file_type())));
    }
    let tag = tagged.primary_tag_mut().ok_or(TagError::NoTagSlot)?;

    write_string_field(tag, ItemKey::TrackTitle, meta.title.clone());
    write_string_field(tag, ItemKey::TrackArtist, meta.artist.clone());
    write_string_field(tag, ItemKey::AlbumTitle, meta.album.clone());
    write_string_field(tag, ItemKey::AlbumArtist, meta.album_artist.clone());
    write_string_field(tag, ItemKey::Genre, meta.genre.clone());
    write_number_field(tag, ItemKey::Year, meta.year)?;
    write_number_field(tag, ItemKey::TrackNumber, meta.track_no)?;
    write_number_field(tag, ItemKey::DiscNumber, meta.disc_no)?;

    std::fs::copy(path, backup_path(path))?;
    tagged.save_to_path(path, WriteOptions::default())?;
    Ok(())
}

/// Edit a file's core tags — the tag-editor entry point. Merges `edits` onto
/// the file's current metadata, writes them (with a `.bak` backup), then
/// re-reads the file so callers get the tags that actually landed on disk.
pub fn apply_edits(path: &Path, edits: &TagEdits) -> Result<AudioMetadata, TagError> {
    let current = read_metadata(path)?;
    let merged = edits.merge(&current);
    write_metadata(path, &merged)?;
    read_metadata(path)
}

/// Set a free-form string tag (`insert_text` replaces existing items with
/// that key); `None` removes the tag from the file.
fn write_string_field(tag: &mut Tag, key: ItemKey, value: Option<String>) {
    match value {
        Some(v) => {
            tag.insert_text(key, v);
        }
        None => tag.remove_key(&key),
    }
}

/// Set a numeric tag (lofty maps year/track/disc through its accessors);
/// `None` removes the tag from the file (via the matching accessor, so the
/// exact per-format item key is used — e.g. ID3v2 stores the year under
/// `RecordingDate`, not `Year`).
fn write_number_field(tag: &mut Tag, key: ItemKey, value: Option<i64>) -> Result<(), TagError> {
    let Some(v) = value else {
        match key {
            ItemKey::Year => tag.remove_year(),
            ItemKey::TrackNumber => tag.remove_track(),
            ItemKey::DiscNumber => tag.remove_disk(),
            _ => {}
        }
        return Ok(());
    };
    let n = u32::try_from(v).map_err(|_| TagError::Unsupported("tag value out of range".into()))?;
    match key {
        ItemKey::Year => tag.set_year(n),
        ItemKey::TrackNumber => tag.set_track(n),
        ItemKey::DiscNumber => tag.set_disk(n),
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read::read_metadata;
    use crate::testutil::{write_silent_wav, TempDir};

    fn full_meta() -> AudioMetadata {
        let mut m = AudioMetadata::new("wav");
        m.title = Some("Judul".into());
        m.artist = Some("Artis".into());
        m.album = Some("Album".into());
        m.album_artist = Some("Album Artis".into());
        m.genre = Some("Jazz".into());
        m.year = Some(2021);
        m.track_no = Some(3);
        m.disc_no = Some(1);
        m.duration_ms = 500;
        m
    }

    #[test]
    fn write_all_fields_round_trip() {
        let tmp = TempDir::new("w-all");
        let p = tmp.path("a.wav");
        write_silent_wav(&p, 44_100, 2, 0.5);

        write_metadata(&p, &full_meta()).expect("write should succeed");
        let m = read_metadata(&p).expect("read should succeed");

        assert_eq!(m.title.as_deref(), Some("Judul"));
        assert_eq!(m.artist.as_deref(), Some("Artis"));
        assert_eq!(m.album.as_deref(), Some("Album"));
        assert_eq!(m.album_artist.as_deref(), Some("Album Artis"));
        assert_eq!(m.genre.as_deref(), Some("Jazz"));
        assert_eq!(m.year, Some(2021));
        assert_eq!(m.track_no, Some(3));
        assert_eq!(m.disc_no, Some(1));
    }

    #[test]
    fn second_write_overwrites_tags() {
        let tmp = TempDir::new("w-overwrite");
        let p = tmp.path("b.wav");
        write_silent_wav(&p, 44_100, 1, 0.5);

        write_metadata(&p, &full_meta()).expect("first write");
        let mut second = full_meta();
        second.title = Some("Judul Baru".into());
        second.year = Some(2022);
        write_metadata(&p, &second).expect("second write");

        let m = read_metadata(&p).expect("read should succeed");
        assert_eq!(m.title.as_deref(), Some("Judul Baru"));
        assert_eq!(m.year, Some(2022));
        assert_eq!(
            m.artist.as_deref(),
            Some("Artis"),
            "untouched fields persist"
        );
        assert_eq!(m.genre.as_deref(), Some("Jazz"));
    }

    #[test]
    fn none_fields_are_removed_from_file() {
        let tmp = TempDir::new("w-remove");
        let p = tmp.path("c.wav");
        write_silent_wav(&p, 44_100, 1, 0.4);

        write_metadata(&p, &full_meta()).expect("write full tags");
        let mut slim = AudioMetadata::new("wav");
        slim.title = Some("Judul".into());
        slim.album = Some("Album".into());
        write_metadata(&p, &slim).expect("write slim tags");

        let m = read_metadata(&p).expect("read should succeed");
        assert_eq!(m.title.as_deref(), Some("Judul"));
        assert_eq!(m.album.as_deref(), Some("Album"));
        assert_eq!(m.artist, None, "default (None) fields must be removed");
        assert_eq!(m.genre, None);
        assert_eq!(m.year, None);
        assert_eq!(m.track_no, None);
        assert_eq!(m.disc_no, None);
    }

    #[test]
    fn backup_created_with_pre_edit_bytes() {
        let tmp = TempDir::new("w-bak");
        let p = tmp.path("d.wav");
        write_silent_wav(&p, 44_100, 1, 0.25);
        let original = std::fs::read(&p).expect("read original");

        write_metadata(&p, &full_meta()).expect("write should succeed");

        let bak = backup_path(&p);
        assert!(bak.exists(), ".bak must exist after writing");
        let backup = std::fs::read(&bak).expect("read backup");
        assert_eq!(backup, original, "backup must hold the pre-edit bytes");
        assert_ne!(
            std::fs::read(&p).expect("read edited"),
            original,
            "the file itself must change"
        );
    }

    #[test]
    fn missing_file_is_error() {
        assert!(write_metadata(Path::new("Z:/definitely/not/here.wav"), &full_meta()).is_err());
    }

    #[test]
    fn untaggable_file_is_error_and_leaves_no_backup() {
        let tmp = TempDir::new("w-untaggable");
        let p = tmp.path("e.txt");
        std::fs::write(&p, b"not audio").expect("write txt");

        let res = write_metadata(&p, &full_meta());
        assert!(res.is_err(), ".txt must not be taggable");
        assert!(
            !backup_path(&p).exists(),
            "no stray .bak for untaggable files"
        );
    }

    use iwaks_core::track::TagEdits;

    fn tag_edits() -> TagEdits {
        TagEdits {
            title: "Judul Edit".into(),
            artist: Some("Artis Edit".into()),
            album: Some("Album Edit".into()),
            album_artist: Some("Album Artis Edit".into()),
            genre: Some("Rock".into()),
            year: Some(2019),
            track_no: Some(7),
            disc_no: Some(2),
        }
    }

    #[test]
    fn apply_edits_writes_and_returns_new_tags() {
        let tmp = TempDir::new("ed-apply");
        let p = tmp.path("a.wav");
        write_silent_wav(&p, 44_100, 2, 0.5);

        let meta = apply_edits(&p, &tag_edits()).expect("apply should succeed");

        assert_eq!(meta.title.as_deref(), Some("Judul Edit"));
        assert_eq!(meta.artist.as_deref(), Some("Artis Edit"));
        assert_eq!(meta.album.as_deref(), Some("Album Edit"));
        assert_eq!(meta.album_artist.as_deref(), Some("Album Artis Edit"));
        assert_eq!(meta.genre.as_deref(), Some("Rock"));
        assert_eq!(meta.year, Some(2019));
        assert_eq!(meta.track_no, Some(7));
        assert_eq!(meta.disc_no, Some(2));

        let reread = read_metadata(&p).expect("re-read after apply");
        assert_eq!(reread.title.as_deref(), Some("Judul Edit"));
        assert_eq!(reread.track_no, Some(7));
        assert!(backup_path(&p).exists(), ".bak must exist after apply");
    }

    #[test]
    fn apply_edits_blank_title_and_none_fields_clear_tags() {
        let tmp = TempDir::new("ed-clear");
        let p = tmp.path("b.wav");
        write_silent_wav(&p, 44_100, 1, 0.5);
        write_metadata(&p, &full_meta()).expect("pre-write full tags");

        let edits = TagEdits {
            title: "   ".into(),
            artist: None,
            album: Some("Album Edit".into()),
            ..Default::default()
        };
        let meta = apply_edits(&p, &edits).expect("apply should succeed");

        assert_eq!(meta.title, None, "blank title must remove the title tag");
        assert_eq!(meta.artist, None, "None must remove the artist tag");
        assert_eq!(meta.album.as_deref(), Some("Album Edit"));
        assert_eq!(meta.genre, None);
        assert_eq!(meta.year, None);
        assert_eq!(meta.track_no, None);
        assert_eq!(meta.disc_no, None);
    }

    #[test]
    fn apply_edits_backs_up_original_bytes() {
        let tmp = TempDir::new("ed-bak");
        let p = tmp.path("c.wav");
        write_silent_wav(&p, 44_100, 1, 0.3);
        let original = std::fs::read(&p).expect("read original");

        apply_edits(&p, &tag_edits()).expect("apply should succeed");

        let backup = std::fs::read(backup_path(&p)).expect("read backup");
        assert_eq!(backup, original, "backup must hold the pre-edit bytes");
    }

    #[test]
    fn apply_edits_missing_file_is_error() {
        assert!(apply_edits(Path::new("Z:/definitely/not/here.wav"), &tag_edits()).is_err());
    }
}
