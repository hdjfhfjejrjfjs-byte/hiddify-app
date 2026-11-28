//! Unified C-ABI FFI facade (`libhiddify_rust`) for iOS, Android, macOS, and Linux.
//!
//! Safety invariants:
//! - Every `extern "C"` function catches panics via `std::panic::catch_unwind` so no Rust unwind
//!   can ever cross the C ABI boundary.
//! - Every returned `char*` is allocated via `CString::into_raw` and MUST be freed by calling
//!   `hiddify_rust_free_string`.
//! - Boolean predicates return `1` (`true`), `0` (`false`), or `-1` on invalid pointer/UTF-8/panic.

use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::slice;

use hiddify_config::{OptionalRange, UserOverrideSpec};
use hiddify_crypto::MtlsIdentity;
use hiddify_diagnostics::TelegramReportInput;
use hiddify_profile::download::DownloadProblemKind;
use indexmap::IndexMap;
use serde_json::{json, Map, Value};

const ABI_VERSION: u32 = 5;

unsafe fn cstr_to_str<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    CStr::from_ptr(ptr).to_str().ok()
}

fn into_c_string(s: String) -> *mut c_char {
    // Replace any interior NUL byte defensively so CString never fails
    let sanitized = if s.as_bytes().contains(&0) {
        s.replace('\0', "")
    } else {
        s
    };
    CString::new(sanitized)
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

fn ffi_bool<F: FnOnce() -> Option<bool>>(f: F) -> i32 {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Some(true)) => 1,
        Ok(Some(false)) => 0,
        _ => -1,
    }
}

fn ffi_string<F: FnOnce() -> Option<String>>(f: F) -> *mut c_char {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Some(s)) => into_c_string(s),
        _ => std::ptr::null_mut(),
    }
}

/// Returns the C-ABI contract version (`1`).
#[no_mangle]
pub extern "C" fn hiddify_rust_abi_version() -> u32 {
    ABI_VERSION
}

/// Frees a NUL-terminated string previously returned by any `hiddify_rust_*` function.
#[no_mangle]
pub unsafe extern "C" fn hiddify_rust_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            drop(CString::from_raw(ptr));
        }));
    }
}

// ============================================================================
// 1. hiddify-crypto FFI
// ============================================================================

/// Generates a new RSA-2048 mTLS identity and returns JSON `{"certificatePem": "...", "privateKeyPem": "..."}`.
#[no_mangle]
pub extern "C" fn hiddify_crypto_generate_identity_json() -> *mut c_char {
    ffi_string(|| {
        let id = MtlsIdentity::generate().ok()?;
        Some(
            json!({
                "certificatePem": id.certificate_pem,
                "privateKeyPem": id.private_key_pem,
            })
            .to_string(),
        )
    })
}

/// Loads or atomically creates an mTLS client identity in `directory_path`, returning JSON.
#[no_mangle]
pub unsafe extern "C" fn hiddify_crypto_load_or_create_identity_json(
    directory_path: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        let dir = cstr_to_str(directory_path)?;
        let id = MtlsIdentity::load_or_create(Path::new(dir)).ok()?;
        Some(
            json!({
                "certificatePem": id.certificate_pem,
                "privateKeyPem": id.private_key_pem,
            })
            .to_string(),
        )
    })
}

/// Checks whether a PEM certificate + PKCS#8 key pair is valid past `now + renew_before_secs`.
#[no_mangle]
pub unsafe extern "C" fn hiddify_crypto_is_identity_usable(
    certificate_pem: *const c_char,
    private_key_pem: *const c_char,
    renew_before_secs: u64,
) -> i32 {
    ffi_bool(|| {
        let cert = cstr_to_str(certificate_pem)?;
        let key = cstr_to_str(private_key_pem)?;
        Some(hiddify_crypto::is_identity_usable(
            cert,
            key,
            renew_before_secs,
        ))
    })
}

/// Verifies a presented server certificate (`presented_der` + `presented_pem`) against `expected_server_pem` and `host`.
#[no_mangle]
pub unsafe extern "C" fn hiddify_crypto_verify_pinned_cert(
    presented_der_ptr: *const u8,
    presented_der_len: usize,
    presented_pem: *const c_char,
    expected_server_pem: *const c_char,
    host: *const c_char,
) -> i32 {
    ffi_bool(|| {
        let der = if presented_der_ptr.is_null() || presented_der_len == 0 {
            &[]
        } else {
            slice::from_raw_parts(presented_der_ptr, presented_der_len)
        };
        let p_pem = cstr_to_str(presented_pem).unwrap_or("");
        let e_pem = cstr_to_str(expected_server_pem)?;
        let h = cstr_to_str(host)?;
        Some(hiddify_crypto::verify_pinned_certificate(
            der, p_pem, e_pem, h,
        ))
    })
}

/// Generates an unambiguous dash-grouped proxy secret of `length` characters.
#[no_mangle]
pub extern "C" fn hiddify_crypto_generate_proxy_secret(length: usize) -> *mut c_char {
    ffi_string(|| Some(hiddify_crypto::generate_proxy_secret(length)))
}

/// Validates a `sha256:<64 hex chars>` release asset digest string.
#[no_mangle]
pub unsafe extern "C" fn hiddify_crypto_is_valid_sha256_digest(digest: *const c_char) -> i32 {
    ffi_bool(|| Some(hiddify_crypto::is_valid_sha256_digest(cstr_to_str(digest)?)))
}

/// Streams `file_path` and verifies both `expected_size` (`0` = ignore) and `expected_digest`.
#[no_mangle]
pub unsafe extern "C" fn hiddify_crypto_verify_file_sha256(
    file_path: *const c_char,
    expected_size: u64,
    expected_digest: *const c_char,
) -> i32 {
    ffi_bool(|| {
        let path = cstr_to_str(file_path)?;
        let digest = cstr_to_str(expected_digest)?;
        Some(hiddify_crypto::verify_file_sha256(
            Path::new(path),
            expected_size,
            digest,
        ))
    })
}

// ============================================================================
// 2. hiddify-net FFI
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn hiddify_net_is_url(input: *const c_char) -> i32 {
    ffi_bool(|| Some(hiddify_net::is_url(cstr_to_str(input)?)))
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_net_is_port(input: *const c_char) -> i32 {
    ffi_bool(|| Some(hiddify_net::is_port(cstr_to_str(input)?)))
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_net_is_port_or_port_range(input: *const c_char) -> i32 {
    ffi_bool(|| Some(hiddify_net::is_port_or_port_range(cstr_to_str(input)?)))
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_net_is_process_name(input: *const c_char) -> i32 {
    ffi_bool(|| Some(hiddify_net::is_process_name(cstr_to_str(input)?)))
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_net_is_process_path(input: *const c_char) -> i32 {
    ffi_bool(|| Some(hiddify_net::is_process_path(cstr_to_str(input)?)))
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_net_is_ip_cidr(input: *const c_char) -> i32 {
    ffi_bool(|| Some(hiddify_net::is_ip_cidr(cstr_to_str(input)?)))
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_net_is_domain(input: *const c_char) -> i32 {
    ffi_bool(|| Some(hiddify_net::is_domain(cstr_to_str(input)?)))
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_net_is_domain_suffix(input: *const c_char) -> i32 {
    ffi_bool(|| Some(hiddify_net::is_domain_suffix(cstr_to_str(input)?)))
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_net_obscure_ip(ip: *const c_char) -> *mut c_char {
    ffi_string(|| Some(hiddify_net::obscure_ip(cstr_to_str(ip)?)))
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_net_sanitize_tag(tag: *const c_char) -> *mut c_char {
    ffi_string(|| Some(hiddify_net::sanitize_tag(cstr_to_str(tag)?)))
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_net_strip_status_mark(proxy_type: *const c_char) -> *mut c_char {
    ffi_string(|| Some(hiddify_net::strip_status_mark(cstr_to_str(proxy_type)?)))
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_net_is_invalid_proxy_type(
    type_without_status: *const c_char,
) -> i32 {
    ffi_bool(|| {
        Some(hiddify_net::is_invalid_proxy_type(cstr_to_str(
            type_without_status,
        )?))
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_net_split_scanner_endpoint_json(
    proxy_type: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        let parts = hiddify_net::split_scanner_endpoint(cstr_to_str(proxy_type)?)?;
        serde_json::to_string(&parts).ok()
    })
}

// ============================================================================
// 3. hiddify-config FFI
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn hiddify_config_profile_proxies_json(
    config_json: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        let proxies = hiddify_config::profile_proxies(cstr_to_str(config_json)?).ok()?;
        serde_json::to_string(&proxies).ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_config_count_proxies(config_json: *const c_char) -> i64 {
    match catch_unwind(AssertUnwindSafe(|| {
        let cfg = cstr_to_str(config_json)?;
        Some(hiddify_config::count_proxies(cfg) as i64)
    })) {
        Ok(Some(n)) => n,
        _ => -1,
    }
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_config_country_of(tag: *const c_char) -> *mut c_char {
    ffi_string(|| hiddify_config::country_of(cstr_to_str(tag)?))
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_config_flag_of(code: *const c_char) -> *mut c_char {
    ffi_string(|| Some(hiddify_config::flag_of(cstr_to_str(code)?)))
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_config_protocol_name(proxy_type: *const c_char) -> *mut c_char {
    ffi_string(|| Some(hiddify_config::protocol_name(cstr_to_str(proxy_type)?)))
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_config_editable_config_json(
    content: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        let res = hiddify_config::editable_config(cstr_to_str(content)?);
        serde_json::to_string(&res).ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_config_single_proxy_config(
    tag: *const c_char,
    host: *const c_char,
    port: i64,
    raw_config: *const c_char,
    generated_config: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        let tag_s = cstr_to_str(tag)?;
        let host_s = cstr_to_str(host).unwrap_or("");
        let raw_opt = cstr_to_str(raw_config);
        let gen_opt = cstr_to_str(generated_config);
        hiddify_config::single_proxy_config(tag_s, host_s, port, raw_opt, gen_opt)
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_config_profile_override(
    populated_headers_json: *const c_char,
    user_override_json: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        let headers: Option<Map<String, Value>> = match cstr_to_str(populated_headers_json) {
            Some(s) if !s.is_empty() => serde_json::from_str(s).ok(),
            _ => None,
        };
        let uo: Option<UserOverrideSpec> = match cstr_to_str(user_override_json) {
            Some(s) if !s.is_empty() => serde_json::from_str(s).ok(),
            _ => None,
        };
        Some(hiddify_config::profile_override(
            headers.as_ref(),
            uo.as_ref(),
        ))
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_config_apply_profile_override(
    main_config_json: *const c_char,
    profile_override_str: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        let main_map: Map<String, Value> = serde_json::from_str(cstr_to_str(main_config_json)?)
            .ok()?;
        let override_opt = cstr_to_str(profile_override_str);
        let merged = hiddify_config::apply_profile_override(&main_map, override_opt).ok()?;
        serde_json::to_string(&merged).ok()
    })
}

// ============================================================================
// 4. hiddify-profile FFI
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn hiddify_profile_safe_decode_base64(input: *const c_char) -> *mut c_char {
    ffi_string(|| Some(hiddify_profile::safe_decode_base64(cstr_to_str(input)?)))
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_profile_decode_base64_title(value: *const c_char) -> *mut c_char {
    ffi_string(|| Some(hiddify_profile::decode_base64_title(cstr_to_str(value)?)))
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_profile_filename_from_content_disposition(
    header: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        Some(hiddify_profile::filename_from_content_disposition(
            cstr_to_str(header)?,
        ))
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_profile_populate_headers_json(
    content: *const c_char,
    remote_headers_json: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        let content_s = cstr_to_str(content)?;
        let remote: Option<Map<String, Value>> = match cstr_to_str(remote_headers_json) {
            Some(s) if !s.is_empty() => serde_json::from_str(s).ok(),
            _ => None,
        };
        let populated = hiddify_profile::populate_headers(content_s, remote.as_ref());
        serde_json::to_string(&populated).ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_profile_parse_subscription_info_json(
    sub_info_str: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        let info = hiddify_profile::parse_subscription_info(cstr_to_str(sub_info_str)?)?;
        serde_json::to_string(&info).ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_profile_detect_protocol(content: *const c_char) -> *mut c_char {
    ffi_string(|| Some(hiddify_profile::detect_protocol(cstr_to_str(content)?)))
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_profile_is_subscription_link(line: *const c_char) -> i32 {
    ffi_bool(|| Some(hiddify_profile::is_subscription_link(cstr_to_str(line)?)))
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_profile_parse_link_json(link: *const c_char) -> *mut c_char {
    ffi_string(|| {
        let parsed = hiddify_profile::parse_link(cstr_to_str(link)?)?;
        serde_json::to_string(&parsed).ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_profile_parse_simple_link_json(
    link: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        let parsed = hiddify_profile::parse_simple_link(cstr_to_str(link)?)?;
        serde_json::to_string(&parsed).ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_profile_parse_deep_link_json(link: *const c_char) -> *mut c_char {
    ffi_string(|| {
        let parsed = hiddify_profile::parse_deep_link(cstr_to_str(link)?)?;
        serde_json::to_string(&parsed).ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_profile_generate_sub_share_link(
    url_str: *const c_char,
    name: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        let u = cstr_to_str(url_str)?;
        let n = cstr_to_str(name);
        Some(hiddify_profile::generate_sub_share_link(u, n))
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_profile_decode_text_bytes(
    bytes_ptr: *const u8,
    bytes_len: usize,
) -> *mut c_char {
    ffi_string(|| {
        let bytes = if bytes_ptr.is_null() || bytes_len == 0 {
            &[]
        } else {
            slice::from_raw_parts(bytes_ptr, bytes_len)
        };
        Some(hiddify_profile::decode_text_bytes(bytes))
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_profile_read_zip_text(
    bytes_ptr: *const u8,
    bytes_len: usize,
) -> *mut c_char {
    ffi_string(|| {
        if bytes_ptr.is_null() || bytes_len == 0 {
            return Some(String::new());
        }
        let bytes = slice::from_raw_parts(bytes_ptr, bytes_len);
        Some(hiddify_profile::read_zip_text(bytes))
    })
}

// ============================================================================
// 5. hiddify-diagnostics FFI
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_telegram_payload(
    os: *const c_char,
    ov: *const c_char,
    v: *const c_char,
    t: *const c_char,
    pm: *const c_char,
    l: *const c_char,
    a: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        let input = TelegramReportInput {
            os: cstr_to_str(os)?,
            ov: cstr_to_str(ov)?,
            v: cstr_to_str(v)?,
            t: cstr_to_str(t)?,
            pm: cstr_to_str(pm),
            l: cstr_to_str(l),
            a: cstr_to_str(a),
        };
        Some(hiddify_diagnostics::telegram_report_payload(&input))
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_windows_version(
    os_version: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        Some(hiddify_diagnostics::windows_version(cstr_to_str(
            os_version,
        )?))
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_linux_distro_json(
    os_release: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        let (code, version) = hiddify_diagnostics::linux_distro(cstr_to_str(os_release)?);
        Some(json!({"code": code, "version": version}).to_string())
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_crash_excerpt_json(
    content: *const c_char,
    find_go_marker: bool,
) -> *mut c_char {
    ffi_string(|| {
        let ex =
            hiddify_diagnostics::extract_crash_excerpt(cstr_to_str(content)?, find_go_marker)?;
        serde_json::to_string(&ex).ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_extract_changelog(body: *const c_char) -> *mut c_char {
    ffi_string(|| hiddify_diagnostics::extract_changelog(cstr_to_str(body)?))
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_parse_release_tag_json(
    tag_name: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        let parsed = hiddify_diagnostics::parse_release_tag(cstr_to_str(tag_name)?)?;
        serde_json::to_string(&parsed).ok()
    })
}

// ============================================================================
// 6. Phase 2 Extensions — Crypto, Net, Config, Profile, Diagnostics
// ============================================================================

#[no_mangle]
pub extern "C" fn hiddify_crypto_generate_numeric_pin(length: usize) -> *mut c_char {
    ffi_string(|| Some(hiddify_crypto::generate_numeric_pin(length)))
}

#[no_mangle]
pub extern "C" fn hiddify_crypto_generate_url_token(byte_len: usize) -> *mut c_char {
    ffi_string(|| Some(hiddify_crypto::generate_url_token(byte_len)))
}

#[no_mangle]
pub extern "C" fn hiddify_crypto_generate_alphanumeric_secret(length: usize) -> *mut c_char {
    ffi_string(|| Some(hiddify_crypto::generate_alphanumeric_secret(length)))
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_crypto_verify_secret_path(
    path: *const c_char,
    token: *const c_char,
) -> i32 {
    ffi_bool(|| {
        Some(hiddify_crypto::verify_secret_path(
            cstr_to_str(path)?,
            cstr_to_str(token)?,
        ))
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_net_is_local_network_bytes(
    raw_ip_ptr: *const u8,
    raw_ip_len: usize,
) -> i32 {
    ffi_bool(|| {
        if raw_ip_ptr.is_null() || raw_ip_len == 0 {
            return Some(false);
        }
        let raw = slice::from_raw_parts(raw_ip_ptr, raw_ip_len);
        Some(hiddify_net::is_local_network_bytes(raw))
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_net_is_allowed_host_header(
    host_header: *const c_char,
    expected_authority: *const c_char,
) -> i32 {
    ffi_bool(|| {
        let host = cstr_to_str(host_header);
        let expected = cstr_to_str(expected_authority)?;
        Some(hiddify_net::is_allowed_host_header(host, expected))
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_net_parse_phone_import_form_json(
    body_ptr: *const u8,
    body_len: usize,
    max_text_bytes: usize,
) -> *mut c_char {
    ffi_string(|| {
        let bytes = if body_ptr.is_null() || body_len == 0 {
            &[]
        } else {
            slice::from_raw_parts(body_ptr, body_len)
        };
        let res = hiddify_net::parse_phone_import_form(bytes, max_text_bytes);
        serde_json::to_string(&res).ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_config_parse_optional_range_json(
    input: *const c_char,
    allow_empty: bool,
) -> *mut c_char {
    ffi_string(|| {
        let range = OptionalRange::try_parse(cstr_to_str(input)?, allow_empty)?;
        serde_json::to_string(&range).ok()
    })
}

#[no_mangle]
pub extern "C" fn hiddify_config_format_optional_range(
    has_min: bool,
    min_val: i64,
    has_max: bool,
    max_val: i64,
) -> *mut c_char {
    ffi_string(|| {
        let range = OptionalRange {
            min: has_min.then_some(min_val),
            max: has_max.then_some(max_val),
        };
        Some(range.format())
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_config_extra_security_mode(value: *const c_char) -> *mut c_char {
    ffi_string(|| {
        let v = cstr_to_str(value)?;
        hiddify_config::extra_security_mode(Some(v)).map(str::to_string)
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_config_keep_top_header(value: *const c_char) -> i64 {
    match catch_unwind(AssertUnwindSafe(|| {
        let v = cstr_to_str(value)?;
        hiddify_config::keep_top_header(Some(v))
    })) {
        Ok(Some(n)) => n,
        _ => -9999,
    }
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_config_fragment_tricks_json(value: *const c_char) -> *mut c_char {
    ffi_string(|| {
        let v = cstr_to_str(value)?;
        let tricks = hiddify_config::fragment_tricks(Some(v))?;
        serde_json::to_string(&tricks).ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_config_keep_own_dns_json(
    own_json: *const c_char,
    merged_json: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        let own: Map<String, Value> = serde_json::from_str(cstr_to_str(own_json)?).ok()?;
        let merged: Map<String, Value> = serde_json::from_str(cstr_to_str(merged_json)?).ok()?;
        let guarded = hiddify_config::keep_own_dns_behind_extra_security(&own, &merged);
        serde_json::to_string(&guarded).ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_config_merge_options_with_override_json(
    own_json: *const c_char,
    profile_override_str: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        let own: Map<String, Value> = serde_json::from_str(cstr_to_str(own_json)?).ok()?;
        let override_opt = cstr_to_str(profile_override_str);
        let merged = hiddify_config::merge_options_with_profile_override(&own, override_opt).ok()?;
        serde_json::to_string(&merged).ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_config_builtin_rule_pb_json(
    short_name: *const c_char,
    platform: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        hiddify_config::route_rules::builtin_rule_pb_json(
            cstr_to_str(short_name)?,
            cstr_to_str(platform)?,
        )
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_config_builtin_of_rule_name(
    rule_name: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        hiddify_config::route_rules::builtin_of_rule_name(cstr_to_str(rule_name)?)
            .map(str::to_string)
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_config_with_builtins_pb_json(
    saved_pb_json: *const c_char,
    region: *const c_char,
    platform: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        hiddify_config::route_rules::with_builtins_pb_json(
            cstr_to_str(saved_pb_json)?,
            cstr_to_str(region)?,
            cstr_to_str(platform)?,
        )
        .ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_config_duplicate_rule_pb_json(
    current_pb_json: *const c_char,
    list_order: i64,
) -> *mut c_char {
    ffi_string(|| {
        let (rules_json, new_order) = hiddify_config::route_rules::duplicate_rule_pb_json(
            cstr_to_str(current_pb_json)?,
            list_order,
        )
        .ok()?;
        Some(json!({"pb_json": rules_json, "order": new_order}).to_string())
    })
}

#[no_mangle]
pub extern "C" fn hiddify_profile_download_problem_of_status(code: i32) -> i32 {
    DownloadProblemKind::of_status(code).index()
}

#[no_mangle]
pub extern "C" fn hiddify_profile_download_problem_of_core(
    fetch_error: i32,
    status_code: i32,
) -> i32 {
    match DownloadProblemKind::of_core(fetch_error, status_code) {
        Some(p) => p.index(),
        None => -1,
    }
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_profile_download_problem_classify_dio(
    status_code: i32,
    dio_type: *const c_char,
    is_cert_exc: bool,
    is_tls_exc: bool,
    text: *const c_char,
) -> i32 {
    match catch_unwind(AssertUnwindSafe(|| {
        let dt = cstr_to_str(dio_type).unwrap_or("");
        let t = cstr_to_str(text).unwrap_or("");
        let sc = (status_code >= 0).then_some(status_code);
        DownloadProblemKind::classify_dio(sc, dt, is_cert_exc, is_tls_exc, t).index()
    })) {
        Ok(idx) => idx,
        Err(_) => DownloadProblemKind::Unknown.index(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_profile_download_problem_most_telling(
    indices_ptr: *const i32,
    len: usize,
) -> i32 {
    match catch_unwind(AssertUnwindSafe(|| {
        if indices_ptr.is_null() || len == 0 {
            return DownloadProblemKind::Unknown.index();
        }
        let raw = slice::from_raw_parts(indices_ptr, len);
        let kinds: Vec<DownloadProblemKind> = raw
            .iter()
            .copied()
            .map(DownloadProblemKind::from_index)
            .collect();
        DownloadProblemKind::most_telling(&kinds).index()
    })) {
        Ok(idx) => idx,
        Err(_) => DownloadProblemKind::Unknown.index(),
    }
}

#[no_mangle]
pub extern "C" fn hiddify_profile_download_problem_meta_json(
    problem_idx: i32,
    status_code: i32,
) -> *mut c_char {
    ffi_string(|| {
        let kind = DownloadProblemKind::from_index(problem_idx);
        let sc = (status_code >= 0).then_some(status_code);
        Some(
            json!({
                "may_pass": kind.may_pass(),
                "code": kind.code_str(sc),
                "failed_stop": kind.failed_stop(),
                "side": kind.side_index(),
            })
            .to_string(),
        )
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_profile_flatten_headers_json(
    headers_json: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        let map: IndexMap<String, Vec<String>> =
            serde_json::from_str(cstr_to_str(headers_json)?).ok()?;
        let flat = hiddify_profile::download::flatten_http_headers(&map);
        serde_json::to_string(&flat).ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_profile_plan_link_expansion_json(
    raw_text: *const c_char,
    decode_base64_first: bool,
) -> *mut c_char {
    ffi_string(|| {
        let plan =
            hiddify_profile::download::plan_link_expansion(cstr_to_str(raw_text)?, decode_base64_first);
        serde_json::to_string(&plan).ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_profile_merge_expanded_links(
    decoded_text: *const c_char,
    replacements_json: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        let decoded = cstr_to_str(decoded_text)?;
        let reps: Vec<(usize, String)> =
            serde_json::from_str(cstr_to_str(replacements_json)?).ok()?;
        Some(hiddify_profile::download::merge_expanded_links(decoded, &reps))
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_profile_resolve_profile_name(
    override_name: *const c_char,
    profile_title_header: *const c_char,
    content_disposition_header: *const c_char,
    remote_url: *const c_char,
    existing_local_name: *const c_char,
    temp_file_path: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        hiddify_profile::resolve_profile_name_from_path(
            cstr_to_str(override_name),
            cstr_to_str(profile_title_header),
            cstr_to_str(content_disposition_header),
            cstr_to_str(remote_url),
            cstr_to_str(existing_local_name),
            cstr_to_str(temp_file_path)?,
        )
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_profile_parse_update_interval_hours(
    header_value: *const c_char,
    has_override: bool,
    override_hours: i64,
    is_auto_update_disable: bool,
) -> i64 {
    match catch_unwind(AssertUnwindSafe(|| {
        hiddify_profile::parse_update_interval_hours(
            cstr_to_str(header_value),
            has_override.then_some(override_hours),
            is_auto_update_disable,
        )
    })) {
        Ok(Some(hours)) => hours,
        _ => -1,
    }
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_open_log_file(path: *const c_char) -> i32 {
    ffi_bool(|| {
        Some(hiddify_diagnostics::log_crash_store::open_log_file(
            cstr_to_str(path)?,
        ))
    })
}

#[no_mangle]
pub extern "C" fn hiddify_diagnostics_is_log_file_open() -> i32 {
    ffi_bool(|| Some(hiddify_diagnostics::log_crash_store::is_log_file_open()))
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_write_log_line(line: *const c_char) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Some(s) = cstr_to_str(line) {
            hiddify_diagnostics::log_crash_store::write_log_line(s);
        }
    }));
}

#[no_mangle]
pub extern "C" fn hiddify_diagnostics_close_log_file() {
    let _ = catch_unwind(|| {
        hiddify_diagnostics::log_crash_store::close_log_file();
    });
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_write_text_to_file(
    path: *const c_char,
    content: *const c_char,
) -> i32 {
    ffi_bool(|| {
        Some(hiddify_diagnostics::log_crash_store::write_text_to_file(
            cstr_to_str(path)?,
            cstr_to_str(content)?,
        ))
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_origin_of_stack(
    stack: *const c_char,
    own_prefix: *const c_char,
    logger_internals: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        hiddify_diagnostics::log_crash_store::origin_of_stack(
            cstr_to_str(stack)?,
            cstr_to_str(own_prefix)?,
            cstr_to_str(logger_internals)?,
        )
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_own_stack_frames(
    stack: *const c_char,
    own_prefix: *const c_char,
    logger_internals: *const c_char,
    max_frames: usize,
) -> *mut c_char {
    ffi_string(|| {
        hiddify_diagnostics::log_crash_store::own_stack_frames(
            cstr_to_str(stack)?,
            cstr_to_str(own_prefix)?,
            cstr_to_str(logger_internals)?,
            max_frames,
        )
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_open_app_crash_store(
    working_dir: *const c_char,
) -> i32 {
    ffi_bool(|| {
        Some(hiddify_diagnostics::log_crash_store::open_app_crash_store(
            cstr_to_str(working_dir)?,
        ))
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_record_app_crash(
    kind: *const c_char,
    text: *const c_char,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let (Some(k), Some(t)) = (cstr_to_str(kind), cstr_to_str(text)) {
            hiddify_diagnostics::log_crash_store::record_app_crash(k, t);
        }
    }));
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_list_app_crash_files_json(
    working_dir: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        let files = hiddify_diagnostics::log_crash_store::list_app_crash_files(cstr_to_str(
            working_dir,
        )?);
        serde_json::to_string(&files).ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_list_fatal_app_crash_reports_json(
    working_dir: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        let reports = hiddify_diagnostics::log_crash_store::list_fatal_app_crash_reports(
            cstr_to_str(working_dir)?,
        );
        serde_json::to_string(&reports).ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_scrub_preferences_json(
    prefs_json: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        let prefs: Map<String, Value> = serde_json::from_str(cstr_to_str(prefs_json)?).ok()?;
        let scrubbed = hiddify_diagnostics::log_crash_store::scrub_preferences_map(&prefs);
        serde_json::to_string(&scrubbed).ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_assemble_payload_json(
    description: *const c_char,
    app_info_json: *const c_char,
    ring_logs_text: *const c_char,
    app_log_file_path: *const c_char,
    working_dir: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        let payload = hiddify_diagnostics::log_crash_store::assemble_diagnostics_payload(
            cstr_to_str(description),
            cstr_to_str(app_info_json),
            cstr_to_str(ring_logs_text)?,
            cstr_to_str(app_log_file_path),
            cstr_to_str(working_dir)?,
        );
        serde_json::to_string(&payload).ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_parse_core_stdout_line_json(
    line: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        let parsed = hiddify_diagnostics::core_guard::parse_core_stdout_line(cstr_to_str(line)?);
        serde_json::to_string(&parsed).ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_core_crash_headline(
    output: *const c_char,
) -> *mut c_char {
    ffi_string(|| hiddify_diagnostics::core_guard::core_crash_headline(cstr_to_str(output)?))
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_core_crash_reason_of(
    crash_file_path: *const c_char,
    stderr_tail: *const c_char,
    started_at_ms: i64,
) -> *mut c_char {
    ffi_string(|| {
        hiddify_diagnostics::core_guard::core_crash_reason_of(
            cstr_to_str(crash_file_path)?,
            cstr_to_str(stderr_tail)?,
            started_at_ms,
        )
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_windows_app_error_for(
    xml: *const c_char,
    pid: i64,
) -> *mut c_char {
    ffi_string(|| {
        hiddify_diagnostics::core_guard::windows_application_error_for(cstr_to_str(xml)?, pid)
    })
}

#[no_mangle]
pub extern "C" fn hiddify_diagnostics_describe_windows_exit_code(exit_code: i64) -> *mut c_char {
    ffi_string(|| hiddify_diagnostics::core_guard::describe_windows_exit_code(exit_code))
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_is_core_gone_grpc_error(
    grpc_code: i32,
    message: *const c_char,
) -> i32 {
    ffi_bool(|| {
        Some(hiddify_diagnostics::core_guard::is_core_gone_grpc_error(
            grpc_code,
            cstr_to_str(message),
        ))
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_classify_core_start_failure(
    message: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        Some(
            hiddify_diagnostics::core_guard::classify_core_start_failure(cstr_to_str(message)?)
                .to_string(),
        )
    })
}

// ============================================================================
// Phase 3: Core Diagnostics Export, Config/Log Sanitizer & VLESS X25519 FFI
// ============================================================================

/// Generates a clamped X25519 VLESS encryption/decryption pair (`{"decryption": "...", "encryption": "..."}`).
#[no_mangle]
pub extern "C" fn hiddify_crypto_generate_vless_encryption_json() -> *mut c_char {
    ffi_string(|| {
        let pair = hiddify_crypto::generate_vless_encryption_pair();
        serde_json::to_string(&pair).ok()
    })
}

/// Recursively redacts sensitive keys in a Sing-box / Hiddify JSON config (`v2/config/sanitize.go`).
#[no_mangle]
pub unsafe extern "C" fn hiddify_config_sanitize_json(config_json: *const c_char) -> *mut c_char {
    ffi_string(|| hiddify_config::sanitize_config_json(cstr_to_str(config_json)?))
}

/// Scrubs proxy links, URL tokens, UUIDs, emails, inline secrets, IPv4/IPv6, MACs, and domains from text.
#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_sanitize_report_text(
    text: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        let raw = cstr_to_str(text)?;
        let out = hiddify_diagnostics::report_export::sanitize_report_text(raw.as_bytes());
        String::from_utf8(out).ok()
    })
}

/// Scans `crash_reports`, `oom_reports`, and live `stderr` files in `working_dir` (`listCrashReports`).
#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_list_core_crash_reports_json(
    working_dir: *const c_char,
) -> *mut c_char {
    ffi_string(|| {
        let dir = cstr_to_str(working_dir)?;
        let reports = hiddify_diagnostics::report_export::list_core_crash_reports(Path::new(dir));
        serde_json::to_string(&reports).ok()
    })
}

/// Assembles, scrubs, zips, and optionally X25519 `age`-encrypts a full diagnostics report.
/// Returns JSON `{"ok": true, "path": "...", "encrypted": bool}` or `{"ok": false, "error": "..."}`.
#[no_mangle]
pub unsafe extern "C" fn hiddify_diagnostics_export_report_json(
    request_json: *const c_char,
) -> *mut c_char {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    use hiddify_diagnostics::report_export::{export_diagnostics_report, ExportDiagnosticsOptions};
    use std::collections::BTreeMap;

    ffi_string(|| {
        let req_str = cstr_to_str(request_json)?;
        let req: Value = match serde_json::from_str(req_str) {
            Ok(v) => v,
            Err(e) => {
                return Some(json!({ "ok": false, "error": e.to_string() }).to_string());
            }
        };
        let working_dir = req.get("workingDir").and_then(Value::as_str).unwrap_or("");
        let temp_dir = req.get("tempDir").and_then(Value::as_str).unwrap_or("");
        if working_dir.is_empty() || temp_dir.is_empty() {
            return Some(
                json!({ "ok": false, "error": "workingDir and tempDir are required" }).to_string(),
            );
        }

        let decode_map = |field: &str| -> BTreeMap<String, Vec<u8>> {
            let mut out = BTreeMap::new();
            if let Some(obj) = req.get(field).and_then(Value::as_object) {
                for (k, v) in obj {
                    if let Some(b64) = v.as_str() {
                        if let Ok(bytes) = STANDARD.decode(b64.as_bytes()) {
                            out.insert(k.clone(), bytes);
                        }
                    }
                }
            }
            out
        };

        let dest_str = req
            .get("destinationPath")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty());
        let opts = ExportDiagnosticsOptions {
            working_dir: Path::new(working_dir),
            temp_dir: Path::new(temp_dir),
            destination_path: dest_str.map(Path::new),
            unencrypted: req
                .get("unencrypted")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            inline: req.get("inline").and_then(Value::as_bool).unwrap_or(false),
            public_key_override: req
                .get("publicKey")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty()),
            extra_files: decode_map("extraFilesB64"),
            generic_files: decode_map("genericFilesB64"),
            mode: req.get("mode").and_then(Value::as_str),
            core_version: req.get("coreVersion").and_then(Value::as_str),
            core_commit: req.get("coreCommit").and_then(Value::as_str),
            core_state: req.get("coreState").and_then(Value::as_str),
        };

        match export_diagnostics_report(&opts) {
            Ok(outcome) => Some(
                json!({
                    "ok": true,
                    "path": outcome.path,
                    "encrypted": outcome.encrypted,
                })
                .to_string(),
            ),
            Err(err) => Some(
                json!({
                    "ok": false,
                    "error": err,
                })
                .to_string(),
            ),
        }
    })
}

// ============================================================================
// Phase 4: FetchUrl, Desktop Admin Service Elevation & User System Proxy FFI
// ============================================================================

/// Downloads a URL using Chrome-like TLS + system & Mozilla roots (`v2/hcore/fetch_url.go`).
#[no_mangle]
pub unsafe extern "C" fn hiddify_net_fetch_url_json(
    request_json: *const c_char,
) -> *mut c_char {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    use hiddify_net::fetch_url::{fetch_url, FetchUrlRequest};

    ffi_string(|| {
        let req_str = cstr_to_str(request_json)?;
        let req: Value = serde_json::from_str(req_str).ok()?;
        let url = req.get("url").and_then(Value::as_str).unwrap_or("").to_string();
        let user_agent = req
            .get("userAgent")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let timeout_ms = req
            .get("timeoutMs")
            .and_then(Value::as_i64)
            .unwrap_or(0) as i32;

        match fetch_url(&FetchUrlRequest {
            url,
            user_agent,
            timeout_ms,
        }) {
            Ok(outcome) => Some(
                json!({
                    "ok": true,
                    "statusCode": outcome.status_code,
                    "headers": outcome.headers,
                    "bodyB64": STANDARD.encode(&outcome.body),
                    "error": outcome.error.as_i32(),
                    "errorDetail": outcome.error_detail,
                })
                .to_string(),
            ),
            Err(err) => Some(
                json!({
                    "ok": false,
                    "invalidArgument": true,
                    "errorDetail": err,
                })
                .to_string(),
            ),
        }
    })
}

/// Reports whether the current process runs with administrator (Windows) or root (Unix) privileges.
#[no_mangle]
pub extern "C" fn hiddify_net_is_admin() -> i32 {
    ffi_bool(|| Some(hiddify_net::admin_service::is_admin()))
}

/// Reports whether the current process is allowed to open a TUN interface.
#[no_mangle]
pub extern "C" fn hiddify_net_tun_allowed() -> i32 {
    ffi_bool(|| Some(hiddify_net::admin_service::tun_allowed()))
}

/// Evaluates whether the elevated `admin_bgservice` is needed (`v2/hcore/admin_bgservice.go:NeedAdminBgService`).
#[no_mangle]
pub unsafe extern "C" fn hiddify_net_need_admin_bg_service_json(
    request_json: *const c_char,
) -> *mut c_char {
    use hiddify_net::admin_service::{need_admin_bg_service, NeedAdminBgServiceRequest};

    ffi_string(|| {
        let req_str = cstr_to_str(request_json)?;
        let req: Value = serde_json::from_str(req_str).ok()?;
        let outcome = need_admin_bg_service(&NeedAdminBgServiceRequest {
            service_mode: req
                .get("serviceMode")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            hiddify_settings_json: req
                .get("hiddifySettingsJson")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            admin_preference: req
                .get("adminPreference")
                .and_then(Value::as_i64)
                .unwrap_or(0) as i32,
            privilege_error_remembered: req
                .get("privilegeErrorRemembered")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            cli_path: req
                .get("cliPath")
                .and_then(Value::as_str)
                .map(str::to_string),
            build_id: req
                .get("buildId")
                .and_then(Value::as_str)
                .map(str::to_string),
        });
        Some(
            json!({
                "decision": outcome.decision,
                "adminNeededProxies": outcome.admin_needed_proxies,
            })
            .to_string(),
        )
    })
}

/// Installs/starts `admin_bgservice` with UAC / PolicyKit / sudo elevation and connects it (`EnsureAdminBgService`).
#[no_mangle]
pub unsafe extern "C" fn hiddify_net_ensure_admin_bg_service_json(
    request_json: *const c_char,
) -> *mut c_char {
    use hiddify_net::admin_service::{ensure_admin_bg_service, EnsureAdminBgServiceRequest};
    use std::collections::BTreeMap;

    ffi_string(|| {
        let req_str = cstr_to_str(request_json)?;
        let req: Value = serde_json::from_str(req_str).ok()?;
        let mut resources_b64 = BTreeMap::new();
        if let Some(obj) = req.get("resourcesB64").and_then(Value::as_object) {
            for (k, v) in obj {
                if let Some(b64) = v.as_str() {
                    resources_b64.insert(k.clone(), b64.to_string());
                }
            }
        }
        let events = ensure_admin_bg_service(&EnsureAdminBgServiceRequest {
            config_content: req
                .get("configContent")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            config_name: req
                .get("configName")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            settings_json: req
                .get("settingsJson")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            resources_b64,
            disable_memory_limit: req
                .get("disableMemoryLimit")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            owner_pid: req
                .get("ownerPid")
                .and_then(Value::as_i64)
                .unwrap_or(0) as i32,
            cli_path: req
                .get("cliPath")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            owner_user: req
                .get("ownerUser")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            proxy_only: req
                .get("proxyOnly")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            build_id: req
                .get("buildId")
                .and_then(Value::as_str)
                .map(str::to_string),
        });
        serde_json::to_string(&events).ok()
    })
}

/// Enables or disables the user-session system proxy and cleans up `data/system-proxy.on` leftovers.
#[no_mangle]
pub unsafe extern "C" fn hiddify_net_set_system_proxy_enabled_json(
    request_json: *const c_char,
) -> *mut c_char {
    use hiddify_net::system_proxy::set_system_proxy_enabled;

    ffi_string(|| {
        let req_str = cstr_to_str(request_json)?;
        let req: Value = serde_json::from_str(req_str).ok()?;
        let working_dir = req
            .get("workingDir")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(Path::new);
        let is_enabled = req
            .get("isEnabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let address = req
            .get("address")
            .and_then(Value::as_str)
            .unwrap_or("");
        let support_socks = req
            .get("supportSocks")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        match set_system_proxy_enabled(working_dir, is_enabled, address, support_socks) {
            Ok(()) => Some(json!({ "ok": true, "message": "" }).to_string()),
            Err(msg) => Some(json!({ "ok": false, "message": msg }).to_string()),
        }
    })
}

/// Returns `{"available": bool, "enabled": bool}` (`GetSystemProxyStatus`).
#[no_mangle]
pub extern "C" fn hiddify_net_get_system_proxy_status_json() -> *mut c_char {
    ffi_string(|| {
        let st = hiddify_net::system_proxy::get_system_proxy_status();
        serde_json::to_string(&st).ok()
    })
}

// ============================================================================
// Phase 5 (C-ABI v5): Task Cancellation, Streaming Update Download, LAN Servers,
// Health Probe, and Native Subscription Fetch/Expansion Pipeline
// ============================================================================

#[no_mangle]
pub extern "C" fn hiddify_net_create_task() -> u64 {
    catch_unwind(hiddify_net::cancel::create_task).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn hiddify_net_cancel_task(task_id: u64) -> i32 {
    ffi_bool(|| Some(hiddify_net::cancel::cancel_task(task_id)))
}

#[no_mangle]
pub extern "C" fn hiddify_net_remove_task(task_id: u64) {
    let _ = catch_unwind(|| hiddify_net::cancel::remove_task(task_id));
}

#[no_mangle]
pub extern "C" fn hiddify_net_task_progress_json(task_id: u64) -> *mut c_char {
    ffi_string(|| {
        let snap = hiddify_net::cancel::task_progress(task_id)?;
        serde_json::to_string(&snap).ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_net_download_update_verified_json(
    request_json: *const c_char,
) -> *mut c_char {
    use hiddify_net::update_download::{download_and_verify_update, UpdateDownloadRequest};
    ffi_string(|| {
        let req_str = cstr_to_str(request_json)?;
        let req: UpdateDownloadRequest = serde_json::from_str(req_str).ok()?;
        let res = download_and_verify_update(&req);
        serde_json::to_string(&res).ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_net_health_check_json(
    request_json: *const c_char,
) -> *mut c_char {
    use hiddify_net::health_check::{run_health_checks, HealthCheckOptions};
    ffi_string(|| {
        let req_str = cstr_to_str(request_json)?;
        let opts: HealthCheckOptions = serde_json::from_str(req_str).ok()?;
        let report = run_health_checks(&opts);
        serde_json::to_string(&report).ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_net_phone_import_server_start_json(
    bind_ip: *const c_char,
    page_text_json: *const c_char,
) -> *mut c_char {
    use hiddify_net::lan_server::{phone_import_server_start, PhoneImportPageText};
    ffi_string(|| {
        let ip = cstr_to_str(bind_ip)?;
        let text_str = cstr_to_str(page_text_json)?;
        let text: PhoneImportPageText = serde_json::from_str(text_str).ok()?;
        match phone_import_server_start(ip, text) {
            Ok(info) => Some(json!({ "ok": true, "info": info }).to_string()),
            Err(err) => Some(json!({ "ok": false, "error": err }).to_string()),
        }
    })
}

#[no_mangle]
pub extern "C" fn hiddify_net_phone_import_server_poll_json(handle: u64) -> *mut c_char {
    ffi_string(|| {
        let snap = hiddify_net::lan_server::phone_import_server_poll(handle)?;
        serde_json::to_string(&snap).ok()
    })
}

#[no_mangle]
pub extern "C" fn hiddify_net_phone_import_server_is_locked(handle: u64) -> i32 {
    ffi_bool(|| Some(hiddify_net::lan_server::phone_import_server_is_locked(handle)))
}

#[no_mangle]
pub extern "C" fn hiddify_net_phone_import_server_handled(handle: u64) -> i32 {
    ffi_bool(|| Some(hiddify_net::lan_server::phone_import_server_handled(handle)))
}

#[no_mangle]
pub extern "C" fn hiddify_net_phone_import_server_close(handle: u64) -> i32 {
    ffi_bool(|| Some(hiddify_net::lan_server::phone_import_server_close(handle)))
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_net_lan_file_server_start_json(
    bind_ip: *const c_char,
    file_path: *const c_char,
) -> *mut c_char {
    use hiddify_net::lan_server::lan_file_server_start;
    ffi_string(|| {
        let ip = cstr_to_str(bind_ip)?;
        let path = cstr_to_str(file_path)?;
        match lan_file_server_start(ip, path) {
            Ok(info) => Some(json!({ "ok": true, "info": info }).to_string()),
            Err(err) => Some(json!({ "ok": false, "error": err }).to_string()),
        }
    })
}

#[no_mangle]
pub extern "C" fn hiddify_net_lan_file_server_poll_json(handle: u64) -> *mut c_char {
    ffi_string(|| {
        let list = hiddify_net::lan_server::lan_file_server_poll_downloads(handle)?;
        serde_json::to_string(&list).ok()
    })
}

#[no_mangle]
pub extern "C" fn hiddify_net_lan_file_server_close(handle: u64) -> i32 {
    ffi_bool(|| Some(hiddify_net::lan_server::lan_file_server_close(handle)))
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_profile_fetch_and_prepare_remote_attempt_json(
    request_json: *const c_char,
) -> *mut c_char {
    use hiddify_profile::pipeline::{
        fetch_and_prepare_remote_attempt, RemoteProfileAttemptRequest,
    };
    ffi_string(|| {
        let req_str = cstr_to_str(request_json)?;
        let req: RemoteProfileAttemptRequest = serde_json::from_str(req_str).ok()?;
        let res = fetch_and_prepare_remote_attempt(&req);
        serde_json::to_string(&res).ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_profile_finalize_downloaded_remote_json(
    request_json: *const c_char,
) -> *mut c_char {
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use base64::Engine;
    use hiddify_net::fetch_url::FetchHttpHeader;
    use hiddify_profile::pipeline::{finalize_downloaded_remote_bytes, ProfileMetadataInput};

    ffi_string(|| {
        let req_str = cstr_to_str(request_json)?;
        let req: Value = serde_json::from_str(req_str).ok()?;
        let body_b64 = req.get("bodyB64").and_then(Value::as_str).unwrap_or("");
        let raw_bytes = BASE64_STANDARD.decode(body_b64.as_bytes()).ok()?;
        let headers: Vec<FetchHttpHeader> = req
            .get("headers")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let metadata: ProfileMetadataInput = req
            .get("metadata")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let res = finalize_downloaded_remote_bytes(
            &raw_bytes,
            &headers,
            req.get("tempFilePath").and_then(Value::as_str).unwrap_or(""),
            req.get("userAgent").and_then(Value::as_str).unwrap_or(""),
            req.get("proxyPort").and_then(Value::as_u64).unwrap_or(0) as u16,
            req.get("proxySecret").and_then(Value::as_str).unwrap_or(""),
            req.get("timeoutMs").and_then(Value::as_i64).unwrap_or(15_000) as i32,
            req.get("cancelTaskId").and_then(Value::as_u64).unwrap_or(0),
            &metadata,
        );
        serde_json::to_string(&res).ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_profile_prepare_local_json(
    request_json: *const c_char,
) -> *mut c_char {
    use hiddify_profile::pipeline::{prepare_local_profile, LocalProfilePrepareRequest};
    ffi_string(|| {
        let req_str = cstr_to_str(request_json)?;
        let req: LocalProfilePrepareRequest = serde_json::from_str(req_str).ok()?;
        let res = prepare_local_profile(&req);
        serde_json::to_string(&res).ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn hiddify_profile_expand_links_native_json(
    request_json: *const c_char,
) -> *mut c_char {
    use hiddify_profile::pipeline::{expand_links_native, ExpandLinksRequest};
    ffi_string(|| {
        let req_str = cstr_to_str(request_json)?;
        let req: ExpandLinksRequest = serde_json::from_str(req_str).ok()?;
        match expand_links_native(&req) {
            Ok(text) => Some(json!({ "status": "ok", "text": text }).to_string()),
            Err(msg) if msg == "cancelled" => Some(json!({ "status": "cancelled" }).to_string()),
            Err(err) => Some(json!({ "status": "error", "error": err }).to_string()),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn ffi_roundtrip_and_memory_release() {
        assert_eq!(hiddify_rust_abi_version(), 5);

        let ip = CString::new("1.2.3.4").unwrap();
        let masked_ptr = unsafe { hiddify_net_obscure_ip(ip.as_ptr()) };
        assert!(!masked_ptr.is_null());
        let masked = unsafe { CStr::from_ptr(masked_ptr).to_str().unwrap().to_owned() };
        unsafe { hiddify_rust_free_string(masked_ptr) };
        assert_eq!(masked, "1.*.*.4");

        let domain = CString::new(".noon.com").unwrap();
        assert_eq!(unsafe { hiddify_net_is_domain_suffix(domain.as_ptr()) }, 1);

        let vless_ptr = hiddify_crypto_generate_vless_encryption_json();
        assert!(!vless_ptr.is_null());
        let vless_str = unsafe { CStr::from_ptr(vless_ptr).to_str().unwrap().to_owned() };
        unsafe { hiddify_rust_free_string(vless_ptr) };
        assert!(vless_str.contains("mlkem768x25519plus"));
    }
}
