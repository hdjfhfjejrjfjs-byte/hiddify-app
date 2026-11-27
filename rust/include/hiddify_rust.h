#ifndef HIDDIFY_RUST_H
#define HIDDIFY_RUST_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

uint32_t hiddify_rust_abi_version(void);
void hiddify_rust_free_string(char *ptr);

/* hiddify-crypto */
char *hiddify_crypto_generate_identity_json(void);
char *hiddify_crypto_load_or_create_identity_json(const char *directory_path);
int32_t hiddify_crypto_is_identity_usable(
    const char *certificate_pem,
    const char *private_key_pem,
    uint64_t renew_before_secs
);
int32_t hiddify_crypto_verify_pinned_cert(
    const uint8_t *presented_der_ptr,
    size_t presented_der_len,
    const char *presented_pem,
    const char *expected_server_pem,
    const char *host
);
char *hiddify_crypto_generate_proxy_secret(size_t length);
int32_t hiddify_crypto_is_valid_sha256_digest(const char *digest);
int32_t hiddify_crypto_verify_file_sha256(
    const char *file_path,
    uint64_t expected_size,
    const char *expected_digest
);

/* hiddify-net */
int32_t hiddify_net_is_url(const char *input);
int32_t hiddify_net_is_port(const char *input);
int32_t hiddify_net_is_port_or_port_range(const char *input);
int32_t hiddify_net_is_process_name(const char *input);
int32_t hiddify_net_is_process_path(const char *input);
int32_t hiddify_net_is_ip_cidr(const char *input);
int32_t hiddify_net_is_domain(const char *input);
int32_t hiddify_net_is_domain_suffix(const char *input);
char *hiddify_net_obscure_ip(const char *ip);
char *hiddify_net_sanitize_tag(const char *tag);
char *hiddify_net_strip_status_mark(const char *proxy_type);
int32_t hiddify_net_is_invalid_proxy_type(const char *type_without_status);
char *hiddify_net_split_scanner_endpoint_json(const char *proxy_type);

/* hiddify-config */
char *hiddify_config_profile_proxies_json(const char *config_json);
int64_t hiddify_config_count_proxies(const char *config_json);
char *hiddify_config_country_of(const char *tag);
char *hiddify_config_flag_of(const char *code);
char *hiddify_config_protocol_name(const char *proxy_type);
char *hiddify_config_editable_config_json(const char *content);
char *hiddify_config_single_proxy_config(
    const char *tag,
    const char *host,
    int64_t port,
    const char *raw_config,
    const char *generated_config
);
char *hiddify_config_profile_override(
    const char *populated_headers_json,
    const char *user_override_json
);
char *hiddify_config_apply_profile_override(
    const char *main_config_json,
    const char *profile_override_str
);

/* hiddify-profile */
char *hiddify_profile_safe_decode_base64(const char *input);
char *hiddify_profile_decode_base64_title(const char *value);
char *hiddify_profile_filename_from_content_disposition(const char *header);
char *hiddify_profile_populate_headers_json(
    const char *content,
    const char *remote_headers_json
);
char *hiddify_profile_parse_subscription_info_json(const char *sub_info_str);
char *hiddify_profile_detect_protocol(const char *content);
int32_t hiddify_profile_is_subscription_link(const char *line);
char *hiddify_profile_parse_link_json(const char *link);
char *hiddify_profile_parse_simple_link_json(const char *link);
char *hiddify_profile_parse_deep_link_json(const char *link);
char *hiddify_profile_generate_sub_share_link(const char *url_str, const char *name);
char *hiddify_profile_decode_text_bytes(const uint8_t *bytes_ptr, size_t bytes_len);
char *hiddify_profile_read_zip_text(const uint8_t *bytes_ptr, size_t bytes_len);

/* hiddify-diagnostics */
char *hiddify_diagnostics_telegram_payload(
    const char *os,
    const char *ov,
    const char *v,
    const char *t,
    const char *pm,
    const char *l,
    const char *a
);
char *hiddify_diagnostics_windows_version(const char *os_version);
char *hiddify_diagnostics_linux_distro_json(const char *os_release);
char *hiddify_diagnostics_crash_excerpt_json(const char *content, bool find_go_marker);
char *hiddify_diagnostics_extract_changelog(const char *body);
char *hiddify_diagnostics_parse_release_tag_json(const char *tag_name);

/* Phase 2 extensions */
char *hiddify_crypto_generate_numeric_pin(size_t length);
char *hiddify_crypto_generate_url_token(size_t byte_len);
char *hiddify_crypto_generate_alphanumeric_secret(size_t length);
int32_t hiddify_crypto_verify_secret_path(const char *path, const char *token);

int32_t hiddify_net_is_local_network_bytes(const uint8_t *raw_ip_ptr, size_t raw_ip_len);
int32_t hiddify_net_is_allowed_host_header(
    const char *host_header,
    const char *expected_authority
);
char *hiddify_net_parse_phone_import_form_json(
    const uint8_t *body_ptr,
    size_t body_len,
    size_t max_text_bytes
);

char *hiddify_config_parse_optional_range_json(const char *input, bool allow_empty);
char *hiddify_config_format_optional_range(
    bool has_min,
    int64_t min_val,
    bool has_max,
    int64_t max_val
);
char *hiddify_config_extra_security_mode(const char *value);
int64_t hiddify_config_keep_top_header(const char *value);
char *hiddify_config_fragment_tricks_json(const char *value);
char *hiddify_config_keep_own_dns_json(const char *own_json, const char *merged_json);
char *hiddify_config_merge_options_with_override_json(
    const char *own_json,
    const char *profile_override_str
);
char *hiddify_config_builtin_rule_pb_json(const char *short_name, const char *platform);
char *hiddify_config_builtin_of_rule_name(const char *rule_name);
char *hiddify_config_with_builtins_pb_json(
    const char *saved_pb_json,
    const char *region,
    const char *platform
);
char *hiddify_config_duplicate_rule_pb_json(const char *current_pb_json, int64_t list_order);

int32_t hiddify_profile_download_problem_of_status(int32_t code);
int32_t hiddify_profile_download_problem_of_core(int32_t fetch_error, int32_t status_code);
int32_t hiddify_profile_download_problem_classify_dio(
    int32_t status_code,
    const char *dio_type,
    bool is_cert_exc,
    bool is_tls_exc,
    const char *text
);
int32_t hiddify_profile_download_problem_most_telling(const int32_t *indices_ptr, size_t len);
char *hiddify_profile_download_problem_meta_json(int32_t problem_idx, int32_t status_code);
char *hiddify_profile_flatten_headers_json(const char *headers_json);
char *hiddify_profile_plan_link_expansion_json(const char *raw_text, bool decode_base64_first);
char *hiddify_profile_merge_expanded_links(
    const char *decoded_text,
    const char *replacements_json
);
char *hiddify_profile_resolve_profile_name(
    const char *override_name,
    const char *profile_title_header,
    const char *content_disposition_header,
    const char *remote_url,
    const char *existing_local_name,
    const char *temp_file_path
);
int64_t hiddify_profile_parse_update_interval_hours(
    const char *header_value,
    bool has_override,
    int64_t override_hours,
    bool is_auto_update_disable
);

int32_t hiddify_diagnostics_open_log_file(const char *path);
int32_t hiddify_diagnostics_is_log_file_open(void);
void hiddify_diagnostics_write_log_line(const char *line);
void hiddify_diagnostics_close_log_file(void);
int32_t hiddify_diagnostics_write_text_to_file(const char *path, const char *content);
char *hiddify_diagnostics_origin_of_stack(
    const char *stack,
    const char *own_prefix,
    const char *logger_internals
);
char *hiddify_diagnostics_own_stack_frames(
    const char *stack,
    const char *own_prefix,
    const char *logger_internals,
    size_t max_frames
);
int32_t hiddify_diagnostics_open_app_crash_store(const char *working_dir);
void hiddify_diagnostics_record_app_crash(const char *kind, const char *text);
char *hiddify_diagnostics_list_app_crash_files_json(const char *working_dir);
char *hiddify_diagnostics_list_fatal_app_crash_reports_json(const char *working_dir);
char *hiddify_diagnostics_scrub_preferences_json(const char *prefs_json);
char *hiddify_diagnostics_assemble_payload_json(
    const char *description,
    const char *app_info_json,
    const char *ring_logs_text,
    const char *app_log_file_path,
    const char *working_dir
);
char *hiddify_diagnostics_parse_core_stdout_line_json(const char *line);
char *hiddify_diagnostics_core_crash_headline(const char *output);
char *hiddify_diagnostics_core_crash_reason_of(
    const char *crash_file_path,
    const char *stderr_tail,
    int64_t started_at_ms
);
char *hiddify_diagnostics_windows_app_error_for(const char *xml, int64_t pid);
char *hiddify_diagnostics_describe_windows_exit_code(int64_t exit_code);
int32_t hiddify_diagnostics_is_core_gone_grpc_error(int32_t grpc_code, const char *message);
char *hiddify_diagnostics_classify_core_start_failure(const char *message);

/* Phase 3: Core Diagnostics Export, Config/Log Sanitizer & VLESS X25519 */
char *hiddify_crypto_generate_vless_encryption_json(void);
char *hiddify_config_sanitize_json(const char *config_json);
char *hiddify_diagnostics_sanitize_report_text(const char *text);
char *hiddify_diagnostics_list_core_crash_reports_json(const char *working_dir);
char *hiddify_diagnostics_export_report_json(const char *request_json);

#ifdef __cplusplus
}
#endif

#endif /* HIDDIFY_RUST_H */
