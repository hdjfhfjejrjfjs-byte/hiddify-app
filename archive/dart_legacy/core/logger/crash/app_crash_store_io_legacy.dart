// ARCHIVED LEGACY DART IMPLEMENTATION (Phase 2)
// Original path: lib/core/logger/crash/app_crash_store_io.dart & lib/features/crash_report/data/crash_excerpt.dart
// Replaced by:   rust/crates/hiddify-diagnostics (log_crash_store::open_app_crash_store, record_app_crash, list_app_crash_files, list_fatal_app_crash_reports)

import 'dart:io';

import 'package:path/path.dart' as p;

const appCrashDirectory = 'app_crashes';
const fatalAppCrashKinds = {'android', 'native', 'anr'};
const _keep = 10;
const _maxPerRun = 20;

Directory? _directory;
var _recorded = 0;
String? _last;
DateTime? _lastTime;

void legacyOpenAppCrashStore(String workingDir) {
  try {
    final directory = Directory(p.join(workingDir, appCrashDirectory))..createSync(recursive: true);
    _directory = directory;
  } catch (_) {
    _directory = null;
  }
}

void legacyRecordAppCrash(String kind, String text) {
  final directory = _directory;
  if (directory == null || _recorded >= _maxPerRun || text == _last) return;
  _recorded++;
  _last = text;
  try {
    var time = DateTime.now().toUtc();
    if (_lastTime case final last? when !time.isAfter(last)) time = last.add(const Duration(milliseconds: 1));
    var file = _file(directory, time, kind);
    while (file.existsSync()) {
      time = time.add(const Duration(milliseconds: 1));
      file = _file(directory, time, kind);
    }
    _lastTime = time;
    file.writeAsStringSync(text, flush: true);
    _prune(directory);
  } catch (_) {}
}

File _file(Directory directory, DateTime time, String kind) {
  final stamp = time.toIso8601String().substring(0, 23).replaceAll(':', '-').replaceAll('.', '-');
  return File(p.join(directory.path, '$stamp-$kind.txt'));
}

void _prune(Directory directory) {
  final files = directory.listSync().whereType<File>().toList()..sort((a, b) => a.path.compareTo(b.path));
  for (final file in files.take(files.length - _keep < 0 ? 0 : files.length - _keep)) {
    file.deleteSync();
  }
}

List<File> legacyAppCrashFiles(Directory workingDir) {
  final directory = Directory(p.join(workingDir.path, appCrashDirectory));
  if (!directory.existsSync()) return [];
  return directory.listSync().whereType<File>().where((f) => f.path.endsWith('.txt')).toList()
    ..sort((a, b) => a.path.compareTo(b.path));
}

String legacyKind(File file) {
  final name = p.basenameWithoutExtension(file.path);
  return name.substring(name.lastIndexOf('-') + 1);
}

String legacyTime(File file) {
  final name = p.basename(file.path);
  if (name.length < 19) return '';
  final stamp = name.substring(0, 19);
  return '${stamp.substring(0, 10)}T${stamp.substring(11).replaceAll('-', ':')}Z';
}
