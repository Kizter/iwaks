//! Embedded cover (album art) extraction via `lofty`.

use std::path::Path;

use lofty::picture::PictureType;
use lofty::prelude::*;

use crate::TagError;

/// Album art extracted from an audio file's tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cover {
    /// MIME type of the image, e.g. `image/jpeg`, `image/png`.
    pub mime: String,
    /// Raw encoded image bytes.
    pub data: Vec<u8>,
}

/// Read the "best" embedded picture: prefers CoverFront, then any other
/// picture, skipping file-icon/other-file-icon placeholders. Returns `None`
/// when the file has tags but no pictures.
pub fn read_cover(path: &Path) -> Result<Option<Cover>, TagError> {
    let tagged = lofty::read_from_path(path)?;
    let mut best: Option<(u8, &lofty::picture::Picture)> = None;
    for tag in tagged.tags() {
        for pic in tag.pictures() {
            let rank = picture_rank(pic.pic_type());
            if best.is_none_or(|(r, _)| rank < r) {
                best = Some((rank, pic));
            }
        }
    }
    Ok(best.map(|(_, pic)| Cover {
        mime: pic
            .mime_type()
            .map(|m| m.as_str().to_string())
            .unwrap_or_else(|| "image/jpeg".to_string()),
        data: pic.data().to_vec(),
    }))
}

/// Lower rank = preferred; CoverFront beats Back/Misc, icons are last.
fn picture_rank(t: PictureType) -> u8 {
    match t {
        PictureType::CoverFront => 0,
        PictureType::Icon | PictureType::OtherIcon | PictureType::Other => 2,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BACK_PNG: &[u8] = b"PNG-THE-BACK";
    const FRONT_PNG: &[u8] = b"PNG-THE-FRONT";

    /// STREAMINFO for 2ch/16-bit/44.1kHz, 1 second, all zeros MD5.
    fn flac_streaminfo() -> Vec<u8> {
        let mut b = Vec::with_capacity(34);
        b.extend_from_slice(&4096u16.to_be_bytes()); // min block size
        b.extend_from_slice(&4096u16.to_be_bytes()); // max block size
        b.extend_from_slice(&[0, 0, 0]); // min frame size
        b.extend_from_slice(&[0, 0, 0]); // max frame size
                                         // sr(20) | ch-1(3) | bps-1(5) | total samples(36)
        let packed: u64 = (44_100u64 << 44) | (1u64 << 41) | (15u64 << 36) | 44_100;
        b.extend_from_slice(&packed.to_be_bytes());
        b.extend_from_slice(&[0u8; 16]); // MD5
        b
    }

    /// FLAC PICTURE metadata block body.
    fn picture_body(pic_type: u32, mime: &[u8], desc: &[u8], data: &[u8]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&pic_type.to_be_bytes());
        b.extend_from_slice(&(mime.len() as u32).to_be_bytes());
        b.extend_from_slice(mime);
        b.extend_from_slice(&(desc.len() as u32).to_be_bytes());
        b.extend_from_slice(desc);
        b.extend_from_slice(&1u32.to_be_bytes()); // width
        b.extend_from_slice(&1u32.to_be_bytes()); // height
        b.extend_from_slice(&24u32.to_be_bytes()); // depth
        b.extend_from_slice(&0u32.to_be_bytes()); // colors
        b.extend_from_slice(&(data.len() as u32).to_be_bytes());
        b.extend_from_slice(data);
        b
    }

    /// Metadata block header: last-block flag + 3-byte big-endian length.
    fn block_header(last: bool, block_type: u8, body: &[u8]) -> Vec<u8> {
        let mut h = Vec::with_capacity(4);
        h.push((if last { 0x80 } else { 0 }) | block_type);
        h.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
        h
    }

    /// Minimal playable FLAC: `fLaC` + STREAMINFO + optional PICTURE blocks,
    /// terminated by an empty VORBIS_COMMENT when there are no pictures.
    fn build_flac(pictures: Vec<(u32, &[u8], &[u8])>) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(b"fLaC");
        let si = flac_streaminfo();
        f.extend_from_slice(&block_header(false, 0, &si));
        f.extend_from_slice(&si);
        if pictures.is_empty() {
            // Ensure a terminating (last) block always exists.
            f.extend_from_slice(&[0x84, 0, 0, 0]);
        } else {
            for (i, (pt, mime, data)) in pictures.iter().enumerate() {
                let last = i == pictures.len() - 1;
                let body = picture_body(*pt, mime, b"", data);
                f.extend_from_slice(&block_header(last, 6, &body));
                f.extend_from_slice(&body);
            }
        }
        f
    }

    fn write_flac(path: &Path, pictures: Vec<(u32, &[u8], &[u8])>) {
        std::fs::write(path, build_flac(pictures)).expect("write flac fixture");
    }

    #[test]
    fn reads_front_cover_preferring_it_over_back() {
        let dir = std::env::temp_dir().join(format!("iwaks-cover-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("front.flac");
        let f = build_flac(vec![
            (4, b"image/png", BACK_PNG),  // CoverBack
            (3, b"image/png", FRONT_PNG), // CoverFront (last)
        ]);
        std::fs::write(&path, &f).expect("write");

        let cover = read_cover(&path)
            .expect("read should succeed")
            .expect("cover");
        assert_eq!(cover.mime, "image/png");
        assert_eq!(cover.data, FRONT_PNG);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn returns_none_when_file_has_no_picture() {
        let dir = std::env::temp_dir().join(format!("iwaks-cover-test2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("plain.flac");
        write_flac(&path, vec![]);

        assert_eq!(read_cover(&path).expect("read should succeed"), None);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_file_is_error() {
        assert!(read_cover(Path::new("Z:/definitely/not/here.flac")).is_err());
    }

    #[test]
    fn corrupt_file_is_error() {
        let dir = std::env::temp_dir().join(format!("iwaks-cover-test3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("broken.flac");
        std::fs::write(&path, b"this is not flac at all").expect("write");

        assert!(read_cover(&path).is_err());

        std::fs::remove_dir_all(&dir).ok();
    }
}
