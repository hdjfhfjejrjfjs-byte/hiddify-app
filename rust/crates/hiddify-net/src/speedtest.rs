//! Standalone `speedtest.net` client (`cmd/cmd_speedtest.go` + `showwin/speedtest-go` equivalent):
//! - Direct or SOCKS5 (`socks://[user:pass@]host:port`) transport with Chrome User-Agent
//! - User geolocation (`speedtest-config.php`) & server discovery (`/api/js/servers` + XML fallback)
//! - Haversine distance (`R = 6378.137 km`) & closest-to-`target_km` server selection
//! - HTTP Ping + Jitter (`<dir>/latency.txt`, 1 warm-up + 10 RTT samples)
//! - Multi-threaded Download (`random1000x1000.jpg`) & Upload (`1,499,490` bytes `POST`) with
//!   5s/50ms Welford EWMA stability convergence (`CV < 0.03`) and `DecimalBits` formatting.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use regex::Regex;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::fetch_url::system_and_bundled_roots;
use crate::health_check::format_health_duration;
use crate::proxy_dial::{connect_tcp_with_deadline, dial_socks5_tcp, Socks5Spec};

pub const SPEEDTEST_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
pub const SPEEDTEST_CONFIG_URL: &str = "https://www.speedtest.net/speedtest-config.php";
pub const SPEEDTEST_SERVERS_JSON_URL: &str = "https://www.speedtest.net/api/js/servers";
pub const SPEEDTEST_SERVERS_XML_URL: &str = "https://www.speedtest.net/speedtest-servers-static.php";

const CAPTURE_DURATION: Duration = Duration::from_secs(15);
const CAPTURE_FREQ: Duration = Duration::from_millis(50);
const UPLOAD_CHUNK_BYTES: usize = (1500 * 100 - 51) * 10; // ulSizes[4] = 1500 -> 1,499,490 bytes

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeedtestOptions {
    #[serde(default)]
    pub proxy_addr: String,
    #[serde(default = "default_target_distance")]
    pub target_distance_km: f64,
    #[serde(default)]
    pub no_download: bool,
    #[serde(default)]
    pub no_upload: bool,
    #[serde(default)]
    pub no_ping: bool,
    #[serde(default)]
    pub no_jitter: bool,
    #[serde(default)]
    pub verbose: u8,
    /// Optional custom endpoint URLs `(config_url, servers_url)` for deterministic unit tests.
    #[serde(default)]
    pub endpoint_override: Option<(String, String)>,
}

fn default_target_distance() -> f64 {
    2000.0
}

impl Default for SpeedtestOptions {
    fn default() -> Self {
        Self {
            proxy_addr: String::new(),
            target_distance_km: 2000.0,
            no_download: false,
            no_upload: false,
            no_ping: false,
            no_jitter: false,
            verbose: 0,
            endpoint_override: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeedtestServer {
    pub url: String,
    pub lat: String,
    pub lon: String,
    pub name: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub sponsor: String,
    pub id: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub distance: f64,
}

impl SpeedtestServer {
    pub fn display_string(&self) -> String {
        if self.sponsor == "?" {
            format!("[{:>4}] {}", self.id, self.name)
        } else if self.country.is_empty() {
            format!(
                "[{:>4}] {:.2}km {} by {}",
                self.id, self.distance, self.name, self.sponsor
            )
        } else {
            format!(
                "[{:>4}] {:.2}km {} ({}) by {}",
                self.id, self.distance, self.name, self.country, self.sponsor
            )
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeedtestStepResult {
    pub name: String,
    pub ok: bool,
    pub value: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeedtestReport {
    pub ok: bool,
    pub server: Option<String>,
    pub latency_us: Option<u64>,
    pub jitter_us: Option<u64>,
    /// Bytes per second (`-1.0` for N/A).
    pub download_bps_bytes: Option<f64>,
    pub upload_bps_bytes: Option<f64>,
    pub steps: Vec<SpeedtestStepResult>,
    pub fatal_error: Option<String>,
}

// ============================================================================
// Math, Units & Welford EWMA (`unit.go`, `server.go`, `welford.go`)
// ============================================================================

/// Haversine great-circle distance in km (`R = 6378.137` matching `speedtest-go/server.go:341`).
pub fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let radius = 6378.137_f64;
    let phi1 = lat1.to_radians();
    let phi2 = lat2.to_radians();
    let dphi_half = (lat1 - lat2) * std::f64::consts::PI / 360.0;
    let dlambda_half = (lon1 - lon2) * std::f64::consts::PI / 360.0;
    let sin_sq = dphi_half.sin().powi(2) + phi1.cos() * phi2.cos() * dlambda_half.sin().powi(2);
    let delta = 2.0 * sin_sq.sqrt().atan2((1.0 - sin_sq).max(0.0).sqrt());
    radius * delta
}

/// Formats a byte-per-second rate identically to `formatSpeedtestRate` in `cmd_speedtest.go`
/// (`UnitTypeDecimalBits` = `0`, spaces stripped: `"125.40Mbps"`, `"0.00Mbps"`, `"N/A"`).
pub fn format_speedtest_rate(bytes_per_sec: f64) -> String {
    if bytes_per_sec == 0.0 {
        return "0.00Mbps".to_string();
    }
    if bytes_per_sec < 0.0 {
        return "N/A".to_string();
    }
    let bits = bytes_per_sec * 8.0;
    if bytes_per_sec >= 1_000_000_000.0 {
        format!("{:.2}Gbps", bits / 1_000_000_000.0)
    } else if bytes_per_sec >= 1_000_000.0 {
        format!("{:.2}Mbps", bits / 1_000_000.0)
    } else if bytes_per_sec >= 1_000.0 {
        format!("{:.2}Kbps", bits / 1_000.0)
    } else {
        format!("{:.2}bps", bits)
    }
}

/// Picks the server whose distance is closest to `target_km` (`selectSpeedtestServer` in `cmd_speedtest.go`).
pub fn select_speedtest_server(
    servers: &[SpeedtestServer],
    target_km: f64,
) -> Result<SpeedtestServer, String> {
    if servers.is_empty() {
        return Err("no speedtest server found".to_string());
    }
    let mut best_idx = 0usize;
    let mut best_delta = f64::MAX;
    for (i, srv) in servers.iter().enumerate() {
        let delta = (srv.distance - target_km).abs();
        if delta < best_delta {
            best_delta = delta;
            best_idx = i;
        }
    }
    Ok(servers[best_idx].clone())
}

/// Mean and standard deviation in nanoseconds (`request.go:StandardDeviation`).
pub fn mean_and_stddev_ns(samples_ns: &[i64]) -> (Duration, Duration) {
    if samples_ns.is_empty() {
        return (Duration::ZERO, Duration::ZERO);
    }
    let n = samples_ns.len() as i128;
    let sum: i128 = samples_ns.iter().map(|&x| x as i128).sum();
    let mean = sum / n;
    let var_sum: i128 = samples_ns
        .iter()
        .map(|&x| {
            let d = (x as i128) - mean;
            d * d
        })
        .sum();
    let variance = (var_sum / n) as f64;
    let stddev = variance.sqrt() as u64;
    (
        Duration::from_nanos(mean.max(0) as u64),
        Duration::from_nanos(stddev),
    )
}

/// Exact port of `speedtest-go/speedtest/internal/welford.go`.
pub struct Welford {
    n: usize,
    cap: usize,
    vector: Vec<f64>,
    moving_vector: Vec<f64>,
    mean: f64,
    sum: f64,
    erase_index: usize,
    current_stddev: f64,
    consecutive_stable: usize,
    stable_threshold: usize,
    cv: f64,
    ewma_mean: f64,
    steps: usize,
    min_steps: usize,
    beta: f64,
    scale: f64,
    moving_avg: f64,
}

impl Welford {
    pub fn new(cycle: Duration, frequency: Duration) -> Self {
        let window_size = (cycle.as_millis() / frequency.as_millis()).max(2) as usize;
        Self {
            n: 0,
            cap: window_size,
            vector: vec![0.0; window_size],
            moving_vector: vec![0.0; window_size],
            mean: 0.0,
            sum: 0.0,
            erase_index: 0,
            current_stddev: 0.0,
            consecutive_stable: 0,
            stable_threshold: window_size / 3,
            cv: 0.0,
            ewma_mean: 0.0,
            steps: 0,
            min_steps: window_size * 2,
            beta: 2.0 / (window_size as f64 + 1.0),
            scale: 1000.0 / (frequency.as_millis().max(1) as f64),
            moving_avg: 0.0,
        }
    }

    pub fn update(&mut self, global_avg: f64, raw_delta_bytes: f64) -> bool {
        let value = raw_delta_bytes * self.scale;
        if self.n == self.cap {
            let delta = self.vector[self.erase_index] - self.mean;
            self.mean -= delta / ((self.n - 1) as f64);
            self.sum -= delta * (self.vector[self.erase_index] - self.mean);
            if self.sum < 0.0 {
                self.sum = 0.0;
            }
            self.vector[self.erase_index] = global_avg;
            self.moving_avg -= self.moving_vector[self.erase_index];
            self.moving_vector[self.erase_index] = value;
            self.moving_avg += value;
            self.erase_index = (self.erase_index + 1) % self.cap;
        } else {
            self.vector[self.n] = global_avg;
            self.moving_vector[self.n] = value;
            self.moving_avg += value;
            self.n += 1;
        }
        let delta = global_avg - self.mean;
        self.mean += delta / (self.n as f64);
        self.sum += delta * (global_avg - self.mean);
        let variance = if self.n < 2 {
            0.0
        } else {
            self.sum / ((self.n - 1) as f64)
        };
        self.current_stddev = variance.max(0.0).sqrt();
        if self.mean != 0.0 {
            self.cv = self.current_stddev / self.mean;
        }
        self.ewma_mean = value * self.beta + self.ewma_mean * (1.0 - self.beta);
        if self.n == self.cap && self.cv < 0.03 {
            self.consecutive_stable += 1;
        } else if self.consecutive_stable > 0 {
            self.consecutive_stable -= 1;
        }
        self.steps += 1;
        self.consecutive_stable >= self.stable_threshold && self.steps > self.min_steps
    }

    pub fn ewma(&self) -> f64 {
        if self.n == 0 {
            return 0.0;
        }
        self.ewma_mean * 0.5 + (self.moving_avg / (self.n as f64)) * 0.5
    }
}

// ============================================================================
// XML & JSON Parsers (`speedtest-config.php` & `speedtest-servers-static.php`)
// ============================================================================

fn extract_xml_attr(tag_text: &str, attr_name: &str) -> Option<String> {
    let pattern = format!(r#"\b{attr_name}\s*=\s*"([^"]*)""#);
    let re = Regex::new(&pattern).ok()?;
    re.captures(tag_text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

pub fn parse_user_lat_lon_xml(xml: &str) -> Result<(f64, f64), String> {
    let client_re = Regex::new(r"(?is)<client\b([^>]*)/?>").unwrap();
    let caps = client_re
        .captures(xml)
        .ok_or_else(|| "failed to fetch user information".to_string())?;
    let attrs = caps.get(1).map(|m| m.as_str()).unwrap_or("");
    let lat_str = extract_xml_attr(attrs, "lat").unwrap_or_default();
    let lon_str = extract_xml_attr(attrs, "lon").unwrap_or_default();
    let lat = lat_str.parse::<f64>().unwrap_or(0.0);
    let lon = lon_str.parse::<f64>().unwrap_or(0.0);
    Ok((lat, lon))
}

pub fn parse_servers_xml(xml: &str) -> Vec<SpeedtestServer> {
    let server_re = Regex::new(r"(?is)<server\b([^>]*)/?>").unwrap();
    let mut out = Vec::new();
    for caps in server_re.captures_iter(xml) {
        let attrs = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let Some(url) = extract_xml_attr(attrs, "url").filter(|u| !u.is_empty()) else {
            continue;
        };
        out.push(SpeedtestServer {
            url,
            lat: extract_xml_attr(attrs, "lat").unwrap_or_default(),
            lon: extract_xml_attr(attrs, "lon").unwrap_or_default(),
            name: extract_xml_attr(attrs, "name").unwrap_or_default(),
            country: extract_xml_attr(attrs, "country").unwrap_or_default(),
            sponsor: extract_xml_attr(attrs, "sponsor").unwrap_or_default(),
            id: extract_xml_attr(attrs, "id").unwrap_or_default(),
            host: extract_xml_attr(attrs, "host").unwrap_or_default(),
            distance: 0.0,
        });
    }
    out
}

// ============================================================================
// HTTP/1.1 Transport over Direct or SOCKS5
// ============================================================================

struct SpeedtestTransport {
    socks: Option<Socks5Spec>,
    tls_cfg: Arc<ClientConfig>,
}

impl SpeedtestTransport {
    fn new(proxy_addr: &str, custom_roots: Option<Arc<RootCertStore>>) -> Result<Self, String> {
        let socks = if proxy_addr.trim().is_empty() {
            None
        } else {
            Some(Socks5Spec::parse(proxy_addr)?)
        };
        let roots = custom_roots.unwrap_or_else(system_and_bundled_roots);
        let provider = Arc::new(rustls_rustcrypto::provider());
        let mut cfg = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_root_certificates(roots)
            .with_no_client_auth();
        cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Self {
            socks,
            tls_cfg: Arc::new(cfg),
        })
    }

    fn connect_stream(
        &self,
        url: &Url,
        timeout: Duration,
    ) -> io::Result<(Box<dyn ReadWrite>, String, String)> {
        let host = url
            .host_str()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing URL host"))?;
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

        let deadline = Instant::now() + timeout;
        let tcp = match &self.socks {
            Some(spec) => dial_socks5_tcp(spec, clean_host, port, deadline)?,
            None => connect_tcp_with_deadline(clean_host, port, deadline)?,
        };
        let _ = tcp.set_read_timeout(Some(timeout));
        let _ = tcp.set_write_timeout(Some(timeout));

        if url.scheme() == "https" {
            let sni = ServerName::try_from(clean_host.to_string())
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
            let conn = ClientConnection::new(self.tls_cfg.clone(), sni)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
            Ok((Box::new(StreamOwned::new(conn, tcp)), authority, path_and_query))
        } else {
            Ok((Box::new(tcp), authority, path_and_query))
        }
    }

    fn get_bytes(&self, url_str: &str, timeout: Duration) -> Result<Vec<u8>, String> {
        let mut cur = Url::parse(url_str).map_err(|e| e.to_string())?;
        for _ in 0..5 {
            let (mut stream, authority, pq) =
                self.connect_stream(&cur, timeout).map_err(|e| e.to_string())?;
            let req = format!(
                "GET {pq} HTTP/1.1\r\nHost: {authority}\r\nUser-Agent: {SPEEDTEST_USER_AGENT}\r\nAccept: */*\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
            stream.flush().map_err(|e| e.to_string())?;

            let mut reader = BufReader::new(stream);
            let (code, cl, chunked, loc) = read_resp_headers(&mut reader)?;
            if matches!(code, 301 | 302 | 303 | 307 | 308) {
                if let Some(l) = loc {
                    if let Ok(next) = cur.join(&l) {
                        cur = next;
                        continue;
                    }
                }
            }
            if !(200..300).contains(&code) {
                return Err(format!("HTTP {code}"));
            }
            return read_resp_body(&mut reader, cl, chunked);
        }
        Err("too many redirects".to_string())
    }
}

trait ReadWrite: Read + Write + Send {}
impl<T: Read + Write + Send> ReadWrite for T {}

fn read_resp_headers<R: BufRead>(
    reader: &mut R,
) -> Result<(u16, Option<usize>, bool, Option<String>), String> {
    let mut status_line = String::new();
    if reader
        .read_line(&mut status_line)
        .map_err(|e| e.to_string())?
        == 0
    {
        return Err("EOF reading status".to_string());
    }
    let code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| "invalid HTTP status".to_string())?;
    let mut cl = None;
    let mut chunked = false;
    let mut loc = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0
            || line == "\r\n"
            || line == "\n"
        {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim();
            let v = v.trim();
            if k.eq_ignore_ascii_case("content-length") {
                cl = v.parse::<usize>().ok();
            } else if k.eq_ignore_ascii_case("transfer-encoding")
                && v.to_ascii_lowercase().contains("chunked")
            {
                chunked = true;
            } else if k.eq_ignore_ascii_case("location") {
                loc = Some(v.to_string());
            }
        }
    }
    Ok((code, cl, chunked, loc))
}

fn read_resp_body<R: BufRead>(
    reader: &mut R,
    content_length: Option<usize>,
    chunked: bool,
) -> Result<Vec<u8>, String> {
    if chunked {
        let mut out = Vec::new();
        loop {
            let mut sline = String::new();
            if reader.read_line(&mut sline).map_err(|e| e.to_string())? == 0 {
                break;
            }
            let hex = sline.trim().split(';').next().unwrap_or("").trim();
            if hex.is_empty() {
                continue;
            }
            let sz = usize::from_str_radix(hex, 16).map_err(|e| e.to_string())?;
            if sz == 0 {
                break;
            }
            let start = out.len();
            out.resize(start + sz, 0);
            reader
                .read_exact(&mut out[start..start + sz])
                .map_err(|e| e.to_string())?;
            let mut crlf = [0u8; 2];
            let _ = reader.read_exact(&mut crlf);
        }
        Ok(out)
    } else if let Some(len) = content_length {
        let mut buf = vec![0u8; len];
        reader.read_exact(&mut buf).map_err(|e| e.to_string())?;
        Ok(buf)
    } else {
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        Ok(buf)
    }
}

fn sibling_server_url(server_url: &str, leaf_file: &str) -> Result<Url, String> {
    let mut u = Url::parse(server_url).map_err(|e| e.to_string())?;
    let path = u.path().to_string();
    let dir = match path.rfind('/') {
        Some(0) | None => "/",
        Some(idx) => &path[..idx],
    };
    let joined = if dir.ends_with('/') {
        format!("{dir}{leaf_file}")
    } else {
        format!("{dir}/{leaf_file}")
    };
    u.set_path(&joined);
    u.set_query(None);
    Ok(u)
}

fn run_http_ping(
    transport: &SpeedtestTransport,
    server_url: &str,
    echo_times: usize,
    interval: Duration,
    timeout: Duration,
) -> Result<(Duration, Duration), String> {
    let ping_url = sibling_server_url(server_url, "latency.txt")?;
    let mut samples_ns = Vec::with_capacity(echo_times);
    let total_iters = echo_times + 1; // 1 warm-up + echo_times measured (`request.go:314`)
    let mut fails = 0usize;

    for i in 0..total_iters {
        let t0 = Instant::now();
        let ok = match transport.connect_stream(&ping_url, timeout) {
            Ok((mut s, auth, pq)) => {
                let req = format!(
                    "GET {pq} HTTP/1.1\r\nHost: {auth}\r\nUser-Agent: {SPEEDTEST_USER_AGENT}\r\nConnection: close\r\n\r\n"
                );
                if s.write_all(req.as_bytes()).is_ok() && s.flush().is_ok() {
                    let _ = io::copy(&mut s, &mut io::sink());
                    true
                } else {
                    false
                }
            }
            Err(_) => false,
        };
        let rtt = t0.elapsed();
        if !ok {
            fails += 1;
            continue;
        }
        if i > 0 {
            samples_ns.push(rtt.as_nanos() as i64);
        }
        if !interval.is_zero() && i + 1 < total_iters {
            thread::sleep(interval);
        }
    }

    if fails == total_iters || samples_ns.is_empty() {
        return Err("server connect timeout".to_string());
    }
    Ok(mean_and_stddev_ns(&samples_ns))
}

fn measure_direction<F>(workers: usize, max_duration: Duration, task_fn: F) -> f64
where
    F: Fn(&AtomicBool, &AtomicU64) -> bool + Sync,
{
    let running = AtomicBool::new(true);
    let total_bytes = AtomicU64::new(0);
    let req_count = AtomicI64::new(0);
    let err_count = AtomicI64::new(0);
    let n_threads = workers.clamp(1, 16);

    let mut final_ewma = 0.0_f64;

    thread::scope(|s| {
        for _ in 0..n_threads {
            s.spawn(|| {
                while running.load(Ordering::Acquire) {
                    req_count.fetch_add(1, Ordering::Relaxed);
                    if !task_fn(&running, &total_bytes) {
                        err_count.fetch_add(1, Ordering::Relaxed);
                        if !running.load(Ordering::Acquire) {
                            break;
                        }
                        thread::sleep(Duration::from_millis(20));
                    }
                }
            });
        }

        // Rate capture loop (`data_manager.go:266`)
        let mut welford = Welford::new(Duration::from_secs(5), CAPTURE_FREQ);
        let start = Instant::now();
        let mut prev_vol = 0u64;
        while running.load(Ordering::Acquire) && start.elapsed() < max_duration {
            thread::sleep(CAPTURE_FREQ);
            let cur_vol = total_bytes.load(Ordering::Relaxed);
            let delta = cur_vol.saturating_sub(prev_vol);
            prev_vol = cur_vol;
            let elapsed_ms = start.elapsed().as_millis().max(1) as f64;
            let global_avg = (cur_vol as f64) / elapsed_ms * 1000.0;
            if welford.update(global_avg, delta as f64) {
                running.store(false, Ordering::Release);
                break;
            }
        }
        running.store(false, Ordering::Release);
        final_ewma = welford.ewma();
    });

    let reqs = req_count.load(Ordering::Relaxed);
    let errs = err_count.load(Ordering::Relaxed);
    if final_ewma == 0.0 && reqs > 0 && (errs as f64) / (reqs as f64) > 0.1 {
        -1.0
    } else {
        final_ewma
    }
}

fn run_download_test(
    transport: &SpeedtestTransport,
    server_url: &str,
    workers: usize,
    max_duration: Duration,
) -> Result<f64, String> {
    let dl_url = sibling_server_url(server_url, "random1000x1000.jpg")?;
    let rate = measure_direction(workers, max_duration, |running, total_bytes| {
        let Ok((mut stream, auth, pq)) = transport.connect_stream(&dl_url, Duration::from_secs(10)) else {
            return false;
        };
        let req = format!(
            "GET {pq} HTTP/1.1\r\nHost: {auth}\r\nUser-Agent: {SPEEDTEST_USER_AGENT}\r\nConnection: close\r\n\r\n"
        );
        if stream.write_all(req.as_bytes()).is_err() || stream.flush().is_err() {
            return false;
        }
        let mut reader = BufReader::with_capacity(16 * 1024, stream);
        let Ok((code, _, _, _)) = read_resp_headers(&mut reader) else {
            return false;
        };
        if !(200..300).contains(&code) {
            return false;
        }
        let mut buf = [0u8; 8192];
        while running.load(Ordering::Acquire) {
            match reader.read(&mut buf) {
                Ok(0) => return true,
                Ok(n) => {
                    total_bytes.fetch_add(n as u64, Ordering::Relaxed);
                }
                Err(_) => return false,
            }
        }
        true
    });
    Ok(rate)
}

fn run_upload_test(
    transport: &SpeedtestTransport,
    server_url: &str,
    workers: usize,
    max_duration: Duration,
) -> Result<f64, String> {
    let ul_url = Url::parse(server_url).map_err(|e| e.to_string())?;
    let block = [0xAAu8; 8192];
    let rate = measure_direction(workers, max_duration, |running, total_bytes| {
        let Ok((mut stream, auth, pq)) = transport.connect_stream(&ul_url, Duration::from_secs(10)) else {
            return false;
        };
        let hdr = format!(
            "POST {pq} HTTP/1.1\r\nHost: {auth}\r\nUser-Agent: {SPEEDTEST_USER_AGENT}\r\nContent-Type: application/octet-stream\r\nContent-Length: {UPLOAD_CHUNK_BYTES}\r\nConnection: close\r\n\r\n"
        );
        if stream.write_all(hdr.as_bytes()).is_err() {
            return false;
        }
        let mut left = UPLOAD_CHUNK_BYTES;
        while left > 0 && running.load(Ordering::Acquire) {
            let chunk = left.min(block.len());
            if stream.write_all(&block[..chunk]).is_err() {
                return false;
            }
            total_bytes.fetch_add(chunk as u64, Ordering::Relaxed);
            left -= chunk;
        }
        let _ = stream.flush();
        if left == 0 {
            let mut drain = [0u8; 512];
            let _ = stream.read(&mut drain);
        }
        true
    });
    Ok(rate)
}

/// Runs the complete speedtest workflow matching `cmd/cmd_speedtest.go`.
pub fn run_speedtest(opts: &SpeedtestOptions) -> SpeedtestReport {
    run_speedtest_with_roots(opts, None, CAPTURE_DURATION)
}

pub fn run_speedtest_with_roots(
    opts: &SpeedtestOptions,
    custom_roots: Option<Arc<RootCertStore>>,
    capture_duration: Duration,
) -> SpeedtestReport {
    let no_jitter = opts.no_jitter || opts.no_ping;
    let transport = match SpeedtestTransport::new(&opts.proxy_addr, custom_roots) {
        Ok(t) => t,
        Err(err) => {
            return SpeedtestReport {
                ok: false,
                server: None,
                latency_us: None,
                jitter_us: None,
                download_bps_bytes: None,
                upload_bps_bytes: None,
                steps: Vec::new(),
                fatal_error: Some(err),
            };
        }
    };

    let (cfg_url, srv_url) = opts
        .endpoint_override
        .as_ref()
        .map(|(c, s)| (c.as_str(), s.as_str()))
        .unwrap_or((SPEEDTEST_CONFIG_URL, SPEEDTEST_SERVERS_JSON_URL));

    let user_xml = match transport.get_bytes(cfg_url, Duration::from_secs(20)) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => {
            return SpeedtestReport {
                ok: false,
                server: None,
                latency_us: None,
                jitter_us: None,
                download_bps_bytes: None,
                upload_bps_bytes: None,
                steps: Vec::new(),
                fatal_error: Some(format!("fetch user info: {e}")),
            };
        }
    };
    let (user_lat, user_lon) = match parse_user_lat_lon_xml(&user_xml) {
        Ok(pair) => pair,
        Err(e) => {
            return SpeedtestReport {
                ok: false,
                server: None,
                latency_us: None,
                jitter_us: None,
                download_bps_bytes: None,
                upload_bps_bytes: None,
                steps: Vec::new(),
                fatal_error: Some(format!("fetch user info: {e}")),
            };
        }
    };

    let mut servers: Vec<SpeedtestServer> = match transport.get_bytes(srv_url, Duration::from_secs(20)) {
        Ok(bytes) if !bytes.is_empty() => {
            if let Ok(list) = serde_json::from_slice::<Vec<SpeedtestServer>>(&bytes) {
                list
            } else {
                parse_servers_xml(&String::from_utf8_lossy(&bytes))
            }
        }
        _ if opts.endpoint_override.is_none() => match transport
            .get_bytes(SPEEDTEST_SERVERS_XML_URL, Duration::from_secs(20))
        {
            Ok(xml_bytes) => parse_servers_xml(&String::from_utf8_lossy(&xml_bytes)),
            Err(e) => {
                return SpeedtestReport {
                    ok: false,
                    server: None,
                    latency_us: None,
                    jitter_us: None,
                    download_bps_bytes: None,
                    upload_bps_bytes: None,
                    steps: Vec::new(),
                    fatal_error: Some(format!("fetch servers: {e}")),
                };
            }
        },
        Err(e) => {
            return SpeedtestReport {
                ok: false,
                server: None,
                latency_us: None,
                jitter_us: None,
                download_bps_bytes: None,
                upload_bps_bytes: None,
                steps: Vec::new(),
                fatal_error: Some(format!("fetch servers: {e}")),
            };
        }
        _ => Vec::new(),
    };

    for srv in &mut servers {
        let slat = srv.lat.parse::<f64>().unwrap_or(0.0);
        let slon = srv.lon.parse::<f64>().unwrap_or(0.0);
        srv.distance = haversine_km(slat, slon, user_lat, user_lon);
    }
    servers.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap_or(std::cmp::Ordering::Equal));

    let selected = match select_speedtest_server(&servers, opts.target_distance_km) {
        Ok(s) => s,
        Err(e) => {
            return SpeedtestReport {
                ok: false,
                server: None,
                latency_us: None,
                jitter_us: None,
                download_bps_bytes: None,
                upload_bps_bytes: None,
                steps: Vec::new(),
                fatal_error: Some(e),
            };
        }
    };

    if opts.verbose >= 2 {
        eprintln!(
            "debug: using server {} (target {:.0}km)",
            selected.display_string(),
            opts.target_distance_km
        );
    }

    let workers = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    let mut steps = Vec::new();
    let mut fatal_error: Option<String> = None;
    let mut latency_us = None;
    let mut jitter_us = None;
    let mut dl_rate = None;
    let mut ul_rate = None;

    if !opts.no_ping {
        match run_http_ping(
            &transport,
            &selected.url,
            10,
            Duration::from_millis(200),
            Duration::from_secs(5),
        ) {
            Ok((lat, jit)) => {
                latency_us = Some(lat.as_micros() as u64);
                jitter_us = Some(jit.as_micros() as u64);
                let mut val = format_health_duration(lat);
                if !no_jitter {
                    val.push_str(&format!(" ±{}", format_health_duration(jit)));
                }
                steps.push(SpeedtestStepResult {
                    name: "ping".to_string(),
                    ok: true,
                    value: val,
                    error: None,
                });
            }
            Err(err) => {
                let mut val = "0s".to_string();
                if !no_jitter {
                    val.push_str(" ±0s");
                }
                steps.push(SpeedtestStepResult {
                    name: "ping".to_string(),
                    ok: false,
                    value: val,
                    error: Some(err.clone()),
                });
                fatal_error = Some(format!("ping: {err}"));
            }
        }
    }

    if !opts.no_download {
        match run_download_test(&transport, &selected.url, workers, capture_duration) {
            Ok(rate) => {
                dl_rate = Some(rate);
                steps.push(SpeedtestStepResult {
                    name: "download".to_string(),
                    ok: true,
                    value: format_speedtest_rate(rate),
                    error: None,
                });
            }
            Err(err) => {
                steps.push(SpeedtestStepResult {
                    name: "download".to_string(),
                    ok: false,
                    value: "0.00Mbps".to_string(),
                    error: Some(err.clone()),
                });
                if fatal_error.is_none() {
                    fatal_error = Some(format!("download: {err}"));
                }
            }
        }
    }

    if !opts.no_upload {
        match run_upload_test(&transport, &selected.url, workers, capture_duration) {
            Ok(rate) => {
                ul_rate = Some(rate);
                steps.push(SpeedtestStepResult {
                    name: "upload".to_string(),
                    ok: true,
                    value: format_speedtest_rate(rate),
                    error: None,
                });
            }
            Err(err) => {
                steps.push(SpeedtestStepResult {
                    name: "upload".to_string(),
                    ok: false,
                    value: "0.00Mbps".to_string(),
                    error: Some(err.clone()),
                });
                if fatal_error.is_none() {
                    fatal_error = Some(format!("upload: {err}"));
                }
            }
        }
    }

    SpeedtestReport {
        ok: fatal_error.is_none(),
        server: Some(selected.display_string()),
        latency_us,
        jitter_us,
        download_bps_bytes: dl_rate,
        upload_bps_bytes: ul_rate,
        steps,
        fatal_error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn haversine_rate_formatting_welford_and_end_to_end_speedtest_match_go() {
        assert_eq!(format_speedtest_rate(0.0), "0.00Mbps");
        assert_eq!(format_speedtest_rate(-1.0), "N/A");
        assert_eq!(format_speedtest_rate(12_500_000.0), "100.00Mbps");
        assert_eq!(format_speedtest_rate(250_000_000.0), "2000.00Mbps");
        assert_eq!(format_speedtest_rate(123_123_123.123), "984.98Mbps");
        assert_eq!(format_speedtest_rate(1_231_231_231.123), "9.85Gbps");
        assert_eq!(format_speedtest_rate(123_123.123), "984.98Kbps");
        assert_eq!(format_speedtest_rate(123.1), "984.80bps");

        // Tokyo (35.6895, 139.6917) -> Seoul (37.5665, 126.9780) ~ 1150-1160 km
        let d_km = haversine_km(35.6895, 139.6917, 37.5665, 126.9780);
        assert!((1140.0..1170.0).contains(&d_km), "d_km={d_km}");

        // End-to-end loopback speedtest server (config XML + servers JSON + latency.txt + jpg + upload.php)
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            let jpg_body = vec![0x55u8; 64 * 1024];
            for stream in listener.incoming().flatten() {
                let jpg = jpg_body.clone();
                thread::spawn(move || {
                    let mut s = stream;
                    let mut r = BufReader::new(s.try_clone().unwrap());
                    let mut req_line = String::new();
                    if r.read_line(&mut req_line).unwrap_or(0) == 0 {
                        return;
                    }
                    let mut content_len = 0usize;
                    loop {
                        let mut h = String::new();
                        if r.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                            break;
                        }
                        if let Some((k, v)) = h.split_once(':') {
                            if k.trim().eq_ignore_ascii_case("content-length") {
                                content_len = v.trim().parse().unwrap_or(0);
                            }
                        }
                    }
                    if content_len > 0 {
                        let _ = io::copy(&mut r.take(content_len as u64), &mut io::sink());
                    }
                    let path = req_line.split_whitespace().nth(1).unwrap_or("/");
                    match path {
                        "/speedtest-config.php" => {
                            let xml = r#"<?xml version="1.0"?><settings><client ip="1.2.3.4" lat="35.6895" lon="139.6917" isp="TestISP"/></settings>"#;
                            let resp = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{xml}",
                                xml.len()
                            );
                            let _ = s.write_all(resp.as_bytes());
                        }
                        "/api/js/servers" => {
                            let json = format!(
                                r#"[{{"url":"http://{addr}/speedtest/upload.php","lat":"37.5665","lon":"126.9780","name":"Seoul","country":"KR","sponsor":"TestTel","id":"101","host":"{addr}"}}]"#
                            );
                            let resp = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{json}",
                                json.len()
                            );
                            let _ = s.write_all(resp.as_bytes());
                        }
                        "/speedtest/latency.txt" => {
                            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\ntest=test\n");
                        }
                        "/speedtest/random1000x1000.jpg" => {
                            let hdr = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                                jpg.len()
                            );
                            let _ = s.write_all(hdr.as_bytes());
                            let _ = s.write_all(&jpg);
                        }
                        "/speedtest/upload.php" => {
                            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
                        }
                        _ => {
                            let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
                        }
                    }
                });
            }
        });

        let report = run_speedtest_with_roots(
            &SpeedtestOptions {
                proxy_addr: String::new(),
                target_distance_km: 1200.0,
                no_download: false,
                no_upload: false,
                no_ping: false,
                no_jitter: false,
                verbose: 0,
                endpoint_override: Some((
                    format!("http://{addr}/speedtest-config.php"),
                    format!("http://{addr}/api/js/servers"),
                )),
            },
            None,
            Duration::from_millis(250),
        );

        assert!(report.ok, "speedtest failed: {report:?}");
        assert_eq!(report.steps.len(), 3);
        assert_eq!(report.steps[0].name, "ping");
        assert!(report.steps[0].value.contains('±'));
        assert_eq!(report.steps[1].name, "download");
        assert_eq!(report.steps[2].name, "upload");
        assert!(report.download_bps_bytes.unwrap() > 0.0);
        assert!(report.upload_bps_bytes.unwrap() > 0.0);
    }
}
