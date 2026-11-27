# Archived Legacy Dart Implementations

本目录归档了 `hiddify-app` **一期（Phase 1）与二期（Phase 2）**中已经完整迁移到 Rust 原生模块（`rust/`）并与之重叠的旧版纯 Dart 实现（共 **31 个归档单元**）。
这些文件不参与 Flutter/Dart 编译与静态分析（已在 `analysis_options.yaml` 中排除），仅保留作历史对照与审计参考。

> 所有线上/开发构建（iOS、Android、macOS、Linux）统一通过 `lib/core/rust/hiddify_rust.dart`（`dart:ffi`）直接调用 `libhiddify_rust` 动态库/Framework，不再维护双份重复实现。

---

## 一期归档清单（Phase 1 · 17 项）

| 归档文件 (`archive/dart_legacy/...`) | 原 `lib/` 路径 | 迁移后的 Rust Crate & 模块 |
|---|---|---|
| `hiddifycore/core_interface/grpc_client_identity.dart` | `lib/hiddifycore/core_interface/grpc_client_identity.dart` | `rust/crates/hiddify-crypto` (`identity`) |
| `hiddifycore/core_interface/mtls_channel_cred_pinning.dart` | `lib/hiddifycore/core_interface/mtls_channel_cred.dart` | `rust/crates/hiddify-crypto` (`pinning`) |
| `core/utils/proxy_secret.dart` | `lib/core/utils/proxy_secret.dart` | `rust/crates/hiddify-crypto` (`secret`) |
| `features/app_update/notifier/app_updater_sha256_legacy.dart` | `lib/features/app_update/notifier/app_updater.dart` | `rust/crates/hiddify-crypto` (`digest`) |
| `core/utils/ip_utils.dart` | `lib/core/utils/ip_utils.dart` | `rust/crates/hiddify-net` (`ip`) |
| `utils/validators.dart` | `lib/utils/validators.dart` | `rust/crates/hiddify-net` (`validators`) |
| `features/proxy/model/proxy_entity_parsing.dart` | `lib/features/proxy/model/proxy_entity.dart` | `rust/crates/hiddify-net` (`proxy_display`) |
| `features/proxy/data/single_proxy_config.dart` | `lib/features/proxy/data/single_proxy_config.dart` | `rust/crates/hiddify-config` (`single_proxy`) |
| `features/proxy/keep_top/keep_top_parsing.dart` | `lib/features/proxy/keep_top/keep_top.dart` | `rust/crates/hiddify-config` (`keep_top`) |
| `features/proxy/keep_top/keep_tags_helpers.dart` | `lib/features/proxy/keep_top/keep_tags_dialog.dart` | `rust/crates/hiddify-config` (`keep_top`) |
| `features/profile/details/editable_config.dart` | `lib/features/profile/details/profile_details_notifier.dart` | `rust/crates/hiddify-config` (`editable`) |
| `features/profile/data/profile_parser_legacy.dart` | `lib/features/profile/data/profile_parser.dart` | `rust/crates/hiddify-profile` + `rust/crates/hiddify-config` (`override_merge`) |
| `utils/link_parsers.dart` | `lib/utils/link_parsers.dart` | `rust/crates/hiddify-profile` (`links`, `base64`) |
| `utils/text_utils_io.dart` | `lib/utils/text_utils.dart` | `rust/crates/hiddify-profile` (`text_zip`) |
| `features/crash_report/data/telegram_report_link_legacy.dart` | `lib/features/crash_report/data/telegram_report_link.dart` | `rust/crates/hiddify-diagnostics` (`telegram`, `os_detect`) |
| `features/crash_report/data/crash_excerpt_legacy.dart` | `lib/features/crash_report/data/crash_excerpt.dart` | `rust/crates/hiddify-diagnostics` (`crash`) |
| `features/app_update/data/github_release_parser_legacy.dart` | `lib/features/app_update/data/github_release_parser.dart` | `rust/crates/hiddify-diagnostics` (`release`) |

---

## 二期归档清单（Phase 2 · 13 项）

| 归档文件 (`archive/dart_legacy/...`) | 原 `lib/` 路径 | 迁移后的 Rust Crate & 模块 |
|---|---|---|
| `core/logger/sinks/file_sink_io_legacy.dart` | `lib/core/logger/sinks/file_sink_io.dart` | `rust/crates/hiddify-diagnostics` (`log_crash_store`) |
| `core/logger/logger_setup_stack_legacy.dart` | `lib/core/logger/logger_setup.dart` | `rust/crates/hiddify-diagnostics` (`log_crash_store`) |
| `core/logger/crash/app_crash_store_io_legacy.dart` | `lib/core/logger/crash/app_crash_store_io.dart`<br/>`lib/features/crash_report/data/crash_excerpt.dart` | `rust/crates/hiddify-diagnostics` (`log_crash_store`) |
| `features/crash_report/data/diagnostics_scrub_and_assemble_legacy.dart` | `lib/features/crash_report/data/app_diagnostics.dart`<br/>`lib/features/crash_report/data/diagnostics_exporter.dart` | `rust/crates/hiddify-diagnostics` (`log_crash_store`) |
| `features/profile/model/download_problem_legacy.dart` | `lib/features/profile/model/download_problem.dart` | `rust/crates/hiddify-profile` (`download`) |
| `features/profile/data/profile_parser_expand_legacy.dart` | `lib/features/profile/data/profile_parser.dart` | `rust/crates/hiddify-profile` (`download`) + `hiddify-config` |
| `features/profile/data/profile_parser_name_interval_legacy.dart` | `lib/features/profile/data/profile_parser.dart`<br/>`lib/features/proxy/keep_top/keep_tags_dialog.dart` | `rust/crates/hiddify-profile` (`resolve_profile_name_from_path`, `parse_update_interval_hours`) + `hiddify-net` (`sanitize_tag`) |
| `features/profile/add/phone_import/lan_security_legacy.dart` | `lib/features/profile/add/phone_import/phone_import_server.dart`<br/>`lib/features/log/data/lan_file_server.dart` | `rust/crates/hiddify-net` + `rust/crates/hiddify-crypto` |
| `core/model/optional_range_legacy.dart` | `lib/core/model/optional_range.dart` | `rust/crates/hiddify-config` (`OptionalRange`) |
| `features/settings/data/config_option_override_legacy.dart` | `lib/features/settings/data/config_option_repository.dart` | `rust/crates/hiddify-config` (`keep_own_dns_behind_extra_security`) |
| `features/route_rules/notifier/builtin_rules_legacy.dart` | `lib/features/route_rules/notifier/rules_notifier.dart` | `rust/crates/hiddify-config` (`route_rules`) |
| `hiddifycore/core_interface/core_process_crash_and_handshake_legacy.dart` | `lib/hiddifycore/core_interface/core_process.dart` | `rust/crates/hiddify-diagnostics` (`core_guard`) |
| `hiddifycore/core_interface/windows_crash_legacy.dart` | `lib/hiddifycore/core_interface/windows_crash.dart` | `rust/crates/hiddify-diagnostics` (`core_guard`) |
| `hiddifycore/grpc_and_start_errors_legacy.dart` | `lib/hiddifycore/grpc_errors.dart`<br/>`lib/hiddifycore/hiddify_core_service.dart`<br/>`lib/hiddifycore/core_interface/core_interface_desktop.dart` | `rust/crates/hiddify-diagnostics` (`core_guard`) + `hiddify-crypto` |
