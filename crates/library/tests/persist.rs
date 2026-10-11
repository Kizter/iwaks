//! Persistence roundtrip (issue #01): player settings and the last-session
//! snapshot survive a close/reopen of the database file.

use std::path::PathBuf;

use iwaks_library::db::Library;
use iwaks_library::persist::{EqPreset, PlayerSettings, SessionSnapshot};

struct TempDir(PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("iwaks-persist-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        TempDir(dir)
    }
    fn path(&self, p: &str) -> PathBuf {
        self.0.join(p)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn settings() -> PlayerSettings {
    PlayerSettings {
        volume: 63,
        mute: true,
        repeat: "all".to_string(),
        shuffle: true,
        speed: 1.25,
        replaygain: "album".to_string(),
        eq_preamp: -3.5,
        eq: vec![1.0, -2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 3.5],
        sleep_seconds: Some(412.0),
    }
}

#[test]
fn settings_and_session_survive_reopen() {
    let tmp = TempDir::new("persist");
    let path = tmp.path("iwaks.db").to_string_lossy().to_string();

    let session = SessionSnapshot {
        paths: vec![
            r"C:\music\a.flac".to_string(),
            r"C:\music\b.flac".to_string(),
        ],
        index: 1,
        position: 12.5,
        shuffle: false,
    };

    {
        let lib = Library::open(&path).expect("open");
        // Nothing written yet → nothing to restore.
        assert_eq!(lib.load_player_settings().unwrap(), None);
        assert_eq!(lib.load_session().unwrap(), None);
        lib.save_player_settings(&settings()).unwrap();
        lib.save_session(&session).unwrap();
    }

    let lib = Library::open(&path).expect("reopen");
    assert_eq!(lib.load_player_settings().unwrap(), Some(settings()));
    assert_eq!(lib.load_session().unwrap(), Some(session));
}

#[test]
fn saving_settings_replaces_the_previous_value() {
    let lib = Library::open(":memory:").expect("open");
    let mut s = settings();
    lib.save_player_settings(&s).unwrap();
    s.volume = 10;
    s.repeat = "off".to_string();
    lib.save_player_settings(&s).unwrap();
    assert_eq!(lib.load_player_settings().unwrap(), Some(s));
}

fn eq_presets() -> Vec<EqPreset> {
    vec![
        EqPreset {
            name: "My Rock".to_string(),
            preamp: -1.0,
            gains: vec![4.0, 3.0, 1.0, -1.0, -2.0, -1.0, 1.0, 3.0, 4.0, 4.0],
        },
        EqPreset {
            name: "Podcast".to_string(),
            preamp: -2.0,
            gains: vec![-3.0, -2.0, -1.0, 1.0, 3.0, 4.0, 3.0, 1.0, 0.0, -1.0],
        },
    ]
}

#[test]
fn eq_presets_survive_reopen() {
    let tmp = TempDir::new("eq-presets");
    let path = tmp.path("iwaks.db").to_string_lossy().to_string();

    {
        let lib = Library::open(&path).expect("open");
        // Nothing written yet → an empty list, never an error.
        assert_eq!(lib.load_eq_presets().unwrap(), Vec::new());
        lib.save_eq_presets(&eq_presets()).unwrap();
    }

    let lib = Library::open(&path).expect("reopen");
    assert_eq!(lib.load_eq_presets().unwrap(), eq_presets());
}

#[test]
fn saving_eq_presets_replaces_the_previous_list() {
    let lib = Library::open(":memory:").expect("open");
    let first = vec![EqPreset {
        name: "A".to_string(),
        preamp: 0.0,
        gains: vec![0.0; 10],
    }];
    lib.save_eq_presets(&first).unwrap();
    lib.save_eq_presets(&eq_presets()).unwrap();
    assert_eq!(lib.load_eq_presets().unwrap(), eq_presets());

    // Saving an empty list clears the key rather than leaving the old one.
    lib.save_eq_presets(&[]).unwrap();
    assert_eq!(lib.load_eq_presets().unwrap(), Vec::new());
}
