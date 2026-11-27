//! Independent diagnostic, crash excerpt, release changelog, OS version/distro detection,
//! Telegram report payload encoding, and bounded ring-buffer log utilities.

use std::collections::{HashMap, VecDeque};
use std::sync::OnceLock;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use regex::Regex;
use serde::{Deserialize, Serialize};
use url::Url;

pub mod core_guard;
pub mod log_crash_store;
pub mod report_export;

/// Maximum Telegram bot `start` parameter length.
pub const MAX_TELEGRAM_PAYLOAD_LEN: usize = 64;

/// Default Telegram issues bot username.
pub const DEFAULT_TELEGRAM_BOT: &str = "hiddify_issues_bot";

/// Maximum crash excerpt character count (UTF-16 code units, matching Dart `String.length`).
pub const MAX_CRASH_EXCERPT_UTF16_LEN: usize = 3000;

/// Maximum crash excerpt line count.
pub const MAX_CRASH_EXCERPT_LINES: usize = 60;

/// Maximum crash headline character count (UTF-16 code units).
pub const MAX_CRASH_HEADLINE_UTF16_LEN: usize = 200;

/// Maximum GitHub release changelog length (UTF-16 code units).
pub const MAX_CHANGELOG_UTF16_LEN: usize = 4000;

/// Input fields for building a Telegram diagnostic report link.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TelegramReportInput<'a> {
    pub os: &'a str,
    pub ov: &'a str,
    pub v: &'a str,
    pub t: &'a str,
    pub pm: Option<&'a str>,
    pub l: Option<&'a str>,
    pub a: Option<&'a str>,
}

/// Sanitizes a report value by stripping characters outside `[A-Za-z0-9._-]`.
pub fn clean_report_value(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .collect()
}

/// Encodes device report details into Telegram's `<= 64`-char base64url (unpadded) `start` payload,
/// progressively dropping `a`, then `l`, then shortening `ov` character-by-character, matching Dart.
pub fn telegram_report_payload(input: &TelegramReportInput<'_>) -> String {
    fn encode_fields(
        os: &str,
        ov: &str,
        v: &str,
        pm: Option<&str>,
        t: &str,
        l: Option<&str>,
        a: Option<&str>,
    ) -> String {
        let ordered: [(&str, Option<&str>); 7] = [
            ("os", Some(os)),
            ("ov", Some(ov)),
            ("v", Some(v)),
            ("pm", pm),
            ("t", Some(t)),
            ("l", l),
            ("a", a),
        ];
        let mut pairs = Vec::with_capacity(7);
        for (key, val) in ordered {
            if let Some(raw) = val {
                let cleaned = clean_report_value(raw);
                if !cleaned.is_empty() {
                    pairs.push(format!("{key}={cleaned}"));
                }
            }
        }
        URL_SAFE_NO_PAD.encode(pairs.join(";").as_bytes())
    }

    let mut version: Vec<u16> = input.ov.encode_utf16().collect();
    let version_str = String::from_utf16_lossy(&version);

    let mut payload = encode_fields(
        input.os,
        &version_str,
        input.v,
        input.pm,
        input.t,
        input.l,
        input.a,
    );
    if payload.len() > MAX_TELEGRAM_PAYLOAD_LEN {
        payload = encode_fields(
            input.os,
            &version_str,
            input.v,
            input.pm,
            input.t,
            input.l,
            None,
        );
    }
    if payload.len() > MAX_TELEGRAM_PAYLOAD_LEN {
        payload = encode_fields(input.os, &version_str, input.v, input.pm, input.t, None, None);
    }
    while payload.len() > MAX_TELEGRAM_PAYLOAD_LEN && !version.is_empty() {
        version.pop();
        let shortened = String::from_utf16_lossy(&version);
        payload = encode_fields(input.os, &shortened, input.v, input.pm, input.t, None, None);
    }
    payload
}

/// Builds the full `https://t.me/<bot>?start=<payload>` URL.
pub fn telegram_report_url(bot_username: &str, input: &TelegramReportInput<'_>) -> String {
    let payload = telegram_report_payload(input);
    let mut url = Url::parse(&format!("https://t.me/{bot_username}")).expect("valid base url");
    url.query_pairs_mut().append_pair("start", &payload);
    url.to_string()
}

/// Distinguishes Windows 11 (`>= 22000`) from Windows 10 (`< 22000`) via `Build (\d+)`.
pub fn windows_version(os_version_string: &str) -> String {
    static BUILD_RE: OnceLock<Regex> = OnceLock::new();
    let re = BUILD_RE.get_or_init(|| Regex::new(r"Build (\d+)").expect("regex"));
    let Some(caps) = re.captures(os_version_string) else {
        return String::new();
    };
    let Some(build) = caps.get(1).and_then(|m| m.as_str().parse::<i64>().ok()) else {
        return String::new();
    };
    if build >= 22000 {
        "11".to_string()
    } else {
        "10".to_string()
    }
}

/// Parses `/etc/os-release` content into `(distro_code, version_id)`, matching `linuxDistro` in Dart.
pub fn linux_distro(os_release: &str) -> (String, String) {
    let mut values = HashMap::new();
    for raw_line in os_release.split('\n') {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if let Some(at) = line.find('=') {
            if at > 0 {
                let key = line[..at].trim().to_string();
                let mut val = line[at + 1..].trim();
                if val.starts_with('"') || val.starts_with('\'') {
                    val = &val[1..];
                }
                if val.ends_with('"') || val.ends_with('\'') {
                    val = &val[..val.len() - 1];
                }
                values.insert(key, val.to_string());
            }
        }
    }

    let id = values
        .get("ID")
        .map(|s| s.to_lowercase())
        .unwrap_or_default();

    let code = match id.as_str() {
        "ubuntu" => "ubu",
        "debian" => "deb",
        "fedora" => "fed",
        "linuxmint" => "mint",
        "arch" => "arch",
        "manjaro" => "manj",
        "endeavouros" => "eos",
        "cachyos" => "cachy",
        "garuda" => "garu",
        "pop" => "pop",
        "elementary" => "elem",
        "zorin" => "zorin",
        "mx" => "mx",
        "neon" => "neon",
        "kali" => "kali",
        "almalinux" => "alma",
        "rocky" => "rocky",
        "centos" => "cent",
        "rhel" => "rhel",
        "nixos" => "nix",
        "gentoo" => "gen",
        "void" => "void",
        "alpine" => "alp",
        "deepin" => "deep",
        "kylin" | "ubuntukylin" => "kylin",
        "astra" => "astra",
        "solus" => "sol",
        "slackware" => "slack",
        "steamos" => "steam",
        "bazzite" => "bazz",
        s if s.starts_with("opensuse") || s == "sles" => "sus",
        _ => "linux",
    };

    let version_id = values.remove("VERSION_ID").unwrap_or_default();
    (code.to_string(), version_id)
}

/// Extracted crash headline and body excerpt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrashExcerpt {
    pub headline: String,
    pub text: String,
}

/// Extracts a short crash headline and excerpt from a crash or Go stderr log.
/// When `find_go_marker` is `true`, seeks the first Go crash marker (`panic: `, `fatal error: `, etc.).
pub fn extract_crash_excerpt(content: &str, find_go_marker: bool) -> Option<CrashExcerpt> {
    static GO_CRASH_RE: OnceLock<Regex> = OnceLock::new();
    let start_byte = if find_go_marker {
        let re = GO_CRASH_RE.get_or_init(|| {
            Regex::new(r"(?m)^(panic: |fatal error: |runtime: |SIG[A-Z]+: |\[signal )")
                .expect("regex")
        });
        re.find(content).map(|m| m.start()).unwrap_or(0)
    } else {
        0
    };

    let slice = &content[start_byte..];
    let lines: Vec<&str> = slice.split('\n').collect();
    let headline_raw = lines
        .iter()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim())
        .unwrap_or("");
    if headline_raw.is_empty() {
        return None;
    }

    let joined = lines
        .into_iter()
        .take(MAX_CRASH_EXCERPT_LINES)
        .collect::<Vec<_>>()
        .join("\n");
    let trimmed_text = joined.trim_end();

    let text = if utf16_len(trimmed_text) > MAX_CRASH_EXCERPT_UTF16_LEN {
        format!(
            "{}\n…",
            utf16_prefix(trimmed_text, MAX_CRASH_EXCERPT_UTF16_LEN)
        )
    } else {
        trimmed_text.to_string()
    };

    let headline = if utf16_len(headline_raw) > MAX_CRASH_HEADLINE_UTF16_LEN {
        format!("{}…", utf16_prefix(headline_raw, MAX_CRASH_HEADLINE_UTF16_LEN))
    } else {
        headline_raw.to_string()
    };

    Some(CrashExcerpt { headline, text })
}

/// Extracts the crash kind suffix from a file basename (`2026-11-13T19-48-43-android.txt` -> `android`).
pub fn crash_file_kind(file_name: &str) -> String {
    let stem = file_name
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(file_name);
    match stem.rfind('-') {
        Some(idx) => stem[idx + 1..].to_string(),
        None => stem.to_string(),
    }
}

/// Extracts RFC 3339 UTC timestamp from a crash file name (`2026-11-13T19-48-43-android.txt` -> `2026-11-13T19:48:43Z`).
pub fn crash_file_time(file_name: &str) -> String {
    let utf16: Vec<u16> = file_name.encode_utf16().collect();
    if utf16.len() < 19 {
        return String::new();
    }
    let date_part = String::from_utf16_lossy(&utf16[..10]);
    let time_part = String::from_utf16_lossy(&utf16[11..19]).replace('-', ":");
    format!("{date_part}T{time_part}Z")
}

/// Parsed semantic tag components of a GitHub release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedReleaseTag {
    pub version: String,
    pub suffix: String,
    pub build_number: String,
}

/// Parses a GitHub release tag (`v5.4.0`, `v5.4.0-beta.1`, `v5.4.0.dev+540`).
pub fn parse_release_tag(tag_name: &str) -> Option<ParsedReleaseTag> {
    static TAG_RE: OnceLock<Regex> = OnceLock::new();
    let re = TAG_RE.get_or_init(|| {
        Regex::new(r"^v?(\d+\.\d+\.\d+)(?:[-.]([0-9A-Za-z.-]+))?(?:\+([0-9A-Za-z.-]+))?$")
            .expect("regex")
    });
    let trimmed = tag_name.trim();
    let caps = re.captures(trimmed)?;
    Some(ParsedReleaseTag {
        version: caps.get(1)?.as_str().to_string(),
        suffix: caps.get(2).map(|m| m.as_str()).unwrap_or("").to_string(),
        build_number: caps.get(3).map(|m| m.as_str()).unwrap_or("").to_string(),
    })
}

/// Extracts the Markdown changelog section from a GitHub release body, matching `GithubReleaseParser.changelog`.
pub fn extract_changelog(body: &str) -> Option<String> {
    static HEADING_OPEN_RE: OnceLock<Regex> = OnceLock::new();
    static HEADING_ANY_RE: OnceLock<Regex> = OnceLock::new();
    static DOWNLOAD_RE: OnceLock<Regex> = OnceLock::new();
    static COMMENT_RE: OnceLock<Regex> = OnceLock::new();
    static RULE_RE: OnceLock<Regex> = OnceLock::new();

    let open_re = HEADING_OPEN_RE.get_or_init(|| {
        Regex::new(r"(?i)^(#{1,6})\s*(?:change\s*-?\s*logs?|changes|what'?s\s+new|highlights?|تغییرات)\s*:?\s*#*\s*$")
            .expect("regex")
    });
    let heading_re = HEADING_ANY_RE.get_or_init(|| Regex::new(r"^(#{1,6})\s").expect("regex"));
    let download_re = DOWNLOAD_RE.get_or_init(|| Regex::new(r"(?i)download").expect("regex"));
    let comment_re = COMMENT_RE.get_or_init(|| Regex::new(r"(?s)<!--.*?-->").expect("regex"));
    let rule_re = RULE_RE.get_or_init(|| Regex::new(r"^(?:-{3,}|\*{3,}|_{3,})$").expect("regex"));

    let normalized = body.replace("\r\n", "\n");
    let cleaned = comment_re.replace_all(&normalized, "");
    let lines: Vec<&str> = cleaned.split('\n').collect();

    let start_idx = lines.iter().position(|l| open_re.is_match(l.trim()))?;
    let level = open_re
        .captures(lines[start_idx].trim())?
        .get(1)?
        .as_str()
        .len();

    let mut section = Vec::new();
    for &line in &lines[start_idx + 1..] {
        let trimmed = line.trim();
        if let Some(caps) = heading_re.captures(trimmed) {
            if caps.get(1).map(|m| m.as_str().len()).unwrap_or(0) <= level
                || download_re.is_match(trimmed)
            {
                break;
            }
        }
        if rule_re.is_match(trimmed) || trimmed.starts_with("<table") {
            break;
        }
        section.push(line);
    }

    let text = section.join("\n").trim().to_string();
    if text.is_empty() {
        return None;
    }

    if utf16_len(&text) > MAX_CHANGELOG_UTF16_LEN {
        let prefix = utf16_prefix(&text, MAX_CHANGELOG_UTF16_LEN);
        // Dart `text.lastIndexOf('\n', maxChangelogLength)` includes index `maxChangelogLength` itself
        let next_is_newline = text
            .encode_utf16()
            .nth(MAX_CHANGELOG_UTF16_LEN)
            .map(|u| u == b'\n' as u16)
            .unwrap_or(false);
        let cut_str = if next_is_newline {
            prefix.as_str()
        } else if let Some(byte_pos) = prefix.rfind('\n') {
            if byte_pos > 0 {
                &prefix[..byte_pos]
            } else {
                prefix.as_str()
            }
        } else {
            prefix.as_str()
        };
        return Some(format!("{}\n\n…", cut_str.trim_end()));
    }

    Some(text)
}

/// Log category grouping matching `LogCategory` in Dart (`lib/core/logger/ring/log_ring.dart`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogCategory {
    Boot,
    App,
    Core,
}

impl LogCategory {
    pub fn matches(self, logger_name: &str) -> bool {
        match self {
            Self::Boot => logger_name.starts_with("boot."),
            Self::Core => logger_name.starts_with("core."),
            Self::App => !logger_name.starts_with("boot.") && !logger_name.starts_with("core."),
        }
    }
}

/// Lightweight structured log entry for standalone `LogRing` usage in Rust / native consumers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub level: i32,
    pub logger_name: String,
    pub message: String,
    pub timestamp_ms: i64,
}

/// Bounded in-memory log ring buffer with monotonic revision tracking.
#[derive(Debug, Clone)]
pub struct LogRing {
    capacity: usize,
    records: VecDeque<LogEntry>,
    revision: u64,
}

impl LogRing {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            records: VecDeque::with_capacity(capacity.min(1024)),
            revision: 0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn set_capacity(&mut self, value: usize) {
        self.capacity = value;
        self.trim();
        self.revision += 1;
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn add(&mut self, entry: LogEntry) {
        self.records.push_back(entry);
        self.trim();
        self.revision += 1;
    }

    pub fn clear(&mut self) {
        self.records.clear();
        self.revision += 1;
    }

    fn trim(&mut self) {
        while self.records.len() > self.capacity {
            self.records.pop_front();
        }
    }

    pub fn view(
        &self,
        min_level: Option<i32>,
        category: Option<LogCategory>,
        text: Option<&str>,
    ) -> Vec<&LogEntry> {
        self.records
            .iter()
            .filter(|r| {
                if let Some(min) = min_level {
                    if r.level < min {
                        return false;
                    }
                }
                if let Some(cat) = category {
                    if !cat.matches(&r.logger_name) {
                        return false;
                    }
                }
                if let Some(q) = text {
                    if !q.is_empty() && !r.message.contains(q) {
                        return false;
                    }
                }
                true
            })
            .collect()
    }
}

pub(crate) fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

pub(crate) fn utf16_prefix(s: &str, max_units: usize) -> String {
    let units: Vec<u16> = s.encode_utf16().take(max_units).collect();
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telegram_report_link_matches_dart_vectors() {
        assert_eq!(
            telegram_report_payload(&TelegramReportInput {
                os: "android",
                ov: "14",
                v: "50901",
                pm: Some("play"),
                t: "c",
                l: None,
                a: None,
            }),
            "b3M9YW5kcm9pZDtvdj0xNDt2PTUwOTAxO3BtPXBsYXk7dD1j"
        );
        assert_eq!(
            telegram_report_url(
                DEFAULT_TELEGRAM_BOT,
                &TelegramReportInput {
                    os: "win",
                    ov: "11",
                    v: "50901",
                    pm: Some("msix"),
                    t: "c",
                    l: None,
                    a: None,
                }
            ),
            "https://t.me/hiddify_issues_bot?start=b3M9d2luO292PTExO3Y9NTA5MDE7cG09bXNpeDt0PWM"
        );

        let over = telegram_report_payload(&TelegramReportInput {
            os: "linux",
            ov: "6.8.12-300",
            v: "50901",
            pm: Some("appimage"),
            t: "b",
            l: Some("fa"),
            a: Some("x64"),
        });
        assert!(over.len() <= 64);
        let decoded = String::from_utf8(URL_SAFE_NO_PAD.decode(over).unwrap()).unwrap();
        assert_eq!(decoded, "os=linux;ov=6.8.12-300;v=50901;pm=appimage;t=b");
    }

    #[test]
    fn os_version_and_distro_detection() {
        assert_eq!(windows_version("\"Windows 10 Pro\" 10.0 (Build 22631)"), "11");
        assert_eq!(windows_version("\"Windows 10 Pro\" 10.0 (Build 19045)"), "10");
        assert_eq!(windows_version("something else"), "");

        assert_eq!(
            linux_distro("NAME=\"Ubuntu\"\nID=ubuntu\nVERSION_ID=\"24.04\"\n"),
            ("ubu".to_string(), "24.04".to_string())
        );
        assert_eq!(
            linux_distro("ID=\"opensuse-tumbleweed\"\nVERSION_ID=\"20261101\"\n"),
            ("sus".to_string(), "20261101".to_string())
        );
        assert_eq!(
            linux_distro("ID='someos'\nVERSION_ID='3'\n"),
            ("linux".to_string(), "3".to_string())
        );
    }

    #[test]
    fn crash_excerpt_and_filename_metadata() {
        let log = "info: starting\npanic: runtime error: index out of range\ngoroutine 1 [running]:\nmain.main()\n";
        let ex = extract_crash_excerpt(log, true).expect("excerpt");
        assert_eq!(ex.headline, "panic: runtime error: index out of range");
        assert!(ex.text.starts_with("panic: runtime error:"));

        assert_eq!(crash_file_kind("2026-11-13T19-48-43-2-android.txt"), "android");
        assert_eq!(crash_file_time("2026-11-13T19-48-43-android.txt"), "2026-11-13T19:48:43Z");
    }

    #[test]
    fn changelog_extraction_handles_persian_and_boundaries() {
        let md = "<!-- comment -->\r\n## تغییرات\r\n- Fix A\r\n- Fix B\r\n## Downloads\r\n| OS | Link |";
        assert_eq!(extract_changelog(md).as_deref(), Some("- Fix A\n- Fix B"));
    }

    #[test]
    fn log_sink_stack_origin_crash_store_and_windows_crash() {
        use core_guard::*;
        use log_crash_store::*;

        let dir = tempfile::tempdir().expect("tempdir");
        let log_path = dir.path().join("nested/app.log");
        assert!(open_log_file(log_path.to_str().unwrap()));
        write_log_line("SEVERE broke");
        assert_eq!(
            std::fs::read_to_string(&log_path).unwrap(),
            "SEVERE broke\n"
        );
        close_log_file();

        let stack = "#0      appLogger.error (package:hiddify/core/logger/custom.dart:10:5)\n#1      ProfileRepository.save.<anonymous closure> (package:hiddify/features/profile/repo.dart:88:12)";
        assert_eq!(
            origin_of_stack(stack, "package:hiddify/", "package:hiddify/core/logger/").as_deref(),
            Some("ProfileRepository.save (repo.dart:88)")
        );

        assert!(open_app_crash_store(dir.path().to_str().unwrap()));
        record_app_crash("dart", "same");
        record_app_crash("dart", "same");
        assert_eq!(list_app_crash_files(dir.path().to_str().unwrap()).len(), 1);
        for i in 0..15 {
            record_app_crash("native", &format!("err {i}"));
        }
        assert_eq!(list_app_crash_files(dir.path().to_str().unwrap()).len(), 10);

        assert_eq!(
            describe_windows_exit_code(-1073741819).as_deref(),
            Some("ended by Windows: access violation (0xc0000005)")
        );
        assert_eq!(
            core_crash_headline("Exception 0xc0000005 0x0 0x7ff8\nPC=0x1").as_deref(),
            Some("Exception 0xc0000005 0x0 0x7ff8")
        );
        assert!(is_core_gone_grpc_error(14, None));
        assert!(is_core_gone_grpc_error(
            2,
            Some("HTTP/2 error: Connection is being forcefully terminated.")
        ));
        assert!(!is_core_gone_grpc_error(2, Some("other")));
    }
}
