//! Minimal field readers for the flat JSON replies of Luna services.
//! Adapted from the MIT-licensed Webos-EARC-Volume-Overlay watcher
//! (Copyright (c) 2026 eARC Volume Overlay contributors).

/// Finds `"key"` followed by `:` and returns the text after it.
fn after_key<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{}\"", key);
    let mut from = 0;
    while let Some(at) = json[from..].find(&needle) {
        let end = from + at + needle.len();
        if let Some(rest) = json[end..].trim_start().strip_prefix(':') {
            return Some(rest.trim_start());
        }
        from = end;
    }
    None
}

/// Extracts a string field from a flat-enough JSON reply.
pub fn string_field<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let rest = after_key(json, key)?.strip_prefix('"')?;
    Some(&rest[..rest.find('"')?])
}

/// Extracts a boolean field.
pub fn bool_field(json: &str, key: &str) -> Option<bool> {
    let rest = after_key(json, key)?;
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

/// The `soundOutput` value, accepted only if it is a plain identifier (the
/// same `[A-Za-z0-9_-]+` rule as watch.sh), so it is safe to send back.
pub fn sound_output(json: &str) -> Option<String> {
    let value = string_field(json, "soundOutput")?;
    let ok = !value.is_empty() && value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    ok.then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_fields() {
        let line = r#"{"returnValue":true,"volumeStatus":{"volume":12,"muteStatus":false,"soundOutput":"external_arc"},"subscribed":true}"#;
        assert_eq!(string_field(line, "soundOutput"), Some("external_arc"));
        assert_eq!(bool_field(line, "returnValue"), Some(true));
        assert_eq!(bool_field(line, "muteStatus"), Some(false));
        assert_eq!(string_field(line, "missing"), None);
        assert_eq!(sound_output(line).as_deref(), Some("external_arc"));
    }

    #[test]
    fn reads_settings_reply() {
        let reply = r#"{ "subscribed": true, "category": "sound", "method": "getSystemSettings", "settings": { "soundOutput": "bt_soundbar" }, "returnValue": true }"#;
        assert_eq!(sound_output(reply).as_deref(), Some("bt_soundbar"));
        assert_eq!(bool_field(reply, "returnValue"), Some(true));
    }

    #[test]
    fn reads_replies_from_an_lg_c3() {
        let audio = r#"{"returnValue":true,"lastSoundOutput":"tv_speaker","soundOutput":"external_arc","settingsSoundOutputs":["external_arc"]}"#;
        assert_eq!(sound_output(audio).as_deref(), Some("external_arc"));
        let settings = r#"{"subscribed":true,"category":"sound","method":"getSystemSettings","dimension":{"audioOutput":"default"},"settings":{"soundOutput":"external_arc"},"returnValue":true}"#;
        assert_eq!(sound_output(settings).as_deref(), Some("external_arc"));
        let volume = r#"{"returnValue":true,"volumeStatus":{"volumeLimitable":false,"activeStatus":true,"maxVolume":100,"soundOutput":"external_arc","volume":3,"muteStatus":false},"callerId":"com.webos.lunasend-25137"}"#;
        assert_eq!(sound_output(volume).as_deref(), Some("external_arc"));
    }

    #[test]
    fn rejects_unsafe_output_values() {
        assert_eq!(sound_output(r#"{"soundOutput":"a\"}"}"#), None);
        assert_eq!(sound_output(r#"{"soundOutput":"x y"}"#), None);
        assert_eq!(sound_output(r#"{"soundOutput":""}"#), None);
    }

    #[test]
    fn skips_key_text_that_is_not_a_key() {
        let reply = r#"{"note":"soundOutput","soundOutput":"tv_speaker"}"#;
        assert_eq!(sound_output(reply).as_deref(), Some("tv_speaker"));
    }
}
