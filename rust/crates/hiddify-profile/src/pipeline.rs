//! End-to-end native subscription fetch, 4-way concurrent inner-link expansion, header population,
//! single-pass atomic temp-file write, and profile metadata resolution (`profile_parser.dart`).

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;

use hiddify_net::cancel::{ensure_task, finish_task, is_cancelled};
use hiddify_net::fetch_url::{fetch_url_ext, FetchHttpHeader, FetchUrlError, FetchUrlRequest};
use hiddify_net::is_url;
use hiddify_net::proxy_dial::LocalHttpProxySpec;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::download::{
    flatten_http_headers, merge_expanded_links, plan_link_expansion, DownloadProblemKind,
};
use crate::{
    decode_text_bytes, parse_subscription_info, parse_update_interval_hours, populate_headers,
    resolve_profile_name, safe_decode_base64, SubscriptionInfoSpec,
};

const INNER_LINK_CONCURRENCY: usize = 4;
const DEFAULT_CORE_FETCH_TIMEOUT_MS: i32 = 30_000;
const DEFAULT_APP_FETCH_TIMEOUT_MS: i32 = 15_000;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileMetadataInput {
    pub override_name: Option<String>,
    pub override_update_interval: Option<i64>,
    #[serde(default)]
    pub is_auto_update_disable: bool,
    pub remote_url: Option<String>,
    pub existing_local_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedProfileMetadata {
    pub name: String,
    pub update_interval_hours: Option<i64>,
    pub sub_info: Option<SubscriptionInfoSpec>,
    pub web_page_url: Option<String>,
    pub support_url: Option<String>,
    pub populated_headers: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PriorCoreFailure {
    /// `Some(problem_idx)` if the core returned an HTTP error or classified network error;
    /// `None` if the core itself was unavailable (`"core: unavailable"`).
    pub problem_index: Option<i32>,
    pub status_code: Option<i32>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteProfileAttemptRequest {
    pub url: String,
    pub temp_file_path: String,
    #[serde(default)]
    pub user_agent: String,
    #[serde(default)]
    pub proxy_port: u16,
    #[serde(default)]
    pub proxy_secret: String,
    #[serde(default)]
    pub core_timeout_ms: i32,
    #[serde(default)]
    pub app_timeout_ms: i32,
    #[serde(default)]
    pub skip_core_direct: bool,
    pub prior_core_failure: Option<PriorCoreFailure>,
    #[serde(default)]
    pub cancel_task_id: u64,
    #[serde(default)]
    pub metadata: ProfileMetadataInput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteProfileAttemptResult {
    /// `"ok"`, `"cancelled"`, `"download_failed"`, or `"unexpected_error"`.
    pub status: String,
    pub resolved: Option<ResolvedProfileMetadata>,
    pub problem_index: Option<i32>,
    pub status_code: Option<i32>,
    pub detail: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalProfilePrepareRequest {
    pub content: String,
    pub temp_file_path: String,
    #[serde(default)]
    pub user_agent: String,
    #[serde(default)]
    pub proxy_port: u16,
    #[serde(default)]
    pub proxy_secret: String,
    #[serde(default)]
    pub timeout_ms: i32,
    #[serde(default)]
    pub cancel_task_id: u64,
    #[serde(default)]
    pub metadata: ProfileMetadataInput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpandLinksRequest {
    pub text: String,
    #[serde(default)]
    pub decode_base64_first: bool,
    #[serde(default)]
    pub user_agent: String,
    #[serde(default)]
    pub proxy_port: u16,
    #[serde(default)]
    pub proxy_secret: String,
    #[serde(default)]
    pub timeout_ms: i32,
    #[serde(default)]
    pub cancel_task_id: u64,
}

fn headers_to_flattened_map(headers: &[FetchHttpHeader]) -> Map<String, Value> {
    let mut grouped: IndexMap<String, Vec<String>> = IndexMap::new();
    for h in headers {
        let key = h.name.to_ascii_lowercase();
        grouped.entry(key).or_default().push(h.value.clone());
    }
    flatten_http_headers(&grouped)
}

fn write_file_atomic_or_direct(path_str: &str, content: &[u8]) -> Result<(), String> {
    if path_str.trim().is_empty() {
        return Ok(());
    }
    let path = Path::new(path_str);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| format!("create profile temp dir: {e}"))?;
        }
    }
    fs::write(path, content).map_err(|e| format!("write profile temp file: {e}"))
}

fn resolve_metadata_from_headers_and_content(
    populated_headers: Map<String, Value>,
    meta: &ProfileMetadataInput,
    local_content: &str,
) -> ResolvedProfileMetadata {
    let title_hdr = populated_headers.get("profile-title").and_then(Value::as_str);
    let cd_hdr = populated_headers
        .get("content-disposition")
        .and_then(Value::as_str);

    let name = resolve_profile_name(
        meta.override_name.as_deref(),
        title_hdr,
        cd_hdr,
        meta.remote_url.as_deref(),
        meta.existing_local_name.as_deref(),
        if meta.remote_url.is_some() {
            None
        } else {
            Some(local_content)
        },
    );

    let update_interval_hours = parse_update_interval_hours(
        populated_headers
            .get("profile-update-interval")
            .and_then(Value::as_str),
        meta.override_update_interval,
        meta.is_auto_update_disable,
    );

    let sub_info = populated_headers
        .get("subscription-userinfo")
        .and_then(Value::as_str)
        .and_then(parse_subscription_info);

    let web_page_url = populated_headers
        .get("profile-web-page-url")
        .and_then(Value::as_str)
        .filter(|u| is_url(u))
        .map(str::to_string);

    let support_url = populated_headers
        .get("support-url")
        .and_then(Value::as_str)
        .filter(|u| is_url(u))
        .map(str::to_string);

    ResolvedProfileMetadata {
        name,
        update_interval_hours,
        sub_info,
        web_page_url,
        support_url,
        populated_headers,
    }
}

/// Expands all inner HTTP(S) subscription links in `raw_text` using up to 4 concurrent Rust threads.
/// Returns `Err("cancelled")` if `cancel_task_id` is cancelled.
pub fn expand_links_native(req: &ExpandLinksRequest) -> Result<String, String> {
    ensure_task(req.cancel_task_id);
    if is_cancelled(req.cancel_task_id) {
        return Err("cancelled".to_string());
    }

    let plan = plan_link_expansion(&req.text, req.decode_base64_first);
    if plan.targets.is_empty() {
        return Ok(plan.decoded_text);
    }

    let timeout_ms = if req.timeout_ms > 0 {
        req.timeout_ms
    } else {
        DEFAULT_APP_FETCH_TIMEOUT_MS
    };
    let proxy = LocalHttpProxySpec {
        port: req.proxy_port,
        secret: req.proxy_secret.clone(),
    };
    let cursor = AtomicUsize::new(0);
    let replacements: Mutex<Vec<(usize, String)>> =
        Mutex::new(Vec::with_capacity(plan.targets.len()));
    let worker_count = plan.targets.len().min(INNER_LINK_CONCURRENCY).max(1);

    thread::scope(|s| {
        for _ in 0..worker_count {
            s.spawn(|| loop {
                if is_cancelled(req.cancel_task_id) {
                    return;
                }
                let pos = cursor.fetch_add(1, Ordering::Relaxed);
                let Some(target) = plan.targets.get(pos) else {
                    return;
                };
                let fetch_req = FetchUrlRequest {
                    url: target.url.clone(),
                    user_agent: req.user_agent.clone(),
                    timeout_ms,
                };
                let body_str = match fetch_url_ext(
                    &fetch_req,
                    Some(&proxy),
                    true,
                    req.cancel_task_id,
                    None,
                ) {
                    Ok(out)
                        if out.error == FetchUrlError::None
                            && (200..=299).contains(&out.status_code) =>
                    {
                        decode_text_bytes(&out.body)
                    }
                    _ => String::new(),
                };
                if let Ok(mut list) = replacements.lock() {
                    list.push((target.index, body_str));
                }
            });
        }
    });

    if is_cancelled(req.cancel_task_id) {
        return Err("cancelled".to_string());
    }

    let reps = replacements.into_inner().unwrap_or_default();
    Ok(merge_expanded_links(&plan.decoded_text, &reps))
}

/// Processes already-downloaded bytes + headers (e.g. from direct fetch or a unit-test `CoreFetch` mock):
/// populates headers, expands inner links concurrently, writes `temp_file_path` once, and resolves metadata.
pub fn finalize_downloaded_remote_bytes(
    raw_bytes: &[u8],
    headers: &[FetchHttpHeader],
    temp_file_path: &str,
    user_agent: &str,
    proxy_port: u16,
    proxy_secret: &str,
    timeout_ms: i32,
    cancel_task_id: u64,
    metadata: &ProfileMetadataInput,
) -> RemoteProfileAttemptResult {
    let raw_content = decode_text_bytes(raw_bytes);
    let remote_headers = headers_to_flattened_map(headers);
    let populated = populate_headers(&raw_content, Some(&remote_headers));

    let decoded_trimmed = safe_decode_base64(raw_content.trim());
    let final_text = match expand_links_native(&ExpandLinksRequest {
        text: decoded_trimmed.clone(),
        decode_base64_first: false,
        user_agent: user_agent.to_string(),
        proxy_port,
        proxy_secret: proxy_secret.to_string(),
        timeout_ms,
        cancel_task_id,
    }) {
        Ok(expanded) => {
            if expanded != decoded_trimmed {
                expanded
            } else {
                raw_content.clone()
            }
        }
        Err(msg) if msg == "cancelled" => {
            return RemoteProfileAttemptResult {
                status: "cancelled".to_string(),
                resolved: None,
                problem_index: None,
                status_code: None,
                detail: None,
                error: None,
            };
        }
        Err(err) => {
            return RemoteProfileAttemptResult {
                status: "unexpected_error".to_string(),
                resolved: None,
                problem_index: None,
                status_code: None,
                detail: None,
                error: Some(err),
            };
        }
    };

    if let Err(err) = write_file_atomic_or_direct(temp_file_path, final_text.as_bytes()) {
        return RemoteProfileAttemptResult {
            status: "unexpected_error".to_string(),
            resolved: None,
            problem_index: None,
            status_code: None,
            detail: None,
            error: Some(err),
        };
    }

    let resolved = resolve_metadata_from_headers_and_content(populated, metadata, &final_text);
    RemoteProfileAttemptResult {
        status: "ok".to_string(),
        resolved: Some(resolved),
        problem_index: None,
        status_code: None,
        detail: None,
        error: None,
    }
}

/// Executes one complete remote subscription download attempt:
/// 1. Direct Chrome-like TLS fetch (`skip_core_direct == false`)
/// 2. Fallback app HTTP fetch via `"PROXY 127.0.0.1:<port>; DIRECT"` (single attempt, no inner retries)
/// 3. `DownloadProblemKind::most_telling` failure classification or full header/link-expansion/file-write finalization.
pub fn fetch_and_prepare_remote_attempt(
    req: &RemoteProfileAttemptRequest,
) -> RemoteProfileAttemptResult {
    let task_id = req.cancel_task_id;
    ensure_task(task_id);
    let finish = |res: RemoteProfileAttemptResult| -> RemoteProfileAttemptResult {
        finish_task(task_id);
        res
    };

    if is_cancelled(task_id) {
        return finish(RemoteProfileAttemptResult {
            status: "cancelled".to_string(),
            resolved: None,
            problem_index: None,
            status_code: None,
            detail: None,
            error: None,
        });
    }

    let link = req.url.trim();
    let mut problems: Vec<(DownloadProblemKind, Option<i32>)> = Vec::new();
    let mut details: Vec<String> = Vec::new();

    if let Some(ref prior) = req.prior_core_failure {
        if let Some(pidx) = prior.problem_index {
            problems.push((DownloadProblemKind::from_index(pidx), prior.status_code));
        }
        details.push(prior.detail.clone());
    } else if !req.skip_core_direct {
        let core_timeout = if req.core_timeout_ms > 0 {
            req.core_timeout_ms
        } else {
            DEFAULT_CORE_FETCH_TIMEOUT_MS
        };
        match fetch_url_ext(
            &FetchUrlRequest {
                url: link.to_string(),
                user_agent: req.user_agent.clone(),
                timeout_ms: core_timeout,
            },
            None,
            false,
            task_id,
            None,
        ) {
            Err(msg) if msg == "cancelled" || is_cancelled(task_id) => {
                return finish(RemoteProfileAttemptResult {
                    status: "cancelled".to_string(),
                    resolved: None,
                    problem_index: None,
                    status_code: None,
                    detail: None,
                    error: None,
                });
            }
            Err(_) => {
                details.push("core: unavailable".to_string());
            }
            Ok(rs) => match DownloadProblemKind::of_core(rs.error.as_i32(), rs.status_code) {
                None => {
                    return finish(finalize_downloaded_remote_bytes(
                        &rs.body,
                        &rs.headers,
                        &req.temp_file_path,
                        &req.user_agent,
                        req.proxy_port,
                        &req.proxy_secret,
                        req.app_timeout_ms,
                        task_id,
                        &req.metadata,
                    ));
                }
                Some(problem) => {
                    let has_status = rs.error == FetchUrlError::None;
                    problems.push((problem, has_status.then_some(rs.status_code)));
                    let d = if has_status {
                        format!("core: HTTP {}", rs.status_code)
                    } else {
                        format!("core: {}", rs.error_detail)
                    };
                    details.push(d);
                }
            },
        }
    }

    // Fallback: app download ("PROXY 127.0.0.1:<port>; DIRECT", single attempt without retries)
    let app_timeout = if req.app_timeout_ms > 0 {
        req.app_timeout_ms
    } else {
        DEFAULT_APP_FETCH_TIMEOUT_MS
    };
    let proxy = LocalHttpProxySpec {
        port: req.proxy_port,
        secret: req.proxy_secret.clone(),
    };

    match fetch_url_ext(
        &FetchUrlRequest {
            url: link.to_string(),
            user_agent: req.user_agent.clone(),
            timeout_ms: app_timeout,
        },
        Some(&proxy),
        true,
        task_id,
        None,
    ) {
        Err(msg) if msg == "cancelled" || is_cancelled(task_id) => {
            return finish(RemoteProfileAttemptResult {
                status: "cancelled".to_string(),
                resolved: None,
                problem_index: None,
                status_code: None,
                detail: None,
                error: None,
            });
        }
        Err(msg) => {
            let scrubbed = msg.replace(link, "<url>");
            problems.push((DownloadProblemKind::Unknown, None));
            details.push(format!("app: {scrubbed}"));
        }
        Ok(rs) => match DownloadProblemKind::of_core(rs.error.as_i32(), rs.status_code) {
            None => {
                return finish(finalize_downloaded_remote_bytes(
                    &rs.body,
                    &rs.headers,
                    &req.temp_file_path,
                    &req.user_agent,
                    req.proxy_port,
                    &req.proxy_secret,
                    app_timeout,
                    task_id,
                    &req.metadata,
                ));
            }
            Some(problem) => {
                let has_status = rs.error == FetchUrlError::None;
                problems.push((problem, has_status.then_some(rs.status_code)));
                let d = if has_status {
                    format!("app: HTTP {}", rs.status_code)
                } else if rs.error_detail.is_empty() {
                    format!("app: {}", problem.code_str(None))
                } else {
                    format!("app: {}", rs.error_detail.replace(link, "<url>"))
                };
                details.push(d);
            }
        },
    }

    let only_kinds: Vec<DownloadProblemKind> = problems.iter().map(|(k, _)| *k).collect();
    let best = DownloadProblemKind::most_telling(&only_kinds);
    let best_status = problems
        .iter()
        .find(|(k, _)| *k == best)
        .and_then(|(_, code)| *code);

    finish(RemoteProfileAttemptResult {
        status: "download_failed".to_string(),
        resolved: None,
        problem_index: Some(best.index()),
        status_code: best_status,
        detail: Some(details.join("\n")),
        error: None,
    })
}

/// Prepares a local profile in a single Rust call (`ProfileParser.addLocal`):
/// populates comment headers, expands any inner subscription links in `temp_file_path` (rewriting
/// the file only when expanded text differs), and resolves the local profile metadata.
pub fn prepare_local_profile(req: &LocalProfilePrepareRequest) -> RemoteProfileAttemptResult {
    let task_id = req.cancel_task_id;
    ensure_task(task_id);
    let finish = |res: RemoteProfileAttemptResult| -> RemoteProfileAttemptResult {
        finish_task(task_id);
        res
    };

    let populated = populate_headers(&req.content, None);

    let file_raw = if !req.temp_file_path.trim().is_empty() && Path::new(&req.temp_file_path).exists() {
        match fs::read_to_string(&req.temp_file_path) {
            Ok(s) => s,
            Err(e) => {
                return finish(RemoteProfileAttemptResult {
                    status: "unexpected_error".to_string(),
                    resolved: None,
                    problem_index: None,
                    status_code: None,
                    detail: None,
                    error: Some(e.to_string()),
                });
            }
        }
    } else {
        req.content.clone()
    };

    let decoded = safe_decode_base64(file_raw.trim());
    let final_for_name = match expand_links_native(&ExpandLinksRequest {
        text: decoded.clone(),
        decode_base64_first: false,
        user_agent: req.user_agent.clone(),
        proxy_port: req.proxy_port,
        proxy_secret: req.proxy_secret.clone(),
        timeout_ms: req.timeout_ms,
        cancel_task_id: task_id,
    }) {
        Ok(expanded) => {
            if expanded != decoded && !req.temp_file_path.trim().is_empty() {
                if let Err(e) = write_file_atomic_or_direct(&req.temp_file_path, expanded.as_bytes()) {
                    return finish(RemoteProfileAttemptResult {
                        status: "unexpected_error".to_string(),
                        resolved: None,
                        problem_index: None,
                        status_code: None,
                        detail: None,
                        error: Some(e),
                    });
                }
                expanded
            } else {
                file_raw
            }
        }
        Err(msg) if msg == "cancelled" => {
            return finish(RemoteProfileAttemptResult {
                status: "cancelled".to_string(),
                resolved: None,
                problem_index: None,
                status_code: None,
                detail: None,
                error: None,
            });
        }
        Err(err) => {
            return finish(RemoteProfileAttemptResult {
                status: "unexpected_error".to_string(),
                resolved: None,
                problem_index: None,
                status_code: None,
                detail: None,
                error: Some(err),
            });
        }
    };

    let resolved =
        resolve_metadata_from_headers_and_content(populated, &req.metadata, &final_for_name);
    finish(RemoteProfileAttemptResult {
        status: "ok".to_string(),
        resolved: Some(resolved),
        problem_index: None,
        status_code: None,
        detail: None,
        error: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    #[test]
    fn remote_pipeline_fetches_expands_inner_links_concurrently_and_writes_once() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                thread::spawn(move || {
                    let mut s = stream;
                    let mut r = BufReader::new(s.try_clone().unwrap());
                    let mut req_line = String::new();
                    let _ = r.read_line(&mut req_line);
                    loop {
                        let mut h = String::new();
                        if r.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                            break;
                        }
                    }
                    let path = req_line.split_whitespace().nth(1).unwrap_or("/");
                    match path {
                        "/main" => {
                            let body = format!(
                                "# profile-update-interval: 6\nhttp://{addr}/sub1\nhttp://{addr}/sub2"
                            );
                            let resp = format!(
                                "HTTP/1.1 200 OK\r\nProfile-Title: base64:TXkgVmlw\r\nSubscription-Userinfo: upload=10;download=20;total=100;expire=1800000000\r\nContent-Length: {}\r\n\r\n{body}",
                                body.len()
                            );
                            let _ = s.write_all(resp.as_bytes());
                        }
                        "/sub1" => {
                            let body = "vless://node1@1.1.1.1:443#Node1";
                            let resp = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                                body.len()
                            );
                            let _ = s.write_all(resp.as_bytes());
                        }
                        "/sub2" => {
                            let body = "trojan://node2@2.2.2.2:443#Node2";
                            let resp = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                                body.len()
                            );
                            let _ = s.write_all(resp.as_bytes());
                        }
                        _ => {
                            let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
                        }
                    }
                });
            }
        });

        let dir = tempfile::tempdir().unwrap();
        let tmp = dir.path().join("sub.txt");
        let res = fetch_and_prepare_remote_attempt(&RemoteProfileAttemptRequest {
            url: format!("http://{addr}/main"),
            temp_file_path: tmp.to_string_lossy().into_owned(),
            user_agent: "HiddifyTest/5".to_string(),
            proxy_port: 0,
            proxy_secret: String::new(),
            core_timeout_ms: 5_000,
            app_timeout_ms: 5_000,
            skip_core_direct: false,
            prior_core_failure: None,
            cancel_task_id: 0,
            metadata: ProfileMetadataInput {
                remote_url: Some(format!("http://{addr}/main")),
                ..Default::default()
            },
        });

        assert_eq!(res.status, "ok", "{res:?}");
        let meta = res.resolved.expect("resolved");
        assert_eq!(meta.name, "My Vip");
        assert_eq!(meta.update_interval_hours, Some(6));
        assert_eq!(meta.sub_info.unwrap().download, 20);

        let written = fs::read_to_string(&tmp).unwrap();
        assert!(written.contains("vless://node1@1.1.1.1:443#Node1"));
        assert!(written.contains("trojan://node2@2.2.2.2:443#Node2"));
        assert!(!written.contains("/sub1"));
    }
}
