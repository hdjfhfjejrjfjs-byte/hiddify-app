//! Subscription download failure classification (`DownloadProblem`), HTTP header flattening,
//! and inner subscription link-expansion planning & reassembly (`profile_parser.dart`).

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{is_subscription_link, safe_decode_base64};

/// Exact 0..=13 ordinal variants matching Dart `DownloadProblem.values` in `download_problem.dart`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DownloadProblemKind {
    ServerError = 0,
    NotFound = 1,
    Denied = 2,
    TooManyRequests = 3,
    BadStatus = 4,
    TooLarge = 5,
    NoInternet = 6,
    Dns = 7,
    Certificate = 8,
    Reset = 9,
    Tls = 10,
    Refused = 11,
    Timeout = 12,
    Unknown = 13,
}

impl DownloadProblemKind {
    pub fn from_index(idx: i32) -> Self {
        match idx {
            0 => Self::ServerError,
            1 => Self::NotFound,
            2 => Self::Denied,
            3 => Self::TooManyRequests,
            4 => Self::BadStatus,
            5 => Self::TooLarge,
            6 => Self::NoInternet,
            7 => Self::Dns,
            8 => Self::Certificate,
            9 => Self::Reset,
            10 => Self::Tls,
            11 => Self::Refused,
            12 => Self::Timeout,
            _ => Self::Unknown,
        }
    }

    pub fn index(self) -> i32 {
        self as i32
    }

    pub fn may_pass(self) -> bool {
        !matches!(
            self,
            Self::NotFound
                | Self::Denied
                | Self::BadStatus
                | Self::TooLarge
                | Self::Certificate
        )
    }

    pub fn of_status(code: i32) -> Self {
        match code {
            c if c >= 500 => Self::ServerError,
            404 | 410 => Self::NotFound,
            401 | 403 => Self::Denied,
            429 => Self::TooManyRequests,
            _ => Self::BadStatus,
        }
    }

    /// Maps `FetchUrlResponse(error, statusCode)` (`hcore.pbenum.dart:FetchUrlError`).
    /// Returns `None` when the core succeeded (`error == 0 && 200..=299`).
    pub fn of_core(fetch_error: i32, status_code: i32) -> Option<Self> {
        match fetch_error {
            0 => {
                if (200..=299).contains(&status_code) {
                    None
                } else {
                    Some(Self::of_status(status_code))
                }
            }
            1 => Some(Self::Timeout),
            2 => Some(Self::Dns),
            3 => Some(Self::Refused),
            4 => Some(Self::Reset),
            5 => Some(Self::NoInternet),
            6 => Some(Self::Tls),
            7 => Some(Self::Certificate),
            9 => Some(Self::TooLarge),
            _ => Some(Self::Unknown),
        }
    }

    /// Classifies a `DioException` into `DownloadProblemKind`, matching `DownloadProblem.ofDio`.
    pub fn classify_dio(
        status_code: Option<i32>,
        dio_type: &str,
        is_cert_exception: bool,
        is_tls_or_handshake_exception: bool,
        cause_and_message: &str,
    ) -> Self {
        if let Some(code) = status_code {
            if !(200..=299).contains(&code) {
                return Self::of_status(code);
            }
        }
        match dio_type {
            "connectionTimeout" | "sendTimeout" | "receiveTimeout" => return Self::Timeout,
            "badCertificate" => return Self::Certificate,
            _ => {}
        }
        if is_cert_exception {
            return Self::Certificate;
        }
        let text = cause_and_message.to_lowercase();
        if text.contains("certificate") {
            return Self::Certificate;
        }
        if text.contains("failed host lookup")
            || text.contains("nodename nor servname")
            || text.contains("no address")
        {
            return Self::Dns;
        }
        if text.contains("network is unreachable") || text.contains("no route to host") {
            return Self::NoInternet;
        }
        if text.contains("refused") {
            return Self::Refused;
        }
        if text.contains("reset by peer")
            || text.contains("connection reset")
            || text.contains("forcibly closed")
            || text.contains("terminated during handshake")
            || text.contains("broken pipe")
        {
            return Self::Reset;
        }
        if text.contains("timed out") {
            return Self::Timeout;
        }
        if is_tls_or_handshake_exception {
            return Self::Tls;
        }
        Self::Unknown
    }

    pub fn most_telling(problems: &[Self]) -> Self {
        problems
            .iter()
            .copied()
            .fold(Self::Unknown, |a, b| if (b as i32) < (a as i32) { b } else { a })
    }

    pub fn code_str(self, status: Option<i32>) -> String {
        match self {
            Self::ServerError
            | Self::NotFound
            | Self::Denied
            | Self::TooManyRequests
            | Self::BadStatus => status
                .map(|s| s.to_string())
                .unwrap_or_else(|| "?".to_string()),
            Self::TooLarge => "TOO_LARGE".to_string(),
            Self::NoInternet => "NET_OFFLINE".to_string(),
            Self::Dns => "DNS_NOT_FOUND".to_string(),
            Self::Certificate => "TLS_CERT".to_string(),
            Self::Reset => "CONN_RESET".to_string(),
            Self::Tls => "TLS_FAILED".to_string(),
            Self::Refused => "CONN_REFUSED".to_string(),
            Self::Timeout => "NET_TIMEOUT".to_string(),
            Self::Unknown => "UNKNOWN".to_string(),
        }
    }

    pub fn failed_stop(self) -> Option<i32> {
        match self {
            Self::NoInternet => Some(0),
            Self::Dns | Self::Certificate | Self::Reset | Self::Tls | Self::Timeout => Some(1),
            Self::ServerError
            | Self::NotFound
            | Self::Denied
            | Self::TooManyRequests
            | Self::BadStatus
            | Self::TooLarge
            | Self::Refused => Some(2),
            Self::Unknown => None,
        }
    }

    pub fn side_index(self) -> i32 {
        // 0 = provider, 1 = subscription, 2 = network, 3 = unknown
        match self {
            Self::ServerError
            | Self::TooManyRequests
            | Self::BadStatus
            | Self::TooLarge
            | Self::Refused => 0,
            Self::NotFound | Self::Denied => 1,
            Self::NoInternet
            | Self::Dns
            | Self::Certificate
            | Self::Reset
            | Self::Tls
            | Self::Timeout => 2,
            Self::Unknown => 3,
        }
    }
}

/// Flattens multi-value HTTP headers: single-element list -> `String`, multi-element -> `Array`
/// (`ProfileParser._flattenHeaders`).
pub fn flatten_http_headers(headers: &IndexMap<String, Vec<String>>) -> Map<String, Value> {
    let mut out = Map::new();
    for (k, v) in headers {
        if v.len() == 1 {
            out.insert(k.clone(), Value::String(v[0].clone()));
        } else {
            out.insert(
                k.clone(),
                Value::Array(v.iter().cloned().map(Value::String).collect()),
            );
        }
    }
    out
}

/// One inner subscription link discovered by `plan_link_expansion`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InnerLinkTarget {
    pub index: usize,
    pub url: String,
}

/// Plan returned by `plan_link_expansion` before concurrent HTTP fetches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkExpansionPlan {
    pub decoded_text: String,
    pub targets: Vec<InnerLinkTarget>,
}

/// Decodes `raw_text` (optional Base64) and scans all lines for inner subscription links
/// (`ProfileParser._expandLinksInFile` + `expandLinks`).
pub fn plan_link_expansion(raw_text: &str, decode_base64_first: bool) -> LinkExpansionPlan {
    let decoded_text = if decode_base64_first {
        safe_decode_base64(raw_text.trim())
    } else {
        raw_text.to_string()
    };

    let mut targets = Vec::new();
    for (index, line) in decoded_text.split('\n').enumerate() {
        if is_subscription_link(line) {
            targets.push(InnerLinkTarget {
                index,
                url: line.trim_end().to_string(),
            });
        }
    }

    LinkExpansionPlan {
        decoded_text,
        targets,
    }
}

/// Reassembles `decoded_text` by replacing each inner link line at `index` with its trimmed fetched body
/// (or `""` if download failed), matching `ProfileParser.expandLinks`.
pub fn merge_expanded_links(decoded_text: &str, replacements: &[(usize, String)]) -> String {
    if replacements.is_empty() {
        return decoded_text.to_string();
    }
    let mut lines: Vec<String> = decoded_text.split('\n').map(str::to_string).collect();
    for (idx, body) in replacements {
        if let Some(slot) = lines.get_mut(*idx) {
            *slot = body.trim().to_string();
        }
    }
    lines.join("\n")
}
