//! Windows named-pipe transport — thin glue over `windows-sys`.
//!
//! The Discord desktop client listens on `\\.\pipe\discord-ipc-0..9`
//! (Windows). The client side is a plain file handle: `CreateFileW` +
//! `ReadFile`/`WriteFile`. All protocol logic lives in [`crate::protocol`]
//! and [`crate::client`]; this module only shuttles bytes.
//!
//! Reads are **non-blocking** via `PeekNamedPipe`: the driver polls for
//! frames instead of blocking on a reader thread, which keeps the single
//! writer and the poll loop deadlock-free without overlapped I/O.

use std::io;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_BROKEN_PIPE, ERROR_PIPE_NOT_CONNECTED, HANDLE, INVALID_HANDLE_VALUE,
};
#[cfg(test)]
use windows_sys::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, ReadFile, WriteFile, OPEN_EXISTING};
use windows_sys::Win32::System::Pipes::PeekNamedPipe;
#[cfg(test)]
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
};

use crate::client::Transport;
use crate::protocol::{decode_frame, Frame};

/// How many `discord-ipc-N` pipes to try before giving up.
const IPC_MAX: u32 = 10;

/// Named-pipe transport; `handle` is `INVALID_HANDLE_VALUE` when closed.
#[derive(Debug)]
pub struct NamedPipe {
    handle: HANDLE,
}

impl Default for NamedPipe {
    fn default() -> Self {
        Self::new()
    }
}

impl NamedPipe {
    pub fn new() -> Self {
        Self {
            handle: INVALID_HANDLE_VALUE,
        }
    }

    /// Open one Discord IPC pipe by index.
    fn open(index: u32) -> io::Result<HANDLE> {
        Self::open_named(&format!(r"\\.\pipe\discord-ipc-{index}"))
    }

    /// Open a specific named pipe (tests use an isolated name; the driver
    /// uses `discord-ipc-0..9` via [`NamedPipe::connect`]).
    fn open_named(name: &str) -> io::Result<HANDLE> {
        let wide = wide(name);
        // Shared read/write, no overlapped I/O; reads are polled via
        // PeekNamedPipe so nothing ever blocks on an empty pipe.
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                0xC000_0000, // GENERIC_READ | GENERIC_WRITE
                0x3,         // FILE_SHARE_READ | FILE_SHARE_WRITE
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            Ok(handle)
        }
    }
}

impl Transport for NamedPipe {
    fn connect(&mut self) -> Result<(), String> {
        self.close();
        for index in 0..IPC_MAX {
            match Self::open(index) {
                Ok(handle) => {
                    self.handle = handle;
                    return Ok(());
                }
                // A busy/absent pipe just means "try the next index".
                Err(_) => continue,
            }
        }
        Err("no Discord IPC pipe available (is the Discord desktop client running?)".to_string())
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
        let mut written: u32 = 0;
        let ok = unsafe {
            WriteFile(
                self.handle,
                bytes.as_ptr(),
                bytes.len() as u32,
                &mut written,
                std::ptr::null_mut(), // overlapped (unused)
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        if written as usize != bytes.len() {
            return Err(format!("short write: {written}/{} bytes", bytes.len()));
        }
        Ok(())
    }

    /// Non-blocking: `Ok(Some)` on a complete frame, `Ok(None)` when no data
    /// is buffered. A broken pipe surfaces as `Err` (the driver reconnects).
    ///
    /// The wire header is `[opcode u32][length u32]` — the length sits in
    /// bytes 4..8, matching [`crate::protocol::decode_frame`]. The header is
    /// **peeked non-destructively** first: if the declared frame is not yet
    /// fully buffered, nothing is consumed and the next poll retries.
    fn try_read_frame(&mut self) -> Result<Option<Frame>, String> {
        let mut header = [0u8; 8];
        let available = peek_into(self.handle, &mut header).map_err(|e| e.to_string())?;
        if available < 8 {
            return Ok(None);
        }
        let len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        if (available as usize) < 8 + len {
            // Header is present but the payload has not fully arrived — wait
            // for the next poll instead of consuming a partial frame.
            return Ok(None);
        }
        let mut buf = header.to_vec();
        buf.resize(8 + len, 0);
        read_all(self.handle, &mut buf)?;
        decode_frame(&buf).map(Some).map_err(|e| e.to_string())
    }

    fn close(&mut self) {
        if self.handle != INVALID_HANDLE_VALUE {
            unsafe {
                CloseHandle(self.handle);
            }
            self.handle = INVALID_HANDLE_VALUE;
        }
    }

    fn is_open(&self) -> bool {
        self.handle != INVALID_HANDLE_VALUE
    }
}

impl Drop for NamedPipe {
    fn drop(&mut self) {
        self.close();
    }
}

/// Peek without consuming: copies up to `buf.len()` buffered bytes into
/// `buf` and returns the total number of bytes waiting in the pipe. Used to
/// inspect a frame header before deciding whether a full frame is ready.
fn peek_into(handle: HANDLE, buf: &mut [u8]) -> io::Result<u32> {
    let mut available: u32 = 0;
    let ok = unsafe {
        PeekNamedPipe(
            handle,
            buf.as_mut_ptr() as *mut core::ffi::c_void,
            buf.len() as u32,
            std::ptr::null_mut(),
            &mut available,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(available)
    }
}

/// Read until `buf` is full, an error, or the peer closes. Returns the number
/// of bytes read; a value below `buf.len()` means EOF.
fn read_all(handle: HANDLE, buf: &mut [u8]) -> Result<usize, String> {
    let mut total = 0usize;
    while total < buf.len() {
        let mut got: u32 = 0;
        let ok = unsafe {
            ReadFile(
                handle,
                buf[total..].as_mut_ptr(),
                (buf.len() - total) as u32,
                &mut got,
                std::ptr::null_mut(), // overlapped (unused)
            )
        };
        if ok == 0 {
            let err = io::Error::last_os_error();
            // A closed pipe surfaces as an error in some cases; treat it as
            // EOF so the client sees an orderly disconnect.
            if matches!(
                err.raw_os_error(),
                Some(e) if e == ERROR_BROKEN_PIPE as i32 || e == ERROR_PIPE_NOT_CONNECTED as i32
            ) {
                return Ok(total);
            }
            return Err(err.to_string());
        }
        if got == 0 {
            break; // EOF
        }
        total += got as usize;
    }
    Ok(total)
}

/// Encode a string as a nul-terminated UTF-16 buffer.
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::protocol::{encode_frame, OP_FRAME};
    use std::sync::mpsc;

    /// Isolated pipe names — Discord only listens on `discord-ipc-0..9`, so
    /// indices far outside that range cannot collide with the real client.
    const TEST_BASE: u32 = 190;

    fn pipe_name(index: u32) -> String {
        format!(r"\\.\pipe\discord-ipc-{index}")
    }

    /// Create a server-side named-pipe instance (byte mode, one instance).
    fn server_pipe(name: &str) -> HANDLE {
        let wide_name = wide(name);
        let handle = unsafe {
            CreateNamedPipeW(
                wide_name.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,
                4096,
                4096,
                0,
                std::ptr::null_mut(),
            )
        };
        assert!(
            handle != INVALID_HANDLE_VALUE,
            "CreateNamedPipeW failed: {name}"
        );
        handle
    }

    fn server_write(handle: HANDLE, bytes: &[u8]) {
        let mut written: u32 = 0;
        let ok = unsafe {
            WriteFile(
                handle,
                bytes.as_ptr(),
                bytes.len() as u32,
                &mut written,
                std::ptr::null_mut(),
            )
        };
        assert!(ok != 0, "server WriteFile failed");
        assert_eq!(written as usize, bytes.len());
    }

    #[test]
    fn pipe_name_format_matches_discord() {
        // The exact path the Discord client listens on.
        assert_eq!(pipe_name(0), r"\\.\pipe\discord-ipc-0");
        assert_eq!(pipe_name(9), r"\\.\pipe\discord-ipc-9");
    }

    #[test]
    fn wide_appends_nul_terminator() {
        let w = wide("ipc");
        assert_eq!(w, vec!['i' as u16, 'p' as u16, 'c' as u16, 0]);
    }

    /// Regression: the wire header is `[opcode][length]`, so the length must
    /// be read from bytes 4..8. With the old `[length][opcode]` assumption
    /// this read an opcode (1) as the length and failed to decode READY.
    #[test]
    fn reads_frame_from_real_pipe_in_opcode_length_order() {
        let name = pipe_name(TEST_BASE);
        let server = server_pipe(&name);
        let h = server as usize;
        let frame = encode_frame(OP_FRAME, "hello");
        let (tx_done, rx_done) = mpsc::channel::<()>();
        std::thread::spawn(move || {
            let h = h as HANDLE;
            unsafe {
                let _ = ConnectNamedPipe(h, std::ptr::null_mut());
                server_write(h, &frame);
                std::thread::sleep(std::time::Duration::from_millis(30));
                let _ = CloseHandle(h);
            }
            let _ = tx_done.send(());
        });

        let mut pipe = NamedPipe::new();
        pipe.handle = NamedPipe::open_named(&name).expect("client connect");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Ok(Some(f)) = pipe.try_read_frame() {
                assert_eq!(f.opcode, OP_FRAME);
                assert_eq!(f.payload, "hello");
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for the frame"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = rx_done.recv_timeout(std::time::Duration::from_secs(1));
    }

    /// A frame that arrives in two chunks must not be consumed until fully
    /// buffered: peeking the header returns `None` (nothing eaten), and the
    /// frame decodes once the payload lands.
    #[test]
    fn returns_none_until_full_frame_buffered_without_consuming() {
        let name = pipe_name(TEST_BASE + 1);
        let server = server_pipe(&name);
        let h = server as usize;
        let frame = encode_frame(OP_FRAME, "xx"); // 8-byte header + 2-byte payload
        let header_chunk = frame[..8].to_vec();
        let payload_chunk = frame[8..].to_vec();
        let (tx_h, rx_h) = mpsc::channel::<()>();
        let (tx_p, rx_p) = mpsc::channel::<()>();
        std::thread::spawn(move || {
            let h = h as HANDLE;
            unsafe {
                let _ = ConnectNamedPipe(h, std::ptr::null_mut());
                server_write(h, &header_chunk);
                let _ = tx_h.send(());
                std::thread::sleep(std::time::Duration::from_millis(60));
                server_write(h, &payload_chunk);
                let _ = tx_p.send(());
                std::thread::sleep(std::time::Duration::from_millis(30));
                let _ = CloseHandle(h);
            }
        });

        let mut pipe = NamedPipe::new();
        pipe.handle = NamedPipe::open_named(&name).expect("client connect");

        rx_h.recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        // Header only — must return Ok(None) and leave the header unconsumed.
        assert_eq!(pipe.try_read_frame(), Ok(None));

        rx_p.recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Ok(Some(f)) = pipe.try_read_frame() {
                assert_eq!(f.opcode, OP_FRAME);
                assert_eq!(f.payload, "xx");
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for the full frame"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
