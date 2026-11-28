//! Standalone connectivity health checker (`cmd/cmd_health.go`):
//! - Raw DNS query over UDP or TCP (with 2-byte big-endian length prefix) and 1 automatic retry
//! - HTTP / HTTPS `HEAD` probe (default `http://connectivitycheck.gstatic.com/generate_204`)
//! - Pure-Rust RFC 9000 + RFC 9001 QUIC v1 TLS 1.3 `h3` handshake probe (default `1.1.1.1:443`,
//!   SNI `cloudflare-dns.com`), working over both direct UDP and SOCKS5 `UDP ASSOCIATE`.

use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aes::cipher::{BlockEncrypt, KeyInit as BlockKeyInit};
use aes::{Aes128, Aes256};
use aes_gcm::{AeadInPlace, Aes128Gcm, Aes256Gcm};
use rand::RngCore;
use rustls::crypto::cipher::{self, AeadKey, Iv};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::ServerName;
use rustls::quic::{self, KeyChange, Keys, PacketKey, Version};
use rustls::{
    CipherSuiteCommon, ClientConfig, ClientConnection, Error as TlsError, RootCertStore, Side,
    StreamOwned, SupportedCipherSuite, Tls13CipherSuite,
};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::fetch_url::system_and_bundled_roots;
use crate::proxy_dial::{connect_tcp_with_deadline, dial_socks5_tcp, DatagramChannel, Socks5Spec};

pub const DEFAULT_HEALTH_TARGET: &str = "google.com";
pub const DEFAULT_HEALTH_DNS_SERVER: &str = "1.1.1.1:53";
pub const DEFAULT_HEALTH_HTTP_URL: &str = "http://connectivitycheck.gstatic.com/generate_204";
pub const DEFAULT_HEALTH_QUIC_ADDR: &str = "1.1.1.1:443";
pub const DEFAULT_HEALTH_QUIC_SNI: &str = "cloudflare-dns.com";
pub const DEFAULT_HEALTH_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthCheckOptions {
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub proxy_addr: String,
    #[serde(default)]
    pub check_dns: bool,
    #[serde(default)]
    pub check_http: bool,
    #[serde(default)]
    pub check_quic: bool,
    #[serde(default)]
    pub dns_server: String,
    #[serde(default)]
    pub timeout_ms: u64,
    #[serde(default)]
    pub verbose: u8,
    /// Optional override for QUIC `(host:port, sni)` used in deterministic unit tests.
    #[serde(default)]
    pub quic_endpoint_override: Option<(String, String)>,
}

impl Default for HealthCheckOptions {
    fn default() -> Self {
        Self {
            target: DEFAULT_HEALTH_TARGET.to_string(),
            proxy_addr: String::new(),
            check_dns: false,
            check_http: true,
            check_quic: false,
            dns_server: DEFAULT_HEALTH_DNS_SERVER.to_string(),
            timeout_ms: DEFAULT_HEALTH_TIMEOUT.as_millis() as u64,
            verbose: 0,
            quic_endpoint_override: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthCheckItemResult {
    pub name: String,
    pub ok: bool,
    pub duration_us: u64,
    pub formatted_duration: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthCheckReport {
    pub ok: bool,
    pub results: Vec<HealthCheckItemResult>,
    pub total_duration_us: u64,
    pub formatted_total_duration: String,
    pub fatal_error: Option<String>,
}

/// Formats a `Duration` identically to `cmd_health.go:formatDuration`
/// (`Round(Microsecond)` if `< 1ms`, else `Round(Millisecond)`).
pub fn format_health_duration(d: Duration) -> String {
    let nanos = d.as_nanos();
    if nanos < 1_000_000 {
        let us = (nanos + 500) / 1_000;
        if us == 0 {
            "0s".to_string()
        } else {
            format!("{us}µs")
        }
    } else {
        let ms = (nanos + 500_000) / 1_000_000;
        if ms >= 1_000 && ms % 1_000 == 0 {
            format!("{}s", ms / 1_000)
        } else if ms >= 1_000 {
            let secs = ms / 1_000;
            let rem = ms % 1_000;
            let frac = format!("{rem:03}").trim_end_matches('0').to_string();
            format!("{secs}.{frac}s")
        } else {
            format!("{ms}ms")
        }
    }
}

enum HealthDialer {
    Direct,
    Socks5(Socks5Spec),
}

impl HealthDialer {
    fn from_proxy_addr(proxy_addr: &str) -> Result<Self, String> {
        if proxy_addr.trim().is_empty() {
            Ok(Self::Direct)
        } else {
            Ok(Self::Socks5(Socks5Spec::parse(proxy_addr)?))
        }
    }

    fn dial_tcp(&self, host: &str, port: u16, deadline: Instant) -> io::Result<TcpStream> {
        match self {
            Self::Direct => connect_tcp_with_deadline(host, port, deadline),
            Self::Socks5(spec) => dial_socks5_tcp(spec, host, port, deadline),
        }
    }

    fn dial_udp(&self, host: &str, port: u16, deadline: Instant) -> io::Result<DatagramChannel> {
        match self {
            Self::Direct => DatagramChannel::dial_direct(host, port, deadline),
            Self::Socks5(spec) => DatagramChannel::dial_socks5(spec, host, port, deadline),
        }
    }
}

/// Runs all enabled checks sequentially, stopping at the first failure just like `cmd_health.go`.
pub fn run_health_checks(opts: &HealthCheckOptions) -> HealthCheckReport {
    run_health_checks_with_roots(opts, None)
}

pub fn run_health_checks_with_roots(
    opts: &HealthCheckOptions,
    custom_roots: Option<Arc<RootCertStore>>,
) -> HealthCheckReport {
    let total_start = Instant::now();
    let target = if opts.target.trim().is_empty() {
        DEFAULT_HEALTH_TARGET
    } else {
        opts.target.trim()
    };
    let dns_server = if opts.dns_server.trim().is_empty() {
        DEFAULT_HEALTH_DNS_SERVER
    } else {
        opts.dns_server.trim()
    };
    let timeout = if opts.timeout_ms > 0 {
        Duration::from_millis(opts.timeout_ms)
    } else {
        DEFAULT_HEALTH_TIMEOUT
    };
    let (mut do_dns, mut do_http, do_quic) = (opts.check_dns, opts.check_http, opts.check_quic);
    if !do_dns && !do_http && !do_quic {
        do_http = true;
    }
    let _ = &mut do_dns;

    let dialer = match HealthDialer::from_proxy_addr(&opts.proxy_addr) {
        Ok(d) => d,
        Err(err) => {
            let total_dur = total_start.elapsed();
            return HealthCheckReport {
                ok: false,
                results: Vec::new(),
                total_duration_us: total_dur.as_micros() as u64,
                formatted_total_duration: format_health_duration(total_dur),
                fatal_error: Some(err),
            };
        }
    };

    let mut results = Vec::new();

    if do_dns {
        let check_start = Instant::now();
        let outcome = match test_health_dns(&dialer, dns_server, target, timeout) {
            Ok(d) => Ok(d),
            Err(_first_err) => {
                if opts.verbose >= 1 {
                    eprintln!("debug: dns failed, retrying");
                }
                test_health_dns(&dialer, dns_server, target, timeout)
            }
        };
        if !record_step(&mut results, "dns", check_start, outcome) {
            return finalize_report(results, total_start);
        }
    }

    if do_http {
        let check_start = Instant::now();
        let outcome = test_health_http(&dialer, target, timeout, custom_roots.clone());
        if !record_step(&mut results, "http", check_start, outcome) {
            return finalize_report(results, total_start);
        }
    }

    if do_quic {
        let check_start = Instant::now();
        let (quic_addr, quic_sni) = opts
            .quic_endpoint_override
            .as_ref()
            .map(|(a, s)| (a.as_str(), s.as_str()))
            .unwrap_or((DEFAULT_HEALTH_QUIC_ADDR, DEFAULT_HEALTH_QUIC_SNI));
        let outcome = test_health_quic(&dialer, quic_addr, quic_sni, timeout, custom_roots);
        if !record_step(&mut results, "quic", check_start, outcome) {
            return finalize_report(results, total_start);
        }
    }

    finalize_report(results, total_start)
}

fn record_step(
    results: &mut Vec<HealthCheckItemResult>,
    name: &str,
    check_start: Instant,
    outcome: Result<Duration, String>,
) -> bool {
    match outcome {
        Ok(dur) => {
            let d = if dur.is_zero() {
                check_start.elapsed()
            } else {
                dur
            };
            results.push(HealthCheckItemResult {
                name: name.to_string(),
                ok: true,
                duration_us: d.as_micros() as u64,
                formatted_duration: format_health_duration(d),
                error: None,
            });
            true
        }
        Err(err) => {
            let d = check_start.elapsed();
            results.push(HealthCheckItemResult {
                name: name.to_string(),
                ok: false,
                duration_us: d.as_micros() as u64,
                formatted_duration: String::new(),
                error: Some(err),
            });
            false
        }
    }
}

fn finalize_report(results: Vec<HealthCheckItemResult>, total_start: Instant) -> HealthCheckReport {
    let total_dur = total_start.elapsed();
    let fatal_error = results
        .iter()
        .find(|r| !r.ok)
        .map(|r| format!("{}: {}", r.name, r.error.as_deref().unwrap_or("failed")));
    HealthCheckReport {
        ok: fatal_error.is_none(),
        results,
        total_duration_us: total_dur.as_micros() as u64,
        formatted_total_duration: format_health_duration(total_dur),
        fatal_error,
    }
}

// ============================================================================
// 1. DNS Check (`buildDNSQuery` + `testHealthDNS` in `cmd_health.go`)
// ============================================================================

/// Builds a raw DNS A-record query packet identical to `cmd_health.go:buildDNSQuery`
/// (`ID=0xabcd`, `Flags=0x0100`, `QDCOUNT=1`, `QTYPE=1`, `QCLASS=1`).
pub fn build_dns_query(name: &str) -> Vec<u8> {
    let mut query = Vec::with_capacity(64);
    query.extend_from_slice(&0xabcdu16.to_be_bytes());
    query.extend_from_slice(&0x0100u16.to_be_bytes());
    query.extend_from_slice(&1u16.to_be_bytes());
    query.extend_from_slice(&[0, 0, 0, 0, 0, 0]);

    for label in name.split('.') {
        if label.is_empty() {
            continue;
        }
        let bytes = label.as_bytes();
        let len = bytes.len().min(63);
        query.push(len as u8);
        query.extend_from_slice(&bytes[..len]);
    }
    query.extend_from_slice(&[0x00, 0x00, 0x01, 0x00, 0x01]);
    query
}

fn parse_host_port_default(raw: &str, default_port: u16) -> Result<(String, u16), String> {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix('[') {
        if let Some((v6, after)) = rest.split_once(']') {
            if let Some(port_str) = after.strip_prefix(':') {
                let port = port_str
                    .parse::<u16>()
                    .map_err(|_| format!("invalid port in {raw}"))?;
                return Ok((v6.to_string(), port));
            }
            return Ok((v6.to_string(), default_port));
        }
    }
    if let Some((h, p)) = trimmed.rsplit_once(':') {
        if !h.contains(':') {
            let port = p
                .parse::<u16>()
                .map_err(|_| format!("invalid port in {raw}"))?;
            return Ok((h.to_string(), port));
        }
    }
    Ok((trimmed.to_string(), default_port))
}

fn test_health_dns(
    dialer: &HealthDialer,
    dns_server: &str,
    target: &str,
    timeout: Duration,
) -> Result<Duration, String> {
    let start = Instant::now();
    let deadline = start + timeout;

    let (is_tcp, server_str) = if let Some(rest) = dns_server.strip_prefix("udp://") {
        (false, rest)
    } else if let Some(rest) = dns_server.strip_prefix("tcp://") {
        (true, rest)
    } else {
        (false, dns_server)
    };
    let (server_host, server_port) = parse_host_port_default(server_str, 53)?;

    let mut qname = target;
    let parsed_url;
    if target.contains("://") {
        if let Ok(u) = Url::parse(target) {
            if let Some(h) = u.host_str() {
                parsed_url = h.to_string();
                qname = &parsed_url;
            }
        }
    }
    let query = build_dns_query(qname);

    if is_tcp {
        let mut tcp = dialer
            .dial_tcp(&server_host, server_port, deadline)
            .map_err(|e| e.to_string())?;
        let mut payload = Vec::with_capacity(2 + query.len());
        payload.extend_from_slice(&(query.len() as u16).to_be_bytes());
        payload.extend_from_slice(&query);
        tcp.write_all(&payload).map_err(|e| e.to_string())?;
        tcp.flush().map_err(|e| e.to_string())?;

        let mut len_buf = [0u8; 2];
        tcp.read_exact(&mut len_buf).map_err(|_| "invalid DNS response".to_string())?;
        let resp_len = u16::from_be_bytes(len_buf) as usize;
        if resp_len < 12 || resp_len > 4096 {
            return Err("invalid DNS response".to_string());
        }
        let mut resp = vec![0u8; resp_len];
        tcp.read_exact(&mut resp)
            .map_err(|_| "invalid DNS response".to_string())?;
        if (resp[3] & 0x0F) != 0 {
            return Err("invalid DNS response".to_string());
        }
    } else {
        let udp = dialer
            .dial_udp(&server_host, server_port, deadline)
            .map_err(|e| e.to_string())?;
        udp.send(&query).map_err(|e| e.to_string())?;
        let mut buf = [0u8; 4096];
        let n = udp.recv(&mut buf).map_err(|e| e.to_string())?;
        if n < 12 || (buf[3] & 0x0F) != 0 {
            return Err("invalid DNS response".to_string());
        }
    }

    Ok(start.elapsed())
}

// ============================================================================
// 2. HTTP / HTTPS Check (`testHealthHTTP` in `cmd_health.go`)
// ============================================================================

fn test_health_http(
    dialer: &HealthDialer,
    target: &str,
    timeout: Duration,
    custom_roots: Option<Arc<RootCertStore>>,
) -> Result<Duration, String> {
    let start = Instant::now();
    let deadline = start + timeout;
    let test_url = if target.contains("://") {
        target
    } else {
        DEFAULT_HEALTH_HTTP_URL
    };
    let u = Url::parse(test_url).map_err(|e| e.to_string())?;
    let host = u
        .host_str()
        .filter(|h| !h.is_empty())
        .ok_or_else(|| "invalid HTTP target".to_string())?;
    let clean_host = host.trim_start_matches('[').trim_end_matches(']');
    let port = u
        .port_or_known_default()
        .unwrap_or(if u.scheme() == "https" { 443 } else { 80 });
    let path_and_query = match u.query() {
        Some(q) => format!("{}?{q}", u.path()),
        None => {
            if u.path().is_empty() {
                "/".to_string()
            } else {
                u.path().to_string()
            }
        }
    };
    let authority = match u.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.to_string(),
    };

    let mut tcp = dialer
        .dial_tcp(clean_host, port, deadline)
        .map_err(|e| e.to_string())?;

    if u.scheme() == "https" {
        let roots = custom_roots.unwrap_or_else(system_and_bundled_roots);
        let provider = Arc::new(rustls_rustcrypto::provider());
        let cfg = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_root_certificates(roots)
            .with_no_client_auth();
        let server_name =
            ServerName::try_from(clean_host.to_string()).map_err(|e| e.to_string())?;
        let conn = ClientConnection::new(Arc::new(cfg), server_name).map_err(|e| e.to_string())?;
        let mut tls_stream = StreamOwned::new(conn, tcp);
        send_and_verify_head(&mut tls_stream, &authority, &path_and_query)?;
    } else {
        send_and_verify_head(&mut tcp, &authority, &path_and_query)?;
    }

    Ok(start.elapsed())
}

fn send_and_verify_head<S: Read + Write>(
    stream: &mut S,
    authority: &str,
    path_and_query: &str,
) -> Result<(), String> {
    let req = format!(
        "HEAD {path_and_query} HTTP/1.1\r\nHost: {authority}\r\nUser-Agent: Go-http-client/1.1\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    if reader
        .read_line(&mut status_line)
        .map_err(|e| e.to_string())?
        == 0
    {
        return Err("EOF".to_string());
    }
    let trimmed = status_line.trim();
    let mut parts = trimmed.splitn(2, ' ');
    let _http_ver = parts.next().unwrap_or("");
    let status_rest = parts.next().unwrap_or("").trim();
    let code: u16 = status_rest
        .split_whitespace()
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("malformed HTTP status: {trimmed}"))?;
    if code >= 400 {
        return Err(format!("HTTP status {status_rest}"));
    }
    Ok(())
}

// ============================================================================
// 3. Pure-Rust RFC 9000 + RFC 9001 QUIC v1 Handshake Check (`testHealthQUIC`)
// ============================================================================

struct Aes128HpKey(Aes128);
struct Aes256HpKey(Aes256);

fn apply_aes_hp_mask(mask: &[u8; 16], first: &mut u8, pn: &mut [u8], decrypt: bool) -> Result<(), TlsError> {
    let flags_mask = if (*first & 0x80) != 0 { 0x0f } else { 0x1f };
    if decrypt {
        *first ^= mask[0] & flags_mask;
        let pn_len = ((*first & 0x03) as usize) + 1;
        if pn.len() < pn_len {
            return Err(TlsError::General("truncated QUIC packet number".to_string()));
        }
        for (b, m) in pn[..pn_len].iter_mut().zip(&mask[1..]) {
            *b ^= *m;
        }
    } else {
        let pn_len = ((*first & 0x03) as usize) + 1;
        if pn.len() < pn_len {
            return Err(TlsError::General("truncated QUIC packet number".to_string()));
        }
        *first ^= mask[0] & flags_mask;
        for (b, m) in pn[..pn_len].iter_mut().zip(&mask[1..]) {
            *b ^= *m;
        }
    }
    Ok(())
}

impl quic::HeaderProtectionKey for Aes128HpKey {
    fn encrypt_in_place(&self, sample: &[u8], first: &mut u8, pn: &mut [u8]) -> Result<(), TlsError> {
        if sample.len() < 16 {
            return Err(TlsError::General("short QUIC HP sample".to_string()));
        }
        let mut block = aes::Block::clone_from_slice(&sample[..16]);
        self.0.encrypt_block(&mut block);
        let mask: [u8; 16] = block.into();
        apply_aes_hp_mask(&mask, first, pn, false)
    }

    fn decrypt_in_place(&self, sample: &[u8], first: &mut u8, pn: &mut [u8]) -> Result<(), TlsError> {
        if sample.len() < 16 {
            return Err(TlsError::General("short QUIC HP sample".to_string()));
        }
        let mut block = aes::Block::clone_from_slice(&sample[..16]);
        self.0.encrypt_block(&mut block);
        let mask: [u8; 16] = block.into();
        apply_aes_hp_mask(&mask, first, pn, true)
    }

    fn sample_len(&self) -> usize {
        16
    }
}

impl quic::HeaderProtectionKey for Aes256HpKey {
    fn encrypt_in_place(&self, sample: &[u8], first: &mut u8, pn: &mut [u8]) -> Result<(), TlsError> {
        if sample.len() < 16 {
            return Err(TlsError::General("short QUIC HP sample".to_string()));
        }
        let mut block = aes::Block::clone_from_slice(&sample[..16]);
        self.0.encrypt_block(&mut block);
        let mask: [u8; 16] = block.into();
        apply_aes_hp_mask(&mask, first, pn, false)
    }

    fn decrypt_in_place(&self, sample: &[u8], first: &mut u8, pn: &mut [u8]) -> Result<(), TlsError> {
        if sample.len() < 16 {
            return Err(TlsError::General("short QUIC HP sample".to_string()));
        }
        let mut block = aes::Block::clone_from_slice(&sample[..16]);
        self.0.encrypt_block(&mut block);
        let mask: [u8; 16] = block.into();
        apply_aes_hp_mask(&mask, first, pn, true)
    }

    fn sample_len(&self) -> usize {
        16
    }
}

enum QuicAeadCipher {
    Aes128(Aes128Gcm),
    Aes256(Aes256Gcm),
}

struct PureRustQuicPacketKey {
    iv: Iv,
    cipher: QuicAeadCipher,
}

impl PacketKey for PureRustQuicPacketKey {
    fn encrypt_in_place(
        &self,
        packet_number: u64,
        header: &[u8],
        payload: &mut [u8],
    ) -> Result<quic::Tag, TlsError> {
        let nonce = cipher::Nonce::new(&self.iv, packet_number).0;
        match &self.cipher {
            QuicAeadCipher::Aes128(c) => {
                let tag = c
                    .encrypt_in_place_detached((&nonce).into(), header, payload)
                    .map_err(|_| TlsError::EncryptError)?;
                Ok(quic::Tag::from(tag.as_slice()))
            }
            QuicAeadCipher::Aes256(c) => {
                let tag = c
                    .encrypt_in_place_detached((&nonce).into(), header, payload)
                    .map_err(|_| TlsError::EncryptError)?;
                Ok(quic::Tag::from(tag.as_slice()))
            }
        }
    }

    fn decrypt_in_place<'a>(
        &self,
        packet_number: u64,
        header: &[u8],
        payload: &'a mut [u8],
    ) -> Result<&'a [u8], TlsError> {
        if payload.len() < 16 {
            return Err(TlsError::DecryptError);
        }
        let plain_len = payload.len() - 16;
        let (ct, tag_bytes) = payload.split_at_mut(plain_len);
        let nonce = cipher::Nonce::new(&self.iv, packet_number).0;
        match &self.cipher {
            QuicAeadCipher::Aes128(c) => c
                .decrypt_in_place_detached((&nonce).into(), header, ct, (&*tag_bytes).into())
                .map_err(|_| TlsError::DecryptError)?,
            QuicAeadCipher::Aes256(c) => c
                .decrypt_in_place_detached((&nonce).into(), header, ct, (&*tag_bytes).into())
                .map_err(|_| TlsError::DecryptError)?,
        }
        Ok(&payload[..plain_len])
    }

    fn tag_len(&self) -> usize {
        16
    }

    fn confidentiality_limit(&self) -> u64 {
        1 << 23
    }

    fn integrity_limit(&self) -> u64 {
        1 << 52
    }
}

struct Aes128QuicAlg;
struct Aes256QuicAlg;

impl quic::Algorithm for Aes128QuicAlg {
    fn packet_key(&self, key: AeadKey, iv: Iv) -> Box<dyn PacketKey> {
        let c = Aes128Gcm::new_from_slice(key.as_ref()).expect("16-byte AES-128 key");
        Box::new(PureRustQuicPacketKey {
            iv,
            cipher: QuicAeadCipher::Aes128(c),
        })
    }

    fn header_protection_key(&self, key: AeadKey) -> Box<dyn quic::HeaderProtectionKey> {
        let c = Aes128::new_from_slice(key.as_ref()).expect("16-byte AES-128 HP key");
        Box::new(Aes128HpKey(c))
    }

    fn aead_key_len(&self) -> usize {
        16
    }
}

impl quic::Algorithm for Aes256QuicAlg {
    fn packet_key(&self, key: AeadKey, iv: Iv) -> Box<dyn PacketKey> {
        let c = Aes256Gcm::new_from_slice(key.as_ref()).expect("32-byte AES-256 key");
        Box::new(PureRustQuicPacketKey {
            iv,
            cipher: QuicAeadCipher::Aes256(c),
        })
    }

    fn header_protection_key(&self, key: AeadKey) -> Box<dyn quic::HeaderProtectionKey> {
        let c = Aes256::new_from_slice(key.as_ref()).expect("32-byte AES-256 HP key");
        Box::new(Aes256HpKey(c))
    }

    fn aead_key_len(&self) -> usize {
        32
    }
}

pub(crate) fn quic_crypto_provider() -> CryptoProvider {
    static AES128_SUITE: std::sync::OnceLock<&'static Tls13CipherSuite> =
        std::sync::OnceLock::new();
    static AES256_SUITE: std::sync::OnceLock<&'static Tls13CipherSuite> =
        std::sync::OnceLock::new();

    let s128 = *AES128_SUITE.get_or_init(|| {
        let SupportedCipherSuite::Tls13(base) = rustls_rustcrypto::TLS13_AES_128_GCM_SHA256 else {
            unreachable!()
        };
        Box::leak(Box::new(Tls13CipherSuite {
            common: CipherSuiteCommon {
                suite: base.common.suite,
                hash_provider: base.common.hash_provider,
                confidentiality_limit: base.common.confidentiality_limit,
            },
            hkdf_provider: base.hkdf_provider,
            aead_alg: base.aead_alg,
            quic: Some(&Aes128QuicAlg),
        }))
    });
    let s256 = *AES256_SUITE.get_or_init(|| {
        let SupportedCipherSuite::Tls13(base) = rustls_rustcrypto::TLS13_AES_256_GCM_SHA384 else {
            unreachable!()
        };
        Box::leak(Box::new(Tls13CipherSuite {
            common: CipherSuiteCommon {
                suite: base.common.suite,
                hash_provider: base.common.hash_provider,
                confidentiality_limit: base.common.confidentiality_limit,
            },
            hkdf_provider: base.hkdf_provider,
            aead_alg: base.aead_alg,
            quic: Some(&Aes256QuicAlg),
        }))
    });

    let mut provider = rustls_rustcrypto::provider();
    provider.cipher_suites = vec![
        SupportedCipherSuite::Tls13(s128),
        SupportedCipherSuite::Tls13(s256),
    ];
    provider
}

pub(crate) fn quic_initial_tls13_suite() -> &'static Tls13CipherSuite {
    let provider = quic_crypto_provider();
    let SupportedCipherSuite::Tls13(s) = provider.cipher_suites[0] else {
        unreachable!()
    };
    s
}

pub(crate) fn encode_quic_varint(out: &mut Vec<u8>, v: u64) {
    if v < 64 {
        out.push(v as u8);
    } else if v < 16_384 {
        out.extend_from_slice(&((v as u16) | 0x4000).to_be_bytes());
    } else if v < 1_073_741_824 {
        out.extend_from_slice(&((v as u32) | 0x8000_0000).to_be_bytes());
    } else {
        out.extend_from_slice(&(v | 0xC000_0000_0000_0000).to_be_bytes());
    }
}

pub(crate) fn decode_quic_varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let first = *buf.get(*pos)?;
    let prefix = first >> 6;
    let len = 1usize << prefix;
    if *pos + len > buf.len() {
        return None;
    }
    let mut val = (first & 0x3f) as u64;
    for &b in &buf[*pos + 1..*pos + len] {
        val = (val << 8) | (b as u64);
    }
    *pos += len;
    Some(val)
}

pub(crate) fn encode_client_transport_params(scid: &[u8], timeout_ms: u64) -> Vec<u8> {
    let mut tp = Vec::with_capacity(64);
    let mut add_int = |id: u64, val: u64| {
        encode_quic_varint(&mut tp, id);
        let mut tmp = Vec::with_capacity(8);
        encode_quic_varint(&mut tmp, val);
        encode_quic_varint(&mut tp, tmp.len() as u64);
        tp.extend_from_slice(&tmp);
    };
    add_int(0x01, timeout_ms.max(1000)); // max_idle_timeout
    add_int(0x04, 65_536); // initial_max_data
    add_int(0x05, 65_536); // initial_max_stream_data_bidi_local
    add_int(0x06, 65_536); // initial_max_stream_data_bidi_remote
    add_int(0x07, 65_536); // initial_max_stream_data_uni
    add_int(0x08, 4); // initial_max_streams_bidi
    add_int(0x09, 4); // initial_max_streams_uni

    // Mandatory RFC 9000 §7.3: initial_source_connection_id (0x0f)
    encode_quic_varint(&mut tp, 0x0f);
    encode_quic_varint(&mut tp, scid.len() as u64);
    tp.extend_from_slice(scid);
    tp
}

/// Seals a QUIC v1 Long Header packet (`packet_type`: `0` = Initial, `2` = Handshake).
pub(crate) fn seal_long_header_packet(
    packet_type: u8,
    dcid: &[u8],
    scid: &[u8],
    pn: u32,
    frames: &[u8],
    pad_to_1200: bool,
    keys: &Keys,
) -> Result<Vec<u8>, String> {
    let mut hdr = Vec::with_capacity(64);
    // Long header: Fixed(0x40) | Form(0x80) | Type(packet_type << 4) | PN length 4 bytes (0x03)
    hdr.push(0xC0 | ((packet_type & 0x03) << 4) | 0x03);
    hdr.extend_from_slice(&0x0000_0001u32.to_be_bytes());
    hdr.push(dcid.len() as u8);
    hdr.extend_from_slice(dcid);
    hdr.push(scid.len() as u8);
    hdr.extend_from_slice(scid);
    if packet_type == 0 {
        // Initial packet carries empty Retry Token length = 0
        hdr.push(0x00);
    }

    let mut plain = frames.to_vec();
    // Ensure sample (at pn_offset + 4, 16 bytes long) is always in bounds, and pad Initial to >= 1200 bytes
    let min_plain = if pad_to_1200 {
        1200usize.saturating_sub(hdr.len() + 2 + 4 + 16)
    } else {
        16
    };
    if plain.len() < min_plain {
        plain.resize(min_plain, 0x00); // PADDING frames (0x00)
    }

    // Length field = 4 (PN) + plain.len() + 16 (AEAD tag), encoded as 2-byte varint
    let rem_len = (4 + plain.len() + 16) as u16;
    hdr.extend_from_slice(&(rem_len | 0x4000).to_be_bytes());
    let pn_offset = hdr.len();
    hdr.extend_from_slice(&pn.to_be_bytes());

    let tag = keys
        .local
        .packet
        .encrypt_in_place(pn as u64, &hdr, &mut plain)
        .map_err(|e| format!("QUIC encrypt error: {e}"))?;

    let mut pkt = hdr;
    pkt.extend_from_slice(&plain);
    pkt.extend_from_slice(tag.as_ref());

    let sample_start = pn_offset + 4;
    let sample = pkt[sample_start..sample_start + 16].to_vec();
    let (before_pn, from_pn) = pkt.split_at_mut(pn_offset);
    keys.local
        .header
        .encrypt_in_place(&sample, &mut before_pn[0], &mut from_pn[..4])
        .map_err(|e| format!("QUIC HP encrypt error: {e}"))?;

    Ok(pkt)
}

/// Opens a single QUIC v1 Long Header packet at the front of `datagram`, returning
/// `(consumed_bytes, packet_type, remote_scid, packet_number, decrypted_frames)`.
pub(crate) fn open_long_header_packet(
    datagram: &[u8],
    initial_keys: &Keys,
    handshake_keys: Option<&Keys>,
) -> Result<(usize, u8, Vec<u8>, u64, Vec<u8>), String> {
    if datagram.len() < 7 || (datagram[0] & 0x80) == 0 {
        return Err("not a long header packet".to_string());
    }
    let pkt_type = (datagram[0] >> 4) & 0x03;
    let keys = match pkt_type {
        0 => initial_keys,
        2 => handshake_keys.ok_or_else(|| "handshake keys not yet ready".to_string())?,
        _ => return Err(format!("skipping long packet type {pkt_type}")),
    };

    let mut pos = 5usize; // skip first byte + 4-byte version
    let dcid_len = *datagram.get(pos).ok_or("truncated dcid len")? as usize;
    pos += 1;
    if pos + dcid_len > datagram.len() {
        return Err("truncated dcid".to_string());
    }
    pos += dcid_len;

    let scid_len = *datagram.get(pos).ok_or("truncated scid len")? as usize;
    pos += 1;
    if pos + scid_len > datagram.len() {
        return Err("truncated scid".to_string());
    }
    let remote_scid = datagram[pos..pos + scid_len].to_vec();
    pos += scid_len;

    if pkt_type == 0 {
        let tok_len = decode_quic_varint(datagram, &mut pos).ok_or("truncated token len")? as usize;
        if pos + tok_len > datagram.len() {
            return Err("truncated token".to_string());
        }
        pos += tok_len;
    }

    let payload_len = decode_quic_varint(datagram, &mut pos).ok_or("truncated length")? as usize;
    let pn_offset = pos;
    let total_pkt_len = pn_offset + payload_len;
    if total_pkt_len > datagram.len() || payload_len < 20 || pn_offset + 20 > total_pkt_len {
        return Err("truncated QUIC packet payload".to_string());
    }

    let mut pkt = datagram[..total_pkt_len].to_vec();
    let sample = pkt[pn_offset + 4..pn_offset + 20].to_vec();
    {
        let (before_pn, from_pn) = pkt.split_at_mut(pn_offset);
        keys.remote
            .header
            .decrypt_in_place(&sample, &mut before_pn[0], &mut from_pn[..4])
            .map_err(|e| format!("QUIC HP decrypt error: {e}"))?;
    }

    let pn_len = ((pkt[0] & 0x03) as usize) + 1;
    let mut pn: u64 = 0;
    for &b in &pkt[pn_offset..pn_offset + pn_len] {
        pn = (pn << 8) | (b as u64);
    }

    let (hdr, cipher_and_tag) = pkt.split_at_mut(pn_offset + pn_len);
    let plain = keys
        .remote
        .packet
        .decrypt_in_place(pn, hdr, cipher_and_tag)
        .map_err(|e| format!("QUIC payload decrypt error: {e}"))?;

    Ok((total_pkt_len, pkt_type, remote_scid, pn, plain.to_vec()))
}

#[derive(Default)]
pub(crate) struct CryptoReassembler {
    next_offset: u64,
    chunks: BTreeMap<u64, Vec<u8>>,
}

impl CryptoReassembler {
    pub(crate) fn insert(&mut self, offset: u64, data: &[u8]) {
        if data.is_empty() || offset + (data.len() as u64) <= self.next_offset {
            return;
        }
        if offset < self.next_offset {
            let skip = (self.next_offset - offset) as usize;
            self.chunks.insert(self.next_offset, data[skip..].to_vec());
        } else {
            self.chunks.insert(offset, data.to_vec());
        }
    }

    pub(crate) fn drain_contiguous(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        while let Some((&off, _)) = self.chunks.first_key_value() {
            if off > self.next_offset {
                break;
            }
            let chunk = self.chunks.remove(&off).unwrap();
            let end = off + chunk.len() as u64;
            if end > self.next_offset {
                let start_idx = (self.next_offset - off) as usize;
                out.extend_from_slice(&chunk[start_idx..]);
                self.next_offset = end;
            }
        }
        out
    }
}

/// Parses frames inside a decrypted Initial/Handshake payload, pushing `CRYPTO` frames into `reasm`
/// and returning `Err(...)` if the peer sent a `CONNECTION_CLOSE` (`0x1c`).
pub(crate) fn extract_crypto_frames(
    plain: &[u8],
    reasm: &mut CryptoReassembler,
) -> Result<bool, String> {
    let mut pos = 0usize;
    let mut ack_eliciting = false;
    while pos < plain.len() {
        let Some(frame_type) = decode_quic_varint(plain, &mut pos) else {
            break;
        };
        match frame_type {
            0x00 => {} // PADDING
            0x01 => {
                ack_eliciting = true; // PING
            }
            0x02 | 0x03 => {
                // ACK frame
                let _largest = decode_quic_varint(plain, &mut pos).ok_or("bad ack")?;
                let _delay = decode_quic_varint(plain, &mut pos).ok_or("bad ack delay")?;
                let range_count = decode_quic_varint(plain, &mut pos).ok_or("bad ack count")?;
                let _first_range = decode_quic_varint(plain, &mut pos).ok_or("bad first range")?;
                for _ in 0..range_count {
                    let _gap = decode_quic_varint(plain, &mut pos).ok_or("bad ack gap")?;
                    let _len = decode_quic_varint(plain, &mut pos).ok_or("bad ack len")?;
                }
                if frame_type == 0x03 {
                    let _ = decode_quic_varint(plain, &mut pos);
                    let _ = decode_quic_varint(plain, &mut pos);
                    let _ = decode_quic_varint(plain, &mut pos);
                }
            }
            0x06 => {
                // CRYPTO frame
                ack_eliciting = true;
                let offset = decode_quic_varint(plain, &mut pos).ok_or("bad crypto offset")?;
                let len = decode_quic_varint(plain, &mut pos).ok_or("bad crypto len")? as usize;
                if pos + len > plain.len() {
                    return Err("truncated CRYPTO frame".to_string());
                }
                reasm.insert(offset, &plain[pos..pos + len]);
                pos += len;
            }
            0x1c => {
                let err_code = decode_quic_varint(plain, &mut pos).unwrap_or(0);
                let _ftype = decode_quic_varint(plain, &mut pos).unwrap_or(0);
                let rlen = decode_quic_varint(plain, &mut pos).unwrap_or(0) as usize;
                let reason = if pos + rlen <= plain.len() {
                    String::from_utf8_lossy(&plain[pos..pos + rlen]).into_owned()
                } else {
                    String::new()
                };
                return Err(format!("QUIC peer closed connection (code {err_code}): {reason}"));
            }
            _ => {
                // Handshake packets only carry PADDING, PING, ACK, CRYPTO, CONNECTION_CLOSE
                break;
            }
        }
    }
    Ok(ack_eliciting)
}

pub(crate) fn encode_crypto_and_ack_frames(
    ack_pn: Option<u64>,
    crypto_offset: &mut u64,
    crypto_data: &[u8],
) -> Vec<u8> {
    let mut frames = Vec::with_capacity(32 + crypto_data.len());
    if let Some(pn) = ack_pn {
        frames.push(0x02); // ACK
        encode_quic_varint(&mut frames, pn);
        encode_quic_varint(&mut frames, 0); // ack_delay = 0
        encode_quic_varint(&mut frames, 0); // ack_range_count = 0
        encode_quic_varint(&mut frames, 0); // first_ack_range = 0
    }
    if !crypto_data.is_empty() {
        frames.push(0x06); // CRYPTO
        encode_quic_varint(&mut frames, *crypto_offset);
        encode_quic_varint(&mut frames, crypto_data.len() as u64);
        frames.extend_from_slice(crypto_data);
        *crypto_offset += crypto_data.len() as u64;
    }
    frames
}

fn encode_connection_close_frame(error_code: u64, reason: &str) -> Vec<u8> {
    let mut f = Vec::with_capacity(8 + reason.len());
    f.push(0x1c);
    encode_quic_varint(&mut f, error_code);
    encode_quic_varint(&mut f, 0);
    encode_quic_varint(&mut f, reason.len() as u64);
    f.extend_from_slice(reason.as_bytes());
    f
}

fn test_health_quic(
    dialer: &HealthDialer,
    addr_str: &str,
    sni: &str,
    timeout: Duration,
    custom_roots: Option<Arc<RootCertStore>>,
) -> Result<Duration, String> {
    let start = Instant::now();
    let deadline = start + timeout;
    let (host, port) = parse_host_port_default(addr_str, 443)?;

    let udp = dialer
        .dial_udp(&host, port, deadline)
        .map_err(|e| e.to_string())?;

    let roots = custom_roots.unwrap_or_else(system_and_bundled_roots);
    let provider = Arc::new(quic_crypto_provider());
    let mut tls_cfg = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| e.to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth();
    tls_cfg.alpn_protocols = vec![b"h3".to_vec()];

    let mut scid = [0u8; 8];
    let mut initial_dcid = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut scid);
    rand::thread_rng().fill_bytes(&mut initial_dcid);
    let mut peer_dcid = initial_dcid.to_vec();
    let mut peer_dcid_locked = false;

    let server_name = ServerName::try_from(sni.to_string()).map_err(|e| e.to_string())?;
    let tp = encode_client_transport_params(&scid, timeout.as_millis() as u64);
    let mut quic_tls = quic::ClientConnection::new(Arc::new(tls_cfg), Version::V1, server_name, tp)
        .map_err(|e| e.to_string())?;

    let initial_keys = Keys::initial(
        Version::V1,
        quic_initial_tls13_suite(),
        &Aes128QuicAlg,
        &initial_dcid,
        Side::Client,
    );
    let mut handshake_keys: Option<Keys> = None;

    let mut init_crypto_off = 0u64;
    let mut hs_crypto_off = 0u64;
    let mut init_pn = 0u32;
    let mut hs_pn = 0u32;

    let mut ch_bytes = Vec::new();
    let _ = quic_tls.write_hs(&mut ch_bytes);
    let init_frames = encode_crypto_and_ack_frames(None, &mut init_crypto_off, &ch_bytes);
    let first_datagram = seal_long_header_packet(
        0,
        &peer_dcid,
        &scid,
        init_pn,
        &init_frames,
        true,
        &initial_keys,
    )?;
    init_pn += 1;
    udp.send(&first_datagram).map_err(|e| e.to_string())?;

    let mut init_reasm = CryptoReassembler::default();
    let mut hs_reasm = CryptoReassembler::default();
    let mut recv_buf = [0u8; 65_536];
    let mut retransmits = 0u8;

    while quic_tls.is_handshaking() {
        let now = Instant::now();
        if now >= deadline {
            return Err("QUIC handshake timed out".to_string());
        }
        let step_wait = (deadline - now).min(Duration::from_millis(800));
        let _ = udp.set_read_timeout(Some(step_wait));

        let n = match udp.recv(&mut recv_buf) {
            Ok(n) => n,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                if Instant::now() >= deadline || retransmits >= 4 {
                    return Err("QUIC handshake timed out".to_string());
                }
                retransmits += 1;
                // Retransmit ClientHello Initial packet with incremented PN
                let mut resend_off = 0u64;
                let resend_frames = encode_crypto_and_ack_frames(None, &mut resend_off, &ch_bytes);
                let pkt = seal_long_header_packet(
                    0,
                    &initial_dcid,
                    &scid,
                    init_pn,
                    &resend_frames,
                    true,
                    &initial_keys,
                )?;
                init_pn += 1;
                let _ = udp.send(&pkt);
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };

        let mut offset = 0usize;
        let mut ack_init_pn: Option<u64> = None;
        let mut ack_hs_pn: Option<u64> = None;

        while offset < n {
            let rem = &recv_buf[offset..n];
            if (rem[0] & 0x80) == 0 {
                // Coalesced 1-RTT packet at tail of datagram
                break;
            }
            match open_long_header_packet(rem, &initial_keys, handshake_keys.as_ref()) {
                Ok((consumed, pkt_type, remote_scid, pn, plain)) => {
                    offset += consumed;
                    if !peer_dcid_locked && !remote_scid.is_empty() {
                        peer_dcid = remote_scid;
                        peer_dcid_locked = true;
                    }
                    if pkt_type == 0 {
                        if extract_crypto_frames(&plain, &mut init_reasm)? {
                            ack_init_pn = Some(pn);
                        }
                        let contig = init_reasm.drain_contiguous();
                        if !contig.is_empty() {
                            quic_tls
                                .read_hs(&contig)
                                .map_err(|e| format!("QUIC TLS error: {e}"))?;
                        }
                        // Install Handshake keys immediately if ServerHello completed inside Initial!
                        let mut scratch = Vec::new();
                        while let Some(kc) = quic_tls.write_hs(&mut scratch) {
                            if let KeyChange::Handshake { keys } = kc {
                                handshake_keys = Some(keys);
                            }
                        }
                    } else if pkt_type == 2 {
                        if extract_crypto_frames(&plain, &mut hs_reasm)? {
                            ack_hs_pn = Some(pn);
                        }
                        let contig = hs_reasm.drain_contiguous();
                        if !contig.is_empty() {
                            quic_tls
                                .read_hs(&contig)
                                .map_err(|e| format!("QUIC TLS error: {e}"))?;
                        }
                    }
                }
                Err(_) => break,
            }
        }

        // Send Initial ACK if needed + Client Handshake Finished + Handshake ACK
        let mut datagram_out = Vec::new();
        if let Some(pn) = ack_init_pn {
            let ack_frames = encode_crypto_and_ack_frames(Some(pn), &mut init_crypto_off, &[]);
            if let Ok(pkt) = seal_long_header_packet(
                0,
                &peer_dcid,
                &scid,
                init_pn,
                &ack_frames,
                false,
                &initial_keys,
            ) {
                init_pn += 1;
                datagram_out.extend_from_slice(&pkt);
            }
        }

        let mut client_hs_flight = Vec::new();
        loop {
            let prev_len = client_hs_flight.len();
            match quic_tls.write_hs(&mut client_hs_flight) {
                Some(KeyChange::Handshake { keys }) => {
                    handshake_keys = Some(keys);
                }
                Some(KeyChange::OneRtt { .. }) => {
                    break;
                }
                None => {
                    if client_hs_flight.len() == prev_len {
                        break;
                    }
                }
            }
        }

        if let Some(ref hs_keys) = handshake_keys {
            if ack_hs_pn.is_some() || !client_hs_flight.is_empty() {
                let hs_frames =
                    encode_crypto_and_ack_frames(ack_hs_pn, &mut hs_crypto_off, &client_hs_flight);
                let pkt = seal_long_header_packet(
                    2,
                    &peer_dcid,
                    &scid,
                    hs_pn,
                    &hs_frames,
                    datagram_out.is_empty() && !quic_tls.is_handshaking(),
                    hs_keys,
                )?;
                hs_pn += 1;
                datagram_out.extend_from_slice(&pkt);
            }
        }

        if !datagram_out.is_empty() {
            let _ = udp.send(&datagram_out);
        }
    }

    // Politely close with CONNECTION_CLOSE(0, "health check done") matching `cmd_health.go:375`
    if let Some(ref hs_keys) = handshake_keys {
        let close_frame = encode_connection_close_frame(0, "health check done");
        if let Ok(close_pkt) =
            seal_long_header_packet(2, &peer_dcid, &scid, hs_pn, &close_frame, false, hs_keys)
        {
            let _ = udp.send(&close_pkt);
        }
    }

    Ok(start.elapsed())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, UdpSocket};
    use std::thread;

    use hiddify_crypto::MtlsIdentity;
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use rustls::ServerConfig;

    fn test_cert_and_roots() -> (
        Vec<CertificateDer<'static>>,
        PrivateKeyDer<'static>,
        Arc<RootCertStore>,
    ) {
        let id = MtlsIdentity::generate_with_options(
            "127.0.0.1",
            365,
            true,
            &["127.0.0.1", "::1", "localhost"],
        )
        .expect("cert");
        let cert = CertificateDer::from_pem_slice(id.certificate_pem.as_bytes()).expect("pem cert");
        let key = PrivateKeyDer::from_pem_slice(id.private_key_pem.as_bytes()).expect("pem key");
        let mut roots = RootCertStore::empty();
        roots.add(cert.clone()).expect("add root");
        (vec![cert], key, Arc::new(roots))
    }

    /// Minimal loopback SOCKS5 server supporting both TCP `CONNECT` and UDP `UDP ASSOCIATE`!
    fn spawn_socks5_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                thread::spawn(move || {
                    let mut s = stream;
                    let mut greet = [0u8; 2];
                    if s.read_exact(&mut greet).is_err() {
                        return;
                    }
                    let mut methods = vec![0u8; greet[1] as usize];
                    let _ = s.read_exact(&mut methods);
                    let _ = s.write_all(&[0x05, 0x00]); // No auth

                    let mut hdr = [0u8; 4];
                    if s.read_exact(&mut hdr).is_err() {
                        return;
                    }
                    let target_host = match hdr[3] {
                        0x01 => {
                            let mut ip = [0u8; 4];
                            s.read_exact(&mut ip).unwrap();
                            std::net::Ipv4Addr::from(ip).to_string()
                        }
                        0x03 => {
                            let mut len = [0u8; 1];
                            s.read_exact(&mut len).unwrap();
                            let mut d = vec![0u8; len[0] as usize];
                            s.read_exact(&mut d).unwrap();
                            String::from_utf8(d).unwrap()
                        }
                        _ => return,
                    };
                    let mut pbytes = [0u8; 2];
                    s.read_exact(&mut pbytes).unwrap();
                    let target_port = u16::from_be_bytes(pbytes);

                    if hdr[1] == 0x01 {
                        // TCP CONNECT
                        let mut up = TcpStream::connect((target_host.as_str(), target_port)).unwrap();
                        s.write_all(&[0x05, 0x00, 0x00, 0x01, 127, 0, 0, 1, 0, 0])
                            .unwrap();
                        let mut s_clone = s.try_clone().unwrap();
                        let mut up_clone = up.try_clone().unwrap();
                        let t = thread::spawn(move || {
                            let _ = io::copy(&mut s_clone, &mut up_clone);
                        });
                        let _ = io::copy(&mut up, &mut s);
                        let _ = t.join();
                    } else if hdr[1] == 0x03 {
                        // UDP ASSOCIATE
                        let relay = UdpSocket::bind("127.0.0.1:0").unwrap();
                        let rport = relay.local_addr().unwrap().port();
                        let mut reply = vec![0x05, 0x00, 0x00, 0x01, 127, 0, 0, 1];
                        reply.extend_from_slice(&rport.to_be_bytes());
                        s.write_all(&reply).unwrap();

                        let _ = relay.set_read_timeout(Some(Duration::from_secs(3)));
                        let mut client_udp_addr: Option<std::net::SocketAddr> = None;
                        let mut buf = [0u8; 65_535];
                        while let Ok((n, src)) = relay.recv_from(&mut buf) {
                            if client_udp_addr.is_none() || Some(src) == client_udp_addr {
                                client_udp_addr = Some(src);
                                // Unwrap SOCKS5 UDP header -> forward to target
                                let (dst_addr, hdr_len) = match buf[3] {
                                    0x01 => {
                                        let ip = std::net::Ipv4Addr::new(
                                            buf[4], buf[5], buf[6], buf[7],
                                        );
                                        let port = u16::from_be_bytes([buf[8], buf[9]]);
                                        (format!("{ip}:{port}"), 10usize)
                                    }
                                    0x03 => {
                                        let dlen = buf[4] as usize;
                                        let dom =
                                            std::str::from_utf8(&buf[5..5 + dlen]).unwrap();
                                        let port =
                                            u16::from_be_bytes([buf[5 + dlen], buf[6 + dlen]]);
                                        (format!("{dom}:{port}"), 7 + dlen)
                                    }
                                    _ => continue,
                                };
                                let _ = relay.send_to(&buf[hdr_len..n], &dst_addr);
                            } else if let Some(caddr) = client_udp_addr {
                                // Wrap response in SOCKS5 UDP header back to client
                                let mut out = vec![0x00, 0x00, 0x00, 0x01, 127, 0, 0, 1, 0, 0];
                                out.extend_from_slice(&buf[..n]);
                                let _ = relay.send_to(&out, caddr);
                            }
                        }
                    }
                });
            }
        });
        format!("socks://127.0.0.1:{}", addr.port())
    }

    /// Minimal loopback QUIC v1 TLS 1.3 Server using `rustls::quic::ServerConnection`!
    fn spawn_quic_v1_server(
        certs: Vec<CertificateDer<'static>>,
        key: PrivateKeyDer<'static>,
    ) -> String {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = sock.local_addr().unwrap();
        thread::spawn(move || {
            let _ = sock.set_read_timeout(Some(Duration::from_secs(5)));
            let provider = Arc::new(quic_crypto_provider());
            let mut scfg = ServerConfig::builder_with_provider(provider)
                .with_protocol_versions(&[&rustls::version::TLS13])
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(certs, key)
                .unwrap();
            scfg.alpn_protocols = vec![b"h3".to_vec()];
            let scfg = Arc::new(scfg);

            let mut buf = [0u8; 65_535];
            let Ok((n, peer)) = sock.recv_from(&mut buf) else {
                return;
            };
            // Parse client's initial DCID (at byte 6..6+dcid_len) & SCID
            let dcid_len = buf[5] as usize;
            let client_dcid = buf[6..6 + dcid_len].to_vec();
            let scid_len = buf[6 + dcid_len] as usize;
            let client_scid = buf[7 + dcid_len..7 + dcid_len + scid_len].to_vec();

            let server_scid = *b"srv_scid";
            // Server transport params: original_destination_connection_id (0x00) + initial_source_connection_id (0x0f)
            let mut tp = Vec::new();
            encode_quic_varint(&mut tp, 0x00);
            encode_quic_varint(&mut tp, client_dcid.len() as u64);
            tp.extend_from_slice(&client_dcid);
            encode_quic_varint(&mut tp, 0x0f);
            encode_quic_varint(&mut tp, server_scid.len() as u64);
            tp.extend_from_slice(&server_scid);

            let mut srv_tls =
                quic::ServerConnection::new(scfg, Version::V1, tp).expect("quic server conn");
            let initial_keys = Keys::initial(
                Version::V1,
                quic_initial_tls13_suite(),
                &Aes128QuicAlg,
                &client_dcid,
                Side::Server,
            );
            let mut hs_keys: Option<Keys> = None;
            let mut init_reasm = CryptoReassembler::default();
            let mut hs_reasm = CryptoReassembler::default();
            let mut init_off = 0u64;
            let mut hs_off = 0u64;
            let mut init_pn = 0u32;
            let mut hs_pn = 0u32;

            let (_, _, _, c_pn, plain) =
                open_long_header_packet(&buf[..n], &initial_keys, None).expect("open client init");
            extract_crypto_frames(&plain, &mut init_reasm).unwrap();
            srv_tls.read_hs(&init_reasm.drain_contiguous()).unwrap();

            // Write ServerHello (Initial) + EncryptedExtensions/Cert/CertVerify/Finished (Handshake)
            let mut sh_buf = Vec::new();
            if let Some(KeyChange::Handshake { keys }) = srv_tls.write_hs(&mut sh_buf) {
                hs_keys = Some(keys);
            }
            let mut hs_flight = Vec::new();
            let _ = srv_tls.write_hs(&mut hs_flight);

            let mut out_dgram = Vec::new();
            let init_frames = encode_crypto_and_ack_frames(Some(c_pn), &mut init_off, &sh_buf);
            out_dgram.extend_from_slice(
                &seal_long_header_packet(
                    0,
                    &client_scid,
                    &server_scid,
                    init_pn,
                    &init_frames,
                    false,
                    &initial_keys,
                )
                .unwrap(),
            );
            init_pn += 1;
            if let Some(ref hk) = hs_keys {
                let hs_frames = encode_crypto_and_ack_frames(None, &mut hs_off, &hs_flight);
                out_dgram.extend_from_slice(
                    &seal_long_header_packet(
                        2,
                        &client_scid,
                        &server_scid,
                        hs_pn,
                        &hs_frames,
                        false,
                        hk,
                    )
                    .unwrap(),
                );
                hs_pn += 1;
            }
            let _ = sock.send_to(&out_dgram, peer);

            // Receive client's Finished
            if let Ok((n2, _)) = sock.recv_from(&mut buf) {
                let mut off = 0usize;
                while off < n2 && (buf[off] & 0x80) != 0 {
                    if let Ok((used, ptype, _, pn2, pl)) =
                        open_long_header_packet(&buf[off..n2], &initial_keys, hs_keys.as_ref())
                    {
                        off += used;
                        if ptype == 2 {
                            let _ = extract_crypto_frames(&pl, &mut hs_reasm);
                            let contig = hs_reasm.drain_contiguous();
                            if !contig.is_empty() {
                                let _ = srv_tls.read_hs(&contig);
                            }
                            if let Some(ref hk) = hs_keys {
                                let ack = encode_crypto_and_ack_frames(Some(pn2), &mut hs_off, &[]);
                                if let Ok(pkt) = seal_long_header_packet(
                                    2,
                                    &client_scid,
                                    &server_scid,
                                    hs_pn,
                                    &ack,
                                    false,
                                    hk,
                                ) {
                                    let _ = sock.send_to(&pkt, peer);
                                }
                            }
                        }
                    } else {
                        break;
                    }
                }
            }
            let _ = init_pn;
        });
        addr.to_string()
    }

    #[test]
    fn health_checks_dns_http_and_quic_over_socks5_match_go_behavior() {
        let (certs, key, roots) = test_cert_and_roots();
        let socks_url = spawn_socks5_server();

        // 1. UDP DNS server on loopback
        let dns_udp = UdpSocket::bind("127.0.0.1:0").unwrap();
        let dns_addr = dns_udp.local_addr().unwrap();
        thread::spawn(move || {
            let mut b = [0u8; 512];
            if let Ok((n, peer)) = dns_udp.recv_from(&mut b) {
                assert!(n >= 12);
                assert_eq!(&b[0..2], &[0xab, 0xcd]);
                let mut resp = b[..n].to_vec();
                resp[2] = 0x81;
                resp[3] = 0x80; // QR=1, RCODE=0
                let _ = dns_udp.send_to(&resp, peer);
            }
        });

        // 2. HTTP 204 server on loopback
        let http_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let http_addr = http_listener.local_addr().unwrap();
        thread::spawn(move || {
            if let Ok((mut s, _)) = http_listener.accept() {
                let mut r = BufReader::new(&mut s);
                let mut line = String::new();
                let _ = r.read_line(&mut line);
                assert!(line.starts_with("HEAD /generate_204"));
                loop {
                    let mut h = String::new();
                    if r.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                        break;
                    }
                }
                let _ = s.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n");
            }
        });

        // 3. QUIC v1 server on loopback
        let quic_addr = spawn_quic_v1_server(certs, key);

        let opts = HealthCheckOptions {
            target: format!("http://{http_addr}/generate_204"),
            proxy_addr: socks_url,
            check_dns: true,
            check_http: true,
            check_quic: true,
            dns_server: dns_addr.to_string(),
            timeout_ms: 5_000,
            verbose: 0,
            quic_endpoint_override: Some((quic_addr, "localhost".to_string())),
        };

        let report = run_health_checks_with_roots(&opts, Some(roots));
        assert!(report.ok, "report failed: {report:?}");
        assert_eq!(report.results.len(), 3);
        assert_eq!(report.results[0].name, "dns");
        assert_eq!(report.results[1].name, "http");
        assert_eq!(report.results[2].name, "quic");
        assert!(report.results.iter().all(|r| r.ok));
    }
}
