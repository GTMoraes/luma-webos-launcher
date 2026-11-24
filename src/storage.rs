use crate::model::Preferences;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Android TV: the app's private folder (see `prepare_private`).
#[cfg(target_os = "android")]
pub fn default_path(_webos: bool) -> Result<PathBuf, String> {
    crate::android::preferences_path()
}

#[cfg(not(target_os = "android"))]
pub fn default_path(webos: bool) -> Result<PathBuf, String> {
    if webos {
        #[cfg(all(feature = "webos", unix))]
        return tv_path();
        #[cfg(not(all(feature = "webos", unix)))]
        return Err("TV storage requires a webOS build".into());
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
        .ok_or("Set HOME, XDG_CONFIG_HOME or --preferences for native preferences")?;
    Ok(base.join("luma-native/preferences.json"))
}

#[cfg(all(feature = "webos", unix))]
fn tv_path() -> Result<PathBuf, String> {
    use std::{
        io::Write,
        os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    };

    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let legacy = executable.with_file_name("preferences.json");
    let id = crate::platform::APP_ID;
    // These volumes persist across boots. The app-specific directory also
    // survives replacement of the installed payload on an ordinary update.
    // Production native jails expose internal storage instead of developer.
    let developer = PathBuf::from("/media/developer")
        .join(id)
        .join("preferences.json");
    let shared_developer = PathBuf::from("/media/developer/temp")
        .join(id)
        .join("preferences.json");
    let internal = PathBuf::from("/media/internal")
        .join(format!(".{id}"))
        .join("preferences.json");
    let mut candidates = if executable.starts_with("/media/cryptofs") {
        [internal, developer, shared_developer]
    } else {
        [developer, shared_developer, internal]
    };
    // Keep a previously used destination even if another candidate becomes
    // writable after a reboot; do not accidentally restart with fresh settings.
    candidates.sort_by_key(|path| !path.exists());
    let mut failures = Vec::new();
    for path in candidates {
        let directory = path.parent().ok_or("Invalid TV storage directory")?;
        // Create only this app's leaf, not a missing system mount hierarchy.
        let created = fs::DirBuilder::new().mode(0o700).create(directory);
        if let Err(error) = created
            && error.kind() != std::io::ErrorKind::AlreadyExists
        {
            failures.push(format!("{}: {error}", directory.display()));
            continue;
        }
        let metadata = match fs::symlink_metadata(directory) {
            Ok(metadata) => metadata,
            Err(error) => {
                failures.push(format!("{}: {error}", directory.display()));
                continue;
            }
        };
        // Do not silently reuse a symlink or root-created debug directory as
        // another process's private app storage.
        if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
            failures.push(format!(
                "{} is not owned by the app user",
                directory.display()
            ));
            continue;
        }
        let probe = directory.join(format!(".write-probe-{}", std::process::id()));
        let writable = (|| {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&probe)?;
            let result = file
                .write_all(b"Luma storage probe")
                .and_then(|_| file.sync_all());
            drop(file);
            // Only remove a probe that this process created successfully.
            let cleanup = fs::remove_file(&probe);
            result.and(cleanup)
        })();
        if let Err(error) = writable {
            failures.push(format!("{}: {error}", directory.display()));
            continue;
        }
        if !path.exists() {
            // Import JSON only from the old path in THIS installation, never
            // browser storage or another app ID's preferences.
            match fs::read(&legacy) {
                Ok(bytes) => {
                    let mut preferences: Preferences =
                        serde_json::from_slice(&bytes).map_err(|e| {
                            format!(
                                "Existing preferences {} could not be read: {e}",
                                legacy.display()
                            )
                        })?;
                    migrate_selection(&bytes, &mut preferences);
                    preferences.sanitize();
                    save(&path, &preferences)?;
                    eprintln!(
                        "Luma preferences migrated from {} to {}",
                        legacy.display(),
                        path.display()
                    );
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!(
                        "Existing preferences {} could not be read: {error}",
                        legacy.display()
                    ));
                }
            }
            let old_cache = crate::weather_service::cache_path(&legacy);
            let cache = crate::weather_service::cache_path(&path);
            if !cache.exists()
                && let Some(reading) = crate::weather_service::load_cache(&old_cache)
                && let Err(error) = crate::weather_service::save_cache(&cache, &reading)
            {
                eprintln!("Luma weather cache migration: {error}");
            }
        }
        eprintln!("Luma persistent storage: {}", directory.display());
        return Ok(path);
    }
    Err(format!(
        "No writable persistent TV storage found: {}",
        failures.join("; ")
    ))
}

pub fn load(path: &Path) -> Result<Preferences, String> {
    match fs::read(path) {
        Ok(bytes) => {
            let mut preferences: Preferences = serde_json::from_slice(&bytes)
                .map_err(|e| format!("Preferences could not be read: {e}"))?;
            migrate_selection(&bytes, &mut preferences);
            preferences.sanitize();
            Ok(preferences)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Preferences::default()),
        Err(e) => Err(format!("Preferences could not be read: {e}")),
    }
}

pub fn save(path: &Path, preferences: &Preferences) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "Could not prepare preferences directory {}: {e}",
                parent.display()
            )
        })?;
    }
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(preferences)
        .map_err(|e| format!("Could not encode preferences: {e}"))?;
    fs::write(&temporary, bytes).map_err(|e| {
        format!(
            "Could not write preferences temporary file {}: {e}",
            temporary.display()
        )
    })?;
    fs::rename(&temporary, path)
        .map_err(|e| format!("Could not replace preferences file {}: {e}", path.display()))?;
    #[cfg(target_os = "android")]
    crate::android::after_save(path);
    Ok(())
}

const PREFERENCES: &str = "preferences.json";
/// Written once the settings left the shared folder; the copy there is a backup from then on.
const MOVED: &str = "preferences.moved-from-shared";
/// Dropped in the shared folder with adb: replaces the settings once, then is removed.
pub const IMPORT: &str = "preferences-importar.json";

/// Android TV: the settings live in the app's private folder, which is ready
/// whenever Luma starts. The shared folder (reachable with adb) may still be
/// unmounted at boot: on 24/11/2026 Luma opened with the defaults after a cold
/// boot of the Chromecast. The shared folder keeps the log, a copy of the
/// settings after each change (`mirror`) and an import slot (`IMPORT`).
///
/// The first time the shared folder is there, the settings it holds are moved
/// in, over any file the private folder already has (a cold boot could have
/// left one with the defaults). Returns what happened, for the log.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn prepare_private(private: &Path, shared: Option<&Path>) -> Vec<String> {
    let mut notes = Vec::new();
    let Some(shared) = shared else {
        notes.push(
            "shared folder not ready: settings from the private folder, no copy this time".into(),
        );
        return notes;
    };
    let target = private.join(PREFERENCES);
    let moved = private.join(MOVED);
    if !moved.exists() {
        let old = shared.join(PREFERENCES);
        if old.is_file() {
            if let Err(error) = copy_checked(&old, &target) {
                notes.push(format!("settings not moved to the private folder: {error}"));
                return notes;
            }
            copy_caches(shared, private);
            notes.push(format!("settings moved to {}", target.display()));
        }
        if let Err(error) = fs::create_dir_all(private).and_then(|()| fs::write(&moved, b"1\n")) {
            notes.push(format!("could not mark the move: {error}"));
        }
    }
    let import = shared.join(IMPORT);
    if import.is_file() {
        match copy_checked(&import, &target) {
            Ok(()) => {
                let _ = fs::remove_file(&import);
                mirror(&target, shared);
                notes.push(format!("{IMPORT} imported"));
            }
            Err(error) => notes.push(format!("{IMPORT} not imported: {error}")),
        }
    }
    notes
}

/// Copies the saved settings to the shared folder (backup and adb access).
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn mirror(saved: &Path, shared: &Path) {
    let copy = shared.join(PREFERENCES);
    let temporary = copy.with_extension("json.tmp");
    if fs::copy(saved, &temporary).is_ok() {
        let _ = fs::rename(&temporary, &copy);
    } else {
        let _ = fs::remove_file(&temporary);
    }
}

/// Only a file Luma can read replaces the settings.
fn copy_checked(from: &Path, to: &Path) -> Result<(), String> {
    let bytes = fs::read(from).map_err(|e| format!("{}: {e}", from.display()))?;
    serde_json::from_slice::<Preferences>(&bytes)
        .map_err(|e| format!("{} is not a Luma preferences file: {e}", from.display()))?;
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let temporary = to.with_extension("json.tmp");
    fs::write(&temporary, &bytes).map_err(|e| format!("{}: {e}", temporary.display()))?;
    fs::rename(&temporary, to).map_err(|e| format!("{}: {e}", to.display()))
}

/// The weather cache and the app icons follow the settings (they are named after them).
fn copy_caches(shared: &Path, private: &Path) {
    let weather = Path::new(PREFERENCES).with_extension("weather.json");
    let _ = fs::copy(shared.join(&weather), private.join(&weather));
    let artwork = Path::new(PREFERENCES).with_extension("artwork");
    if let Ok(entries) = fs::read_dir(shared.join(&artwork)) {
        let folder = private.join(&artwork);
        if fs::create_dir_all(&folder).is_ok() {
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|kind| kind.is_file()) {
                    let _ = fs::copy(entry.path(), folder.join(entry.file_name()));
                }
            }
        }
    }
}

/// A missing modern field (not an explicitly empty set) activates one-time migration.
fn migrate_selection(bytes: &[u8], preferences: &mut Preferences) {
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) {
        // Migrate the original five paired presets only if no independent choice was saved.
        if value.get("heading_font").is_none() && value.get("text_font").is_none() {
            let old = value
                .get("font_pair")
                .and_then(|v| v.as_str())
                .unwrap_or("cinema");
            use crate::typography::{HeadingFont as H, TextFont as T};
            let (h, t) = match old {
                "editorial" => (H::Fraunces, T::Inter),
                "geometric" => (H::Unbounded, T::Manrope),
                "classic" | "original" => (H::Inter, T::Inter),
                _ => (H::Archivo, T::Inter),
            };
            preferences.heading_font = h;
            preferences.text_font = t;
        }
    }
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes)
        && value.get("video_excluded").is_none()
    {
        let permitted = crate::video_catalog::filtered(preferences.video_sources_mask);
        for (i, video) in crate::video_catalog::all().iter().enumerate() {
            if !permitted.contains(&i) {
                preferences.video_excluded.insert(video.id.clone());
            }
        }
    }
}

#[cfg(test)]
mod private_folder_tests {
    use super::*;

    fn folders(name: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("luma-private-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let private = root.join("files");
        let shared = root.join("sdcard");
        fs::create_dir_all(&private).unwrap();
        fs::create_dir_all(&shared).unwrap();
        (private, shared)
    }

    fn pretty() -> Vec<u8> {
        serde_json::to_vec_pretty(&Preferences::default()).unwrap()
    }

    fn compact() -> Vec<u8> {
        serde_json::to_vec(&Preferences::default()).unwrap()
    }

    #[test]
    fn first_start_moves_the_shared_settings_over_a_cold_boot_file() {
        let (private, shared) = folders("move");
        fs::write(shared.join(PREFERENCES), pretty()).unwrap();
        fs::write(shared.join("preferences.weather.json"), b"{\"clima\":1}").unwrap();
        fs::create_dir_all(shared.join("preferences.artwork")).unwrap();
        fs::write(shared.join("preferences.artwork/a.png"), b"png").unwrap();
        // left by the cold boot: defaults written to the private folder
        fs::write(private.join(PREFERENCES), compact()).unwrap();

        let notes = prepare_private(&private, Some(&shared));
        assert!(
            notes.iter().any(|n| n.starts_with("settings moved")),
            "{notes:?}"
        );
        assert_eq!(fs::read(private.join(PREFERENCES)).unwrap(), pretty());
        assert_eq!(
            fs::read(private.join("preferences.weather.json")).unwrap(),
            b"{\"clima\":1}"
        );
        assert_eq!(
            fs::read(private.join("preferences.artwork/a.png")).unwrap(),
            b"png"
        );
        assert!(private.join(MOVED).exists());

        // from then on the shared file is only a copy: it never comes back in
        fs::write(shared.join(PREFERENCES), compact()).unwrap();
        let notes = prepare_private(&private, Some(&shared));
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(fs::read(private.join(PREFERENCES)).unwrap(), pretty());
    }

    #[test]
    fn without_the_shared_folder_nothing_moves() {
        let (private, _shared) = folders("absent");
        fs::write(private.join(PREFERENCES), compact()).unwrap();
        let notes = prepare_private(&private, None);
        assert_eq!(notes.len(), 1);
        assert_eq!(fs::read(private.join(PREFERENCES)).unwrap(), compact());
        assert!(
            !private.join(MOVED).exists(),
            "the move waits for the shared folder"
        );
    }

    #[test]
    fn an_unreadable_shared_file_is_not_moved_and_is_tried_again() {
        let (private, shared) = folders("broken");
        fs::write(shared.join(PREFERENCES), b"{ not json").unwrap();
        let notes = prepare_private(&private, Some(&shared));
        assert!(
            notes.iter().any(|n| n.starts_with("settings not moved")),
            "{notes:?}"
        );
        assert!(!private.join(PREFERENCES).exists());
        assert!(!private.join(MOVED).exists());
    }

    #[test]
    fn a_fresh_install_just_marks_the_move() {
        let (private, shared) = folders("fresh");
        let notes = prepare_private(&private, Some(&shared));
        assert!(notes.is_empty(), "{notes:?}");
        assert!(private.join(MOVED).exists());
    }

    #[test]
    fn the_import_slot_replaces_the_settings_once() {
        let (private, shared) = folders("import");
        fs::write(private.join(MOVED), b"1\n").unwrap();
        fs::write(private.join(PREFERENCES), compact()).unwrap();
        fs::write(shared.join(IMPORT), pretty()).unwrap();
        let notes = prepare_private(&private, Some(&shared));
        assert_eq!(notes, vec![format!("{IMPORT} imported")]);
        assert_eq!(fs::read(private.join(PREFERENCES)).unwrap(), pretty());
        assert!(!shared.join(IMPORT).exists());
        assert_eq!(
            fs::read(shared.join(PREFERENCES)).unwrap(),
            pretty(),
            "copy refreshed"
        );

        fs::write(shared.join(IMPORT), b"nope").unwrap();
        let notes = prepare_private(&private, Some(&shared));
        assert!(notes[0].contains("not imported"), "{notes:?}");
        assert!(shared.join(IMPORT).exists(), "a bad file stays for a look");
        assert_eq!(fs::read(private.join(PREFERENCES)).unwrap(), pretty());
    }

    #[test]
    fn mirror_copies_the_saved_settings() {
        let (private, shared) = folders("mirror");
        let saved = private.join(PREFERENCES);
        save(&saved, &Preferences::default()).unwrap();
        mirror(&saved, &shared);
        assert_eq!(
            fs::read(shared.join(PREFERENCES)).unwrap(),
            fs::read(&saved).unwrap()
        );
        assert!(!shared.join("preferences.json.tmp").exists());
    }
}
