//! Network rule validation, IPv4/IPv6 privacy masking, and proxy tag/endpoint display parsers.
//! Designed as a zero-UI, deterministic standalone crate with 100% behavioral equivalence
//! to Hiddify's Dart validators and utilities.

use std::net::{IpAddr, Ipv6Addr};
use std::str::FromStr;
use std::sync::OnceLock;

use percent_encoding::percent_decode_str;
use regex::Regex;
use serde::{Deserialize, Serialize};

/// Fallback obscured IP string when input is neither dotted IPv4 nor colon-separated IPv6.
pub const FALLBACK_OBSCURED_ADDRESS: &str = "*.*.*.*";

/// Validates an HTTP/HTTPS/FTP URL using Hiddify's exact URL rule (`validators.dart:isUrl`).
pub fn is_url(input: &str) -> bool {
    static URL_RE: OnceLock<Regex> = OnceLock::new();
    let re = URL_RE.get_or_init(|| {
        Regex::new(
            r"^(?:(?:https?|ftp)://)(?:\S+(?::\S*)?@)?(?:(?:[0-9]{1,3}\.){3}[0-9]{1,3}|(?:(?:[a-z\u{00a1}-\u{d7ff}\u{e000}-\u{10ffff}0-9]+-?)*[a-z\u{00a1}-\u{d7ff}\u{e000}-\u{10ffff}0-9]+)(?:\.(?:[a-z\u{00a1}-\u{d7ff}\u{e000}-\u{10ffff}0-9]+-?)*[a-z\u{00a1}-\u{d7ff}\u{e000}-\u{10ffff}0-9]+)*(?:\.(?:[a-z\u{00a1}-\u{d7ff}\u{e000}-\u{10ffff}]{2,})))(?::\d{2,5})?(?:/[^\s]*)?#?.*\z",
        )
        .expect("valid url regex")
    });
    let normalized = input.trim().to_lowercase();
    re.is_match(&normalized)
}

/// Validates a TCP/UDP port number (`1..=65535`, no leading zeros).
pub fn is_port(input: &str) -> bool {
    static PORT_RE: OnceLock<Regex> = OnceLock::new();
    let re = PORT_RE.get_or_init(|| {
        Regex::new(r"^([1-9][0-9]{0,3}|[1-5][0-9]{4}|6[0-4][0-9]{3}|65[0-4][0-9]{2}|655[0-2][0-9]|6553[0-5])\z")
            .expect("valid port regex")
    });
    re.is_match(input)
}

/// Validates a port or `start:end` port range (`1..=65000`, matching `validators.dart:isPortOrPortRange`).
pub fn is_port_or_port_range(input: &str) -> bool {
    static PORT_RANGE_RE: OnceLock<Regex> = OnceLock::new();
    let re = PORT_RANGE_RE.get_or_init(|| {
        Regex::new(r"^(65000|6[0-4]\d{3}|[1-5]\d{4}|[1-9]\d{0,3})(:(65000|6[0-4]\d{3}|[1-5]\d{4}|[1-9]\d{0,3}))?\z")
            .expect("valid port range regex")
    });
    re.is_match(input)
}

/// Validates a Windows process executable name (`*.exe`, `*.bat`, `*.cmd`, `*.com`).
pub fn is_windows_process_name(input: &str) -> bool {
    let Some((stem, ext)) = input.rsplit_once('.') else {
        return false;
    };
    if !matches!(ext.to_ascii_lowercase().as_str(), "exe" | "bat" | "cmd" | "com") {
        return false;
    }
    is_valid_windows_stem(stem)
}

fn is_valid_windows_segment(seg: &str) -> bool {
    !seg.is_empty()
        && !seg
            .chars()
            .any(|c| matches!(c, '<' | '>' | ':' | '"' | '\\' | '/' | '|' | '?' | '*'))
}

fn is_valid_windows_stem(stem: &str) -> bool {
    if !is_valid_windows_segment(stem) {
        return false;
    }
    let Some(last_char) = stem.chars().last() else {
        return false;
    };
    !last_char.is_whitespace() && last_char != '.'
}

/// Validates a Linux process name (`^[^\\]+$`).
pub fn is_linux_process_name(input: &str) -> bool {
    !input.is_empty() && !input.contains('\\')
}

/// Validates a macOS process name (`^[^\\:]+$`).
pub fn is_macos_process_name(input: &str) -> bool {
    !input.is_empty() && !input.contains('\\') && !input.contains(':')
}

/// Validates a process name on any desktop OS (Windows, Linux, or macOS).
pub fn is_process_name(input: &str) -> bool {
    is_windows_process_name(input) || is_linux_process_name(input) || is_macos_process_name(input)
}

/// Validates a Windows absolute process path (`C:\dir\app.exe`).
pub fn is_windows_process_path(input: &str) -> bool {
    let bytes = input.as_bytes();
    if bytes.len() < 4 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' || bytes[2] != b'\\' {
        return false;
    }
    let rest = &input[3..];
    let parts: Vec<&str> = rest.split('\\').collect();
    if parts.is_empty() {
        return false;
    }
    for &dir in &parts[..parts.len() - 1] {
        if !is_valid_windows_segment(dir) {
            return false;
        }
    }
    is_windows_process_name(parts[parts.len() - 1])
}

/// Validates a Linux absolute process path (`^/([^/]*(?<!/)/)*[^/]+$`).
pub fn is_linux_process_path(input: &str) -> bool {
    let Some(rest) = input.strip_prefix('/') else {
        return false;
    };
    if rest.is_empty() {
        return false;
    }
    rest.split('/').all(|seg| !seg.is_empty())
}

/// Validates a macOS absolute process path (`^/([^/:]*(?<!/)/)*[^/:]+$`).
pub fn is_macos_process_path(input: &str) -> bool {
    is_linux_process_path(input) && !input.contains(':')
}

/// Validates a process path on Windows, Linux, or macOS.
pub fn is_process_path(input: &str) -> bool {
    is_windows_process_path(input) || is_linux_process_path(input) || is_macos_process_path(input)
}

/// Parses an IPv4 address matching Dart's `Uri.parseIPv4Address` (4 decimal octets `0..=255`).
pub fn is_valid_dart_ipv4(addr: &str) -> bool {
    let parts: Vec<&str> = addr.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    for part in parts {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        let Ok(val) = part.parse::<u16>() else {
            return false;
        };
        if val > 255 {
            return false;
        }
    }
    true
}

/// Validates an IPv4 or IPv6 address with optional `/prefix` CIDR notation (`validators.dart:isIpCidr`).
pub fn is_ip_cidr(input: &str) -> bool {
    static CIDR_PREFIX_RE: OnceLock<Regex> = OnceLock::new();
    let prefix_re = CIDR_PREFIX_RE.get_or_init(|| Regex::new(r"^(0|[1-9]\d{0,2})\z").expect("regex"));

    let parts: Vec<&str> = input.split('/').collect();
    if parts.len() > 2 {
        return false;
    }
    let ip_part = parts[0];
    let is_ipv6 = ip_part.contains(':');
    let valid_ip = if is_ipv6 {
        Ipv6Addr::from_str(ip_part).is_ok()
    } else {
        is_valid_dart_ipv4(ip_part)
    };
    if !valid_ip {
        return false;
    }
    if parts.len() == 1 {
        return true;
    }
    let prefix_str = parts[1];
    if !prefix_re.is_match(prefix_str) {
        return false;
    }
    let Ok(prefix) = prefix_str.parse::<u16>() else {
        return false;
    };
    prefix <= if is_ipv6 { 128 } else { 32 }
}

/// Validates a standard domain name (`^([a-zA-Z\d\-]+\.)+[a-zA-Z\d\-]{2,}$`).
pub fn is_domain(input: &str) -> bool {
    static DOMAIN_RE: OnceLock<Regex> = OnceLock::new();
    let re = DOMAIN_RE
        .get_or_init(|| Regex::new(r"(?i)^([a-zA-Z\d\-]+\.)+[a-zA-Z\d\-]{2,}\z").expect("regex"));
    re.is_match(input)
}

/// Validates a sing-box `domain_suffix` value (`^\.?([a-zA-Z\d\-_]+\.)*[a-zA-Z\d\-]{2,}$`).
pub fn is_domain_suffix(input: &str) -> bool {
    static SUFFIX_RE: OnceLock<Regex> = OnceLock::new();
    let re = SUFFIX_RE.get_or_init(|| {
        Regex::new(r"(?i)^\.?([a-zA-Z\d\-_]+\.)*[a-zA-Z\d\-]{2,}\z").expect("regex")
    });
    re.is_match(input)
}

/// Masks an IPv4 or IPv6 address for UI display (`1.1.1.1` -> `1.*.*.1`, `::1` -> `::*`),
/// matching `obscureIp` in `lib/core/utils/ip_utils.dart`.
pub fn obscure_ip(ip: &str) -> String {
    if ip.contains('.') {
        let splits: Vec<&str> = ip.split('.').collect();
        if let (Some(first), Some(last)) = (splits.first(), splits.last()) {
            return format!("{first}.*.*.{last}");
        }
    } else if ip.contains(':') {
        let splits: Vec<&str> = ip.split(':').collect();
        if let Some((&first, rest)) = splits.split_first() {
            let mut out = Vec::with_capacity(splits.len());
            out.push(first.to_string());
            for part in rest {
                let utf16_count = part.encode_utf16().count();
                out.push("*".repeat(utf16_count));
            }
            return out.join(":");
        }
    }
    FALLBACK_OBSCURED_ADDRESS.to_string()
}

/// Strips the core's duplicate/hidden `§...` suffix and trailing whitespace from a proxy tag.
pub fn sanitize_tag(tag: &str) -> String {
    let prefix = match tag.find('§') {
        Some(idx) => &tag[..idx],
        None => tag,
    };
    prefix.trim_end().to_string()
}

/// Strips the transient status emoji suffix (` ⏳ Connecting...`) from a proxy display type.
pub fn strip_status_mark(proxy_type: &str) -> String {
    static STATUS_MARK_RE: OnceLock<Regex> = OnceLock::new();
    let re = STATUS_MARK_RE
        .get_or_init(|| Regex::new(r"\s+(⏳|⌛|🔍|❌|🔑)\s.*$").expect("status mark regex"));
    if let Some(m) = re.find(proxy_type) {
        let mut out = String::with_capacity(proxy_type.len());
        out.push_str(&proxy_type[..m.start()]);
        out.push_str(&proxy_type[m.end()..]);
        out
    } else {
        proxy_type.to_string()
    }
}

/// Checks whether a proxy display type (with status mark already stripped) represents an invalid config.
pub fn is_invalid_proxy_type(type_without_status: &str) -> bool {
    static INVALID_RE: OnceLock<Regex> = OnceLock::new();
    let re = INVALID_RE.get_or_init(|| Regex::new(r"^Invalid:?\s+\S").expect("invalid regex"));
    re.is_match(type_without_status)
}

/// Split result of a WARP scanner endpoint embedded in a proxy display type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScannerEndpointParts {
    pub before: String,
    pub endpoint: String,
    pub after: String,
}

/// Splits a WARP scanner display string (`"WARP · 🔍 162.159.192.1:2408 ⏳ Connecting"`).
pub fn split_scanner_endpoint(proxy_type: &str) -> Option<ScannerEndpointParts> {
    static SCANNER_RE: OnceLock<Regex> = OnceLock::new();
    let re = SCANNER_RE.get_or_init(|| {
        Regex::new(r"^(.*?) · 🔍 (\[[0-9a-fA-F:]+\]:\d+|\d+\.\d+\.\d+\.\d+:\d+)(.*)\z")
            .expect("scanner regex")
    });
    let caps = re.captures(proxy_type)?;
    Some(ScannerEndpointParts {
        before: caps.get(1)?.as_str().to_string(),
        endpoint: caps.get(2)?.as_str().to_string(),
        after: caps.get(3)?.as_str().to_string(),
    })
}

/// Checks whether raw 4-byte IPv4 or 16-byte IPv6 address bytes belong to a local network:
/// IPv4-mapped IPv6 unwrap (`::ffff:a.b.c.d`), loopback (`127/8`, `::1`), link-local (`169.254/16`, `fe80::/10`),
/// RFC 1918 private IPv4 (`10/8`, `172.16/12`, `192.168/16`), or IPv6 unique-local (`fc00::/7`).
/// Mirrors `isLocalNetwork` in `phone_import_server.dart`.
pub fn is_local_network_bytes(raw_ip: &[u8]) -> bool {
    match raw_ip.len() {
        4 => is_local_ipv4([raw_ip[0], raw_ip[1], raw_ip[2], raw_ip[3]]),
        16 => {
            if raw_ip[..10].iter().all(|&b| b == 0) && raw_ip[10] == 0xff && raw_ip[11] == 0xff {
                return is_local_ipv4([raw_ip[12], raw_ip[13], raw_ip[14], raw_ip[15]]);
            }
            let v6 = Ipv6Addr::from(<[u8; 16]>::try_from(raw_ip).unwrap());
            if v6.is_loopback() {
                return true;
            }
            // Link-local fe80::/10
            if raw_ip[0] == 0xfe && (raw_ip[1] & 0xc0) == 0x80 {
                return true;
            }
            // Unique-local fc00::/7
            (raw_ip[0] & 0xfe) == 0xfc
        }
        _ => false,
    }
}

fn is_local_ipv4(b: [u8; 4]) -> bool {
    // Loopback 127.0.0.0/8 or Link-local 169.254.0.0/16
    if b[0] == 127 || (b[0] == 169 && b[1] == 254) {
        return true;
    }
    b[0] == 10 || (b[0] == 172 && (16..=31).contains(&b[1])) || (b[0] == 192 && b[1] == 168)
}

/// String-parsing convenience wrapper around `is_local_network_bytes`.
pub fn is_local_network_ip(ip_str: &str) -> bool {
    match IpAddr::from_str(ip_str.trim()) {
        Ok(IpAddr::V4(v4)) => is_local_network_bytes(&v4.octets()),
        Ok(IpAddr::V6(v6)) => is_local_network_bytes(&v6.octets()),
        Err(_) => false,
    }
}

/// Validates the HTTP `Host` header against `expected_authority` (case-insensitive)
/// to block DNS rebinding attacks against local servers.
pub fn is_allowed_host_header(host_header: Option<&str>, expected_authority: &str) -> bool {
    host_header
        .map(|h| h.eq_ignore_ascii_case(expected_authority))
        .unwrap_or(false)
}

/// Outcome of parsing an `application/x-www-form-urlencoded` body sent to `PhoneImportServer`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "text", rename_all = "snake_case")]
pub enum PhoneImportFormResult {
    Ok(String),
    Empty,
    TooLarge,
    BadRequest,
}

fn decode_form_component_strict(raw: &str) -> Option<String> {
    let plus_as_space = raw.replace('+', " ");
    let bytes = plus_as_space.as_bytes();
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
    percent_decode_str(&plus_as_space)
        .decode_utf8()
        .ok()
        .map(|c| c.into_owned())
}

/// Parses an `application/x-www-form-urlencoded` body (`link=...`), enforcing strict UTF-8 & `%XX`
/// decoding and `max_text_bytes` UTF-8 length limit (`PhoneImportServer._receive`).
pub fn parse_phone_import_form(body: &[u8], max_text_bytes: usize) -> PhoneImportFormResult {
    if body.len() > max_text_bytes.saturating_mul(3) {
        return PhoneImportFormResult::TooLarge;
    }
    let Ok(body_str) = std::str::from_utf8(body) else {
        return PhoneImportFormResult::BadRequest;
    };
    let mut link_val: Option<String> = None;
    if !body_str.is_empty() {
        for pair in body_str.split('&') {
            if pair.is_empty() {
                continue;
            }
            let (raw_k, raw_v) = match pair.split_once('=') {
                Some((k, v)) => (k, v),
                None => (pair, ""),
            };
            let Some(k) = decode_form_component_strict(raw_k) else {
                return PhoneImportFormResult::BadRequest;
            };
            let Some(v) = decode_form_component_strict(raw_v) else {
                return PhoneImportFormResult::BadRequest;
            };
            if k == "link" {
                link_val = Some(v);
            }
        }
    }
    let trimmed = link_val.unwrap_or_default().trim().to_string();
    if trimmed.len() > max_text_bytes {
        return PhoneImportFormResult::TooLarge;
    }
    if trimmed.is_empty() {
        return PhoneImportFormResult::Empty;
    }
    PhoneImportFormResult::Ok(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_suffix_matches_dart_tests() {
        for valid in [
            ".com",
            ".ir",
            "ir",
            "noon.com",
            ".noon.com",
            "mtalk.google.com",
            "xn--mgba3a4f16a.ir",
        ] {
            assert!(is_domain_suffix(valid), "expected valid: {valid}");
        }
        for invalid in [
            "",
            ".",
            "..com",
            "noon..com",
            "noon.com.",
            ".c",
            "noon com",
            "http://noon.com",
            "*.noon.com",
        ] {
            assert!(!is_domain_suffix(invalid), "expected invalid: {invalid}");
        }
    }

    #[test]
    fn ip_cidr_and_obscure_ip() {
        assert!(is_ip_cidr("192.168.1.1/24"));
        assert!(is_ip_cidr("01.2.3.4/32"));
        assert!(is_ip_cidr("2001:db8::1/128"));
        assert!(!is_ip_cidr("192.168.1.1/33"));
        assert!(!is_ip_cidr("2001:db8::1/129"));
        assert!(!is_ip_cidr("192.168.1.1/01"));

        assert_eq!(obscure_ip("1.1.1.1"), "1.*.*.1");
        assert_eq!(
            obscure_ip("FEDC:BA98:7654:3210:FEDC:BA98:7654:3210"),
            "FEDC:****:****:****:****:****:****:****"
        );
        assert_eq!(obscure_ip("::1"), "::*");
        assert_eq!(obscure_ip("invalid"), "*.*.*.*");
    }

    #[test]
    fn process_and_port_validators() {
        assert!(is_port("443"));
        assert!(is_port("65535"));
        assert!(!is_port("0"));
        assert!(!is_port("65536"));

        assert!(is_port_or_port_range("80:443"));
        assert!(is_port_or_port_range("65000"));
        assert!(!is_port_or_port_range("65001"));

        assert!(is_windows_process_path(r"C:\Program Files\App\app.exe"));
        assert!(!is_windows_process_path(r"C:\\Program Files\app.exe"));
        assert!(!is_windows_process_path(r"C:\Program Files\app .exe"));
        assert!(is_linux_process_path("/usr/bin/curl"));
        assert!(!is_linux_process_path("/usr//bin/curl"));
        assert!(!is_linux_process_path("/"));
    }

    #[test]
    fn tag_and_scanner_endpoint_helpers() {
        assert_eq!(sanitize_tag("Germany § 2"), "Germany");
        assert_eq!(
            split_scanner_endpoint("WARP · 🔍 [2606:4700::1]:443 ⏳ Connecting"),
            Some(ScannerEndpointParts {
                before: "WARP".to_string(),
                endpoint: "[2606:4700::1]:443".to_string(),
                after: " ⏳ Connecting".to_string(),
            })
        );
    }

    #[test]
    fn lan_security_and_form_parsing() {
        for local in [
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.1",
            "192.168.1.20",
            "169.254.1.1",
            "127.0.0.1",
            "fe80::1",
            "fd12:3456::1",
            "::ffff:192.168.1.20",
        ] {
            assert!(is_local_network_ip(local), "expected local: {local}");
        }
        for public in [
            "8.8.8.8",
            "172.32.0.1",
            "100.64.0.1",
            "2001:4860::8888",
            "::ffff:8.8.8.8",
        ] {
            assert!(!is_local_network_ip(public), "expected non-local: {public}");
        }

        assert!(is_allowed_host_header(Some("127.0.0.1:8080"), "127.0.0.1:8080"));
        assert!(!is_allowed_host_header(Some("evil.example"), "127.0.0.1:8080"));

        assert_eq!(
            parse_phone_import_form(b"link=+https%3A%2F%2Fexample.com%2Fsub%3Fa%3D1%26b%3D2+", 65536),
            PhoneImportFormResult::Ok("https://example.com/sub?a=1&b=2".to_string())
        );
        assert_eq!(
            parse_phone_import_form(b"link=+++", 65536),
            PhoneImportFormResult::Empty
        );
        assert_eq!(
            parse_phone_import_form(b"link=abcdef", 3),
            PhoneImportFormResult::TooLarge
        );
        assert_eq!(
            parse_phone_import_form(b"link=%zz", 65536),
            PhoneImportFormResult::BadRequest
        );
    }
}
