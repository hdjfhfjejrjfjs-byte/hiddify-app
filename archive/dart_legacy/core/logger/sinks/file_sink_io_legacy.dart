// ARCHIVED LEGACY DART IMPLEMENTATION (Phase 2)
// Original path: lib/core/logger/sinks/file_sink_io.dart
// Replaced by:   rust/crates/hiddify-diagnostics (log_crash_store::open_log_file, write_log_line, write_text_to_file)

import 'dart:io';

String? _path;

bool get isLogFileOpen => _path != null;

const _maxSize = 1024 * 1024;
const _keepSize = 256 * 1024;

void openLogFile(String path) {
  try {
    final file = File(path);
    file.parent.createSync(recursive: true);
    if (file.existsSync() && file.lengthSync() > _maxSize) {
      final bytes = file.readAsBytesSync();
      file.writeAsBytesSync(bytes.sublist(bytes.length - _keepSize));
    }
    file.writeAsStringSync('', mode: FileMode.append);
    _path = path;
  } catch (_) {
    _path = null;
  }
}

void writeLogLine(String line) {
  final path = _path;
  if (path == null) return;
  try {
    File(path).writeAsStringSync('$line\n', mode: FileMode.append);
  } catch (_) {}
}

Future<void> closeLogFile() async {
  _path = null;
}

Future<void> writeLinesToFile(String path, Iterable<String> lines) async {
  await File(path).writeAsString(lines.join('\n'));
}
