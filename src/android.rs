//! Android TV (Chromecast with Google TV). SDLActivity loads libluma.so and
//! calls `SDL_main` on its own thread. `LumaActivity` (android/app) sets the
//! storage paths in the environment before SDL starts and answers the app
//! inventory, artwork and launch calls below over JNI.
use crate::model::{App, CATEGORIES, Inventory, valid_app_id};
use jni::{
    JNIEnv,
    objects::{JByteArray, JObject, JString, JValue},
};
use serde::Deserialize;
use std::{
    ffi::{c_char, c_int, c_long, c_void},
    os::fd::AsRawFd,
    path::{Path, PathBuf},
};

/// Luma's own package: never listed as an app to open.
const PACKAGE: &str = "com.frx9.luma";

unsafe extern "C" {
    fn SDL_AndroidGetJNIEnv() -> *mut c_void;
    fn SDL_AndroidGetActivity() -> *mut c_void;
    fn dup2(old: c_int, new: c_int) -> c_int;
    fn time(out: *mut c_long) -> c_long;
    fn localtime_r(clock: *const c_long, out: *mut Tm) -> *mut Tm;
}

/// Bionic's `struct tm`; `time_t` and `tm_gmtoff` are `long` on every Android ABI.
#[repr(C)]
struct Tm {
    tm_sec: c_int,
    tm_min: c_int,
    tm_hour: c_int,
    tm_mday: c_int,
    tm_mon: c_int,
    tm_year: c_int,
    tm_wday: c_int,
    tm_yday: c_int,
    tm_isdst: c_int,
    tm_gmtoff: c_long,
    tm_zone: *const c_char,
}

/// Called by SDLActivity. Arguments come from the environment, because
/// `std::env::args` is empty inside a library loaded by the JVM.
#[unsafe(no_mangle)]
pub extern "C" fn SDL_main(_argc: c_int, _argv: *const *const c_char) -> c_int {
    log_to_file();
    let code = match arguments().and_then(|args| crate::Options::parse_from(args.into_iter())) {
        Ok(options) => match crate::run_with(options) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("Luma native: {error}");
                1
            }
        },
        Err(error) => {
            eprintln!("Luma native: {error}");
            1
        }
    };
    // SDL cannot run SDL_main twice in one process: the next launch starts fresh.
    std::process::exit(code);
}

fn folder(variable: &str) -> Result<PathBuf, String> {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| format!("{variable} was not set by LumaActivity"))
}

fn arguments() -> Result<Vec<String>, String> {
    let text = |path: PathBuf| path.to_string_lossy().into_owned();
    Ok(vec![
        "--assets".into(),
        text(folder("LUMA_ASSETS")?),
        "--preferences".into(),
        text(preferences_path()?),
    ])
}

/// The app's shared folder (`/sdcard/Android/data/com.frx9.luma/files`),
/// reachable with adb like Mystify's configuration.
pub fn preferences_path() -> Result<PathBuf, String> {
    Ok(folder("LUMA_DATA")?.join("preferences.json"))
}

/// Home Assistant credentials stay in the app's private storage. A copy of
/// the TV's `ha-auth.json` pushed next to the preferences is imported once
/// and then removed from the shared folder.
pub fn auth_path(preferences: &Path) -> PathBuf {
    let private = folder("LUMA_PRIVATE").unwrap_or_else(|_| {
        preferences
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default()
    });
    let path = private.join("ha-auth.json");
    let pushed = preferences.with_file_name("ha-auth.json");
    if pushed != path && pushed.is_file() {
        match std::fs::read(&pushed)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<crate::smart_home::Secrets>(&bytes).ok())
        {
            Some(secrets) => match crate::smart_home::save_auth(&path, &secrets) {
                Ok(()) => {
                    let _ = std::fs::remove_file(&pushed);
                    eprintln!("Luma Home Assistant: credentials imported");
                }
                Err(error) => eprintln!("Luma Home Assistant: credentials not imported: {error}"),
            },
            None => eprintln!(
                "Luma Home Assistant: {} is not a Luma ha-auth.json",
                pushed.display()
            ),
        }
    }
    path
}

/// stdout and stderr go to luma.log in the shared folder; the previous
/// session is kept as luma.log.1 once the log passes 1 MiB.
fn log_to_file() {
    let Ok(folder) = folder("LUMA_DATA") else {
        return;
    };
    let path = folder.join("luma.log");
    if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() > 1 << 20) {
        let _ = std::fs::rename(&path, folder.join("luma.log.1"));
    }
    if let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        // SAFETY: duplicating a valid descriptor onto the standard streams.
        unsafe {
            dup2(file.as_raw_fd(), 1);
            dup2(file.as_raw_fd(), 2);
        }
    }
    let now = time::OffsetDateTime::now_utc().to_offset(local_offset());
    eprintln!(
        "==== Luma {} Android, {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        env!("CARGO_PKG_VERSION"),
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );
}

/// The device's current UTC offset. Bionic's localtime_r reads the system
/// time zone safely from any thread, unlike `time`'s Unix local offset.
pub fn local_offset() -> time::UtcOffset {
    // SAFETY: plain C calls with valid pointers; `tm` is plain data.
    let offset = unsafe {
        let now = time(std::ptr::null_mut());
        let mut tm: Tm = std::mem::zeroed();
        if localtime_r(&now, &mut tm).is_null() {
            0
        } else {
            tm.tm_gmtoff
        }
    };
    time::UtcOffset::from_whole_seconds(i32::try_from(offset).unwrap_or(0))
        .unwrap_or(time::UtcOffset::UTC)
}

/// Runs `call` with this thread's JNI environment and the activity. SDL
/// attaches the calling thread to the JVM on first use.
fn with_activity<T>(
    call: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<T>,
) -> Result<T, String> {
    // SAFETY: SDL returns this thread's attached JNIEnv, or null.
    let raw = unsafe { SDL_AndroidGetJNIEnv() };
    if raw.is_null() {
        return Err("No JNI environment".into());
    }
    // SAFETY: a valid JNIEnv pointer for the current thread.
    let mut env = unsafe { JNIEnv::from_raw(raw.cast()) }.map_err(|error| error.to_string())?;
    let result = env.with_local_frame(16, |env| {
        // SAFETY: a new local reference, released with this frame.
        let activity = unsafe { JObject::from_raw(SDL_AndroidGetActivity().cast()) };
        if activity.is_null() {
            return Err(jni::errors::Error::NullPtr("activity"));
        }
        call(env, &activity)
    });
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
    result.map_err(|error| error.to_string())
}

#[derive(Deserialize)]
struct Listed {
    id: String,
    title: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    version: String,
}

/// Every app with a TV (Leanback) or ordinary launcher entry. Inputs stay
/// empty: the Chromecast has no HDMI inputs.
pub fn inventory() -> Result<(Inventory, Option<String>), String> {
    let json = with_activity(|env, activity| {
        let value = env
            .call_method(activity, "lumaApps", "()Ljava/lang/String;", &[])?
            .l()?;
        let value = JString::from(value);
        Ok(String::from(env.get_string(&value)?))
    })?;
    let listed: Vec<Listed> =
        serde_json::from_str(&json).map_err(|error| format!("App list: {error}"))?;
    let mut apps: Vec<App> = Vec::new();
    for item in listed.into_iter().take(1000) {
        if !valid_app_id(&item.id) || item.id == PACKAGE || apps.iter().any(|a| a.id == item.id) {
            continue;
        }
        let title: String = item
            .title
            .chars()
            .filter(|c| !c.is_control())
            .take(120)
            .collect();
        let title = if title.trim().is_empty() {
            item.id.clone()
        } else {
            title.trim().to_owned()
        };
        let mut app = App::new(&item.id, &title);
        if app.brand == "generic" && CATEGORIES.contains(&item.category.as_str()) {
            app.category = item.category;
        }
        // The version is part of the artwork cache key: an update reads it again.
        app.icon = Some(crate::app_artwork::Source::new(
            &item.id,
            &serde_json::json!({ "version": item.version }),
        ));
        apps.push(app);
    }
    apps.sort_by_key(|app| app.title.to_lowercase());
    eprintln!("Luma Android inventory: {} apps", apps.len());
    Ok((
        Inventory {
            apps,
            inputs: vec![],
        },
        None,
    ))
}

/// The app's TV banner (or its icon, without one) as PNG bytes.
pub fn app_art(id: &str) -> Result<Vec<u8>, String> {
    if !valid_app_id(id) {
        return Err("Invalid application identifier".into());
    }
    with_activity(|env, activity| {
        let id = env.new_string(id)?;
        let value = env
            .call_method(
                activity,
                "lumaArt",
                "(Ljava/lang/String;)[B",
                &[JValue::Object(&id)],
            )?
            .l()?;
        if value.is_null() {
            return Ok(None);
        }
        Ok(Some(env.convert_byte_array(JByteArray::from(value))?))
    })?
    .ok_or_else(|| format!("No artwork for {id}"))
}

fn java_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'$'))
}

/// What `lumaOpen` accepts: a package, `package/.Activity`, `action:` with a
/// settings action, `uninstall:` with a package, `notification:` with a
/// notification key, or `notifications:clear`.
fn valid_target(target: &str) -> bool {
    if let Some(action) = target.strip_prefix("action:") {
        return java_name(action);
    }
    if let Some(package) = target.strip_prefix("uninstall:") {
        return valid_app_id(package);
    }
    if let Some(key) = target.strip_prefix("notification:") {
        return !key.is_empty() && key.len() <= 400 && !key.chars().any(char::is_control);
    }
    if target == "notifications:clear" {
        return true;
    }
    if let Some(minutes) = target.strip_prefix("timeout:") {
        return minutes.parse::<u32>().is_ok_and(|m| (1..=60).contains(&m));
    }
    let (package, activity) = target.split_once('/').unwrap_or((target, ""));
    valid_app_id(package) && (activity.is_empty() || java_name(activity))
}

/// Opens an app or one of the targets described in `valid_target`.
pub fn launch(target: &str) -> Result<(), String> {
    if !valid_target(target) {
        return Err("Invalid application identifier".into());
    }
    let opened = with_activity(|env, activity| {
        let target = env.new_string(target)?;
        env.call_method(
            activity,
            "lumaOpen",
            "(Ljava/lang/String;)Z",
            &[JValue::Object(&target)],
        )?
        .z()
    })?;
    if opened {
        Ok(())
    } else {
        Err(format!("Luma Android: {target} could not be opened"))
    }
}

#[derive(Deserialize)]
struct QuickReply {
    #[serde(default)]
    network: String,
    #[serde(default)]
    signal: u8,
    #[serde(default)]
    allowed: bool,
    #[serde(default)]
    notices: Vec<NoticeReply>,
    #[serde(default)]
    timeout: u32,
    #[serde(default)]
    writable: bool,
}

#[derive(Deserialize)]
struct NoticeReply {
    key: String,
    #[serde(default)]
    app: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    text: String,
}

/// The connection and recent notifications for the quick settings panel.
pub fn quick() -> Result<crate::platform::QuickInfo, String> {
    let json = with_activity(|env, activity| {
        let value = env
            .call_method(activity, "lumaQuick", "()Ljava/lang/String;", &[])?
            .l()?;
        let value = JString::from(value);
        Ok(String::from(env.get_string(&value)?))
    })?;
    let reply: QuickReply =
        serde_json::from_str(&json).map_err(|error| format!("Quick settings: {error}"))?;
    let clean = |text: String, limit: usize| -> String {
        text.chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .take(limit)
            .collect::<String>()
            .trim()
            .to_owned()
    };
    Ok(crate::platform::QuickInfo {
        network: reply.network,
        signal: reply.signal.min(4),
        notifications_allowed: reply.allowed,
        timeout_minutes: reply.timeout.min(1440),
        timeout_writable: reply.writable,
        notices: reply
            .notices
            .into_iter()
            .filter(|notice| !notice.key.is_empty() && notice.key.len() <= 400)
            .take(8)
            .map(|notice| crate::platform::Notice {
                key: notice.key,
                app: clean(notice.app, 60),
                title: clean(notice.title, 80),
                text: clean(notice.text, 120),
            })
            .collect(),
    })
}
