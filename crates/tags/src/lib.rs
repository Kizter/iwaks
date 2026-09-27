//! Audio tag reading/writing via `lofty` (reading implemented in M1;
//! writing + backup lands with the tag editor in M4).

pub mod cover;
pub mod lyrics;
pub mod read;
pub mod write;

#[cfg(test)]
pub(crate) mod testutil {
    use std::io::Write;
    use std::path::Path;

    /// Minimal, valid 16-bit PCM WAV with silent samples, written by hand
    /// (no external crate needed). Shared by read/write tag tests.
    pub fn write_silent_wav(path: &Path, sample_rate: u32, channels: u16, secs: f32) {
        let bits: u16 = 16;
        let block_align = channels * bits / 8;
        let byte_rate = sample_rate * u32::from(block_align);
        let data_len =
            ((byte_rate as f32 * secs) as u32 / u32::from(block_align)) * u32::from(block_align);

        let mut buf = Vec::new();
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&(36 + data_len).to_le_bytes());
        buf.extend_from_slice(b"WAVE");
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
        buf.extend_from_slice(&channels.to_le_bytes());
        buf.extend_from_slice(&sample_rate.to_le_bytes());
        buf.extend_from_slice(&byte_rate.to_le_bytes());
        buf.extend_from_slice(&block_align.to_le_bytes());
        buf.extend_from_slice(&bits.to_le_bytes());
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_len.to_le_bytes());
        buf.resize(buf.len() + data_len as usize, 0);

        let mut f = std::fs::File::create(path).expect("create fixture");
        f.write_all(&buf).expect("write fixture");
    }

    /// Unique temp dir per test binary run, cleaned up on drop.
    pub struct TempDir(std::path::PathBuf);
    impl TempDir {
        pub fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("iwaks-tags-test-{}-{name}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("create temp dir");
            TempDir(dir)
        }
        pub fn path(&self, name: &str) -> std::path::PathBuf {
            self.0.join(name)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

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
