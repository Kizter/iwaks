//! Minimal hand-rolled FFI bindings for libmpv (classic 1.x client API),
//! dynamically loaded at **runtime** via `libloading` — builds and unit
//! tests compile without any mpv library present on the machine.
//!
//! DLL resolution order ([`Mpv::load_library`]):
//!   1. `IWAKS_LIBMPV` env var (explicit path — CI/tests)
//!   2. `<exe-dir>\libmpv\libmpv-2.dll` — Tauri v2 keeps `bundle.resources`
//!      next to the executable, preserving the declared subpath
//!   3. `<exe-dir>\libmpv-2.dll` then bare `libmpv-2.dll` / `mpv-2.dll`
//!      (flat next to the exe or on `PATH` — covers dev builds)
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
/// The request format for "give me the property as an mpv node tree".
///
/// List-of-options properties must be read this way — asking for a
/// `MPV_FORMAT_STRING` returns an empty string, which is why the first version
/// of the device picker showed nothing at all. Asking for the more specific
/// `MPV_FORMAT_NODE_LIST` (8) instead is rejected with `MPV_ERROR_PROPERTY_FORMAT`:
/// mpv only converts *to* a node list, not *from* a node array, so the caller has
/// to dispatch on the shape the node turns out to have.
pub const FORMAT_NODE: c_int = 6;
/// A node whose own `format` says it is a string.
pub const NODE_FORMAT_STRING: c_int = 1;
/// A node whose own `format` says it is an array.
pub const NODE_FORMAT_ARRAY: c_int = 7;
/// A node whose own `format` says it is a map.
pub const NODE_FORMAT_MAP: c_int = 8;

/// `mpv_node`'s name for its automatic "let the OS choose" entry, which
/// `audio-device-list` puts first. The app maps it to the empty device name so
/// it collapses into the picker's own "System default" row.
pub const AUTO_DEVICE: &str = "auto";

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

/// C layout of `mpv_node_list`, and of `mpv_node_map` — see [`RawNodeMap`].
#[repr(C)]
struct RawNodeList {
    num: c_int,
    values: *mut RawNode,
    /// Null for a true array; populated for a map.
    keys: *mut *mut c_char,
}

/// C layout of `mpv_node_map`, what a node tagged [`NODE_FORMAT_MAP`] points at.
///
/// **Identical to [`RawNodeList`]**, and that is not a typo: verified against the
/// libmpv this app ships, whose map node reads as `num, values, keys` — the same
/// order as the list. Believing the two were reversed (as some transcriptions of
/// `client.h` suggest) makes every `keys` pointer land on a `mpv_node*`, which
/// then gets dereferenced as a string and crashes the process with
/// `STATUS_ACCESS_VIOLATION`. `mpv_node_list` is used for both shapes so the
/// layout exists in exactly one place.
type RawNodeMap = RawNodeList;

/// C layout of `mpv_node`: a format tag plus a union whose largest member is a
/// pointer, so the struct is one 8-byte slot plus the 4-byte tag on x64.
#[repr(C)]
struct RawNode {
    /// The union is read as the pointer type of whichever arm the format selects.
    value: *mut c_void,
    format: c_int,
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
type FreeNodeContentsFn = unsafe extern "C" fn(*mut RawNode);

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
    free_node_contents: FreeNodeContentsFn,
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

        // Absolute candidates anchored at the executable's directory. Tauri v2
        // places `bundle.resources` next to the exe, preserving the declared
        // subpath (`src-tauri/tauri.conf.json` → `libmpv/libmpv-2.dll`), so a
        // plain DLL name alone can never find it once installed.
        let mut candidates: Vec<std::ffi::OsString> = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                candidates.push(dir.join("libmpv").join("libmpv-2.dll").into_os_string());
                candidates.push(dir.join("libmpv-2.dll").into_os_string());
            }
        }
        // Fallbacks: bare names resolve through Windows' standard DLL search
        // order (app dir, then PATH), which covers running from the project
        // tree where the DLL sits flat next to the exe.
        for name in ["libmpv-2.dll", "mpv-2.dll", "libmpv.dll", "libmpv.so.2"] {
            candidates.push(name.into());
        }
        for path in &candidates {
            // SAFETY: same rationale as above — libmpv is loaded at runtime.
            if let Ok(lib) = unsafe { Library::new(path) } {
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
            // Present in every libmpv 2.x build; without it a node read could
            // not release mpv's tree, so treat a missing symbol as a hard error
            // rather than leaking on every read.
            let free_node_contents: FreeNodeContentsFn = *lib
                .get(b"mpv_free_node_contents")
                .map_err(|e| format!("mpv_free_node_contents: {e}"))?;
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
                free_node_contents,
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

    /// Read a list-of-options property (`audio-device-list` and friends).
    ///
    /// Each entry comes back as its key plus whatever string values it contains,
    /// so the caller does not have to know mpv's exact layout — mpv has shipped
    /// this property both as an array of maps and as a map of maps.
    ///
    /// `Err(rc)` when mpv refuses the read; `Ok(vec![])` when the property is
    /// valid but empty (no audio device on this machine).
    pub fn get_option_list(&self, name: &str) -> Result<Vec<Vec<(String, String)>>, c_int> {
        let mut node = RawNode {
            value: std::ptr::null_mut(),
            format: 0,
        };
        let rc = self.get_prop(name, FORMAT_NODE, (&mut node as *mut RawNode).cast());
        if rc < 0 {
            return Err(rc);
        }
        // SAFETY: on success mpv filled `node` with a tree it owns; we read it
        // and then hand it back exactly once via mpv_free_node_contents.
        let entries = unsafe { Self::read_entries(&node) };
        // SAFETY: `node` was allocated by mpv and is released once, on every
        // path out of here.
        unsafe { (self.free_node_contents)(&mut node) };
        Ok(entries)
    }

    /// Split the root node into one entry per element, whichever shape it has.
    ///
    /// # Safety
    /// `node` must point at a tree mpv just handed over and not yet freed.
    unsafe fn read_entries(node: &RawNode) -> Vec<Vec<(String, String)>> {
        if node.value.is_null() {
            return Vec::new();
        }
        match node.format {
            // An array of entries. mpv keeps no keys for an array.
            NODE_FORMAT_ARRAY => {
                // SAFETY: the tag says array, which is `mpv_node_list`'s layout.
                let list = unsafe { &*(node.value as *const RawNodeList) };
                let count = list.num.max(0) as usize;
                if count == 0 || list.values.is_null() {
                    return Vec::new();
                }
                // SAFETY: `num` elements are valid inside the tree mpv owns.
                let values = unsafe { std::slice::from_raw_parts(list.values, count) };
                values
                    .iter()
                    .map(|value| {
                        let mut entry = Vec::new();
                        // SAFETY: `value` is a node inside the tree mpv owns.
                        unsafe { Self::collect_strings(value, &mut entry, 0) };
                        entry
                    })
                    .collect()
            }
            // A map of entries, keyed by the mpv option name.
            NODE_FORMAT_MAP => {
                // SAFETY: the tag says map; `RawNodeMap` is `RawNodeList`, whose
                // field order is the one verified against this libmpv.
                let map = unsafe { &*(node.value as *const RawNodeMap) };
                let count = map.num.max(0) as usize;
                if count == 0 || map.values.is_null() {
                    return Vec::new();
                }
                // SAFETY: `num` elements are valid inside the tree mpv owns.
                let values = unsafe { std::slice::from_raw_parts(map.values, count) };
                let keys = unsafe { Self::read_keys(map.keys, count) };
                values
                    .iter()
                    .enumerate()
                    .map(|(i, value)| {
                        let mut entry = Vec::new();
                        // The outer key is mpv's option name ("wasapi/0"), not
                        // something to show. It is recorded first so a lookup of
                        // "name"/"description" finds the entry's own fields and
                        // not this.
                        entry.push((keys.get(i).cloned().unwrap_or_default(), String::new()));
                        // SAFETY: `value` is a node inside the tree mpv owns.
                        unsafe { Self::collect_strings(value, &mut entry, 0) };
                        entry
                    })
                    .collect()
            }
            // Not a list at all: report it as a single entry, so a caller that
            // expected several can still see what mpv said.
            _ => {
                let mut entry = Vec::new();
                // SAFETY: `node` is the tree mpv owns.
                unsafe { Self::collect_strings(node, &mut entry, 0) };
                if entry.is_empty() {
                    Vec::new()
                } else {
                    vec![entry]
                }
            }
        }
    }

    /// Copy up to `count` NUL-terminated keys out of an mpv tree.
    ///
    /// # Safety
    /// `ptr` must be null or point at `count` keys inside a live tree mpv owns.
    unsafe fn read_keys(ptr: *mut *mut c_char, count: usize) -> Vec<String> {
        if ptr.is_null() {
            return Vec::new();
        }
        // SAFETY: `count` keys are valid inside the tree mpv owns.
        unsafe { std::slice::from_raw_parts(ptr, count) }
            .iter()
            .map(|k| {
                if k.is_null() {
                    String::new()
                } else {
                    // SAFETY: keys are NUL-terminated strings mpv owns.
                    unsafe { std::ffi::CStr::from_ptr(*k) }
                        .to_string_lossy()
                        .into_owned()
                }
            })
            .collect()
    }

    /// Gather a node's string-valued keys into `out`, descending into nested
    /// containers. Depth is bounded so a malformed tree cannot spin here.
    ///
    /// # Safety
    /// `node` must belong to a live tree mpv owns.
    unsafe fn collect_strings(node: &RawNode, out: &mut Vec<(String, String)>, depth: usize) {
        if depth > 3 || node.value.is_null() {
            return;
        }
        match node.format {
            NODE_FORMAT_STRING => {
                // SAFETY: the tag says string; mpv NUL-terminates it.
                let text = unsafe { std::ffi::CStr::from_ptr(node.value as *const c_char) };
                out.push((String::new(), text.to_string_lossy().into_owned()));
            }
            NODE_FORMAT_MAP => {
                // SAFETY: the tag says map; `RawNodeMap` is `RawNodeList` (see its definition).
                let map = unsafe { &*(node.value as *const RawNodeMap) };
                let count = map.num.max(0) as usize;
                if count == 0 || map.values.is_null() {
                    return;
                }
                // SAFETY: `num` elements are valid inside the tree mpv owns.
                let values = unsafe { std::slice::from_raw_parts(map.values, count) };
                let keys = unsafe { Self::read_keys(map.keys, count) };
                for (i, value) in values.iter().enumerate() {
                    let key = keys.get(i).cloned().unwrap_or_default();
                    if value.format == NODE_FORMAT_STRING && !value.value.is_null() {
                        // SAFETY: the tag says string; mpv NUL-terminates it.
                        let text =
                            unsafe { std::ffi::CStr::from_ptr(value.value as *const c_char) };
                        out.push((key, text.to_string_lossy().into_owned()));
                    } else {
                        out.push((key, String::new()));
                        // SAFETY: `value` is a node inside the tree mpv owns.
                        unsafe { Self::collect_strings(value, out, depth + 1) };
                    }
                }
            }
            // Numbers, flags, arrays: nothing a caller can show as text.
            _ => {}
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
