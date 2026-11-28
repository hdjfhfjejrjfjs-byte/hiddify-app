import 'dart:async';
import 'dart:io';

import 'package:hiddify/core/app_info/app_info_provider.dart';
import 'package:hiddify/core/directories/directories_provider.dart';
import 'package:hiddify/core/preferences/general_preferences.dart';
import 'package:hiddify/core/router/dialog/dialog_notifier.dart';
import 'package:hiddify/core/rust/hiddify_rust.dart';
import 'package:hiddify/features/crash_report/data/crash_excerpt.dart';
import 'package:hiddify/hiddifycore/generated/v2/hcore/hcore.pb.dart';
import 'package:hiddify/hiddifycore/hiddify_core_service_provider.dart';
import 'package:hiddify/utils/custom_loggers.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:path/path.dart' as p;

final _log = appLogger('CrashReport');

final crashReportNotifierProvider = NotifierProvider<CrashReportNotifier, CrashReport?>(CrashReportNotifier.new);

/// After a crash, the next run offers to send a report: scans archived crash/OOM reports and live
/// stderr files directly via `libhiddify_rust` alongside `app_crashes/`, showing the dialog once
/// for the newest crash without waiting on the Go foreground core to boot.
class CrashReportNotifier extends Notifier<CrashReport?> {
  @override
  CrashReport? build() {
    final crashes = ref.read(hiddifyCoreServiceProvider).core.coreCrashes.listen((_) => unawaited(_checkAfterCrash()));
    ref.onDispose(crashes.cancel);
    return null;
  }

  bool _checked = false;
  bool _offering = false;

  List<CrashReport> _readCoreCrashReportsViaRust(Directory workingDir) {
    final list = hiddifyRust.require(
      hiddifyRust.listCoreCrashReports(workingDir.path),
      'listCoreCrashReports',
    );
    return [
      for (final r in list)
        CrashReport(
          name: r.name,
          source: r.source,
          crashedAt: r.crashedAt,
          isCrash: r.isCrash,
        ),
    ];
  }

  Future<void> _checkAfterCrash() async {
    if (!ref.read(Preferences.introCompleted)) return;
    final workingDir = ref.read(appDirectoriesProvider).requireValue.workingDir;
    await _offer(_readCoreCrashReportsViaRust(workingDir));
  }

  /// Called once the first frame is up.
  Future<void> checkOnStart() async {
    if (_checked) return;
    _checked = true;
    if (!ref.read(Preferences.introCompleted)) return;
    final workingDir = ref.read(appDirectoriesProvider).requireValue.workingDir;
    await _dropStaleCrashes(workingDir);
    final app = appCrashReports(workingDir);
    final rustReports = _readCoreCrashReportsViaRust(workingDir);
    await _offer([...rustReports, ...app]);
  }

  Future<void> _dropStaleCrashes(Directory workingDir) async {
    final info = await ref.read(appInfoProvider.future);
    final version = '${info.version}+${info.buildNumber}';
    final upgraded = ref.read(Preferences.crashReportsVersion) != version;
    final now = DateTime.now().toUtc();
    for (final file in appCrashFiles(workingDir)) {
      final time = DateTime.tryParse(
        appCrashReports(workingDir).where((r) => r.name == p.basename(file.path)).firstOrNull?.crashedAt ?? '',
      );
      if (upgraded || (time != null && _isOld(time, now))) {
        try {
          file.deleteSync();
        } catch (_) {}
      }
    }
    if (upgraded) {
      _staleBefore = now;
      await ref.read(Preferences.crashReportsVersion.notifier).update(version);
    }
  }

  DateTime? _staleBefore;

  static bool _isOld(DateTime time, DateTime now) => now.difference(time) > const Duration(days: 1);

  Future<void> _offer(List<CrashReport> reports) async {
    if (_offering) return;
    final now = DateTime.now().toUtc();
    final crashes = reports.where((report) {
      if (!report.isCrash) return false;
      final time = DateTime.tryParse(report.crashedAt)?.toUtc();
      if (time == null) return true;
      final before = _staleBefore;
      return !_isOld(time, now) && (before == null || time.isAfter(before));
    }).toList()..sort((a, b) => a.crashedAt.compareTo(b.crashedAt));
    if (crashes.isEmpty) return;
    final newest = crashes.last;
    final key = _seenKey(newest);
    if (key == ref.read(Preferences.lastSeenCrashReport)) return;
    _log.info('new crash report ${newest.name} (${newest.source}), asking to send it');
    _offering = true;
    try {
      state = newest;
      await ref.read(dialogNotifierProvider.notifier).showCrashReport();
      await ref.read(Preferences.lastSeenCrashReport.notifier).update(key);
    } finally {
      state = null;
      _offering = false;
    }
  }

  static String _seenKey(CrashReport report) =>
      report.source == report.name ? '${report.name}@${report.crashedAt}' : report.name;
}
