//! Single-pass streaming installer downloader with on-the-fly SHA-256 verification,
//! cached-file fast-path reuse, `"PROXY 127.0.0.1:<port>; DIRECT"` fallback, and
//! atomic `.part` -> destination rename (`AppUpdater._download` in `app_updater.dart`).

use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use crate::cancel::{ensure_task, finish_task, is_cancelled, update_progress};
use crate::fetch_url::system_and_bundled_roots;
use crate::proxy_dial::{connect_tcp_with_deadline, dial_http_proxy_connect, LocalHttpProxySpec};

const MAX_UPDATE_REDIRECTS: usize = 10;
const STREAM_CHUNK_SIZE: usize = 64 * 1024;
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_CHUNK_READ_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDownloadRequest {
    pub url: String,
    pub destination_path: String,
    pub expected_size: u64,
    pub expected_sha256: String,
    #[serde(default)]
    pub user_agent: String,
    #[serde(default)]
    pub proxy_port: u16,
    #[serde(default)]
    pub proxy_secret: String,
    #[serde(default)]
    pub cancel_task_id: u64,
    #[serde(default)]
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDownloadResult {
    /// `"ok"`, `"reused"`, `"cancelled"`, `"invalid_digest"`, `"size_mismatch"`,
    /// `"sha256_mismatch"`, `"http_error"`, or `"network_error"`.
    pub status: String,
    pub path: String,
    pub bytes_written: u64,
    pub error: Option<String>,
}

/// Parses `"sha256:<64 hex>"` into lowercase 64-character hex string, returning `None` if invalid.
pub fn parse_sha256_digest(digest: &str) -> Option<String> {
    let hex_part = digest.strip_prefix("sha256:")?;
    if hex_part.len() != 64 || !hex_part.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(hex_part.to_ascii_lowercase())
}

fn hex_encode_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn verify_existing_file(path: &Path, expected_size: u64, expected_hex: &str) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() || (expected_size > 0 && meta.len() != expected_size) {
        return false;
    }
    let Ok(mut f) = File::open(path) else {
        return false;
    };
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; STREAM_CHUNK_SIZE];
    loop {
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(_) => return false,
        }
    }
    let actual_hex = hex_encode_lower(&hasher.finalize());
    actual_hex == expected_hex
}

fn part_file_path(dest: &Path) -> PathBuf {
    let mut os = dest.as_os_str().to_os_string();
    os.push(".part");
    PathBuf::from(os)
}

/// Downloads an installer artifact in a single streaming pass while computing `SHA-256` on the fly.
pub fn download_and_verify_update(req: &UpdateDownloadRequest) -> UpdateDownloadResult {
    download_and_verify_update_with_roots(req, None)
}

pub fn download_and_verify_update_with_roots(
    req: &UpdateDownloadRequest,
    custom_roots: Option<Arc<RootCertStore>>,
) -> UpdateDownloadResult {
    let task_id = req.cancel_task_id;
    ensure_task(task_id);
    let finish = |res: UpdateDownloadResult| -> UpdateDownloadResult {
        finish_task(task_id);
        res
    };

    let Some(expected_hex) = parse_sha256_digest(&req.expected_sha256) else {
        return finish(UpdateDownloadResult {
            status: "invalid_digest".to_string(),
            path: req.destination_path.clone(),
            bytes_written: 0,
            error: Some("the release has no valid SHA-256 digest; the update was not downloaded.".to_string()),
        });
    };

    let dest_path = Path::new(&req.destination_path);
    if let Some(parent) = dest_path.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(e) = fs::create_dir_all(parent) {
                return finish(UpdateDownloadResult {
                    status: "network_error".to_string(),
                    path: req.destination_path.clone(),
                    bytes_written: 0,
                    error: Some(format!("create updates directory: {e}")),
                });
            }
        }
    }

    // Fast path: if a previously downloaded file already matches size + SHA-256, reuse it!
    if dest_path.exists() {
        if verify_existing_file(dest_path, req.expected_size, &expected_hex) {
            let len = fs::metadata(dest_path).map(|m| m.len()).unwrap_or(req.expected_size);
            update_progress(task_id, len, len);
            return finish(UpdateDownloadResult {
                status: "reused".to_string(),
                path: req.destination_path.clone(),
                bytes_written: len,
                error: None,
            });
        }
        let _ = fs::remove_file(dest_path);
    }

    if is_cancelled(task_id) {
        return finish(UpdateDownloadResult {
            status: "cancelled".to_string(),
            path: req.destination_path.clone(),
            bytes_written: 0,
            error: Some("cancelled".to_string()),
        });
    }

    let Ok(initial_url) = Url::parse(req.url.trim()) else {
        return finish(UpdateDownloadResult {
            status: "network_error".to_string(),
            path: req.destination_path.clone(),
            bytes_written: 0,
            error: Some("invalid download URL".to_string()),
        });
    };

    let roots = custom_roots.unwrap_or_else(system_and_bundled_roots);
    let provider = Arc::new(rustls_rustcrypto::provider());
    let Ok(mut tls_cfg) = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map(|b| b.with_root_certificates(roots).with_no_client_auth())
    else {
        return finish(UpdateDownloadResult {
            status: "network_error".to_string(),
            path: req.destination_path.clone(),
            bytes_written: 0,
            error: Some("failed to initialize TLS client".to_string()),
        });
    };
    tls_cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    let tls_cfg = Arc::new(tls_cfg);

    let proxy = LocalHttpProxySpec {
        port: req.proxy_port,
        secret: req.proxy_secret.clone(),
    };
    let step_timeout = if req.timeout_ms > 0 {
        Duration::from_millis(req.timeout_ms)
    } else {
        DEFAULT_CHUNK_READ_TIMEOUT
    };

    let part_path = part_file_path(dest_path);
    let _ = fs::remove_file(&part_path);

    let mut current_url = initial_url;
    for _hop in 0..=MAX_UPDATE_REDIRECTS {
        if is_cancelled(task_id) {
            let _ = fs::remove_file(&part_path);
            return finish(UpdateDownloadResult {
                status: "cancelled".to_string(),
                path: req.destination_path.clone(),
                bytes_written: 0,
                error: Some("cancelled".to_string()),
            });
        }

        match stream_attempt_once(
            &current_url,
            &req.user_agent,
            &proxy,
            &tls_cfg,
            step_timeout,
            &part_path,
            req.expected_size,
            task_id,
        ) {
            Ok(AttemptOutcome::Redirect(next_url)) => {
                current_url = next_url;
                continue;
            }
            Ok(AttemptOutcome::Downloaded {
                bytes_written,
                actual_sha256_hex,
            }) => {
                if is_cancelled(task_id) {
                    let _ = fs::remove_file(&part_path);
                    return finish(UpdateDownloadResult {
                        status: "cancelled".to_string(),
                        path: req.destination_path.clone(),
                        bytes_written,
                        error: Some("cancelled".to_string()),
                    });
                }
                if req.expected_size > 0 && bytes_written != req.expected_size {
                    let _ = fs::remove_file(&part_path);
                    return finish(UpdateDownloadResult {
                        status: "size_mismatch".to_string(),
                        path: req.destination_path.clone(),
                        bytes_written,
                        error: Some(format!(
                            "got {bytes_written} of {} bytes",
                            req.expected_size
                        )),
                    });
                }
                if actual_sha256_hex != expected_hex {
                    let _ = fs::remove_file(&part_path);
                    return finish(UpdateDownloadResult {
                        status: "sha256_mismatch".to_string(),
                        path: req.destination_path.clone(),
                        bytes_written,
                        error: Some(
                            "SHA-256 verification failed; the update was not installed."
                                .to_string(),
                        ),
                    });
                }
                if let Err(e) = fs::rename(&part_path, dest_path) {
                    let _ = fs::remove_file(&part_path);
                    return finish(UpdateDownloadResult {
                        status: "network_error".to_string(),
                        path: req.destination_path.clone(),
                        bytes_written,
                        error: Some(format!("finalize downloaded file: {e}")),
                    });
                }
                return finish(UpdateDownloadResult {
                    status: "ok".to_string(),
                    path: req.destination_path.clone(),
                    bytes_written,
                    error: None,
                });
            }
            Err(StreamError::Cancelled) => {
                let _ = fs::remove_file(&part_path);
                return finish(UpdateDownloadResult {
                    status: "cancelled".to_string(),
                    path: req.destination_path.clone(),
                    bytes_written: 0,
                    error: Some("cancelled".to_string()),
                });
            }
            Err(StreamError::HttpStatus(code)) => {
                let _ = fs::remove_file(&part_path);
                return finish(UpdateDownloadResult {
                    status: "http_error".to_string(),
                    path: req.destination_path.clone(),
                    bytes_written: 0,
                    error: Some(format!("HTTP {code}")),
                });
            }
            Err(StreamError::Network(msg)) => {
                let _ = fs::remove_file(&part_path);
                return finish(UpdateDownloadResult {
                    status: "network_error".to_string(),
                    path: req.destination_path.clone(),
                    bytes_written: 0,
                    error: Some(msg),
                });
            }
        }
    }

    let _ = fs::remove_file(&part_path);
    finish(UpdateDownloadResult {
        status: "network_error".to_string(),
        path: req.destination_path.clone(),
        bytes_written: 0,
        error: Some("stopped after 10 redirects".to_string()),
    })
}

enum AttemptOutcome {
    Redirect(Url),
    Downloaded {
        bytes_written: u64,
        actual_sha256_hex: String,
    },
}

enum StreamError {
    Cancelled,
    HttpStatus(u16),
    Network(String),
}

#[allow(clippy::too_many_arguments)]
fn stream_attempt_once(
    url: &Url,
    user_agent: &str,
    proxy: &LocalHttpProxySpec,
    tls_cfg: &Arc<ClientConfig>,
    step_timeout: Duration,
    part_path: &Path,
    declared_size: u64,
    task_id: u64,
) -> Result<AttemptOutcome, StreamError> {
    let host = url
        .host_str()
        .ok_or_else(|| StreamError::Network("missing host".to_string()))?;
    let clean_host = host.trim_start_matches('[').trim_end_matches(']');
    let port = url
        .port_or_known_default()
        .unwrap_or(if url.scheme() == "https" { 443 } else { 80 });
    let path_and_query = match url.query() {
        Some(q) => format!("{}?{q}", url.path()),
        None => {
            if url.path().is_empty() {
                "/".to_string()
            } else {
                url.path().to_string()
            }
        }
    };
    let authority = match url.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.to_string(),
    };

    let connect_deadline = Instant::now() + step_timeout.min(DEFAULT_CONNECT_TIMEOUT);

    // Try local proxy first if enabled ("PROXY 127.0.0.1:<port>; DIRECT"), falling back to direct
    if proxy.is_enabled() {
        if url.scheme() == "https" {
            if let Ok(tcp) =
                dial_http_proxy_connect(proxy, clean_host, port, user_agent, connect_deadline)
            {
                let _ = tcp.set_read_timeout(Some(step_timeout));
                let _ = tcp.set_write_timeout(Some(step_timeout));
                return stream_over_https(
                    url,
                    tcp,
                    clean_host,
                    &authority,
                    &path_and_query,
                    user_agent,
                    tls_cfg,
                    part_path,
                    declared_size,
                    task_id,
                );
            }
        } else if let Ok(mut tcp) =
            connect_tcp_with_deadline("127.0.0.1", proxy.port, connect_deadline)
        {
            let _ = tcp.set_read_timeout(Some(step_timeout));
            let _ = tcp.set_write_timeout(Some(step_timeout));
            if let Ok(res) = stream_http11_response(
                url,
                &mut tcp,
                &authority,
                url.as_str(),
                user_agent,
                proxy.proxy_authorization_header().as_deref(),
                part_path,
                declared_size,
                task_id,
            ) {
                return Ok(res);
            }
        }
    }

    let mut tcp = connect_tcp_with_deadline(clean_host, port, connect_deadline)
        .map_err(|e| StreamError::Network(e.to_string()))?;
    let _ = tcp.set_read_timeout(Some(step_timeout));
    let _ = tcp.set_write_timeout(Some(step_timeout));

    if url.scheme() == "https" {
        stream_over_https(
            url,
            tcp,
            clean_host,
            &authority,
            &path_and_query,
            user_agent,
            tls_cfg,
            part_path,
            declared_size,
            task_id,
        )
    } else {
        stream_http11_response(
            url,
            &mut tcp,
            &authority,
            &path_and_query,
            user_agent,
            None,
            part_path,
            declared_size,
            task_id,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn stream_over_https(
    url: &Url,
    tcp: std::net::TcpStream,
    clean_host: &str,
    authority: &str,
    path_and_query: &str,
    user_agent: &str,
    tls_cfg: &Arc<ClientConfig>,
    part_path: &Path,
    declared_size: u64,
    task_id: u64,
) -> Result<AttemptOutcome, StreamError> {
    let server_name = ServerName::try_from(clean_host.to_string())
        .map_err(|e| StreamError::Network(format!("tls server name: {e}")))?;
    let conn = ClientConnection::new(tls_cfg.clone(), server_name)
        .map_err(|e| StreamError::Network(format!("tls handshake: {e}")))?;
    let mut tls_stream = StreamOwned::new(conn, tcp);
    stream_http11_response(
        url,
        &mut tls_stream,
        authority,
        path_and_query,
        user_agent,
        None,
        part_path,
        declared_size,
        task_id,
    )
}

#[allow(clippy::too_many_arguments)]
fn stream_http11_response<S: Read + Write>(
    base_url: &Url,
    stream: &mut S,
    authority: &str,
    request_target: &str,
    user_agent: &str,
    proxy_auth: Option<&str>,
    part_path: &Path,
    declared_size: u64,
    task_id: u64,
) -> Result<AttemptOutcome, StreamError> {
    if is_cancelled(task_id) {
        return Err(StreamError::Cancelled);
    }
    let mut req = format!("GET {request_target} HTTP/1.1\r\nHost: {authority}\r\n");
    if !user_agent.is_empty() {
        req.push_str(&format!("User-Agent: {user_agent}\r\n"));
    }
    if let Some(auth) = proxy_auth {
        req.push_str(&format!("Proxy-Authorization: {auth}\r\n"));
    }
    req.push_str("Accept: */*\r\nConnection: close\r\n\r\n");

    stream
        .write_all(req.as_bytes())
        .map_err(|e| StreamError::Network(e.to_string()))?;
    stream
        .flush()
        .map_err(|e| StreamError::Network(e.to_string()))?;

    let mut reader = BufReader::with_capacity(STREAM_CHUNK_SIZE, stream);
    let mut status_line = String::new();
    if reader
        .read_line(&mut status_line)
        .map_err(|e| StreamError::Network(e.to_string()))?
        == 0
    {
        return Err(StreamError::Network("unexpected EOF reading status".to_string()));
    }
    let code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| StreamError::Network("malformed HTTP status".to_string()))?;

    let mut content_length: Option<u64> = None;
    let mut is_chunked = false;
    let mut location: Option<String> = None;

    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .map_err(|e| StreamError::Network(e.to_string()))?;
        if n == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let name = k.trim();
            let val = v.trim();
            if name.eq_ignore_ascii_case("content-length") {
                content_length = val.parse::<u64>().ok();
            } else if name.eq_ignore_ascii_case("transfer-encoding")
                && val.to_ascii_lowercase().contains("chunked")
            {
                is_chunked = true;
            } else if name.eq_ignore_ascii_case("location") {
                location = Some(val.to_string());
            }
        }
    }

    if matches!(code, 301 | 302 | 303 | 307 | 308) {
        if let Some(loc) = location {
            if let Ok(next) = base_url.join(&loc) {
                if matches!(next.scheme(), "http" | "https") {
                    return Ok(AttemptOutcome::Redirect(next));
                }
            }
        }
    }

    if !(200..300).contains(&code) {
        return Err(StreamError::HttpStatus(code));
    }

    let total_hint = content_length.filter(|&c| c > 0).unwrap_or(declared_size);
    update_progress(task_id, 0, total_hint);

    let mut out_file =
        File::create(part_path).map_err(|e| StreamError::Network(format!("create part file: {e}")))?;
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;

    if is_chunked {
        loop {
            if is_cancelled(task_id) {
                return Err(StreamError::Cancelled);
            }
            let mut size_line = String::new();
            if reader
                .read_line(&mut size_line)
                .map_err(|e| StreamError::Network(e.to_string()))?
                == 0
            {
                break;
            }
            let hex_str = size_line.trim().split(';').next().unwrap_or("").trim();
            if hex_str.is_empty() {
                continue;
            }
            let chunk_len = u64::from_str_radix(hex_str, 16)
                .map_err(|_| StreamError::Network("invalid chunk size".to_string()))?;
            if chunk_len == 0 {
                break;
            }
            let mut rem = chunk_len;
            let mut buf = vec![0u8; STREAM_CHUNK_SIZE];
            while rem > 0 {
                if is_cancelled(task_id) {
                    return Err(StreamError::Cancelled);
                }
                let want = (rem as usize).min(buf.len());
                reader
                    .read_exact(&mut buf[..want])
                    .map_err(|e| StreamError::Network(e.to_string()))?;
                out_file
                    .write_all(&buf[..want])
                    .map_err(|e| StreamError::Network(e.to_string()))?;
                hasher.update(&buf[..want]);
                received += want as u64;
                rem -= want as u64;
                update_progress(task_id, received, total_hint);
            }
            let mut crlf = [0u8; 2];
            let _ = reader.read_exact(&mut crlf);
        }
    } else {
        let mut buf = vec![0u8; STREAM_CHUNK_SIZE];
        let mut remaining = content_length;
        loop {
            if is_cancelled(task_id) {
                return Err(StreamError::Cancelled);
            }
            let max_read = match remaining {
                Some(0) => break,
                Some(r) => (r as usize).min(buf.len()),
                None => buf.len(),
            };
            let n = reader
                .read(&mut buf[..max_read])
                .map_err(|e| StreamError::Network(e.to_string()))?;
            if n == 0 {
                if let Some(rem) = remaining {
                    if rem > 0 {
                        return Err(StreamError::Network(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "unexpected EOF during download",
                        )
                        .to_string()));
                    }
                }
                break;
            }
            out_file
                .write_all(&buf[..n])
                .map_err(|e| StreamError::Network(e.to_string()))?;
            hasher.update(&buf[..n]);
            received += n as u64;
            if let Some(ref mut r) = remaining {
                *r -= n as u64;
            }
            update_progress(task_id, received, total_hint);
        }
    }

    out_file
        .sync_all()
        .map_err(|e| StreamError::Network(e.to_string()))?;
    drop(out_file);

    Ok(AttemptOutcome::Downloaded {
        bytes_written: received,
        actual_sha256_hex: hex_encode_lower(&hasher.finalize()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn streaming_update_download_verifies_sha256_follows_redirects_and_reuses_cache() {
        let payload = b"trusted update payload for streaming sha256 verification!";
        let digest_hex = hex_encode_lower(&Sha256::digest(payload));
        let expected_digest = format!("sha256:{}", digest_hex.to_ascii_uppercase());

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("Hiddify-Update.apk");

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let payload_vec = payload.to_vec();

        thread::spawn(move || {
            // Request 1: 302 redirect -> /cdn/file.apk
            if let Ok((mut s1, _)) = listener.accept() {
                let mut r = BufReader::new(&mut s1);
                loop {
                    let mut l = String::new();
                    if r.read_line(&mut l).unwrap_or(0) == 0 || l == "\r\n" {
                        break;
                    }
                }
                let resp = format!(
                    "HTTP/1.1 302 Found\r\nLocation: http://{addr}/cdn/file.apk\r\nContent-Length: 0\r\n\r\n"
                );
                let _ = s1.write_all(resp.as_bytes());
            }
            // Request 2: serve payload
            if let Ok((mut s2, _)) = listener.accept() {
                let mut r = BufReader::new(&mut s2);
                loop {
                    let mut l = String::new();
                    if r.read_line(&mut l).unwrap_or(0) == 0 || l == "\r\n" {
                        break;
                    }
                }
                let hdr = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                    payload_vec.len()
                );
                let _ = s2.write_all(hdr.as_bytes());
                let _ = s2.write_all(&payload_vec);
            }
            // Request 3: tampered payload
            if let Ok((mut s3, _)) = listener.accept() {
                let mut r = BufReader::new(&mut s3);
                loop {
                    let mut l = String::new();
                    if r.read_line(&mut l).unwrap_or(0) == 0 || l == "\r\n" {
                        break;
                    }
                }
                let mut bad = payload_vec.clone();
                bad[0] ^= 0xff;
                let hdr = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", bad.len());
                let _ = s3.write_all(hdr.as_bytes());
                let _ = s3.write_all(&bad);
            }
        });

        let task_id = crate::cancel::create_task();
        let res = download_and_verify_update(&UpdateDownloadRequest {
            url: format!("http://{addr}/release/latest"),
            destination_path: dest.to_string_lossy().into_owned(),
            expected_size: payload.len() as u64,
            expected_sha256: expected_digest.clone(),
            user_agent: "HiddifyUpdateTest/1.0".to_string(),
            proxy_port: 0,
            proxy_secret: String::new(),
            cancel_task_id: task_id,
            timeout_ms: 5_000,
        });
        assert_eq!(res.status, "ok", "{res:?}");
        assert_eq!(fs::read(&dest).unwrap(), payload);
        crate::cancel::remove_task(task_id);

        // Calling again with existing verified file hits cache ("reused") without hitting socket!
        let reused = download_and_verify_update(&UpdateDownloadRequest {
            url: format!("http://{addr}/unused"),
            destination_path: dest.to_string_lossy().into_owned(),
            expected_size: payload.len() as u64,
            expected_sha256: expected_digest.clone(),
            user_agent: String::new(),
            proxy_port: 0,
            proxy_secret: String::new(),
            cancel_task_id: 0,
            timeout_ms: 5_000,
        });
        assert_eq!(reused.status, "reused");

        // Tampered payload on new destination is rejected and cleaned up!
        let bad_dest = dir.path().join("Tampered.apk");
        let bad_res = download_and_verify_update(&UpdateDownloadRequest {
            url: format!("http://{addr}/cdn/tampered"),
            destination_path: bad_dest.to_string_lossy().into_owned(),
            expected_size: payload.len() as u64,
            expected_sha256: expected_digest,
            user_agent: String::new(),
            proxy_port: 0,
            proxy_secret: String::new(),
            cancel_task_id: 0,
            timeout_ms: 5_000,
        });
        assert_eq!(bad_res.status, "sha256_mismatch");
        assert!(!bad_dest.exists());
    }
}
