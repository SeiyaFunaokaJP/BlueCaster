//! The last broadcast started (stream.start_broadcast's parameters), kept
//! across restarts so that the Auracast form shows it again.
//!
//! Two files: `auracast_last.json` holds the parameters without the
//! Broadcast_Code, and `pairing\auracast_code.bin` holds the code alone,
//! encrypted for the Windows user ([`win_sys::dpapi`]) as the link keys
//! are. A broadcast is usually started again with the same code (the
//! receivers that are not told by the assistant have it typed in), so it is
//! kept, but not as plain text and never in a diagnostic bundle.

use std::io;
use std::path::Path;

use ipc::Method;

use crate::paths;

/// The last broadcast kept in `config`, with its Broadcast_Code when one
/// is kept and can be decrypted; None when there is none (or it is not a
/// stream.start_broadcast)
pub fn load(config: &Path) -> Option<Method> {
    let text = std::fs::read_to_string(paths::auracast_last_file(config)).ok()?;
    let mut last = serde_json::from_str::<Method>(&text)
        .inspect_err(|e| tracing::warn!("Auracast: last broadcast not read ({e})"))
        .ok()
        .filter(|m| matches!(m, Method::StreamStartBroadcast { .. }))?;
    if let Method::StreamStartBroadcast { code, .. } = &mut last {
        *code = win_sys::dpapi::read_secret(&paths::auracast_code_file(config))
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .filter(|c| !c.is_empty());
    }
    Some(last)
}

/// Keeps `last` in `config`: its parameters as JSON, its Broadcast_Code
/// encrypted in a file of its own (removed when it has none)
pub fn save(config: &Path, last: &Method) -> io::Result<()> {
    let mut plain = last.clone();
    let code = match &mut plain {
        Method::StreamStartBroadcast { code, .. } => code.take().filter(|c| !c.is_empty()),
        _ => None,
    };
    let text = serde_json::to_string_pretty(&plain).map_err(io::Error::other)?;
    std::fs::write(paths::auracast_last_file(config), text)?;
    let code_file = paths::auracast_code_file(config);
    match code {
        Some(code) => win_sys::dpapi::write_secret(&code_file, code.as_bytes()),
        None => match std::fs::remove_file(&code_file) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start(code: Option<&str>) -> Method {
        Method::StreamStartBroadcast {
            name: "Room".into(),
            code: code.map(str::to_string),
            quality: "hq".into(),
            rear_quality: None,
            mono: None,
            split: None,
            layout: Some("single_2_1".into()),
            source: "tone".into(),
            source_id: None,
            source_name: None,
            volume: None,
            follow_volume: Some(false),
            receivers: None,
            leave_receivers: None,
        }
    }

    #[test]
    fn the_code_comes_back_but_is_not_in_a_file_as_plain_text() {
        let dir = std::env::temp_dir().join(format!("bc-auracast-last-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(load(&dir), None);
        let with_code = start(Some("secret-code-12"));
        save(&dir, &with_code).unwrap();
        // Neither file shows the code
        let json = std::fs::read_to_string(paths::auracast_last_file(&dir)).unwrap();
        assert!(!json.contains("secret-code-12"), "{json}");
        let blob = std::fs::read(paths::auracast_code_file(&dir)).unwrap();
        assert!(win_sys::dpapi::is_encrypted(&blob));
        assert!(!String::from_utf8_lossy(&blob).contains("secret-code-12"));
        // And it is restored with the rest
        assert_eq!(load(&dir), Some(with_code));
        // An open broadcast next: the code kept goes
        save(&dir, &start(None)).unwrap();
        assert!(!paths::auracast_code_file(&dir).exists());
        assert_eq!(load(&dir), Some(start(None)));
        // The code's file lost or unreadable: the rest still comes back
        save(&dir, &start(Some("secret-code-12"))).unwrap();
        std::fs::remove_file(paths::auracast_code_file(&dir)).unwrap();
        assert_eq!(load(&dir), Some(start(None)));
        // Another method is not a last broadcast
        std::fs::write(paths::auracast_last_file(&dir), r#"{"method":"stream.stop"}"#).unwrap();
        assert_eq!(load(&dir), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
