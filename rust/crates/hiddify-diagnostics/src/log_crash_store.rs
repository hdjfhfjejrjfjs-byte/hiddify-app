//! Persistent error log sink (`file_sink_io.dart`), Dart stack-frame origin extraction
//! (`logger_setup.dart`), monotonic app crash store (`app_crash_store_io.dart`),
//! preference secret scrubbing (`app_diagnostics.dart`), and byte-budgeted diagnostics assembly
//! (`diagnostics_exporter.dart`).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{crash_file_kind, crash_file_time, utf16_len, utf16_prefix};

pub const APP_CRASH_DIRECTORY: &str = "app_crashes";
pub const FATAL_APP_CRASH_KINDS: &[&str] = &["android", "native", "anr"];

const LOG_MAX_SIZE: u64 = 1024 * 1024;
const LOG_KEEP_SIZE: usize = 256 * 1024;

const CRASH_KEEP_FILES: usize = 10;
const CRASH_MAX_PER_RUN: usize = 20;

const MAX_PREFERENCE_UTF16_LEN: usize = 4000;

const DIAG_MAX_REQUEST_SIZE: usize = 3584 * 1024;
const DIAG_MAX_EXTRA_FILE_SIZE: usize = 1 << 20;
const DIAG_MAX_APP_LOG_FILE_SIZE: usize = 256 * 1024;
const DIAG_MAX_BACKGROUND_ZIP_SIZE: usize = 2 << 20;
const DIAG_MAX_DESCRIPTION_UTF16_LEN: usize = 4000;
const DIAG_MAX_APP_CRASHES: usize = 5;
const DIAG_MAX_APP_CRASH_SIZE: usize = 128 * 1024;

// ============================================================================
// 1. Persistent Error Log File Sink (`file_sink_io.dart`)
// ============================================================================

fn log_sink_state() -> &'static Mutex<Option<PathBuf>> {
    static STATE: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(None))
}

/// Opens `path` for persistent error logging, creating parent directories and truncating files
/// larger than 1 MiB to their newest 256 KiB (`openLogFile`). Returns `true` if opened and writable.
pub fn open_log_file(path: &str) -> bool {
    let p = PathBuf::from(path);
    let open_res = (|| -> std::io::Result<()> {
        if let Some(parent) = p.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        if p.exists() {
            let meta = fs::metadata(&p)?;
            if meta.len() > LOG_MAX_SIZE {
                let bytes = fs::read(&p)?;
                if bytes.len() > LOG_KEEP_SIZE {
                    fs::write(&p, &bytes[bytes.len() - LOG_KEEP_SIZE..])?;
                }
            }
        }
        let mut f = OpenOptions::new().create(true).append(true).open(&p)?;
        f.write_all(b"")?;
        Ok(())
    })();

    let mut guard = log_sink_state().lock().unwrap_or_else(|e| e.into_inner());
    match open_res {
        Ok(()) => {
            *guard = Some(p);
            true
        }
        Err(_) => {
            *guard = None;
            false
        }
    }
}

pub fn is_log_file_open() -> bool {
    log_sink_state()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some()
}

/// Appends `line` + `\n` directly to the open log file without buffering (`writeLogLine`).
pub fn write_log_line(line: &str) {
    let path_opt = log_sink_state()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let Some(path) = path_opt else {
        return;
    };
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(line.as_bytes());
        let _ = f.write_all(b"\n");
    }
}

pub fn close_log_file() {
    *log_sink_state().lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Writes `content` to `path` in one shot (`writeLinesToFile`).
pub fn write_text_to_file(path: &str, content: &str) -> bool {
    fs::write(path, content.as_bytes()).is_ok()
}

// ============================================================================
// 2. Dart Stack-Trace Origin & Frame Filter (`logger_setup.dart`)
// ============================================================================

fn is_our_frame(frame: &str, own_prefix: &str, logger_internals: &str) -> bool {
    frame.contains(own_prefix) && !frame.contains(logger_internals)
}

/// Turns the first frame from `own_prefix` (excluding `logger_internals`) into
/// `Class.method (file.dart:line)` (`logger_setup.dart:originOf`).
pub fn origin_of_stack(stack: &str, own_prefix: &str, logger_internals: &str) -> Option<String> {
    static FRAME_NUM_RE: OnceLock<Regex> = OnceLock::new();
    let num_re = FRAME_NUM_RE.get_or_init(|| Regex::new(r"^#\d+\s+").expect("regex"));

    for frame in stack.split('\n') {
        if !is_our_frame(frame, own_prefix, logger_internals) {
            continue;
        }
        let Some(open) = frame.find('(') else {
            continue;
        };
        let Some(close) = frame.rfind(')') else {
            continue;
        };
        if close <= open {
            continue;
        }
        let raw_method = &frame[..open];
        let without_num = num_re.replace(raw_method, "");
        let method = without_num.replace(".<anonymous closure>", "").trim().to_string();

        let parts: Vec<&str> = frame[open + 1..close].split(':').collect();
        if parts.len() < 3 {
            continue;
        }
        let file = parts[parts.len() - 3]
            .rsplit('/')
            .next()
            .unwrap_or(parts[parts.len() - 3]);
        let line = parts[parts.len() - 2];
        return Some(format!("{method} ({file}:{line})"));
    }
    None
}

/// Keeps at most `max_frames` stack trace lines belonging to `own_prefix` (`logger_setup.dart:_ownFrames`).
pub fn own_stack_frames(
    stack: &str,
    own_prefix: &str,
    logger_internals: &str,
    max_frames: usize,
) -> Option<String> {
    let mine: Vec<&str> = stack
        .split('\n')
        .filter(|f| is_our_frame(f, own_prefix, logger_internals))
        .take(max_frames)
        .collect();
    if mine.is_empty() {
        None
    } else {
        Some(mine.join("\n"))
    }
}

// ============================================================================
// 3. Monotonic App Crash Store (`app_crash_store_io.dart` & `crash_excerpt.dart`)
// ============================================================================

#[derive(Default)]
struct CrashStoreState {
    directory: Option<PathBuf>,
    recorded: usize,
    last_text: Option<String>,
    last_time_ms: Option<i64>,
}

fn crash_store_state() -> &'static Mutex<CrashStoreState> {
    static STATE: OnceLock<Mutex<CrashStoreState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(CrashStoreState::default()))
}

/// Howard Hinnant `civil_from_days` UTC timestamp formatter matching
/// `DateTime.toUtc().toIso8601String().substring(0, 23).replaceAll(':', '-').replaceAll('.', '-')`.
pub fn format_utc_millis_stamp(unix_ms: i64) -> String {
    let total_secs = unix_ms.div_euclid(1000);
    let millis = unix_ms.rem_euclid(1000);
    let days = total_secs.div_euclid(86_400);
    let sec_of_day = total_secs.rem_euclid(86_400);
    let hour = sec_of_day / 3600;
    let minute = (sec_of_day % 3600) / 60;
    let second = sec_of_day % 60;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };

    format!("{year:04}-{m:02}-{d:02}T{hour:02}-{minute:02}-{second:02}-{millis:03}")
}

/// Initializes `<working_dir>/app_crashes` (`openAppCrashStore`).
pub fn open_app_crash_store(working_dir: &str) -> bool {
    let dir = Path::new(working_dir).join(APP_CRASH_DIRECTORY);
    let mut guard = crash_store_state().lock().unwrap_or_else(|e| e.into_inner());
    match fs::create_dir_all(&dir) {
        Ok(()) => {
            if guard.directory.as_deref() != Some(dir.as_path()) {
                guard.recorded = 0;
                guard.last_text = None;
                guard.last_time_ms = None;
            }
            guard.directory = Some(dir);
            true
        }
        Err(_) => {
            guard.directory = None;
            false
        }
    }
}

/// Records a crash synchronously with deduplication, 20-per-run cap, monotonic UTC millisecond naming,
/// `sync_all()` flush, and pruning to the newest 10 files (`recordAppCrash`).
pub fn record_app_crash(kind: &str, text: &str) {
    let mut guard = crash_store_state().lock().unwrap_or_else(|e| e.into_inner());
    let Some(dir) = guard.directory.clone() else {
        return;
    };
    if guard.recorded >= CRASH_MAX_PER_RUN || guard.last_text.as_deref() == Some(text) {
        return;
    }
    guard.recorded += 1;
    guard.last_text = Some(text.to_string());

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let mut time_ms = match guard.last_time_ms {
        Some(last) if now_ms <= last => last + 1,
        _ => now_ms,
    };

    loop {
        let candidate = dir.join(format!("{}-{kind}.txt", format_utc_millis_stamp(time_ms)));
        if !candidate.exists() {
            guard.last_time_ms = Some(time_ms);
            if let Ok(mut file) = File::create(&candidate) {
                let _ = file.write_all(text.as_bytes());
                let _ = file.sync_all();
            }
            prune_crash_files(&dir);
            break;
        }
        time_ms += 1;
    }
}

fn prune_crash_files(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.path())
        .collect();
    files.sort();
    if files.len() > CRASH_KEEP_FILES {
        for old in &files[..files.len() - CRASH_KEEP_FILES] {
            let _ = fs::remove_file(old);
        }
    }
}

/// Lists sorted `.txt` crash file paths in `<working_dir>/app_crashes`, oldest first (`appCrashFiles`).
pub fn list_app_crash_files(working_dir: &str) -> Vec<String> {
    let dir = Path::new(working_dir).join(APP_CRASH_DIRECTORY);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut paths: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|e| {
            let s = e.path().to_string_lossy().into_owned();
            s.ends_with(".txt").then_some(s)
        })
        .collect();
    paths.sort();
    paths
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppCrashReportMeta {
    pub name: String,
    pub source: String,
    pub crashed_at: String,
}

/// Lists fatal app crashes (`android`, `native`, `anr`) with RFC 3339 timestamps (`appCrashReports`).
pub fn list_fatal_app_crash_reports(working_dir: &str) -> Vec<AppCrashReportMeta> {
    let mut out = Vec::new();
    for full_path in list_app_crash_files(working_dir) {
        let name = Path::new(&full_path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or(full_path);
        let kind = crash_file_kind(&name);
        if FATAL_APP_CRASH_KINDS.contains(&kind.as_str()) {
            out.push(AppCrashReportMeta {
                crashed_at: crash_file_time(&name),
                name,
                source: APP_CRASH_DIRECTORY.to_string(),
            });
        }
    }
    out
}

// ============================================================================
// 4. Preferences Scrubber & Diagnostics Budget Assembler
// ============================================================================

/// Scrubs secret or oversized preference values (`app_diagnostics.dart:_preferenceValue`),
/// returning sorted keys.
pub fn scrub_preferences_map(prefs: &Map<String, Value>) -> Map<String, Value> {
    static HIDDEN_RE: OnceLock<Regex> = OnceLock::new();
    let re = HIDDEN_RE.get_or_init(|| {
        Regex::new(r"password|secret|license|token|pairing|clean-ip|per_app_proxy_(include|exclude)_list")
            .expect("regex")
    });

    let mut keys: Vec<&String> = prefs.keys().collect();
    keys.sort();

    let mut out = Map::new();
    for key in keys {
        let val = prefs.get(key).unwrap_or(&Value::Null);
        let scrubbed = if re.is_match(key) {
            match val {
                Value::Null => Value::Null,
                Value::Array(arr) => Value::String(format!("<{} entries>", arr.len())),
                Value::String(text) if key.ends_with("_list") => {
                    let count = text.split(';').filter(|e| !e.is_empty()).count();
                    Value::String(format!("<{count} entries>"))
                }
                Value::String(text) if text.is_empty() => Value::String("<empty>".to_string()),
                _ => Value::String("<set>".to_string()),
            }
        } else if let Value::String(text) = val {
            let len = utf16_len(text);
            if len > MAX_PREFERENCE_UTF16_LEN {
                Value::String(format!(
                    "{}… <{len} characters>",
                    utf16_prefix(text, MAX_PREFERENCE_UTF16_LEN)
                ))
            } else {
                val.clone()
            }
        } else {
            val.clone()
        };
        out.insert(key.clone(), scrubbed);
    }
    out
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssembledDiagnosticsPayload {
    /// Base64-encoded bytes keyed by file name for `ExportDiagnosticsRequest.extraFiles`.
    pub extra_files_b64: Map<String, Value>,
    /// Base64-encoded bytes keyed by file name for `ExportDiagnosticsRequest.genericFiles`.
    pub generic_files_b64: Map<String, Value>,
    /// Remaining byte budget (`<= 2 MiB`) available for `background-core.zip`.
    pub remaining_bg_zip_budget: usize,
}

fn tail_bytes(bytes: &[u8], max: usize) -> &[u8] {
    if bytes.len() > max {
        &bytes[bytes.len() - max..]
    } else {
        bytes
    }
}

/// Assembles `description.txt`, `app-info.json`, `app-logs.txt`, `app.log`, and newest 5 `app-crash-*`
/// files within Hiddify's 3.5 MiB gRPC diagnostics budget (`diagnostics_exporter.dart`).
pub fn assemble_diagnostics_payload(
    description: Option<&str>,
    app_info_json: Option<&str>,
    ring_logs_text: &str,
    app_log_file_path: Option<&str>,
    working_dir: &str,
) -> AssembledDiagnosticsPayload {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;

    let mut extra_b64 = Map::new();
    let mut generic_b64 = Map::new();
    let mut total_bytes: usize = 0;

    let desc_trimmed = description.unwrap_or("").trim();
    if !desc_trimmed.is_empty() {
        let cut = if utf16_len(desc_trimmed) > DIAG_MAX_DESCRIPTION_UTF16_LEN {
            utf16_prefix(desc_trimmed, DIAG_MAX_DESCRIPTION_UTF16_LEN)
        } else {
            desc_trimmed.to_string()
        };
        total_bytes += cut.len();
        extra_b64.insert(
            "description.txt".to_string(),
            Value::String(STANDARD.encode(cut.as_bytes())),
        );
    }

    if let Some(info_raw) = app_info_json {
        if !info_raw.is_empty() {
            let pretty = serde_json::from_str::<Value>(info_raw)
                .ok()
                .and_then(|v| serde_json::to_string_pretty(&v).ok())
                .unwrap_or_else(|| info_raw.to_string());
            total_bytes += pretty.len();
            extra_b64.insert(
                "app-info.json".to_string(),
                Value::String(STANDARD.encode(pretty.as_bytes())),
            );
        }
    }

    let ring_tail = tail_bytes(ring_logs_text.as_bytes(), DIAG_MAX_EXTRA_FILE_SIZE);
    total_bytes += ring_tail.len();
    extra_b64.insert(
        "app-logs.txt".to_string(),
        Value::String(STANDARD.encode(ring_tail)),
    );

    if let Some(log_path) = app_log_file_path {
        if let Ok(bytes) = fs::read(log_path) {
            let tail = tail_bytes(&bytes, DIAG_MAX_APP_LOG_FILE_SIZE);
            total_bytes += tail.len();
            extra_b64.insert(
                "app.log".to_string(),
                Value::String(STANDARD.encode(tail)),
            );
        }
    }

    let crash_paths = list_app_crash_files(working_dir);
    for crash_path in crash_paths.into_iter().rev().take(DIAG_MAX_APP_CRASHES) {
        if let Ok(bytes) = fs::read(&crash_path) {
            let name = Path::new(&crash_path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let tail = tail_bytes(&bytes, DIAG_MAX_APP_CRASH_SIZE);
            total_bytes += tail.len();
            generic_b64.insert(
                format!("app-crash-{name}"),
                Value::String(STANDARD.encode(tail)),
            );
        }
    }

    let room = DIAG_MAX_REQUEST_SIZE.saturating_sub(total_bytes);
    AssembledDiagnosticsPayload {
        extra_files_b64: extra_b64,
        generic_files_b64: generic_b64,
        remaining_bg_zip_budget: room.min(DIAG_MAX_BACKGROUND_ZIP_SIZE),
    }
}
