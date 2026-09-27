//! Windows implementation of the SMTC session.
//!
//! One dedicated thread owns the WinRT objects (`MediaPlayer` + its
//! `SystemMediaTransportControls`) for its whole lifetime, so COM/WinRT
//! apartment rules never leak into the rest of the app: everything else just
//! sends [`NowPlaying`] snapshots over a channel and registers a callback
//! for button presses. Non-Windows targets get a no-op stub so the workspace
//! still compiles everywhere.

use crate::{secs_to_100ns, silent_wav, NowPlaying, SmtcButton};

#[cfg(windows)]
pub mod imp {
    use super::*;
    use std::sync::{mpsc, Arc, Mutex};
    use windows::core::HSTRING;
    use windows::Foundation::{TimeSpan, TypedEventHandler, Uri};
    use windows::Media::Playback::MediaPlayer;
    use windows::Media::{
        MediaPlaybackStatus, MediaPlaybackType, SystemMediaTransportControls,
        SystemMediaTransportControlsButton, SystemMediaTransportControlsButtonPressedEventArgs,
        SystemMediaTransportControlsTimelineProperties,
    };
    use windows::Storage::Streams::RandomAccessStreamReference;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

    type ButtonFn = dyn Fn(SmtcButton) + Send + Sync;

    enum Msg {
        Update(Box<NowPlaying>),
        Cover(Option<Box<[u8]>>),
    }

    /// Handle to the SMTC session thread. Cheap to clone; `Send + Sync`
    /// because no WinRT call happens outside the session thread itself.
    #[derive(Clone)]
    pub struct SmtcSession {
        tx: mpsc::Sender<Msg>,
        handler: Arc<Mutex<Arc<ButtonFn>>>,
    }

    /// WinRT objects created and used only on the session thread.
    struct Core {
        _player: MediaPlayer,
        smtc: SystemMediaTransportControls,
        /// Current thumbnail source; `None` = no cover (clears the flyout).
        thumbnail: Option<RandomAccessStreamReference>,
        /// Last cover bytes applied — skips rewriting the temp file when the
        /// same cover (e.g. same album) is pushed again.
        last_cover: Option<Box<[u8]>>,
    }

    impl SmtcSession {
        /// Spawn the session thread and register the SMTC surface. Returns
        /// `None` if WinRT activation fails (locked-down / exotic systems).
        pub fn start() -> Option<Self> {
            let (tx, rx) = mpsc::channel::<Msg>();
            let noop: Arc<ButtonFn> = Arc::new(|_: SmtcButton| {});
            let handler = Arc::new(Mutex::new(noop));
            let thread_handler = Arc::clone(&handler);
            std::thread::Builder::new()
                .name("iwaks-smtc".into())
                .spawn(move || run(rx, thread_handler))
                .ok()?;
            Some(Self { tx, handler })
        }

        /// Push fresh playback state to the session — local-channel cheap,
        /// safe at the player's ~10 Hz tick rate.
        pub fn update(&self, np: &NowPlaying) {
            let _ = self.tx.send(Msg::Update(Box::new(np.clone())));
        }

        /// Push the current track's embedded cover art (raw image bytes).
        /// `None` clears the thumbnail. The app should call this once per
        /// track change; identical bytes are skipped on the session thread.
        pub fn set_cover(&self, cover: Option<Box<[u8]>>) {
            let _ = self.tx.send(Msg::Cover(cover));
        }

        /// Replace the callback invoked when the user presses a media key or
        /// a flyout button (play / pause / next / previous / stop).
        pub fn on_button(&self, f: Box<ButtonFn>) {
            *self.handler.lock().unwrap() = Arc::from(f);
        }
    }

    fn run(rx: mpsc::Receiver<Msg>, handler: Arc<Mutex<Arc<ButtonFn>>>) {
        // MTA + RoInitialize: MediaPlayer is agile and this thread is the
        // only one touching WinRT, so MTA is enough. Both calls return an
        // HRESULT; S_FALSE "already initialized" is not an error.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
        let Some(mut core) = build_core(handler) else {
            return;
        };
        while let Ok(msg) = rx.recv() {
            match msg {
                Msg::Update(np) => apply(&core, &np),
                Msg::Cover(cover) => set_cover(&mut core, cover),
            }
        }
    }

    fn build_core(handler: Arc<Mutex<Arc<ButtonFn>>>) -> Option<Core> {
        let player = MediaPlayer::new().ok()?;
        let smtc = player.SystemMediaTransportControls().ok()?;
        smtc.SetIsEnabled(true).ok()?;
        smtc.SetIsPlayEnabled(true).ok()?;
        smtc.SetIsPauseEnabled(true).ok()?;
        smtc.SetIsNextEnabled(true).ok()?;
        smtc.SetIsPreviousEnabled(true).ok()?;
        smtc.SetIsStopEnabled(true).ok()?;

        // A silent source keeps the session visible to the OS even before the
        // first real track. Volume 0 + never calling `Play` = no sound. The
        // WAV is a 2 KB temp file written once — avoids async stream I/O on
        // this dedicated thread entirely.
        if let Some(source) = silent_source() {
            player.SetSource(&source).ok()?;
            player.SetVolume(0.0).ok()?;
            player.SetIsLoopingEnabled(true).ok()?;
        }

        let button_handler = handler.clone();
        let pressed = TypedEventHandler::<
            SystemMediaTransportControls,
            SystemMediaTransportControlsButtonPressedEventArgs,
        >::new(move |_ctl, args| {
            let args = args.ok()?;
            let button = args.Button()?;
            let action = match button {
                SystemMediaTransportControlsButton::Play => SmtcButton::Play,
                SystemMediaTransportControlsButton::Pause => SmtcButton::Pause,
                SystemMediaTransportControlsButton::Next => SmtcButton::Next,
                SystemMediaTransportControlsButton::Previous => SmtcButton::Previous,
                SystemMediaTransportControlsButton::Stop => SmtcButton::Stop,
                _ => return Ok(()),
            };
            (button_handler.lock().unwrap())(action);
            Ok(())
        });
        smtc.ButtonPressed(&pressed).ok()?;

        Some(Core {
            _player: player,
            smtc,
            thumbnail: None,
            last_cover: None,
        })
    }

    /// Silent WAV written once to the temp dir and handed to the dummy
    /// player as a `file:` URI source (synchronous — no async WinRT I/O).
    fn silent_source() -> Option<windows::Media::Core::MediaSource> {
        let path = std::env::temp_dir().join("iwaks-smtc-silence.wav");
        std::fs::write(&path, silent_wav(2)).ok()?;
        let file_uri = format!("file:///{}", path.to_string_lossy().replace('\\', "/"));
        let uri = Uri::CreateUri(&HSTRING::from(file_uri)).ok()?;
        windows::Media::Core::MediaSource::CreateFromUri(&uri).ok()
    }

    /// Apply a new cover to the session: skip when bytes are unchanged (same
    /// album across tracks), otherwise write a fresh temp file and rebuild
    /// the stream reference. `None` clears the thumbnail.
    fn set_cover(core: &mut Core, cover: Option<Box<[u8]>>) {
        if cover == core.last_cover {
            return;
        }
        core.last_cover = cover;
        core.thumbnail = match &core.last_cover {
            Some(bytes) => thumbnail_from_bytes(bytes),
            None => None,
        };
    }

    /// Write the cover to the temp dir as a `file:` URI source and wrap it
    /// in a stream reference — synchronous, mirroring the silent-WAV trick.
    fn thumbnail_from_bytes(bytes: &[u8]) -> Option<RandomAccessStreamReference> {
        let ext = crate::cover_extension(bytes);
        let path = std::env::temp_dir().join(format!("iwaks-smtc-cover.{ext}"));
        std::fs::write(&path, bytes).ok()?;
        let file_uri = format!("file:///{}", path.to_string_lossy().replace('\\', "/"));
        let uri = Uri::CreateUri(&HSTRING::from(file_uri)).ok()?;
        RandomAccessStreamReference::CreateFromUri(&uri).ok()
    }

    fn apply(core: &Core, np: &NowPlaying) {
        let smtc = &core.smtc;
        let status = match np.status() {
            crate::PlayStatus::Playing => MediaPlaybackStatus::Playing,
            crate::PlayStatus::Paused => MediaPlaybackStatus::Paused,
            crate::PlayStatus::Stopped => MediaPlaybackStatus::Stopped,
        };
        let _ = smtc.SetPlaybackStatus(status);
        let _ = (|| -> windows::core::Result<()> {
            let updater = smtc.DisplayUpdater()?;
            updater.SetType(MediaPlaybackType::Music)?;
            let props = updater.MusicProperties()?;
            let (title, artist, album) = np.metadata();
            let (h_title, h_artist, h_album) = (
                HSTRING::from(title),
                HSTRING::from(artist),
                HSTRING::from(album),
            );
            props.SetTitle(&h_title)?;
            props.SetArtist(&h_artist)?;
            props.SetAlbumTitle(&h_album)?;
            updater.SetThumbnail(core.thumbnail.as_ref())?;
            updater.Update()?;
            // Timeline = the progress bar the flyout / Music Presence draws.
            let end = TimeSpan {
                Duration: secs_to_100ns(np.duration_secs),
            };
            let pos = TimeSpan {
                Duration: secs_to_100ns(np.position_secs),
            };
            let tl = SystemMediaTransportControlsTimelineProperties::new()?;
            tl.SetStartTime(TimeSpan { Duration: 0 })?;
            tl.SetMinSeekTime(TimeSpan { Duration: 0 })?;
            tl.SetPosition(pos)?;
            tl.SetMaxSeekTime(end)?;
            tl.SetEndTime(end)?;
            smtc.UpdateTimelineProperties(&tl)?;
            Ok(())
        })();
    }
}

#[cfg(not(windows))]
pub mod imp {
    use crate::{NowPlaying, SmtcButton};

    /// No-op stub for non-Windows builds.
    #[derive(Clone, Default)]
    pub struct SmtcSession;

    impl SmtcSession {
        pub fn start() -> Option<Self> {
            None
        }
        pub fn update(&self, _np: &NowPlaying) {}
        pub fn set_cover(&self, _cover: Option<Box<[u8]>>) {}
        pub fn on_button(&self, _f: Box<dyn Fn(SmtcButton) + Send + Sync>) {}
    }
}

#[cfg(windows)]
pub use imp::SmtcSession;

#[cfg(not(windows))]
pub use imp::SmtcSession;

#[cfg(all(test, windows))]
mod verify {
    use super::*;
    use crate::NowPlaying;
    use std::time::{Duration, Instant};
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
    use windows_future::AsyncStatus;

    /// End-to-end proof that the SMTC session registers with the OS: start a
    /// session, publish a fake "now playing", then enumerate every session
    /// the system sees and require one whose source app id contains "iwaks".
    /// Manual on a real Windows desktop:
    /// `cargo test -p iwaks-smtc -- --ignored --nocapture`
    #[test]
    #[ignore = "interactive Windows desktop check; not part of CI"]
    fn session_appears_in_os_enumeration() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
        let session = SmtcSession::start().expect("session must start on this machine");
        session.update(&NowPlaying {
            title: "Iwaks SMTC Verification".into(),
            artist: Some("Music Presence".into()),
            album: None,
            position_secs: 42.0,
            duration_secs: 200.0,
            playing: true,
            stopped: false,
        });
        // Give the session thread time to register and apply the update.
        std::thread::sleep(Duration::from_millis(500));

        // RequestAsync returns windows_future::IAsyncOperation, which only
        // awaits via IntoFuture (needs a runtime) — poll Status() instead.
        let op = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
            .expect("request manager");
        let deadline = Instant::now() + Duration::from_secs(5);
        let manager = loop {
            match op.Status().expect("status") {
                AsyncStatus::Completed => break op.GetResults().expect("results"),
                AsyncStatus::Error => {
                    panic!("session manager request failed: {:?}", op.ErrorCode())
                }
                AsyncStatus::Canceled => panic!("session manager request canceled"),
                _ => {
                    assert!(
                        Instant::now() < deadline,
                        "session manager request timed out"
                    );
                    std::thread::sleep(Duration::from_millis(25));
                }
            }
        };
        let sessions = manager.GetSessions().expect("sessions");
        let mut found: Vec<String> = Vec::new();
        let mut matched = false;
        for i in 0..sessions.Size().unwrap_or(0) {
            let session = sessions.GetAt(i).expect("session");
            let app_id = session
                .SourceAppUserModelId()
                .map(|h| h.to_string())
                .unwrap_or_default();
            if app_id.to_lowercase().contains("iwaks") {
                matched = true;
            }
            found.push(app_id);
        }
        eprintln!("SMTC sessions visible to the OS: {found:?}");
        assert!(
            matched,
            "no session with an iwaks source app id; found: {found:?}"
        );
    }
}
