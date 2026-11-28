//! Ezytel Telegram public-channel viewer service, domain-fronted HTTP/TLS fetcher,
//! Persian (Jalali) calendar formatter, 8-worker image cache, and native h2c gRPC server.
//!
//! 100% behavioral and wire-level port of `hiddify-core/v2/ezytel/ezytel_service_imp.go`,
//! `v2/ezytel/ezytel.proto`, `v2/ezytel/ezytel_service.proto`, and `cmd/cmd_ezytel.go`.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use flate2::read::GzDecoder;
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use rand::Rng;
use regex::Regex;
use rustls::pki_types::ServerName;
use rustls::{ClientConnection, StreamOwned};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::fetch_url::{build_tls_config, decode_hpack_headers_stateful, encode_hpack_literal};

const EZYTEL_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36";

const GOOGLE_DOMAINS: &[&str] = &[
    "safebrowsing.google.com",
    "images.google.com",
    "maps.google.com",
    "news.google.com",
    "scholar.google.com",
    "meet.google.com",
    "mail.google.com",
    "drive.google.com",
];

const TOTAL_TIMEOUT: Duration = Duration::from_secs(35);
const HEADER_TIMEOUT: Duration = Duration::from_secs(30);
const INLINE_WORKERS: usize = 8;

/// Tehran is UTC+03:30 year-round (no DST since 2022, matching `ezytel_service_imp.go:740`).
const TEHRAN_OFFSET_SECS: i64 = 3 * 3600 + 1800;

// ============================================================================
// Public Request / Response Models (`ezytel.proto` JSON & Protobuf compatible)
// ============================================================================

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelInfoRequest {
    #[serde(default, alias = "channelId")]
    pub channel_id: String,
    #[serde(default, alias = "lastRead")]
    pub last_read: i64,
    #[serde(default, alias = "disableInlineImages")]
    pub disable_inline_images: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelInfo {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub avatar_path: String,
    #[serde(default)]
    pub date: i64,
    #[serde(default)]
    pub date_str: String,
    #[serde(default)]
    pub newmsg: String,
    #[serde(default)]
    pub last_post_id: i64,
    #[serde(default)]
    pub ok: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelMessagesRequest {
    #[serde(default, alias = "channelId")]
    pub channel_id: String,
    #[serde(default)]
    pub before: i64,
    #[serde(default, alias = "disableInlineImages")]
    pub disable_inline_images: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelMessagesResponse {
    #[serde(default)]
    pub html: String,
    #[serde(default)]
    pub channel_avatar: String,
    #[serde(default)]
    pub last_post_id: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyImageRequest {
    #[serde(default, alias = "hexUrl")]
    pub hex_url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyImageResponse {
    #[serde(default)]
    pub data: Vec<u8>,
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub cache_name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseChannelsRequest {
    #[serde(default)]
    pub raw: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseChannelsResponse {
    #[serde(default)]
    pub channel_ids: Vec<String>,
}

// ============================================================================
// Service Implementation (`EzytelService`)
// ============================================================================

pub struct EzytelService {
    cache_dir: PathBuf,
    dir_mu: Mutex<()>,
    info_mu: Mutex<()>,
    /// Optional loopback base URL (`http://127.0.0.1:PORT`) used in deterministic unit tests.
    base_url_override: Option<String>,
}

impl EzytelService {
    pub fn new(cache_dir: &str) -> Self {
        let dir = if cache_dir.trim().is_empty() {
            std::env::temp_dir().join("ezytel-cache")
        } else {
            PathBuf::from(cache_dir)
        };
        Self {
            cache_dir: dir,
            dir_mu: Mutex::new(()),
            info_mu: Mutex::new(()),
            base_url_override: None,
        }
    }

    pub fn with_base_url(cache_dir: &str, base_url: &str) -> Self {
        let mut svc = Self::new(cache_dir);
        if !base_url.trim().is_empty() {
            svc.base_url_override = Some(base_url.trim_end_matches('/').to_string());
        }
        svc
    }

    pub fn get_cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    fn ensure_cache_dir(&self) -> Result<(), String> {
        let _guard = self.dir_mu.lock().map_err(|_| "lock poisoned".to_string())?;
        fs::create_dir_all(&self.cache_dir).map_err(|e| e.to_string())
    }

    /// `GetChannelMessages` — port of `html_parse()` in `libs.php` (`ezytel_service_imp.go:74`).
    pub fn get_channel_messages(
        &self,
        req: &ChannelMessagesRequest,
    ) -> Result<ChannelMessagesResponse, String> {
        let chid = req.channel_id.trim();
        if !is_valid_channel_id(chid) {
            return Err("invalid channel_id".to_string());
        }
        let path = if req.before != 0 {
            format!("{chid}?before={}", req.before)
        } else {
            chid.to_string()
        };
        let mut html = self.curl_auto(&path)?;
        let lower_check = html.to_ascii_lowercase();
        if !lower_check.contains("</header>") || !lower_check.contains("</main>") {
            return Err("upstream page missing header/main markers".to_string());
        }

        // Disable Telegram internal protocol-relative background images
        html = case_replace(&html, "background-image:url('//", "background-temp:url('//");
        html = rewrite_background_images(&html);
        html = rewrite_img_sources(&html);
        html = rewrite_times(&html, now_unix_secs());

        let chan_pic = str_find(&html, &["tgme_page_photo_image", "src=\""], "\"");
        let mut content = str_find(&html, &["</header>"], "</main>");
        if content.is_empty() {
            return Err("could not isolate <main> content".to_string());
        }
        content.push_str("</main>");

        if content.to_ascii_lowercase().contains("messages_more_wrap") {
            let link = str_find(&content, &["messages_more_wrap", ">"], "</div>");
            let data = str_find(&content, &["messages_more_wrap", "data-before=\""], "\"");
            if !link.is_empty() && !data.is_empty() {
                let replacement = format!(
                    r##"<a class="tme_messages_more" href="#" onclick="load_more({data},'{chid}',this)"></a>"##
                );
                content = case_replace(&content, &link, &replacement);
            }
        }

        if content.to_ascii_lowercase().contains("data-after=") {
            let raw = str_find(&content, &["data-after=", "*<a"], "</a>");
            if !raw.is_empty() {
                let needle = format!("<a {raw}</a>");
                content = case_replace(&content, &needle, "");
            }
        }

        let mut resp = ChannelMessagesResponse {
            html: String::new(),
            channel_avatar: chan_pic.clone(),
            last_post_id: 0,
        };

        if req.before == 0 {
            let subscribers = str_find(&html, &[r#"<div class="tgme_header_counter">"#], "</div>");
            let channel_name = str_find(&html, &[r#"<div class="tgme_header_title">"#], "</div>");
            let header = format!(
                r#"<header class="tgme_header search_collapsed"><div class="menu_channel_header"><div class="menu_channel_avatar"><img src="{chan_pic}"></div><div class="menu_channel_info"><p>{}</p><span>{subscribers}</span></div></div></header>"#,
                strip_tags(&channel_name)
            );
            content = format!("{header}{content}");

            let last_post_raw = str_find(&html, &[r#"*data-post=""#, "/"], "\"");
            if let Ok(id) = last_post_raw.trim().parse::<i64>() {
                resp.last_post_id = id;
            }
        }

        if !req.disable_inline_images {
            let chan_pic_uri = self.inline_proxy_placeholder(&chan_pic);
            if !chan_pic_uri.is_empty() {
                resp.channel_avatar = chan_pic_uri;
            }
            content = self.inline_proxy_placeholders(&content);
        }

        resp.html = content;
        Ok(resp)
    }

    /// `GetChannelInfo` — port of `json_info()` in `libs.php` (`ezytel_service_imp.go:263`).
    pub fn get_channel_info(&self, req: &ChannelInfoRequest) -> Result<ChannelInfo, String> {
        let chid = req.channel_id.trim();
        if !is_valid_channel_id(chid) {
            return Err("invalid channel_id".to_string());
        }

        let cache_file = self.cache_dir.join(format!("{chid}.json"));
        let fetched = self.curl_auto(chid);
        let ok_html = match &fetched {
            Ok(h) if h.contains(r#"<meta property="og:title" content=""#) => Some(h.as_str()),
            _ => None,
        };

        let Some(html) = ok_html else {
            if let Ok(data) = fs::read(&cache_file) {
                if let Ok(mut snap) = serde_json::from_slice::<ChannelInfo>(&data) {
                    snap.newmsg = "OFF".to_string();
                    snap.ok = false;
                    if !req.disable_inline_images && snap.avatar_path.starts_with("cache/") {
                        let rel = snap.avatar_path.trim_start_matches("cache/");
                        if let Ok(bytes) = fs::read(self.cache_dir.join(rel)) {
                            snap.avatar_path = data_url(&bytes, "image/jpeg");
                        } else {
                            snap.avatar_path.clear();
                        }
                    }
                    return Ok(snap);
                }
            }
            return match fetched {
                Err(err) => Err(err),
                Ok(_) => Err("channel page malformed".to_string()),
            };
        };

        let mut info = ChannelInfo {
            name: str_find(html, &[r#"<meta property="og:title" content=""#], "\""),
            ok: true,
            ..Default::default()
        };

        let pic = str_find(
            html,
            &[r#"<meta property="og:image" content="https://"#],
            "\"",
        );
        if !pic.is_empty() {
            if let Ok((data, cache_name)) = self.fetch_image_bytes(&format!("https://{pic}")) {
                if req.disable_inline_images {
                    info.avatar_path = format!("cache/{cache_name}");
                } else {
                    info.avatar_path = data_url(&data, "image/jpeg");
                }
            }
        }

        if html.contains(r#"data-post=""#) {
            let last_post = str_find(html, &[r#"*data-post=""#], "");
            let last_code = str_find(&last_post, &["/"], "\"")
                .trim()
                .parse::<i64>()
                .unwrap_or(0);
            info.last_post_id = last_code;

            if last_post
                .to_ascii_lowercase()
                .contains("tgme_widget_message_text")
            {
                info.description = strip_tags(&str_find(
                    &last_post,
                    &["tgme_widget_message_text", ">"],
                    "</div>",
                ));
            } else {
                info.description = "فایل".to_string();
            }

            if last_post.to_ascii_lowercase().contains("<time datetime=") {
                let t = str_find(&last_post, &[r#"<time datetime=""#], "\"");
                if let Some(ts) = parse_rfc3339_unix(&t) {
                    info.date = ts;
                    let converted = date_convert(ts);
                    if converted.len() >= 5 {
                        info.date_str = converted[5..].to_string();
                    }
                }
            }

            if req.last_read > 0 && last_code != req.last_read {
                let diff = (last_code - req.last_read).clamp(0, 99);
                info.newmsg = format!("+{diff}");
            }
        }

        if self.ensure_cache_dir().is_ok() {
            let avatar_on_disk = if !req.disable_inline_images && !pic.is_empty() {
                format!("cache/{}.jpg", md5sum(pic.as_bytes()))
            } else {
                info.avatar_path.clone()
            };
            let snap = ChannelInfo {
                avatar_path: avatar_on_disk,
                ..info.clone()
            };
            if let Ok(bytes) = serde_json::to_vec(&snap) {
                if let Ok(_guard) = self.info_mu.lock() {
                    let _ = fs::write(&cache_file, &bytes);
                }
            }
        }

        Ok(info)
    }

    /// `ProxyImage` — port of `proxy.php`'s image branch (`ezytel_service_imp.go:377`).
    pub fn proxy_image(&self, req: &ProxyImageRequest) -> Result<ProxyImageResponse, String> {
        if req.hex_url.is_empty() {
            return Err("hex_url is required".to_string());
        }
        let raw = hex_decode(&req.hex_url)
            .map_err(|e| format!("hex_url is not valid hex: {e}"))?;
        let raw_str = String::from_utf8(raw)
            .map_err(|e| format!("hex_url is not valid utf-8: {e}"))?;
        let (data, cache_name) = self.fetch_image_bytes(&format!("https://{raw_str}"))?;
        Ok(ProxyImageResponse {
            data,
            content_type: "image/jpeg".to_string(),
            cache_name,
        })
    }

    /// `ParseChannels` — port of `file_parse()` in `libs.php` (`ezytel_service_imp.go:426`).
    pub fn parse_channels(
        &self,
        req: &ParseChannelsRequest,
    ) -> Result<ParseChannelsResponse, String> {
        Ok(parse_channels_raw(&req.raw))
    }

    fn inline_proxy_placeholder(&self, reference: &str) -> String {
        const PREFIX: &str = "proxy.php?url=";
        let Some(idx) = reference.find(PREFIX) else {
            return String::new();
        };
        let hex_str = &reference[idx + PREFIX.len()..];
        let Ok(raw) = hex_decode(hex_str) else {
            return String::new();
        };
        let Ok(raw_str) = String::from_utf8(raw) else {
            return String::new();
        };
        match self.fetch_image_bytes(&format!("https://{raw_str}")) {
            Ok((data, _)) => data_url(&data, "image/jpeg"),
            Err(_) => String::new(),
        }
    }

    fn inline_proxy_placeholders(&self, content: &str) -> String {
        let hexes = extract_proxy_hexes(content);
        if hexes.is_empty() {
            return content.to_string();
        }
        let queue = Mutex::new(VecDeque::from_iter(hexes.iter().cloned()));
        let results: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
        let worker_count = INLINE_WORKERS.min(hexes.len()).max(1);

        thread::scope(|s| {
            for _ in 0..worker_count {
                s.spawn(|| loop {
                    let next_hex = {
                        let Ok(mut q) = queue.lock() else { break };
                        q.pop_front()
                    };
                    let Some(h) = next_hex else { break };
                    let Ok(raw) = hex_decode(&h) else { continue };
                    let Ok(raw_str) = String::from_utf8(raw) else {
                        continue;
                    };
                    if let Ok((data, _)) = self.fetch_image_bytes(&format!("https://{raw_str}")) {
                        if let Ok(mut out) = results.lock() {
                            out.push((h, data_url(&data, "image/jpeg")));
                        }
                    }
                });
            }
        });

        let pairs = results.into_inner().unwrap_or_default();
        if pairs.is_empty() {
            return content.to_string();
        }
        let mut out = content.to_string();
        for (h, uri) in pairs {
            let placeholder = format!("proxy.php?url={h}");
            // Case-insensitive match for hex case preservation if needed, plus exact lowercase match
            out = case_replace(&out, &placeholder, &uri);
        }
        out
    }

    fn fetch_image_bytes(&self, src: &str) -> Result<(Vec<u8>, String), String> {
        let stripped = src
            .strip_prefix("https://")
            .or_else(|| src.strip_prefix("http://"))
            .unwrap_or(src);
        let cache_name = format!("{}.jpg", md5sum(stripped.as_bytes()));
        self.curl_download(src, &cache_name)?;
        let data = fs::read(self.cache_dir.join(&cache_name)).map_err(|e| e.to_string())?;
        Ok((data, cache_name))
    }

    fn curl_auto(&self, params: &str) -> Result<String, String> {
        let mut last_err = match self.curl_get(params, 0) {
            Ok(body) => return Ok(body),
            Err(e) => e,
        };
        for attempt in 1..=2 {
            if !is_timeout_err(&last_err) {
                break;
            }
            match self.curl_get(params, attempt) {
                Ok(body) => return Ok(body),
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }

    fn curl_get(&self, params: &str, attempt: usize) -> Result<String, String> {
        let outer_host = domain_pick(attempt > 1);
        let sep = if params.contains('?') { "&" } else { "?" };
        let path_and_query =
            format!("/s/{params}{sep}_x_tr_sl=el&_x_tr_tl=en&_x_tr_hl=en&_x_tr_pto=wapp");

        let (scheme, connect_host, connect_port) = self.resolve_front_endpoint(&outer_host)?;
        let (_status, body) = fronted_http1_get(
            &scheme,
            &connect_host,
            connect_port,
            &path_and_query,
            "t-me.translate.goog",
            true,
        )?;
        Ok(String::from_utf8_lossy(&body).into_owned())
    }

    fn curl_download(&self, src: &str, name: &str) -> Result<(), String> {
        self.ensure_cache_dir()?;
        let target = self.cache_dir.join(name);
        if target.exists() {
            return Ok(());
        }
        let parsed = Url::parse(src).map_err(|_| format!("invalid url: {src}"))?;
        let Some(host) = parsed.host_str() else {
            return Err(format!("invalid url: {src}"));
        };
        let host_with_port = match parsed.port() {
            Some(p) => format!("{host}:{p}"),
            None => host.to_string(),
        };
        let front_host = format!("{}.translate.goog", host_with_port.replace('.', "-"));
        let mut path_and_query = parsed.path().to_string();
        if path_and_query.is_empty() {
            path_and_query.push('/');
        }
        if let Some(q) = parsed.query() {
            path_and_query.push('?');
            path_and_query.push_str(q);
        }

        let (scheme, connect_host, connect_port) = self.resolve_front_endpoint("www.google.com")?;
        let (status, body) = fronted_http1_get(
            &scheme,
            &connect_host,
            connect_port,
            &path_and_query,
            &front_host,
            false,
        )?;
        if status != 200 {
            return Err(format!("upstream status {status}"));
        }
        fs::write(&target, &body).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&target, fs::Permissions::from_mode(0o644));
        }
        Ok(())
    }

    fn resolve_front_endpoint(&self, default_https_host: &str) -> Result<(String, String, u16), String> {
        if let Some(base) = &self.base_url_override {
            let u = Url::parse(base).map_err(|e| format!("invalid base_url: {e}"))?;
            let scheme = u.scheme().to_string();
            let host = u
                .host_str()
                .ok_or_else(|| "missing host in base_url".to_string())?
                .to_string();
            let port = u.port_or_known_default().unwrap_or(80);
            Ok((scheme, host, port))
        } else {
            Ok(("https".to_string(), default_https_host.to_string(), 443))
        }
    }
}

/// Unified JSON dispatcher for C-ABI v7 (`hiddify_net_ezytel_invoke_json`).
pub fn invoke_ezytel_json(request_json: &str) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_str(request_json).map_err(|e| format!("invalid json: {e}"))?;
    let op = v.get("op").and_then(|x| x.as_str()).unwrap_or("");
    let cache_dir = v.get("cacheDir").and_then(|x| x.as_str()).unwrap_or("");
    let base_url = v.get("baseUrl").and_then(|x| x.as_str()).unwrap_or("");
    let svc = EzytelService::with_base_url(cache_dir, base_url);

    match op {
        "info" => {
            let req: ChannelInfoRequest =
                serde_json::from_value(v).map_err(|e| format!("invalid info request: {e}"))?;
            let resp = svc.get_channel_info(&req)?;
            serde_json::to_string(&resp).map_err(|e| e.to_string())
        }
        "messages" => {
            let req: ChannelMessagesRequest =
                serde_json::from_value(v).map_err(|e| format!("invalid messages request: {e}"))?;
            let resp = svc.get_channel_messages(&req)?;
            serde_json::to_string(&resp).map_err(|e| e.to_string())
        }
        "proxy_image" => {
            let req: ProxyImageRequest = serde_json::from_value(v)
                .map_err(|e| format!("invalid proxy_image request: {e}"))?;
            let resp = svc.proxy_image(&req)?;
            serde_json::to_string(&serde_json::json!({
                "content_type": resp.content_type,
                "cache_name": resp.cache_name,
                "data": BASE64_STANDARD.encode(&resp.data),
            }))
            .map_err(|e| e.to_string())
        }
        "parse_channels" => {
            let req: ParseChannelsRequest = serde_json::from_value(v)
                .map_err(|e| format!("invalid parse_channels request: {e}"))?;
            let resp = svc.parse_channels(&req)?;
            serde_json::to_string(&resp).map_err(|e| e.to_string())
        }
        other => Err(format!("unknown ezytel op: {other}")),
    }
}

// ============================================================================
// Pure Functions: Channel List Parser, HTML Rewriters, MD5, Jalali Calendar
// ============================================================================

pub fn is_valid_channel_id(chid: &str) -> bool {
    !chid.is_empty()
        && chid.len() <= 64
        && chid
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

pub fn parse_channels_raw(input: &str) -> ParseChannelsResponse {
    let mut raw = input.to_string();
    for ch in ["'", "\"", "@"] {
        raw = raw.replace(ch, "");
    }
    for p in ["https://", "http://", "HTTPS://", "HTTP://"] {
        raw = raw.replace(p, "");
    }
    raw = raw.replace("/s/", "");
    raw = raw.replace("t.me", "");
    raw = raw.replace('/', "");

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for line in raw.split('\n') {
        let name = line.trim().replace(' ', "").to_lowercase();
        if name.is_empty() {
            continue;
        }
        if seen.insert(name.clone()) {
            out.push(name);
        }
    }
    ParseChannelsResponse { channel_ids: out }
}

pub fn data_url(data: &[u8], content_type: &str) -> String {
    let ct = if content_type.is_empty() {
        "image/jpeg"
    } else {
        content_type
    };
    format!("data:{ct};base64,{}", BASE64_STANDARD.encode(data))
}

fn domain_pick(random: bool) -> String {
    if !random {
        return "www.google.com".to_string();
    }
    let idx = rand::thread_rng().gen_range(0..GOOGLE_DOMAINS.len());
    GOOGLE_DOMAINS[idx].to_string()
}

fn is_timeout_err(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("timeout") || lower.contains("timed out") || lower.contains("deadline")
}

/// ASCII case-insensitive substring search from byte offset `from` (preserves exact UTF-8 byte indices).
fn ascii_find_from(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from > haystack.len() || needle.len() > haystack.len() - from {
        return None;
    }
    if needle.is_empty() {
        return Some(from);
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle))
        .map(|pos| from + pos)
}

fn ascii_rfind_in(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    if needle.is_empty() {
        return Some(haystack.len());
    }
    haystack
        .windows(needle.len())
        .rposition(|w| w.eq_ignore_ascii_case(needle))
}

/// Reproduces PHP `str_find()` (`ezytel_service_imp.go:594`).
pub fn str_find(s: &str, find: &[&str], end: &str) -> String {
    let bytes = s.as_bytes();
    let mut start = 0usize;
    for &needle in find {
        if let Some(rev) = needle.strip_prefix('*') {
            let rev_bytes = rev.as_bytes();
            let search_slice = if start == 0 { bytes } else { &bytes[..start] };
            let Some(pos) = ascii_rfind_in(search_slice, rev_bytes) else {
                return String::new();
            };
            start = pos + rev_bytes.len();
        } else {
            let n_bytes = needle.as_bytes();
            let Some(pos) = ascii_find_from(bytes, n_bytes, start) else {
                return String::new();
            };
            start = pos + n_bytes.len();
        }
    }
    if end.is_empty() {
        return s[start..].trim().to_string();
    }
    let Some(fin) = ascii_find_from(bytes, end.as_bytes(), start) else {
        return String::new();
    };
    s[start..fin].trim().to_string()
}

/// Case-insensitive `str_ireplace` (`ezytel_service_imp.go:642`).
pub fn case_replace(s: &str, old: &str, new_val: &str) -> String {
    if old.is_empty() {
        return s.to_string();
    }
    let hb = s.as_bytes();
    let nb = old.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut pos = 0usize;
    while let Some(idx) = ascii_find_from(hb, nb, pos) {
        out.push_str(&s[pos..idx]);
        out.push_str(new_val);
        pos = idx + nb.len();
    }
    out.push_str(&s[pos..]);
    out
}

pub fn strip_tags(s: &str) -> String {
    static TAG_RE: OnceLock<Regex> = OnceLock::new();
    let re = TAG_RE.get_or_init(|| Regex::new(r"<[^>]*>").expect("tag regex"));
    re.replace_all(s, "").into_owned()
}

pub fn extract_proxy_hexes(s: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    const PREFIX: &str = "proxy.php?url=";
    let mut i = 0usize;
    while let Some(rel) = s[i..].find(PREFIX) {
        let start = i + rel + PREFIX.len();
        let mut end = start;
        let bytes = s.as_bytes();
        while end < bytes.len() && bytes[end].is_ascii_hexdigit() {
            end += 1;
        }
        if end > start {
            out.insert(s[start..end].to_ascii_lowercase());
        }
        i = end;
    }
    out
}

pub fn rewrite_background_images(html: &str) -> String {
    let mut owned = html.to_string();
    const PREFIX: &str = "background-image:url('https://";
    loop {
        let Some(idx) = ascii_find_from(owned.as_bytes(), PREFIX.as_bytes(), 0) else {
            return owned;
        };
        let start = idx + PREFIX.len();
        let Some(rel_end) = owned[start..].find(')') else {
            return owned;
        };
        let raw = owned[start..start + rel_end].trim_matches(|c| c == '\'' || c == '"');
        let full = format!("https://{raw}");
        let hexed = hex_encode(raw.as_bytes());
        owned = owned.replace(&full, &format!("proxy.php?url={hexed}"));
    }
}

pub fn rewrite_img_sources(html: &str) -> String {
    let mut owned = html.to_string();
    const PREFIX: &str = r#"<img src="https://"#;
    loop {
        let Some(idx) = owned.find(PREFIX) else {
            return owned;
        };
        let start = idx + PREFIX.len();
        let Some(rel_end) = owned[start..].find('"') else {
            return owned;
        };
        let raw = &owned[start..start + rel_end];
        let full = format!("https://{raw}");
        let hexed = hex_encode(raw.as_bytes());
        owned = owned.replace(&full, &format!("proxy.php?url={hexed}"));
    }
}

pub fn rewrite_times(html: &str, now_secs: i64) -> String {
    let mut owned = html.to_string();
    loop {
        if !owned.contains(r#"<time datetime=""#) {
            return owned;
        }
        let t = str_find(&owned, &[r#"<time datetime=""#], "\"");
        if t.is_empty() {
            return owned;
        }
        let raw = str_find(&owned, &[r#"datetime=""#, "*<time "], "</time>");
        if raw.is_empty() {
            return owned;
        }
        let human = match parse_rfc3339_unix(&t) {
            Some(ts) => date_easy_read(ts, now_secs),
            None => t,
        };
        owned = owned.replacen(&raw, &format!(r#"class="time">{human}"#), 1);
    }
}

fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs() as i64
}

/// Tehran-local Jalali (Persian) date + `HH:mm` (`ezytel_service_imp.go:760`).
pub fn date_convert(unix_secs: i64) -> String {
    let local_secs = unix_secs + TEHRAN_OFFSET_SECS;
    let days_since_epoch = local_secs.div_euclid(86_400);
    let sec_of_day = local_secs.rem_euclid(86_400);
    let hh = sec_of_day / 3600;
    let mm = (sec_of_day % 3600) / 60;

    let (mut gy, gm, gd) = civil_from_days(days_since_epoch);
    let g_dm = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let mut jy: i64;
    if gy > 1600 {
        jy = 979;
        gy -= 1600;
    } else {
        jy = 0;
        gy -= 621;
    }
    let gy2 = if gm > 2 { gy + 1 } else { gy };
    let mut days =
        (365 * gy) + (gy2 + 3) / 4 - (gy2 + 99) / 100 + (gy2 + 399) / 400 - 80 + gd + g_dm[(gm - 1) as usize];
    jy += 33 * (days / 12053);
    days %= 12053;
    jy += 4 * (days / 1461);
    days %= 1461;
    if days > 365 {
        jy += (days - 1) / 365;
        days = (days - 1) % 365;
    }
    let (jm, jd) = if days < 186 {
        (1 + days / 31, 1 + days % 31)
    } else {
        (7 + (days - 186) / 30, 1 + (days - 186) % 30)
    };
    format!("{jy:04}-{jm:02}-{jd:02} {hh:02}:{mm:02}")
}

pub fn date_easy_read(unix_secs: i64, now_secs: i64) -> String {
    let full = date_convert(unix_secs);
    let today = date_convert(now_secs);
    let yesterday = date_convert(now_secs - 86_400);
    if today.len() >= 10 && full.contains(&today[..10]) {
        return full.replacen(&today[..10], "Today", 1);
    }
    if yesterday.len() >= 10 && full.contains(&yesterday[..10]) {
        return full.replacen(&yesterday[..10], "Yesterday", 1);
    }
    full
}

/// Howard Hinnant's Gregorian `(year, month, day)` from days since `1970-01-01`.
fn civil_from_days(z_in: i64) -> (i64, i64, i64) {
    let z = z_in + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
}

fn days_from_civil(y_in: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y_in - 1 } else { y_in };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Parses RFC 3339 timestamps (`2026-11-28T14:30:00+00:00`, `2026-11-28T14:30:00Z`, fractional secs).
pub fn parse_rfc3339_unix(input: &str) -> Option<i64> {
    let s = input.trim();
    if s.len() < 19 {
        return None;
    }
    let year: i64 = s.get(0..4)?.parse().ok()?;
    if s.as_bytes().get(4) != Some(&b'-') {
        return None;
    }
    let month: i64 = s.get(5..7)?.parse().ok()?;
    if !(1..=12).contains(&month) || s.as_bytes().get(7) != Some(&b'-') {
        return None;
    }
    let day: i64 = s.get(8..10)?.parse().ok()?;
    if !(1..=31).contains(&day) || !matches!(s.as_bytes().get(10), Some(b'T' | b't' | b' ')) {
        return None;
    }
    let hour: i64 = s.get(11..13)?.parse().ok()?;
    if !(0..=23).contains(&hour) || s.as_bytes().get(13) != Some(&b':') {
        return None;
    }
    let min: i64 = s.get(14..16)?.parse().ok()?;
    if !(0..=59).contains(&min) || s.as_bytes().get(16) != Some(&b':') {
        return None;
    }
    let sec: i64 = s.get(17..19)?.parse().ok()?;
    if !(0..=60).contains(&sec) {
        return None;
    }
    let mut rest = &s[19..];
    if let Some(after_dot) = rest.strip_prefix('.') {
        let digits = after_dot.bytes().take_while(|b| b.is_ascii_digit()).count();
        if digits == 0 {
            return None;
        }
        rest = &after_dot[digits..];
    }
    let offset_secs: i64 = if rest.eq_ignore_ascii_case("Z") {
        0
    } else if rest.len() == 6 && matches!(rest.as_bytes()[0], b'+' | b'-') && rest.as_bytes()[3] == b':' {
        let sign = if rest.as_bytes()[0] == b'-' { -1 } else { 1 };
        let oh: i64 = rest[1..3].parse().ok()?;
        let om: i64 = rest[4..6].parse().ok()?;
        sign * (oh * 3600 + om * 60)
    } else {
        return None;
    };

    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3600 + min * 60 + sec - offset_secs)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

fn hex_decode(hex_str: &str) -> Result<Vec<u8>, &'static str> {
    let b = hex_str.as_bytes();
    if b.len() % 2 != 0 {
        return Err("odd hex length");
    }
    let val = |c: u8| -> Result<u8, &'static str> {
        match c {
            b'0'..=b'9' => Ok(c - b'0'),
            b'a'..=b'f' => Ok(c - b'a' + 10),
            b'A'..=b'F' => Ok(c - b'A' + 10),
            _ => Err("invalid hex digit"),
        }
    };
    let mut out = Vec::with_capacity(b.len() / 2);
    for pair in b.chunks_exact(2) {
        out.push((val(pair[0])? << 4) | val(pair[1])?);
    }
    Ok(out)
}

/// Pure-Rust RFC 1321 MD5 lowercase hex digest (`md5sum` in `ezytel_service_imp.go`).
pub fn md5sum(data: &[u8]) -> String {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];

    let mut a0: u32 = 0x67452301;
    let mut b0: u32 = 0xefcdab89;
    let mut c0: u32 = 0x98badcfe;
    let mut d0: u32 = 0x10325476;

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = Vec::with_capacity((data.len() + 72) & !63);
    msg.extend_from_slice(data);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut m = [0u32; 16];
        for (i, w) in chunk.chunks_exact(4).enumerate() {
            m[i] = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | ((!b) & d), i),
                16..=31 => ((d & b) | ((!d) & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | (!d)), (7 * i) % 16),
            };
            let temp = d;
            d = c;
            c = b;
            b = b.wrapping_add(
                a.wrapping_add(f)
                    .wrapping_add(K[i])
                    .wrapping_add(m[g])
                    .rotate_left(S[i]),
            );
            a = temp;
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }

    let mut digest = [0u8; 16];
    digest[0..4].copy_from_slice(&a0.to_le_bytes());
    digest[4..8].copy_from_slice(&b0.to_le_bytes());
    digest[8..12].copy_from_slice(&c0.to_le_bytes());
    digest[12..16].copy_from_slice(&d0.to_le_bytes());
    hex_encode(&digest)
}

// ============================================================================
// Domain-Fronted HTTP/1.1 + TLS 1.2+ Client (`curlGet` / `curlDownload`)
// ============================================================================

fn fronted_http1_get(
    scheme: &str,
    connect_host: &str,
    connect_port: u16,
    path_and_query: &str,
    fronted_host_header: &str,
    no_cache_headers: bool,
) -> Result<(u16, Vec<u8>), String> {
    let addr = resolve_socket_addr(connect_host, connect_port)?;
    let tcp = TcpStream::connect_timeout(&addr, HEADER_TIMEOUT)
        .map_err(|e| format!("dial {connect_host}:{connect_port}: {e}"))?;
    let _ = tcp.set_read_timeout(Some(TOTAL_TIMEOUT));
    let _ = tcp.set_write_timeout(Some(HEADER_TIMEOUT));

    let mut req = format!(
        "GET {path_and_query} HTTP/1.1\r\nHost: {fronted_host_header}\r\nUser-Agent: {EZYTEL_USER_AGENT}\r\nAccept-Encoding: gzip\r\n"
    );
    if no_cache_headers {
        req.push_str("Pragma: no-cache\r\nCache-Control: no-cache\r\n");
    }
    req.push_str("Connection: close\r\n\r\n");

    if scheme.eq_ignore_ascii_case("https") {
        let mut tls_cfg = (*build_tls_config(None)?).clone();
        tls_cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
        let sni = ServerName::try_from(connect_host.to_string())
            .map_err(|e| format!("invalid SNI {connect_host}: {e}"))?;
        let conn = ClientConnection::new(Arc::new(tls_cfg), sni)
            .map_err(|e| format!("tls init: {e}"))?;
        let mut tls_stream = StreamOwned::new(conn, tcp);
        tls_stream
            .write_all(req.as_bytes())
            .map_err(|e| format!("tls write: {e}"))?;
        tls_stream.flush().map_err(|e| format!("tls flush: {e}"))?;
        read_http1_response(tls_stream)
    } else {
        let mut plain = tcp;
        plain
            .write_all(req.as_bytes())
            .map_err(|e| format!("http write: {e}"))?;
        plain.flush().map_err(|e| format!("http flush: {e}"))?;
        read_http1_response(plain)
    }
}

fn resolve_socket_addr(host: &str, port: u16) -> Result<SocketAddr, String> {
    (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("dns {host}: {e}"))?
        .next()
        .ok_or_else(|| format!("dns {host}: no addresses"))
}

fn read_http1_response<R: Read>(stream: R) -> Result<(u16, Vec<u8>), String> {
    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    if reader
        .read_line(&mut status_line)
        .map_err(|e| format!("http read status: {e}"))?
        == 0
    {
        return Err("unexpected EOF reading HTTP status".to_string());
    }
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("malformed HTTP status: {}", status_line.trim()))?;

    let mut content_len: Option<usize> = None;
    let mut chunked = false;
    let mut gzip = false;
    loop {
        let mut line = String::new();
        if reader
            .read_line(&mut line)
            .map_err(|e| format!("http read header: {e}"))?
            == 0
            || line == "\r\n"
            || line == "\n"
        {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim();
            let val = v.trim();
            if key.eq_ignore_ascii_case("content-length") {
                content_len = val.parse().ok();
            } else if key.eq_ignore_ascii_case("transfer-encoding")
                && val.to_ascii_lowercase().contains("chunked")
            {
                chunked = true;
            } else if key.eq_ignore_ascii_case("content-encoding")
                && val.eq_ignore_ascii_case("gzip")
            {
                gzip = true;
            }
        }
    }

    let raw_body = if chunked {
        read_chunked_body(&mut reader)?
    } else if let Some(len) = content_len {
        let mut buf = vec![0u8; len];
        reader
            .read_exact(&mut buf)
            .map_err(|e| format!("http read body: {e}"))?;
        buf
    } else {
        let mut buf = Vec::new();
        reader
            .read_to_end(&mut buf)
            .map_err(|e| format!("http read body: {e}"))?;
        buf
    };

    let body = if gzip && !raw_body.is_empty() {
        let mut decoded = Vec::new();
        GzDecoder::new(&raw_body[..])
            .read_to_end(&mut decoded)
            .map_err(|e| format!("gzip decode: {e}"))?;
        decoded
    } else {
        raw_body
    };
    Ok((status, body))
}

fn read_chunked_body<R: BufRead>(reader: &mut R) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let mut size_line = String::new();
        if reader
            .read_line(&mut size_line)
            .map_err(|e| format!("chunk header: {e}"))?
            == 0
        {
            break;
        }
        let hex_part = size_line.trim().split(';').next().unwrap_or("");
        let size = usize::from_str_radix(hex_part, 16)
            .map_err(|_| format!("invalid chunk size: {hex_part}"))?;
        if size == 0 {
            let mut trailer = String::new();
            let _ = reader.read_line(&mut trailer);
            break;
        }
        let start = out.len();
        out.resize(start + size, 0);
        reader
            .read_exact(&mut out[start..])
            .map_err(|e| format!("chunk data: {e}"))?;
        let mut crlf = [0u8; 2];
        reader
            .read_exact(&mut crlf)
            .map_err(|e| format!("chunk crlf: {e}"))?;
    }
    Ok(out)
}

// ============================================================================
// Wire-Exact Protobuf Codec (`v2/ezytel/ezytel.proto`)
// ============================================================================

pub mod pb {
    use super::*;

    fn put_varint(out: &mut Vec<u8>, mut val: u64) {
        while val >= 0x80 {
            out.push(((val as u8) & 0x7f) | 0x80);
            val >>= 7;
        }
        out.push(val as u8);
    }

    fn put_i64(out: &mut Vec<u8>, field: u32, val: i64) {
        if val != 0 {
            put_varint(out, ((field as u64) << 3) | 0);
            put_varint(out, val as u64);
        }
    }

    fn put_bool(out: &mut Vec<u8>, field: u32, val: bool) {
        if val {
            put_varint(out, ((field as u64) << 3) | 0);
            out.push(1);
        }
    }

    fn put_bytes(out: &mut Vec<u8>, field: u32, bytes: &[u8]) {
        if !bytes.is_empty() {
            put_varint(out, ((field as u64) << 3) | 2);
            put_varint(out, bytes.len() as u64);
            out.extend_from_slice(bytes);
        }
    }

    fn put_str(out: &mut Vec<u8>, field: u32, s: &str) {
        put_bytes(out, field, s.as_bytes());
    }

    fn read_varint(buf: &[u8], pos: &mut usize) -> Result<u64, String> {
        let mut val = 0u64;
        let mut shift = 0u32;
        loop {
            if *pos >= buf.len() || shift >= 64 {
                return Err("truncated protobuf varint".to_string());
            }
            let b = buf[*pos];
            *pos += 1;
            val |= ((b & 0x7f) as u64) << shift;
            if (b & 0x80) == 0 {
                return Ok(val);
            }
            shift += 7;
        }
    }

    fn read_len_bytes<'a>(buf: &'a [u8], pos: &mut usize) -> Result<&'a [u8], String> {
        let len = read_varint(buf, pos)? as usize;
        if *pos + len > buf.len() {
            return Err("truncated protobuf length-delimited field".to_string());
        }
        let slice = &buf[*pos..*pos + len];
        *pos += len;
        Ok(slice)
    }

    fn skip_field(buf: &[u8], pos: &mut usize, wire_type: u8) -> Result<(), String> {
        match wire_type {
            0 => {
                let _ = read_varint(buf, pos)?;
                Ok(())
            }
            1 => {
                if *pos + 8 > buf.len() {
                    return Err("truncated fixed64".to_string());
                }
                *pos += 8;
                Ok(())
            }
            2 => {
                let _ = read_len_bytes(buf, pos)?;
                Ok(())
            }
            5 => {
                if *pos + 4 > buf.len() {
                    return Err("truncated fixed32".to_string());
                }
                *pos += 4;
                Ok(())
            }
            other => Err(format!("unsupported protobuf wire type {other}")),
        }
    }

    pub fn decode_channel_info_req(buf: &[u8]) -> Result<ChannelInfoRequest, String> {
        let mut out = ChannelInfoRequest::default();
        let mut pos = 0usize;
        while pos < buf.len() {
            let tag = read_varint(buf, &mut pos)?;
            match ((tag >> 3) as u32, (tag & 7) as u8) {
                (1, 2) => {
                    out.channel_id =
                        String::from_utf8_lossy(read_len_bytes(buf, &mut pos)?).into_owned()
                }
                (2, 0) => out.last_read = read_varint(buf, &mut pos)? as i64,
                (3, 0) => out.disable_inline_images = read_varint(buf, &mut pos)? != 0,
                (_, wt) => skip_field(buf, &mut pos, wt)?,
            }
        }
        Ok(out)
    }

    pub fn encode_channel_info(msg: &ChannelInfo) -> Vec<u8> {
        let mut out = Vec::new();
        put_str(&mut out, 1, &msg.name);
        put_str(&mut out, 2, &msg.description);
        put_str(&mut out, 3, &msg.avatar_path);
        put_i64(&mut out, 4, msg.date);
        put_str(&mut out, 5, &msg.date_str);
        put_str(&mut out, 6, &msg.newmsg);
        put_i64(&mut out, 7, msg.last_post_id);
        put_bool(&mut out, 8, msg.ok);
        out
    }

    pub fn encode_channel_info_req(req: &ChannelInfoRequest) -> Vec<u8> {
        let mut out = Vec::new();
        put_str(&mut out, 1, &req.channel_id);
        put_i64(&mut out, 2, req.last_read);
        put_bool(&mut out, 3, req.disable_inline_images);
        out
    }

    pub fn decode_channel_info(buf: &[u8]) -> Result<ChannelInfo, String> {
        let mut out = ChannelInfo::default();
        let mut pos = 0usize;
        while pos < buf.len() {
            let tag = read_varint(buf, &mut pos)?;
            match ((tag >> 3) as u32, (tag & 7) as u8) {
                (1, 2) => out.name = String::from_utf8_lossy(read_len_bytes(buf, &mut pos)?).into_owned(),
                (2, 2) => out.description = String::from_utf8_lossy(read_len_bytes(buf, &mut pos)?).into_owned(),
                (3, 2) => out.avatar_path = String::from_utf8_lossy(read_len_bytes(buf, &mut pos)?).into_owned(),
                (4, 0) => out.date = read_varint(buf, &mut pos)? as i64,
                (5, 2) => out.date_str = String::from_utf8_lossy(read_len_bytes(buf, &mut pos)?).into_owned(),
                (6, 2) => out.newmsg = String::from_utf8_lossy(read_len_bytes(buf, &mut pos)?).into_owned(),
                (7, 0) => out.last_post_id = read_varint(buf, &mut pos)? as i64,
                (8, 0) => out.ok = read_varint(buf, &mut pos)? != 0,
                (_, wt) => skip_field(buf, &mut pos, wt)?,
            }
        }
        Ok(out)
    }

    pub fn decode_channel_messages_req(buf: &[u8]) -> Result<ChannelMessagesRequest, String> {
        let mut out = ChannelMessagesRequest::default();
        let mut pos = 0usize;
        while pos < buf.len() {
            let tag = read_varint(buf, &mut pos)?;
            match ((tag >> 3) as u32, (tag & 7) as u8) {
                (1, 2) => {
                    out.channel_id =
                        String::from_utf8_lossy(read_len_bytes(buf, &mut pos)?).into_owned()
                }
                (2, 0) => out.before = read_varint(buf, &mut pos)? as i64,
                (3, 0) => out.disable_inline_images = read_varint(buf, &mut pos)? != 0,
                (_, wt) => skip_field(buf, &mut pos, wt)?,
            }
        }
        Ok(out)
    }

    pub fn encode_channel_messages_resp(msg: &ChannelMessagesResponse) -> Vec<u8> {
        let mut out = Vec::new();
        put_str(&mut out, 1, &msg.html);
        put_str(&mut out, 2, &msg.channel_avatar);
        put_i64(&mut out, 3, msg.last_post_id);
        out
    }

    pub fn decode_proxy_image_req(buf: &[u8]) -> Result<ProxyImageRequest, String> {
        let mut out = ProxyImageRequest::default();
        let mut pos = 0usize;
        while pos < buf.len() {
            let tag = read_varint(buf, &mut pos)?;
            match ((tag >> 3) as u32, (tag & 7) as u8) {
                (1, 2) => {
                    out.hex_url =
                        String::from_utf8_lossy(read_len_bytes(buf, &mut pos)?).into_owned()
                }
                (_, wt) => skip_field(buf, &mut pos, wt)?,
            }
        }
        Ok(out)
    }

    pub fn encode_proxy_image_resp(msg: &ProxyImageResponse) -> Vec<u8> {
        let mut out = Vec::new();
        put_bytes(&mut out, 1, &msg.data);
        put_str(&mut out, 2, &msg.content_type);
        put_str(&mut out, 3, &msg.cache_name);
        out
    }

    pub fn decode_parse_channels_req(buf: &[u8]) -> Result<ParseChannelsRequest, String> {
        let mut out = ParseChannelsRequest::default();
        let mut pos = 0usize;
        while pos < buf.len() {
            let tag = read_varint(buf, &mut pos)?;
            match ((tag >> 3) as u32, (tag & 7) as u8) {
                (1, 2) => {
                    out.raw = String::from_utf8_lossy(read_len_bytes(buf, &mut pos)?).into_owned()
                }
                (_, wt) => skip_field(buf, &mut pos, wt)?,
            }
        }
        Ok(out)
    }

    pub fn encode_parse_channels_req(req: &ParseChannelsRequest) -> Vec<u8> {
        let mut out = Vec::new();
        put_str(&mut out, 1, &req.raw);
        out
    }

    pub fn encode_parse_channels_resp(msg: &ParseChannelsResponse) -> Vec<u8> {
        let mut out = Vec::new();
        for id in &msg.channel_ids {
            put_str(&mut out, 1, id);
        }
        out
    }

    pub fn decode_parse_channels_resp(buf: &[u8]) -> Result<ParseChannelsResponse, String> {
        let mut out = ParseChannelsResponse::default();
        let mut pos = 0usize;
        while pos < buf.len() {
            let tag = read_varint(buf, &mut pos)?;
            match ((tag >> 3) as u32, (tag & 7) as u8) {
                (1, 2) => out.channel_ids.push(
                    String::from_utf8_lossy(read_len_bytes(buf, &mut pos)?).into_owned(),
                ),
                (_, wt) => skip_field(buf, &mut pos, wt)?,
            }
        }
        Ok(out)
    }
}

// ============================================================================
// Native HTTP/2 Cleartext (`h2c`) gRPC Server (`HiddifyCli ezytel serve`)
// ============================================================================

const H2_CLIENT_PREFACE: &[u8; 24] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
const H2_DATA: u8 = 0x0;
const H2_HEADERS: u8 = 0x1;
const H2_RST_STREAM: u8 = 0x3;
const H2_SETTINGS: u8 = 0x4;
const H2_PING: u8 = 0x6;
const H2_GOAWAY: u8 = 0x7;
const H2_WINDOW_UPDATE: u8 = 0x8;
const H2_CONTINUATION: u8 = 0x9;

const FLAG_END_STREAM: u8 = 0x1;
const FLAG_ACK: u8 = 0x1;
const FLAG_END_HEADERS: u8 = 0x4;
const FLAG_PADDED: u8 = 0x8;
const FLAG_PRIORITY: u8 = 0x20;

const GRPC_PERCENT_SET: &AsciiSet = &CONTROLS.add(b'%');

#[derive(Default)]
struct H2StreamReq {
    header_block: Vec<u8>,
    headers_done: bool,
    path: String,
    data: Vec<u8>,
}

/// Runs the Ezytel `h2c` gRPC server on `listener` until `stop` becomes `true`.
pub fn serve_ezytel_grpc_on_listener(
    listener: TcpListener,
    service: Arc<EzytelService>,
    stop: Arc<AtomicBool>,
) -> io::Result<()> {
    listener.set_nonblocking(true)?;
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _peer)) => {
                let _ = stream.set_nonblocking(false);
                let svc = service.clone();
                let st = stop.clone();
                thread::spawn(move || {
                    let _ = handle_h2c_grpc_conn(stream, svc, st);
                });
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn write_h2_frame<W: Write>(
    w: &mut W,
    ftype: u8,
    flags: u8,
    stream_id: u32,
    payload: &[u8],
) -> io::Result<()> {
    let len = payload.len();
    let sid = (stream_id & 0x7fff_ffff).to_be_bytes();
    let hdr = [
        ((len >> 16) & 0xff) as u8,
        ((len >> 8) & 0xff) as u8,
        (len & 0xff) as u8,
        ftype,
        flags,
        sid[0],
        sid[1],
        sid[2],
        sid[3],
    ];
    w.write_all(&hdr)?;
    if !payload.is_empty() {
        w.write_all(payload)?;
    }
    Ok(())
}

fn handle_h2c_grpc_conn(
    mut stream: TcpStream,
    service: Arc<EzytelService>,
    stop: Arc<AtomicBool>,
) -> io::Result<()> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(60)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));

    let mut preface = [0u8; 24];
    stream.read_exact(&mut preface)?;
    if &preface != H2_CLIENT_PREFACE {
        return Ok(());
    }

    // Send initial SETTINGS + large connection WINDOW_UPDATE (16 MiB)
    write_h2_frame(&mut stream, H2_SETTINGS, 0, 0, &[])?;
    write_h2_frame(
        &mut stream,
        H2_WINDOW_UPDATE,
        0,
        0,
        &(16u32 << 20).to_be_bytes(),
    )?;
    stream.flush()?;

    let mut dyn_table: Vec<(String, String)> = Vec::new();
    let mut streams: HashMap<u32, H2StreamReq> = HashMap::new();

    while !stop.load(Ordering::Acquire) {
        let mut fh = [0u8; 9];
        if let Err(e) = stream.read_exact(&mut fh) {
            if matches!(
                e.kind(),
                io::ErrorKind::UnexpectedEof | io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) {
                break;
            }
            return Err(e);
        }
        let len = ((fh[0] as usize) << 16) | ((fh[1] as usize) << 8) | (fh[2] as usize);
        let ftype = fh[3];
        let flags = fh[4];
        let sid = u32::from_be_bytes([fh[5] & 0x7f, fh[6], fh[7], fh[8]]);

        let mut payload = vec![0u8; len];
        if len > 0 {
            stream.read_exact(&mut payload)?;
        }

        match ftype {
            H2_SETTINGS => {
                if (flags & FLAG_ACK) == 0 {
                    write_h2_frame(&mut stream, H2_SETTINGS, FLAG_ACK, 0, &[])?;
                    stream.flush()?;
                }
            }
            H2_PING => {
                if (flags & FLAG_ACK) == 0 && payload.len() == 8 {
                    write_h2_frame(&mut stream, H2_PING, FLAG_ACK, 0, &payload)?;
                    stream.flush()?;
                }
            }
            H2_GOAWAY => break,
            H2_RST_STREAM => {
                streams.remove(&sid);
            }
            H2_HEADERS if sid > 0 => {
                let mut slice = &payload[..];
                if (flags & FLAG_PADDED) != 0 && !slice.is_empty() {
                    let pad = slice[0] as usize;
                    slice = &slice[1..slice.len().saturating_sub(pad)];
                }
                if (flags & FLAG_PRIORITY) != 0 && slice.len() >= 5 {
                    slice = &slice[5..];
                }
                let entry = streams.entry(sid).or_default();
                entry.header_block.extend_from_slice(slice);
                if (flags & FLAG_END_HEADERS) != 0 {
                    finalize_stream_headers(entry, &mut dyn_table);
                }
                if (flags & FLAG_END_STREAM) != 0 && entry.headers_done {
                    if let Some(done) = streams.remove(&sid) {
                        dispatch_grpc_stream(&mut stream, sid, done, &service)?;
                    }
                }
            }
            H2_CONTINUATION if sid > 0 => {
                if let Some(entry) = streams.get_mut(&sid) {
                    entry.header_block.extend_from_slice(&payload);
                    if (flags & FLAG_END_HEADERS) != 0 {
                        finalize_stream_headers(entry, &mut dyn_table);
                    }
                }
            }
            H2_DATA if sid > 0 => {
                let mut slice = &payload[..];
                if (flags & FLAG_PADDED) != 0 && !slice.is_empty() {
                    let pad = slice[0] as usize;
                    slice = &slice[1..slice.len().saturating_sub(pad)];
                }
                if len > 0 {
                    let inc = (len as u32) & 0x7fff_ffff;
                    write_h2_frame(&mut stream, H2_WINDOW_UPDATE, 0, 0, &inc.to_be_bytes())?;
                    write_h2_frame(&mut stream, H2_WINDOW_UPDATE, 0, sid, &inc.to_be_bytes())?;
                }
                if let Some(entry) = streams.get_mut(&sid) {
                    entry.data.extend_from_slice(slice);
                }
                if (flags & FLAG_END_STREAM) != 0 {
                    if let Some(done) = streams.remove(&sid) {
                        dispatch_grpc_stream(&mut stream, sid, done, &service)?;
                    }
                } else {
                    stream.flush()?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn finalize_stream_headers(entry: &mut H2StreamReq, dyn_table: &mut Vec<(String, String)>) {
    entry.headers_done = true;
    if let Ok(hdrs) = decode_hpack_headers_stateful(&entry.header_block, dyn_table) {
        for (k, v) in hdrs {
            if k == ":path" {
                entry.path = v;
            }
        }
    }
    entry.header_block.clear();
}

fn dispatch_grpc_stream<W: Write>(
    w: &mut W,
    sid: u32,
    req: H2StreamReq,
    service: &EzytelService,
) -> io::Result<()> {
    let mut hdr_block = Vec::new();
    hdr_block.push(0x88); // Indexed :status = 200 (index 8)
    encode_hpack_literal(&mut hdr_block, "content-type", "application/grpc");

    if req.data.len() < 5 {
        return send_grpc_trailers_only(w, sid, &hdr_block, 13, "malformed grpc frame");
    }
    if req.data[0] != 0 {
        return send_grpc_trailers_only(w, sid, &hdr_block, 12, "compressed grpc frame unsupported");
    }
    let msg_len =
        u32::from_be_bytes([req.data[1], req.data[2], req.data[3], req.data[4]]) as usize;
    if req.data.len() < 5 + msg_len {
        return send_grpc_trailers_only(w, sid, &hdr_block, 13, "truncated grpc message");
    }
    let pb_in = &req.data[5..5 + msg_len];

    let result: Result<Vec<u8>, (u8, String)> = match req.path.as_str() {
        "/ezytel.Ezytel/GetChannelInfo" => pb::decode_channel_info_req(pb_in)
            .map_err(|e| (3u8, e))
            .and_then(|r| service.get_channel_info(&r).map_err(|e| (2u8, e)))
            .map(|resp| pb::encode_channel_info(&resp)),
        "/ezytel.Ezytel/GetChannelMessages" => pb::decode_channel_messages_req(pb_in)
            .map_err(|e| (3u8, e))
            .and_then(|r| service.get_channel_messages(&r).map_err(|e| (2u8, e)))
            .map(|resp| pb::encode_channel_messages_resp(&resp)),
        "/ezytel.Ezytel/ProxyImage" => pb::decode_proxy_image_req(pb_in)
            .map_err(|e| (3u8, e))
            .and_then(|r| service.proxy_image(&r).map_err(|e| (2u8, e)))
            .map(|resp| pb::encode_proxy_image_resp(&resp)),
        "/ezytel.Ezytel/ParseChannels" => pb::decode_parse_channels_req(pb_in)
            .map_err(|e| (3u8, e))
            .and_then(|r| service.parse_channels(&r).map_err(|e| (2u8, e)))
            .map(|resp| pb::encode_parse_channels_resp(&resp)),
        _ => Err((12u8, format!("unknown method {}", req.path))),
    };

    match result {
        Ok(pb_out) => {
            write_h2_frame(w, H2_HEADERS, FLAG_END_HEADERS, sid, &hdr_block)?;
            let mut framed = Vec::with_capacity(5 + pb_out.len());
            framed.push(0);
            framed.extend_from_slice(&(pb_out.len() as u32).to_be_bytes());
            framed.extend_from_slice(&pb_out);
            for chunk in framed.chunks(16_384) {
                write_h2_frame(w, H2_DATA, 0, sid, chunk)?;
            }
            let mut trailers = Vec::new();
            encode_hpack_literal(&mut trailers, "grpc-status", "0");
            write_h2_frame(
                w,
                H2_HEADERS,
                FLAG_END_STREAM | FLAG_END_HEADERS,
                sid,
                &trailers,
            )?;
            w.flush()
        }
        Err((code, msg)) => send_grpc_trailers_only(w, sid, &hdr_block, code, &msg),
    }
}

fn send_grpc_trailers_only<W: Write>(
    w: &mut W,
    sid: u32,
    initial_headers: &[u8],
    grpc_status: u8,
    message: &str,
) -> io::Result<()> {
    let mut block = initial_headers.to_vec();
    encode_hpack_literal(&mut block, "grpc-status", &grpc_status.to_string());
    let encoded_msg = utf8_percent_encode(message, GRPC_PERCENT_SET).to_string();
    encode_hpack_literal(&mut block, "grpc-message", &encoded_msg);
    write_h2_frame(
        w,
        H2_HEADERS,
        FLAG_END_STREAM | FLAG_END_HEADERS,
        sid,
        &block,
    )?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_jalali_parse_channels_and_e2e_fronting_and_grpc_match_go() {
        // RFC 1321 MD5 vectors
        assert_eq!(md5sum(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5sum(b"abc"), "900150983cd24fb0d6963f7d28e17f72");

        // ParseChannels parity
        let parsed = parse_channels_raw(
            "  @Durov \nhttps://t.me/s/Hiddify\n\"HTTP://t.me/durov/\"\n  \n'telegram'\n",
        );
        assert_eq!(
            parsed.channel_ids,
            vec![
                "durov".to_string(),
                "hiddify".to_string(),
                "telegram".to_string()
            ]
        );

        // Jalali date conversion parity: 2024-03-20T00:00:00Z -> 2024-03-20 03:30 IRST -> 1403-01-01 03:30 (Nowruz!)
        let ts = parse_rfc3339_unix("2024-03-20T00:00:00+00:00").unwrap();
        assert_eq!(date_convert(ts), "1403-01-01 03:30");
        assert_eq!(date_easy_read(ts, ts + 60), "Today 03:30");
        assert_eq!(date_easy_read(ts, ts + 86_400), "Yesterday 03:30");

        // Loopback domain-fronted upstream + image server
        let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
        let up_addr = upstream.local_addr().unwrap();
        let serve_ok = Arc::new(AtomicBool::new(true));
        let serve_ok_bg = serve_ok.clone();
        thread::spawn(move || {
            for stream in upstream.incoming().flatten() {
                let ok_mode = serve_ok_bg.load(Ordering::Acquire);
                thread::spawn(move || {
                    let mut s = stream;
                    let mut r = BufReader::new(s.try_clone().unwrap());
                    let mut req_line = String::new();
                    if r.read_line(&mut req_line).unwrap_or(0) == 0 {
                        return;
                    }
                    let mut host_hdr = String::new();
                    loop {
                        let mut h = String::new();
                        if r.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                            break;
                        }
                        if let Some((k, v)) = h.split_once(':') {
                            if k.trim().eq_ignore_ascii_case("host") {
                                host_hdr = v.trim().to_string();
                            }
                        }
                    }
                    let path = req_line.split_whitespace().nth(1).unwrap_or("/");
                    if !ok_mode {
                        let _ = s.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 4\r\n\r\nfail");
                        return;
                    }
                    if host_hdr == "t-me.translate.goog" && path.starts_with("/s/durov") {
                        let html = concat!(
                            r#"<html><head><meta property="og:title" content="Pavel Durov">"#,
                            r#"<meta property="og:image" content="https://cdn4.telesco.pe/avatar.jpg"></head>"#,
                            r#"<body><header><div class="tgme_header_title"><span>Pavel Durov</span></div>"#,
                            r#"<div class="tgme_header_counter">10M subscribers</div>"#,
                            r#"<img class="tgme_page_photo_image" src="https://cdn4.telesco.pe/avatar.jpg"></header>"#,
                            r#"<main><div class="messages_more_wrap"><a data-before="40">More</a></div>"#,
                            r#"<div data-post="durov/42"><div class="tgme_widget_message_text">Hello <b>World</b></div>"#,
                            r#"<img src="https://cdn4.telesco.pe/post1.jpg">"#,
                            r#"<time datetime="2024-03-20T00:00:00+00:00">Mar 20</time></div></main></body></html>"#
                        );
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{html}",
                            html.len()
                        );
                        let _ = s.write_all(resp.as_bytes());
                    } else if host_hdr == "cdn4-telesco-pe.translate.goog" {
                        let img = b"\xff\xd8\xff\xe0MOCKJPEG";
                        let hdr = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", img.len());
                        let _ = s.write_all(hdr.as_bytes());
                        let _ = s.write_all(img);
                    } else {
                        let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
                    }
                });
            }
        });

        let cache = tempfile::tempdir().unwrap();
        let svc = Arc::new(EzytelService::with_base_url(
            &cache.path().to_string_lossy(),
            &format!("http://{up_addr}"),
        ));

        let info = svc
            .get_channel_info(&ChannelInfoRequest {
                channel_id: "durov".to_string(),
                last_read: 40,
                disable_inline_images: false,
            })
            .unwrap();
        assert!(info.ok);
        assert_eq!(info.name, "Pavel Durov");
        assert_eq!(info.description, "Hello World");
        assert_eq!(info.last_post_id, 42);
        assert_eq!(info.newmsg, "+2");
        assert_eq!(info.date_str, "01-01 03:30");
        assert!(info.avatar_path.starts_with("data:image/jpeg;base64,"));

        let msgs = svc
            .get_channel_messages(&ChannelMessagesRequest {
                channel_id: "durov".to_string(),
                before: 0,
                disable_inline_images: false,
            })
            .unwrap();
        assert_eq!(msgs.last_post_id, 42);
        assert!(msgs.html.contains("load_more(40,'durov',this)"));
        assert!(msgs.html.contains("data:image/jpeg;base64,"));
        assert!(msgs.html.contains("1403-01-01 03:30"));

        // Offline fallback from cached `<chid>.json` snapshot
        serve_ok.store(false, Ordering::Release);
        let stale = svc
            .get_channel_info(&ChannelInfoRequest {
                channel_id: "durov".to_string(),
                last_read: 40,
                disable_inline_images: false,
            })
            .unwrap();
        assert!(!stale.ok);
        assert_eq!(stale.newmsg, "OFF");
        assert!(stale.avatar_path.starts_with("data:image/jpeg;base64,"));
        serve_ok.store(true, Ordering::Release);

        // End-to-end h2c gRPC server test (`/ezytel.Ezytel/ParseChannels` & `GetChannelInfo`)
        let grpc_lis = TcpListener::bind("127.0.0.1:0").unwrap();
        let grpc_addr = grpc_lis.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_bg = stop.clone();
        let svc_bg = svc.clone();
        let server_join =
            thread::spawn(move || serve_ezytel_grpc_on_listener(grpc_lis, svc_bg, stop_bg));

        let mut client = TcpStream::connect(grpc_addr).unwrap();
        client.write_all(H2_CLIENT_PREFACE).unwrap();
        write_h2_frame(&mut client, H2_SETTINGS, 0, 0, &[]).unwrap();

        let mut hpack = Vec::new();
        hpack.push(0x83); // :method = POST
        hpack.push(0x86); // :scheme = http
        encode_hpack_literal(&mut hpack, ":path", "/ezytel.Ezytel/ParseChannels");
        encode_hpack_literal(&mut hpack, "content-type", "application/grpc");
        write_h2_frame(&mut client, H2_HEADERS, FLAG_END_HEADERS, 1, &hpack).unwrap();

        let pb_req = pb::encode_parse_channels_req(&ParseChannelsRequest {
            raw: "@hiddify\nt.me/durov".to_string(),
        });
        let mut grpc_body = vec![0u8];
        grpc_body.extend_from_slice(&(pb_req.len() as u32).to_be_bytes());
        grpc_body.extend_from_slice(&pb_req);
        write_h2_frame(&mut client, H2_DATA, FLAG_END_STREAM, 1, &grpc_body).unwrap();
        client.flush().unwrap();

        let mut resp_data = Vec::new();
        loop {
            let mut fh = [0u8; 9];
            client.read_exact(&mut fh).unwrap();
            let len = ((fh[0] as usize) << 16) | ((fh[1] as usize) << 8) | (fh[2] as usize);
            let ftype = fh[3];
            let flags = fh[4];
            let sid = u32::from_be_bytes([fh[5] & 0x7f, fh[6], fh[7], fh[8]]);
            let mut payload = vec![0u8; len];
            if len > 0 {
                client.read_exact(&mut payload).unwrap();
            }
            if ftype == H2_DATA && sid == 1 {
                resp_data.extend_from_slice(&payload);
            }
            if sid == 1 && (flags & FLAG_END_STREAM) != 0 {
                break;
            }
        }
        assert!(resp_data.len() > 5);
        let decoded = pb::decode_parse_channels_resp(&resp_data[5..]).unwrap();
        assert_eq!(
            decoded.channel_ids,
            vec!["hiddify".to_string(), "durov".to_string()]
        );

        stop.store(true, Ordering::Release);
        drop(client);
        let _ = server_join.join();
    }
}
