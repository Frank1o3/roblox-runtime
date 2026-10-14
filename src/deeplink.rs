//! Route Roblox WebView server joins into the already running client.

const MESSAGE_BUS: &str = "com/roblox/universalapp/messagebus/MessageBus";
const DETECT_URL: &str = "Linking.detectURL";

/// Forward a Roblox custom-scheme game URL through the engine's Linking
/// message. This avoids asking the desktop environment to start a second
/// Roblox process.
#[allow(unsafe_code)]
pub fn join_from_webview(engine: &crate::LoadedEngine, url: &str) -> Result<(), String> {
    let (place_id, instance_id) = parse_game_url(url)?;
    let mut launch_url = format!("roblox://experiences/start?placeId={place_id}");
    if let Some(instance_id) = instance_id.as_deref() {
        launch_url.push_str("&gameInstanceId=");
        launch_url.push_str(instance_id);
    }

    let publish = engine
        .symbol("Java_com_roblox_universalapp_messagebus_MessageBus_publishRaw")
        .ok_or_else(|| "Roblox MessageBus.publishRaw is unavailable".to_owned())?;
    let payload = format!("{{\"url\":\"{}\"}}", launch_url);
    // SAFETY: publishRaw is the engine's static `(String, String) -> void`
    // JNI export. Both strings are bounded and contain no JSON metacharacters.
    unsafe {
        crate::jni::game_activity::call_static_strings(
            publish,
            MESSAGE_BUS,
            &[DETECT_URL, &payload],
        )
    }
}

fn parse_game_url(url: &str) -> Result<(String, Option<String>), String> {
    if url.len() > 2048 || !url.is_ascii() {
        return Err("Roblox join URL is too long or contains non-ASCII data".into());
    }
    if let Some(query) = url.strip_prefix("roblox://experiences/start?") {
        return parse_mobile_query(query);
    }
    let payload = url
        .strip_prefix("roblox-player:")
        .or_else(|| {
            url.get(.."roblox-player:".len())
                .filter(|scheme| scheme.eq_ignore_ascii_case("roblox-player:"))
                .map(|_| &url["roblox-player:".len()..])
        })
        .ok_or_else(|| "unsupported Roblox join URL".to_owned())?;
    parse_desktop_launch(payload)
}

fn parse_mobile_query(query: &str) -> Result<(String, Option<String>), String> {
    let mut place_id = None;
    let mut instance_id = None;
    for pair in query.split('&') {
        let (key, value) = pair
            .split_once('=')
            .ok_or_else(|| "malformed Roblox join URL".to_owned())?;
        match key {
            "placeId" if place_id.is_none() => {
                if value.is_empty()
                    || value.len() > 20
                    || !value.bytes().all(|b| b.is_ascii_digit())
                {
                    return Err("invalid placeId in Roblox join URL".into());
                }
                place_id = Some(value.to_owned());
            }
            "gameInstanceId" if instance_id.is_none() => {
                if value.is_empty()
                    || value.len() > 64
                    || !value.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
                {
                    return Err("invalid gameInstanceId in Roblox join URL".into());
                }
                instance_id = Some(value.to_owned());
            }
            _ => return Err("unexpected or duplicate parameter in Roblox join URL".into()),
        }
    }
    place_id
        .map(|place_id| (place_id, instance_id))
        .ok_or_else(|| "Roblox join URL has no placeId".into())
}

fn parse_desktop_launch(payload: &str) -> Result<(String, Option<String>), String> {
    let mut launch_mode = None;
    let mut launcher_url = None;
    for field in payload.trim_start_matches('/').split('+') {
        let Some((key, value)) = field.split_once(':') else {
            continue;
        };
        match key.to_ascii_lowercase().as_str() {
            "launchmode" => launch_mode = Some(value),
            "placelauncherurl" => launcher_url = Some(value),
            // gameinfo is a one-time browser credential and is deliberately
            // not forwarded into the already authenticated Android client.
            _ => {}
        }
    }
    if launch_mode.is_some_and(|mode| !mode.eq_ignore_ascii_case("play")) {
        return Err("Roblox desktop link is not a play launch".into());
    }
    let launcher_url =
        launcher_url.ok_or_else(|| "Roblox desktop link has no PlaceLauncher URL".to_owned())?;
    let launcher_url = percent_decode(launcher_url)?;
    let (origin_path, query) = launcher_url
        .split_once('?')
        .ok_or_else(|| "Roblox PlaceLauncher URL has no query".to_owned())?;
    if origin_path != "https://assetgame.roblox.com/game/PlaceLauncher.ashx" {
        return Err("unsupported Roblox PlaceLauncher endpoint".into());
    }
    let mut place_id = None;
    let mut instance_id = None;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key.to_ascii_lowercase().as_str() {
            "placeid" if place_id.is_none() => {
                if value.is_empty()
                    || value.len() > 20
                    || !value.bytes().all(|b| b.is_ascii_digit())
                {
                    return Err("invalid placeId in Roblox PlaceLauncher URL".into());
                }
                place_id = Some(value.to_owned());
            }
            "gameid" | "jobid" if instance_id.is_none() => {
                if !value.is_empty()
                    && value.len() <= 64
                    && value.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
                {
                    instance_id = Some(value.to_owned());
                }
            }
            _ => {}
        }
    }
    place_id
        .map(|place_id| (place_id, instance_id))
        .ok_or_else(|| "Roblox PlaceLauncher URL has no valid placeId".into())
}

fn percent_decode(value: &str) -> Result<String, String> {
    let mut decoded = Vec::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let digits = bytes
                .get(index + 1..index + 3)
                .ok_or_else(|| "malformed escape in Roblox PlaceLauncher URL".to_owned())?;
            let high = hex_value(digits[0])
                .ok_or_else(|| "malformed escape in Roblox PlaceLauncher URL".to_owned())?;
            let low = hex_value(digits[1])
                .ok_or_else(|| "malformed escape in Roblox PlaceLauncher URL".to_owned())?;
            decoded.push(high * 16 + low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| "Roblox PlaceLauncher URL is not UTF-8".into())
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
