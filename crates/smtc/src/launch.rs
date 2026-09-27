//! Locate and launch a Music Presence installation.
//!
//! [Music Presence](https://github.com/ungive/discord-music-presence) is the
//! separate tray app that turns the `iwaks-smtc` session into a Discord
//! status ("Listening to ..."). It is not a library Iwaks could embed, so to
//! "start together with the app" Iwaks detects an existing installation and
//! spawns it. Detection is layered:
//!
//! 1. Windows uninstall registry — entries whose `DisplayName` contains
//!    "music presence" contribute their `InstallLocation` (a directory) and
//!    `DisplayIcon` (an exe path, `,N` icon index stripped);
//! 2. well-known per-user install dirs under `%LOCALAPPDATA%` (the Tauri /
//!    electron-builder NSIS installers both install per-user), plus
//!    `%ProgramFiles%` / `%ProgramFiles(x86)%` for machine-wide installs.
//!
//! The executable is looked up as `Music Presence.exe` / `musicpresence.exe`
//! (the installer and the portable zip both ship it under those names).
//!
//! Only the registry read needs Windows; every probing step is plain
//! [`std::path`] so it is fully unit-tested on all platforms.

use std::path::{Path, PathBuf};

/// Executable names Music Presence ships under (installer + portable zip).
/// Windows paths are case-insensitive, so casing here is best-effort.
const EXE_NAMES: [&str; 2] = ["Music Presence.exe", "musicpresence.exe"];

/// Subdirectories of `%LOCALAPPDATA%` where the installers commonly put the
/// app (Tauri v2 NSIS installs per-user directly under AppData\Local;
/// electron-builder uses a `Programs\` subdirectory).
const LOCALAPPDATA_SUBDIRS: [&str; 5] = [
    "Programs\\Music Presence",
    "Music Presence",
    "Music Presence\\latest",
    "Programs\\MusicPresence",
    "MusicPresence",
];

/// Subdirectories of `%ProgramFiles%` / `%ProgramFiles(x86)%` for machine-wide
/// installs (per-user is the default for NSIS, but cover both).
const PROGRAM_FILES_SUBDIRS: [&str; 2] = ["Music Presence", "MusicPresence"];

/// One uninstall-registry entry that looks like Music Presence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayEntry {
    pub install_location: Option<String>,
    pub display_icon: Option<String>,
}

/// First existing Music Presence executable among `candidates`: a candidate
/// may be the exe itself (registry `DisplayIcon`) or a directory to probe for
/// the known exe names (registry `InstallLocation`, well-known dirs).
pub fn search(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find_map(|c| {
        if c.is_file() {
            Some(c.clone())
        } else {
            probe_dir(c)
        }
    })
}

/// First known Music Presence exe inside `dir`, or `None`.
pub fn probe_dir(dir: &Path) -> Option<PathBuf> {
    EXE_NAMES
        .iter()
        .map(|name| dir.join(name))
        .find(|p| p.is_file())
}

/// True when an uninstall entry's `DisplayName` refers to Music Presence.
pub fn is_music_presence_name(name: &str) -> bool {
    name.to_ascii_lowercase().contains("music presence")
}

/// Strip a trailing icon index from a `DisplayIcon` value
/// (`C:\…\Music Presence.exe,0` → the exe path). Non-exe or non-numeric
/// suffixes are left untouched.
pub fn strip_icon_index(value: &str) -> &str {
    if let Some((head, tail)) = value.rsplit_once(',') {
        if tail.chars().all(|c| c.is_ascii_digit()) && head.to_ascii_lowercase().ends_with(".exe") {
            return head;
        }
    }
    value
}

/// Candidate paths derived from uninstall entries: each `InstallLocation`
/// (directory) plus each `DisplayIcon` (exe path). Empty / whitespace-only
/// values are skipped.
pub fn entry_candidates(entries: &[DisplayEntry]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in entries {
        if let Some(path) = entry
            .install_location
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            out.push(PathBuf::from(path));
        }
        if let Some(path) = entry
            .display_icon
            .as_deref()
            .map(strip_icon_index)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            out.push(PathBuf::from(path));
        }
    }
    out
}

/// Well-known install directories under the given environment roots.
pub fn well_known_candidates(
    appdata: Option<&Path>,
    program_files: Option<&Path>,
    program_files_x86: Option<&Path>,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(base) = appdata {
        out.extend(LOCALAPPDATA_SUBDIRS.iter().map(|sub| base.join(sub)));
    }
    for base in [program_files, program_files_x86].into_iter().flatten() {
        out.extend(PROGRAM_FILES_SUBDIRS.iter().map(|sub| base.join(sub)));
    }
    out
}

/// Find an installed Music Presence executable, or `None`.
#[cfg(windows)]
pub fn find_installation() -> Option<PathBuf> {
    let appdata = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let program_files = std::env::var_os("ProgramFiles").map(PathBuf::from);
    let program_files_x86 = std::env::var_os("ProgramFiles(x86)").map(PathBuf::from);
    let mut candidates = entry_candidates(&registry_entries());
    candidates.extend(well_known_candidates(
        appdata.as_deref(),
        program_files.as_deref(),
        program_files_x86.as_deref(),
    ));
    search(&candidates)
}

/// No-op outside Windows (Music Presence is a desktop tray app).
#[cfg(not(windows))]
pub fn find_installation() -> Option<PathBuf> {
    None
}

/// Launch the app detached — the returned handle is dropped immediately and
/// the process keeps running on its own.
pub fn spawn(exe: &Path) -> std::io::Result<std::process::Child> {
    std::process::Command::new(exe).spawn()
}

/// Uninstall-registry entries whose display name mentions Music Presence
/// (per-user HKCU plus machine-wide HKLM, including the 32-bit view).
#[cfg(windows)]
fn registry_entries() -> Vec<DisplayEntry> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
    use winreg::RegKey;

    const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
    let roots = [
        (RegKey::predef(HKEY_CURRENT_USER), UNINSTALL),
        (RegKey::predef(HKEY_LOCAL_MACHINE), UNINSTALL),
        (
            RegKey::predef(HKEY_LOCAL_MACHINE),
            r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
    ];
    let mut out = Vec::new();
    for (root, subkey) in roots {
        let Ok(uninstall) = root.open_subkey_with_flags(subkey, KEY_READ) else {
            continue;
        };
        for name in uninstall.enum_keys().flatten() {
            let Ok(entry) = uninstall.open_subkey_with_flags(&name, KEY_READ) else {
                continue;
            };
            let display: Option<String> = entry.get_value("DisplayName").ok();
            if !display.as_deref().is_some_and(is_music_presence_name) {
                continue;
            }
            out.push(DisplayEntry {
                install_location: entry.get_value("InstallLocation").ok(),
                display_icon: entry.get_value("DisplayIcon").ok(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Fresh scratch dir per test (pid-scoped to avoid collisions).
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("iwaks-launch-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn probe_dir_finds_exe_with_space() {
        let dir = scratch("a");
        let exe = dir.join("Music Presence.exe");
        fs::write(&exe, b"x").expect("write");
        assert_eq!(probe_dir(&dir), Some(exe));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn probe_dir_finds_single_word_exe() {
        let dir = scratch("b");
        let exe = dir.join("musicpresence.exe");
        fs::write(&exe, b"x").expect("write");
        assert_eq!(probe_dir(&dir), Some(exe));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn probe_dir_empty_is_none() {
        let dir = scratch("c");
        assert_eq!(probe_dir(&dir), None);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn search_probes_dirs_and_accepts_direct_exe() {
        let filled = scratch("d");
        let exe = filled.join("Music Presence.exe");
        fs::write(&exe, b"x").expect("write");
        let empty = scratch("e");

        // Directory first (probe inside), then a direct exe path.
        assert_eq!(search(&[filled.clone(), empty.clone()]), Some(exe.clone()));
        // Direct exe path wins when listed first; order matters (first hit).
        assert_eq!(search(&[exe.clone(), empty.clone()]), Some(exe.clone()));
        // Nothing to find.
        assert_eq!(search(std::slice::from_ref(&empty)), None);

        fs::remove_dir_all(&filled).ok();
        fs::remove_dir_all(&empty).ok();
    }

    #[test]
    fn entry_candidates_keeps_location_and_strips_icon_index() {
        let entries = [
            DisplayEntry {
                install_location: Some("C:\\Users\\me\\AppData\\Local\\Music Presence".to_string()),
                display_icon: Some(
                    "C:\\Users\\me\\AppData\\Local\\Music Presence\\Music Presence.exe,0"
                        .to_string(),
                ),
            },
            DisplayEntry {
                install_location: Some("   ".to_string()),
                display_icon: None,
            },
        ];
        let candidates = entry_candidates(&entries);
        assert_eq!(candidates.len(), 2);
        assert_eq!(
            candidates[0],
            PathBuf::from("C:\\Users\\me\\AppData\\Local\\Music Presence")
        );
        assert_eq!(
            candidates[1],
            PathBuf::from("C:\\Users\\me\\AppData\\Local\\Music Presence\\Music Presence.exe")
        );
    }

    #[test]
    fn strip_icon_index_only_touches_exe_comma_number() {
        assert_eq!(
            strip_icon_index(r"C:\a\Music Presence.exe,0"),
            r"C:\a\Music Presence.exe"
        );
        assert_eq!(
            strip_icon_index(r"C:\a\Music Presence.exe"),
            r"C:\a\Music Presence.exe"
        );
        assert_eq!(strip_icon_index(r"C:\a\icon.ico,0"), r"C:\a\icon.ico,0");
        assert_eq!(strip_icon_index(r"C:\a\x.exe,abc"), r"C:\a\x.exe,abc");
    }

    #[test]
    fn name_match_is_case_insensitive_substring() {
        assert!(is_music_presence_name("Music Presence"));
        assert!(is_music_presence_name("music presence 2.3.6"));
        assert!(!is_music_presence_name("Spotify"));
        assert!(!is_music_presence_name(""));
    }

    #[test]
    fn well_known_candidates_builds_dir_list() {
        let appdata = scratch("appdata");
        let pf = scratch("pf");
        let candidates = well_known_candidates(Some(&appdata), Some(&pf), None);
        assert_eq!(
            candidates.len(),
            LOCALAPPDATA_SUBDIRS.len() + PROGRAM_FILES_SUBDIRS.len()
        );
        assert!(candidates.contains(&appdata.join("Music Presence")));
        assert!(candidates.contains(&pf.join("MusicPresence")));
        fs::remove_dir_all(&appdata).ok();
        fs::remove_dir_all(&pf).ok();
    }

    #[test]
    fn end_to_end_fake_install_is_found() {
        // Registry points at a real dir containing the exe; the well-known
        // dirs stay empty. The pure search must resolve it.
        let base = scratch("e2e");
        let install = base.join("Music Presence");
        fs::create_dir_all(&install).expect("mkdir");
        let exe = install.join("Music Presence.exe");
        fs::write(&exe, b"x").expect("write");
        let entries = [DisplayEntry {
            install_location: Some(install.to_string_lossy().into_owned()),
            display_icon: None,
        }];
        let mut candidates = entry_candidates(&entries);
        candidates.extend(well_known_candidates(
            Some(&base.join("AppData")),
            None,
            None,
        ));
        assert_eq!(search(&candidates), Some(exe));
        fs::remove_dir_all(&base).ok();
    }
}
