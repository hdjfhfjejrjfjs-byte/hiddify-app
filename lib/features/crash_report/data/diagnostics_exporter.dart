import 'dart:io';

import 'package:grpc/grpc.dart';
import 'package:hiddify/core/app_info/app_info_provider.dart';
import 'package:hiddify/core/directories/directories_provider.dart';
import 'package:hiddify/core/logger/logger_setup.dart';
import 'package:hiddify/core/rust/hiddify_rust.dart';
import 'package:hiddify/features/crash_report/data/app_diagnostics.dart';
import 'package:hiddify/features/log/data/log_data_providers.dart';
import 'package:hiddify/hiddifycore/generated/v2/hcore/hcore.pb.dart';
import 'package:hiddify/hiddifycore/hiddify_core_service_provider.dart';
import 'package:hiddify/utils/custom_loggers.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

final _log = appLogger('Diagnostics');

const _diagnosticsPublicKey = String.fromEnvironment('DIAGNOSTICS_PUBLIC_KEY');

final diagnosticsExporterProvider = Provider<DiagnosticsExporter>(DiagnosticsExporter.new);

/// A report exported to disk, and whether its sensitive part (config, logs, crashes) is encrypted
/// to the Hiddify team's age X25519 key.
typedef DiagnosticsReport = ({File file, bool encrypted});

/// Builds the zip the user sends after a crash or to report a problem: gathers crash, OOM and power
/// reports, stderr and core log files, the previous run's goroutine snapshots and the sanitized
/// config directly via `libhiddify_rust` (surviving even if the foreground Go core is down), while
/// still querying `bgClient.exportDiagnostics(inline: true)` when the privileged background core
/// runs in its own `0700` root / system-extension sandbox directory.
class DiagnosticsExporter {
  DiagnosticsExporter(this._ref);

  final Ref _ref;

  Future<DiagnosticsReport> export({String? description, bool unencrypted = false}) async {
    final coreService = _ref.read(hiddifyCoreServiceProvider);
    final core = coreService.core;

    Map<String, Object?>? appInfo;
    try {
      appInfo = await collectAppDiagnostics(_ref);
    } catch (e, stack) {
      _log.warning('collecting the app diagnostics failed: $e', e, stack);
    }

    final appLogFile = _ref.read(logPathResolverProvider).appFile();
    final dirs = _ref.read(appDirectoriesProvider).requireValue;
    final workingDir = dirs.workingDir.path;
    final tempDir = dirs.tempDir.path;
    final assembled = hiddifyRust.require(
      hiddifyRust.assembleDiagnosticsPayload(
        description: description,
        appInfo: appInfo,
        ringLogsText: logRing.records.map(formatRecord).join('\n'),
        appLogFilePath: appLogFile.existsSync() ? appLogFile.path : null,
        workingDir: workingDir,
      ),
      'assembleDiagnosticsPayload',
    );

    final extraFiles = Map<String, List<int>>.of(assembled.extraFiles);
    final genericFiles = Map<String, List<int>>.of(assembled.genericFiles);

    if (!core.isSingleChannel()) {
      // the background core may run in a root 0700 directory or system extension sandbox: ask inline
      final options = CallOptions(timeout: const Duration(seconds: 5));
      try {
        final response = await core.bgClient.exportDiagnostics(
          ExportDiagnosticsRequest(inline: true, unencrypted: true),
          options: options,
        );
        if (response.content.length <= assembled.remainingBgZipBudget) {
          extraFiles['background-core.zip'] = response.content;
        } else {
          _log.info('the background core zip is too large to include: ${response.content.length} bytes');
        }
      } catch (e) {
        _log.info('no diagnostics from the background core: $e');
      }
    }

    if (hiddifyRust.isAvailable) {
      String? version;
      try {
        final info = await _ref.read(appInfoProvider.future);
        version = info.version;
      } catch (_) {}

      final outcome = hiddifyRust.require(
        hiddifyRust.exportDiagnosticsReport(
          workingDir: workingDir,
          tempDir: tempDir,
          unencrypted: unencrypted,
          publicKey: _diagnosticsPublicKey.isEmpty ? null : _diagnosticsPublicKey,
          extraFiles: extraFiles,
          genericFiles: genericFiles,
          coreVersion: version,
          coreState: coreService.currentState.runtimeType.toString(),
        ),
        'exportDiagnosticsReport',
      );
      _log.info('diagnostics written via rust to ${outcome.path}, encrypted: ${outcome.encrypted}');
      return (file: File(outcome.path), encrypted: outcome.encrypted);
    }

    final response = await core.fgClient.exportDiagnostics(
      ExportDiagnosticsRequest(extraFiles: extraFiles, genericFiles: genericFiles, unencrypted: unencrypted),
    );
    _log.info('diagnostics written to ${response.path}, encrypted: ${response.encrypted}');
    return (file: File(response.path), encrypted: response.encrypted);
  }
}
