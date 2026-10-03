//! Runtime-owned persistence for Roblox login sessions.
//! Cookie files are private (0600) and must be treated like passwords.

use std::ffi::{CStr, c_char};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const SETTINGS: &str = "com/roblox/engine/jni/NativeSettingsInterface";
const DOMAINS: [&str; 4] = [
    "roblox.com",
    ".roblox.com",
    "apis.roblox.com",
    "auth.roblox.com",
];
const IDENTITY_FILE: &str = "roblox-identity";
static ACTIVE_SESSION_DIR: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

struct SavedIdentity {
    user_id: i64,
    username: String,
    display_name: String,
    membership_type: i64,
    is_under13: bool,
    has_roblox_subscription: bool,
}

/// Select the profile that receives login and logout updates from the engine.
/// Install the callbacks before the app bridge can report an authentication
/// event so a successful sign-in is immediately recorded for the next launch.
pub fn initialize(session_dir: Option<&Path>) {
    let active = ACTIVE_SESSION_DIR.get_or_init(|| Mutex::new(None));
    if let Ok(mut active) = active.lock() {
        *active = session_dir.map(Path::to_path_buf);
    }
    crate::jni::game_activity::identity_set_sinks(identity_logged_in, identity_logged_out);
}

/// A named login profile owned by the runtime. Session names are directory
/// names and are restricted to portable ASCII identifiers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    name: String,
    directory: PathBuf,
}

impl Session {
    pub fn open(root: &Path, name: &str) -> Result<Self, String> {
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err("session name may contain only ASCII letters, numbers, '-' and '_'".into());
        }
        let directory = root.join(name);
        std::fs::create_dir_all(&directory)
            .map_err(|e| format!("create session directory: {e}"))?;
        Ok(Self {
            name: name.to_owned(),
            directory,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// List valid profiles stored under a runtime session root.
    pub fn list(root: &Path) -> Result<Vec<Self>, String> {
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(format!("read session directory: {error}")),
        };
        let mut sessions = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| format!("read session entry: {error}"))?;
            if !entry
                .file_type()
                .map_err(|error| format!("inspect session entry: {error}"))?
                .is_dir()
            {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            {
                continue;
            }
            sessions.push(Self {
                name,
                directory: entry.path(),
            });
        }
        sessions.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(sessions)
    }
}

#[allow(unsafe_code)]
pub fn restore(engine: &crate::LoadedEngine, session_dir: &Path) -> Result<(), String> {
    let path = session_dir.join("roblox-cookies");
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return restore_identity(engine, session_dir);
        }
        Err(error) => return Err(format!("read saved Roblox cookies: {error}")),
    };
    let mut restored = 0;
    if let Some(native) =
        engine.symbol("Java_com_roblox_engine_jni_NativeSettingsInterface_nativeSetMultipleCookies")
    {
        for line in contents.lines().filter(|line| !line.starts_with('#')) {
            let Some((domain, cookies)) = line.split_once('\t') else {
                continue;
            };
            let cookies = unescape(cookies);
            if cookies.is_empty() {
                continue;
            }
            // SAFETY: this is the live static settings native and both strings stay alive.
            unsafe {
                crate::jni::game_activity::call_static_strings(
                    native,
                    SETTINGS,
                    &[domain, &cookies],
                )
            }
            .map_err(|e| format!("restore Roblox session cookies: {e}"))?;
            restored += 1;
        }
    } else {
        eprintln!("[session] engine has no cookie restore native");
    }
    if restored > 0 {
        eprintln!("[session] restored cookies for {restored} Roblox domains");
    }
    restore_identity(engine, session_dir)?;
    Ok(())
}

fn restore_identity(engine: &crate::LoadedEngine, session_dir: &Path) -> Result<(), String> {
    let path = session_dir.join(IDENTITY_FILE);
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("read saved Roblox identity: {error}")),
    };
    let value = serde_json::from_str::<serde_json::Value>(&contents)
        .map_err(|error| format!("parse saved Roblox identity: {error}"))?;
    let identity =
        decode_identity(&value).ok_or_else(|| "saved Roblox identity is incomplete".to_owned())?;
    crate::jni::game_activity::identity_publish(
        identity.user_id,
        &identity.username,
        &identity.display_name,
        identity.membership_type,
        identity.is_under13,
        identity.has_roblox_subscription,
    );
    // The app-start parameters and NativeUserJavaInterface read the runtime's
    // mirrors, but Roblox also keeps its own user id. Cordial measured that
    // leaving this copy empty still routed a cookie-authenticated profile to
    // Landing. Restore it after app-bridge init and before StartAppParams is
    // built, which is the point at which this function is called.
    let native = engine
        .symbol("Java_com_roblox_engine_jni_NativeSettingsInterface_nativeSetUserId")
        .ok_or_else(|| "engine has no NativeSettingsInterface.nativeSetUserId".to_owned())?;
    let user_id = identity.user_id.to_string();
    // SAFETY: the JNI export belongs to this mapped engine and startup has
    // initialized JNI and app-bridge before session restoration.
    unsafe {
        crate::jni::game_activity::call_static_strings(native, SETTINGS, &[user_id.as_str()])
    }
    .map_err(|error| format!("restore Roblox native user id: {error}"))?;
    eprintln!("[session] restored saved account identity and native user id");
    Ok(())
}

#[allow(unsafe_code)]
unsafe extern "C" fn identity_logged_in(payload: *const c_char) {
    if payload.is_null() {
        return;
    }
    // SAFETY: the native callback supplies a NUL-terminated payload that stays
    // alive for the duration of this callback.
    let Ok(payload) = unsafe { CStr::from_ptr(payload) }.to_str() else {
        eprintln!("[session] could not decode login identity payload");
        return;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
        eprintln!("[session] could not parse login identity payload");
        return;
    };
    let Some(identity) = decode_identity(&value) else {
        eprintln!("[session] login identity payload is incomplete");
        return;
    };
    crate::jni::game_activity::identity_publish(
        identity.user_id,
        &identity.username,
        &identity.display_name,
        identity.membership_type,
        identity.is_under13,
        identity.has_roblox_subscription,
    );

    let active = ACTIVE_SESSION_DIR.get_or_init(|| Mutex::new(None));
    let session_dir = active.lock().ok().and_then(|active| active.clone());
    if let Some(session_dir) = session_dir {
        if let Err(error) = write_private_json(
            &session_dir.join(IDENTITY_FILE),
            &encode_identity(&identity),
        ) {
            eprintln!("[session] could not save signed-in account identity: {error}");
        } else {
            eprintln!("[session] saved signed-in account identity");
        }
    }
}

extern "C" fn identity_logged_out() {
    crate::jni::game_activity::identity_clear();
    let active = ACTIVE_SESSION_DIR.get_or_init(|| Mutex::new(None));
    let session_dir = active.lock().ok().and_then(|active| active.clone());
    if let Some(session_dir) = session_dir {
        for file in [IDENTITY_FILE, "roblox-cookies"] {
            if let Err(error) = std::fs::remove_file(session_dir.join(file)) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("[session] could not clear signed-out session data: {error}");
                }
            }
        }
    }
    eprintln!("[session] cleared signed-out account state");
}

fn json_i64(value: Option<&serde_json::Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_str()?.parse::<i64>().ok())
}

fn decode_identity(value: &serde_json::Value) -> Option<SavedIdentity> {
    let user_id = json_i64(value.get("userId")).filter(|id| *id > 0)?;
    let username = value.get("username")?.as_str()?;
    if username.is_empty() {
        return None;
    }
    Some(SavedIdentity {
        user_id,
        username: username.to_owned(),
        display_name: value
            .get("displayName")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(username)
            .to_owned(),
        membership_type: value
            .get("membershipType")
            .and_then(|value| json_i64(Some(value)))
            .unwrap_or_default(),
        is_under13: value
            .get("isUnder13")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        has_roblox_subscription: value
            .get("hasRobloxSubscription")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    })
}

fn encode_identity(identity: &SavedIdentity) -> serde_json::Value {
    serde_json::json!({
        "userId": identity.user_id,
        "username": identity.username,
        "displayName": identity.display_name,
        "membershipType": identity.membership_type,
        "isUnder13": identity.is_under13,
        "hasRobloxSubscription": identity.has_roblox_subscription,
    })
}

fn write_private_json(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "session identity path has no parent".to_owned())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("create session directory: {error}"))?;
    let temp = path.with_extension("tmp");
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temp)
        .map_err(|error| format!("open saved identity: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("secure saved identity: {error}"))?;
    }
    use std::io::Write;
    file.write_all(&bytes)
        .map_err(|error| format!("write saved identity: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("sync saved identity: {error}"))?;
    std::fs::rename(temp, path).map_err(|error| format!("publish saved identity: {error}"))
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(ch) => {
                out.push('\\');
                out.push(ch);
            }
            None => out.push('\\'),
        }
    }
    out
}

pub fn flush_if_due(engine: &crate::LoadedEngine, data_dir: &Path) {
    static LAST: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
    let due = LAST
        .lock()
        .map(|mut last| {
            let now = Instant::now();
            if last.is_some_and(|at| now.duration_since(at) < Duration::from_secs(30)) {
                false
            } else {
                *last = Some(now);
                true
            }
        })
        .unwrap_or(false);
    if due {
        let _ = save(engine, data_dir);
    }
}

#[allow(unsafe_code)]
pub fn save(engine: &crate::LoadedEngine, data_dir: &Path) -> Result<(), String> {
    let Some(native) = engine
        .symbol("Java_com_roblox_engine_jni_NativeSettingsInterface_nativeGetCookiesForDomain")
    else {
        return Ok(());
    };
    let mut output = String::from("# rusty-blox session cookie store; treat as a password\n");
    for domain in DOMAINS {
        // SAFETY: this is the live static settings native and the domain is valid UTF-8.
        let jar =
            unsafe { crate::jni::game_activity::cookies_for_domain(native, SETTINGS, domain) }
                .map_err(|e| format!("read Roblox session cookies: {e}"))?;
        let cookies = jar
            .expose()
            .split("; ")
            .filter_map(|record| {
                let fields: Vec<_> = record.split('\t').collect();
                if fields.len() < 7 {
                    return None;
                }
                let name = fields[fields.len() - 2];
                let value = fields[fields.len() - 1];
                (!name.is_empty()).then(|| format!("{name}={value}"))
            })
            .collect::<Vec<_>>()
            .join("; ");
        if !cookies.is_empty() {
            output.push_str(domain);
            output.push('\t');
            output.push_str(
                &cookies
                    .replace('\\', "\\\\")
                    .replace('\n', "\\n")
                    .replace('\t', "\\t"),
            );
            output.push('\n');
        }
    }
    if output.ends_with("password\n") {
        return Ok(());
    }
    let path = data_dir.join("roblox-cookies");
    let temp = data_dir.join("roblox-cookies.tmp");
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    file.write_all(output.as_bytes())
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    std::fs::rename(temp, path).map_err(|e| e.to_string())?;
    Ok(())
}
