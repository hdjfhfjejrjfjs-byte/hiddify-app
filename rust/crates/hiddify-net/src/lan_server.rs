//! Native LAN HTTP micro-servers replacing `dart:io` `HttpServer` in:
//! - `PhoneImportServer` + `phoneImportPage` (`phone_import_server.dart`, `phone_import_page.dart`)
//! - `LanFileServer` (`lan_file_server.dart`)

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use rand::distributions::Uniform;
use rand::rngs::OsRng;
use rand::{Rng, RngCore};
use serde::{Deserialize, Serialize};

use crate::{
    is_allowed_host_header, is_local_network_bytes, parse_phone_import_form, PhoneImportFormResult,
};

pub const PHONE_IMPORT_TOKEN_LENGTH: usize = 6;
pub const PHONE_IMPORT_MAX_MISSES: usize = 10;
pub const PHONE_IMPORT_MAX_TEXT: usize = 64 * 1024;
pub const PHONE_IMPORT_MAX_PENDING: usize = 20;
const LAN_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const FILE_STREAM_CHUNK: usize = 64 * 1024;

const PATH_SEGMENT_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'/')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn generate_numeric_pin(length: usize) -> String {
    let dist = Uniform::from(0u8..10u8);
    let mut rng = OsRng;
    (0..length)
        .map(|_| (b'0' + rng.sample(dist)) as char)
        .collect()
}

fn generate_url_token(num_bytes: usize) -> String {
    let mut buf = vec![0u8; num_bytes];
    OsRng.fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(&buf)
}

fn ip_is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_local_network_bytes(&v4.octets()),
        IpAddr::V6(v6) => is_local_network_bytes(&v6.octets()),
    }
}

fn format_authority(addr: SocketAddr) -> String {
    addr.to_string()
}

fn next_server_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn wake_listener_to_stop(bind_addr: SocketAddr) {
    let connect_ip = if bind_addr.ip().is_unspecified() {
        if bind_addr.is_ipv6() {
            IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)
        } else {
            IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
        }
    } else {
        bind_addr.ip()
    };
    let _ = TcpStream::connect_timeout(
        &SocketAddr::new(connect_ip, bind_addr.port()),
        Duration::from_millis(80),
    );
}

// ============================================================================
// Minimal HTTP/1.1 Request Parser (Content-Length & Chunked Transfer-Encoding)
// ============================================================================

struct ParsedHttpRequest {
    method: String,
    path: String,
    query: Option<String>,
    host: Option<String>,
    content_type: Option<String>,
    content_length: Option<usize>,
    is_chunked: bool,
}

enum BodyReadOutcome {
    Ok(Vec<u8>),
    TooLarge,
    BadRequest,
}

fn read_http_request_head<R: BufRead>(reader: &mut R) -> io::Result<Option<ParsedHttpRequest>> {
    let mut req_line = String::new();
    let n = reader.read_line(&mut req_line)?;
    if n == 0 {
        return Ok(None);
    }
    let parts: Vec<&str> = req_line.trim_end_matches(&['\r', '\n'][..]).split_whitespace().collect();
    if parts.len() < 2 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "malformed request line"));
    }
    let method = parts[0].to_ascii_uppercase();
    let target = parts[1];
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), Some(q.to_string())),
        None => (target.to_string(), None),
    };

    let mut host = None;
    let mut content_type = None;
    let mut content_length = None;
    let mut is_chunked = false;

    for _ in 0..128 {
        let mut line = String::new();
        let r = reader.read_line(&mut line)?;
        if r == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let name = k.trim();
            let val = v.trim();
            if name.eq_ignore_ascii_case("host") {
                host = Some(val.to_string());
            } else if name.eq_ignore_ascii_case("content-type") {
                let mime = val.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
                content_type = Some(mime);
            } else if name.eq_ignore_ascii_case("content-length") {
                content_length = val.parse::<usize>().ok();
            } else if name.eq_ignore_ascii_case("transfer-encoding")
                && val.to_ascii_lowercase().contains("chunked")
            {
                is_chunked = true;
            }
        }
    }

    Ok(Some(ParsedHttpRequest {
        method,
        path,
        query,
        host,
        content_type,
        content_length,
        is_chunked,
    }))
}

fn read_http_body_bounded<R: BufRead>(
    reader: &mut R,
    req: &ParsedHttpRequest,
    max_bytes: usize,
) -> BodyReadOutcome {
    if let Some(cl) = req.content_length {
        if cl > max_bytes {
            // Drain up to 256 KiB so the client receives our 413 response cleanly without TCP RST
            let drain = (cl - 0).min(256 * 1024) as u64;
            let _ = io::copy(&mut reader.take(drain), &mut io::sink());
            return BodyReadOutcome::TooLarge;
        }
    }

    if req.is_chunked {
        let mut out = Vec::new();
        let mut overflow = false;
        loop {
            let mut size_line = String::new();
            let Ok(n) = reader.read_line(&mut size_line) else {
                return BodyReadOutcome::BadRequest;
            };
            if n == 0 {
                break;
            }
            let hex = size_line.trim().split(';').next().unwrap_or("").trim();
            if hex.is_empty() {
                continue;
            }
            let Ok(chunk_len) = usize::from_str_radix(hex, 16) else {
                return BodyReadOutcome::BadRequest;
            };
            if chunk_len == 0 {
                let mut trailer = String::new();
                let _ = reader.read_line(&mut trailer);
                break;
            }
            if !overflow && out.len() + chunk_len <= max_bytes {
                let start = out.len();
                out.resize(start + chunk_len, 0);
                if reader.read_exact(&mut out[start..start + chunk_len]).is_err() {
                    return BodyReadOutcome::BadRequest;
                }
            } else {
                overflow = true;
                let drain = chunk_len.min(256 * 1024) as u64;
                if io::copy(&mut reader.take(drain), &mut io::sink()).is_err() {
                    break;
                }
            }
            let mut crlf = [0u8; 2];
            let _ = reader.read_exact(&mut crlf);
        }
        if overflow {
            BodyReadOutcome::TooLarge
        } else {
            BodyReadOutcome::Ok(out)
        }
    } else if let Some(len) = req.content_length {
        let mut buf = vec![0u8; len];
        if reader.read_exact(&mut buf).is_err() {
            BodyReadOutcome::BadRequest
        } else {
            BodyReadOutcome::Ok(buf)
        }
    } else {
        let mut buf = Vec::new();
        if reader.take((max_bytes + 1) as u64).read_to_end(&mut buf).is_err() {
            return BodyReadOutcome::BadRequest;
        }
        if buf.len() > max_bytes {
            BodyReadOutcome::TooLarge
        } else {
            BodyReadOutcome::Ok(buf)
        }
    }
}

fn status_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        303 => "See Other",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Request Entity Too Large",
        415 => "Unsupported Media Type",
        429 => "Too Many Requests",
        _ => "Internal Server Error",
    }
}

// ============================================================================
// 1. PhoneImportServer + phoneImportPage (`phone_import_server.dart` & `phone_import_page.dart`)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneImportStepText {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneImportPageText {
    pub lang: String,
    pub rtl: bool,
    pub title: String,
    pub intro: String,
    pub network: PhoneImportStepText,
    pub copy: PhoneImportStepText,
    pub paste: PhoneImportStepText,
    pub confirm: PhoneImportStepText,
    pub label: String,
    pub hint: String,
    pub send: String,
    pub sent_title: String,
    pub sent: String,
    pub empty: String,
    pub footer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneImportEntryEvent {
    pub text: String,
    pub from_ip: String,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneImportStartInfo {
    pub handle: u64,
    pub url: String,
    pub host: String,
    pub port: u16,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneImportPollSnapshot {
    pub locked: bool,
    pub entries: Vec<PhoneImportEntryEvent>,
}

struct PhoneImportServerState {
    closed: AtomicBool,
    locked: AtomicBool,
    misses: AtomicUsize,
    pending: AtomicUsize,
    bind_addr: SocketAddr,
    authority: String,
    secret_path: String,
    text: PhoneImportPageText,
    entries: Mutex<VecDeque<PhoneImportEntryEvent>>,
}

fn phone_servers() -> &'static Mutex<HashMap<u64, Arc<PhoneImportServerState>>> {
    static MAP: OnceLock<Mutex<HashMap<u64, Arc<PhoneImportServerState>>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

fn html_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 16);
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// Renders the exact self-contained HTML guide & form from `phone_import_page.dart`.
pub fn render_phone_import_page(
    t: &PhoneImportPageText,
    action: &str,
    max_text: usize,
    sent: bool,
    error: Option<&str>,
) -> String {
    const LOGO: &str = concat!(
        r##"<svg viewBox="0 0 64 64" aria-hidden="true"><g fill="#fff">"##,
        r#"<path d="M47.1582 14.4283C47.1582 13.8368 47.4684 13.2886 47.9757 12.9842L61.4494 4.89891C62.5721 4.22529 64.0003 5.03389 64.0003 6.34306V20.2103C64.0003 21.1405 63.2461 21.8945 62.3161 21.8945H48.8424C47.9124 21.8945 47.1582 21.1405 47.1582 20.2103V14.4283Z"/>"#,
        r#"<path fill-rule="evenodd" d="M24.3966 26.4593C23.8893 26.7637 23.5789 27.3118 23.5789 27.9034V43.7921L23.5789 45.4751L23.5789 45.4763L23.5789 48.0001C23.5789 48.9301 22.8249 49.6001 21.8947 49.6001H18.5263C17.5962 49.6001 16.8421 48.9301 16.8421 48.0001V45.4751V34.976C16.8421 33.6669 15.4139 32.8583 14.2913 33.5319L0.817607 41.6171C0.310362 41.9216 0 42.4696 0 43.0615V60.6342C0 61.5642 0.754048 62.3184 1.68421 62.3184H15.1579C16.0881 62.3184 16.8421 61.5642 16.8421 60.6342V58.9488V56.8314C16.8421 55.9011 17.5962 55.3376 18.5263 55.3376H21.8947C22.8249 55.3376 23.5789 55.9011 23.5789 56.8314V58.9488L23.5789 60.6342C23.5789 61.5642 24.333 62.3184 25.2632 62.3184H38.7368C39.6669 62.3184 40.4211 61.5642 40.4211 60.6342V45.4763V43.7921V19.8182C40.4211 18.509 38.9928 17.7004 37.8701 18.374L24.3966 26.4593Z"/>"#,
        r#"<path d="M47.1592 28.7999V60.7999C47.1592 61.6836 47.9132 62.3999 48.8434 62.3999H62.3171C63.2472 62.3999 64.0013 61.6836 64.0013 60.7999V28.7999C64.0013 27.9162 63.2472 27.1999 62.3171 27.1999H48.8434C47.9132 27.1999 47.1592 27.9162 47.1592 28.7999Z"/>"#,
        r#"</g></svg>"#
    );
    const ICON_WIFI: &str = r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M1 9l2 2c4.97-4.97 13.03-4.97 18 0l2-2C16.93 2.93 7.08 2.93 1 9zm8 8l3 3 3-3c-1.65-1.66-4.34-1.66-6 0zm-4-4l2 2c2.76-2.76 7.24-2.76 10 0l2-2C15.14 9.14 8.87 9.14 5 13z"/></svg>"#;
    const ICON_LINK: &str = r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3.9 12c0-1.71 1.39-3.1 3.1-3.1h4V7H7c-2.76 0-5 2.24-5 5s2.24 5 5 5h4v-1.9H7c-1.71 0-3.1-1.39-3.1-3.1zM8 13h8v-2H8v2zm9-6h-4v1.9h4c1.71 0 3.1 1.39 3.1 3.1s-1.39 3.1-3.1 3.1h-4V17h4c2.76 0 5-2.24 5-5s-2.24-5-5-5z"/></svg>"#;
    const ICON_PASTE: &str = r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M19 2h-4.18C14.4.84 13.3 0 12 0S9.6.84 9.18 2H5c-1.1 0-2 .9-2 2v16c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zm-7 0c.55 0 1 .45 1 1s-.45 1-1 1-1-.45-1-1 .45-1 1-1zm7 18H5V4h2v3h10V4h2v16z"/></svg>"#;
    const ICON_TV: &str = r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M21 3H3c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h5v2h8v-2h5c1.1 0 1.99-.9 1.99-2L23 5c0-1.1-.9-2-2-2zm0 14H3V5h18v12z"/></svg>"#;
    const ICON_SEND: &str = r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M2.01 21 23 12 2.01 3 2 10l15 2-15 2z"/></svg>"#;
    const ICON_CHECK: &str = r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M9 16.17 4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z"/></svg>"#;
    const ICON_LOCK: &str = r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M18 8h-1V6c0-2.76-2.24-5-5-5S7 3.24 7 6v2H6c-1.1 0-2 .9-2 2v10c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V10c0-1.1-.9-2-2-2zm-6 9c-1.1 0-2-.9-2-2s.9-2 2-2 2 .9 2 2-.9 2-2 2zm3.1-9H8.9V6c0-1.71 1.39-3.1 3.1-3.1 1.71 0 3.1 1.39 3.1 3.1v2z"/></svg>"#;
    const ICON_ERROR: &str = r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm1 15h-2v-2h2v2zm0-4h-2V7h2v6z"/></svg>"#;

    const CSS: &str = r#":root{--brand:#455fe9;--brand2:#7b5cff;--bg:#f3f5fb;--card:#fff;--field:#f7f8fc;--text:#14161f;--muted:#5d6275;--line:#e2e5f0;--ok:#15803d;--okbg:#e7f6ec;--bad:#c62828;--badbg:#fdecec}
@media (prefers-color-scheme:dark){:root{--bg:#0d0e13;--card:#171922;--field:#10121a;--text:#eceef6;--muted:#9aa0b4;--line:#272b38;--ok:#4ade80;--okbg:#10291b;--bad:#f87171;--badbg:#2e1515}}
*{box-sizing:border-box}
html{-webkit-text-size-adjust:100%}
body{margin:0;min-height:100vh;font:16px/1.55 system-ui,-apple-system,"Segoe UI",Roboto,Vazirmatn,Tahoma,sans-serif;color:var(--text);background:radial-gradient(900px 420px at 50% -160px,rgba(69,95,233,.30),transparent 72%),var(--bg)}
.wrap{max-width:560px;margin:0 auto;padding:28px 18px calc(32px + env(safe-area-inset-bottom))}
.brand{display:flex;align-items:center;justify-content:center;gap:10px;margin:4px 0 22px}
.logo{width:46px;height:46px;border-radius:14px;display:grid;place-items:center;background:linear-gradient(135deg,var(--brand),var(--brand2));box-shadow:0 10px 26px rgba(69,95,233,.38)}
.logo svg{width:26px;height:26px}
.word{font-size:22px;font-weight:700;letter-spacing:.2px}
h1{margin:0 0 8px;font-size:clamp(22px,6vw,28px);line-height:1.25;text-align:center}
.intro{max-width:430px;margin:0 auto 24px;color:var(--muted);text-align:center}
.notice{display:flex;align-items:flex-start;gap:12px;margin:0 0 20px;padding:14px 16px;border-radius:18px;font-size:15px}
.notice svg{flex:none;width:24px;height:24px;fill:currentColor}
.notice strong{display:block;margin-bottom:2px;font-size:17px;color:var(--text)}
.ok{background:var(--okbg);color:var(--ok)}
.bad{background:var(--badbg);color:var(--bad)}
.steps{margin:0;padding:0;list-style:none}
.step{position:relative;display:flex;gap:14px;padding-bottom:16px}
.step:not(:last-child)::before{content:"";position:absolute;inset-inline-start:17px;top:42px;bottom:6px;width:2px;border-radius:2px;background:var(--line)}
.num{flex:none;display:grid;place-items:center;width:36px;height:36px;border-radius:50%;font-weight:700;color:#fff;background:linear-gradient(135deg,var(--brand),var(--brand2));box-shadow:0 6px 14px rgba(69,95,233,.32)}
.num svg{width:20px;height:20px;fill:#fff}
.done .num{background:var(--ok);box-shadow:none}
.done .head,.done .body>p{opacity:.7}
.body{flex:1;min-width:0;padding:14px 16px;border:1px solid var(--line);border-radius:18px;background:var(--card)}
.active .body{border-color:var(--brand);box-shadow:0 0 0 4px rgba(69,95,233,.14)}
.head{display:flex;align-items:center;gap:8px}
.head svg{flex:none;width:20px;height:20px;fill:var(--brand)}
h2{margin:0;font-size:17px;line-height:1.3}
.body p{margin:4px 0 0;font-size:15px;color:var(--muted)}
label{display:block;margin:14px 0 6px;font-size:14px;font-weight:600}
textarea{display:block;width:100%;min-height:118px;padding:12px 14px;font:15px/1.45 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;color:var(--text);text-align:left;background:var(--field);border:1.5px solid var(--line);border-radius:14px;resize:vertical}
textarea:focus{outline:none;border-color:var(--brand);box-shadow:0 0 0 4px rgba(69,95,233,.18)}
textarea::placeholder{font-family:system-ui,-apple-system,"Segoe UI",Roboto,Vazirmatn,Tahoma,sans-serif;color:var(--muted);opacity:.8}
[dir=rtl] textarea:placeholder-shown{direction:rtl;text-align:right}
button{display:flex;align-items:center;justify-content:center;gap:10px;width:100%;min-height:54px;margin-top:12px;font-family:inherit;font-size:17px;font-weight:600;color:#fff;border:0;border-radius:14px;background:linear-gradient(135deg,var(--brand),var(--brand2));box-shadow:0 12px 26px rgba(69,95,233,.36);cursor:pointer;-webkit-tap-highlight-color:transparent}
button:active{transform:translateY(1px);box-shadow:0 6px 16px rgba(69,95,233,.3)}
button svg{width:20px;height:20px;fill:#fff}
[dir=rtl] button svg{transform:scaleX(-1)}
.foot{display:flex;align-items:flex-start;justify-content:center;gap:8px;margin:10px auto 0;max-width:430px;font-size:13px;color:var(--muted);text-align:center}
.foot svg{flex:none;width:16px;height:16px;margin-top:2px;fill:currentColor}
@media (min-width:600px){.wrap{padding-top:52px}}
"#;

    let step = |n: usize, s: &PhoneImportStepText, icon: &str, done: bool, active: bool, extra: &str| -> String {
        let mut cls = String::from("step");
        if done {
            cls.push_str(" done");
        }
        if active {
            cls.push_str(" active");
        }
        let num_inner = if done {
            ICON_CHECK.to_string()
        } else {
            n.to_string()
        };
        format!(
            r#"<li class="{cls}"><span class="num">{num_inner}</span><div class="body"><div class="head">{icon}<h2>{}</h2></div><p>{}</p>{extra}</div></li>"#,
            html_escape(&s.title),
            html_escape(&s.body),
        )
    };

    let notice = if let Some(err) = error {
        format!(
            r#"<div class="notice bad" role="alert">{ICON_ERROR}<div>{}</div></div>"#,
            html_escape(err)
        )
    } else if sent {
        format!(
            r#"<div class="notice ok" role="status">{ICON_CHECK}<div><strong>{}</strong>{}</div></div>"#,
            html_escape(&t.sent_title),
            html_escape(&t.sent)
        )
    } else {
        String::new()
    };

    let form = format!(
        r#"<form method="post" action="{}" accept-charset="utf-8"><label for="link">{}</label><textarea id="link" name="link" required maxlength="{max_text}" placeholder="{}" autocapitalize="off" autocomplete="off" autocorrect="off" spellcheck="false" dir="ltr"></textarea><button type="submit">{ICON_SEND}<span>{}</span></button></form>"#,
        html_escape(action),
        html_escape(&t.label),
        html_escape(&t.hint),
        html_escape(&t.send),
    );

    let dir = if t.rtl { "rtl" } else { "ltr" };
    format!(
        concat!(
            r#"<!doctype html><html lang="{}" dir="{dir}"><head>"#,
            r#"<meta charset="utf-8">"#,
            r#"<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">"#,
            r#"<meta name="color-scheme" content="light dark">"#,
            r##"<meta name="theme-color" content="#455FE9">"##,
            r#"<meta name="referrer" content="no-referrer">"#,
            r#"<title>{}</title><style>{CSS}</style></head><body><main class="wrap">"#,
            r#"<header class="brand"><span class="logo">{LOGO}</span><span class="word">Hiddify</span></header>"#,
            r#"<h1>{}</h1><p class="intro">{}</p>"#,
            "{notice}",
            r#"<ol class="steps">{}{}{}{}</ol>"#,
            r#"<p class="foot">{ICON_LOCK}<span>{}</span></p>"#,
            r#"</main></body></html>"#
        ),
        html_escape(&t.lang),
        html_escape(&t.title),
        html_escape(&t.title),
        html_escape(&t.intro),
        step(1, &t.network, ICON_WIFI, sent, false, ""),
        step(2, &t.copy, ICON_LINK, sent, false, ""),
        step(3, &t.paste, ICON_PASTE, sent, !sent, &form),
        step(4, &t.confirm, ICON_TV, false, sent, ""),
        html_escape(&t.footer),
        dir = dir,
        CSS = CSS,
        LOGO = LOGO,
        notice = notice,
        ICON_LOCK = ICON_LOCK,
    )
}

fn write_phone_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    extra_headers: &[(&str, &str)],
    body: &[u8],
    head_only: bool,
) {
    let mut hdr = format!(
        "HTTP/1.1 {status} {}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'\r\n\
         X-Content-Type-Options: nosniff\r\n\
         X-Frame-Options: DENY\r\n\
         Referrer-Policy: no-referrer\r\n\
         Connection: close\r\n",
        status_reason(status),
        body.len(),
    );
    for (k, v) in extra_headers {
        hdr.push_str(&format!("{k}: {v}\r\n"));
    }
    hdr.push_str("\r\n");

    let _ = stream.write_all(hdr.as_bytes());
    if !head_only && !body.is_empty() {
        let _ = stream.write_all(body);
    }
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Write);
}

fn handle_phone_import_conn(mut stream: TcpStream, peer: SocketAddr, state: &PhoneImportServerState) {
    let _ = stream.set_read_timeout(Some(LAN_IDLE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(LAN_IDLE_TIMEOUT));

    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });
    let req = match read_http_request_head(&mut reader) {
        Ok(Some(r)) => r,
        _ => {
            write_phone_response(&mut stream, 400, "text/plain; charset=utf-8", &[], b"Bad request", false);
            return;
        }
    };

    let head_only = req.method == "HEAD";

    if !ip_is_local(peer.ip()) || state.locked.load(Ordering::Acquire) {
        write_phone_response(&mut stream, 404, "text/plain; charset=utf-8", &[], b"Not found", head_only);
        return;
    }

    let valid_host = is_allowed_host_header(req.host.as_deref(), &state.authority);
    let valid_path = ct_eq(req.path.as_bytes(), state.secret_path.as_bytes());
    if !valid_host || !valid_path {
        if req.path != "/favicon.ico" {
            let prev = state.misses.fetch_add(1, Ordering::AcqRel);
            if prev + 1 >= PHONE_IMPORT_MAX_MISSES {
                state.locked.store(true, Ordering::Release);
            }
        }
        write_phone_response(&mut stream, 404, "text/plain; charset=utf-8", &[], b"Not found", head_only);
        return;
    }

    match req.method.as_str() {
        "GET" | "HEAD" => {
            let sent = req
                .query
                .as_deref()
                .map(|q| {
                    q.split('&')
                        .any(|kv| kv.split('=').next() == Some("sent"))
                })
                .unwrap_or(false);
            let html = render_phone_import_page(
                &state.text,
                &state.secret_path,
                PHONE_IMPORT_MAX_TEXT,
                sent,
                None,
            );
            write_phone_response(
                &mut stream,
                200,
                "text/html; charset=utf-8",
                &[],
                html.as_bytes(),
                head_only,
            );
        }
        "POST" => {
            if req.content_type.as_deref() != Some("application/x-www-form-urlencoded") {
                write_phone_response(
                    &mut stream,
                    415,
                    "text/plain; charset=utf-8",
                    &[],
                    b"Unsupported media type",
                    false,
                );
                return;
            }
            let body = match read_http_body_bounded(&mut reader, &req, PHONE_IMPORT_MAX_TEXT * 3) {
                BodyReadOutcome::Ok(b) => b,
                BodyReadOutcome::TooLarge => {
                    write_phone_response(
                        &mut stream,
                        413,
                        "text/plain; charset=utf-8",
                        &[],
                        b"Too large",
                        false,
                    );
                    return;
                }
                BodyReadOutcome::BadRequest => {
                    write_phone_response(
                        &mut stream,
                        400,
                        "text/plain; charset=utf-8",
                        &[],
                        b"Bad request",
                        false,
                    );
                    return;
                }
            };

            match parse_phone_import_form(&body, PHONE_IMPORT_MAX_TEXT) {
                PhoneImportFormResult::BadRequest => {
                    write_phone_response(
                        &mut stream,
                        400,
                        "text/plain; charset=utf-8",
                        &[],
                        b"Bad request",
                        false,
                    );
                }
                PhoneImportFormResult::TooLarge => {
                    write_phone_response(
                        &mut stream,
                        413,
                        "text/plain; charset=utf-8",
                        &[],
                        b"Too large",
                        false,
                    );
                }
                PhoneImportFormResult::Empty => {
                    let html = render_phone_import_page(
                        &state.text,
                        &state.secret_path,
                        PHONE_IMPORT_MAX_TEXT,
                        false,
                        Some(&state.text.empty),
                    );
                    write_phone_response(
                        &mut stream,
                        400,
                        "text/html; charset=utf-8",
                        &[],
                        html.as_bytes(),
                        false,
                    );
                }
                PhoneImportFormResult::Ok(trimmed_text) => {
                    // Atomically reserve a pending slot up to PHONE_IMPORT_MAX_PENDING
                    let mut reserved = false;
                    let mut cur = state.pending.load(Ordering::Acquire);
                    while cur < PHONE_IMPORT_MAX_PENDING {
                        match state.pending.compare_exchange_weak(
                            cur,
                            cur + 1,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        ) {
                            Ok(_) => {
                                reserved = true;
                                break;
                            }
                            Err(actual) => cur = actual,
                        }
                    }
                    if !reserved {
                        write_phone_response(
                            &mut stream,
                            429,
                            "text/plain; charset=utf-8",
                            &[],
                            b"Too many requests",
                            false,
                        );
                        return;
                    }

                    let ts_ms = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);
                    if let Ok(mut q) = state.entries.lock() {
                        q.push_back(PhoneImportEntryEvent {
                            text: trimmed_text,
                            from_ip: peer.ip().to_string(),
                            timestamp_ms: ts_ms,
                        });
                    }
                    let loc = format!("{}?sent", state.secret_path);
                    write_phone_response(
                        &mut stream,
                        303,
                        "text/plain; charset=utf-8",
                        &[("Location", &loc)],
                        b"",
                        false,
                    );
                }
            }
        }
        _ => {
            write_phone_response(
                &mut stream,
                405,
                "text/plain; charset=utf-8",
                &[("Allow", "GET, POST")],
                b"Method not allowed",
                head_only,
            );
        }
    }
}

pub fn phone_import_server_start(
    bind_ip: &str,
    text: PhoneImportPageText,
) -> Result<PhoneImportStartInfo, String> {
    let ip: IpAddr = bind_ip
        .trim()
        .parse()
        .map_err(|e| format!("invalid bind IP {bind_ip}: {e}"))?;
    let listener = TcpListener::bind(SocketAddr::new(ip, 0)).map_err(|e| e.to_string())?;
    let local_addr = listener.local_addr().map_err(|e| e.to_string())?;
    let authority = format_authority(local_addr);
    let token = generate_numeric_pin(PHONE_IMPORT_TOKEN_LENGTH);
    let secret_path = format!("/{token}");
    let url = format!("http://{authority}{secret_path}");
    let host_str = match local_addr.ip() {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => v6.to_string(),
    };

    let handle = next_server_id();
    let state = Arc::new(PhoneImportServerState {
        closed: AtomicBool::new(false),
        locked: AtomicBool::new(false),
        misses: AtomicUsize::new(0),
        pending: AtomicUsize::new(0),
        bind_addr: local_addr,
        authority,
        secret_path,
        text,
        entries: Mutex::new(VecDeque::new()),
    });

    phone_servers()
        .lock()
        .map_err(|_| "lock poisoned".to_string())?
        .insert(handle, state.clone());

    thread::spawn(move || {
        for incoming in listener.incoming() {
            if state.closed.load(Ordering::Acquire) {
                break;
            }
            if let Ok(stream) = incoming {
                if state.closed.load(Ordering::Acquire) {
                    break;
                }
                if let Ok(peer) = stream.peer_addr() {
                    let st = state.clone();
                    thread::spawn(move || handle_phone_import_conn(stream, peer, &st));
                }
            }
        }
    });

    Ok(PhoneImportStartInfo {
        handle,
        url,
        host: host_str,
        port: local_addr.port(),
        token,
    })
}

pub fn phone_import_server_poll(handle: u64) -> Option<PhoneImportPollSnapshot> {
    let state = phone_servers().lock().ok()?.get(&handle)?.clone();
    let locked = state.locked.load(Ordering::Acquire);
    let mut drained = Vec::new();
    if let Ok(mut q) = state.entries.lock() {
        while let Some(ev) = q.pop_front() {
            drained.push(ev);
        }
    }
    Some(PhoneImportPollSnapshot {
        locked,
        entries: drained,
    })
}

pub fn phone_import_server_is_locked(handle: u64) -> bool {
    phone_servers()
        .lock()
        .ok()
        .and_then(|m| m.get(&handle).map(|s| s.locked.load(Ordering::Acquire)))
        .unwrap_or(false)
}

pub fn phone_import_server_handled(handle: u64) -> bool {
    let Some(state) = phone_servers().lock().ok().and_then(|m| m.get(&handle).cloned()) else {
        return false;
    };
    let mut cur = state.pending.load(Ordering::Acquire);
    while cur > 0 {
        match state.pending.compare_exchange_weak(
            cur,
            cur - 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => break,
            Err(actual) => cur = actual,
        }
    }
    true
}

pub fn phone_import_server_close(handle: u64) -> bool {
    let Some(state) = phone_servers().lock().ok().and_then(|mut m| m.remove(&handle)) else {
        return false;
    };
    state.closed.store(true, Ordering::Release);
    wake_listener_to_stop(state.bind_addr);
    true
}

// ============================================================================
// 2. LanFileServer (`lan_file_server.dart`)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanFileStartInfo {
    pub handle: u64,
    pub url: String,
    pub host: String,
    pub port: u16,
    pub token: String,
}

struct LanFileServerState {
    closed: AtomicBool,
    bind_addr: SocketAddr,
    authority: String,
    token: String,
    file_path: PathBuf,
    file_name: String,
    downloads: Mutex<VecDeque<String>>,
}

fn lan_file_servers() -> &'static Mutex<HashMap<u64, Arc<LanFileServerState>>> {
    static MAP: OnceLock<Mutex<HashMap<u64, Arc<LanFileServerState>>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

fn file_content_type(file_name: &str) -> &'static str {
    let ext = Path::new(file_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "zip" => "application/zip",
        "txt" | "log" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn write_lan_file_plain(
    stream: &mut TcpStream,
    status: u16,
    extra_headers: &[(&str, &str)],
    message: &[u8],
    head_only: bool,
) {
    let mut hdr = format!(
        "HTTP/1.1 {status} {}\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Referrer-Policy: no-referrer\r\n\
         Connection: close\r\n",
        status_reason(status),
        message.len(),
    );
    for (k, v) in extra_headers {
        hdr.push_str(&format!("{k}: {v}\r\n"));
    }
    hdr.push_str("\r\n");

    let _ = stream.write_all(hdr.as_bytes());
    if !head_only && !message.is_empty() {
        let _ = stream.write_all(message);
    }
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Write);
}

fn handle_lan_file_conn(mut stream: TcpStream, peer: SocketAddr, state: &LanFileServerState) {
    let _ = stream.set_read_timeout(Some(LAN_IDLE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(LAN_IDLE_TIMEOUT));

    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });
    let Ok(Some(req)) = read_http_request_head(&mut reader) else {
        write_lan_file_plain(&mut stream, 500, &[], b"Error", false);
        return;
    };

    let head_only = req.method == "HEAD";
    let first_seg = req
        .path
        .trim_start_matches('/')
        .split('/')
        .find(|s| !s.is_empty())
        .unwrap_or("");

    if !ip_is_local(peer.ip())
        || !is_allowed_host_header(req.host.as_deref(), &state.authority)
        || first_seg.is_empty()
        || !ct_eq(first_seg.as_bytes(), state.token.as_bytes())
    {
        write_lan_file_plain(&mut stream, 404, &[], b"Not found", head_only);
        return;
    }

    if req.method != "GET" && req.method != "HEAD" {
        write_lan_file_plain(
            &mut stream,
            405,
            &[("Allow", "GET, HEAD")],
            b"Method not allowed",
            head_only,
        );
        return;
    }

    let Ok(mut file) = File::open(&state.file_path) else {
        write_lan_file_plain(&mut stream, 500, &[], b"Error", head_only);
        return;
    };
    let Ok(meta) = file.metadata() else {
        write_lan_file_plain(&mut stream, 500, &[], b"Error", head_only);
        return;
    };

    let safe_name = state.file_name.replace('"', "");
    let ctype = file_content_type(&state.file_name);
    let hdr = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: {ctype}\r\n\
         Content-Length: {}\r\n\
         Content-Disposition: attachment; filename=\"{safe_name}\"\r\n\
         Cache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Referrer-Policy: no-referrer\r\n\
         Connection: close\r\n\r\n",
        meta.len(),
    );

    if stream.write_all(hdr.as_bytes()).is_err() {
        return;
    }
    if head_only {
        let _ = stream.flush();
        let _ = stream.shutdown(Shutdown::Write);
        return;
    }

    let mut buf = vec![0u8; FILE_STREAM_CHUNK];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if stream.write_all(&buf[..n]).is_err() {
                    return;
                }
            }
            Err(_) => return,
        }
    }
    if stream.flush().is_ok() {
        let _ = stream.shutdown(Shutdown::Write);
        if let Ok(mut q) = state.downloads.lock() {
            q.push_back(peer.ip().to_string());
        }
    }
}

pub fn lan_file_server_start(bind_ip: &str, file_path: &str) -> Result<LanFileStartInfo, String> {
    let ip: IpAddr = bind_ip
        .trim()
        .parse()
        .map_err(|e| format!("invalid bind IP {bind_ip}: {e}"))?;
    let path = PathBuf::from(file_path);
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .unwrap_or("download.bin")
        .to_string();

    let listener = TcpListener::bind(SocketAddr::new(ip, 0)).map_err(|e| e.to_string())?;
    let local_addr = listener.local_addr().map_err(|e| e.to_string())?;
    let authority = format_authority(local_addr);
    let token = generate_url_token(18);
    let encoded_name = utf8_percent_encode(&file_name, PATH_SEGMENT_ENCODE_SET).to_string();
    let url = format!("http://{authority}/{token}/{encoded_name}");
    let host_str = match local_addr.ip() {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => v6.to_string(),
    };

    let handle = next_server_id();
    let state = Arc::new(LanFileServerState {
        closed: AtomicBool::new(false),
        bind_addr: local_addr,
        authority,
        token: token.clone(),
        file_path: path,
        file_name,
        downloads: Mutex::new(VecDeque::new()),
    });

    lan_file_servers()
        .lock()
        .map_err(|_| "lock poisoned".to_string())?
        .insert(handle, state.clone());

    thread::spawn(move || {
        for incoming in listener.incoming() {
            if state.closed.load(Ordering::Acquire) {
                break;
            }
            if let Ok(stream) = incoming {
                if state.closed.load(Ordering::Acquire) {
                    break;
                }
                if let Ok(peer) = stream.peer_addr() {
                    let st = state.clone();
                    thread::spawn(move || handle_lan_file_conn(stream, peer, &st));
                }
            }
        }
    });

    Ok(LanFileStartInfo {
        handle,
        url,
        host: host_str,
        port: local_addr.port(),
        token,
    })
}

pub fn lan_file_server_poll_downloads(handle: u64) -> Option<Vec<String>> {
    let state = lan_file_servers().lock().ok()?.get(&handle)?.clone();
    let mut out = Vec::new();
    if let Ok(mut q) = state.downloads.lock() {
        while let Some(ip) = q.pop_front() {
            out.push(ip);
        }
    }
    Some(out)
}

pub fn lan_file_server_close(handle: u64) -> bool {
    let Some(state) = lan_file_servers().lock().ok().and_then(|mut m| m.remove(&handle)) else {
        return false;
    };
    state.closed.store(true, Ordering::Release);
    wake_listener_to_stop(state.bind_addr);
    true
}

// ============================================================================
// 3. Phone-to-TV Pairing Client (`send_to_tv.dart`)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SendToTvOutcome {
    Sent,
    NotTvCode,
    Expired,
    Busy,
    Unreachable,
}

impl SendToTvOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sent => "sent",
            Self::NotTvCode => "notTvCode",
            Self::Expired => "expired",
            Self::Busy => "busy",
            Self::Unreachable => "unreachable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TvImportTarget {
    pub addr: SocketAddr,
    pub authority: String,
    pub path: String,
    pub normalized_url: String,
}

const FORM_VALUE_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

/// Validates a QR code scanned from a TV's add-profile sheet (`parseTvImportCode` in `send_to_tv.dart`):
/// - `http://` scheme only, explicit `:port` required, no `userInfo`, no `?query`, no `#fragment`
/// - Literal IPv4 or `[IPv6]` address belonging to a local network (`is_local_network_bytes`)
/// - Exact `/<6 digits>` secret path (`PHONE_IMPORT_TOKEN_LENGTH`).
pub fn parse_tv_import_code(scanned: &str) -> Option<TvImportTarget> {
    let trimmed = scanned.trim();
    if trimmed.contains('?') || trimmed.contains('#') {
        return None;
    }
    if trimmed.len() <= 7 || !trimmed[..7].eq_ignore_ascii_case("http://") {
        return None;
    }
    let after_scheme = &trimmed[7..];
    let (authority_raw, path_no_slash) = after_scheme.split_once('/')?;
    if authority_raw.is_empty() || authority_raw.contains('@') {
        return None;
    }

    let (ip, port_str) = if let Some(rest_v6) = authority_raw.strip_prefix('[') {
        let (v6_str, after_bracket) = rest_v6.split_once(']')?;
        let p_str = after_bracket.strip_prefix(':')?;
        let v6: std::net::Ipv6Addr = v6_str.parse().ok()?;
        (IpAddr::V6(v6), p_str)
    } else {
        let (v4_str, p_str) = authority_raw.split_once(':')?;
        if !crate::is_valid_dart_ipv4(v4_str) {
            return None;
        }
        let v4: std::net::Ipv4Addr = v4_str.parse().ok()?;
        (IpAddr::V4(v4), p_str)
    };

    if port_str.is_empty() || !port_str.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let port: u16 = port_str.parse().ok()?;
    if port == 0 || !ip_is_local(ip) {
        return None;
    }

    if path_no_slash.len() != PHONE_IMPORT_TOKEN_LENGTH
        || !path_no_slash.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }

    let addr = SocketAddr::new(ip, port);
    let authority = format_authority(addr);
    let path = format!("/{path_no_slash}");
    let normalized_url = format!("http://{authority}{path}");
    Some(TvImportTarget {
        addr,
        authority,
        path,
        normalized_url,
    })
}

/// Sends `link` directly to the TV pairing server identified by `scanned` (`sendLinkToTv` in `send_to_tv.dart`).
pub fn send_link_to_tv(scanned: &str, link: &str) -> SendToTvOutcome {
    let Some(target) = parse_tv_import_code(scanned) else {
        return SendToTvOutcome::NotTvCode;
    };

    let mut stream = match TcpStream::connect_timeout(&target.addr, Duration::from_secs(8)) {
        Ok(s) => s,
        Err(_) => return SendToTvOutcome::Unreachable,
    };
    let io_timeout = Some(Duration::from_secs(10));
    let _ = stream.set_read_timeout(io_timeout);
    let _ = stream.set_write_timeout(io_timeout);
    let _ = stream.set_nodelay(true);

    let encoded_link = utf8_percent_encode(link, FORM_VALUE_ENCODE_SET).to_string();
    let body = format!("link={encoded_link}");
    let req = format!(
        "POST {} HTTP/1.1\r\n\
         Host: {}\r\n\
         Content-Type: application/x-www-form-urlencoded; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n\
         {body}",
        target.path,
        target.authority,
        body.len(),
    );

    if stream.write_all(req.as_bytes()).is_err() || stream.flush().is_err() {
        return SendToTvOutcome::Unreachable;
    }

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    if reader.read_line(&mut status_line).unwrap_or(0) == 0 {
        return SendToTvOutcome::Unreachable;
    }
    let Some(code) = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
    else {
        return SendToTvOutcome::Unreachable;
    };

    // Drain remainder of response cleanly
    let _ = io::copy(&mut reader, &mut io::sink());

    match code {
        200 | 303 => SendToTvOutcome::Sent,
        429 => SendToTvOutcome::Busy,
        404 => SendToTvOutcome::Expired,
        _ => SendToTvOutcome::Unreachable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_text() -> PhoneImportPageText {
        PhoneImportPageText {
            lang: "en".to_string(),
            rtl: false,
            title: "Send <to> TV".to_string(),
            intro: "Four steps".to_string(),
            network: PhoneImportStepText {
                title: "Same Wi-Fi".to_string(),
                body: "Join it".to_string(),
            },
            copy: PhoneImportStepText {
                title: "Copy".to_string(),
                body: "Copy the link".to_string(),
            },
            paste: PhoneImportStepText {
                title: "Paste".to_string(),
                body: "Paste it".to_string(),
            },
            confirm: PhoneImportStepText {
                title: "Confirm".to_string(),
                body: "Press Import".to_string(),
            },
            label: "Link".to_string(),
            hint: "Paste".to_string(),
            send: "Send".to_string(),
            sent_title: "Sent to TV".to_string(),
            sent: "Press Import on the TV".to_string(),
            empty: "Paste a link first".to_string(),
            footer: "Local only".to_string(),
        }
    }

    fn raw_http(port: u16, raw_req: &str) -> (u16, String, Vec<u8>) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(raw_req.as_bytes()).unwrap();
        s.flush().unwrap();
        let mut resp = Vec::new();
        s.read_to_end(&mut resp).unwrap();
        let split = resp
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("header end");
        let head = String::from_utf8_lossy(&resp[..split]).into_owned();
        let code: u16 = head
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        (code, head, resp[split + 4..].to_vec())
    }

    #[test]
    fn phone_import_and_lan_file_servers_enforce_all_security_rules() {
        let info = phone_import_server_start("127.0.0.1", sample_text()).unwrap();
        assert_eq!(info.token.len(), 6);
        let auth = format!("127.0.0.1:{}", info.port);

        // GET page at secret path -> 200 OK with escaped HTML and CSP
        let (code, head, body) = raw_http(
            info.port,
            &format!("GET /{} HTTP/1.1\r\nHost: {auth}\r\n\r\n", info.token),
        );
        assert_eq!(code, 200);
        assert!(head.to_ascii_lowercase().contains("content-security-policy:"));
        let html = String::from_utf8(body).unwrap();
        assert!(html.contains("Send &lt;to&gt; TV"));
        assert!(!html.contains("<script"));

        // POST chunked form -> 303 See Other + entry queued
        let chunk_payload = "link=+https%3A%2F%2Fexample.com%2Fsub+";
        let chunked_req = format!(
            "POST /{} HTTP/1.1\r\nHost: {auth}\r\nContent-Type: application/x-www-form-urlencoded\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{chunk_payload}\r\n0\r\n\r\n",
            info.token,
            chunk_payload.len()
        );
        let (pcode, phead, _) = raw_http(info.port, &chunked_req);
        assert_eq!(pcode, 303);
        assert!(phead.contains(&format!("Location: /{}?sent", info.token)));
        let poll = phone_import_server_poll(info.handle).unwrap();
        assert_eq!(poll.entries.len(), 1);
        assert_eq!(poll.entries[0].text, "https://example.com/sub");
        assert!(phone_import_server_handled(info.handle));

        // 10 misses lock the server permanently (favicon.ico does not count)
        let _ = raw_http(
            info.port,
            &format!("GET /favicon.ico HTTP/1.1\r\nHost: {auth}\r\n\r\n"),
        );
        for i in 0..9 {
            let (c, _, _) = raw_http(
                info.port,
                &format!("GET /wrong{i} HTTP/1.1\r\nHost: {auth}\r\n\r\n"),
            );
            assert_eq!(c, 404);
        }
        assert!(!phone_import_server_is_locked(info.handle));
        // Wrong Host header counts as the 10th miss -> locks!
        let (c10, _, _) = raw_http(
            info.port,
            &format!("GET /{} HTTP/1.1\r\nHost: evil.example\r\n\r\n", info.token),
        );
        assert_eq!(c10, 404);
        assert!(phone_import_server_is_locked(info.handle));
        let (after_lock, _, _) = raw_http(
            info.port,
            &format!("GET /{} HTTP/1.1\r\nHost: {auth}\r\n\r\n", info.token),
        );
        assert_eq!(after_lock, 404);
        assert!(phone_import_server_close(info.handle));

        // LanFileServer test
        let dir = tempfile::tempdir().unwrap();
        let fpath = dir.path().join("diagnostics.zip");
        let expected: Vec<u8> = (0..100_000).map(|i| (i % 256) as u8).collect();
        std::fs::write(&fpath, &expected).unwrap();

        let finfo = lan_file_server_start("127.0.0.1", &fpath.to_string_lossy()).unwrap();
        let fauth = format!("127.0.0.1:{}", finfo.port);
        let (fc, fhead, fbody) = raw_http(
            finfo.port,
            &format!(
                "GET /{}/diagnostics.zip HTTP/1.1\r\nHost: {fauth}\r\n\r\n",
                finfo.token
            ),
        );
        assert_eq!(fc, 200);
        assert!(fhead.contains("Content-Disposition: attachment; filename=\"diagnostics.zip\""));
        assert_eq!(fbody, expected);
        let dl = lan_file_server_poll_downloads(finfo.handle).unwrap();
        assert_eq!(dl, vec!["127.0.0.1".to_string()]);
        assert!(lan_file_server_close(finfo.handle));

        // Phone-to-TV pairing client tests (`send_to_tv_test.dart` parity)
        assert!(parse_tv_import_code("http://192.168.1.20:41234/123456").is_some());
        assert!(parse_tv_import_code(" http://10.0.0.5:8080/000000\n").is_some());
        assert!(parse_tv_import_code("http://[fd00::1]:8080/123456").is_some());
        for bad in [
            "vless://uuid@1.2.3.4:443",
            "https://example.com/sub",
            "http://8.8.8.8:80/123456",
            "http://tv.local:8080/123456",
            "http://192.168.1.20/123456",
            "http://192.168.1.20:8080/12345",
            "http://192.168.1.20:8080/1234567",
            "http://192.168.1.20:8080/abcdef",
            "http://192.168.1.20:8080/123456?x=1",
            "http://u:p@192.168.1.20:8080/123456",
            "https://192.168.1.20:8080/123456",
        ] {
            assert!(parse_tv_import_code(bad).is_none(), "expected None for {bad}");
        }

        let tv = phone_import_server_start("127.0.0.1", sample_text()).unwrap();
        let link = "https://sub.example.com/abc?name=My Sub#Mine";
        assert_eq!(send_link_to_tv(&tv.url, link), SendToTvOutcome::Sent);
        let tv_poll = phone_import_server_poll(tv.handle).unwrap();
        assert_eq!(tv_poll.entries.len(), 1);
        assert_eq!(tv_poll.entries[0].text, link);
        assert_eq!(
            send_link_to_tv("https://example.com", "x"),
            SendToTvOutcome::NotTvCode
        );
        let stale_tok = if tv.token == "000000" { "111111" } else { "000000" };
        assert_eq!(
            send_link_to_tv(&format!("http://127.0.0.1:{}/{stale_tok}", tv.port), "x"),
            SendToTvOutcome::Expired
        );
        assert!(phone_import_server_close(tv.handle));
    }
}
