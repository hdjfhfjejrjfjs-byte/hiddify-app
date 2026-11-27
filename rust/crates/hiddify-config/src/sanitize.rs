//! Recursive JSON configuration secret scrubber matching `hiddify-core/v2/config/sanitize.go`.

use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

const REDACTED_CONFIG_VALUE: &str = "<redacted>";

fn sensitive_key_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(^|_)(server|host|hosts|sni|address|addresses|password|passwd|secret|token|auth|auth_str|user|users|username|uuid|key|keys|cert|certificate|path|short_id|obfs|reserved|cookie|license|dns_stamp|url)($|_)")
            .expect("valid sensitive config key regex")
    })
}

pub fn is_sensitive_config_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    if lower.ends_with("_port") || lower == "listen" || lower == "listen_port" {
        return false;
    }
    sensitive_key_re().is_match(&lower)
}

pub fn sanitize_config_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if is_sensitive_config_key(key) {
                    *child = Value::String(REDACTED_CONFIG_VALUE.to_string());
                } else {
                    sanitize_config_value(child);
                }
            }
        }
        Value::Array(items) => {
            for child in items.iter_mut() {
                sanitize_config_value(child);
            }
        }
        _ => {}
    }
}

/// Replaces the values of sensitive keys at any depth with `"<redacted>"`, keeping ports,
/// listen addresses, types, tags, routes, and structure.
/// Returns `None` on invalid JSON so unparseable configs are never leaked in the clear.
pub fn sanitize_config_json(content: &str) -> Option<String> {
    let mut root: Value = serde_json::from_str(content).ok()?;
    sanitize_config_value(&mut root);
    let mut formatted = serde_json::to_string_pretty(&root).ok()?;
    formatted.push('\n');
    Some(formatted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_config_json_matches_go_rules() {
        let raw = r#"{"outbounds":[{"type":"vless","tag":"p","server":"my.server.example","server_port":443,"listen":"127.0.0.1","listen_port":1080,"uuid":"0a1b2c3d-1111-2222-3333-444455556666","tls":{"server_name":"sni.example","utls":{"enabled":true}},"transport":{"path":"/secret"}}]}"#;
        let out = sanitize_config_json(raw).expect("valid json");
        assert!(!out.contains("my.server.example"));
        assert!(!out.contains("0a1b2c3d"));
        assert!(!out.contains("/secret"));
        assert!(out.contains(r#""type": "vless""#));
        assert!(out.contains(r#""server": "<redacted>""#));
        assert!(out.contains(r#""server_port": 443"#));
        assert!(out.contains(r#""listen": "127.0.0.1""#));
        assert!(out.contains(r#""listen_port": 1080"#));
        assert!(out.contains(r#""uuid": "<redacted>""#));
        assert!(out.contains(r#""path": "<redacted>""#));
        assert!(!out.contains(r"\u003c"));

        assert!(sanitize_config_json("{not valid json").is_none());
    }
}
