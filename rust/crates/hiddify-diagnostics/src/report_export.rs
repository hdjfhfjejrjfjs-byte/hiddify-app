//! Full diagnostics report scrubber, crash scanner, ZIP packager, and X25519 `age` encryptor.
//!
//! 100% behavior-equivalent port of:
//! - `hiddify-sing-box/experimental/libbox/h_report_sanitize.go` (`CreateZipArchive` + `sanitizeReportText`)
//! - `hiddify-core/v2/hcore/diagnostics.go` (`listCrashReports` + `exportDiagnostics`)
//! - `hiddify-core/v2/hcore/diagnostics_encryption.go` (`encryptDirectory` + `reportReadme`)

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::net::Ipv6Addr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use age::x25519::Recipient;
use hiddify_config::sanitize_config_json;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::log_crash_store::format_utc_millis_stamp;

pub const MAX_DIAGNOSTICS_REPORTS: usize = 5;
pub const MAX_DIAGNOSTICS_LOG_SIZE: u64 = 2 << 20;
pub const SENSITIVE_DIRECTORY: &str = "sensitive";
pub const ENCRYPTED_SENSITIVE_FILE: &str = "sensitive.encrypted.age";
pub const REPORT_NUMBER_FILE: &str = "diagnostics-number";

const REPORT_DIRECTORIES: &[&str] = &["crash_reports", "oom_reports", "power_reports"];

const SETUP_MODE_NAMES: &[&str] = &[
    "OLD",
    "GRPC_NORMAL",
    "GRPC_BACKGROUND",
    "GRPC_NORMAL_INSECURE",
    "GRPC_BACKGROUND_INSECURE",
];

const APP_SUMMARY_SECTIONS: &[&str] = &[
    "app",
    "system",
    "connection",
    "profiles",
    "core",
    "settings",
];

// ============================================================================
// 1. Report Text & File Scrubber (`libbox/h_report_sanitize.go`)
// ============================================================================

pub fn sensitive_report_file(name: &str) -> bool {
    name == "configuration.json" || name.ends_with(".connections.json")
}

pub fn text_report_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".log") || lower.ends_with(".txt") || lower.ends_with(".json")
}

struct ScrubberRule {
    pattern: Regex,
    replacement: &'static str,
}

fn report_scrubbers() -> &'static [ScrubberRule] {
    static RULES: OnceLock<Vec<ScrubberRule>> = OnceLock::new();
    RULES.get_or_init(|| {
        vec![
            ScrubberRule {
                pattern: Regex::new(r#"(?i)\b(?:vless|vmess|trojan|ss|ssr|socks5?|hysteria2?|hy2|tuic|wireguard|wg|awg|warp|mieru|naive|anytls|shadowtls|sdns|ssh|sudoku)://[^\s"'<>]+"#).unwrap(),
                replacement: "<link>",
            },
            ScrubberRule {
                pattern: Regex::new(r#"(?i)\b(https?|wss?)://([^/\s"'<>?#]+)[^\s"'<>]*"#).unwrap(),
                replacement: "${1}://<host>",
            },
            ScrubberRule {
                pattern: Regex::new(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b").unwrap(),
                replacement: "<uuid>",
            },
            ScrubberRule {
                pattern: Regex::new(r"(?i)\b[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}\b").unwrap(),
                replacement: "<email>",
            },
            ScrubberRule {
                pattern: Regex::new(r#"(?i)\b(password|passwd|token|auth[_-]?token|secret|private[_-]?key|preshared[_-]?key|psk|key|cookie|license|uuid)(["']?\s*[:=]\s*["']?)[^\s"',;&]+"#).unwrap(),
                replacement: "${1}${2}<redacted>",
            },
            ScrubberRule {
                pattern: Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").unwrap(),
                replacement: "<ipv4>",
            },
            ScrubberRule {
                pattern: Regex::new(r"(?i)\b(?:[0-9a-f]{2}[:-]){5}[0-9a-f]{2}\b").unwrap(),
                replacement: "<mac>",
            },
        ]
    })
}

fn is_word_byte(c: u8) -> bool {
    c == b'_' || c.is_ascii_digit() || c.is_ascii_alphabetic()
}

fn trim_single_colons(mut s: &str) -> &str {
    if s.starts_with(':') && !s.starts_with("::") {
        s = &s[1..];
    }
    if s.ends_with(':') && !s.ends_with("::") {
        s = &s[..s.len() - 1];
    }
    s
}

fn is_valid_ipv6_with_optional_zone(s: &str) -> bool {
    let addr_part = match s.split_once('%') {
        Some((addr, zone)) => {
            if zone.is_empty() {
                return false;
            }
            addr
        }
        None => s,
    };
    addr_part.parse::<Ipv6Addr>().is_ok()
}

fn scrub_ipv6(text: &str) -> String {
    static IPV6_CANDIDATE: OnceLock<Regex> = OnceLock::new();
    let re = IPV6_CANDIDATE.get_or_init(|| {
        Regex::new(r"(?i)[0-9a-f:]*:[0-9a-f:]*:[0-9a-f:]*(?:%[0-9a-z._-]+)?").unwrap()
    });

    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;

    for m in re.find_iter(text) {
        let (start, end) = (m.start(), m.end());
        if (start > 0 && is_word_byte(bytes[start - 1]))
            || (end < bytes.len() && is_word_byte(bytes[end]))
        {
            continue;
        }
        let candidate = &text[start..end];
        for trimmed in [candidate, trim_single_colons(candidate)] {
            if is_valid_ipv6_with_optional_zone(trimmed) {
                out.push_str(&text[last..start]);
                out.push_str(&candidate.replacen(trimmed, "<ipv6>", 1));
                last = end;
                break;
            }
        }
    }
    out.push_str(&text[last..]);
    out
}

fn generic_tlds() -> &'static HashSet<&'static str> {
    static TLDS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    TLDS.get_or_init(|| {
        [
            "ir", "com", "net", "org", "info", "biz", "edu", "gov", "mil", "int", "xyz", "top",
            "online", "site", "cloud", "club", "shop", "store", "tech", "space", "website", "icu",
            "vip", "pro", "live", "link", "click", "fun", "life", "world", "today", "news", "blog",
            "host", "network", "digital", "media", "work", "cyou", "buzz", "sbs", "lol", "monster",
            "rest", "best", "asia", "mobi", "name", "email", "lat", "uno", "run", "hair", "beauty",
            "quest", "skin",
        ]
        .into_iter()
        .collect()
    })
}

fn file_extension_tlds() -> &'static HashSet<&'static str> {
    static EXT: OnceLock<HashSet<&'static str>> = OnceLock::new();
    EXT.get_or_init(|| {
        [
            "so", "sh", "py", "rs", "md", "pl", "cc", "mm", "js", "ts", "go", "kt", "gz", "xz",
            "db", "rb", "cs", "hh",
        ]
        .into_iter()
        .collect()
    })
}

fn scrub_domains(text: &str) -> String {
    static DOMAIN_CANDIDATE: OnceLock<Regex> = OnceLock::new();
    let re = DOMAIN_CANDIDATE.get_or_init(|| {
        Regex::new(r"(?i)\b(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+([a-z]{2,24})\b\.?").unwrap()
    });

    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;

    for caps in re.captures_iter(text) {
        let Some(full) = caps.get(0) else { continue };
        let Some(tld_match) = caps.get(1) else { continue };
        let (start, end) = (full.start(), full.end());
        let tld_raw = tld_match.as_str();
        let tld_lower = tld_raw.to_ascii_lowercase();

        if !generic_tlds().contains(tld_lower.as_str())
            && (tld_lower.len() != 2 || file_extension_tlds().contains(tld_lower.as_str()))
        {
            continue;
        }
        if (start > 0 && matches!(bytes[start - 1], b'/' | b'\\' | b'<' | b'@'))
            || (end < bytes.len() && bytes[end] == b'/')
        {
            continue;
        }
        out.push_str(&text[last..start]);
        out.push_str("<domain>.");
        out.push_str(tld_raw);
        if bytes[end - 1] == b'.' {
            out.push('.');
        }
        last = end;
    }
    out.push_str(&text[last..]);
    out
}

/// Removes proxy links, URL paths/queries, UUIDs, emails, inline secrets, IPv4/IPv6 addresses,
/// MAC addresses, and domain names from report text, matching `libbox.sanitizeReportText`.
pub fn sanitize_report_text(content: &[u8]) -> Vec<u8> {
    let mut text = String::from_utf8_lossy(content).into_owned();
    for rule in report_scrubbers() {
        text = rule.pattern.replace_all(&text, rule.replacement).into_owned();
    }
    scrub_domains(&scrub_ipv6(&text)).into_bytes()
}

// ============================================================================
// 2. Crash Report Scanner (`diagnostics.go:listCrashReports`)
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreCrashReport {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub source: String,
    #[serde(rename = "crashedAt", alias = "crashed_at", default)]
    pub crashed_at: String,
    #[serde(rename = "isCrash", alias = "is_crash", default)]
    pub is_crash: bool,
}

fn live_crash_files() -> Vec<String> {
    let mut files: Vec<String> = SETUP_MODE_NAMES
        .iter()
        .map(|mode| format!("data/stderr{mode}.log"))
        .collect();
    for extra in [
        "stderr.log",
        "stderr2.log",
        "CrashReport-.log",
        "network_extension_error.log",
    ] {
        files.push(extra.to_string());
    }
    files
}

fn crash_marker_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?m)^(panic: |fatal error: |runtime: |SIG[A-Z]+: |\[signal )|goroutine \d+ \[")
            .unwrap()
    })
}

/// Formats Unix seconds as UTC RFC 3339 (`YYYY-MM-DDTHH:MM:SSZ`).
pub fn format_utc_rfc3339_secs(unix_secs: i64) -> String {
    let stamp = format_utc_millis_stamp(unix_secs.saturating_mul(1000));
    // `stamp` is `YYYY-MM-DDTHH-MM-SS-mmm`
    let date = &stamp[..10];
    let time = stamp[11..19].replace('-', ":");
    format!("{date}T{time}Z")
}

/// Formats Unix seconds as UTC directory stamp (`YYYY-MM-DDTHH-MM-SS`).
pub fn format_utc_dir_stamp(unix_secs: i64) -> String {
    let stamp = format_utc_millis_stamp(unix_secs.saturating_mul(1000));
    stamp[..19].to_string()
}

/// Parses `2006-01-02T15-04-05[-n]` into `2006-01-02T15:04:05Z`, returning `""` on malformed input.
pub fn report_time(name: &str) -> String {
    if name.len() < 19 || !name.is_char_boundary(19) {
        return String::new();
    }
    let head = &name.as_bytes()[..19];
    // Validate exact `YYYY-MM-DDTHH-MM-SS` ASCII layout
    let valid_digits = [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18]
        .into_iter()
        .all(|i| head[i].is_ascii_digit());
    let valid_seps = head[4] == b'-'
        && head[7] == b'-'
        && head[10] == b'T'
        && head[13] == b'-'
        && head[16] == b'-';
    if !valid_digits || !valid_seps {
        return String::new();
    }
    let month: u32 = name[5..7].parse().unwrap_or(0);
    let day: u32 = name[8..10].parse().unwrap_or(0);
    let hour: u32 = name[11..13].parse().unwrap_or(0);
    let min: u32 = name[14..16].parse().unwrap_or(0);
    let sec: u32 = name[17..19].parse().unwrap_or(0);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || min > 59
        || sec > 59
    {
        return String::new();
    }
    format!("{}T{}Z", &name[..10], name[11..19].replace('-', ":"))
}

/// Scans `crash_reports`, `oom_reports`, and live `stderr` files in `working_dir`,
/// matching `hcore.listCrashReports`.
pub fn list_core_crash_reports(working_dir: &Path) -> Vec<CoreCrashReport> {
    let mut reports = Vec::new();
    let marker = crash_marker_re();

    for &directory in &REPORT_DIRECTORIES[..2] {
        let dir_path = working_dir.join(directory);
        let Ok(entries) = fs::read_dir(&dir_path) else {
            continue;
        };
        let mut dir_names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        dir_names.sort();

        for entry_name in dir_names {
            let crashed_at = report_time(&entry_name);
            let go_log = fs::read(dir_path.join(&entry_name).join("go.log")).unwrap_or_default();
            let text = String::from_utf8_lossy(&go_log);
            let is_crash = directory == "oom_reports" || marker.is_match(&text);
            reports.push(CoreCrashReport {
                name: entry_name,
                source: directory.to_string(),
                crashed_at,
                is_crash,
            });
        }
    }

    for rel_name in live_crash_files() {
        let path = working_dir.join(&rel_name);
        let Ok(content) = fs::read(&path) else {
            continue;
        };
        let text = String::from_utf8_lossy(&content);
        if text.trim().is_empty() {
            continue;
        }
        let crashed_at = fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| format_utc_rfc3339_secs(d.as_secs() as i64))
            .unwrap_or_default();
        reports.push(CoreCrashReport {
            is_crash: marker.is_match(&text),
            name: rel_name.clone(),
            source: rel_name,
            crashed_at,
        });
    }

    reports.sort_by(|a, b| a.crashed_at.cmp(&b.crashed_at));
    reports
}

// ============================================================================
// 3. Scrubbed ZIP Archive + Age Encryptor (`diagnostics.go` & `diagnostics_encryption.go`)
// ============================================================================

fn collect_walk_entries(root: &Path, current: &Path, out: &mut Vec<(PathBuf, bool)>) -> std::io::Result<()> {
    let mut entries: Vec<_> = fs::read_dir(current)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let ft = entry.file_type()?;
        if ft.is_dir() {
            out.push((path.clone(), true));
            collect_walk_entries(root, &path, out)?;
        } else if ft.is_file() {
            out.push((path, false));
        }
    }
    Ok(())
}

/// Zips `source_dir` into `destination_path`, scrubbing `.log`/`.txt`/`.json` files and omitting
/// `configuration.json` / `*.connections.json`, matching `libbox.CreateZipArchive`.
pub fn create_scrubbed_zip_archive(source_dir: &Path, destination_path: &Path) -> Result<(), String> {
    let meta = fs::metadata(source_dir).map_err(|e| e.to_string())?;
    if !meta.is_dir() {
        return Err("source path is not a directory".to_string());
    }
    if let Some(parent) = destination_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }

    let root_name = source_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "diagnostics".to_string());

    let mut entries = Vec::new();
    collect_walk_entries(source_dir, source_dir, &mut entries).map_err(|e| e.to_string())?;

    let dest_file = File::create(destination_path).map_err(|e| e.to_string())?;
    let mut zip_writer = ZipWriter::new(dest_file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    for (path, is_dir) in entries {
        let rel = path
            .strip_prefix(source_dir)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        if rel.is_empty() || rel == "." {
            continue;
        }
        let archive_path = format!("{root_name}/{rel}");
        if is_dir {
            zip_writer
                .add_directory(format!("{archive_path}/"), options)
                .map_err(|e| e.to_string())?;
            continue;
        }
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if sensitive_report_file(&file_name) {
            continue;
        }

        zip_writer
            .start_file(&archive_path, options)
            .map_err(|e| e.to_string())?;

        if text_report_file(&file_name) {
            let raw = fs::read(&path).map_err(|e| e.to_string())?;
            let scrubbed = sanitize_report_text(&raw);
            zip_writer.write_all(&scrubbed).map_err(|e| e.to_string())?;
        } else {
            let mut src = File::open(&path).map_err(|e| e.to_string())?;
            std::io::copy(&mut src, &mut zip_writer).map_err(|e| e.to_string())?;
        }
    }

    let mut out_file = zip_writer.finish().map_err(|e| e.to_string())?;
    out_file.flush().map_err(|e| e.to_string())?;
    Ok(())
}

fn encrypt_file_to_recipient(
    source: &Path,
    destination: &Path,
    recipient: &Recipient,
) -> Result<(), String> {
    let mut input = File::open(source).map_err(|e| e.to_string())?;
    let output = File::create(destination).map_err(|e| e.to_string())?;
    let encryptor = age::Encryptor::with_recipients(std::iter::once(recipient as &dyn age::Recipient))
        .map_err(|e| e.to_string())?;
    let mut writer = encryptor.wrap_output(output).map_err(|e| e.to_string())?;
    std::io::copy(&mut input, &mut writer).map_err(|e| e.to_string())?;
    let mut out_file = writer.finish().map_err(|e| e.to_string())?;
    out_file.flush().map_err(|e| e.to_string())?;
    Ok(())
}

fn encrypt_directory(
    directory: &Path,
    destination: &Path,
    recipient: &Recipient,
) -> Result<(), String> {
    let mut plain_os = destination.as_os_str().to_os_string();
    plain_os.push(".zip");
    let plain = PathBuf::from(plain_os);

    fs::create_dir_all(directory)
        .map_err(|e| format!("create the sensitive directory: {e}"))?;
    if let Err(e) = create_scrubbed_zip_archive(directory, &plain) {
        let _ = fs::remove_file(&plain);
        return Err(format!("zip the sensitive files: {e}"));
    }
    if let Err(e) = encrypt_file_to_recipient(&plain, destination, recipient) {
        let _ = fs::remove_file(&plain);
        let _ = fs::remove_file(destination);
        return Err(format!("encrypt the sensitive files: {e}"));
    }
    let _ = fs::remove_file(&plain);
    fs::remove_dir_all(directory).map_err(|e| e.to_string())
}

fn list_relative_files(directory: &Path) -> Vec<String> {
    let mut entries = Vec::new();
    if collect_walk_entries(directory, directory, &mut entries).is_err() {
        return Vec::new();
    }
    let mut files: Vec<String> = entries
        .into_iter()
        .filter_map(|(path, is_dir)| {
            if is_dir {
                return None;
            }
            path.strip_prefix(directory)
                .ok()
                .map(|r| r.to_string_lossy().replace('\\', "/"))
        })
        .collect();
    files.sort();
    files
}

pub fn report_readme(sensitive_files: &[String], encrypted: bool) -> String {
    let mut list = String::new();
    for name in sensitive_files {
        list.push_str("    ");
        list.push_str(name);
        list.push('\n');
    }

    let mut out = String::from(
        "Hiddify diagnostics report\n\
==========================\n\
\n\
metadata.json\n\
    Not encrypted. Generic information anyone with the report can read: the app version and\n\
    build, the system, the connection state, the settings without passwords or keys, and the\n\
    list of crash reports. No config, server, address or log.\n\
\n\
goroutines/, crash_reports/ and app/\n\
    Not encrypted. Stack traces: the previous run's goroutines, the core's crashes and the app's\n\
    own exceptions. They show which code ran, not your config or servers.\n\
\n",
    );

    if encrypted {
        out.push_str(&format!(
            "{ENCRYPTED_SENSITIVE_FILE}\n\
    Encrypted on the device, to the Hiddify team's key, in the age format\n\
    (https://age-encryption.org). Only the Hiddify team can open it. It is a zip of:\n\
\n\
{list}    The Hiddify team opens it with:\n\
        age -d -i hiddify-diagnostics.key {ENCRYPTED_SENSITIVE_FILE} > sensitive.zip\n\
\n\
    To look inside it yourself, save the report again in the app: Report a problem,\n\
    Save the file, Unencrypted.\n"
        ));
    } else {
        out.push_str(&format!(
            "{SENSITIVE_DIRECTORY}/\n\
    NOT encrypted. The config with servers and keys removed, and the logs:\n\
\n\
{list}    Addresses, links and secrets are removed from these files, but they can still tell which\n\
    apps and sites you used. Send this file only to people you trust.\n"
        ));
    }
    out
}

pub fn app_relative_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let parts: Vec<&str> = normalized
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    for (i, part) in parts.iter().enumerate() {
        if part.to_ascii_lowercase().contains("hiddify") {
            return parts[i..].join("/");
        }
    }
    parts.last().copied().unwrap_or("").to_string()
}

fn copy_log_tail(source: &Path, destination: &Path) {
    let Ok(mut file) = File::open(source) else {
        return;
    };
    let Ok(meta) = file.metadata() else {
        return;
    };
    let size = meta.len();
    if size == 0 {
        return;
    }
    if size > MAX_DIAGNOSTICS_LOG_SIZE {
        let _ = file.seek(SeekFrom::Start(size - MAX_DIAGNOSTICS_LOG_SIZE));
    }
    if let Some(parent) = destination.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let Ok(mut target) = File::create(destination) else {
        return;
    };
    if size > MAX_DIAGNOSTICS_LOG_SIZE {
        let _ = writeln!(
            target,
            "... the last {} bytes of {} ...",
            MAX_DIAGNOSTICS_LOG_SIZE, size
        );
    }
    let _ = std::io::copy(&mut file, &mut target);
}

fn copy_directory_tails(source: &Path, destination: &Path) {
    let mut entries = Vec::new();
    if collect_walk_entries(source, source, &mut entries).is_err() {
        return;
    }
    for (path, is_dir) in entries {
        let Ok(rel) = path.strip_prefix(source) else {
            continue;
        };
        let target = destination.join(rel);
        if is_dir {
            let _ = fs::create_dir_all(&target);
        } else {
            copy_log_tail(&path, &target);
        }
    }
}

fn copy_newest_reports(source: &Path, destination: &Path) {
    let Ok(entries) = fs::read_dir(source) else {
        return;
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let start = names.len().saturating_sub(MAX_DIAGNOSTICS_REPORTS);
    for name in &names[start..] {
        copy_directory_tails(&source.join(name), &destination.join(name));
    }
}

fn write_app_files(directory: &Path, files: &BTreeMap<String, Vec<u8>>) {
    if files.is_empty() {
        return;
    }
    let _ = fs::create_dir_all(directory);
    for (name, content) in files {
        let base = Path::new(name)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| name.clone());
        if !base.is_empty() {
            let _ = fs::write(directory.join(base), content);
        }
    }
}

fn add_app_summary(metadata: &mut Map<String, Value>, app_info: Option<&[u8]>) {
    let Some(raw) = app_info else {
        return;
    };
    if raw.is_empty() {
        return;
    }
    let Ok(Value::Object(info)) = serde_json::from_slice::<Value>(raw) else {
        return;
    };
    for &section in APP_SUMMARY_SECTIONS {
        if !metadata.contains_key(section) {
            if let Some(val) = info.get(section) {
                metadata.insert(section.to_string(), val.clone());
            }
        }
    }
}

pub fn report_number(working_dir: &Path, next: bool) -> u32 {
    let path = working_dir.join(REPORT_NUMBER_FILE);
    let mut last: u32 = fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(0);
    if !next && last > 0 {
        return last;
    }
    last = last.saturating_add(1);
    let _ = fs::write(&path, last.to_string());
    last
}

pub fn report_name(working_dir: &Path, stamp: &str, inline: bool, copy_of_last: bool) -> String {
    if inline {
        return format!("hiddify-{stamp}.zip");
    }
    let num = report_number(working_dir, !copy_of_last);
    if copy_of_last {
        format!("hiddify-{num:03}-unencrypted.zip")
    } else {
        format!("hiddify-{num:03}.zip")
    }
}

fn parse_recipient(public_key_override: Option<&str>) -> Result<Option<Recipient>, String> {
    let raw = match public_key_override {
        Some(k) => k.trim(),
        None => option_env!("DIAGNOSTICS_PUBLIC_KEY").unwrap_or("").trim(),
    };
    if raw.is_empty() {
        return Ok(None);
    }
    let recipient = raw
        .parse::<Recipient>()
        .map_err(|e| format!("the diagnostics public key of this build is not an age key: {e}"))?;
    Ok(Some(recipient))
}

#[derive(Debug, Clone)]
pub struct ExportDiagnosticsOptions<'a> {
    pub working_dir: &'a Path,
    pub temp_dir: &'a Path,
    pub destination_path: Option<&'a Path>,
    pub unencrypted: bool,
    pub inline: bool,
    pub public_key_override: Option<&'a str>,
    pub extra_files: BTreeMap<String, Vec<u8>>,
    pub generic_files: BTreeMap<String, Vec<u8>>,
    pub mode: Option<&'a str>,
    pub core_version: Option<&'a str>,
    pub core_commit: Option<&'a str>,
    pub core_state: Option<&'a str>,
}

impl Default for ExportDiagnosticsOptions<'_> {
    fn default() -> Self {
        Self {
            working_dir: Path::new(""),
            temp_dir: Path::new(""),
            destination_path: None,
            unencrypted: false,
            inline: false,
            public_key_override: None,
            extra_files: BTreeMap::new(),
            generic_files: BTreeMap::new(),
            mode: None,
            core_version: None,
            core_commit: None,
            core_state: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportDiagnosticsOutcome {
    pub path: String,
    pub encrypted: bool,
}

struct TempStagingGuard(PathBuf);
impl Drop for TempStagingGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Assembles, scrubs, optionally X25519 `age`-encrypts, and zips a complete Hiddify diagnostics report,
/// matching `hcore.exportDiagnostics`.
pub fn export_diagnostics_report(
    opts: &ExportDiagnosticsOptions<'_>,
) -> Result<ExportDiagnosticsOutcome, String> {
    let recipient = parse_recipient(opts.public_key_override)?;
    let encrypt = recipient.is_some() && !opts.unencrypted;

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let stamp = format_utc_dir_stamp(now_secs);
    let staging = opts.temp_dir.join(format!("diagnostics-{stamp}"));
    fs::create_dir_all(&staging)
        .map_err(|e| format!("create the diagnostics directory: {e}"))?;
    let _staging_guard = TempStagingGuard(staging.clone());

    // 1. Generic files (unencrypted)
    copy_newest_reports(
        &opts.working_dir.join("crash_reports"),
        &staging.join("crash_reports"),
    );
    for mode in SETUP_MODE_NAMES {
        let prev_name = format!("goroutines-{}.previous.txt", mode.to_ascii_lowercase());
        copy_log_tail(
            &opts.working_dir.join("data").join(&prev_name),
            &staging.join("goroutines").join(&prev_name),
        );
    }
    write_app_files(&staging.join("app"), &opts.generic_files);

    // 2. Sensitive files
    let sensitive = staging.join(SENSITIVE_DIRECTORY);
    for reports_kind in ["oom_reports", "power_reports"] {
        copy_newest_reports(
            &opts.working_dir.join(reports_kind),
            &sensitive.join(reports_kind),
        );
    }
    for name in live_crash_files() {
        copy_log_tail(
            &opts.working_dir.join(&name),
            &sensitive.join("logs").join(name.replace('/', "-")),
        );
    }
    copy_log_tail(
        &opts.working_dir.join("data").join("box.log"),
        &sensitive.join("logs").join("box.log"),
    );
    let _ = fs::create_dir_all(&sensitive);
    let net_json = format!(
        "{}\n",
        serde_json::to_string_pretty(&json!({
            "coreRunning": matches!(opts.core_state, Some("STARTED" | "WAITING" | "HOT_RELOADING" | "NO_AVAILABLE_PROXIES")),
        }))
        .unwrap_or_else(|_| "{}".to_string())
    );
    let _ = fs::write(sensitive.join("network.json"), net_json);

    if let Ok(raw_cfg) = fs::read_to_string(opts.working_dir.join("data").join("current-config.json"))
    {
        if let Some(sanitized) = sanitize_config_json(&raw_cfg) {
            let _ = fs::write(sensitive.join("config.json"), sanitized);
        }
    }
    write_app_files(&sensitive.join("app"), &opts.extra_files);

    let sensitive_files = list_relative_files(&sensitive);
    if encrypt {
        if let Some(ref rec) = recipient {
            encrypt_directory(&sensitive, &staging.join(ENCRYPTED_SENSITIVE_FILE), rec)?;
        }
    }

    // 3. Metadata & README
    let reports = list_core_crash_reports(opts.working_dir);
    let mut metadata = Map::new();
    metadata.insert(
        "exportedAt".to_string(),
        Value::String(format_utc_rfc3339_secs(now_secs)),
    );
    metadata.insert(
        "mode".to_string(),
        Value::String(opts.mode.unwrap_or("RUST_LOCAL").to_string()),
    );
    metadata.insert(
        "os".to_string(),
        Value::String(std::env::consts::OS.to_string()),
    );
    metadata.insert(
        "arch".to_string(),
        Value::String(std::env::consts::ARCH.to_string()),
    );
    metadata.insert(
        "coreVersion".to_string(),
        Value::String(opts.core_version.unwrap_or("").to_string()),
    );
    metadata.insert(
        "coreCommit".to_string(),
        Value::String(opts.core_commit.unwrap_or("").to_string()),
    );
    metadata.insert(
        "coreState".to_string(),
        Value::String(opts.core_state.unwrap_or("STOPPED").to_string()),
    );
    metadata.insert(
        "reports".to_string(),
        serde_json::to_value(&reports).unwrap_or(Value::Null),
    );
    metadata.insert(
        "workingPath".to_string(),
        Value::String(app_relative_path(&opts.working_dir.to_string_lossy())),
    );
    metadata.insert("encrypted".to_string(), Value::Bool(encrypt));
    add_app_summary(
        &mut metadata,
        opts.extra_files.get("app-info.json").map(Vec::as_slice),
    );

    let mut metadata_json =
        serde_json::to_string_pretty(&Value::Object(metadata)).unwrap_or_else(|_| "{}".to_string());
    metadata_json.push('\n');
    let _ = fs::write(staging.join("metadata.json"), metadata_json);
    let _ = fs::write(
        staging.join("README.txt"),
        report_readme(&sensitive_files, encrypt),
    );

    // 4. Final scrubbed ZIP archive
    let destination = match opts.destination_path {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => {
            let copy_of_last = opts.unencrypted && recipient.is_some();
            opts.temp_dir.join(report_name(
                opts.working_dir,
                &stamp,
                opts.inline,
                copy_of_last,
            ))
        }
    };
    create_scrubbed_zip_archive(&staging, &destination)
        .map_err(|e| format!("zip the diagnostics: {e}"))?;

    Ok(ExportDiagnosticsOutcome {
        path: destination.to_string_lossy().into_owned(),
        encrypted: encrypt,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read};
    use zip::ZipArchive;

    fn write_test_file(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn read_zip_entries(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
        let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut map = BTreeMap::new();
        for i in 0..archive.len() {
            let mut file = archive.by_index(i).unwrap();
            if file.name().ends_with('/') {
                continue;
            }
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).unwrap();
            let stripped = file
                .name()
                .split_once('/')
                .map(|(_, rest)| rest.to_string())
                .unwrap_or_else(|| file.name().to_string());
            map.insert(stripped, buf);
        }
        map
    }

    #[test]
    fn sanitize_report_text_matches_all_libbox_vectors() {
        let time_and_ip_cases = [
            (
                "[2026-11-09 10:01:22 +0000] (packet-tunnel) starting",
                "[2026-11-09 10:01:22 +0000] (packet-tunnel) starting",
            ),
            (
                "10:01:22.534989 [DEBUG] dns: exchange",
                "10:01:22.534989 [DEBUG] dns: exchange",
            ),
            (
                "+0330 2026-11-09 13:32:04 INFO[0004] started",
                "+0330 2026-11-09 13:32:04 INFO[0004] started",
            ),
            ("connect to 2001:db8::1 failed", "connect to <ipv6> failed"),
            (
                "connect to [2001:db8:0:0:1:2:3:4]:443",
                "connect to [<ipv6>]:443",
            ),
            ("from fe80::1%en0: refused", "from <ipv6>: refused"),
            (
                "lookup succeed: 2606:4700:4700::1111 2606:4700:4700::1001",
                "lookup succeed: <ipv6> <ipv6>",
            ),
            ("std::vector, Foo::Bar", "std::vector, Foo::Bar"),
            ("dial ::1 and ::", "dial <ipv6> and <ipv6>"),
            (
                "WIFI state changed: SSID=home, BSSID=a4:2b:b0:1c:9e:7f",
                "WIFI state changed: SSID=home, BSSID=<mac>",
            ),
            (
                "inbound connection from 192.0.2.4:52862 to 198.51.100.7:443",
                "inbound connection from <ipv4>:52862 to <ipv4>:443",
            ),
        ];
        for (input, expected) in time_and_ip_cases {
            let got = String::from_utf8(sanitize_report_text(input.as_bytes())).unwrap();
            assert_eq!(got, expected, "input: {input}");
        }

        let domain_cases = [
            (
                "connection download closed: remote error: resolve error: lookup ire-oneid.shalltry.com on 10.0.0.1:53: no such host",
                "connection download closed: remote error: resolve error: lookup <domain>.com on <ipv4>:53: no such host",
            ),
            (
                "dns: exchanged A www.digikala.ir. 60 IN A 192.0.2.1",
                "dns: exchanged A <domain>.ir. 60 IN A <ipv4>",
            ),
            (
                "outbound/vless[proxy]: outbound connection to Example.CO.uk:443",
                "outbound/vless[proxy]: outbound connection to <domain>.uk:443",
            ),
            (
                "sniffed domain=api.telegram.org protocol=tls",
                "sniffed domain=<domain>.org protocol=tls",
            ),
            (
                "open configuration.json and libbox.so failed",
                "open configuration.json and libbox.so failed",
            ),
            (
                "github.com/sagernet/sing-box/route.(*Router).Start",
                "github.com/sagernet/sing-box/route.(*Router).Start",
            ),
            (
                "/Applications/Hiddify.app/Contents/MacOS/Hiddify",
                "/Applications/Hiddify.app/Contents/MacOS/Hiddify",
            ),
            (
                "#0 ref.read (package:hiddify/main.dart:12)",
                "#0 ref.read (package:hiddify/main.dart:12)",
            ),
            ("version 5.9.2 build 1.2.3", "version 5.9.2 build 1.2.3"),
            ("send to bug@hiddify.com", "send to <email>"),
            ("fetch https://sub.example.com/x?token=1", "fetch https://<host>"),
        ];
        for (input, expected) in domain_cases {
            let got = String::from_utf8(sanitize_report_text(input.as_bytes())).unwrap();
            assert_eq!(got, expected, "input: {input}");
        }
    }

    #[test]
    fn list_crash_reports_and_export_with_age_encryption_match_go_tests() {
        let working = tempfile::tempdir().unwrap();
        let temp = tempfile::tempdir().unwrap();

        write_test_file(
            &working.path().join("crash_reports/2026-10-30T10-00-00/go.log"),
            "panic: boom at 1.2.3.4\n\ngoroutine 1 [running]:\nmain.main()\n",
        );
        write_test_file(
            &working.path().join("crash_reports/2026-10-31T10-00-00/go.log"),
            "some library warning\n",
        );
        write_test_file(
            &working.path().join("oom_reports/2026-10-29T10-00-00/metadata.json"),
            "{}",
        );
        write_test_file(
            &working.path().join("data/stderrGRPC_BACKGROUND.log"),
            "fatal error: concurrent map writes\n",
        );
        write_test_file(&working.path().join("data/stderrGRPC_NORMAL.log"), "   \n");

        let reports = list_core_crash_reports(working.path());
        let by_name: BTreeMap<String, bool> =
            reports.iter().map(|r| (r.name.clone(), r.is_crash)).collect();
        assert_eq!(by_name.get("2026-10-29T10-00-00"), Some(&true));
        assert_eq!(by_name.get("2026-10-30T10-00-00"), Some(&true));
        assert_eq!(by_name.get("2026-10-31T10-00-00"), Some(&false));
        assert_eq!(by_name.get("data/stderrGRPC_BACKGROUND.log"), Some(&true));
        assert!(!by_name.contains_key("data/stderrGRPC_NORMAL.log"));
        assert_eq!(reports[0].crashed_at, "2026-10-29T10:00:00Z");
        assert_eq!(reports[0].source, "oom_reports");

        // Prepare sensitive config + goroutine + logs and verify age X25519 encryption roundtrip
        write_test_file(
            &working.path().join("data/box.log"),
            "INFO a secret log line from 5.6.7.8\n",
        );
        write_test_file(
            &working.path().join("data/goroutines-grpc_normal.previous.txt"),
            "goroutine 1 [running]:\nmain.main()\n",
        );
        write_test_file(
            &working.path().join("data/current-config.json"),
            r#"{"outbounds":[{"type":"vless","tag":"p","server":"my.server.example","server_port":443,"uuid":"0a1b2c3d-1111-2222-3333-444455556666"}]}"#,
        );

        let identity = age::x25519::Identity::generate();
        let pub_key = identity.to_public().to_string();

        let mut extra = BTreeMap::new();
        let app_info = br#"{"app":{"version":"5.9.3"},"settings":{"mode":"tun","window_size":"<set>"}}"#.to_vec();
        extra.insert("app-info.json".to_string(), app_info.clone());
        extra.insert(
            "app-logs.txt".to_string(),
            b"app log line password=hunter2\n".to_vec(),
        );
        let mut generic = BTreeMap::new();
        let app_crash = b"StateError: boom\n#0 main (package:hiddify/main.dart:10)\n".to_vec();
        generic.insert("app-crash-1.txt".to_string(), app_crash.clone());

        let outcome = export_diagnostics_report(&ExportDiagnosticsOptions {
            working_dir: working.path(),
            temp_dir: temp.path(),
            unencrypted: false,
            inline: false,
            public_key_override: Some(&pub_key),
            extra_files: extra,
            generic_files: generic,
            mode: Some("GRPC_NORMAL"),
            core_version: Some("5.9.3"),
            core_commit: Some("abc1234"),
            core_state: Some("STOPPED"),
            destination_path: None,
        })
        .unwrap();

        assert!(outcome.encrypted);
        assert!(outcome.path.ends_with("hiddify-001.zip"));
        // Staging dir must be removed, leaving only hiddify-001.zip
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);

        let outer_bytes = fs::read(&outcome.path).unwrap();
        let outer_files = read_zip_entries(&outer_bytes);
        assert!(outer_files.contains_key("metadata.json"));
        assert!(outer_files.contains_key("README.txt"));
        assert!(outer_files.contains_key("sensitive.encrypted.age"));
        assert!(outer_files.contains_key("goroutines/goroutines-grpc_normal.previous.txt"));
        assert_eq!(outer_files.get("app/app-crash-1.txt"), Some(&app_crash));
        assert!(!String::from_utf8_lossy(&outer_bytes).contains("secret log line"));

        let meta_str = String::from_utf8(outer_files["metadata.json"].clone()).unwrap();
        assert!(meta_str.contains(r#""version": "5.9.3""#));
        assert!(meta_str.contains(r#""encrypted": true"#));
        assert!(meta_str.contains(r#""window_size": "<set>""#));

        let readme_str = String::from_utf8(outer_files["README.txt"].clone()).unwrap();
        assert!(readme_str.contains("    logs/box.log\n"));

        // Decrypt sensitive.encrypted.age with the X25519 identity
        let decryptor = age::Decryptor::new(Cursor::new(&outer_files["sensitive.encrypted.age"])).unwrap();
        let mut reader = decryptor
            .decrypt(std::iter::once(&identity as &dyn age::Identity))
            .unwrap();
        let mut inner_zip_bytes = Vec::new();
        reader.read_to_end(&mut inner_zip_bytes).unwrap();
        let inner_files = read_zip_entries(&inner_zip_bytes);

        let box_log = String::from_utf8(inner_files["logs/box.log"].clone()).unwrap();
        assert!(box_log.contains("secret log line from <ipv4>"));
        let app_logs = String::from_utf8(inner_files["app/app-logs.txt"].clone()).unwrap();
        assert!(!app_logs.contains("hunter2"));
        assert!(app_logs.contains("password=<redacted>"));
        let cfg_str = String::from_utf8(inner_files["config.json"].clone()).unwrap();
        assert!(!cfg_str.contains("my.server.example"));
        assert!(!cfg_str.contains("0a1b2c3d"));
        assert!(cfg_str.contains(r#""server_port": 443"#));

        // Unencrypted copy keeps report number 001 (`hiddify-001-unencrypted.zip`)
        let clear = export_diagnostics_report(&ExportDiagnosticsOptions {
            working_dir: working.path(),
            temp_dir: temp.path(),
            unencrypted: true,
            inline: false,
            public_key_override: Some(&pub_key),
            ..Default::default()
        })
        .unwrap();
        assert!(!clear.encrypted);
        assert!(clear.path.ends_with("hiddify-001-unencrypted.zip"));

        // Invalid non-age key must refuse export
        let bad_key = export_diagnostics_report(&ExportDiagnosticsOptions {
            working_dir: working.path(),
            temp_dir: temp.path(),
            public_key_override: Some("not-an-age-key"),
            ..Default::default()
        });
        assert!(bad_key.is_err());
    }

    #[test]
    fn app_relative_path_and_report_numbering_match_go() {
        assert_eq!(
            app_relative_path("/Users/someone/Library/Application Support/apple.hiddify.com"),
            "apple.hiddify.com"
        );
        assert_eq!(
            app_relative_path("/data/user/0/app.hiddify.com/files"),
            "app.hiddify.com/files"
        );
        assert_eq!(
            app_relative_path("/home/someone/apps/Hiddify/portable_data"),
            "Hiddify/portable_data"
        );
        assert_eq!(app_relative_path("/home/someone/data"), "data");

        let dir = tempfile::tempdir().unwrap();
        assert_eq!(report_name(dir.path(), "stamp", false, false), "hiddify-001.zip");
        assert_eq!(
            report_name(dir.path(), "stamp", false, true),
            "hiddify-001-unencrypted.zip"
        );
        assert_eq!(report_name(dir.path(), "stamp", false, false), "hiddify-002.zip");
        assert_eq!(report_name(dir.path(), "stamp", true, false), "hiddify-stamp.zip");
        assert_eq!(report_name(dir.path(), "stamp", false, false), "hiddify-003.zip");
    }
}
