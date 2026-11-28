//! Chrome-like HTTPS/HTTP subscription downloader (`v2/hcore/fetch_url.go`).
//!
//! Features:
//! - Trusts system roots (`rustls-native-certs`) + bundled Mozilla roots (`webpki-roots`) so older
//!   systems (e.g. Android 7.0 missing ISRG Root X1) verify modern Let's Encrypt chains.
//! - Negotiates ALPN (`h2` or `http/1.1`), follows HTTP redirects, requests & transparently
//!   decompresses `gzip` bodies (stripping `content-encoding` and `content-length`), and enforces
//!   the 16 MiB body cap (`FETCH_URL_ERROR_TOO_LARGE`).
//! - Classifies failures into exact `FetchUrlError` codes (`TIMEOUT=1`, `DNS=2`, `REFUSED=3`,
//!   `RESET=4`, `UNREACHABLE=5`, `TLS=6`, `CERTIFICATE=7`, `OTHER=8`, `TOO_LARGE=9`) while stripping
//!   the request URL from `error_detail` so secret tokens never leak into logs.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use flate2::read::GzDecoder;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::cancel::is_cancelled;
use crate::proxy_dial::{dial_http_proxy_connect, LocalHttpProxySpec};

pub const DEFAULT_FETCH_TIMEOUT: Duration = Duration::from_secs(30);
pub const DIAL_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const FETCH_URL_BODY_MAX: usize = 16 << 20; // 16 MiB
const MAX_REDIRECTS: usize = 10;

/// Exact integer values matching `hcore.FetchUrlError` in `v2/hcore/hcore.pb.go`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i32)]
pub enum FetchUrlError {
    None = 0,
    Timeout = 1,
    Dns = 2,
    Refused = 3,
    Reset = 4,
    Unreachable = 5,
    Tls = 6,
    Certificate = 7,
    Other = 8,
    TooLarge = 9,
}

impl FetchUrlError {
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchHttpHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchUrlRequest {
    pub url: String,
    pub user_agent: String,
    pub timeout_ms: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchUrlOutcome {
    pub status_code: i32,
    pub headers: Vec<FetchHttpHeader>,
    pub body: Vec<u8>,
    pub error: FetchUrlError,
    pub error_detail: String,
}

impl FetchUrlOutcome {
    fn from_error(error: FetchUrlError, detail: impl Into<String>) -> Self {
        Self {
            status_code: 0,
            headers: Vec::new(),
            body: Vec::new(),
            error,
            error_detail: detail.into(),
        }
    }
}

#[derive(Debug)]
enum StageError {
    Dns(String),
    Dial(io::Error),
    TlsHandshakeIo(io::Error),
    TlsCertificate(String),
    TlsHandshake(String),
    HttpIo(io::Error),
    Protocol(String),
    DeadlineExceeded,
    Cancelled,
}

pub(crate) fn system_and_bundled_roots() -> Arc<RootCertStore> {
    static ROOTS: OnceLock<Arc<RootCertStore>> = OnceLock::new();
    ROOTS
        .get_or_init(|| {
            let mut store = RootCertStore::empty();
            store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let native = rustls_native_certs::load_native_certs();
            for cert in native.certs {
                let _ = store.add(cert);
            }
            Arc::new(store)
        })
        .clone()
}

pub(crate) fn build_tls_config(custom_roots: Option<Arc<RootCertStore>>) -> Result<Arc<ClientConfig>, String> {
    let roots = custom_roots.unwrap_or_else(system_and_bundled_roots);
    let provider = Arc::new(rustls_rustcrypto::provider());
    let mut cfg = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth();
    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(Arc::new(cfg))
}

/// Executes `FetchUrl` with system + bundled Mozilla root certificates.
/// Returns `Err(msg)` only for `codes.InvalidArgument` (invalid URL or unsupported scheme),
/// matching `(*CoreService).FetchUrl` in `v2/hcore/fetch_url.go`.
pub fn fetch_url(req: &FetchUrlRequest) -> Result<FetchUrlOutcome, String> {
    fetch_url_with_roots(req, None)
}

pub fn fetch_url_with_roots(
    req: &FetchUrlRequest,
    custom_roots: Option<Arc<RootCertStore>>,
) -> Result<FetchUrlOutcome, String> {
    fetch_url_ext(req, None, false, 0, custom_roots)
}

/// Extended `fetch_url` supporting optional local HTTP mixed proxy (`"PROXY 127.0.0.1:<port>; DIRECT"`)
/// and cooperative task cancellation (`cancel_task_id`).
/// Returns `Err("cancelled")` if `cancel_task_id` is cancelled before or during the transfer.
pub fn fetch_url_ext(
    req: &FetchUrlRequest,
    proxy: Option<&LocalHttpProxySpec>,
    fallback_direct: bool,
    cancel_task_id: u64,
    custom_roots: Option<Arc<RootCertStore>>,
) -> Result<FetchUrlOutcome, String> {
    if is_cancelled(cancel_task_id) {
        return Err("cancelled".to_string());
    }
    let initial = Url::parse(&req.url).map_err(|e| format!("parse url: {e}"))?;
    match initial.scheme() {
        "http" | "https" => {}
        other => return Err(format!("unsupported scheme {other:?}")),
    }

    let timeout = if req.timeout_ms > 0 {
        Duration::from_millis(req.timeout_ms as u64)
    } else {
        DEFAULT_FETCH_TIMEOUT
    };
    let deadline = Instant::now() + timeout;
    let tls_config = build_tls_config(custom_roots)?;

    let mut current_url = initial;
    for _hop in 0..=MAX_REDIRECTS {
        if is_cancelled(cancel_task_id) {
            return Err("cancelled".to_string());
        }
        match round_trip_with_proxy(
            &current_url,
            &req.user_agent,
            deadline,
            &tls_config,
            proxy,
            fallback_direct,
            cancel_task_id,
        ) {
            Ok((status_code, headers, body)) => {
                if is_cancelled(cancel_task_id) {
                    return Err("cancelled".to_string());
                }
                if matches!(status_code, 301 | 302 | 303 | 307 | 308) {
                    if let Some(loc) = headers
                        .iter()
                        .find(|h| h.name.eq_ignore_ascii_case("location"))
                        .map(|h| h.value.as_str())
                    {
                        if let Ok(next_url) = current_url.join(loc) {
                            if matches!(next_url.scheme(), "http" | "https") {
                                current_url = next_url;
                                continue;
                            }
                        }
                    }
                }
                if body.len() > FETCH_URL_BODY_MAX {
                    return Ok(FetchUrlOutcome::from_error(
                        FetchUrlError::TooLarge,
                        format!("body over {FETCH_URL_BODY_MAX} bytes"),
                    ));
                }
                return Ok(FetchUrlOutcome {
                    status_code,
                    headers,
                    body,
                    error: FetchUrlError::None,
                    error_detail: String::new(),
                });
            }
            Err(StageError::Cancelled) => return Err("cancelled".to_string()),
            Err(stage_err) => {
                if is_cancelled(cancel_task_id) {
                    return Err("cancelled".to_string());
                }
                let (code, detail) = classify_stage_error(stage_err);
                return Ok(FetchUrlOutcome::from_error(code, detail));
            }
        }
    }

    Ok(FetchUrlOutcome::from_error(
        FetchUrlError::Other,
        "stopped after 10 redirects",
    ))
}

fn remaining_timeout(deadline: Instant) -> Result<Duration, StageError> {
    let now = Instant::now();
    if now >= deadline {
        Err(StageError::DeadlineExceeded)
    } else {
        Ok(deadline - now)
    }
}

fn resolve_host_addrs(host: &str, port: u16) -> Result<Vec<SocketAddr>, StageError> {
    if host.eq_ignore_ascii_case("no-such-host.invalid") || host.ends_with(".invalid") {
        return Err(StageError::Dns(format!("lookup {host}: no such host")));
    }
    match (host, port).to_socket_addrs() {
        Ok(iter) => {
            let addrs: Vec<SocketAddr> = iter.collect();
            if addrs.is_empty() {
                Err(StageError::Dns(format!("lookup {host}: no such host")))
            } else {
                Ok(addrs)
            }
        }
        Err(e) => Err(StageError::Dns(format!("lookup {host}: {e}"))),
    }
}

fn dial_tcp(addrs: &[SocketAddr], deadline: Instant) -> Result<TcpStream, StageError> {
    let mut last_err: Option<io::Error> = None;
    for &addr in addrs {
        let rem = remaining_timeout(deadline)?;
        let connect_timeout = rem.min(DIAL_CONNECT_TIMEOUT);
        match TcpStream::connect_timeout(&addr, connect_timeout) {
            Ok(stream) => {
                let rw_timeout = remaining_timeout(deadline)?;
                let _ = stream.set_read_timeout(Some(rw_timeout));
                let _ = stream.set_write_timeout(Some(rw_timeout));
                let _ = stream.set_nodelay(true);
                return Ok(stream);
            }
            Err(e) => {
                last_err = Some(e);
            }
        }
    }
    Err(StageError::Dial(last_err.unwrap_or_else(|| {
        io::Error::new(io::ErrorKind::NotConnected, "connect failed")
    })))
}

fn round_trip_with_proxy(
    url: &Url,
    user_agent: &str,
    deadline: Instant,
    tls_config: &Arc<ClientConfig>,
    proxy: Option<&LocalHttpProxySpec>,
    fallback_direct: bool,
    cancel_task_id: u64,
) -> Result<(i32, Vec<FetchHttpHeader>, Vec<u8>), StageError> {
    if is_cancelled(cancel_task_id) {
        return Err(StageError::Cancelled);
    }
    if let Some(p) = proxy.filter(|p| p.is_enabled()) {
        match round_trip_via_http_proxy(url, user_agent, deadline, tls_config, p, cancel_task_id) {
            Ok(res) => return Ok(res),
            Err(StageError::Cancelled) => return Err(StageError::Cancelled),
            Err(_proxy_err) if fallback_direct => {
                // Local proxy not listening or refused -> fall through to DIRECT ("PROXY 127.0.0.1:port; DIRECT")
            }
            Err(e) => return Err(e),
        }
    }
    round_trip_once(url, user_agent, deadline, tls_config, cancel_task_id)
}

fn round_trip_via_http_proxy(
    url: &Url,
    user_agent: &str,
    deadline: Instant,
    tls_config: &Arc<ClientConfig>,
    proxy: &LocalHttpProxySpec,
    cancel_task_id: u64,
) -> Result<(i32, Vec<FetchHttpHeader>, Vec<u8>), StageError> {
    let host = url
        .host_str()
        .ok_or_else(|| StageError::Protocol("missing host".to_string()))?;
    let port = url
        .port_or_known_default()
        .unwrap_or(if url.scheme() == "https" { 443 } else { 80 });
    let path_and_query = match url.query() {
        Some(q) => format!("{}?{q}", url.path()),
        None => {
            let p = url.path();
            if p.is_empty() {
                "/".to_string()
            } else {
                p.to_string()
            }
        }
    };
    let authority = match url.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.to_string(),
    };

    if url.scheme() == "https" {
        let tcp = dial_http_proxy_connect(proxy, host, port, user_agent, deadline)
            .map_err(StageError::Dial)?;
        return finish_tls_round_trip(
            tcp,
            host,
            &authority,
            &path_and_query,
            user_agent,
            deadline,
            tls_config,
            cancel_task_id,
        );
    }

    // Plain HTTP via HTTP proxy: send full absolute-form request target + optional Proxy-Authorization
    let mut tcp = crate::proxy_dial::connect_tcp_with_deadline("127.0.0.1", proxy.port, deadline)
        .map_err(StageError::Dial)?;
    exchange_http11_ext(
        &mut tcp,
        &authority,
        url.as_str(),
        user_agent,
        proxy.proxy_authorization_header().as_deref(),
        deadline,
        cancel_task_id,
    )
}

fn round_trip_once(
    url: &Url,
    user_agent: &str,
    deadline: Instant,
    tls_config: &Arc<ClientConfig>,
    cancel_task_id: u64,
) -> Result<(i32, Vec<FetchHttpHeader>, Vec<u8>), StageError> {
    if is_cancelled(cancel_task_id) {
        return Err(StageError::Cancelled);
    }
    let host = url
        .host_str()
        .ok_or_else(|| StageError::Protocol("missing host".to_string()))?;
    let port = url
        .port_or_known_default()
        .unwrap_or(if url.scheme() == "https" { 443 } else { 80 });
    let path_and_query = match url.query() {
        Some(q) => format!("{}?{q}", url.path()),
        None => {
            let p = url.path();
            if p.is_empty() {
                "/".to_string()
            } else {
                p.to_string()
            }
        }
    };
    let authority = match url.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.to_string(),
    };

    let addrs = resolve_host_addrs(host, port)?;
    if is_cancelled(cancel_task_id) {
        return Err(StageError::Cancelled);
    }
    let mut tcp = dial_tcp(&addrs, deadline)?;

    if url.scheme() == "https" {
        return finish_tls_round_trip(
            tcp,
            host,
            &authority,
            &path_and_query,
            user_agent,
            deadline,
            tls_config,
            cancel_task_id,
        );
    }

    exchange_http11_ext(
        &mut tcp,
        &authority,
        &path_and_query,
        user_agent,
        None,
        deadline,
        cancel_task_id,
    )
}

fn finish_tls_round_trip(
    mut tcp: TcpStream,
    host: &str,
    authority: &str,
    path_and_query: &str,
    user_agent: &str,
    deadline: Instant,
    tls_config: &Arc<ClientConfig>,
    cancel_task_id: u64,
) -> Result<(i32, Vec<FetchHttpHeader>, Vec<u8>), StageError> {
    let clean_host = host.trim_start_matches('[').trim_end_matches(']');
    let server_name = ServerName::try_from(clean_host.to_string())
        .map_err(|e| StageError::TlsHandshake(format!("tls handshake: {e}")))?;
    let mut conn = ClientConnection::new(tls_config.clone(), server_name)
        .map_err(|e| StageError::TlsHandshake(format!("tls handshake: {e}")))?;

    while conn.is_handshaking() {
        if is_cancelled(cancel_task_id) {
            return Err(StageError::Cancelled);
        }
        let rem = remaining_timeout(deadline)?;
        let _ = tcp.set_read_timeout(Some(rem));
        let _ = tcp.set_write_timeout(Some(rem));
        match conn.complete_io(&mut tcp) {
            Ok(_) => {}
            Err(io_err) => {
                return Err(StageError::TlsHandshakeIo(io_err));
            }
        }
    }
    if conn.wants_write() {
        let _ = conn.complete_io(&mut tcp);
    }

    let negotiated_alpn = conn.alpn_protocol().map(|p| p.to_vec());
    let mut tls_stream = StreamOwned::new(conn, tcp);
    if negotiated_alpn.as_deref() == Some(b"h2") {
        return exchange_h2(
            &mut tls_stream,
            "https",
            authority,
            path_and_query,
            user_agent,
            deadline,
        );
    }
    exchange_http11_ext(
        &mut tls_stream,
        authority,
        path_and_query,
        user_agent,
        None,
        deadline,
        cancel_task_id,
    )
}

fn map_stream_io_error(err: io::Error, in_handshake: bool) -> StageError {
    let text = err.to_string();
    let lower = text.to_ascii_lowercase();
    if is_cert_error_message(&lower) {
        return StageError::TlsCertificate(format!("tls handshake: {text}"));
    }
    if in_handshake {
        StageError::TlsHandshakeIo(err)
    } else {
        StageError::HttpIo(err)
    }
}

fn is_cert_error_message(lower: &str) -> bool {
    lower.contains("invalid peer certificate")
        || lower.contains("unknownissuer")
        || lower.contains("unknown issuer")
        || lower.contains("notvalidforname")
        || lower.contains("certificate")
        || lower.contains("badsignature")
        || lower.contains("expired")
        || lower.contains("untrusted")
}

// ============================================================================
// HTTP/1.1 Exchange (with gzip transparent decompression)
// ============================================================================

fn exchange_http11_ext<S: Read + Write>(
    stream: &mut S,
    authority: &str,
    request_target: &str,
    user_agent: &str,
    proxy_auth: Option<&str>,
    deadline: Instant,
    cancel_task_id: u64,
) -> Result<(i32, Vec<FetchHttpHeader>, Vec<u8>), StageError> {
    if is_cancelled(cancel_task_id) {
        return Err(StageError::Cancelled);
    }
    let _ = remaining_timeout(deadline)?;
    let mut req_buf = String::with_capacity(256);
    req_buf.push_str(&format!("GET {request_target} HTTP/1.1\r\n"));
    req_buf.push_str(&format!("Host: {authority}\r\n"));
    if !user_agent.is_empty() {
        req_buf.push_str(&format!("User-Agent: {user_agent}\r\n"));
    }
    if let Some(auth) = proxy_auth {
        req_buf.push_str(&format!("Proxy-Authorization: {auth}\r\n"));
    }
    req_buf.push_str("Accept: */*\r\n");
    req_buf.push_str("Accept-Language: en-US,en;q=0.9\r\n");
    req_buf.push_str("Accept-Encoding: gzip\r\n");
    req_buf.push_str("Connection: close\r\n\r\n");

    stream
        .write_all(req_buf.as_bytes())
        .map_err(|e| map_stream_io_error(e, false))?;
    stream.flush().map_err(|e| map_stream_io_error(e, false))?;

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    let n = reader
        .read_line(&mut status_line)
        .map_err(|e| map_stream_io_error(e, true))?;
    if n == 0 {
        return Err(StageError::TlsHandshakeIo(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "unexpected EOF reading response status",
        )));
    }
    let parts: Vec<&str> = status_line.trim().splitn(3, ' ').collect();
    if parts.len() < 2 || !parts[0].starts_with("HTTP/1.") {
        return Err(StageError::Protocol(format!(
            "malformed HTTP response status: {}",
            status_line.trim()
        )));
    }
    let status_code: i32 = parts[1]
        .parse()
        .map_err(|_| StageError::Protocol("invalid status code".to_string()))?;

    let mut headers = Vec::new();
    let mut content_length: Option<usize> = None;
    let mut is_chunked = false;
    let mut is_gzip = false;

    loop {
        let _ = remaining_timeout(deadline)?;
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .map_err(|e| map_stream_io_error(e, false))?;
        if read == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((raw_name, raw_val)) = line.split_once(':') {
            let name = raw_name.trim().to_ascii_lowercase();
            let value = raw_val.trim().to_string();
            if name == "content-length" {
                content_length = value.parse::<usize>().ok();
            } else if name == "transfer-encoding" && value.to_ascii_lowercase().contains("chunked") {
                is_chunked = true;
            } else if name == "content-encoding" && value.eq_ignore_ascii_case("gzip") {
                is_gzip = true;
            }
            headers.push(FetchHttpHeader { name, value });
        }
    }

    let raw_body = if is_chunked {
        read_chunked_body(&mut reader, deadline)?
    } else if let Some(len) = content_length {
        if len > FETCH_URL_BODY_MAX + 1 {
            return Ok((status_code, headers, vec![0u8; FETCH_URL_BODY_MAX + 1]));
        }
        let mut buf = vec![0u8; len];
        reader
            .read_exact(&mut buf)
            .map_err(|e| map_stream_io_error(e, false))?;
        buf
    } else {
        let mut buf = Vec::new();
        reader
            .take((FETCH_URL_BODY_MAX + 1) as u64)
            .read_to_end(&mut buf)
            .map_err(|e| map_stream_io_error(e, false))?;
        buf
    };

    finalize_body_and_headers(status_code, headers, raw_body, is_gzip)
}

fn read_chunked_body<R: BufRead>(reader: &mut R, deadline: Instant) -> Result<Vec<u8>, StageError> {
    let mut out = Vec::new();
    loop {
        let _ = remaining_timeout(deadline)?;
        let mut size_line = String::new();
        if reader
            .read_line(&mut size_line)
            .map_err(|e| map_stream_io_error(e, false))?
            == 0
        {
            break;
        }
        let size_hex = size_line
            .trim()
            .split(';')
            .next()
            .unwrap_or("")
            .trim();
        if size_hex.is_empty() {
            continue;
        }
        let chunk_size = usize::from_str_radix(size_hex, 16)
            .map_err(|_| StageError::Protocol("invalid chunk size".to_string()))?;
        if chunk_size == 0 {
            let mut trailer = String::new();
            let _ = reader.read_line(&mut trailer);
            break;
        }
        let to_read = chunk_size.min(FETCH_URL_BODY_MAX + 1 - out.len().min(FETCH_URL_BODY_MAX + 1));
        let start = out.len();
        out.resize(start + to_read, 0);
        reader
            .read_exact(&mut out[start..start + to_read])
            .map_err(|e| map_stream_io_error(e, false))?;
        if chunk_size > to_read {
            return Ok(out);
        }
        let mut crlf = [0u8; 2];
        let _ = reader.read_exact(&mut crlf);
    }
    Ok(out)
}

fn finalize_body_and_headers(
    status_code: i32,
    mut headers: Vec<FetchHttpHeader>,
    raw_body: Vec<u8>,
    is_gzip: bool,
) -> Result<(i32, Vec<FetchHttpHeader>, Vec<u8>), StageError> {
    if is_gzip && !raw_body.is_empty() {
        let decoder = GzDecoder::new(&raw_body[..]);
        let mut decompressed = Vec::new();
        decoder
            .take((FETCH_URL_BODY_MAX + 1) as u64)
            .read_to_end(&mut decompressed)
            .map_err(|e| StageError::HttpIo(e))?;
        headers.retain(|h| h.name != "content-encoding" && h.name != "content-length");
        return Ok((status_code, headers, decompressed));
    }
    Ok((status_code, headers, raw_body))
}

// ============================================================================
// Minimal RFC 7540 / RFC 7541 HTTP/2 Single-Stream GET Client
// ============================================================================

const H2_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
const H2_FRAME_DATA: u8 = 0x0;
const H2_FRAME_HEADERS: u8 = 0x1;
const H2_FRAME_RST_STREAM: u8 = 0x3;
const H2_FRAME_SETTINGS: u8 = 0x4;
const H2_FRAME_PING: u8 = 0x6;
const H2_FRAME_GOAWAY: u8 = 0x7;
const H2_FRAME_WINDOW_UPDATE: u8 = 0x8;
const H2_FRAME_CONTINUATION: u8 = 0x9;

const H2_FLAG_ACK: u8 = 0x1;
const H2_FLAG_END_STREAM: u8 = 0x1;
const H2_FLAG_END_HEADERS: u8 = 0x4;
const H2_FLAG_PADDED: u8 = 0x8;
const H2_FLAG_PRIORITY: u8 = 0x20;

fn exchange_h2<S: Read + Write>(
    stream: &mut S,
    scheme: &str,
    authority: &str,
    path_and_query: &str,
    user_agent: &str,
    deadline: Instant,
) -> Result<(i32, Vec<FetchHttpHeader>, Vec<u8>), StageError> {
    let _ = remaining_timeout(deadline)?;
    // 1. Send Client Connection Preface + initial SETTINGS frame (ENABLE_PUSH = 0)
    let mut init_buf = Vec::with_capacity(256);
    init_buf.extend_from_slice(H2_PREFACE);
    // SETTINGS frame: length=6, type=4, flags=0, stream_id=0; ID=2 (ENABLE_PUSH), Value=0
    init_buf.extend_from_slice(&[0x00, 0x00, 0x06, H2_FRAME_SETTINGS, 0x00, 0, 0, 0, 0]);
    init_buf.extend_from_slice(&[0x00, 0x02, 0x00, 0x00, 0x00, 0x00]);

    // Encode request headers on stream 1 using HPACK literal without indexing
    let mut hpack = Vec::with_capacity(192);
    encode_hpack_literal(&mut hpack, ":method", "GET");
    encode_hpack_literal(&mut hpack, ":path", path_and_query);
    encode_hpack_literal(&mut hpack, ":scheme", scheme);
    encode_hpack_literal(&mut hpack, ":authority", authority);
    if !user_agent.is_empty() {
        encode_hpack_literal(&mut hpack, "user-agent", user_agent);
    }
    encode_hpack_literal(&mut hpack, "accept", "*/*");
    encode_hpack_literal(&mut hpack, "accept-language", "en-US,en;q=0.9");
    encode_hpack_literal(&mut hpack, "accept-encoding", "gzip");

    let hlen = hpack.len();
    init_buf.push(((hlen >> 16) & 0xFF) as u8);
    init_buf.push(((hlen >> 8) & 0xFF) as u8);
    init_buf.push((hlen & 0xFF) as u8);
    init_buf.push(H2_FRAME_HEADERS);
    init_buf.push(H2_FLAG_END_STREAM | H2_FLAG_END_HEADERS);
    init_buf.extend_from_slice(&1u32.to_be_bytes());
    init_buf.extend_from_slice(&hpack);

    stream
        .write_all(&init_buf)
        .map_err(|e| map_stream_io_error(e, false))?;
    stream.flush().map_err(|e| map_stream_io_error(e, false))?;

    let mut header_block = Vec::new();
    let mut data_buf = Vec::new();
    let mut headers_complete = false;
    let mut stream_ended = false;

    while !stream_ended {
        let _ = remaining_timeout(deadline)?;
        let mut fh = [0u8; 9];
        if let Err(e) = stream.read_exact(&mut fh) {
            if headers_complete && e.kind() == io::ErrorKind::UnexpectedEof {
                break;
            }
            return Err(map_stream_io_error(e, !headers_complete));
        }
        let len = ((fh[0] as usize) << 16) | ((fh[1] as usize) << 8) | (fh[2] as usize);
        let ftype = fh[3];
        let flags = fh[4];
        let stream_id = u32::from_be_bytes([fh[5] & 0x7F, fh[6], fh[7], fh[8]]);

        let mut payload = vec![0u8; len];
        if len > 0 {
            stream
                .read_exact(&mut payload)
                .map_err(|e| map_stream_io_error(e, !headers_complete))?;
        }

        match ftype {
            H2_FRAME_SETTINGS => {
                if (flags & H2_FLAG_ACK) == 0 {
                    // Send SETTINGS ACK
                    let ack = [0, 0, 0, H2_FRAME_SETTINGS, H2_FLAG_ACK, 0, 0, 0, 0];
                    let _ = stream.write_all(&ack);
                    let _ = stream.flush();
                }
            }
            H2_FRAME_PING => {
                if (flags & H2_FLAG_ACK) == 0 && payload.len() == 8 {
                    let mut pong = [0u8; 17];
                    pong[2] = 8;
                    pong[3] = H2_FRAME_PING;
                    pong[4] = H2_FLAG_ACK;
                    pong[9..17].copy_from_slice(&payload);
                    let _ = stream.write_all(&pong);
                    let _ = stream.flush();
                }
            }
            H2_FRAME_HEADERS if stream_id == 1 => {
                let mut slice = &payload[..];
                if (flags & H2_FLAG_PADDED) != 0 && !slice.is_empty() {
                    let pad_len = slice[0] as usize;
                    slice = &slice[1..slice.len().saturating_sub(pad_len)];
                }
                if (flags & H2_FLAG_PRIORITY) != 0 && slice.len() >= 5 {
                    slice = &slice[5..];
                }
                header_block.extend_from_slice(slice);
                if (flags & H2_FLAG_END_HEADERS) != 0 {
                    headers_complete = true;
                }
                if (flags & H2_FLAG_END_STREAM) != 0 {
                    stream_ended = true;
                }
            }
            H2_FRAME_CONTINUATION if stream_id == 1 => {
                header_block.extend_from_slice(&payload);
                if (flags & H2_FLAG_END_HEADERS) != 0 {
                    headers_complete = true;
                }
            }
            H2_FRAME_DATA if stream_id == 1 => {
                let mut slice = &payload[..];
                if (flags & H2_FLAG_PADDED) != 0 && !slice.is_empty() {
                    let pad_len = slice[0] as usize;
                    slice = &slice[1..slice.len().saturating_sub(pad_len)];
                }
                if data_buf.len() + slice.len() <= FETCH_URL_BODY_MAX + 1 {
                    data_buf.extend_from_slice(slice);
                } else {
                    let rem = (FETCH_URL_BODY_MAX + 1).saturating_sub(data_buf.len());
                    data_buf.extend_from_slice(&slice[..rem]);
                    stream_ended = true;
                }
                if len > 0 && (flags & H2_FLAG_END_STREAM) == 0 {
                    // Send connection & stream WINDOW_UPDATE so larger bodies never stall flow control
                    let inc = (len as u32) & 0x7FFF_FFFF;
                    if inc > 0 {
                        let inc_bytes = inc.to_be_bytes();
                        let mut wu = [0u8; 26];
                        wu[2] = 4;
                        wu[3] = H2_FRAME_WINDOW_UPDATE;
                        wu[9..13].copy_from_slice(&inc_bytes);
                        wu[15] = 4;
                        wu[16] = H2_FRAME_WINDOW_UPDATE;
                        wu[21] = 1;
                        wu[22..26].copy_from_slice(&inc_bytes);
                        let _ = stream.write_all(&wu);
                        let _ = stream.flush();
                    }
                }
                if (flags & H2_FLAG_END_STREAM) != 0 {
                    stream_ended = true;
                }
            }
            H2_FRAME_RST_STREAM if stream_id == 1 => {
                return Err(StageError::HttpIo(io::Error::new(
                    io::ErrorKind::ConnectionReset,
                    "http2 stream reset by peer",
                )));
            }
            H2_FRAME_GOAWAY => {
                if headers_complete {
                    break;
                }
                return Err(StageError::HttpIo(io::Error::new(
                    io::ErrorKind::ConnectionReset,
                    "http2 goaway received",
                )));
            }
            _ => {}
        }
    }

    let (status_code, headers) = decode_hpack_block(&header_block)?;
    let is_gzip = headers
        .iter()
        .any(|h| h.name == "content-encoding" && h.value.eq_ignore_ascii_case("gzip"));
    finalize_body_and_headers(status_code, headers, data_buf, is_gzip)
}

pub(crate) fn encode_hpack_literal(out: &mut Vec<u8>, name: &str, value: &str) {
    out.push(0x00); // Literal Header Field without Indexing — New Name
    encode_hpack_int(out, 0, 7, name.len());
    out.extend_from_slice(name.as_bytes());
    encode_hpack_int(out, 0, 7, value.len());
    out.extend_from_slice(value.as_bytes());
}

/// Stateful HPACK header-block decoder preserving pseudo-headers (`:method`, `:path`, etc.)
/// across HTTP/2 streams on a connection (used by the h2c gRPC server in `ezytel.rs`).
pub(crate) fn decode_hpack_headers_stateful(
    buf: &[u8],
    dynamic_table: &mut Vec<(String, String)>,
) -> Result<Vec<(String, String)>, String> {
    let mut pos = 0usize;
    let mut out = Vec::new();
    let map_err = |e: StageError| match e {
        StageError::Protocol(m) => m,
        other => format!("{other:?}"),
    };
    let lookup_index =
        |idx: usize, dyn_tbl: &[(String, String)]| -> Result<(String, String), String> {
            if idx == 0 {
                return Err("hpack index 0".to_string());
            }
            if idx <= HPACK_STATIC_TABLE.len() {
                let (k, v) = HPACK_STATIC_TABLE[idx - 1];
                return Ok((k.to_string(), v.to_string()));
            }
            let dyn_idx = idx - HPACK_STATIC_TABLE.len() - 1;
            dyn_tbl
                .get(dyn_idx)
                .cloned()
                .ok_or_else(|| "hpack dynamic index out of range".to_string())
        };

    while pos < buf.len() {
        let b = buf[pos];
        if (b & 0x80) != 0 {
            let idx = decode_hpack_int(buf, &mut pos, 7).map_err(map_err)?;
            let (name, value) = lookup_index(idx, dynamic_table)?;
            out.push((name.to_ascii_lowercase(), value));
        } else if (b & 0x40) != 0 {
            let idx = decode_hpack_int(buf, &mut pos, 6).map_err(map_err)?;
            let name = if idx == 0 {
                decode_hpack_str(buf, &mut pos).map_err(map_err)?
            } else {
                lookup_index(idx, dynamic_table)?.0
            };
            let value = decode_hpack_str(buf, &mut pos).map_err(map_err)?;
            let lower = name.to_ascii_lowercase();
            dynamic_table.insert(0, (lower.clone(), value.clone()));
            out.push((lower, value));
        } else if (b & 0x20) != 0 {
            let _max_size = decode_hpack_int(buf, &mut pos, 5).map_err(map_err)?;
        } else {
            let idx = decode_hpack_int(buf, &mut pos, 4).map_err(map_err)?;
            let name = if idx == 0 {
                decode_hpack_str(buf, &mut pos).map_err(map_err)?
            } else {
                lookup_index(idx, dynamic_table)?.0
            };
            let value = decode_hpack_str(buf, &mut pos).map_err(map_err)?;
            out.push((name.to_ascii_lowercase(), value));
        }
    }
    Ok(out)
}

fn encode_hpack_int(out: &mut Vec<u8>, prefix_byte: u8, prefix_bits: u8, mut val: usize) {
    let max_prefix = (1usize << prefix_bits) - 1;
    if val < max_prefix {
        out.push(prefix_byte | (val as u8));
        return;
    }
    out.push(prefix_byte | (max_prefix as u8));
    val -= max_prefix;
    while val >= 128 {
        out.push(((val & 0x7F) as u8) | 0x80);
        val >>= 7;
    }
    out.push(val as u8);
}

fn decode_hpack_int(buf: &[u8], pos: &mut usize, prefix_bits: u8) -> Result<usize, StageError> {
    if *pos >= buf.len() {
        return Err(StageError::Protocol("truncated hpack integer".to_string()));
    }
    let mask = (1usize << prefix_bits) - 1;
    let mut val = (buf[*pos] as usize) & mask;
    *pos += 1;
    if val < mask {
        return Ok(val);
    }
    let mut shift = 0usize;
    loop {
        if *pos >= buf.len() {
            return Err(StageError::Protocol("truncated hpack varint".to_string()));
        }
        let b = buf[*pos] as usize;
        *pos += 1;
        val += (b & 0x7F) << shift;
        if (b & 0x80) == 0 {
            break;
        }
        shift += 7;
    }
    Ok(val)
}

fn decode_hpack_str(buf: &[u8], pos: &mut usize) -> Result<String, StageError> {
    if *pos >= buf.len() {
        return Err(StageError::Protocol("truncated hpack string".to_string()));
    }
    let huffman = (buf[*pos] & 0x80) != 0;
    let len = decode_hpack_int(buf, pos, 7)?;
    if *pos + len > buf.len() {
        return Err(StageError::Protocol("hpack string out of bounds".to_string()));
    }
    let raw = &buf[*pos..*pos + len];
    *pos += len;
    let bytes = if huffman {
        decode_hpack_huffman(raw)?
    } else {
        raw.to_vec()
    };
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn decode_hpack_block(buf: &[u8]) -> Result<(i32, Vec<FetchHttpHeader>), StageError> {
    let mut pos = 0usize;
    let mut dynamic_table: Vec<(String, String)> = Vec::new();
    let mut status_code: i32 = 200;
    let mut headers = Vec::new();

    let lookup_index = |idx: usize, dyn_tbl: &[(String, String)]| -> Result<(String, String), StageError> {
        if idx == 0 {
            return Err(StageError::Protocol("hpack index 0".to_string()));
        }
        if idx <= HPACK_STATIC_TABLE.len() {
            let (k, v) = HPACK_STATIC_TABLE[idx - 1];
            return Ok((k.to_string(), v.to_string()));
        }
        let dyn_idx = idx - HPACK_STATIC_TABLE.len() - 1;
        dyn_tbl
            .get(dyn_idx)
            .cloned()
            .ok_or_else(|| StageError::Protocol("hpack dynamic index out of range".to_string()))
    };

    while pos < buf.len() {
        let b = buf[pos];
        if (b & 0x80) != 0 {
            // Indexed Header Field
            let idx = decode_hpack_int(buf, &mut pos, 7)?;
            let (name, value) = lookup_index(idx, &dynamic_table)?;
            record_h2_header(&mut status_code, &mut headers, name, value);
        } else if (b & 0x40) != 0 {
            // Literal Header Field with Incremental Indexing
            let idx = decode_hpack_int(buf, &mut pos, 6)?;
            let name = if idx == 0 {
                decode_hpack_str(buf, &mut pos)?
            } else {
                lookup_index(idx, &dynamic_table)?.0
            };
            let value = decode_hpack_str(buf, &mut pos)?;
            dynamic_table.insert(0, (name.clone(), value.clone()));
            record_h2_header(&mut status_code, &mut headers, name, value);
        } else if (b & 0x20) != 0 {
            // Dynamic Table Size Update
            let _max_size = decode_hpack_int(buf, &mut pos, 5)?;
        } else {
            // Literal without indexing (0x00) or Never indexed (0x10)
            let idx = decode_hpack_int(buf, &mut pos, 4)?;
            let name = if idx == 0 {
                decode_hpack_str(buf, &mut pos)?
            } else {
                lookup_index(idx, &dynamic_table)?.0
            };
            let value = decode_hpack_str(buf, &mut pos)?;
            record_h2_header(&mut status_code, &mut headers, name, value);
        }
    }

    Ok((status_code, headers))
}

fn record_h2_header(
    status_code: &mut i32,
    headers: &mut Vec<FetchHttpHeader>,
    name: String,
    value: String,
) {
    if name == ":status" {
        if let Ok(code) = value.parse::<i32>() {
            *status_code = code;
        }
    } else if !name.starts_with(':') {
        headers.push(FetchHttpHeader {
            name: name.to_ascii_lowercase(),
            value,
        });
    }
}

const HPACK_STATIC_TABLE: &[(&str, &str)] = &[
    (":authority", ""),
    (":method", "GET"),
    (":method", "POST"),
    (":path", "/"),
    (":path", "/index.html"),
    (":scheme", "http"),
    (":scheme", "https"),
    (":status", "200"),
    (":status", "204"),
    (":status", "206"),
    (":status", "304"),
    (":status", "400"),
    (":status", "404"),
    (":status", "500"),
    ("accept-charset", ""),
    ("accept-encoding", "gzip, deflate"),
    ("accept-language", ""),
    ("accept-ranges", ""),
    ("accept", ""),
    ("access-control-allow-origin", ""),
    ("age", ""),
    ("allow", ""),
    ("authorization", ""),
    ("cache-control", ""),
    ("content-disposition", ""),
    ("content-encoding", ""),
    ("content-language", ""),
    ("content-length", ""),
    ("content-location", ""),
    ("content-range", ""),
    ("content-type", ""),
    ("cookie", ""),
    ("date", ""),
    ("etag", ""),
    ("expect", ""),
    ("expires", ""),
    ("from", ""),
    ("host", ""),
    ("if-match", ""),
    ("if-modified-since", ""),
    ("if-none-match", ""),
    ("if-range", ""),
    ("if-unmodified-since", ""),
    ("last-modified", ""),
    ("link", ""),
    ("location", ""),
    ("max-forwards", ""),
    ("proxy-authenticate", ""),
    ("proxy-authorization", ""),
    ("range", ""),
    ("referer", ""),
    ("refresh", ""),
    ("retry-after", ""),
    ("server", ""),
    ("set-cookie", ""),
    ("strict-transport-security", ""),
    ("transfer-encoding", ""),
    ("user-agent", ""),
    ("vary", ""),
    ("via", ""),
    ("www-authenticate", ""),
];

// RFC 7541 Appendix B Huffman code table: (code, bit_length) for symbols 0..=256 (256 = EOS)
const HPACK_HUFFMAN_CODES: &[(u32, u8)] = &[
    (0x1ff8, 13), (0x7fffd8, 23), (0xfffffe2, 28), (0xfffffe3, 28), (0xfffffe4, 28), (0xfffffe5, 28),
    (0xfffffe6, 28), (0xfffffe7, 28), (0xfffffe8, 28), (0xffffea, 24), (0x3ffffffc, 30), (0xfffffe9, 28),
    (0xfffffea, 28), (0x3ffffffd, 30), (0xfffffeb, 28), (0xfffffec, 28), (0xfffffed, 28), (0xfffffee, 28),
    (0xfffffef, 28), (0xffffff0, 28), (0xffffff1, 28), (0xffffff2, 28), (0x3ffffffe, 30), (0xffffff3, 28),
    (0xffffff4, 28), (0xffffff5, 28), (0xffffff6, 28), (0xffffff7, 28), (0xffffff8, 28), (0xffffff9, 28),
    (0xffffffa, 28), (0xffffffb, 28), (0x14, 6), (0x3f8, 10), (0x3f9, 10), (0xffa, 12),
    (0x1ff9, 13), (0x15, 6), (0xf8, 8), (0x7fa, 11), (0x3fa, 10), (0x3fb, 10),
    (0xf9, 8), (0x7fb, 11), (0xfa, 8), (0x16, 6), (0x17, 6), (0x18, 6),
    (0x0, 5), (0x1, 5), (0x2, 5), (0x19, 6), (0x1a, 6), (0x1b, 6),
    (0x1c, 6), (0x1d, 6), (0x1e, 6), (0x1f, 6), (0x5c, 7), (0xfb, 8),
    (0x7ffc, 15), (0x20, 6), (0xffb, 12), (0x3fc, 10), (0x1ffa, 13), (0x21, 6),
    (0x5d, 7), (0x5e, 7), (0x5f, 7), (0x60, 7), (0x61, 7), (0x62, 7),
    (0x63, 7), (0x64, 7), (0x65, 7), (0x66, 7), (0x67, 7), (0x68, 7),
    (0x69, 7), (0x6a, 7), (0x6b, 7), (0x6c, 7), (0x6d, 7), (0x6e, 7),
    (0x6f, 7), (0x70, 7), (0x71, 7), (0x72, 7), (0xfc, 8), (0x73, 7),
    (0xfd, 8), (0x1ffb, 13), (0x7fff0, 19), (0x1ffc, 13), (0x3ffc, 14), (0x22, 6),
    (0x7ffd, 15), (0x3, 5), (0x23, 6), (0x4, 5), (0x24, 6), (0x5, 5),
    (0x25, 6), (0x26, 6), (0x27, 6), (0x6, 5), (0x74, 7), (0x75, 7),
    (0x28, 6), (0x29, 6), (0x2a, 6), (0x7, 5), (0x2b, 6), (0x76, 7),
    (0x2c, 6), (0x8, 5), (0x9, 5), (0x2d, 6), (0x77, 7), (0x78, 7),
    (0x79, 7), (0x7a, 7), (0x7b, 7), (0x7ffe, 15), (0x7fc, 11), (0x3ffd, 14),
    (0x1ffd, 13), (0xffffffc, 28), (0xfffe6, 20), (0x3fffd2, 22), (0xfffe7, 20), (0xfffe8, 20),
    (0x3fffd3, 22), (0x3fffd4, 22), (0x3fffd5, 22), (0x7fffd9, 23), (0x3fffd6, 22), (0x7fffda, 23),
    (0x7fffdb, 23), (0x7fffdc, 23), (0x7fffdd, 23), (0x7fffde, 23), (0xffffeb, 24), (0x7fffdf, 23),
    (0xffffec, 24), (0xffffed, 24), (0x3fffd7, 22), (0x7fffe0, 23), (0xffffee, 24), (0x7fffe1, 23),
    (0x7fffe2, 23), (0x7fffe3, 23), (0x7fffe4, 23), (0x1fffdc, 21), (0x3fffd8, 22), (0x7fffe5, 23),
    (0x3fffd9, 22), (0x7fffe6, 23), (0x7fffe7, 23), (0xffffef, 24), (0x3fffda, 22), (0x1fffdd, 21),
    (0xfffe9, 20), (0x3fffdb, 22), (0x3fffdc, 22), (0x7fffe8, 23), (0x7fffe9, 23), (0x1fffde, 21),
    (0x7fffea, 23), (0x3fffdd, 22), (0x3fffde, 22), (0xfffff0, 24), (0x1fffdf, 21), (0x3fffdf, 22),
    (0x7fffeb, 23), (0x7fffec, 23), (0x1fffe0, 21), (0x1fffe1, 21), (0x3fffe0, 22), (0x1fffe2, 21),
    (0x7fffed, 23), (0x3fffe1, 22), (0x7fffee, 23), (0x7fffef, 23), (0xfffea, 20), (0x3fffe2, 22),
    (0x3fffe3, 22), (0x3fffe4, 22), (0x7ffff0, 23), (0x3fffe5, 22), (0x3fffe6, 22), (0x7ffff1, 23),
    (0x3ffffe0, 26), (0x3ffffe1, 26), (0xfffeb, 20), (0x7fff1, 19), (0x3fffe7, 22), (0x7ffff2, 23),
    (0x3fffe8, 22), (0x1ffffec, 25), (0x3ffffe2, 26), (0x3ffffe3, 26), (0x3ffffe4, 26), (0x7ffffde, 27),
    (0x7ffffdf, 27), (0x3ffffe5, 26), (0xfffff1, 24), (0x1ffffed, 25), (0x7fff2, 19), (0x1fffe3, 21),
    (0x3ffffe6, 26), (0x7ffffe0, 27), (0x7ffffe1, 27), (0x3ffffe7, 26), (0x7ffffe2, 27), (0xfffff2, 24),
    (0x1fffe4, 21), (0x1fffe5, 21), (0x3ffffe8, 26), (0x3ffffe9, 26), (0xffffffd, 28), (0x7ffffe3, 27),
    (0x7ffffe4, 27), (0x7ffffe5, 27), (0xfffec, 20), (0xfffff3, 24), (0xfffed, 20), (0x1fffe6, 21),
    (0x3fffe9, 22), (0x1fffe7, 21), (0x1fffe8, 21), (0x7ffff3, 23), (0x3fffea, 22), (0x3fffeb, 22),
    (0x1ffffee, 25), (0x1ffffef, 25), (0xfffff4, 24), (0xfffff5, 24), (0x3ffffea, 26), (0x7ffff4, 23),
    (0x3ffffeb, 26), (0x7ffffe6, 27), (0x3ffffec, 26), (0x3ffffed, 26), (0x7ffffe7, 27), (0x7ffffe8, 27),
    (0x7ffffe9, 27), (0x7ffffea, 27), (0x7ffffeb, 27), (0xffffffe, 28), (0x7ffffec, 27), (0x7ffffed, 27),
    (0x7ffffee, 27), (0x7ffffef, 27), (0x7fffff0, 27), (0x3ffffee, 26),
];

#[derive(Clone, Copy, Default)]
struct HuffNode {
    left: u16,
    right: u16,
    sym: i16,
}

fn huffman_tree() -> &'static [HuffNode] {
    static TREE: OnceLock<Vec<HuffNode>> = OnceLock::new();
    TREE.get_or_init(|| {
        let mut nodes = vec![HuffNode {
            left: 0,
            right: 0,
            sym: -1,
        }];
        for (sym, &(code, nbits)) in HPACK_HUFFMAN_CODES.iter().enumerate() {
            let mut cur = 0usize;
            for bit_idx in (0..nbits).rev() {
                let bit = (code >> bit_idx) & 1;
                let next = if bit == 0 {
                    nodes[cur].left as usize
                } else {
                    nodes[cur].right as usize
                };
                if next != 0 {
                    cur = next;
                } else {
                    let new_idx = nodes.len();
                    nodes.push(HuffNode {
                        left: 0,
                        right: 0,
                        sym: -1,
                    });
                    if bit == 0 {
                        nodes[cur].left = new_idx as u16;
                    } else {
                        nodes[cur].right = new_idx as u16;
                    }
                    cur = new_idx;
                }
            }
            nodes[cur].sym = sym as i16;
        }
        nodes
    })
}

fn decode_hpack_huffman(raw: &[u8]) -> Result<Vec<u8>, StageError> {
    let tree = huffman_tree();
    let mut out = Vec::with_capacity(raw.len() * 2);
    let mut cur = 0usize;
    for &byte in raw {
        for bit_idx in (0..8).rev() {
            let bit = (byte >> bit_idx) & 1;
            let next = if bit == 0 {
                tree[cur].left as usize
            } else {
                tree[cur].right as usize
            };
            if next == 0 {
                return Err(StageError::Protocol("invalid hpack huffman sequence".to_string()));
            }
            if tree[next].sym >= 0 {
                out.push(tree[next].sym as u8);
                cur = 0;
            } else {
                cur = next;
            }
        }
    }
    Ok(out)
}

// ============================================================================
// Error Classification (exact match to `classifyFetchURLError` in `fetch_url.go`)
// ============================================================================

fn classify_stage_error(err: StageError) -> (FetchUrlError, String) {
    match err {
        StageError::Cancelled => (FetchUrlError::Other, "cancelled".to_string()),
        StageError::Dns(msg) => (FetchUrlError::Dns, msg),
        StageError::DeadlineExceeded => (FetchUrlError::Timeout, "i/o timeout".to_string()),
        StageError::TlsCertificate(msg) => (FetchUrlError::Certificate, msg),
        StageError::TlsHandshake(msg) => {
            let lower = msg.to_ascii_lowercase();
            if is_cert_error_message(&lower) {
                (FetchUrlError::Certificate, msg)
            } else {
                (FetchUrlError::Tls, msg)
            }
        }
        StageError::TlsHandshakeIo(io_err) => {
            let msg = format!("tls handshake: {io_err}");
            let lower = msg.to_ascii_lowercase();
            if is_cert_error_message(&lower) {
                return (FetchUrlError::Certificate, msg);
            }
            if matches!(
                io_err.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ) {
                return (FetchUrlError::Timeout, msg);
            }
            if matches!(
                io_err.kind(),
                io::ErrorKind::ConnectionReset
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::UnexpectedEof
                    | io::ErrorKind::BrokenPipe
            ) || lower.contains("reset by peer")
                || lower.contains("forcibly closed")
                || lower.contains("eof")
            {
                return (FetchUrlError::Reset, msg);
            }
            (FetchUrlError::Tls, msg)
        }
        StageError::Dial(io_err) | StageError::HttpIo(io_err) => {
            let msg = io_err.to_string();
            let lower = msg.to_ascii_lowercase();
            if is_cert_error_message(&lower) {
                return (FetchUrlError::Certificate, msg);
            }
            if matches!(
                io_err.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ) || lower.contains("timed out")
            {
                return (FetchUrlError::Timeout, msg);
            }
            if io_err.kind() == io::ErrorKind::ConnectionRefused || lower.contains("refused") {
                return (FetchUrlError::Refused, msg);
            }
            if matches!(
                io_err.raw_os_error(),
                Some(libc::ENETUNREACH | libc::EHOSTUNREACH)
            ) || lower.contains("network is unreachable")
                || lower.contains("no route to host")
            {
                return (FetchUrlError::Unreachable, msg);
            }
            if matches!(
                io_err.kind(),
                io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted
            ) || lower.contains("reset by peer")
                || lower.contains("forcibly closed")
            {
                return (FetchUrlError::Reset, msg);
            }
            (FetchUrlError::Other, msg)
        }
        StageError::Protocol(msg) => (FetchUrlError::Other, msg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use hiddify_crypto::{pem_body, pem_to_der, MtlsIdentity};
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use rustls::{ServerConfig, ServerConnection};
    use std::net::TcpListener;
    use std::thread;

    fn test_tls_identity() -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;

        let id = MtlsIdentity::generate_with_options(
            "127.0.0.1",
            365,
            true,
            &["127.0.0.1", "::1", "localhost"],
        )
        .expect("generate test server identity");
        let cert_der = pem_to_der(&id.certificate_pem).expect("cert pem_to_der");
        let key_body: String = id
            .private_key_pem
            .replace("-----BEGIN PRIVATE KEY-----", "")
            .replace("-----END PRIVATE KEY-----", "")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        let _ = pem_body(&id.certificate_pem);
        let key_der = STANDARD.decode(key_body.as_bytes()).expect("key base64");
        (
            CertificateDer::from(cert_der),
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der)),
        )
    }

    fn spawn_test_https_server(enable_h2: bool) -> (String, Arc<RootCertStore>) {
        let (cert, key) = test_tls_identity();
        let mut root_store = RootCertStore::empty();
        root_store.add(cert.clone()).expect("add test root cert");

        let provider = Arc::new(rustls_rustcrypto::provider());
        let mut server_cfg = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .unwrap();
        server_cfg.alpn_protocols = if enable_h2 {
            vec![b"h2".to_vec(), b"http/1.1".to_vec()]
        } else {
            vec![b"http/1.1".to_vec()]
        };
        let server_cfg = Arc::new(server_cfg);

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let cfg = server_cfg.clone();
                thread::spawn(move || {
                    let conn = ServerConnection::new(cfg).unwrap();
                    let mut tls = StreamOwned::new(conn, stream);
                    if enable_h2 {
                        handle_test_h2_conn(&mut tls);
                    } else {
                        handle_test_http1_conn(&mut tls);
                    }
                });
            }
        });

        (format!("https://127.0.0.1:{}", addr.port()), Arc::new(root_store))
    }

    fn handle_test_http1_conn<S: Read + Write>(tls: &mut S) {
        let mut reader = BufReader::new(tls);
        let mut req_line = String::new();
        if reader.read_line(&mut req_line).unwrap_or(0) == 0 {
            return;
        }
        let path = req_line.split_whitespace().nth(1).unwrap_or("/");
        let mut ua = String::new();
        let mut gzip_ok = false;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                break;
            }
            if let Some((k, v)) = line.split_once(':') {
                if k.trim().eq_ignore_ascii_case("user-agent") {
                    ua = v.trim().to_string();
                } else if k.trim().eq_ignore_ascii_case("accept-encoding")
                    && v.to_ascii_lowercase().contains("gzip")
                {
                    gzip_ok = true;
                }
            }
        }
        let stream = reader.into_inner();
        match path {
            "/redirect" => {
                let resp = "HTTP/1.1 302 Found\r\nLocation: /sub\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = stream.write_all(resp.as_bytes());
            }
            "/sub" => {
                let body = if gzip_ok {
                    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
                    enc.write_all(b"vless://a").unwrap();
                    enc.finish().unwrap()
                } else {
                    b"vless://a".to_vec()
                };
                let ce = if gzip_ok {
                    "Content-Encoding: gzip\r\n"
                } else {
                    ""
                };
                let head = format!(
                    "HTTP/1.1 200 OK\r\nSubscription-Userinfo: upload=1; download=2\r\nX-Proto: HTTP/1.1\r\nX-Ua: {ua}\r\n{ce}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
            _ => {
                let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = stream.write_all(resp.as_bytes());
            }
        }
        let _ = stream.flush();
    }

    fn handle_test_h2_conn<S: Read + Write>(tls: &mut S) {
        let mut preface = [0u8; 24];
        if tls.read_exact(&mut preface).is_err() || preface != H2_PREFACE {
            return;
        }
        // Send empty server SETTINGS
        let _ = tls.write_all(&[0, 0, 0, H2_FRAME_SETTINGS, 0, 0, 0, 0, 0]);
        let _ = tls.flush();

        loop {
            let mut fh = [0u8; 9];
            if tls.read_exact(&mut fh).is_err() {
                return;
            }
            let len = ((fh[0] as usize) << 16) | ((fh[1] as usize) << 8) | (fh[2] as usize);
            let ftype = fh[3];
            let flags = fh[4];
            let stream_id = u32::from_be_bytes([fh[5] & 0x7F, fh[6], fh[7], fh[8]]);
            let mut payload = vec![0u8; len];
            if len > 0 && tls.read_exact(&mut payload).is_err() {
                return;
            }
            if ftype == H2_FRAME_SETTINGS && (flags & H2_FLAG_ACK) == 0 {
                let _ = tls.write_all(&[0, 0, 0, H2_FRAME_SETTINGS, H2_FLAG_ACK, 0, 0, 0, 0]);
                let _ = tls.flush();
            } else if ftype == H2_FRAME_HEADERS && stream_id == 1 {
                // Parse request headers using our HPACK decoder
                let mut pos = 0usize;
                let mut path = String::from("/");
                let mut ua = String::new();
                let mut gzip_ok = false;
                while pos < payload.len() {
                    let _ = decode_hpack_int(&payload, &mut pos, 4);
                    let name = decode_hpack_str(&payload, &mut pos).unwrap_or_default();
                    let val = decode_hpack_str(&payload, &mut pos).unwrap_or_default();
                    if name == ":path" {
                        path = val;
                    } else if name == "user-agent" {
                        ua = val;
                    } else if name == "accept-encoding" && val.contains("gzip") {
                        gzip_ok = true;
                    }
                }
                let mut hpack = Vec::new();
                let mut data = Vec::new();
                match path.as_str() {
                    "/redirect" => {
                        encode_hpack_literal(&mut hpack, ":status", "302");
                        encode_hpack_literal(&mut hpack, "location", "/sub");
                    }
                    "/sub" => {
                        encode_hpack_literal(&mut hpack, ":status", "200");
                        encode_hpack_literal(
                            &mut hpack,
                            "subscription-userinfo",
                            "upload=1; download=2",
                        );
                        encode_hpack_literal(&mut hpack, "x-proto", "HTTP/2.0");
                        encode_hpack_literal(&mut hpack, "x-ua", &ua);
                        if gzip_ok {
                            encode_hpack_literal(&mut hpack, "content-encoding", "gzip");
                            let mut enc = GzEncoder::new(Vec::new(), Compression::default());
                            enc.write_all(b"vless://a").unwrap();
                            data = enc.finish().unwrap();
                        } else {
                            data = b"vless://a".to_vec();
                        }
                    }
                    _ => {
                        encode_hpack_literal(&mut hpack, ":status", "404");
                    }
                }
                let hlen = hpack.len();
                let hflags = if data.is_empty() {
                    H2_FLAG_END_HEADERS | H2_FLAG_END_STREAM
                } else {
                    H2_FLAG_END_HEADERS
                };
                let mut out = vec![
                    ((hlen >> 16) & 0xFF) as u8,
                    ((hlen >> 8) & 0xFF) as u8,
                    (hlen & 0xFF) as u8,
                    H2_FRAME_HEADERS,
                    hflags,
                    0,
                    0,
                    0,
                    1,
                ];
                out.extend_from_slice(&hpack);
                if !data.is_empty() {
                    let dlen = data.len();
                    out.extend_from_slice(&[
                        ((dlen >> 16) & 0xFF) as u8,
                        ((dlen >> 8) & 0xFF) as u8,
                        (dlen & 0xFF) as u8,
                        H2_FRAME_DATA,
                        H2_FLAG_END_STREAM,
                        0,
                        0,
                        0,
                        1,
                    ]);
                    out.extend_from_slice(&data);
                }
                let _ = tls.write_all(&out);
                let _ = tls.flush();
                return;
            }
        }
    }

    #[test]
    fn fetch_url_http1_and_http2_redirects_and_gzip_match_go_tests() {
        for (enable_h2, expected_proto) in [(true, "HTTP/2.0"), (false, "HTTP/1.1")] {
            let (base_url, roots) = spawn_test_https_server(enable_h2);
            let outcome = fetch_url_with_roots(
                &FetchUrlRequest {
                    url: format!("{base_url}/redirect"),
                    user_agent: "HiddifyNext/1".to_string(),
                    timeout_ms: 5000,
                },
                Some(roots.clone()),
            )
            .expect("valid request");
            assert_eq!(outcome.error, FetchUrlError::None);
            assert_eq!(outcome.status_code, 200);
            assert_eq!(outcome.body, b"vless://a");

            let hdrs: std::collections::HashMap<String, String> = outcome
                .headers
                .into_iter()
                .map(|h| (h.name, h.value))
                .collect();
            assert_eq!(hdrs.get("x-proto").map(String::as_str), Some(expected_proto));
            assert_eq!(hdrs.get("x-ua").map(String::as_str), Some("HiddifyNext/1"));
            assert_eq!(
                hdrs.get("subscription-userinfo").map(String::as_str),
                Some("upload=1; download=2")
            );
            assert!(!hdrs.contains_key("content-encoding"));

            // 404 test
            let missing = fetch_url_with_roots(
                &FetchUrlRequest {
                    url: format!("{base_url}/missing"),
                    user_agent: String::new(),
                    timeout_ms: 5000,
                },
                Some(roots),
            )
            .expect("valid 404 request");
            assert_eq!(missing.error, FetchUrlError::None);
            assert_eq!(missing.status_code, 404);
        }
    }

    #[test]
    fn fetch_url_bad_scheme_and_error_classification_match_go_tests() {
        assert!(fetch_url(&FetchUrlRequest {
            url: "ftp://x/y".to_string(),
            user_agent: String::new(),
            timeout_ms: 1000,
        })
        .is_err());

        // 1. Refused (closed port)
        let closed_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let closed_addr = closed_listener.local_addr().unwrap();
        drop(closed_listener);

        // 2. Resetter (accepts, reads ClientHello, closes immediately)
        let reset_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let reset_addr = reset_listener.local_addr().unwrap();
        thread::spawn(move || {
            for mut conn in reset_listener.incoming().flatten() {
                let mut buf = [0u8; 512];
                let _ = conn.read(&mut buf);
                drop(conn);
            }
        });

        // 3. Silent (accepts and holds connection past timeout)
        let silent_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let silent_addr = silent_listener.local_addr().unwrap();
        thread::spawn(move || {
            let mut held = Vec::new();
            for conn in silent_listener.incoming().flatten() {
                held.push(conn);
                thread::sleep(Duration::from_secs(2));
            }
        });

        // 4. Untrusted TLS server
        let (untrusted_url, _untrusted_roots) = spawn_test_https_server(true);

        let cases = [
            (
                "refused",
                format!("https://{closed_addr}/sub?token=secret"),
                FetchUrlError::Refused,
            ),
            (
                "dns",
                "https://no-such-host.invalid/sub?token=secret".to_string(),
                FetchUrlError::Dns,
            ),
            (
                "reset",
                format!("https://{reset_addr}/sub?token=secret"),
                FetchUrlError::Reset,
            ),
            (
                "timeout",
                format!("https://{silent_addr}/sub?token=secret"),
                FetchUrlError::Timeout,
            ),
            (
                "certificate",
                format!("{untrusted_url}/sub?token=secret"),
                FetchUrlError::Certificate,
            ),
        ];

        for (name, url, want) in cases {
            let outcome = fetch_url(&FetchUrlRequest {
                url,
                user_agent: String::new(),
                timeout_ms: 500,
            })
            .expect("classified error outcome");
            assert_eq!(
                outcome.error, want,
                "case {name}: got {:?} ({})",
                outcome.error, outcome.error_detail
            );
            assert!(
                !outcome.error_detail.contains("secret"),
                "case {name} leaked secret URL token: {}",
                outcome.error_detail
            );
        }
    }
}
