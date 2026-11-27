import 'dart:io';

import 'package:hiddify/core/logger/crash/app_crash_store_io.dart' show appCrashDirectory;
import 'package:hiddify/core/rust/hiddify_rust.dart';
import 'package:hiddify/hiddifycore/generated/v2/hcore/hcore.pb.dart';
import 'package:path/path.dart' as p;

/// What crashed, short enough for an issue link or a mail body: [headline] is the exception's own
/// line, [text] the lines that follow it.
typedef CrashExcerpt = ({String headline, String text});

/// The kinds of app crash that ended the process (app_crash_store_io.dart, AppCrashRecorder.kt);
/// a `dart` one is an error the app went on after, so it is sent along but offers no report.
const fatalAppCrashKinds = {'android', 'native', 'anr'};

/// The app's crash files, oldest first.
List<File> appCrashFiles(Directory workingDir) => [
  for (final path in hiddifyRust.require(hiddifyRust.listAppCrashFiles(workingDir.path), 'listAppCrashFiles'))
    File(path),
];

/// The app's crashes that ended the process, as the core lists its own, so the dialog offers both.
List<CrashReport> appCrashReports(Directory workingDir) => [
  for (final report in hiddifyRust.require(
    hiddifyRust.listFatalAppCrashReports(workingDir.path),
    'listFatalAppCrashReports',
  ))
    CrashReport(name: report.name, source: report.source, isCrash: true, crashedAt: report.crashedAt),
];

/// What [report] says crashed, or null when its files are gone or say nothing.
CrashExcerpt? readCrashExcerpt(Directory workingDir, CrashReport report) {
  try {
    final String content;
    switch (report.source) {
      case appCrashDirectory:
        content = File(p.join(workingDir.path, appCrashDirectory, report.name)).readAsStringSync();
        hiddifyRust.ensureAvailable('crashExcerpt');
        return hiddifyRust.crashExcerpt(content, findGoMarker: false);
      case 'oom_reports':
        return (
          headline: 'The core ran out of memory',
          text: 'The core ran out of memory (oom_reports/${report.name}).',
        );
      case 'crash_reports':
        content = File(p.join(workingDir.path, 'crash_reports', report.name, 'go.log')).readAsStringSync();
      default:
        // a live stderr file the core has not archived yet
        content = File(p.join(workingDir.path, report.name)).readAsStringSync();
    }
    hiddifyRust.ensureAvailable('crashExcerpt');
    return hiddifyRust.crashExcerpt(content, findGoMarker: true);
  } catch (_) {
    return null;
  }
}
