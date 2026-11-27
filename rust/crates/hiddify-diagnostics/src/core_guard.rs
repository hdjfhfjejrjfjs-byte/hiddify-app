//! Desktop Core process stdout handshake parser, Go/C crash headline & timestamped crash-file
//! inspection (`core_process.dart`), Windows Event 1000 XML & NTSTATUS exit code parser
//! (`windows_crash.dart`), and gRPC error / start-failure classifiers (`grpc_errors.dart`,
//! `hiddify_core_service.dart`).

use std::fs;
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::{utf16_len, utf16_prefix};

const READY_PREFIX: &str = "HIDDIFY_CORE_READY ";
const ERROR_PREFIX: &str = "HIDDIFY_CORE_ERROR ";
const MAX_CORE_REASON_UTF16_LEN: usize = 300;

/// Parsed line from a desktop `CoreProcess` stdout pipe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CoreStdoutLine {
    Ready { certificate_pem: String },
    Error { message: String },
    Log { line: String },
    Empty,
}

/// Parses a single stdout line emitted by `hiddify-core`, decoding the Base64 server PEM on `HIDDIFY_CORE_READY`.
pub fn parse_core_stdout_line(line: &str) -> CoreStdoutLine {
    if let Some(rest) = line.strip_prefix(READY_PREFIX) {
        let b64 = rest.trim();
        if b64.is_empty() {
            return CoreStdoutLine::Ready {
                certificate_pem: String::new(),
            };
        }
        match BASE64_STANDARD
            .decode(b64.as_bytes())
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
        {
            Some(pem) => CoreStdoutLine::Ready {
                certificate_pem: pem,
            },
            None => CoreStdoutLine::Error {
                message: "invalid base64 server certificate from core".to_string(),
            },
        }
    } else if let Some(err) = line.strip_prefix(ERROR_PREFIX) {
        CoreStdoutLine::Error {
            message: err.to_string(),
        }
    } else if line.is_empty() {
        CoreStdoutLine::Empty
    } else {
        CoreStdoutLine::Log {
            line: line.to_string(),
        }
    }
}

/// Extracts the crash headline from Go or Go-C-thread crash output (`CoreProcess.crashHeadline`),
/// shortened to 300 UTF-16 code units with `…`.
pub fn core_crash_headline(output: &str) -> Option<String> {
    static SIG_RE: OnceLock<Regex> = OnceLock::new();
    static WIN_EXC_RE: OnceLock<Regex> = OnceLock::new();

    let sig_re = SIG_RE.get_or_init(|| Regex::new(r"^SIG[A-Z]+: ").expect("regex"));
    let win_exc_re =
        WIN_EXC_RE.get_or_init(|| Regex::new(r"^Exception 0x[0-9a-fA-F]+").expect("regex"));

    let lines: Vec<&str> = output
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l).trim())
        .collect();

    let matched = lines
        .iter()
        .copied()
        .find(|l| l.starts_with("panic: ") || l.starts_with("fatal error: "))
        .or_else(|| lines.iter().copied().find(|l| sig_re.is_match(l)))
        .or_else(|| lines.iter().copied().find(|l| win_exc_re.is_match(l)))?;

    if utf16_len(matched) > MAX_CORE_REASON_UTF16_LEN {
        Some(format!("{}…", utf16_prefix(matched, MAX_CORE_REASON_UTF16_LEN)))
    } else {
        Some(matched.to_string())
    }
}

/// Resolves a crashed `CoreProcess`'s reason from `crash_file_path` (only when modified at or after
/// `started_at_ms - 1000`) or `stderr_tail` (`CoreProcess.crashReasonOf`).
pub fn core_crash_reason_of(
    crash_file_path: &str,
    stderr_tail: &str,
    started_at_ms: i64,
) -> Option<String> {
    let path = Path::new(crash_file_path);
    if path.exists() {
        if let Ok(meta) = fs::metadata(path) {
            if let Ok(modified) = meta.modified() {
                let modified_ms = modified
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or(Duration::ZERO)
                    .as_millis() as i64;
                if modified_ms >= started_at_ms.saturating_sub(1000) {
                    if let Ok(text) = fs::read_to_string(path) {
                        if let Some(headline) = core_crash_headline(&text) {
                            return Some(headline);
                        }
                    }
                }
            }
        }
    }
    core_crash_headline(stderr_tail)
}

fn ntstatus_name(code: u32) -> Option<&'static str> {
    match code {
        0xC0000005 => Some("access violation"),
        0xC0000409 => Some("a fail-fast exit or a stack buffer overrun"),
        0xC00000FD => Some("stack overflow"),
        0xC0000374 => Some("heap corruption"),
        0xC0000017 => Some("out of memory"),
        0xC000009A => Some("out of system resources"),
        0xC0000135 => Some("a DLL it needs was not found"),
        0xC0000142 => Some("a DLL failed to initialize"),
        0xC000001D => Some("an illegal instruction"),
        0xC0000094 => Some("an integer division by zero"),
        0xC000013A => Some("ended with Ctrl+C or its console closing"),
        0x40010004 => Some("ended by a debugger or with the system shutting down"),
        0xE06D7363 => Some("an unhandled C++ exception"),
        _ => None,
    }
}

fn parse_hex_u64(raw: &str) -> Option<u64> {
    let stripped = raw
        .strip_prefix("0x")
        .or_else(|| raw.strip_prefix("0X"))
        .unwrap_or(raw);
    u64::from_str_radix(stripped, 16).ok()
}

/// Parses `wevtutil qe Application ... /f:xml` Event 1000 XML for `pid` (`WindowsCrash.applicationErrorFor`).
pub fn windows_application_error_for(xml: &str, pid: i64) -> Option<String> {
    if pid < 0 {
        return None;
    }
    let target_pid = pid as u64;
    static EVENT_RE: OnceLock<Regex> = OnceLock::new();
    static DATA_RE: OnceLock<Regex> = OnceLock::new();

    let event_re = EVENT_RE.get_or_init(|| Regex::new(r"(?s)<Event[ >].*?</Event>").expect("regex"));
    let data_re =
        DATA_RE.get_or_init(|| Regex::new(r"(?s)<Data(?:\s[^>]*)?>(.*?)</Data>").expect("regex"));

    for ev_match in event_re.find_iter(xml) {
        let data: Vec<&str> = data_re
            .captures_iter(ev_match.as_str())
            .filter_map(|c| c.get(1).map(|m| m.as_str().trim()))
            .collect();
        if data.len() < 9 {
            continue;
        }
        let Some(event_pid) = parse_hex_u64(data[8]) else {
            continue;
        };
        if event_pid != target_pid {
            continue;
        }
        let code_u32 = parse_hex_u64(data[6]).and_then(|v| u32::try_from(v).ok());
        let name_suffix = code_u32
            .and_then(ntstatus_name)
            .map(|n| format!(", {n}"))
            .unwrap_or_default();
        let code_text = match code_u32 {
            Some(c) => format!("0x{c:08x}"),
            None => data[6].to_string(),
        };
        return Some(format!(
            "a native crash in {} (exception {code_text}{name_suffix} at offset {})",
            data[3], data[7]
        ));
    }
    None
}

/// Maps a Windows process exit code to a human-readable explanation (`WindowsCrash.describeExitCode`).
pub fn describe_windows_exit_code(exit_code: i64) -> Option<String> {
    let code = (exit_code as u64 & 0xFFFF_FFFF) as u32;
    if let Some(name) = ntstatus_name(code) {
        return Some(format!("ended by Windows: {name} (0x{code:x})"));
    }
    if code == 1 {
        return Some(
            "ended with exit code 1: an error, or another program (Task Manager, an antivirus) ended it"
                .to_string(),
        );
    }
    None
}

/// Checks whether a gRPC error code + message indicates the core channel simply closed/went away
/// (`grpc_errors.dart:isCoreGoneError`).
/// gRPC status codes: `14` = `UNAVAILABLE`, `2` = `UNKNOWN`.
pub fn is_core_gone_grpc_error(grpc_code: i32, message: Option<&str>) -> bool {
    if grpc_code == 14 {
        return true;
    }
    grpc_code == 2
        && message
            .unwrap_or("")
            .contains("Connection is being forcefully terminated")
}

/// Classifies a core start failure message (`HiddifyCoreService._startFailure`).
/// Returns `"missing_privilege"`, `"tun_address_in_use"`, `"vpn_permission_denied"`, or `"other"`.
pub fn classify_core_start_failure(message: &str) -> &'static str {
    if message.contains("start inbound/tun[tun-in]: configure tun interface: Access is denied.") {
        return "missing_privilege";
    }
    if message.contains(
        "start inbound/tun[tun-in]: configure tun interface: set ipv4 address: The object already exists.",
    ) {
        return "tun_address_in_use";
    }
    if message.contains("denied") {
        return "vpn_permission_denied";
    }
    "other"
}
