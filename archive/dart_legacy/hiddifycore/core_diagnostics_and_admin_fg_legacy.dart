// Archived Legacy Dart / gRPC Foreground-Core Implementations (Phase 3 & Phase 4)
//
// Replaced by `libhiddify_rust` C-ABI FFI:
// - `hiddify_diagnostics_list_crash_reports` & `hiddify_diagnostics_export_report`
//   (`rust/crates/hiddify-diagnostics/src/report_export.rs`)
// - `hiddify_net_fetch_url`
//   (`rust/crates/hiddify-net/src/fetch_url.rs`)
// - `hiddify_net_is_admin`, `hiddify_net_need_admin_bg_service`, `hiddify_net_ensure_admin_bg_service`
//   (`rust/crates/hiddify-net/src/admin_service.rs`)
// - `hiddify_net_set_system_proxy_enabled`, `hiddify_net_get_system_proxy_status`
//   (`rust/crates/hiddify-net/src/system_proxy.rs`)

import 'dart:io';

import 'package:fpdart/fpdart.dart';
import 'package:grpc/grpc.dart' hide Response;
import 'package:hiddify/features/crash_report/data/app_diagnostics.dart';
import 'package:hiddify/hiddifycore/core_interface/core_interface_wrapper.dart';
import 'package:hiddify/hiddifycore/generated/v2/hcommon/common.pb.dart';
import 'package:hiddify/hiddifycore/generated/v2/hcore/hcore.pb.dart';

/// Legacy `fgClient.exportDiagnostics` fallback (formerly in `DiagnosticsExporter.export`).
Future<(String, bool)> exportDiagnosticsViaForegroundCoreLegacy({
  required CoreInterfaceWrapper core,
  required String destinationPath,
  required bool unencrypted,
  required Map<String, List<int>> extra,
  required Map<String, List<int>> generic,
}) async {
  final response = await core.fgClient
      .exportDiagnostics(
        ExportDiagnosticsRequest(
          destinationPath: destinationPath,
          unencrypted: unencrypted,
          extraFiles: extra,
          genericFiles: generic,
        ),
        options: CallOptions(timeout: const Duration(seconds: 30)),
      )
      .timeout(const Duration(seconds: 32));
  return (response.path, response.encrypted);
}

/// Legacy `fgClient.listCrashReports` fallback (formerly in `CrashReportNotifier._check`).
Future<List<CrashReport>> listCrashReportsViaForegroundCoreLegacy(CoreInterfaceWrapper core) async {
  final response = await core.fgClient
      .listCrashReports(
        Empty(),
        options: CallOptions(timeout: const Duration(seconds: 3)),
      )
      .timeout(const Duration(seconds: 4));
  return response.reports;
}

/// Legacy `fgClient.fetchUrl` synchronous TaskEither wrapper (formerly in `HiddifyCoreService.fetchUrl`).
TaskEither<String, FetchUrlResponse> fetchUrlViaForegroundCoreLegacy(
  CoreInterfaceWrapper core,
  String url, {
  String userAgent = "",
}) {
  return TaskEither(
    () => core.fgClient
        .fetchUrl(
          FetchUrlRequest(url: url, userAgent: userAgent),
          options: CallOptions(timeout: const Duration(seconds: 25)),
        )
        .then((res) => right<String, FetchUrlResponse>(res))
        .onError((error, _) => left(error.toString())),
  );
}

/// Legacy `fgClient` desktop TUN privilege / elevation / system-proxy calls
/// (formerly in `CoreInterfaceDesktopTun`).
class DesktopTunForegroundCoreLegacy {
  static Future<Response> setSystemProxyEnabled(
    CoreInterfaceWrapper core, {
    required bool enabled,
    String address = "",
    bool supportSocks = false,
  }) async {
    return await core.fgClient.setSystemProxyEnabled(
      SetSystemProxyEnabledRequest(
        isEnabled: enabled,
        address: address,
        supportSocks: supportSocks,
      ),
      options: CallOptions(timeout: const Duration(seconds: 5)),
    );
  }

  static Future<bool> isAdmin(CoreInterfaceWrapper core) async {
    final res = await core.fgClient.isAdmin(
      Empty(),
      options: CallOptions(timeout: const Duration(seconds: 2)),
    );
    return res.isAdmin;
  }

  static Future<bool> needAdminBgService(CoreInterfaceWrapper core) async {
    final res = await core.fgClient.needAdminBgService(
      Empty(),
      options: CallOptions(timeout: const Duration(seconds: 2)),
    );
    return res.needed;
  }

  static Future<EnsureAdminBgResponse> ensureAdminBgService(CoreInterfaceWrapper core) async {
    return await core.fgClient.ensureAdminBgService(
      Empty(),
      options: CallOptions(timeout: const Duration(minutes: 2)),
    );
  }
}
