//! Minimal hand-rolled FFI bindings for libmpv (classic 1.x client API),
//! dynamically loaded at **runtime** via `libloading` — builds and unit
//! tests compile without any mpv library present on the machine.
//!
//! DLL resolution order ([`Mpv::load_library`]):
//!   1. `IWAKS_LIBMPV` env var (explicit path — CI/tests)
//!   2. `libmpv-2.dll` / `mpv-2.dll` (next to exe or on `PATH`)
//!   3. `resources\libmpv-2.dll` (Tauri bundles resources here)
//!   4. `libmpv.dll` / `libmpv.so.2` (non-Windows fallback)
//!
//! Only the client calls Iwaks needs are bound. `mpv_wait_event` returns a
//! pointer valid only until the *next* call, so every event is copied into an
//! owned Rust value immediately.
//!
//! # Safety
//!
//! libmpv's client API is documented thread-safe: a single `mpv_handle` may be
//! used concurrently from multiple threads. `Mpv` relies on that contract and
//! is therefore `Send + Sync` (only termination is serialized).

use std::ffi::{c_char, c_int, c_void, CString};
use std::sync::atomic::{AtomicBool, Ordering};

use libloading::Library;

/// `mpv_format` values used by the getters below.
pub const FORMAT_STRING: c_int = 1;
pub const FORMAT_FLAG: c_int = 3;
pub const FORMAT_INT64: c_int = 4;
pub const FORMAT_DOUBLE: c_int = 5;

/// `mpv_event_id` values relevant to the pump (mirrored from libmpv 2.x
/// `client.h` so the mapping is verifiable in a pure test).
pub const EVENT_NONE: c_int = 0;
pub const EVENT_SHUTDOWN: c_int = 1;
pub const EVENT_LOG_MESSAGE: c_int = 2;
pub const EVENT_START_FILE: c_int = 6;
pub const EVENT_END_FILE: c_int = 7;
pub const EVENT_FILE_LOADED: c_int = 8;
pub const EVENT_IDLE: c_int = 11;
pub const EVENT_TICK: c_int = 14;

/// Why the current file ended (`mpv_event_end_file.reason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndFileReason {
    Eof,
    Stop,
    Quit,
    Error,
    Redirect,
    Other(c_int),
}

impl EndFileReason {
    fn from_raw(v: c_int) -> Self {
        match v {
            0 => Self::Eof,
            1 => Self::Stop,
            2 => Self::Quit,
            3 => Self::Error,
            4 => Self::Redirect,
            other => Self::Other(other),
        }
    }
}

/// Owned copy of an mpv event (no pointers retained).
///
/// Only the events the pump acts on have explicit variants; everything else
/// (```LOG_MESSAGE```, ```AUDIO_RECONFIG```, deprecated ```TICK```, ...) maps
/// to [`MpvEvent::Other`] and is ignored. Modern libmpv (2.x) removed the
/// dedicated `PAUSE`/`UNPAUSE` events — pause state is read via properties.
#[derive(Debug, Clone, PartialEq)]
pub enum MpvEvent {
    Shutdown,
    /// Text line from mpv's internal log (only requested when `IWAKS_MPV_LOG`).
    LogMessage(String),
    EndFile(EndFileReason),
    FileLoaded,
    /// Any event the pump does not act on.
    Other(c_int),
}

/// C layout of `mpv_event` (only the head of the struct is needed).
#[repr(C)]
#[allow(dead_code)] // reply_userdata used only for layout/alignment
struct RawEvent {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

/// C layout of `mpv_event_log_message` (for `IWAKS_MPV_LOG` debugging).
#[repr(C)]
struct RawLog {
    prefix: *const c_char,
    level: *const c_char,
    text: *const c_char,
    log_level: c_int,
}

type CreateFn = unsafe extern "C" fn() -> *mut c_void;
type InitializeFn = unsafe extern "C" fn(*mut c_void) -> c_int;
type TerminateFn = unsafe extern "C" fn(*mut c_void);
type SetOptFn = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int;
type GetPropFn = unsafe extern "C" fn(*mut c_void, *const c_char, c_int, *mut c_void) -> c_int;
type CommandFn = unsafe extern "C" fn(*mut c_void, *const *const c_char) -> c_int;
type WaitEventFn = unsafe extern "C" fn(*mut c_void, f64) -> *const RawEvent;
type WakeupFn = unsafe extern "C" fn(*mut c_void);
type RequestLogFn = unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int;
type FreeFn = unsafe extern "C" fn(*mut c_void);

/// Runtime-loaded libmpv handle plus the bound symbols.
///
/// Kept alive by `_lib` (the `Library` is the load guard), and by `handle`.
pub struct Mpv {
    _lib: Library,
    handle: *mut c_void,
    terminated: AtomicBool,
    initialize: InitializeFn,
    terminate: TerminateFn,
    set_option_string: SetOptFn,
    get_property: GetPropFn,
    command: CommandFn,
    wait_event: WaitEventFn,
    wakeup: WakeupFn,
    request_log: RequestLogFn,
    mpv_free: FreeFn,
}

// SAFETY: see module docs — libmpv's client API is thread-safe; the handle is
// an opaque pointer and every use goes through the bound function pointers.
unsafe impl Send for Mpv {}
unsafe impl Sync for Mpv {}

impl Mpv {
    /// Find a loadable libmpv library on this machine.
    pub fn load_library() -> Result<Library, String> {
        if let Ok(path) = std::env::var("IWAKS_LIBMPV") {
            if !path.is_empty() {
                // SAFETY: loading a DLL runs its entry point; this is inherent
                // to the runtime-loading design (libmpv is optional, dev-only).
                return unsafe { Library::new(&path) }
                    .map_err(|e| format!("IWAKS_LIBMPV ({path}): {e}"));
            }
        }
        const CANDIDATES: [&str; 6] = [
            "libmpv-2.dll",
            "mpv-2.dll",
            "resources\\libmpv\\libmpv-2.dll",
            "resources\\libmpv-2.dll",
            "libmpv.dll",
            "libmpv.so.2",
        ];
        for name in CANDIDATES {
            // SAFETY: same rationale as above — libmpv is loaded at runtime.
            if let Ok(lib) = unsafe { Library::new(name) } {
                return Ok(lib);
            }
        }
        Err("no libmpv DLL found (set IWAKS_LIBMPV or ship libmpv-2.dll next to the exe)".into())
    }

    /// Load libmpv and create a client handle (not yet initialized).
    pub fn new() -> Result<Self, String> {
        let lib = Self::load_library()?;
        // SAFETY: symbol names are fixed and verified at runtime; each is cast
        // to the matching function type from libmpv's C header.
        unsafe {
            let initialize: InitializeFn = *lib
                .get(b"mpv_initialize")
                .map_err(|e| format!("mpv_initialize: {e}"))?;
            let terminate: TerminateFn = *lib
                .get(b"mpv_terminate_destroy")
                .map_err(|e| format!("mpv_terminate_destroy: {e}"))?;
            let set_option_string: SetOptFn = *lib
                .get(b"mpv_set_option_string")
                .map_err(|e| format!("mpv_set_option_string: {e}"))?;
            let get_property: GetPropFn = *lib
                .get(b"mpv_get_property")
                .map_err(|e| format!("mpv_get_property: {e}"))?;
            let command: CommandFn = *lib
                .get(b"mpv_command")
                .map_err(|e| format!("mpv_command: {e}"))?;
            let wait_event: WaitEventFn = *lib
                .get(b"mpv_wait_event")
                .map_err(|e| format!("mpv_wait_event: {e}"))?;
            let wakeup: WakeupFn = *lib
                .get(b"mpv_wakeup")
                .map_err(|e| format!("mpv_wakeup: {e}"))?;
            let request_log: RequestLogFn = *lib
                .get(b"mpv_request_log_messages")
                .map_err(|e| format!("mpv_request_log_messages: {e}"))?;
            let mpv_free: FreeFn = *lib.get(b"mpv_free").map_err(|e| format!("mpv_free: {e}"))?;
            let create: CreateFn = *lib
                .get(b"mpv_create")
                .map_err(|e| format!("mpv_create: {e}"))?;

            let handle = create();
            if handle.is_null() {
                return Err("mpv_create returned a null handle".into());
            }
            Ok(Self {
                _lib: lib,
                handle,
                terminated: AtomicBool::new(false),
                initialize,
                terminate,
                set_option_string,
                get_property,
                command,
                wait_event,
                wakeup,
                request_log,
                mpv_free,
            })
        }
    }

    /// Initialize the client with the options already set ([`Mpv::set_option`]).
    pub fn initialize(&self) -> Result<(), c_int> {
        // SAFETY: handle is valid; the call is thread-safe.
        let rc = unsafe { (self.initialize)(self.handle) };
        err_rc(rc)
    }

    /// Set a startup option (only valid before [`Mpv::initialize`]).
    pub fn set_option(&self, name: &str, value: &str) -> Result<(), c_int> {
        let n = CString::new(name).map_err(|_| -1)?;
        let v = CString::new(value).map_err(|_| -1)?;
        // SAFETY: nul-terminated C strings; handle is valid.
        let rc = unsafe { (self.set_option_string)(self.handle, n.as_ptr(), v.as_ptr()) };
        err_rc(rc)
    }

    /// Run an mpv command (arguments must not contain NUL bytes).
    pub fn command(&self, args: &[&str]) -> Result<(), c_int> {
        let cstrings: Vec<CString> = args
            .iter()
            .map(|a| CString::new(*a).map_err(|_| -1))
            .collect::<Result<_, _>>()?;
        let mut ptrs: Vec<*const c_char> = cstrings.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        // SAFETY: argv array is null-terminated; handle is valid.
        let rc = unsafe { (self.command)(self.handle, ptrs.as_ptr()) };
        err_rc(rc)
    }

    fn get_prop(&self, name: &str, format: c_int, out: *mut c_void) -> c_int {
        let Ok(n) = CString::new(name) else { return -1 };
        // SAFETY: valid handle + output buffer of the matching C type.
        unsafe { (self.get_property)(self.handle, n.as_ptr(), format, out) }
    }

    pub fn get_double(&self, name: &str) -> Result<f64, c_int> {
        let mut out: f64 = 0.0;
        let rc = self.get_prop(name, FORMAT_DOUBLE, (&mut out as *mut f64).cast());
        if rc < 0 {
            Err(rc)
        } else {
            Ok(out)
        }
    }

    pub fn get_i64(&self, name: &str) -> Result<i64, c_int> {
        let mut out: i64 = 0;
        let rc = self.get_prop(name, FORMAT_INT64, (&mut out as *mut i64).cast());
        if rc < 0 {
            Err(rc)
        } else {
            Ok(out)
        }
    }

    /// Read a string property. The string is allocated by mpv and freed via
    /// `mpv_free` — the returned `String` is always an owned copy.
    pub fn get_string(&self, name: &str) -> Result<String, c_int> {
        let mut out: *mut c_char = std::ptr::null_mut();
        let rc = self.get_prop(name, FORMAT_STRING, (&mut out as *mut *mut c_char).cast());
        if rc < 0 {
            return Err(rc);
        }
        if out.is_null() {
            return Ok(String::new());
        }
        // SAFETY: mpv set `out` to a NUL-terminated string we must free.
        let text = unsafe { std::ffi::CStr::from_ptr(out) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: the pointer came from mpv (owing) and is freed once.
        unsafe { (self.mpv_free)(out.cast()) };
        Ok(text)
    }

    pub fn get_flag(&self, name: &str) -> Result<bool, c_int> {
        let mut out: c_int = 0;
        let rc = self.get_prop(name, FORMAT_FLAG, (&mut out as *mut c_int).cast());
        if rc < 0 {
            Err(rc)
        } else {
            Ok(out != 0)
        }
    }

    /// Block up to `timeout` seconds for the next event; `None` on timeout.
    /// The returned event is an owned copy — safe to keep.
    pub fn wait_event(&self, timeout: f64) -> Option<MpvEvent> {
        // SAFETY: handle is valid; mpv_wait_event is thread-safe. The returned
        // raw pointer is read only while it is valid (before the next call).
        let raw = unsafe { (self.wait_event)(self.handle, timeout) };
        if raw.is_null() {
            return None;
        }
        // SAFETY: raw points at a live mpv_event; we copy the head fields.
        let ev = unsafe { &*raw };
        match ev.event_id {
            // Timeout or wakeup: nothing pending (the pump uses this as its
            // ~4 Hz clock). mpv returns EVENT_NONE, never a null pointer.
            EVENT_NONE => None,
            EVENT_SHUTDOWN => Some(MpvEvent::Shutdown),
            EVENT_LOG_MESSAGE => {
                if ev.data.is_null() {
                    Some(MpvEvent::LogMessage(String::new()))
                } else {
                    // SAFETY: mpv_event_log_message layout as `RawLog`.
                    let log = unsafe { &*(ev.data as *const RawLog) };
                    let text = if log.text.is_null() {
                        String::new()
                    } else {
                        // SAFETY: null-terminated string owned by mpv.
                        unsafe { std::ffi::CStr::from_ptr(log.text) }
                            .to_string_lossy()
                            .into_owned()
                    };
                    Some(MpvEvent::LogMessage(text))
                }
            }
            EVENT_END_FILE => {
                let reason = if ev.data.is_null() {
                    EndFileReason::Other(-1)
                } else {
                    // SAFETY: mpv_event_end_file's first field is `int reason`.
                    unsafe { EndFileReason::from_raw(*(ev.data as *const c_int)) }
                };
                Some(MpvEvent::EndFile(reason))
            }
            EVENT_FILE_LOADED => Some(MpvEvent::FileLoaded),
            other => Some(MpvEvent::Other(other)),
        }
    }

    /// Wake the event loop so a blocked `wait_event` returns promptly.
    pub fn wakeup(&self) {
        // SAFETY: handle is valid; mpv_wakeup is thread-safe.
        unsafe { (self.wakeup)(self.handle) }
    }

    /// Ask mpv to deliver internal log messages as events (for debugging).
    pub fn request_log_messages(&self, level: &str) -> Result<(), c_int> {
        let Ok(l) = CString::new(level) else {
            return Err(-1);
        };
        // SAFETY: nul-terminated level string; handle is valid.
        let rc = unsafe { (self.request_log)(self.handle, l.as_ptr()) };
        err_rc(rc)
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        // Guard against a double destroy (explicit shutdown path + Drop).
        if self.terminated.swap(true, Ordering::SeqCst) {
            return;
        }
        // SAFETY: handle is still valid; no calls happen after this one.
        unsafe { (self.terminate)(self.handle) }
    }
}

fn err_rc(rc: c_int) -> Result<(), c_int> {
    if rc < 0 {
        Err(rc)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_ids_match_libmpv_header() {
        assert_eq!(EVENT_NONE, 0);
        assert_eq!(EVENT_SHUTDOWN, 1);
        assert_eq!(EVENT_START_FILE, 6);
        assert_eq!(EVENT_END_FILE, 7);
        assert_eq!(EVENT_FILE_LOADED, 8);
        assert_eq!(EVENT_IDLE, 11);
        assert_eq!(EVENT_TICK, 14);
    }

    #[test]
    fn end_file_reason_mapping() {
        assert_eq!(EndFileReason::from_raw(0), EndFileReason::Eof);
        assert_eq!(EndFileReason::from_raw(1), EndFileReason::Stop);
        assert_eq!(EndFileReason::from_raw(2), EndFileReason::Quit);
        assert_eq!(EndFileReason::from_raw(3), EndFileReason::Error);
        assert_eq!(EndFileReason::from_raw(4), EndFileReason::Redirect);
        assert_eq!(EndFileReason::from_raw(99), EndFileReason::Other(99));
    }

    #[test]
    fn replaygain_and_af_are_runtime_settable() {
        let api = match Mpv::new() {
            Ok(a) => a,
            Err(_) => {
                eprintln!("SKIP: no libmpv DLL on this machine");
                return;
            }
        };
        api.set_option("idle", "yes").unwrap();
        api.set_option("ao", "null").unwrap();
        api.set_option("vo", "null").unwrap();
        api.set_option("replaygain", "track").unwrap();
        api.initialize().unwrap();

        // ReplayGain mode: the option is readable and switchable at runtime.
        assert_eq!(api.get_string("replaygain").unwrap(), "track");
        api.command(&["set", "replaygain", "album"]).unwrap();
        assert_eq!(api.get_string("replaygain").unwrap(), "album");
        // mpv's off value is `no` (not `off`).
        api.command(&["set", "replaygain", "no"]).unwrap();
        assert_eq!(api.get_string("replaygain").unwrap(), "no");

        // `af` accepts a lavfi graph and an empty string clears it.
        api.command(&["set", "af", "lavfi=[equalizer=f=1000:t=o:w=1:g=3.0]"])
            .unwrap();
        assert!(api.get_string("af").unwrap().contains("equalizer"));
        api.command(&["set", "af", ""]).unwrap();
        assert_eq!(api.get_string("af").unwrap(), "");
    }
}
