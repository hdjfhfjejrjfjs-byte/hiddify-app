//! Subscription metadata & header parsing, `subscription-userinfo` tolerant decoding,
//! RFC 5987 `Content-Disposition` filename extraction, 7-level profile name resolution,
//! deep-link parsing & share-link generation, protocol detection, and ZIP config extraction.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::OnceLock;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use hiddify_net::is_url;
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use url::Url;
use zip::ZipArchive;

pub mod download;

/// Synthetic sentinel base (`1000 TiB`) for unlimited subscription traffic (`total = 0` or missing).
pub const INFINITE_TRAFFIC_THRESHOLD: i64 = 1_099_511_627_776_000;

/// Synthetic Unix timestamp sentinel (seconds) for subscriptions without expiry.
pub const INFINITE_TIME_THRESHOLD: i64 = 92_233_720_368;

/// Maximum allowed `profile-update-interval` in hours (1 year).
pub const MAX_UPDATE_INTERVAL_HOURS: i64 = 24 * 365;

/// Maximum uncompressed size (256 KiB) per file when merging a ZIP archive of configs.
pub const MAX_ZIP_FILE_BYTES: u64 = 256 * 1024;

/// Comment separator inserted between files merged from a ZIP archive.
pub const ZIP_TEXT_SEPARATOR: &str = "\n#===\n";

/// Allowed subscription headers retained by `populate_headers`.
pub const ALLOWED_PROFILE_HEADERS: &[&str] = &[
    "profile-title",
    "content-disposition",
    "subscription-userinfo",
    "profile-update-interval",
    "support-url",
    "profile-web-page-url",
    "ipinfo-url",
    "extra-security",
    "fragment",
    "keep-top",
];

/// Supported deep-link schemes (`LinkParser.protocols`).
pub const DEEP_LINK_PROTOCOLS: &[&str] = &[
    "hiddify",
    "v2ray",
    "v2rayn",
    "v2rayng",
    "clash",
    "clashmeta",
    "sing-box",
];

/// Characters percent-encoded by Dart's `Uri.replace(fragment: ...)` (everything outside RFC 3986 fragment chars).
const FRAGMENT_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

/// Decodes standard Base64 UTF-8 text if valid; otherwise returns `input` unchanged (`safeDecodeBase64`).
pub fn safe_decode_base64(input: &str) -> String {
    let Ok(bytes) = BASE64_STANDARD.decode(input.as_bytes()) else {
        return input.to_string();
    };
    String::from_utf8(bytes).unwrap_or_else(|_| input.to_string())
}

/// Decodes a `profile-title: base64:<value>` payload (accepting missing `=` padding).
/// Returns `""` when invalid Base64 or invalid UTF-8 (`ProfileParser.decodeBase64Title`).
pub fn decode_base64_title(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() % 4 == 1 {
        return String::new();
    }
    let mut normalized = trimmed.to_string();
    while normalized.len() % 4 != 0 {
        normalized.push('=');
    }
    let Ok(bytes) = BASE64_STANDARD.decode(normalized.as_bytes()) else {
        return String::new();
    };
    match String::from_utf8(bytes) {
        Ok(s) => s.trim().to_string(),
        Err(_) => String::new(),
    }
}

/// Strictly percent-decodes `%XX` sequences into UTF-8 (matching Dart `Uri.decodeComponent`).
pub fn decode_uri_component_strict(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len()
                || !bytes[i + 1].is_ascii_hexdigit()
                || !bytes[i + 2].is_ascii_hexdigit()
            {
                return None;
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    percent_decode_str(raw)
        .decode_utf8()
        .ok()
        .map(|cow| cow.into_owned())
}

/// Extracts a filename from an HTTP `Content-Disposition` header, preferring RFC 5987 `filename*=`
/// and falling back to `filename="..."` (`ProfileParser.filenameFromContentDisposition`).
pub fn filename_from_content_disposition(header: &str) -> String {
    static EXT_RE: OnceLock<Regex> = OnceLock::new();
    static QUOTED_RE: OnceLock<Regex> = OnceLock::new();

    let ext_re = EXT_RE.get_or_init(|| {
        Regex::new(r"(?i)filename\*\s*=\s*(?:[^']*'[^']*')?([^;]+)").expect("ext regex")
    });
    if let Some(caps) = ext_re.captures(header) {
        let raw = caps.get(1).map(|m| m.as_str().trim()).unwrap_or("");
        if !raw.is_empty() {
            if let Some(decoded) = decode_uri_component_strict(raw) {
                return decoded;
            }
        }
    }

    let quoted_re = QUOTED_RE.get_or_init(|| Regex::new(r#"filename="([^"]*)""#).expect("quoted"));
    if let Some(caps) = quoted_re.captures(header) {
        return caps
            .get(1)
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
    }

    String::new()
}

/// Extracts `# key: value` or `// key: value` headers from the first 10 lines of subscription content.
pub fn parse_headers_from_content(content: &str) -> Map<String, Value> {
    let decoded = safe_decode_base64(content);
    let mut headers = Map::new();
    for line in decoded.split('\n').take(10) {
        if line.starts_with('#') || line.starts_with("//") {
            let Some(colon_idx) = line.find(':') else {
                continue;
            };
            let prefix = &line[..colon_idx];
            let stripped = prefix
                .strip_prefix('#')
                .or_else(|| prefix.strip_prefix("//"))
                .unwrap_or(prefix);
            let key = stripped.trim().to_lowercase();
            let value = line[colon_idx + 1..].trim().to_string();
            headers.insert(key, Value::String(value));
        }
    }
    headers
}

fn value_is_non_empty(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !format!("{a:?}").is_empty(),
        _ => true,
    }
}

/// Merges content comment headers with remote HTTP response headers (remote takes precedence)
/// and filters to `ALLOWED_PROFILE_HEADERS`.
pub fn populate_headers(
    content: &str,
    remote_headers: Option<&Map<String, Value>>,
) -> Map<String, Value> {
    let content_headers = parse_headers_from_content(content);
    let mut merged: Map<String, Value> = remote_headers.cloned().unwrap_or_default();

    for (k, v) in content_headers {
        if !merged.contains_key(&k) {
            merged.insert(k, v);
        }
    }

    let mut out = Map::new();
    for (k, v) in merged {
        if ALLOWED_PROFILE_HEADERS.contains(&k.as_str()) && value_is_non_empty(&v) {
            out.insert(k, v);
        }
    }
    out
}

/// Parsed `subscription-userinfo` traffic & expiry counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionInfoSpec {
    pub upload: i64,
    pub download: i64,
    pub total: i64,
    /// Expiry timestamp in seconds since Unix epoch (`<= INFINITE_TIME_THRESHOLD`).
    pub expire_secs: i64,
}

fn parse_dart_non_negative_num_to_int(raw: &str) -> Option<i64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(int_val) = trimmed.parse::<i64>() {
        return (int_val >= 0).then_some(int_val);
    }
    let float_val = trimmed.parse::<f64>().ok()?;
    if !float_val.is_finite() || float_val < 0.0 {
        return None;
    }
    if float_val >= i64::MAX as f64 {
        Some(i64::MAX)
    } else {
        Some(float_val.trunc() as i64)
    }
}

/// Parses a `subscription-userinfo` header (`upload=0; download=1024; total=10240; expire=1704054600`),
/// matching `ProfileParser._parseSubscriptionInfo` 100% (including Dart 3 map pattern key presence rules).
pub fn parse_subscription_info(sub_info_str: &str) -> Option<SubscriptionInfoSpec> {
    let mut map: HashMap<String, Option<i64>> = HashMap::new();
    for segment in sub_info_str.split(';') {
        let parts: Vec<&str> = segment.split('=').collect();
        if parts.len() < 2 {
            continue;
        }
        let key = parts[0].trim();
        if key.is_empty() {
            continue;
        }
        let value = parse_dart_non_negative_num_to_int(parts[1]);
        map.insert(key.to_string(), value);
    }

    // Matches Dart 3 map pattern: `{"upload": final upload?, "download": final download?, "total": final total, "expire": var expire}`
    let upload = (*map.get("upload")?)?;
    let download = (*map.get("download")?)?;
    let total_opt = *map.get("total")?;
    let expire_opt = *map.get("expire")?;

    let total = match total_opt {
        None | Some(0) => INFINITE_TRAFFIC_THRESHOLD + 1,
        Some(t) => t,
    };
    let expire_secs = match expire_opt {
        None | Some(0) => INFINITE_TIME_THRESHOLD,
        Some(e) if e > INFINITE_TIME_THRESHOLD => INFINITE_TIME_THRESHOLD,
        Some(e) => e,
    };

    Some(SubscriptionInfoSpec {
        upload,
        download,
        total,
        expire_secs,
    })
}

/// Resolves the auto-update interval in hours (`1..=8760`), or `None` when disabled/unset.
pub fn parse_update_interval_hours(
    header_value: Option<&str>,
    override_hours: Option<i64>,
    is_auto_update_disable: bool,
) -> Option<i64> {
    if is_auto_update_disable {
        return None;
    }
    if let Some(hours) = override_hours {
        if hours > 0 {
            return Some(hours);
        }
    }
    let raw = header_value?.trim();
    let hours = raw.parse::<i64>().ok()?;
    if hours <= 0 {
        return None;
    }
    Some(hours.clamp(1, MAX_UPDATE_INTERVAL_HOURS))
}

/// Checks whether a line is an inner HTTP(S) subscription link (has non-empty path != `"/"`).
pub fn is_subscription_link(line: &str) -> bool {
    if !line.starts_with("http://") && !line.starts_with("https://") {
        return false;
    }
    let trimmed = line.trim_end();
    let Ok(parsed) = Url::parse(trimmed) else {
        return false;
    };
    let path = parsed.path();
    !path.is_empty() && path != "/"
}

fn proxy_type_label_from_json(key_lower: &str) -> Option<&'static str> {
    match key_lower {
        "direct" => Some("Direct"),
        "block" => Some("Block"),
        "dns" => Some("DNS"),
        "socks" => Some("SOCKS"),
        "http" => Some("HTTP"),
        "shadowsocks" => Some("Shadowsocks"),
        "vmess" => Some("VMess"),
        "trojan" => Some("Trojan"),
        "naive" => Some("Naive"),
        "wireguard" => Some("WireGuard"),
        "awg" => Some("AWG"),
        "hysteria" => Some("Hysteria"),
        "tor" => Some("Tor"),
        "ssh" => Some("SSH"),
        "shadowtls" => Some("ShadowTLS"),
        "shadowsocksr" => Some("ShadowsocksR"),
        "vless" => Some("VLESS"),
        "tuic" => Some("TUIC"),
        "hysteria2" => Some("Hysteria2"),
        "mieru" => Some("Mieru"),
        "selector" => Some("Selector"),
        "urltest" => Some("URLTest"),
        "balancer" => Some("Balancer"),
        "warp" => Some("Warp"),
        "xvless" => Some("xVLESS"),
        "xvmess" => Some("xVMess"),
        "xtrojan" => Some("xTrojan"),
        "xfreedom" => Some("xFragment"),
        "xshadowsocks" => Some("xShadowsocks"),
        "xsocks" => Some("xSocks"),
        "invalid" => Some("Invalid"),
        _ => None,
    }
}

fn scheme_label(scheme: &str) -> String {
    let base = scheme.split('+').next().unwrap_or(scheme);
    let lower = base.to_ascii_lowercase();
    if let Some(label) = proxy_type_label_from_json(&lower) {
        return label.to_string();
    }
    let mut chars = lower.chars();
    match chars.next() {
        Some(first) => {
            let mut s = first.to_uppercase().to_string();
            s.push_str(chars.as_str());
            s
        }
        None => "Unknown".to_string(),
    }
}

/// Parses a single line as a URI scheme + optional `#fragment`, matching Dart `Uri.tryParse`.
fn parse_line_scheme_and_fragment(line: &str) -> Option<(String, Option<String>)> {
    static SCHEME_RE: OnceLock<Regex> = OnceLock::new();
    let re = SCHEME_RE
        .get_or_init(|| Regex::new(r"^([A-Za-z][A-Za-z0-9+.-]*):(.*)$").expect("scheme regex"));

    // Dart `Uri.tryParse` rejects whitespace or control chars in unencoded URI strings
    if line.bytes().any(|b| b <= 0x20 || b == 0x7F) {
        return None;
    }
    let caps = re.captures(line)?;
    let scheme = caps.get(1)?.as_str().to_ascii_lowercase();
    let rest = caps.get(2)?.as_str();
    let fragment = match rest.find('#') {
        Some(idx) => Some(rest[idx + 1..].to_string()),
        None => None,
    };
    Some((scheme, fragment))
}

/// Detects the display name or protocol label of local profile content (`ProfileParser.protocol`).
pub fn detect_protocol(content: &str) -> String {
    static OVPN_REMOTE_RE: OnceLock<Regex> = OnceLock::new();
    if content.contains("[Interface]") {
        return "WireGuard".to_string();
    }
    let ovpn_re = OVPN_REMOTE_RE
        .get_or_init(|| Regex::new(r"(?m)^\s*remote\s").expect("ovpn remote regex"));
    if content.contains("<ca>") && ovpn_re.is_match(content) {
        return "OpenVPN".to_string();
    }

    let mut name: Option<String> = None;
    for line in content.split('\n') {
        if name.is_some() {
            break;
        }
        let Some((scheme, raw_frag)) = parse_line_scheme_and_fragment(line) else {
            continue;
        };
        let decoded_frag = match raw_frag {
            Some(ref f) => {
                let first_part = f.split(" -> ").next().unwrap_or("");
                match decode_uri_component_strict(first_part) {
                    Some(d) => Some(d),
                    None => continue, // Dart `Uri.decodeComponent` throws -> caught or skipped
                }
            }
            None => None,
        };

        name = match scheme.as_str() {
            "ss" | "ssconf" => Some(decoded_frag.unwrap_or_else(|| "Shadowsocks".to_string())),
            "vmess" => Some("VMess".to_string()),
            "vless" => Some(decoded_frag.unwrap_or_else(|| "VLESS".to_string())),
            "trojan" => Some(decoded_frag.unwrap_or_else(|| "Trojan".to_string())),
            "tuic" => Some(decoded_frag.unwrap_or_else(|| "TUIC".to_string())),
            "hy2" | "hysteria2" => Some(decoded_frag.unwrap_or_else(|| "Hysteria2".to_string())),
            "hy" | "hysteria" => Some(decoded_frag.unwrap_or_else(|| "Hysteria".to_string())),
            "ssh" => Some(decoded_frag.unwrap_or_else(|| "SSH".to_string())),
            "wg" => Some(decoded_frag.unwrap_or_else(|| "WireGuard".to_string())),
            "awg" => Some(decoded_frag.unwrap_or_else(|| "AWG".to_string())),
            "shadowtls" => Some(decoded_frag.unwrap_or_else(|| "ShadowTLS".to_string())),
            "mieru" => Some(decoded_frag.unwrap_or_else(|| "Mieru".to_string())),
            "warp" => Some(decoded_frag.unwrap_or_else(|| "Warp".to_string())),
            "ovpn" => Some(decoded_frag.unwrap_or_else(|| "OpenVPN".to_string())),
            other if !other.is_empty() && {
                let trimmed = line.trim_start();
                trimmed.len() > other.len() + 3
                    && trimmed[..other.len()].eq_ignore_ascii_case(other)
                    && trimmed[other.len()..].starts_with("://")
                || trimmed.eq_ignore_ascii_case(&format!("{other}://"))
            } =>
            {
                Some(decoded_frag.unwrap_or_else(|| scheme_label(other)))
            }
            _ => None,
        };
    }

    name.unwrap_or_else(|| "Unknown".to_string())
}

/// Resolves a profile's display name using the exact 7-step hierarchy in `ProfileParser.parse`.
pub fn resolve_profile_name(
    override_name: Option<&str>,
    profile_title_header: Option<&str>,
    content_disposition_header: Option<&str>,
    remote_url: Option<&str>,
    existing_local_name: Option<&str>,
    local_content: Option<&str>,
) -> String {
    let is_blank = |s: &str| s.trim().is_empty();
    let mut name = String::new();

    if let Some(o_name) = override_name {
        if !is_blank(o_name) {
            name = o_name.to_string();
        }
    }

    if is_blank(&name) {
        if let Some(title) = profile_title_header {
            if let Some(b64) = title.strip_prefix("base64:") {
                name = decode_base64_title(b64);
            } else {
                name = title.trim().to_string();
            }
        }
    }

    if is_blank(&name) {
        if let Some(cd) = content_disposition_header {
            name = filename_from_content_disposition(cd);
        }
    }

    if let Some(url_str) = remote_url {
        if is_blank(&name) {
            if let Some(hash_idx) = url_str.find('#') {
                name = url_str[hash_idx + 1..].to_string();
            }
        }
        if is_blank(&name) {
            if let Some(last_seg) = url_str.split('/').last() {
                static EXT_PATTERN: OnceLock<Regex> = OnceLock::new();
                let re = EXT_PATTERN.get_or_init(|| {
                    Regex::new(r"(?s)\.(json|yaml|yml|txt).*$").expect("ext pattern")
                });
                name = re.replace(last_seg, "").into_owned();
            }
        }
        if is_blank(&name) {
            name = "Remote Profile".to_string();
        }
    } else if is_blank(&name) {
        if let Some(existing) = existing_local_name {
            if !is_blank(existing) {
                return existing.to_string();
            }
        }
        name = detect_protocol(local_content.unwrap_or(""));
    }

    name
}

/// Resolves a profile's display name, lazily reading `temp_file_path` only when a local profile
/// has no override/header/existing name (`ProfileParser.parse`).
pub fn resolve_profile_name_from_path(
    override_name: Option<&str>,
    profile_title_header: Option<&str>,
    content_disposition_header: Option<&str>,
    remote_url: Option<&str>,
    existing_local_name: Option<&str>,
    temp_file_path: &str,
) -> Option<String> {
    if remote_url.is_some() {
        return Some(resolve_profile_name(
            override_name,
            profile_title_header,
            content_disposition_header,
            remote_url,
            existing_local_name,
            None,
        ));
    }
    let pre = resolve_profile_name(
        override_name,
        profile_title_header,
        content_disposition_header,
        None,
        existing_local_name,
        Some(""),
    );
    // If any of override/header/existing_local_name was non-blank, `pre` is not from local_content.
    let needs_file = override_name.map(|s| s.trim().is_empty()).unwrap_or(true)
        && profile_title_header
            .map(|t| {
                let resolved = match t.strip_prefix("base64:") {
                    Some(b64) => decode_base64_title(b64),
                    None => t.trim().to_string(),
                };
                resolved.trim().is_empty()
            })
            .unwrap_or(true)
        && content_disposition_header
            .map(|cd| filename_from_content_disposition(cd).trim().is_empty())
            .unwrap_or(true)
        && existing_local_name
            .map(|s| s.trim().is_empty())
            .unwrap_or(true);
    if !needs_file {
        return Some(pre);
    }
    let content = std::fs::read_to_string(temp_file_path).ok()?;
    Some(detect_protocol(&content))
}

/// Parsed subscription or deep-link target (`ProfileLink` in `link_parsers.dart`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileLink {
    pub url: String,
    pub name: String,
}

/// Generates a subscription share link preserving userinfo, non-default port, path, and query,
/// replacing fragment when `name` is `Some` (`LinkParser.generateSubShareLink`).
pub fn generate_sub_share_link(url_str: &str, name: Option<&str>) -> String {
    if url_str.bytes().any(|b| b <= 0x20 || b == 0x7F) {
        return String::new();
    }
    let Some(name_val) = name else {
        // Validate URI syntax loosely like `Uri.tryParse`
        if Url::parse(url_str).is_err() && !url_str.contains(':') {
            return String::new();
        }
        return url_str.to_string();
    };
    if Url::parse(url_str).is_err() {
        return String::new();
    }
    let base_without_frag = match url_str.find('#') {
        Some(idx) => &url_str[..idx],
        None => url_str,
    };
    let encoded_frag = utf8_percent_encode(name_val, FRAGMENT_ENCODE_SET).to_string();
    format!("{base_without_frag}#{encoded_frag}")
}

/// Parses a plain HTTP/HTTPS/FTP subscription link (`LinkParser.simple`).
pub fn parse_simple_link(link: &str) -> Option<ProfileLink> {
    if !is_url(link) {
        return None;
    }
    let trimmed = link.trim();
    let parsed = Url::parse(trimmed).ok()?;
    let name = parsed
        .query_pairs()
        .find(|(k, _)| k == "name")
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default();
    Some(ProfileLink {
        url: trimmed.to_string(),
        name,
    })
}

/// Parses a panel deep link (`hiddify://`, `v2ray://`, `clash://`, `sing-box://`, etc. — `LinkParser.deep`).
pub fn parse_deep_link(link: &str) -> Option<ProfileLink> {
    let trimmed = link.trim();
    let parsed = Url::parse(trimmed).ok()?;
    if !parsed.has_host() {
        return None;
    }
    let scheme = parsed.scheme().to_ascii_lowercase();
    if !DEEP_LINK_PROTOCOLS.contains(&scheme.as_str()) {
        return None;
    }

    let mut url_param: Option<String> = None;
    let mut name_param: Option<String> = None;
    for (k, v) in parsed.query_pairs() {
        if k == "url" && url_param.is_none() {
            url_param = Some(v.into_owned());
        } else if k == "name" && name_param.is_none() {
            name_param = Some(v.into_owned());
        }
    }

    let raw_fragment = parsed
        .fragment()
        .map(|f| decode_uri_component_strict(f).unwrap_or_else(|| f.to_string()))
        .unwrap_or_default();

    if scheme == "hiddify" {
        if let Some(u) = url_param {
            let resolved_name = match name_param {
                Some(n) if !n.is_empty() => n,
                _ => raw_fragment,
            };
            Some(ProfileLink {
                url: u,
                name: resolved_name,
            })
        } else {
            let path = parsed.path();
            let stripped_path = path.strip_prefix('/').unwrap_or(path);
            let full_url = match parsed.query() {
                Some(q) => format!("{stripped_path}?{q}"),
                None => stripped_path.to_string(),
            };
            Some(ProfileLink {
                url: full_url,
                name: raw_fragment,
            })
        }
    } else {
        url_param.map(|u| ProfileLink {
            url: u,
            name: name_param.unwrap_or_default(),
        })
    }
}

/// Parses either a simple subscription URL or a deep link (`LinkParser.parse`).
pub fn parse_link(link: &str) -> Option<ProfileLink> {
    parse_simple_link(link).or_else(|| parse_deep_link(link))
}

/// Decodes arbitrary bytes as UTF-8 text, stripping a leading UTF-8 BOM (`0xEF 0xBB 0xBF`)
/// and replacing malformed sequences with `U+FFFD` (`text_utils.dart:decodeTextBytes`).
pub fn decode_text_bytes(bytes: &[u8]) -> String {
    let body = if bytes.len() >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF {
        &bytes[3..]
    } else {
        bytes
    };
    String::from_utf8_lossy(body).into_owned()
}

/// Reads every valid config file in a ZIP archive in stored order, skipping folders,
/// `__MACOSX/`, `._*` metadata files, and files over 256 KiB, merging with `\n#===\n`
/// (`text_utils.dart:readZipText`).
pub fn read_zip_text(bytes: &[u8]) -> String {
    let Ok(mut archive) = ZipArchive::new(Cursor::new(bytes)) else {
        return String::new();
    };

    let mut texts = Vec::new();
    for idx in 0..archive.len() {
        let Ok(mut entry) = archive.by_index(idx) else {
            continue;
        };
        if !entry.is_file() || entry.size() > MAX_ZIP_FILE_BYTES {
            continue;
        }
        let normalized_name = entry.name().replace('\\', "/");
        let segments: Vec<&str> = normalized_name.split('/').collect();
        let file_name = segments.last().copied().unwrap_or("");
        if segments.contains(&"__MACOSX") || file_name.starts_with("._") {
            continue;
        }

        let mut buf = Vec::with_capacity(entry.size() as usize);
        if entry.read_to_end(&mut buf).is_ok() {
            texts.push(decode_text_bytes(&buf));
        }
    }

    texts.join(ZIP_TEXT_SEPARATOR)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    #[test]
    fn content_disposition_and_base64_title() {
        assert_eq!(
            filename_from_content_disposition("attachment; filename=\"MyProfile.txt\""),
            "MyProfile.txt"
        );
        assert_eq!(
            filename_from_content_disposition(
                "attachment; filename=\"fallback.txt\"; filename*=UTF-8''%D9%BE%D8%B1%D9%88%D9%81%D8%A7%DB%8C%D9%84"
            ),
            "پروفایل"
        );
        assert_eq!(
            filename_from_content_disposition(
                "attachment; filename=\"safe.txt\"; filename*=UTF-8''%zz%zz"
            ),
            "safe.txt"
        );

        assert_eq!(decode_base64_title("U3RvY2tob2xtIDE"), "Stockholm 1");
        assert_eq!(decode_base64_title("not base64!"), "");
    }

    #[test]
    fn subscription_userinfo_matches_all_dart_edge_cases() {
        let normal =
            parse_subscription_info("upload=0;download=1024;total=10240.5;expire=1704054600.55")
                .unwrap();
        assert_eq!(normal.total, 10240);
        assert_eq!(normal.expire_secs, 1704054600);

        let unlimited =
            parse_subscription_info("upload=100;download=200;total=1e999;expire=99999999999999999999")
                .unwrap();
        assert_eq!(unlimited.total, INFINITE_TRAFFIC_THRESHOLD + 1);
        assert_eq!(unlimited.expire_secs, INFINITE_TIME_THRESHOLD);

        for bad_upload in ["NaN", "-1", "-Infinity"] {
            assert!(parse_subscription_info(&format!(
                "upload={bad_upload};download=200;total=1000;expire=1704054600"
            ))
            .is_none());
        }
        assert!(parse_subscription_info("upload=100;download=200").is_none());
    }

    #[test]
    fn deep_links_and_protocol_detection() {
        let link = parse_link("hiddify://install-config?url=https%3A%2F%2Fsub.com%2Fx#MyProfile")
            .expect("deep link");
        assert_eq!(link.url, "https://sub.com/x");
        assert_eq!(link.name, "MyProfile");

        assert_eq!(
            generate_sub_share_link("https://panel.example.com:2053/sub/abc", Some("My Profile")),
            "https://panel.example.com:2053/sub/abc#My%20Profile"
        );
        assert_eq!(
            generate_sub_share_link("https://example.com/sub#Old", None),
            "https://example.com/sub#Old"
        );

        assert_eq!(detect_protocol("vless://uuid@1.2.3.4:443#My%20Server"), "My Server");
        assert_eq!(detect_protocol("naive+https://u:p@1.2.3.4:443"), "Naive");
        assert_eq!(detect_protocol("psiphon://"), "Psiphon");
        assert_eq!(
            detect_protocol("client\ndev tun\nremote vpn.example.com 1194\n<ca>\ncert\n</ca>"),
            "OpenVPN"
        );
    }

    #[test]
    fn zip_extraction_filters_macos_junk_and_oversized_files() {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut buf);
            let opts = SimpleFileOptions::default();
            zip.add_directory("export/", opts).unwrap();
            zip.start_file("export/. 🇳🇱 Free-1.conf", opts).unwrap();
            zip.write_all(&[0xEF, 0xBB, 0xBF, b'k', b'e', b'p', b't'])
                .unwrap();
            zip.start_file("__MACOSX/export/._junk.conf", opts).unwrap();
            zip.write_all(b"junk").unwrap();
            zip.start_file("export/._other.conf", opts).unwrap();
            zip.write_all(b"junk").unwrap();
            zip.start_file("export/big.conf", opts).unwrap();
            zip.write_all(&vec![b'x'; 256 * 1024 + 1]).unwrap();
            zip.start_file("export/.hidden", opts).unwrap();
            zip.write_all(b"kept too").unwrap();
            zip.finish().unwrap();
        }
        assert_eq!(read_zip_text(buf.get_ref()), "kept\n#===\nkept too");
    }

    #[test]
    fn download_problem_and_link_expansion() {
        use download::*;

        assert_eq!(DownloadProblemKind::of_core(0, 200), None);
        assert_eq!(
            DownloadProblemKind::of_core(4, 0),
            Some(DownloadProblemKind::Reset)
        );
        assert_eq!(
            DownloadProblemKind::classify_dio(None, "unknown", false, false, "Connection refused"),
            DownloadProblemKind::Refused
        );
        assert_eq!(
            DownloadProblemKind::most_telling(&[
                DownloadProblemKind::Reset,
                DownloadProblemKind::Denied
            ]),
            DownloadProblemKind::Denied
        );
        assert!(!DownloadProblemKind::Denied.may_pass());
        assert!(DownloadProblemKind::ServerError.may_pass());

        let plan = plan_link_expansion(
            "vless://a\nhttps://sub.example.com/path \ntrojan://b",
            false,
        );
        assert_eq!(plan.targets.len(), 1);
        assert_eq!(plan.targets[0].index, 1);
        assert_eq!(plan.targets[0].url, "https://sub.example.com/path");

        let merged = merge_expanded_links(&plan.decoded_text, &[(1, "  ss://expanded  ".to_string())]);
        assert_eq!(merged, "vless://a\nss://expanded\ntrojan://b");
    }
}
