//! Sing-box & Hiddify JSON configuration inspection, `keep-top` proxy extraction,
//! country/protocol classification, editable config filtering, single-proxy detour extraction,
//! and subscription/user override building & deep JSON merging.
//!
//! Uses `serde_json` with `preserve_order` (`IndexMap`) so JSON object key insertion order
//! matches Dart's `LinkedHashMap` (`Map<String, dynamic>`) 100%.

use std::collections::HashSet;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

pub mod route_rules;
pub mod sanitize;

pub use sanitize::sanitize_config_json;

/// Default number of fastest proxies kept by `keep-top` when unspecified.
pub const KEEP_TOP_DEFAULT: i64 = 5;

/// Special `keep-top` sentinel keeping every proxy that answers its first ping.
pub const KEEP_TOP_ANSWERING: i64 = -2;

/// Allowed keys emitted by `profile_override`.
pub const ALLOWED_OVERRIDE_CONFIGS: &[&str] = &[
    "ipv6-mode",
    "connection-test-url",
    "ipinfo-url",
    "direct-dns-address",
    "remote-dns-address",
    "tls-tricks",
    "chain-status",
    "extra-security",
    "keep-top",
    "keep-top-quick",
    "keep-top-countries",
    "keep-top-pinned",
    "keep-tags",
];

const NOT_PROXY_TYPES: &[&str] = &["block", "selector", "urltest", "custom", "dns"];
const BUILTIN_PROXY_TAGS: &[&str] = &["direct", "bypass", "block", "select", "lowest"];

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Invalid JSON configuration: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("Invalid integer range: {0}")]
    InvalidRange(String),
}

/// Optional integer range (`min` or `min-max`) mirroring `OptionalRange` in Dart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptionalRange {
    pub min: Option<i64>,
    pub max: Option<i64>,
}

impl OptionalRange {
    pub fn parse(input: &str, allow_empty: bool) -> Result<Self, ConfigError> {
        let parts: Vec<&str> = input.split('-').collect();
        match parts.as_slice() {
            [val] if val.is_empty() && allow_empty => Ok(Self {
                min: None,
                max: None,
            }),
            [min_str] => {
                let min = min_str
                    .trim()
                    .parse::<i64>()
                    .map_err(|_| ConfigError::InvalidRange(input.to_string()))?;
                // Note: Dart `int.parse` accepts leading/trailing spaces, check exact parse
                if min_str.is_empty() {
                    return Err(ConfigError::InvalidRange(input.to_string()));
                }
                Ok(Self {
                    min: Some(min),
                    max: None,
                })
            }
            [min_str, max_str] => {
                if min_str.is_empty() || max_str.is_empty() {
                    return Err(ConfigError::InvalidRange(input.to_string()));
                }
                let min = min_str
                    .trim()
                    .parse::<i64>()
                    .map_err(|_| ConfigError::InvalidRange(input.to_string()))?;
                let max = max_str
                    .trim()
                    .parse::<i64>()
                    .map_err(|_| ConfigError::InvalidRange(input.to_string()))?;
                Ok(Self {
                    min: Some(min),
                    max: Some(max),
                })
            }
            _ => Err(ConfigError::InvalidRange(input.to_string())),
        }
    }

    pub fn try_parse(input: &str, allow_empty: bool) -> Option<Self> {
        Self::parse(input, allow_empty).ok()
    }

    pub fn format(&self) -> String {
        match (self.min, self.max) {
            (Some(a), Some(b)) => format!("{a}-{b}"),
            (Some(a), None) => a.to_string(),
            (None, Some(b)) => b.to_string(),
            (None, None) => String::new(),
        }
    }
}

/// A user-selectable proxy entry extracted from a profile's `outbounds` or `endpoints`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileProxy {
    pub tag: String,
    #[serde(rename = "type")]
    pub proxy_type: String,
}

/// Extracts user proxies from a Sing-box JSON configuration in config order (`keep_top.dart:profileProxies`).
pub fn profile_proxies(config_json: &str) -> Result<Vec<ProfileProxy>, ConfigError> {
    let parsed: Value = serde_json::from_str(config_json)?;
    let Some(obj) = parsed.as_object() else {
        return Ok(Vec::new());
    };

    let mut proxies = Vec::new();
    for section in ["outbounds", "endpoints"] {
        let Some(items) = obj.get(section).and_then(Value::as_array) else {
            continue;
        };
        for out in items {
            let Some(out_obj) = out.as_object() else {
                continue;
            };
            let tag = match out_obj.get("tag") {
                Some(Value::String(s)) => s.as_str(),
                _ => "",
            };
            let proxy_type = match out_obj.get("type") {
                Some(Value::String(s)) => s.as_str(),
                _ => "",
            };
            if NOT_PROXY_TYPES.contains(&proxy_type)
                || BUILTIN_PROXY_TAGS.contains(&tag)
                || tag.contains("§hide§")
            {
                continue;
            }
            proxies.push(ProfileProxy {
                tag: tag.to_string(),
                proxy_type: proxy_type.to_string(),
            });
        }
    }
    Ok(proxies)
}

/// Tolerant variant returning an empty list on malformed JSON (`readProfileProxies` behavior).
pub fn profile_proxies_lossy(config_json: &str) -> Vec<ProfileProxy> {
    profile_proxies(config_json).unwrap_or_default()
}

/// Counts user proxies in a Sing-box JSON config (`0` on invalid JSON).
pub fn count_proxies(config_json: &str) -> usize {
    profile_proxies_lossy(config_json).len()
}

/// Extracts the two-letter ISO country code from the first regional-indicator flag emoji pair in `tag`.
pub fn country_of(tag: &str) -> Option<String> {
    let is_regional = |c: char| ('\u{1F1E6}'..='\u{1F1FF}').contains(&c);
    let chars: Vec<char> = tag.chars().collect();
    for window in chars.windows(2) {
        if is_regional(window[0]) && is_regional(window[1]) {
            let c1 = char::from_u32(window[0] as u32 - 0x1F1E6 + 0x41)?;
            let c2 = char::from_u32(window[1] as u32 - 0x1F1E6 + 0x41)?;
            let mut s = String::with_capacity(2);
            s.push(c1);
            s.push(c2);
            return Some(s);
        }
    }
    None
}

/// Converts an uppercase two-letter ASCII country code into its flag emoji.
pub fn flag_of(code: &str) -> String {
    code.encode_utf16()
        .filter_map(|u| {
            u32::from(u)
                .checked_sub(0x41)
                .and_then(|d| char::from_u32(d + 0x1F1E6))
        })
        .collect()
}

/// Maps a sing-box outbound/endpoint `type` to its display protocol label (`keep_tags_dialog.dart:protocolName`).
pub fn protocol_name(proxy_type: &str) -> String {
    match proxy_type {
        "shadowsocks" => "Shadowsocks".to_string(),
        "shadowsocksr" => "ShadowsocksR".to_string(),
        "vless" => "VLESS".to_string(),
        "vmess" => "VMess".to_string(),
        "trojan" => "Trojan".to_string(),
        "naive" => "Naive".to_string(),
        "wireguard" => "WireGuard".to_string(),
        "hysteria" => "Hysteria".to_string(),
        "hysteria2" => "Hysteria2".to_string(),
        "tuic" => "TUIC".to_string(),
        "anytls" => "AnyTLS".to_string(),
        "shadowtls" => "ShadowTLS".to_string(),
        "ssh" => "SSH".to_string(),
        "tor" => "Tor".to_string(),
        "socks" => "SOCKS".to_string(),
        "http" => "HTTP".to_string(),
        "warp" => "WARP".to_string(),
        "masque" => "MASQUE".to_string(),
        "psiphon" => "Psiphon".to_string(),
        "mieru" => "Mieru".to_string(),
        "awg" => "Awg".to_string(),
        "dnstt" => "DNSTT".to_string(),
        "xray" => "xray".to_string(),
        "" => "?".to_string(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => {
                    let mut s = first.to_uppercase().to_string();
                    s.push_str(chars.as_str());
                    s
                }
                None => "?".to_string(),
            }
        }
    }
}

/// Editable portion of a profile configuration (`profile_details_notifier.dart:EditableConfig`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditableConfig {
    pub text: String,
    pub is_json: bool,
}

/// Extracts editable `outbounds` and `endpoints` from a generated JSON config, falling back to raw text
/// if `content` is not a valid JSON object (`profile_details_notifier.dart:_editableConfig`).
pub fn editable_config(content: &str) -> EditableConfig {
    let Ok(parsed) = serde_json::from_str::<Value>(content) else {
        return EditableConfig {
            text: content.to_string(),
            is_json: false,
        };
    };
    let Some(obj) = parsed.as_object() else {
        return EditableConfig {
            text: content.to_string(),
            is_json: false,
        };
    };

    let mut outbounds: Vec<&Value> = Vec::new();
    if let Some(Value::Array(list)) = obj.get("outbounds") {
        for outbound in list {
            let Some(out_map) = outbound.as_object() else {
                continue;
            };
            let Some(t_val) = out_map.get("type") else {
                continue;
            };
            if t_val.is_null() {
                continue;
            }
            let t_str = t_val.as_str().unwrap_or("");
            if matches!(t_str, "selector" | "urltest" | "dns" | "block") {
                continue;
            }
            let tag_str = out_map.get("tag").and_then(Value::as_str).unwrap_or("");
            if matches!(tag_str, "direct" | "bypass" | "direct-fragment") {
                continue;
            }
            outbounds.push(outbound);
        }
    }

    let endpoints: Vec<&Value> = match obj.get("endpoints") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(arr)) => arr.iter().collect(),
        Some(_) => {
            return EditableConfig {
                text: content.to_string(),
                is_json: false,
            };
        }
    };

    let Ok(outbounds_json) = serde_json::to_string(&outbounds) else {
        return EditableConfig {
            text: content.to_string(),
            is_json: false,
        };
    };
    let Ok(endpoints_json) = serde_json::to_string(&endpoints) else {
        return EditableConfig {
            text: content.to_string(),
            is_json: false,
        };
    };

    EditableConfig {
        text: format!("{{\"outbounds\": {outbounds_json},\"endpoints\":{endpoints_json} }}"),
        is_json: true,
    }
}

/// Strips the core's duplicate numbering suffix (`"name § 3"` -> `"name"`).
pub fn base_tag(tag: &str) -> String {
    tag.split(" § ").next().unwrap_or("").trim().to_string()
}

/// Checks whether `tag` marks an internal hidden outbound (`§hide§`).
pub fn is_hidden_tag(tag: &str) -> bool {
    tag.contains("§hide§")
}

fn dart_string_of_field(obj: &Map<String, Value>, key: &str) -> String {
    match obj.get(key) {
        None | Some(Value::Null) => "null".to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(other) => other.to_string(),
    }
}

fn decode_json_object(content: Option<&str>) -> Option<Map<String, Value>> {
    let raw = content?;
    if raw.is_empty() {
        return None;
    }
    match serde_json::from_str::<Value>(raw).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

fn iter_config_items(config: &Map<String, Value>) -> Vec<&Map<String, Value>> {
    let mut out = Vec::new();
    for section in ["outbounds", "endpoints"] {
        if let Some(Value::Array(list)) = config.get(section) {
            for item in list {
                if let Some(map) = item.as_object() {
                    out.push(map);
                }
            }
        }
    }
    out
}

/// Extracts a standalone single-proxy JSON config along with its transitive non-hidden `detour` chain
/// (`lib/features/proxy/data/single_proxy_config.dart:singleProxyConfig`).
pub fn single_proxy_config(
    tag: &str,
    host: &str,
    port: i64,
    raw: Option<&str>,
    generated: Option<&str>,
) -> Option<String> {
    let port_agrees = |item: &Map<String, Value>| -> bool {
        if port == 0 {
            return true;
        }
        match item.get("server_port") {
            None | Some(Value::Null) => true,
            Some(Value::Number(n)) => n.as_i64() == Some(port),
            _ => false,
        }
    };
    let host_agrees = |item: &Map<String, Value>| -> bool {
        if host.is_empty() {
            return true;
        }
        match item.get("server") {
            None | Some(Value::Null) => true,
            Some(Value::String(s)) => s == host,
            _ => false,
        }
    };

    if let Some(config) = decode_json_object(raw) {
        let wanted_name = base_tag(tag);
        let mut matches: Vec<&Map<String, Value>> = iter_config_items(&config)
            .into_iter()
            .filter(|item| base_tag(&dart_string_of_field(item, "tag")) == wanted_name)
            .collect();
        if matches.len() > 1 {
            matches.retain(|item| port_agrees(item));
        }
        if matches.len() > 1 {
            matches.retain(|item| host_agrees(item));
        }
        if matches.len() == 1 {
            return Some(extract_single_proxy(&config, matches[0]));
        }
    }

    if let Some(config) = decode_json_object(generated) {
        let found = iter_config_items(&config)
            .into_iter()
            .find(|item| item.get("tag").and_then(Value::as_str) == Some(tag));
        if let Some(proxy) = found {
            if port_agrees(proxy) {
                return Some(extract_single_proxy(&config, proxy));
            }
        }
    }

    None
}

fn extract_single_proxy(config: &Map<String, Value>, proxy: &Map<String, Value>) -> String {
    let mut by_tag: IndexMap<String, &Map<String, Value>> = IndexMap::new();
    for item in iter_config_items(config) {
        by_tag.insert(dart_string_of_field(item, "tag"), item);
    }

    let mut picked: IndexMap<String, Map<String, Value>> = IndexMap::new();
    let mut next_opt: Option<&Map<String, Value>> = Some(proxy);

    while let Some(current) = next_opt {
        let key = dart_string_of_field(current, "tag");
        if picked.contains_key(&key) {
            break;
        }
        let mut item = current.clone();
        item.shift_remove("domain_resolver");

        let next_lookup = match item.get("detour") {
            Some(Value::String(detour)) if !is_hidden_tag(detour) => {
                by_tag.get(detour.as_str()).copied()
            }
            _ => None,
        };
        if next_lookup.is_none() {
            item.shift_remove("detour");
        }
        picked.insert(key, item);
        next_opt = next_lookup;
    }

    if let Some((first_key, first_val)) = picked.first_mut() {
        first_val.insert("tag".to_string(), Value::String(base_tag(first_key)));
    }

    let mut endpoint_tags = HashSet::new();
    if let Some(Value::Array(list)) = config.get("endpoints") {
        for item in list {
            if let Some(obj) = item.as_object() {
                endpoint_tags.insert(dart_string_of_field(obj, "tag"));
            }
        }
    }

    let mut outbounds = Vec::new();
    let mut endpoints = Vec::new();
    for (tag_key, item) in picked {
        if endpoint_tags.contains(&tag_key) {
            endpoints.push(Value::Object(item));
        } else {
            outbounds.push(Value::Object(item));
        }
    }

    let mut root = Map::new();
    if !outbounds.is_empty() {
        root.insert("outbounds".to_string(), Value::Array(outbounds));
    }
    if !endpoints.is_empty() {
        root.insert("endpoints".to_string(), Value::Array(endpoints));
    }

    serde_json::to_string_pretty(&Value::Object(root)).unwrap_or_else(|_| "{}".to_string())
}

/// Resolves an `extra-security` value to the core's documented `ChainMode` key (`psiphon` or `warp_auto`).
pub fn extra_security_mode(value: Option<&str>) -> Option<&'static str> {
    let trimmed = value?.trim();
    match trimmed {
        "psiphon" => Some("psiphon"),
        "warp" | "warp_auto" => Some("warp_auto"),
        _ => None,
    }
}

/// Parses a `keep-top` header (`>= 1` or `-1` for all).
pub fn keep_top_header(value: Option<&str>) -> Option<i64> {
    let parsed = value?.trim().parse::<i64>().ok()?;
    if parsed < 1 && parsed != -1 {
        return None;
    }
    Some(parsed)
}

/// Builds the `tls-tricks` object from a subscription `fragment` triplet header.
pub fn fragment_tricks(value: Option<&str>) -> Option<Map<String, Value>> {
    let parts: Vec<&str> = value?.split(',').map(str::trim).collect();
    if parts.len() < 3 {
        return None;
    }
    let leading_method = parts[0].eq_ignore_ascii_case("tlshello");
    let size = if leading_method { parts[1] } else { parts[0] };
    let sleep = if leading_method { parts[2] } else { parts[1] };

    OptionalRange::try_parse(size, false)?;
    OptionalRange::try_parse(sleep, false)?;

    let mut map = Map::new();
    map.insert("enable-fragment".to_string(), Value::Bool(true));
    map.insert("fragment-size".to_string(), Value::String(size.to_string()));
    map.insert(
        "fragment-sleep".to_string(),
        Value::String(sleep.to_string()),
    );
    Some(map)
}

/// User override fields consumed by `profile_override`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserOverrideSpec {
    pub enable_fragment: Option<bool>,
    pub extra_security: Option<String>,
    pub keep_top: Option<i64>,
    pub keep_top_quick: Option<bool>,
    pub keep_top_countries: Option<Vec<String>>,
    pub keep_top_pinned: Option<Vec<String>>,
    pub keep_tags: Option<Vec<String>>,
}

fn json_value_to_dart_string(val: &Value) -> String {
    match val {
        Value::Null => "null".to_string(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Array(arr) => {
            let inner: Vec<String> = arr.iter().map(json_value_to_dart_string).collect();
            format!("[{}]", inner.join(", "))
        }
        Value::Object(map) => {
            let inner: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{k}: {}", json_value_to_dart_string(v)))
                .collect();
            format!("{{{}}}", inner.join(", "))
        }
    }
}

/// Builds the JSON override string sent to `hiddify-core`, matching `ProfileParser.profileOverride`.
pub fn profile_override(
    populated_headers: Option<&Map<String, Value>>,
    user_override: Option<&UserOverrideSpec>,
) -> String {
    let mut headers: Map<String, Value> = populated_headers.cloned().unwrap_or_default();

    let fragment_input = headers.shift_remove("fragment").map(|v| json_value_to_dart_string(&v));
    let extra_security_input = headers
        .shift_remove("extra-security")
        .map(|v| json_value_to_dart_string(&v));
    let keep_top_input = headers
        .shift_remove("keep-top")
        .map(|v| json_value_to_dart_string(&v));

    headers.shift_remove("keep-top-quick");
    headers.shift_remove("keep-top-countries");
    headers.shift_remove("keep-top-pinned");

    let uo_extra = user_override.and_then(|u| u.extra_security.as_deref());
    let mode = extra_security_mode(uo_extra)
        .or_else(|| extra_security_mode(extra_security_input.as_deref()));
    if let Some(m) = mode {
        headers.insert(
            "chain-status".to_string(),
            Value::String("extra_security".to_string()),
        );
        let mut es_map = Map::new();
        es_map.insert("mode".to_string(), Value::String(m.to_string()));
        headers.insert("extra-security".to_string(), Value::Object(es_map));
    }

    if user_override.and_then(|u| u.enable_fragment) == Some(true) {
        let mut tricks = Map::new();
        tricks.insert("enable-fragment".to_string(), Value::Bool(true));
        headers.insert("tls-tricks".to_string(), Value::Object(tricks));
    } else if let Some(tricks) = fragment_tricks(fragment_input.as_deref()) {
        headers.insert("tls-tricks".to_string(), Value::Object(tricks));
    }

    let keep_top_val = user_override
        .and_then(|u| u.keep_top)
        .or_else(|| keep_top_header(keep_top_input.as_deref()))
        .unwrap_or(KEEP_TOP_DEFAULT);
    headers.insert("keep-top".to_string(), Value::Number(keep_top_val.into()));

    if user_override.and_then(|u| u.keep_top_quick).unwrap_or(true) {
        headers.insert("keep-top-quick".to_string(), Value::Bool(true));
    }

    if let Some(countries) = user_override
        .and_then(|u| u.keep_top_countries.as_ref())
        .filter(|c| !c.is_empty())
    {
        headers.insert(
            "keep-top-countries".to_string(),
            Value::Array(countries.iter().cloned().map(Value::String).collect()),
        );
    }

    if let Some(pinned) = user_override
        .and_then(|u| u.keep_top_pinned.as_ref())
        .filter(|p| !p.is_empty())
    {
        headers.insert(
            "keep-top-pinned".to_string(),
            Value::Array(pinned.iter().cloned().map(Value::String).collect()),
        );
    }

    if let Some(tags) = user_override
        .and_then(|u| u.keep_tags.as_ref())
        .filter(|t| !t.is_empty())
    {
        headers.insert(
            "keep-tags".to_string(),
            Value::Array(tags.iter().cloned().map(Value::String).collect()),
        );
    }

    headers.retain(|key, value| {
        ALLOWED_OVERRIDE_CONFIGS.contains(&key.as_str())
            && !value.is_null()
            && !json_value_to_dart_string(value).is_empty()
    });

    serde_json::to_string(&Value::Object(headers)).unwrap_or_else(|_| "{}".to_string())
}

/// Recursively merges a JSON `profile_override` object into `main_config` (`ProfileParser.applyProfileOverride`).
pub fn apply_profile_override(
    main_config: &Map<String, Value>,
    profile_override_str: Option<&str>,
) -> Result<Map<String, Value>, ConfigError> {
    let mut merged = main_config.clone();
    let Some(override_raw) = profile_override_str else {
        return Ok(merged);
    };
    if !override_raw.contains('{') {
        return Ok(merged);
    }
    let parsed: Value = serde_json::from_str(override_raw)?;
    if let Value::Object(override_map) = parsed {
        merge_json_maps(&mut merged, override_map);
    }
    Ok(merged)
}

/// Preserves the user's own `remote-dns-address` whenever `merged["chain-status"] == "extra_security"`,
/// matching `ConfigOptionRepository.keepOwnDnsBehindExtraSecurity`.
pub fn keep_own_dns_behind_extra_security(
    own: &Map<String, Value>,
    merged: &Map<String, Value>,
) -> Map<String, Value> {
    if merged.get("chain-status").and_then(Value::as_str) != Some("extra_security") {
        return merged.clone();
    }
    let mut out = merged.clone();
    let own_dns = own
        .get("remote-dns-address")
        .cloned()
        .unwrap_or(Value::Null);
    out.insert("remote-dns-address".to_string(), own_dns);
    out
}

/// Combines `apply_profile_override` and `keep_own_dns_behind_extra_security` in one pass
/// (`ConfigOptionRepository.fullOptionsOverrided`).
pub fn merge_options_with_profile_override(
    own: &Map<String, Value>,
    profile_override_str: Option<&str>,
) -> Result<Map<String, Value>, ConfigError> {
    let merged = apply_profile_override(own, profile_override_str)?;
    Ok(keep_own_dns_behind_extra_security(own, &merged))
}

fn merge_json_maps(main: &mut Map<String, Value>, override_map: Map<String, Value>) {
    for (key, value) in override_map {
        match (main.get_mut(&key), value) {
            (Some(Value::Object(existing_obj)), Value::Object(override_obj)) => {
                merge_json_maps(existing_obj, override_obj);
            }
            (_, new_val) => {
                main.insert(key, new_val);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn count_proxies_and_country_flags() {
        let cfg = json!({
            "outbounds": [
                {"type": "vless", "tag": "🇩🇪 a § 0"},
                {"type": "trojan", "tag": "b § 1"},
                {"type": "direct", "tag": "own direct"},
                {"type": "socks", "tag": "hidden §hide§"},
                {"type": "selector", "tag": "group"},
                {"type": "urltest", "tag": "auto"},
                {"type": "direct", "tag": "direct"},
                {"type": "block", "tag": "block"},
                {"type": "dns", "tag": "dns-out"}
            ],
            "endpoints": [
                {"type": "wireguard", "tag": "wg § 2"}
            ]
        })
        .to_string();

        assert_eq!(count_proxies(&cfg), 4);
        assert_eq!(country_of("🇩🇪 a § 0").as_deref(), Some("DE"));
        assert_eq!(flag_of("DE"), "🇩🇪");
    }

    #[test]
    fn single_proxy_config_preserves_detour_and_strips_hidden() {
        let generated = json!({
            "outbounds": [
                {"type": "selector", "tag": "select", "outbounds": ["a § 0", "b § 1"]},
                {"type": "vless", "tag": "a § 0", "server": "1.2.3.4", "detour": "direct-fragment §hide§", "domain_resolver": "dns-direct"},
                {"type": "trojan", "tag": "b § 1", "server": "5.6.7.8", "detour": "a § 0"},
                {"type": "direct", "tag": "direct-fragment §hide§"}
            ],
            "endpoints": [
                {"type": "wireguard", "tag": "wg § 2", "detour": "a § 0"}
            ]
        })
        .to_string();

        let a_only: Value = serde_json::from_str(
            &single_proxy_config("a § 0", "", 0, None, Some(&generated)).unwrap(),
        )
        .unwrap();
        assert_eq!(
            a_only,
            json!({"outbounds": [{"type": "vless", "tag": "a", "server": "1.2.3.4"}]})
        );

        let wg_chain: Value = serde_json::from_str(
            &single_proxy_config("wg § 2", "", 0, None, Some(&generated)).unwrap(),
        )
        .unwrap();
        assert_eq!(
            wg_chain["endpoints"][0],
            json!({"type": "wireguard", "tag": "wg", "detour": "a § 0"})
        );
        assert_eq!(wg_chain["outbounds"][0]["tag"], "a § 0");
    }

    #[test]
    fn profile_override_matches_dart_rules() {
        let mut headers = Map::new();
        headers.insert("extra-security".to_string(), json!("warp"));
        headers.insert("fragment".to_string(), json!("tlshello,2-4,10-20"));
        headers.insert("keep-top".to_string(), json!("10"));
        headers.insert("keep-top-countries".to_string(), json!("DE"));

        let out: Value =
            serde_json::from_str(&profile_override(Some(&headers), None)).expect("override json");
        assert_eq!(out["chain-status"], "extra_security");
        assert_eq!(out["extra-security"]["mode"], "warp_auto");
        assert_eq!(out["tls-tricks"]["fragment-size"], "2-4");
        assert_eq!(out["tls-tricks"]["fragment-sleep"], "10-20");
        assert_eq!(out["keep-top"], 10);
        assert_eq!(out["keep-top-quick"], true);
        assert!(out.get("keep-top-countries").is_none());
    }

    #[test]
    fn extra_security_dns_guard_and_builtin_rules() {
        let own: Map<String, Value> = serde_json::from_value(json!({
            "remote-dns-address": "udp://9.9.9.9",
            "direct-dns-address": "udp://1.1.1.1",
            "chain-status": "off"
        }))
        .unwrap();
        let ov = json!({
            "remote-dns-address": "https://dns.provider/dns-query",
            "chain-status": "extra_security"
        })
        .to_string();
        let merged = merge_options_with_profile_override(&own, Some(&ov)).unwrap();
        assert_eq!(merged["remote-dns-address"], "udp://9.9.9.9");
        assert_eq!(merged["chain-status"], "extra_security");

        let rules_ir: Value = serde_json::from_str(
            &route_rules::with_builtins_pb_json("", "ir", "ios").unwrap(),
        )
        .unwrap();
        assert_eq!(rules_ir["1"].as_array().unwrap().len(), 4);

        let rules_other: Value = serde_json::from_str(
            &route_rules::with_builtins_pb_json("", "other", "ios").unwrap(),
        )
        .unwrap();
        assert_eq!(rules_other["1"].as_array().unwrap().len(), 3);
    }
}
